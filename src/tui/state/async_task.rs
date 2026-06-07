//! Generic background-task spine for the TUI (issue #231).
//!
//! Generalises the off-thread pattern introduced for the GitHub fetch
//! (#217) so any slow, one-shot operation — currently the worktree list
//! refresh (`f` / `r`), with bootstrap / launcher-prep to follow — runs
//! on a worker thread and posts its result back to the event loop rather
//! than blocking it. The event loop keeps rendering (the statusbar
//! spinner animates, `q` / `Esc` stay responsive); a result whose run
//! was superseded mid-flight is dropped.
//!
//! Unlike [`super::github_fetch`] this is **not** a result cache. The
//! GitHub layer caches `(target, number)` lookups and dedupes them; a
//! refresh / sync / bootstrap is a one-shot "run it, give me a fresh
//! result" with nothing worth caching by key. So the spine keeps only
//! two things from that design — *coalescing* and the *late-result
//! drop* — and drops the per-key cache:
//!
//! - `request(kind)` → `Some(generation)` for a cold slot (the caller
//!   spawns a worker tagged with that generation), or `None` when a run
//!   of the same `kind` is already in flight (coalesced — no second
//!   worker).
//! - the worker computes owned, `Send` data off-thread and posts it back
//!   tagged with the `generation` it was handed.
//! - `complete(kind, generation)` → `true` while the generation is still
//!   authoritative (apply the result), `false` when a later
//!   `invalidate` / `request` bumped the generation mid-flight (drop the
//!   late result — the #138 guard, generalised to non-keyed ops).
//! - `invalidate(kind)` bumps the generation and frees the slot, so any
//!   in-flight result is dropped and a fresh run may start.
//!
//! The threading itself (the `mpsc` channel + `thread::spawn` + the
//! "resolve owned `Send` data on the main thread" discipline) lives on
//! the `App` orchestrator, exactly as the GitHub channel does. This
//! module is pure state — no I/O, no `App` dependency — so the contract
//! is pinned by `tests/tui_state_async_task_tests.rs`.

use crate::bootstrap::BootstrapReport;
use crate::github::{IssueStatus, PrStatus};
use crate::sync::SyncReport;
use crate::worktree::WorktreeInfo;
use std::collections::{HashMap, HashSet};

/// Identity of a background task — the coalescing key and the source of
/// the loader label. One variant per migrated op; bootstrap / sync /
/// launcher-prep join here as their call sites move off-thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
  /// Off-thread worktree list refresh (the `f` / `r` key path). The
  /// synchronous `App::refresh` stays for internal post-mutation callers
  /// (create / delete / report-close) that need the list fresh before the
  /// next render.
  RefreshWorktrees,
  /// Off-thread `gh issue view` fetch for the linked issue, keyed by issue
  /// number (issue #255 — migrated from the separate `github_tx`/`inflight`
  /// channel). The number is the coalescing key, so `Issue(42)` and
  /// `Issue(43)` are independent slots with independent generations — the
  /// per-key identity the late-drop guard needs to discard a stale worker
  /// without clobbering a newer one's slot (the #138 guarantee, now keyed).
  GithubIssue(u64),
  /// PR-side counterpart to [`Self::GithubIssue`] (`gh pr view`). Keyed by
  /// PR number; never collides with an issue of the same number.
  GithubPr(u64),
  /// Off-thread `gwm sync` of the selected worktree (issue #258): fetch +
  /// rebase/merge its branch onto upstream. A single global op like
  /// [`Self::RefreshWorktrees`] — one sync in flight at a time, so a second
  /// `S` press while one runs coalesces instead of racing a second rebase.
  Sync,
  /// Off-thread bootstrap of the selected worktree (issue #256 — the `b`
  /// key): `bootstrap::run` (file copies, guards, command hooks) used to
  /// block the event loop. A single global op like [`Self::Sync`] — one
  /// bootstrap in flight at a time, so a second `b` press coalesces instead
  /// of racing a second run. The TOFU trust gate stays on the main thread
  /// before the spawn; completion sets `App::report` and flips to
  /// `View::Report`.
  Bootstrap,
}

impl TaskKind {
  /// Human label the loader shows while this task is in flight, mirroring
  /// the GitHub fetch's "fetching GitHub status…" so every async site
  /// reads consistently.
  pub fn loading_label(self) -> &'static str {
    match self {
      TaskKind::RefreshWorktrees => "refreshing worktrees…",
      TaskKind::GithubIssue(_) | TaskKind::GithubPr(_) => "fetching GitHub status…",
      TaskKind::Sync => "syncing…",
      TaskKind::Bootstrap => "bootstrapping…",
    }
  }

  /// `true` for the GitHub fetch kinds (`GithubIssue` / `GithubPr`). Used
  /// as the predicate for [`TaskRunner::invalidate_matching`] so the `App`
  /// can drop every in-flight GitHub fetch on navigation / explicit refresh
  /// without naming the (now-stale) issue/PR numbers it no longer holds
  /// (issue #255).
  pub fn is_github(self) -> bool {
    matches!(self, TaskKind::GithubIssue(_) | TaskKind::GithubPr(_))
  }
}

/// Result of an off-thread task, posted from a worker thread back to the
/// event loop over `App`'s task channel (issue #231). Carries owned,
/// `Send` data only (no `git2::Repository` crosses the thread boundary)
/// plus the `generation` the worker was spawned with, so
/// [`TaskRunner::complete`] can drop a superseded late result.
pub enum TaskMsg {
  /// A worktree list refresh result: the freshly-listed worktrees, or a
  /// stringified error from the off-thread `discover_repo` + `list`.
  RefreshWorktrees(u64, std::result::Result<Vec<WorktreeInfo>, String>),
  /// A `gh issue view` result (issue #255): the worker's `generation`, the
  /// issue `number` it fetched, and the parsed [`IssueStatus`] (or a
  /// stringified error). The generation lets [`TaskRunner::complete`] drop a
  /// stale worker's result; the number keys it back into the GitHub cache.
  GithubIssue(u64, u64, std::result::Result<IssueStatus, String>),
  /// PR-side counterpart to [`Self::GithubIssue`] (`gh pr view`).
  GithubPr(u64, u64, std::result::Result<PrStatus, String>),
  /// A `gwm sync` result (issue #258): the worker's `generation`, the synced
  /// worktree's display `name` (for the status line), and the [`SyncReport`]
  /// (or a stringified error — dirty tree, no upstream, conflicts).
  Sync(u64, String, std::result::Result<SyncReport, String>),
  /// A bootstrap result (issue #256): the worker's `generation` and the
  /// [`BootstrapReport`] (or a stringified error). On a live generation the
  /// drain sets `App::report` and flips to `View::Report`; a superseded
  /// late result is dropped by [`TaskRunner::complete`].
  Bootstrap(u64, std::result::Result<BootstrapReport, String>),
}

/// Coalescing + late-drop spine for background tasks (issue #231).
///
/// See the module docs for the full contract. The short version: the
/// `App` calls [`Self::request`] before spawning a worker, branches on
/// the returned generation, and reports the worker's result back via
/// [`Self::complete`] so a superseded late result is dropped.
#[derive(Debug, Default)]
pub struct TaskRunner {
  /// Current authoritative generation per kind. Bumped on each `request`
  /// (a cold spawn) and on each `invalidate`; a result whose generation
  /// no longer matches is dropped. Absent keys are generation 0.
  generation: HashMap<TaskKind, u64>,
  /// Kinds with a worker currently in flight. A `request` for a kind
  /// already here is coalesced (returns `None`); drives the loader's
  /// "is anything loading" signal.
  running: HashSet<TaskKind>,
}

impl TaskRunner {
  /// Construct an empty runner — no generations claimed, nothing in
  /// flight. The `App` constructor calls this once.
  pub fn new() -> Self {
    Self::default()
  }

  /// Claim a run of `kind`. Returns `Some(generation)` for a cold slot —
  /// the caller owns the off-thread spawn and must tag the worker's
  /// result with that generation — or `None` when a run of the same
  /// `kind` is already in flight (coalesced; no second worker).
  pub fn request(&mut self, kind: TaskKind) -> Option<u64> {
    // Coalesce: a run of this kind is already in flight, so a second
    // request rides on it instead of spawning a redundant worker.
    if self.running.contains(&kind) {
      return None;
    }
    let generation = self.generation.entry(kind).or_insert(0);
    *generation += 1;
    let claimed = *generation;
    self.running.insert(kind);
    Some(claimed)
  }

  /// Drop any in-flight run of `kind` and bump the generation so a late
  /// result is discarded, freeing the slot for a fresh run.
  pub fn invalidate(&mut self, kind: TaskKind) {
    *self.generation.entry(kind).or_insert(0) += 1;
    self.running.remove(&kind);
  }

  /// [`Self::invalidate`] every in-flight kind matching `pred` (issue #255).
  /// The GitHub fetch needs this because, on navigation, the `App` clears
  /// the per-key cache for *all* GitHub fetches but no longer holds the
  /// (stale) issue/PR numbers to invalidate them by key — a
  /// `|k| k.is_github()` predicate drops every running GitHub worker's slot
  /// so a fresh fetch starts at a new generation and the stale worker's late
  /// result is discarded by [`Self::complete`]. Only running kinds are
  /// touched: an idle kind has no in-flight worker to drop.
  pub fn invalidate_matching<F: Fn(TaskKind) -> bool>(&mut self, pred: F) {
    let hits: Vec<TaskKind> = self.running.iter().copied().filter(|&k| pred(k)).collect();
    for kind in hits {
      self.invalidate(kind);
    }
  }

  /// Decide whether a result tagged `generation` for `kind` is still
  /// authoritative. Returns `true` (apply it) only when the generation
  /// matches the current one and a run was in flight, clearing the slot;
  /// returns `false` for a late result whose generation was bumped by an
  /// intervening `invalidate` / `request` (the #138 guard).
  pub fn complete(&mut self, kind: TaskKind, generation: u64) -> bool {
    let current = self.generation.get(&kind).copied().unwrap_or(0);
    // Short-circuit on a stale generation so the still-authoritative slot
    // of a *newer* run is never cleared by an older worker's late report.
    if generation != current {
      return false;
    }
    self.running.remove(&kind)
  }

  /// `true` while a run of `kind` is in flight.
  pub fn is_loading(&self, kind: TaskKind) -> bool {
    self.running.contains(&kind)
  }

  /// `true` while any task is in flight — drives the statusbar spinner
  /// alongside the GitHub fetch's own loading signal.
  pub fn is_any_loading(&self) -> bool {
    !self.running.is_empty()
  }

  /// The loader label for an in-flight task, if any. `None` when nothing
  /// is loading.
  pub fn loading_label(&self) -> Option<&'static str> {
    self.running.iter().next().map(|kind| kind.loading_label())
  }
}
