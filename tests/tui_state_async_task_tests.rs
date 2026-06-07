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
