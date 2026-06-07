//! Unit tests for the pure `GitHubFetch` sub-struct (issue #128, part
//! 6/6 of the `tui::app::App` decomposition #102; threading migrated onto
//! the async-task spine in #255).
//!
//! Post-#255 `GitHubFetch` is a *result cache* + link state: it owns
//! `link`, `link_slug`, and the per-(target, number) `issue_cache` /
//! `pr_cache`. The off-thread coalescing, the inflight dedupe, and the
//! late-result drop it used to hold are now the generic
//! `state::async_task::TaskRunner` spine (covered by
//! `tui_state_async_task_tests.rs`). These tests pin what the cache still
//! owns:
//!
//! - `mark_loading(key)` flips the per-key entry to `Loading`.
//! - `complete_{issue,pr}(n, result)` stamps the terminal `Loaded` /
//!   `Error` variant (a pure write — the drop decision lives on the spine).
//! - `is_cached(key)` is `true` only for a terminal variant, and the
//!   `(target, number)` tuple is the cache identity: `Issue(42)`,
//!   `Pr(42)`, and `Issue(43)` are all independent (the #138 keying).
//! - `invalidate()` clears the cache.

use gwm::github::{IssueState, IssueStatus, PrState, PrStatus};
use gwm::tui::state::github_fetch::{FetchKey, GitHubFetch, GitHubFetchState};

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

// ---- mark_loading flips the per-key entry to Loading ----------------------

#[test]
fn mark_loading_flips_issue_to_loading() {
  let mut gh = GitHubFetch::new();
  assert!(matches!(gh.issue_fetch_state(42), GitHubFetchState::Idle));
  gh.mark_loading(FetchKey::Issue(42));
  assert!(matches!(gh.issue_fetch_state(42), GitHubFetchState::Loading));
}

#[test]
fn mark_loading_flips_pr_to_loading() {
  let mut gh = GitHubFetch::new();
  gh.mark_loading(FetchKey::Pr(7));
  assert!(matches!(gh.pr_fetch_state(7), GitHubFetchState::Loading));
}

// ---- is_cached is true only for a terminal variant ------------------------

#[test]
fn is_cached_is_false_on_a_cold_cache() {
  let gh = GitHubFetch::new();
  assert!(!gh.is_cached(FetchKey::Issue(42)));
  assert!(!gh.is_cached(FetchKey::Pr(7)));
}

#[test]
fn is_cached_is_false_while_loading() {
  // A Loading entry is not terminal — the `App` must still let the worker
  // finish, not short-circuit on a half-populated slot.
  let mut gh = GitHubFetch::new();
  gh.mark_loading(FetchKey::Issue(42));
  assert!(
    !gh.is_cached(FetchKey::Issue(42)),
    "a Loading entry must not count as cached"
  );
}

#[test]
fn complete_issue_stamps_loaded_and_marks_cached() {
  let mut gh = GitHubFetch::new();
  gh.mark_loading(FetchKey::Issue(42));
  gh.complete_issue(42, Ok(sample_issue(42)));
  assert!(matches!(gh.issue_fetch_state(42), GitHubFetchState::Loaded(_)));
  assert!(gh.is_cached(FetchKey::Issue(42)));
}

#[test]
fn complete_pr_stamps_loaded_and_marks_cached() {
  let mut gh = GitHubFetch::new();
  gh.mark_loading(FetchKey::Pr(7));
  gh.complete_pr(7, Ok(sample_pr(7)));
  assert!(matches!(gh.pr_fetch_state(7), GitHubFetchState::Loaded(_)));
  assert!(gh.is_cached(FetchKey::Pr(7)));
}

#[test]
fn an_errored_complete_is_still_terminal_and_cached() {
  // An errored fetch is still "completed" — re-shelling out on every visit
  // event after a hard `gh` failure would be a noise amplifier. Cache the
  // error; the explicit `F` (refresh) key bypasses via `invalidate()`.
  let mut gh = GitHubFetch::new();
  gh.mark_loading(FetchKey::Issue(42));
  gh.complete_issue(42, Err("gh: connection refused".into()));
  assert!(matches!(gh.issue_fetch_state(42), GitHubFetchState::Error(_)));
  assert!(gh.is_cached(FetchKey::Issue(42)));
}

// ---- (target, number) is the cache identity (#138 keying) -----------------

#[test]
fn issue_and_pr_with_same_number_are_independent_in_the_cache() {
  // Same number, different target — completing one must not warm the other.
  let mut gh = GitHubFetch::new();
  gh.complete_issue(42, Ok(sample_issue(42)));
  assert!(gh.is_cached(FetchKey::Issue(42)));
  assert!(
    !gh.is_cached(FetchKey::Pr(42)),
    "Issue(42) and Pr(42) must not share a cache slot"
  );
}

#[test]
fn caching_one_issue_number_does_not_warm_another() {
  // Pre-#138 the cache was a single per-target slot, so any terminal
  // Issue(_) made every Issue(*) falsely hit. The cache is keyed by
  // (target, number), so Issue(43) stays cold after completing Issue(42).
  let mut gh = GitHubFetch::new();
  gh.complete_issue(42, Ok(sample_issue(42)));
  assert!(gh.is_cached(FetchKey::Issue(42)));
  assert!(
    !gh.is_cached(FetchKey::Issue(43)),
    "Issue(43) was never fetched — the cache identity must include the number (bug #138)"
  );
}

#[test]
fn caching_one_pr_number_does_not_warm_another() {
  let mut gh = GitHubFetch::new();
  gh.complete_pr(7, Ok(sample_pr(7)));
  assert!(gh.is_cached(FetchKey::Pr(7)));
  assert!(
    !gh.is_cached(FetchKey::Pr(8)),
    "Pr(8) was never fetched — the cache identity must include the number (bug #138)"
  );
}

// ---- invalidate clears the cache ------------------------------------------

#[test]
fn invalidate_clears_the_cache() {
  // Simulates `refresh_link` after the user navigates to a different
  // worktree: the cached (issue, pr) tuple is no longer authoritative.
  let mut gh = GitHubFetch::new();
  gh.complete_issue(42, Ok(sample_issue(42)));
  gh.complete_pr(7, Ok(sample_pr(7)));
  assert!(gh.is_cached(FetchKey::Issue(42)));
  assert!(gh.is_cached(FetchKey::Pr(7)));

  gh.invalidate();

  assert!(!gh.is_cached(FetchKey::Issue(42)));
  assert!(!gh.is_cached(FetchKey::Pr(7)));
  assert!(matches!(gh.issue_fetch_state(42), GitHubFetchState::Idle));
  assert!(matches!(gh.pr_fetch_state(7), GitHubFetchState::Idle));
}

// ---- PR auto-detection on the slice (issue #181) --------------------------

#[test]
fn apply_detected_pr_fills_empty_pr_slot_with_detected_source() {
  let mut gh = GitHubFetch::new();
  // Cold link: no PR.
  assert_eq!(gh.link.pr, None);

  gh.apply_detected_pr(Some(128));

  assert_eq!(gh.link.pr, Some(128));
  assert_eq!(gh.link.pr_source, gwm::github::LinkSource::Detected);
}

#[test]
fn apply_detected_pr_does_not_override_explicit_pr() {
  let mut gh = GitHubFetch::new();
  gh.link.pr = Some(61);
  gh.link.pr_source = gwm::github::LinkSource::Explicit;

  gh.apply_detected_pr(Some(128));

  assert_eq!(gh.link.pr, Some(61));
  assert_eq!(gh.link.pr_source, gwm::github::LinkSource::Explicit);
}

#[test]
fn apply_detected_pr_is_noop_when_nothing_detected() {
  let mut gh = GitHubFetch::new();
  gh.apply_detected_pr(None);
  assert_eq!(gh.link.pr, None);
  assert_eq!(gh.link.pr_source, gwm::github::LinkSource::None);
}

#[test]
fn clear_detected_pr_drops_a_detected_link_so_it_re_resolves() {
  let mut gh = GitHubFetch::new();
  gh.apply_detected_pr(Some(128));
  assert_eq!(gh.link.pr_source, gwm::github::LinkSource::Detected);

  gh.clear_detected_pr();

  assert_eq!(gh.link.pr, None);
  assert_eq!(gh.link.pr_source, gwm::github::LinkSource::None);
}

#[test]
fn clear_detected_pr_leaves_an_explicit_link_pinned() {
  let mut gh = GitHubFetch::new();
  gh.link.pr = Some(61);
  gh.link.pr_source = gwm::github::LinkSource::Explicit;

  gh.clear_detected_pr();

  assert_eq!(gh.link.pr, Some(61));
  assert_eq!(gh.link.pr_source, gwm::github::LinkSource::Explicit);
}
