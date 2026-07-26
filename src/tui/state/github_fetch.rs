//! GitHub fetch state for the TUI (issue #128, part 6/6 of the
//! `tui::app::App` decomposition #102; threading migrated onto the
//! shared async-task spine in #255).
//!
//! Owns the slice of `App` state that tracks issue / PR linking + the
//! cached results of the `gh issue view` / `gh pr view` shell-outs:
//!
//! - `link` — the [`BranchLink`] resolved for the currently-selected
//!   worktree's branch (the `(issue, pr)` tuple plus their provenance
//!   `LinkSource` markers).
//! - `link_slug` — the `owner/repo` slug parsed from the `origin`
//!   remote, `None` when there is no GitHub remote.
//! - `issue_cache` / `pr_cache` — per-(target, number) caches keyed
//!   by issue / PR number. Each entry is a [`GitHubFetchState`]: cold
//!   entries are simply absent from the map (treated as `Idle` by
//!   the accessors), `Loading` while a shell-out is in flight,
//!   `Loaded(T)` on success, `Error(msg)` on failure. Per-key identity
//!   matters: pre-#138 the cache was a single per-target slot, so
//!   completing `Issue(42)` falsely "warmed" `Issue(43)` (the cache
//!   identity ignored the number).
//!
//! **What this module is, post-#255:** a *result cache* + link state.
//! It deliberately no longer owns the off-thread coalescing / dedupe /
//! late-result-drop — that machinery is now the generic
//! [`super::async_task::TaskRunner`] spine, shared with the worktree
//! refresh, so there is one off-thread mechanism instead of two. Pre-
//! #255 this module also held an `inflight: HashSet<FetchKey>` that
//! deduped concurrent fetches and gated the #138 late-drop, but it had
//! **no per-fetch generation**: two workers for the same key (the
//! request → invalidate → request retry path) were indistinguishable,
//! so a stale worker that reported first could win the slot and a fresh
//! result be dropped (Codex adversarial-review finding on PR #260). The
//! spine's per-key generation counter fixes that race.
//!
//! The orchestrator pattern (post-#255): `App` checks
//! [`GitHubFetch::is_cached`] for a terminal hit; on a miss it claims a
//! generation from the spine ([`TaskRunner::request`]), marks the cache
//! [`GitHubFetchState::Loading`] via [`GitHubFetch::mark_loading`], and
//! spawns the `gh` worker tagged with that generation. The worker posts
//! a `TaskMsg::Github{Issue,Pr}` back; the event loop applies it only
//! when [`TaskRunner::complete`] confirms the generation is still
//! authoritative, then stamps the terminal result here via
//! [`GitHubFetch::complete_issue`] / [`GitHubFetch::complete_pr`] (pure
//! cache writes — the drop decision lives on the spine now).
//!
//! The explicit user-initiated refresh (`F` key →
//! `App::refresh_github_status`) flushes the cache via
//! [`GitHubFetch::invalidate`] before re-requesting — the user just
//! asked for fresh data, so a `HitCache` short-circuit there would be a
//! bug — and the `App` pairs that flush with a spine
//! `invalidate_matching(is_github)` so any in-flight worker's late
//! result is dropped (the navigation invariant: cache clear and spine
//! generation-bump always move together).

use crate::github::{self, BranchLink, IssueStatus, PrStatus};
use git2::Repository;
use std::collections::HashMap;

/// State of a background GitHub fetch (issue or PR). Generic over `T`
/// so the same enum drives both `IssueStatus` and `PrStatus`. The
/// `Idle` variant is the cold-cache identity; `Loading` flags an
/// inflight `gh` shell-out so the UI can paint a "…loading" badge;
/// `Loaded(T)` and `Error(String)` are the two terminal outcomes.
///
/// Moved out of `tui::app` per #128 — this module owns the type now
/// because it owns the state machine that drives transitions between
/// the variants. Re-exported from `tui::mod` (and from `tui::app` for
/// callers that already imported it from its historical path) so the
/// public surface stays at the same path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubFetchState<T> {
  Idle,
  Loading,
  Loaded(T),
  Error(String),
}

/// Static `Idle` constant for `IssueStatus` so the keyed accessor can
/// hand back a reference for absent keys without allocating per call.
/// Lives at module scope so it has `'static` lifetime — required for
/// the borrow returned by `issue_fetch_state(number)` when the map
/// has no entry.
const IDLE_ISSUE: GitHubFetchState<IssueStatus> = GitHubFetchState::Idle;

/// PR-side counterpart to [`IDLE_ISSUE`].
const IDLE_PR: GitHubFetchState<PrStatus> = GitHubFetchState::Idle;

/// Identity of a GitHub fetch. The `(target, number)` tuple is the
/// dedupe key: `Issue(42)` and `Pr(42)` never collide (they hit
/// different `gh` subcommands), and `Issue(42)` vs `Issue(43)` are
/// independent fetches against different REST endpoints.
///
/// Carried through [`FetchAction::Spawn`] so the orchestrator knows
/// which side to dispatch without re-encoding the discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FetchKey {
  Issue(u64),
  Pr(u64),
}

/// GitHub fetch state slice of the TUI `App` (issue #128; threading on
/// the async-task spine since #255).
///
/// See the module docs for the full contract; the short version is:
/// `App` checks [`Self::is_cached`], marks [`Self::mark_loading`] before
/// spawning the `gh` worker on the [`super::async_task::TaskRunner`]
/// spine, and stamps the terminal result back via
/// [`Self::complete_issue`] / [`Self::complete_pr`] once the spine
/// confirms the worker's generation is still authoritative.
pub struct GitHubFetch {
  pub link: BranchLink,
  pub link_slug: Option<String>,
  /// The resolved forge backend for the active repo (issue #419), or
  /// `None` when `origin` is missing / unparseable. Re-resolved on every
  /// [`Self::refresh_link`] rather than cached on the `App`: in workspace
  /// mode the active repo — and with it the `.gwm.toml` `forge` key — can
  /// change under the cursor.
  pub forge: Option<std::sync::Arc<dyn crate::forge::Forge>>,
  /// Per-issue-number cache. Absent keys are `Idle`. Closed over by
  /// the keyed accessor `issue_fetch_state(number)` (#138 fix: the
  /// cache is keyed by number, not a single per-target slot).
  issue_cache: HashMap<u64, GitHubFetchState<IssueStatus>>,
  /// PR-side counterpart to [`Self::issue_cache`].
  pr_cache: HashMap<u64, GitHubFetchState<PrStatus>>,
}

impl Default for GitHubFetch {
  fn default() -> Self {
    Self::new()
  }
}

impl GitHubFetch {
  /// Construct an empty `GitHubFetch` with no link, no slug, and empty
  /// per-key caches. The `App` constructor calls this once and then
  /// immediately runs [`Self::refresh_link`] against the repo so the
  /// cold state lasts only as long as the constructor itself.
  pub fn new() -> Self {
    Self {
      link: BranchLink::empty(),
      link_slug: None,
      forge: None,
      issue_cache: HashMap::new(),
      pr_cache: HashMap::new(),
    }
  }

  /// Re-read the link for `branch` against `repo`, re-resolve the
  /// repo slug from the `origin` remote, and reset every cached
  /// fetch state. Called by `App::refresh_link` after the user
  /// navigates to a different worktree — the cached state refers to a
  /// different `(issue, pr)` tuple and would be misleading if reused.
  /// (`App::refresh_link` separately drops any in-flight GitHub worker
  /// on the spine — see [`Self::invalidate`] for the pairing.)
  pub fn refresh_link(&mut self, repo: &Repository, branch: Option<&str>, config: &crate::config::Config) {
    self.reread_link(repo, branch, config);
    self.invalidate();
  }

  /// Re-read the link and forge **without** dropping fetched results.
  ///
  /// [`Self::refresh_link`] clears the caches on purpose: a selection
  /// change must not leave the previous row's status on screen (PR #68).
  /// But the open menu calls it only to catch a link made in another
  /// terminal, and clearing there wiped the server-reported `web_url` it
  /// was about to read, so it always fell back to a locally built URL —
  /// wrong exactly where it matters, on an origin whose web host or port
  /// gwm cannot infer (Codex review #458).
  ///
  /// Preserving is safe by construction: the caches are keyed by issue /
  /// PR number, so a link that really did change simply misses.
  pub fn reread_link(&mut self, repo: &Repository, branch: Option<&str>, config: &crate::config::Config) {
    self.link = branch
      .and_then(|b| github::read_link(repo, b).ok())
      .unwrap_or_else(BranchLink::empty);
    self.forge = crate::forge::resolve(repo, config).ok();
    // Kept in sync with `forge` so the many read-only slug consumers
    // (detail overlay keys, status strings) need no rewrite.
    self.link_slug = self.forge.as_ref().map(|f| f.slug().to_string());
  }

  /// Flush every cached fetch state. Equivalent to "the cached
  /// `(issue, pr)` tuples are no longer authoritative". Called by
  /// [`Self::refresh_link`]; exposed standalone for callers (e.g. an
  /// explicit "force refresh" key like `F`) that want to wipe the cache
  /// without re-reading the link.
  ///
  /// Post-#255 this clears only the result cache. Dropping any *in-
  /// flight* worker's late result is the spine's job: the `App` pairs
  /// every `invalidate()` with a
  /// [`TaskRunner::invalidate_matching(is_github)`](super::async_task::TaskRunner::invalidate_matching)
  /// so the stale generation is bumped and its result discarded by
  /// [`TaskRunner::complete`](super::async_task::TaskRunner::complete).
  /// That pairing is the navigation invariant — cache clear and spine
  /// generation-bump always move together.
  pub fn invalidate(&mut self) {
    self.issue_cache.clear();
    self.pr_cache.clear();
  }

  /// Stamp an auto-detected PR onto the resolved `link` when none is
  /// already linked (issue #181). Pure: the `App` orchestrator owns the
  /// `gh pr list --head <branch>` shell-out and feeds the detected number
  /// here so the sidebar's `pr_fetch_state()` can resolve it. Delegates
  /// to [`github::apply_detected_pr`], so an explicit `gwm link --pr`
  /// (already on `link.pr`) always wins and the result is marked
  /// `LinkSource::Detected`. This only mutates in-memory state; the `App`
  /// separately persists the detection via
  /// [`github::persist_detected_pr`] (issue #283) so the table read path
  /// picks it up.
  pub fn apply_detected_pr(&mut self, detected: Option<u64>) {
    github::apply_detected_pr(&mut self.link, detected);
  }

  /// Drop a previously auto-detected PR so the next refresh re-resolves
  /// it from GitHub (issue #181 — the detected link is "resolved live",
  /// so a PR that was opened/closed/replaced while sitting on the same
  /// worktree must not stick across `F` presses). A no-op for an
  /// explicit (`gwm link --pr`) or branch-name link — those stay pinned.
  pub fn clear_detected_pr(&mut self) {
    if self.link.pr_source == github::LinkSource::Detected {
      self.link.pr = None;
      self.link.pr_source = github::LinkSource::None;
    }
  }

  /// Flip the per-key cache entry for `key` to
  /// [`GitHubFetchState::Loading`] (issue #255). Called by the `App`
  /// after it has claimed a generation from the spine and is about to
  /// spawn the `gh` worker, so the sidebar paints a "…loading" badge and
  /// [`App::is_github_loading`](crate::tui::App) reads `true` until the
  /// terminal result lands. Pure: coalescing (skip the spawn if a worker
  /// is already running) is the spine's call, not this module's.
  pub fn mark_loading(&mut self, key: FetchKey) {
    match key {
      FetchKey::Issue(n) => {
        self.issue_cache.insert(n, GitHubFetchState::Loading);
      }
      FetchKey::Pr(n) => {
        self.pr_cache.insert(n, GitHubFetchState::Loading);
      }
    }
  }

  /// `true` when the per-key cache already carries a terminal `Loaded`
  /// or `Error` for `key` (issue #255). The `App` consults this before
  /// claiming a spine generation so an explicit-but-already-warm key
  /// skips a redundant `gh` spawn. The `(target, number)` identity is
  /// the cache key: `Issue(42)` / `Pr(42)` / `Issue(43)` are all
  /// independent (post-#138).
  pub fn is_cached(&self, key: FetchKey) -> bool {
    self.has_terminal(key)
  }

  /// Stamp the terminal outcome of an issue fetch into the per-key cache
  /// (issue #255). Pure cache write — `Ok` → `Loaded`, `Err` → `Error`,
  /// keyed by `number`. After this call,
  /// [`Self::is_cached(Issue(number))`](Self::is_cached) returns `true`.
  ///
  /// Post-#255 there is no late-result guard here: the drop decision for
  /// a superseded worker lives on the spine
  /// ([`TaskRunner::complete`](super::async_task::TaskRunner::complete)),
  /// which the `App` checks *before* calling this. So by the time we
  /// stamp, the result is already known authoritative.
  pub fn complete_issue(&mut self, number: u64, result: std::result::Result<IssueStatus, String>) {
    self.issue_cache.insert(number, into_state(result));
  }

  /// PR-side counterpart to [`Self::complete_issue`] — pure cache write,
  /// no late-result guard (the spine owns that since #255).
  pub fn complete_pr(&mut self, number: u64, result: std::result::Result<PrStatus, String>) {
    self.pr_cache.insert(number, into_state(result));
  }

  /// Stamp the issue fetch state from a fetch result. `Ok(s)` →
  /// `Loaded(s)` (keyed by `s.number`), `Err(msg)` → `Error(msg)`
  /// (keyed by the current `link.issue` if any). Test-friendly
  /// wrapper used by `App::apply_issue_fetch_result`; the helper is for
  /// tests that stamp state directly without going through the spine's
  /// `request → complete` generation flow.
  ///
  /// If `Err` is given and no link issue is set, the helper is a
  /// no-op (there's no number to key by). Tests that exercise the
  /// error path should set up a branch link first via
  /// `make_app_on_branch("feat/#<n>-…")`.
  pub fn apply_issue_result(&mut self, r: std::result::Result<IssueStatus, String>) {
    let (number, state) = match r {
      Ok(s) => (s.number, GitHubFetchState::Loaded(s)),
      Err(e) => {
        let Some(n) = self.link.issue else {
          return;
        };
        (n, GitHubFetchState::Error(e))
      }
    };
    self.issue_cache.insert(number, state);
  }

  /// PR-side counterpart to [`Self::apply_issue_result`]. Same
  /// no-op-on-Err-without-link contract.
  pub fn apply_pr_result(&mut self, r: std::result::Result<PrStatus, String>) {
    let (number, state) = match r {
      Ok(s) => (s.number, GitHubFetchState::Loaded(s)),
      Err(e) => {
        let Some(n) = self.link.pr else {
          return;
        };
        (n, GitHubFetchState::Error(e))
      }
    };
    self.pr_cache.insert(number, state);
  }

  /// Read the cached fetch state for `Issue(number)`. Returns
  /// `&GitHubFetchState::Idle` for absent keys via a `'static`
  /// constant so the borrow is cheap and lifetime-free. Used by the
  /// renderer (`src/tui/ui.rs`) and the `App`-level wrapper
  /// `App::issue_fetch_state` to read the cache without leaking the
  /// per-key map shape.
  pub fn issue_fetch_state(&self, number: u64) -> &GitHubFetchState<IssueStatus> {
    self.issue_cache.get(&number).unwrap_or(&IDLE_ISSUE)
  }

  /// PR-side counterpart to [`Self::issue_fetch_state`].
  pub fn pr_fetch_state(&self, number: u64) -> &GitHubFetchState<PrStatus> {
    self.pr_cache.get(&number).unwrap_or(&IDLE_PR)
  }

  /// `true` when the per-key cache carries a terminal variant
  /// (`Loaded` or `Error`) for `key`. Backs the public
  /// [`Self::is_cached`]. Post-#138 the cache is keyed by number, so
  /// `has_terminal(Issue(43))` after a `complete_issue(42, …)` correctly
  /// returns `false`.
  fn has_terminal(&self, key: FetchKey) -> bool {
    match key {
      FetchKey::Issue(n) => matches!(
        self.issue_cache.get(&n),
        Some(GitHubFetchState::Loaded(_)) | Some(GitHubFetchState::Error(_))
      ),
      FetchKey::Pr(n) => matches!(
        self.pr_cache.get(&n),
        Some(GitHubFetchState::Loaded(_)) | Some(GitHubFetchState::Error(_))
      ),
    }
  }
}

/// Translate a fetch `Result` into the corresponding terminal
/// [`GitHubFetchState`] variant. Pulled out as a free function so
/// both `complete_issue` and `complete_pr` can call it without
/// having to repeat the `match` — same body, two type parameters.
fn into_state<T>(r: std::result::Result<T, String>) -> GitHubFetchState<T> {
  match r {
    Ok(s) => GitHubFetchState::Loaded(s),
    Err(e) => GitHubFetchState::Error(e),
  }
}
