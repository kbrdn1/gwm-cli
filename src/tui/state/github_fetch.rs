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
//!
//! The orchestrator pattern: `App` owns the side-effecting shell-out
//! (`crate::github::fetch_issue` / `fetch_pr`); this sub-struct owns
//! the pure state machine that drives the per-target `*_state`
//! transitions. The `refresh_link` helper resolves the link + slug
//! against the repo + currently-selected branch and invalidates the
//! cached fetch state — the cached result refers to a different
//! `(issue, pr)` tuple after navigation.
//!
//! Slated for a follow-up commit on the same branch: an explicit
//! inflight-dedupe layer (`request` / `complete_*` API) that closes
//! the load-bearing payoff of #128 — multiple concurrent visit events
//! to the same target currently trigger redundant `gh` shell-outs.

use crate::github::{self, BranchLink, IssueStatus, PrStatus};
use git2::Repository;

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

/// GitHub fetch state slice of the TUI `App` (issue #128).
///
/// See the module docs for the full contract; the short version is:
/// owns the cached link / slug / per-target fetch state, with a
/// `refresh_link` helper that re-resolves them against the repo +
/// currently-selected branch and an `invalidate` that flushes the
/// per-target cache.
pub struct GitHubFetch {
  pub link: BranchLink,
  pub link_slug: Option<String>,
  pub issue_state: GitHubFetchState<IssueStatus>,
  pub pr_state: GitHubFetchState<PrStatus>,
}

impl Default for GitHubFetch {
  fn default() -> Self {
    Self::new()
  }
}

impl GitHubFetch {
  /// Construct an empty `GitHubFetch` with no link, no slug, and
  /// `Idle` fetch states. The `App` constructor calls this once and
  /// then immediately runs [`Self::refresh_link`] against the repo so
  /// the cold state lasts only as long as the constructor itself.
  pub fn new() -> Self {
    Self {
      link: BranchLink::empty(),
      link_slug: None,
      issue_state: GitHubFetchState::Idle,
      pr_state: GitHubFetchState::Idle,
    }
  }

  /// Re-read the link for `branch` against `repo`, re-resolve the
  /// repo slug from the `origin` remote, and reset every cached
  /// fetch state. Called by `App::refresh_link` after the user
  /// navigates to a different worktree — the cached state refers to
  /// a different `(issue, pr)` tuple and would be misleading if
  /// reused.
  pub fn refresh_link(&mut self, repo: &Repository, branch: Option<&str>) {
    self.link = branch
      .and_then(|b| github::read_link(repo, b).ok())
      .unwrap_or_else(BranchLink::empty);
    self.link_slug = github::repo_slug(repo).ok();
    self.invalidate();
  }

  /// Flush every cached fetch state. Equivalent to "the cached
  /// `(issue, pr)` tuple is no longer authoritative". Called by
  /// [`Self::refresh_link`]; exposed standalone for callers
  /// (e.g. an explicit "force refresh" key) that want to wipe the
  /// cache without re-reading the link.
  pub fn invalidate(&mut self) {
    self.issue_state = GitHubFetchState::Idle;
    self.pr_state = GitHubFetchState::Idle;
  }

  /// Stamp the issue fetch state from a fetch result. `Ok(s)` →
  /// `Loaded(s)`, `Err(msg)` → `Error(msg)`. Caller is `App` after
  /// it has run `crate::github::fetch_issue` (or, in tests, directly
  /// to pin the post-fetch render contract without spawning `gh`).
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
}
