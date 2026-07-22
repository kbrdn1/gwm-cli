//! Generic background-task spine for the TUI (issue #231).
//!
//! Generalises the off-thread pattern introduced for the GitHub fetch
//! (#217) so any slow, one-shot operation — worktree list refresh, create,
//! sync, bootstrap, delete, and GitHub fetches — runs
//! on a worker thread and posts its result back to the event loop rather
//! than blocking it. The event loop keeps rendering (the statusbar
//! spinner animates, `q` / `Esc` stay responsive); a result whose run
//! was superseded mid-flight is dropped.
//!
//! Unlike [`super::github_fetch`] this is **not** a result cache. The
//! GitHub layer caches `(target, number)` lookups and dedupes them; a
//! create / refresh / sync / bootstrap / delete-worktree is a one-shot "run
//! it, give me a fresh result" with nothing worth caching by key. So the spine keeps only
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
use crate::tui::state::sidebar::SidebarMode;
use crate::tui::ui::SidebarSections;
use crate::worktree::WorktreeInfo;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Identity of a background task — the coalescing key and the source of
/// the loader label. One variant per migrated op.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
  /// Off-thread create flow from the Create modal (issue #276):
  /// `worktree::add` plus bootstrap can touch disk, refs, copies and hooks, so
  /// the modal must stay renderable while it runs. A single global op like
  /// [`Self::Bootstrap`] — one create in flight at a time.
  CreateWorktree,
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
  /// Off-thread delete of the selected worktree (issue #257):
  /// `worktree::remove` can touch git admin files, remove the worktree
  /// directory, and optionally delete the branch, so it must not block the
  /// render loop while the confirm modal is open.
  DeleteWorktree,
  /// Off-thread `git pull` of the selected worktree's branch (#290). One
  /// global slot — a second `p` press coalesces while one is in flight.
  Pull,
  /// Off-thread `git push` of the selected worktree's branch to its remote
  /// (#290). One global slot — a second `P` press coalesces.
  Push,
  /// Off-thread rename of the selected worktree (`c`, #290): renames the
  /// local branch (`git branch -m`), the remote branch when it exists
  /// (`git push origin :<old> <new>:<new>` + re-track), and moves the
  /// worktree directory on disk (`git worktree move`) so the slug stays in
  /// sync. One global slot — a second `c` submit coalesces.
  EditWorktree,
  /// Off-thread re-list of *every* repo in workspace mode (issue #343 /
  /// #36): `maybe_auto_refresh` and the `f` / `r` key path used to call
  /// `refresh_workspace` synchronously (a `Repository::open` + `worktree::list`
  /// per repo) on the event-loop thread, freezing the UI for the duration on a
  /// many-repo workspace. The single-repo refresh ([`Self::RefreshWorktrees`])
  /// can't be reused — it would clobber the merged table with one repo's
  /// worktrees — so this is its workspace-shaped sibling. One global slot; a
  /// second refresh coalesces. The synchronous `App::refresh` (post-mutation
  /// callers) stays synchronous and invalidates this slot.
  RefreshWorkspace,
  /// Off-thread rebuild of the details sidebar's git-backed sections (issue
  /// #343): `git_diff_stat_vs_base` + `git status --porcelain -z` + `git log` /
  /// `git stash list` used to run synchronously inside `terminal.draw()` on
  /// every selection / mode change, stalling `j` / `k` on a large repo. A
  /// single global slot keyed to *the currently selected* worktree — pure
  /// navigation never [`TaskRunner::invalidate`]s it, so a held `j` coalesces
  /// onto the in-flight worker (one at a time) instead of spawning a thread
  /// per row; the render key-check discards a result for a since-moved
  /// selection and the next tick requests the settled one.
  Sidebar,
  /// Off-thread agent-session detection (issue #408): the four artefact
  /// scans under the user's home (`agent_sessions::detect_all`) touch the
  /// filesystem and must never run inside `terminal.draw()`. A single global
  /// slot — a tick that finds a run in flight coalesces; the render reads
  /// the last completed snapshot only.
  AgentSessions,
}

impl TaskKind {
  /// Human label the loader shows while this task is in flight, mirroring
  /// the GitHub fetch's "fetching GitHub status…" so every async site
  /// reads consistently.
  pub fn loading_label(self) -> &'static str {
    match self {
      TaskKind::CreateWorktree => "creating worktree…",
      TaskKind::RefreshWorktrees => "refreshing worktrees…",
      TaskKind::GithubIssue(_) | TaskKind::GithubPr(_) => "fetching GitHub status…",
      TaskKind::Sync => "syncing…",
      TaskKind::Bootstrap => "bootstrapping…",
      TaskKind::DeleteWorktree => "deleting worktree…",
      TaskKind::Pull => "pulling…",
      TaskKind::Push => "pushing…",
      TaskKind::EditWorktree => "renaming worktree…",
      TaskKind::RefreshWorkspace => "refreshing worktrees…",
      TaskKind::Sidebar => "loading preview…",
      TaskKind::AgentSessions => "detecting agent sessions…",
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

  /// `true` for workers that can leave repository / worktree state
  /// partially changed if the process exits before their result is drained.
  pub fn is_mutating(self) -> bool {
    matches!(
      self,
      TaskKind::CreateWorktree
        | TaskKind::Sync
        | TaskKind::Bootstrap
        | TaskKind::DeleteWorktree
        | TaskKind::Pull
        | TaskKind::Push
        | TaskKind::EditWorktree
    )
  }
}

/// Successful result of a Create-modal worker (issue #276).
pub struct CreateWorktreeResult {
  pub branch: String,
  pub created: PathBuf,
  pub report: BootstrapReport,
}

/// Successful result of an Edit-modal worker (`c`, #290). Carries the new
/// branch name, the new on-disk path (after `git worktree move`), and the
/// new worktree display name so the drain can refresh the list and report
/// the rename in the status bar.
pub struct EditWorktreeResult {
  pub new_branch: String,
  pub new_path: PathBuf,
  pub new_name: String,
  /// `true` when the remote branch was also renamed (it existed on origin).
  pub remote_renamed: bool,
}

/// Result of an off-thread task, posted from a worker thread back to the
/// event loop over `App`'s task channel (issue #231). Carries owned,
/// `Send` data only (no `git2::Repository` crosses the thread boundary)
/// plus the `generation` the worker was spawned with, so
/// [`TaskRunner::complete`] can drop a superseded late result.
pub enum TaskMsg {
  /// A create-worktree result (issue #276): the worker's `generation` and the
  /// created worktree + bootstrap report, or a stringified failure from naming,
  /// libgit2 worktree creation, or bootstrap.
  CreateWorktree(u64, std::result::Result<CreateWorktreeResult, String>),
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
  /// A delete-worktree result (issue #257): the worker's generation, the
  /// deleted worktree's display name + path label for the status line, and
  /// the deletion outcome.
  DeleteWorktree(u64, String, String, std::result::Result<(), String>),
  /// A `git pull` result (#290): the worker's generation, the worktree's
  /// display name, and the outcome (a one-line status string on success or
  /// a stringified error).
  Pull(u64, String, std::result::Result<String, String>),
  /// A `git push` result (#290): same shape as [`Self::Pull`].
  Push(u64, String, std::result::Result<String, String>),
  /// An edit-worktree result (`c`, #290): the worker's generation and the
  /// rename outcome (new branch/path/name on success, or a stringified error
  /// from `git branch -m` / `git push` / `git worktree move`).
  EditWorktree(u64, std::result::Result<EditWorktreeResult, String>),
  /// A workspace re-list result (issue #343 / #36): the worker's generation and
  /// the merged `(worktree, repo_index)` rows across every repo. Per-repo open
  /// / list errors are swallowed exactly as the synchronous path did (a broken
  /// repo drops its rows, the rest still list), so there is no error arm to
  /// carry. The drain rebuilds the merged table + row→repo map.
  RefreshWorkspace(u64, Vec<(WorktreeInfo, usize)>),
  /// A rebuilt sidebar payload (issue #343): the worker's generation, the
  /// worktree `path` and [`SidebarMode`] it was built for (the render key), and
  /// the pre-rendered [`SidebarSections`]. The drain stores it into
  /// `SidebarState::cache`; a result whose selection has since moved is dropped
  /// by [`TaskRunner::complete`] (generation) and ignored by the render (key).
  Sidebar(u64, PathBuf, SidebarMode, SidebarSections),
  /// An agent-session detection result (issue #408): the worker's generation
  /// and the per-worktree-path summary. The drain replaces the app snapshot
  /// atomically; a superseded late result is dropped by
  /// [`TaskRunner::complete`].
  AgentSessions(
    u64,
    std::collections::BTreeMap<String, crate::agent_sessions::WorktreeAgents>,
    Vec<crate::agent_sessions::AgentSession>,
  ),
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

  /// `true` while a mutating worker is still in flight. Quit handling uses
  /// this to keep `sync` / `bootstrap` / delete-worktree from being abandoned mid-operation.
  pub fn has_mutating_task_in_flight(&self) -> bool {
    self.running.iter().any(|kind| kind.is_mutating())
  }

  /// Loader label for a mutating in-flight task, if any.
  pub fn mutating_loading_label(&self) -> Option<&'static str> {
    if self.running.contains(&TaskKind::CreateWorktree) {
      Some(TaskKind::CreateWorktree.loading_label())
    } else if self.running.contains(&TaskKind::Sync) {
      Some(TaskKind::Sync.loading_label())
    } else if self.running.contains(&TaskKind::Bootstrap) {
      Some(TaskKind::Bootstrap.loading_label())
    } else if self.running.contains(&TaskKind::DeleteWorktree) {
      Some(TaskKind::DeleteWorktree.loading_label())
    } else if self.running.contains(&TaskKind::Pull) {
      Some(TaskKind::Pull.loading_label())
    } else if self.running.contains(&TaskKind::Push) {
      Some(TaskKind::Push.loading_label())
    } else if self.running.contains(&TaskKind::EditWorktree) {
      Some(TaskKind::EditWorktree.loading_label())
    } else {
      None
    }
  }

  /// The loader label for an in-flight task, if any. `None` when nothing
  /// is loading.
  pub fn loading_label(&self) -> Option<&'static str> {
    self.running.iter().next().map(|kind| kind.loading_label())
  }
}
