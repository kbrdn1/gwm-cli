//! Unit tests for the pure `GitHubFetch` sub-struct (issue #128, part
//! 6/6 of the `tui::app::App` decomposition #102).
//!
//! Exercises the GitHub fetch state slice in isolation — `GitHubFetch`
//! owns `link`, `link_slug`, `issue_state`, `pr_state`, and the
//! inflight-dedupe layer that closes the load-bearing payoff of #128:
//! "multiple concurrent visit events to the same target trigger
//! redundant `gh` shell-outs because there's no dedupe layer". The
//! `App` orchestrator keeps the side-effecting shell-out
//! (`gh issue view`, `gh pr view`); this module's tests pin the pure
//! state machine + dedupe contract.
//!
//! Dedupe contract:
//!
//! - `request(key)` on a cold cache returns `Spawn` and marks the key
//!   inflight so a concurrent caller can dedupe.
//! - A second `request(key)` while inflight returns `AlreadyInflight`
//!   (no redundant shell-out).
//! - `complete(key, result)` clears the inflight flag and stores the
//!   loaded / errored state on `issue_state` or `pr_state` so the
//!   next `request(key)` returns `HitCache` instead of re-spawning.
//! - Different keys (e.g. `Issue(42)` vs `Pr(42)`) never collide — the
//!   target discriminant is part of the dedupe identity.

use gwm::github::{IssueState, IssueStatus, PrState, PrStatus};
use gwm::tui::state::github_fetch::{FetchAction, FetchKey, GitHubFetch, GitHubFetchState};

fn sample_issue(n: u64) -> IssueStatus {
  IssueStatus {
    number: n,
    title: format!("issue #{}", n),
    state: IssueState::Open,
    url: format!("https://example.test/issues/{}", n),
    labels: vec![],
    updated_at: "2026-01-01T00:00:00Z".into(),
  }
}

fn sample_pr(n: u64) -> PrStatus {
  PrStatus {
    number: n,
    title: format!("pr #{}", n),
    state: PrState::Open,
    url: format!("https://example.test/pull/{}", n),
    updated_at: "2026-01-01T00:00:00Z".into(),
    checks_passed: 0,
    checks_total: 0,
  }
}

// ---- Cold cache returns Spawn ---------------------------------------------

#[test]
fn request_on_cold_cache_returns_spawn_for_issue() {
  let mut gh = GitHubFetch::new();
  let action = gh.request(FetchKey::Issue(42));
  assert!(matches!(action, FetchAction::Spawn(FetchKey::Issue(42))));
  // And the per-target `*_state` flips to `Loading` so the UI can paint
  // an in-flight badge — same contract the pre-extraction
  // `refresh_github_status` used.
  assert!(matches!(gh.issue_state, GitHubFetchState::Loading));
}

#[test]
fn request_on_cold_cache_returns_spawn_for_pr() {
  let mut gh = GitHubFetch::new();
  let action = gh.request(FetchKey::Pr(7));
  assert!(matches!(action, FetchAction::Spawn(FetchKey::Pr(7))));
  assert!(matches!(gh.pr_state, GitHubFetchState::Loading));
}

// ---- Dedupe: second request while inflight returns AlreadyInflight --------

#[test]
fn second_request_while_inflight_returns_already_inflight() {
  let mut gh = GitHubFetch::new();
  let first = gh.request(FetchKey::Issue(42));
  assert!(matches!(first, FetchAction::Spawn(_)));

  // Concurrent visit event hits the same key before completion. Without
  // dedupe, this would trigger a redundant `gh issue view 42 …` — the
  // load-bearing payoff of #128 is that it doesn't.
  let second = gh.request(FetchKey::Issue(42));
  assert!(
    matches!(second, FetchAction::AlreadyInflight),
    "expected AlreadyInflight on second request to the same key, got {:?}",
    second
  );
}

#[test]
fn third_request_while_inflight_still_returns_already_inflight() {
  let mut gh = GitHubFetch::new();
  assert!(matches!(gh.request(FetchKey::Pr(11)), FetchAction::Spawn(_)));
  assert!(matches!(gh.request(FetchKey::Pr(11)), FetchAction::AlreadyInflight));
  assert!(matches!(gh.request(FetchKey::Pr(11)), FetchAction::AlreadyInflight));
}

// ---- After complete: cache is warm, request hits cache --------------------

#[test]
fn after_complete_request_returns_hit_cache_for_issue() {
  let mut gh = GitHubFetch::new();
  assert!(matches!(gh.request(FetchKey::Issue(42)), FetchAction::Spawn(_)));
  gh.complete_issue(42, Ok(sample_issue(42)));
  let action = gh.request(FetchKey::Issue(42));
  assert!(
    matches!(action, FetchAction::HitCache),
    "expected HitCache after successful complete, got {:?}",
    action
  );
  // And the loaded state is observable via `issue_state`.
  assert!(matches!(gh.issue_state, GitHubFetchState::Loaded(_)));
}

#[test]
fn after_complete_request_returns_hit_cache_for_pr() {
  let mut gh = GitHubFetch::new();
  assert!(matches!(gh.request(FetchKey::Pr(7)), FetchAction::Spawn(_)));
  gh.complete_pr(7, Ok(sample_pr(7)));
  let action = gh.request(FetchKey::Pr(7));
  assert!(matches!(action, FetchAction::HitCache));
  assert!(matches!(gh.pr_state, GitHubFetchState::Loaded(_)));
}

#[test]
fn after_errored_complete_request_returns_hit_cache() {
  // An errored fetch is still "completed" — re-shelling out on every
  // visit event after a hard `gh` failure would be a noise amplifier,
  // not a fix. Cache the error, let the explicit `F` (refresh) key
  // bypass via `invalidate()`.
  let mut gh = GitHubFetch::new();
  assert!(matches!(gh.request(FetchKey::Issue(42)), FetchAction::Spawn(_)));
  gh.complete_issue(42, Err("gh: connection refused".into()));
  let action = gh.request(FetchKey::Issue(42));
  assert!(matches!(action, FetchAction::HitCache));
  assert!(matches!(gh.issue_state, GitHubFetchState::Error(_)));
}

// ---- Different keys (Issue 42 vs PR 42) don't collide ---------------------

#[test]
fn issue_and_pr_with_same_number_do_not_collide_in_dedupe() {
  let mut gh = GitHubFetch::new();
  // Issue 42 goes inflight.
  let issue_action = gh.request(FetchKey::Issue(42));
  assert!(matches!(issue_action, FetchAction::Spawn(_)));
  // PR 42 — same number, different target — must still Spawn. If the
  // dedupe key only hashed the number, this would wrongly return
  // AlreadyInflight and the PR fetch would never fire.
  let pr_action = gh.request(FetchKey::Pr(42));
  assert!(
    matches!(pr_action, FetchAction::Spawn(_)),
    "Issue(42) and Pr(42) must not collide in dedupe, got {:?}",
    pr_action
  );
}

#[test]
fn completing_issue_does_not_clear_inflight_pr() {
  let mut gh = GitHubFetch::new();
  assert!(matches!(gh.request(FetchKey::Issue(42)), FetchAction::Spawn(_)));
  assert!(matches!(gh.request(FetchKey::Pr(42)), FetchAction::Spawn(_)));
  // Completing the issue must NOT clear the PR's inflight flag.
  gh.complete_issue(42, Ok(sample_issue(42)));
  let pr_action = gh.request(FetchKey::Pr(42));
  assert!(
    matches!(pr_action, FetchAction::AlreadyInflight),
    "complete_issue must not free Pr(42)'s inflight slot, got {:?}",
    pr_action
  );
}

#[test]
fn different_issue_numbers_do_not_collide() {
  let mut gh = GitHubFetch::new();
  assert!(matches!(gh.request(FetchKey::Issue(42)), FetchAction::Spawn(_)));
  // A different issue # must Spawn — dedupe is per-(target, number),
  // not just per-target.
  assert!(matches!(gh.request(FetchKey::Issue(43)), FetchAction::Spawn(_)));
}

// ---- complete() clears the inflight slot so a follow-up request is Spawn
//      after explicit invalidation ----------------------------------------

#[test]
fn complete_clears_inflight_slot() {
  let mut gh = GitHubFetch::new();
  assert!(matches!(gh.request(FetchKey::Issue(42)), FetchAction::Spawn(_)));
  gh.complete_issue(42, Ok(sample_issue(42)));
  // Cache is warm — a `request` returns HitCache, not AlreadyInflight.
  // The inflight slot itself was cleared by `complete`.
  let action = gh.request(FetchKey::Issue(42));
  assert!(matches!(action, FetchAction::HitCache));
  // Invalidate (simulates `refresh_link` after the user navigates to
  // a different worktree) and request again — should Spawn, not
  // AlreadyInflight (the slot was correctly freed by `complete`).
  gh.invalidate();
  let action = gh.request(FetchKey::Issue(42));
  assert!(
    matches!(action, FetchAction::Spawn(_)),
    "after complete + invalidate, request should Spawn (inflight slot was freed by complete), got {:?}",
    action
  );
}
