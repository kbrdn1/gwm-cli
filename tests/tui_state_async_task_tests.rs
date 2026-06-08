//! Unit tests for the pure `TaskRunner` spine (issue #231).
//!
//! `TaskRunner` is the generalised off-thread spine extracted from the
//! GitHub fetch (#217). Unlike `state::github_fetch` (a per-key result
//! *cache* + dedupe), this is a *coalescing + late-drop* layer for
//! one-shot ops that always want a fresh result:
//!
//! - `request(kind)` on a cold slot returns `Some(generation)` (the
//!   caller spawns a worker tagged with that generation); a second
//!   `request` while a run is in flight returns `None` (coalesced — no
//!   second worker).
//! - `complete(kind, generation)` returns `true` only while the
//!   generation is still authoritative; a late result whose generation
//!   was bumped by an intervening `invalidate`/`request` returns `false`
//!   and is dropped (the #138 stale-result guard, generalised to
//!   non-keyed ops).
//! - `invalidate(kind)` bumps the generation and frees the slot.

use gwm::tui::state::async_task::{TaskKind, TaskRunner};

#[test]
fn request_on_cold_slot_returns_a_generation_and_marks_loading() {
  let mut runner = TaskRunner::new();
  assert!(!runner.is_loading(TaskKind::RefreshWorktrees));
  let gen = runner.request(TaskKind::RefreshWorktrees);
  assert_eq!(gen, Some(1), "first request claims generation 1");
  assert!(runner.is_loading(TaskKind::RefreshWorktrees));
  assert!(runner.is_any_loading());
}

#[test]
fn second_request_while_inflight_is_coalesced() {
  let mut runner = TaskRunner::new();
  assert_eq!(runner.request(TaskKind::RefreshWorktrees), Some(1));
  // A run is already in flight → no second worker is spawned.
  assert_eq!(
    runner.request(TaskKind::RefreshWorktrees),
    None,
    "a concurrent request must coalesce onto the in-flight run"
  );
}

#[test]
fn complete_with_matching_generation_applies_and_clears_loading() {
  let mut runner = TaskRunner::new();
  let gen = runner.request(TaskKind::RefreshWorktrees).unwrap();
  assert!(
    runner.complete(TaskKind::RefreshWorktrees, gen),
    "a result tagged with the live generation is authoritative"
  );
  assert!(
    !runner.is_loading(TaskKind::RefreshWorktrees),
    "completing the run clears the loading slot"
  );
  assert!(!runner.is_any_loading());
}

#[test]
fn late_result_after_invalidate_is_dropped() {
  // The #138 guard generalised: spawn gen 1, invalidate (bumps to gen 2),
  // then a worker from gen 1 reports back → dropped, not applied.
  let mut runner = TaskRunner::new();
  let stale = runner.request(TaskKind::RefreshWorktrees).unwrap();
  runner.invalidate(TaskKind::RefreshWorktrees);
  assert!(
    !runner.is_loading(TaskKind::RefreshWorktrees),
    "invalidate frees the slot"
  );
  assert!(
    !runner.complete(TaskKind::RefreshWorktrees, stale),
    "a result from the invalidated generation must be dropped"
  );
}

#[test]
fn invalidate_frees_the_slot_for_a_fresh_run_with_a_new_generation() {
  let mut runner = TaskRunner::new();
  let first = runner.request(TaskKind::RefreshWorktrees).unwrap();
  runner.invalidate(TaskKind::RefreshWorktrees);
  let second = runner
    .request(TaskKind::RefreshWorktrees)
    .expect("slot is free after invalidate");
  assert!(second > first, "a fresh run claims a newer generation");
  // The new run is authoritative; the stale one is not.
  assert!(!runner.complete(TaskKind::RefreshWorktrees, first));
  assert!(runner.complete(TaskKind::RefreshWorktrees, second));
}

#[test]
fn complete_on_a_kind_never_requested_is_dropped() {
  let mut runner = TaskRunner::new();
  assert!(
    !runner.complete(TaskKind::RefreshWorktrees, 1),
    "no run was ever claimed → nothing to apply"
  );
}

#[test]
fn loading_label_surfaces_the_inflight_task_label() {
  let mut runner = TaskRunner::new();
  assert_eq!(runner.loading_label(), None);
  runner.request(TaskKind::RefreshWorktrees);
  assert_eq!(runner.loading_label(), Some("refreshing worktrees…"));
}

// ---- GitHub fetch kinds on the spine (issue #255) -------------------------

#[test]
fn github_kinds_report_the_shared_fetch_label() {
  // Every GitHub fetch reads as the same statusbar copy regardless of which
  // issue / PR number it carries, mirroring the pre-migration "fetching
  // GitHub status…" line.
  assert_eq!(TaskKind::GithubIssue(42).loading_label(), "fetching GitHub status…");
  assert_eq!(TaskKind::GithubPr(7).loading_label(), "fetching GitHub status…");
}

#[test]
fn is_github_is_true_only_for_github_kinds() {
  assert!(TaskKind::GithubIssue(1).is_github());
  assert!(TaskKind::GithubPr(1).is_github());
  assert!(!TaskKind::RefreshWorktrees.is_github());
}

#[test]
fn is_mutating_is_true_only_for_tasks_that_change_worktrees() {
  assert!(TaskKind::Sync.is_mutating());
  assert!(TaskKind::Bootstrap.is_mutating());
  assert!(!TaskKind::RefreshWorktrees.is_mutating());
  assert!(!TaskKind::GithubIssue(1).is_mutating());
  assert!(!TaskKind::GithubPr(1).is_mutating());
}

#[test]
fn runner_reports_when_a_mutating_task_is_in_flight() {
  let mut runner = TaskRunner::new();
  runner.request(TaskKind::RefreshWorktrees);
  runner.request(TaskKind::GithubIssue(42));

  assert!(runner.is_any_loading());
  assert!(!runner.has_mutating_task_in_flight());

  runner.request(TaskKind::Sync);
  assert!(runner.has_mutating_task_in_flight());
}

#[test]
fn mutating_loading_label_prefers_mutating_work_over_read_only_work() {
  let mut runner = TaskRunner::new();
  runner.request(TaskKind::RefreshWorktrees);
  runner.request(TaskKind::GithubIssue(42));
  runner.request(TaskKind::Sync);

  assert_eq!(runner.mutating_loading_label(), Some("syncing…"));
}

#[test]
fn runner_stops_reporting_mutating_work_once_it_completes() {
  let mut runner = TaskRunner::new();
  let generation = runner.request(TaskKind::Bootstrap).unwrap();

  assert!(runner.has_mutating_task_in_flight());
  assert!(runner.complete(TaskKind::Bootstrap, generation));
  assert!(!runner.has_mutating_task_in_flight());
}

#[test]
fn github_issue_and_pr_with_same_number_are_independent_slots() {
  // The (target, number) identity carries onto the spine: Issue(42) and
  // Pr(42) never coalesce or share a generation.
  let mut runner = TaskRunner::new();
  assert_eq!(runner.request(TaskKind::GithubIssue(42)), Some(1));
  assert_eq!(
    runner.request(TaskKind::GithubPr(42)),
    Some(1),
    "Pr(42) must claim its own slot, not coalesce onto Issue(42)"
  );
  assert!(runner.is_loading(TaskKind::GithubIssue(42)));
  assert!(runner.is_loading(TaskKind::GithubPr(42)));
}

#[test]
fn invalidate_matching_drops_only_running_github_slots() {
  // The navigation path: a refresh worker and two GitHub fetches are in
  // flight; `invalidate_matching(is_github)` must drop both GitHub slots and
  // bump their generations, while leaving the refresh slot untouched.
  let mut runner = TaskRunner::new();
  runner.request(TaskKind::RefreshWorktrees);
  let stale_issue = runner.request(TaskKind::GithubIssue(42)).unwrap();
  let stale_pr = runner.request(TaskKind::GithubPr(7)).unwrap();

  runner.invalidate_matching(|k| k.is_github());

  assert!(
    runner.is_loading(TaskKind::RefreshWorktrees),
    "a non-GitHub task must survive invalidate_matching(is_github)"
  );
  assert!(!runner.is_loading(TaskKind::GithubIssue(42)), "GitHub issue slot freed");
  assert!(!runner.is_loading(TaskKind::GithubPr(7)), "GitHub pr slot freed");
  // The stale generations are now superseded — their late results drop.
  assert!(!runner.complete(TaskKind::GithubIssue(42), stale_issue));
  assert!(!runner.complete(TaskKind::GithubPr(7), stale_pr));
}

#[test]
fn a_stale_github_worker_loses_to_a_newer_generation_after_invalidate() {
  // The Codex-flagged race at the spine level: Worker A claims gen 1 for
  // Issue(42); an invalidate (navigate away/back) bumps the generation;
  // Worker B claims a fresh generation. Whatever the arrival order, A's
  // result is dropped and B's is authoritative.
  let mut runner = TaskRunner::new();
  let gen_a = runner.request(TaskKind::GithubIssue(42)).unwrap();
  runner.invalidate_matching(|k| k.is_github());
  let gen_b = runner.request(TaskKind::GithubIssue(42)).unwrap();
  assert_ne!(gen_a, gen_b, "the fresh run must own a distinct generation");

  // Stale worker A reports first — dropped.
  assert!(!runner.complete(TaskKind::GithubIssue(42), gen_a));
  // Fresh worker B reports second — applied, and clears the slot.
  assert!(runner.complete(TaskKind::GithubIssue(42), gen_b));
  assert!(!runner.is_loading(TaskKind::GithubIssue(42)));
}

// ---- Sync task on the spine (issue #258) ----------------------------------

#[test]
fn sync_task_reports_the_syncing_label() {
  assert_eq!(TaskKind::Sync.loading_label(), "syncing…");
  assert!(!TaskKind::Sync.is_github(), "Sync is not a GitHub kind");
}

#[test]
fn second_sync_request_while_inflight_is_coalesced() {
  // Only one sync runs at a time — a second `S` press while a rebase is in
  // flight must not spawn a second concurrent rebase.
  let mut runner = TaskRunner::new();
  assert_eq!(runner.request(TaskKind::Sync), Some(1));
  assert_eq!(
    runner.request(TaskKind::Sync),
    None,
    "a second sync request while one is inflight must coalesce"
  );
  assert!(runner.is_loading(TaskKind::Sync));
}

#[test]
fn a_late_sync_result_after_invalidate_is_dropped() {
  // The generalised #138 guard for sync: a worker whose generation was bumped
  // by an intervening invalidate (e.g. a post-sync refresh) is dropped.
  let mut runner = TaskRunner::new();
  let stale = runner.request(TaskKind::Sync).unwrap();
  runner.invalidate(TaskKind::Sync);
  assert!(!runner.is_loading(TaskKind::Sync), "invalidate frees the slot");
  assert!(
    !runner.complete(TaskKind::Sync, stale),
    "a result from the invalidated generation must be dropped"
  );
}

// ---- Bootstrap task on the spine (issue #256) -----------------------------

#[test]
fn bootstrap_task_reports_the_bootstrapping_label() {
  assert_eq!(TaskKind::Bootstrap.loading_label(), "bootstrapping…");
  assert!(!TaskKind::Bootstrap.is_github(), "Bootstrap is not a GitHub kind");
}

#[test]
fn second_bootstrap_request_while_inflight_is_coalesced() {
  // Only one bootstrap runs at a time — a second `b` press while one is in
  // flight must not spawn a second concurrent run (file copies / hooks).
  let mut runner = TaskRunner::new();
  assert_eq!(runner.request(TaskKind::Bootstrap), Some(1));
  assert_eq!(
    runner.request(TaskKind::Bootstrap),
    None,
    "a second bootstrap request while one is inflight must coalesce"
  );
  assert!(runner.is_loading(TaskKind::Bootstrap));
}

#[test]
fn a_late_bootstrap_result_after_invalidate_is_dropped() {
  // The generalised #138 guard for bootstrap: a worker whose generation was
  // bumped by an intervening invalidate is dropped rather than flipping the
  // view to a stale report.
  let mut runner = TaskRunner::new();
  let stale = runner.request(TaskKind::Bootstrap).unwrap();
  runner.invalidate(TaskKind::Bootstrap);
  assert!(!runner.is_loading(TaskKind::Bootstrap), "invalidate frees the slot");
  assert!(
    !runner.complete(TaskKind::Bootstrap, stale),
    "a result from the invalidated generation must be dropped"
  );
}
