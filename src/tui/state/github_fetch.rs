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
//! - `issue_state` / `pr_state` — the per-target fetch state machine
//!   ([`GitHubFetchState`]): `Idle` cold, `Loading` while a shell-out
//!   is inflight, `Loaded(T)` on success, `Error(msg)` on failure.
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
//! [`FetchAction::HitCache`] the cached state is already on
//! `*_state`; on [`FetchAction::AlreadyInflight`] a previous request
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

use crate::github::{self, BranchLink, IssueStatus, PrStatus};
use git2::Repository;
use std::collections::HashSet;

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
/// - [`Self::HitCache`] — `*_state` is already `Loaded` or `Error` for
///   this key. The caller is a no-op; the rendered UI reads the
///   cached state directly via `issue_state` / `pr_state`.
/// - [`Self::AlreadyInflight`] — `*_state` is `Loading` for this key
///   AND the key is in the inflight set. A previous `request` is
///   still pending its `complete` call. The caller is a no-op (no
///   shell-out, no UI change).
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
  pub issue_state: GitHubFetchState<IssueStatus>,
  pub pr_state: GitHubFetchState<PrStatus>,
  /// Dedupe set: keys whose `gh` shell-out is currently inflight (the
  /// caller got `Spawn` and hasn't called `complete_*` yet). A
  /// concurrent `request` for a key in this set returns
  /// `AlreadyInflight` — the load-bearing fix for #128.
  inflight: HashSet<FetchKey>,
}

impl Default for GitHubFetch {
  fn default() -> Self {
    Self::new()
  }
}

impl GitHubFetch {
  /// Construct an empty `GitHubFetch` with no link, no slug, `Idle`
  /// fetch states, and an empty inflight set. The `App` constructor
  /// calls this once and then immediately runs [`Self::refresh_link`]
  /// against the repo so the cold state lasts only as long as the
  /// constructor itself.
  pub fn new() -> Self {
    Self {
      link: BranchLink::empty(),
      link_slug: None,
      issue_state: GitHubFetchState::Idle,
      pr_state: GitHubFetchState::Idle,
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
  /// "the cached `(issue, pr)` tuple is no longer authoritative".
  /// Called by [`Self::refresh_link`]; exposed standalone for
  /// callers (e.g. an explicit "force refresh" key like `F`) that
  /// want to wipe the cache without re-reading the link.
  pub fn invalidate(&mut self) {
    self.issue_state = GitHubFetchState::Idle;
    self.pr_state = GitHubFetchState::Idle;
    self.inflight.clear();
  }

  /// Decide what the orchestrator should do for `key`. Three cases:
  ///
  /// 1. The per-target `*_state` is already `Loaded` or `Error` for
  ///    this key — return [`FetchAction::HitCache`]. The caller is a
  ///    no-op; the renderer reads the cached state directly.
  /// 2. The key is in the inflight set — return
  ///    [`FetchAction::AlreadyInflight`]. The caller is a no-op; a
  ///    previous `request(key)` is still pending its `complete`.
  /// 3. Otherwise — cold cache. Mark `*_state = Loading`, insert
  ///    `key` into the inflight set, return
  ///    [`FetchAction::Spawn(key)`]. The caller owns the shell-out
  ///    and MUST call `complete_{issue,pr}` to clear the inflight
  ///    slot.
  ///
  /// The (target, number) tuple is the dedupe identity: `Issue(42)`
  /// and `Pr(42)` are independent slots; `Issue(42)` and `Issue(43)`
  /// are independent slots. See `tests/tui_state_github_fetch_tests.rs`
  /// for the pinned contract.
  pub fn request(&mut self, key: FetchKey) -> FetchAction {
    // Cache hit: prior `complete_*` already populated `*_state` with
    // a terminal variant. No shell-out, no inflight change.
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
      FetchKey::Issue(_) => self.issue_state = GitHubFetchState::Loading,
      FetchKey::Pr(_) => self.pr_state = GitHubFetchState::Loading,
    }
    FetchAction::Spawn(key)
  }

  /// Report the outcome of an issue fetch. Clears the inflight slot
  /// for `Issue(number)` and stores the result on `issue_state` as
  /// `Loaded` (on `Ok`) or `Error` (on `Err`). After this call,
  /// `request(Issue(number))` returns `HitCache` instead of
  /// re-spawning — see the module docs for the cache-on-error
  /// rationale.
  pub fn complete_issue(&mut self, number: u64, result: std::result::Result<IssueStatus, String>) {
    self.inflight.remove(&FetchKey::Issue(number));
    self.apply_issue_result(result);
  }

  /// PR-side counterpart to [`Self::complete_issue`]. Clears the
  /// inflight slot for `Pr(number)` and stores the result on
  /// `pr_state`.
  pub fn complete_pr(&mut self, number: u64, result: std::result::Result<PrStatus, String>) {
    self.inflight.remove(&FetchKey::Pr(number));
    self.apply_pr_result(result);
  }

  /// Stamp the issue fetch state from a fetch result. `Ok(s)` →
  /// `Loaded(s)`, `Err(msg)` → `Error(msg)`. Caller is `App` after
  /// it has run `crate::github::fetch_issue` (or, in tests, directly
  /// to pin the post-fetch render contract without spawning `gh`).
  ///
  /// Does NOT touch the inflight set — that's
  /// [`Self::complete_issue`]'s job. Kept as a separate primitive so
  /// the App's test-friendly `apply_issue_fetch_result` wrapper can
  /// stamp state without needing a paired `request` first.
  pub fn apply_issue_result(&mut self, r: std::result::Result<IssueStatus, String>) {
    self.issue_state = match r {
      Ok(s) => GitHubFetchState::Loaded(s),
      Err(e) => GitHubFetchState::Error(e),
    };
  }

  /// PR-side counterpart to [`Self::apply_issue_result`].
  pub fn apply_pr_result(&mut self, r: std::result::Result<PrStatus, String>) {
    self.pr_state = match r {
      Ok(s) => GitHubFetchState::Loaded(s),
      Err(e) => GitHubFetchState::Error(e),
    };
  }

  /// `true` when the per-target `*_state` carries a terminal variant
  /// (`Loaded` or `Error`) for `key`. Used by [`Self::request`] to
  /// decide between `HitCache` and the cold-cache branch.
  fn is_cached(&self, key: FetchKey) -> bool {
    match key {
      FetchKey::Issue(_) => matches!(
        self.issue_state,
        GitHubFetchState::Loaded(_) | GitHubFetchState::Error(_)
      ),
      FetchKey::Pr(_) => matches!(self.pr_state, GitHubFetchState::Loaded(_) | GitHubFetchState::Error(_)),
    }
  }
}
