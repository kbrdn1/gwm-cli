//! GitHub fetch state for the TUI (issue #128, part 6/6 of the
//! `tui::app::App` decomposition #102).
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
//!   the accessors), `Loading` while a shell-out is inflight,
//!   `Loaded(T)` on success, `Error(msg)` on failure. Per-key identity
//!   matters: pre-#138 the cache was a single per-target slot, so
//!   completing `Issue(42)` falsely "warmed" `Issue(43)` (the cache
//!   identity ignored the number).
//! - `inflight` — an internal `HashSet<FetchKey>` that is the
//!   load-bearing payoff of #128: it dedupes concurrent visit events
//!   so two near-simultaneous calls into [`GitHubFetch::request`] for
//!   the same `(target, number)` only spawn one `gh` subprocess.
//!
//! The orchestrator pattern: `App` does NOT shell out directly anymore
//! on the dedupe path. It calls `self.github.request(key)`; if the
//! return is [`FetchAction::Spawn`], the caller actually invokes
//! `crate::github::fetch_{issue,pr}` and then reports the outcome via
//! `self.github.complete_{issue,pr}(number, result)`. On
//! [`FetchAction::HitCache`] the cached state is already in the
//! per-key map; on [`FetchAction::AlreadyInflight`] a previous request
//! is still in flight and the caller is expected to be a no-op.
//!
//! Why an explicit dedupe layer? Pre-#128, every event that called
//! `refresh_github_status` (worktree navigation, explicit `F` key,
//! visit-driven refresh) would unconditionally spawn `gh issue view`
//! and `gh pr view`. A user rapidly arrow-keying through five
//! worktrees would queue ten `gh` subprocesses in flight, and the
//! shell-out cost dominates the TUI redraw cost. The new contract:
//! every spawn goes through `request(key)`, and a concurrent visit
//! event to a still-loading target is dropped at the state boundary
//! instead of all the way down in the shell.
//!
//! The explicit user-initiated refresh (`F` key →
//! `App::refresh_github_status`) still bypasses the cache via
//! [`GitHubFetch::invalidate`] before its `request` loop — the user
//! just asked for fresh data, so a `HitCache` short-circuit there
//! would be a bug. The inflight slot is still claimed so a
//! concurrent visit-driven `request` dedupes correctly.
//!
//! Late-result handling (issue #138): `complete_*` checks whether the
//! inflight slot survived an intervening `invalidate()` and silently
//! drops the result if not. Pre-fix, a shell-out that completed after
//! the user navigated away (and `invalidate()` cleared the slot)
//! would stamp stale data into the now-active worktree's cache.

use crate::github::{self, BranchLink, IssueStatus, PrStatus};
use git2::Repository;
use std::collections::{HashMap, HashSet};

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

/// Outcome of [`GitHubFetch::request`]. Three states, decided by the
/// state of the per-key cache + the inflight set:
///
/// - [`Self::HitCache`] — the per-key cache already carries a terminal
///   `Loaded` or `Error` variant for this key. The caller is a no-op;
///   the rendered UI reads the cached state via the keyed accessors.
/// - [`Self::AlreadyInflight`] — the cache holds `Loading` AND the key
///   is in the inflight set. A previous `request` is still pending
///   its `complete` call. The caller is a no-op (no shell-out, no UI
///   change).
/// - [`Self::Spawn`] — cold cache. The caller owns the side-effecting
///   `gh` shell-out; the [`FetchKey`] payload tells it which
///   subcommand to dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchAction {
  HitCache,
  AlreadyInflight,
  Spawn(FetchKey),
}

/// GitHub fetch state slice of the TUI `App` (issue #128).
///
/// See the module docs for the full contract; the short version is:
/// `App` calls [`Self::request`] before any `gh` shell-out, branches
/// on the returned [`FetchAction`], and reports the result back via
/// [`Self::complete_issue`] / [`Self::complete_pr`] so the next
/// `request` for the same key hits the cache instead of re-spawning.
pub struct GitHubFetch {
  pub link: BranchLink,
  pub link_slug: Option<String>,
  /// Per-issue-number cache. Absent keys are `Idle`. Closed over by
  /// the keyed accessor `issue_fetch_state(number)` (#138 fix: the
  /// cache is keyed by number, not a single per-target slot).
  issue_cache: HashMap<u64, GitHubFetchState<IssueStatus>>,
  /// PR-side counterpart to [`Self::issue_cache`].
  pr_cache: HashMap<u64, GitHubFetchState<PrStatus>>,
  /// Dedupe set: keys whose `gh` shell-out is currently inflight (the
  /// caller got `Spawn` and hasn't called `complete_*` yet). A
  /// concurrent `request` for a key in this set returns
  /// `AlreadyInflight` — the load-bearing fix for #128. Also acts as
  /// the "still authoritative" gate for `complete_*` (#138): a result
  /// arriving for a key not in `inflight` was invalidated mid-flight
  /// and gets dropped.
  inflight: HashSet<FetchKey>,
}

impl Default for GitHubFetch {
  fn default() -> Self {
    Self::new()
  }
}

impl GitHubFetch {
  /// Construct an empty `GitHubFetch` with no link, no slug, empty
  /// per-key caches, and an empty inflight set. The `App` constructor
  /// calls this once and then immediately runs [`Self::refresh_link`]
  /// against the repo so the cold state lasts only as long as the
  /// constructor itself.
  pub fn new() -> Self {
    Self {
      link: BranchLink::empty(),
      link_slug: None,
      issue_cache: HashMap::new(),
      pr_cache: HashMap::new(),
      inflight: HashSet::new(),
    }
  }

  /// Re-read the link for `branch` against `repo`, re-resolve the
  /// repo slug from the `origin` remote, and reset every cached
  /// fetch state + inflight slot. Called by `App::refresh_link`
  /// after the user navigates to a different worktree — the cached
  /// state refers to a different `(issue, pr)` tuple and would be
  /// misleading if reused.
  pub fn refresh_link(&mut self, repo: &Repository, branch: Option<&str>) {
    self.link = branch
      .and_then(|b| github::read_link(repo, b).ok())
      .unwrap_or_else(BranchLink::empty);
    self.link_slug = github::repo_slug(repo).ok();
    self.invalidate();
  }

  /// Flush every cached fetch state + inflight slot. Equivalent to
  /// "the cached `(issue, pr)` tuples are no longer authoritative".
  /// Called by [`Self::refresh_link`]; exposed standalone for
  /// callers (e.g. an explicit "force refresh" key like `F`) that
  /// want to wipe the cache without re-reading the link.
  ///
  /// Clearing `inflight` here is load-bearing for #138: any `gh`
  /// shell-out still in flight will report back via `complete_*`,
  /// and the empty inflight set is how `complete_*` knows to drop
  /// that late result instead of stamping it into the freshly-active
  /// worktree's cache.
  pub fn invalidate(&mut self) {
    self.issue_cache.clear();
    self.pr_cache.clear();
    self.inflight.clear();
  }

  /// Decide what the orchestrator should do for `key`. Three cases:
  ///
  /// 1. The per-key cache holds a terminal `Loaded` or `Error` for
  ///    this key — return [`FetchAction::HitCache`]. The caller is a
  ///    no-op; the renderer reads the cached state via the keyed
  ///    accessors.
  /// 2. The key is in the inflight set — return
  ///    [`FetchAction::AlreadyInflight`]. The caller is a no-op; a
  ///    previous `request(key)` is still pending its `complete`.
  /// 3. Otherwise — cold cache. Insert `Loading` at the key, claim
  ///    the inflight slot, return [`FetchAction::Spawn(key)`]. The
  ///    caller owns the shell-out and MUST call
  ///    `complete_{issue,pr}` to clear the inflight slot.
  ///
  /// The (target, number) tuple is the dedupe identity: `Issue(42)`
  /// and `Pr(42)` are independent slots; `Issue(42)` and `Issue(43)`
  /// are independent slots (the per-key cache enforces this, post-
  /// #138). See `tests/tui_state_github_fetch_tests.rs` for the
  /// pinned contract.
  pub fn request(&mut self, key: FetchKey) -> FetchAction {
    // Cache hit: prior `complete_*` already populated the per-key
    // entry with a terminal variant. No shell-out, no inflight change.
    if self.is_cached(key) {
      return FetchAction::HitCache;
    }
    // Inflight: a prior `request(key)` is still pending its
    // `complete_*`. Dedupe — the redundant `gh` shell-out is exactly
    // what #128 closes.
    if self.inflight.contains(&key) {
      return FetchAction::AlreadyInflight;
    }
    // Cold cache: flip to Loading, claim the inflight slot, hand the
    // caller the key so it knows which subcommand to dispatch.
    self.inflight.insert(key);
    match key {
      FetchKey::Issue(n) => {
        self.issue_cache.insert(n, GitHubFetchState::Loading);
      }
      FetchKey::Pr(n) => {
        self.pr_cache.insert(n, GitHubFetchState::Loading);
      }
    }
    FetchAction::Spawn(key)
  }

  /// Report the outcome of an issue fetch. The contract has two
  /// guards (post-#138):
  ///
  /// 1. If the inflight slot for `Issue(number)` was already cleared
  ///    (typically by an intervening [`Self::invalidate`]), the
  ///    result is dropped — the user has navigated away and the
  ///    late shell-out result is no longer authoritative.
  /// 2. Otherwise, the per-key cache entry is stamped with `Loaded`
  ///    on `Ok` or `Error` on `Err`. After this call,
  ///    `request(Issue(number))` returns `HitCache` instead of
  ///    re-spawning — see the module docs for the cache-on-error
  ///    rationale.
  pub fn complete_issue(&mut self, number: u64, result: std::result::Result<IssueStatus, String>) {
    // #138 guard: if invalidate() cleared the slot mid-flight, drop
    // the late result. Stamping it would corrupt the now-active
    // worktree's cache with the previous worktree's data.
    if !self.inflight.remove(&FetchKey::Issue(number)) {
      return;
    }
    self.issue_cache.insert(number, into_state(result));
  }

  /// PR-side counterpart to [`Self::complete_issue`]. Same #138 guard
  /// applies: a late result whose inflight slot was cleared by an
  /// intervening [`Self::invalidate`] is dropped.
  pub fn complete_pr(&mut self, number: u64, result: std::result::Result<PrStatus, String>) {
    if !self.inflight.remove(&FetchKey::Pr(number)) {
      return;
    }
    self.pr_cache.insert(number, into_state(result));
  }

  /// Stamp the issue fetch state from a fetch result. `Ok(s)` →
  /// `Loaded(s)` (keyed by `s.number`), `Err(msg)` → `Error(msg)`
  /// (keyed by the current `link.issue` if any). Test-friendly
  /// wrapper used by `App::apply_issue_fetch_result`; does NOT
  /// touch the inflight set (that's [`Self::complete_issue`]'s job)
  /// and does NOT honour the late-result drop, because there's no
  /// inflight slot to consult — the helper is for tests that stamp
  /// state directly without going through `request → complete`.
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
  /// (`Loaded` or `Error`) for `key`. Used by [`Self::request`] to
  /// decide between `HitCache` and the cold-cache branch. Post-#138
  /// the cache is keyed by number, so `is_cached(Issue(43))` after
  /// a `complete_issue(42, …)` correctly returns `false`.
  fn is_cached(&self, key: FetchKey) -> bool {
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
