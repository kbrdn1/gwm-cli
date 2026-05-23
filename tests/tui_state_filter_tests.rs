//! Unit tests for the pure `FilterState` sub-struct (issue #124, closes
//! #104).
//!
//! Exercises the fuzzy-filter state machine in isolation — `FilterState`
//! owns `active` (typing-bar flag), `query` (live buffer), and the memo
//! cache for the matched-indices vec. The `App` orchestrator owns the
//! side-effecting wrappers (status bar updates, selection clamping,
//! sidebar cache invalidation); this module's tests pin the pure
//! buffer + cache contract.
//!
//! Memoisation contract (the load-bearing reason for the extraction —
//! `App::filtered_indices` used to recompute 3–5× per frame, see #104):
//!
//! - Two consecutive `filtered_indices(&wts, …)` calls return the same
//!   slice AND the compute closure runs exactly once.
//! - Mutating the query (push_char / pop_char / set_query / clear)
//!   invalidates the cache.
//! - Explicit `invalidate()` flushes the cache (App calls this after
//!   `refresh()` mutates the worktrees vec).
//! - A `worktrees.len()` change between calls auto-invalidates (defence
//!   in depth: if a caller mutates `App.worktrees` without remembering
//!   to call `invalidate`, the cache must not return stale indices that
//!   point past the new vec).

use gwm::tui::state::filter::{fuzzy_match_indices, FilterState};
use gwm::worktree::{BranchStatus, WorktreeInfo};
use std::cell::Cell;
use std::path::PathBuf;

fn wt(name: &str) -> WorktreeInfo {
  WorktreeInfo {
    name: name.into(),
    path: PathBuf::from(format!("/tmp/{}", name)),
    branch: None,
    head: None,
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
    link: gwm::github::BranchLink::empty(),
    age: None,
  }
}

// ---- Buffer API ------------------------------------------------------------

#[test]
fn default_state_is_inactive_empty_buffer() {
  let f = FilterState::new();
  assert!(!f.active);
  assert!(f.query.is_empty());
}

#[test]
fn push_char_appends_to_buffer() {
  let mut f = FilterState::new();
  for c in "tui".chars() {
    f.push_char(c);
  }
  assert_eq!(f.query, "tui");
}

#[test]
fn pop_char_removes_last_character() {
  let mut f = FilterState::new();
  f.set_query("tuix".into());
  f.pop_char();
  assert_eq!(f.query, "tui");
}

#[test]
fn pop_char_on_empty_is_noop() {
  let mut f = FilterState::new();
  f.pop_char();
  assert!(f.query.is_empty());
}

#[test]
fn open_sets_active_preserves_query() {
  let mut f = FilterState::new();
  f.set_query("auth".into());
  f.open();
  assert!(f.active);
  assert_eq!(f.query, "auth", "open must preserve the existing query for refinement");
}

#[test]
fn close_keep_disables_active_keeps_query() {
  let mut f = FilterState::new();
  f.open();
  f.set_query("auth".into());
  f.close_keep();
  assert!(!f.active);
  assert_eq!(f.query, "auth", "close_keep (Enter) is the sticky-filter path");
}

#[test]
fn close_cancel_clears_buffer_and_disables_active() {
  let mut f = FilterState::new();
  f.open();
  f.set_query("auth".into());
  f.close_cancel();
  assert!(!f.active);
  assert!(f.query.is_empty(), "close_cancel (Esc) is the clear-everything path");
}

#[test]
fn clear_drops_buffer_and_active() {
  let mut f = FilterState::new();
  f.open();
  f.set_query("auth".into());
  f.clear();
  assert!(!f.active);
  assert!(f.query.is_empty());
}

// ---- Memoisation contract --------------------------------------------------

/// Build a closure that counts how many times the inner compute fn runs,
/// so the test can assert the cache hit/miss path. The closure has to
/// take `&str + &[WorktreeInfo]` because that's the public `compute`
/// shape of `FilterState::filtered_indices`.
fn counting_compute<'a>(counter: &'a Cell<usize>) -> impl FnOnce(&str, &[WorktreeInfo]) -> Vec<usize> + 'a {
  move |q: &str, wts: &[WorktreeInfo]| {
    counter.set(counter.get() + 1);
    fuzzy_match_indices(q, wts)
  }
}

#[test]
fn filtered_indices_caches_between_consecutive_calls() {
  // The whole point of #104: two reads of `filtered_indices` in the same
  // frame must run the compute closure exactly once.
  let wts = vec![wt("alpha"), wt("beta"), wt("gamma")];
  let mut f = FilterState::new();
  f.set_query("a".into());

  let counter = Cell::new(0usize);
  let first: Vec<usize> = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  let second: Vec<usize> = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();

  assert_eq!(first, second, "cached read must return the same indices");
  assert_eq!(
    counter.get(),
    1,
    "compute closure must run exactly once across two reads"
  );
}

#[test]
fn filtered_indices_recomputes_after_set_query() {
  let wts = vec![wt("alpha"), wt("beta")];
  let mut f = FilterState::new();
  f.set_query("alp".into());

  let counter = Cell::new(0usize);
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 1);

  // Mutating the query must invalidate the cache.
  f.set_query("bet".into());
  let after: Vec<usize> = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 2, "set_query must invalidate the cache");
  assert_eq!(after, vec![1], "recomputed indices must match 'bet' → ['beta']");
}

#[test]
fn filtered_indices_recomputes_after_push_char() {
  let wts = vec![wt("alpha"), wt("beta")];
  let mut f = FilterState::new();
  f.set_query("a".into());

  let counter = Cell::new(0usize);
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 1);

  f.push_char('l');
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 2, "push_char must invalidate the cache");
}

#[test]
fn filtered_indices_recomputes_after_pop_char() {
  let wts = vec![wt("alpha"), wt("beta")];
  let mut f = FilterState::new();
  f.set_query("alp".into());

  let counter = Cell::new(0usize);
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 1);

  f.pop_char();
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 2, "pop_char must invalidate the cache");
}

#[test]
fn explicit_invalidate_forces_recompute() {
  let wts = vec![wt("alpha"), wt("beta")];
  let mut f = FilterState::new();
  f.set_query("a".into());

  let counter = Cell::new(0usize);
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 1);

  f.invalidate();
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 2, "explicit invalidate must flush the cache");
}

#[test]
fn filtered_indices_recomputes_when_worktrees_len_changes() {
  // Defence in depth: if the caller mutates `App.worktrees` without
  // calling `invalidate()`, the cache must auto-detect the staleness so
  // it can never return indices that point past the new vec.
  let wts = vec![wt("alpha"), wt("beta")];
  let mut f = FilterState::new();
  f.set_query("a".into());

  let counter = Cell::new(0usize);
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 1);

  // Caller adds a worktree without remembering to invalidate.
  let mut grown = wts.clone();
  grown.push(wt("apex"));
  let recomputed: Vec<usize> = f.filtered_indices(&grown, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 2, "len mismatch must auto-invalidate the cache");
  // The new worktree at index 2 fuzzy-matches 'a' too, so the recompute
  // must surface it (proving the new vec actually went through the
  // matcher, not the stale cache).
  assert!(
    recomputed.contains(&2),
    "freshly-recomputed indices must reflect the grown vec"
  );
}

#[test]
fn clear_invalidates_cache() {
  let wts = vec![wt("alpha"), wt("beta")];
  let mut f = FilterState::new();
  f.set_query("a".into());

  let counter = Cell::new(0usize);
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 1);

  f.clear();
  let _ = f.filtered_indices(&wts, counting_compute(&counter)).to_vec();
  assert_eq!(counter.get(), 2, "clear must invalidate the cache");
}

// ---- Pure fuzzy matcher (free fn) ------------------------------------------

#[test]
fn fuzzy_match_indices_empty_query_is_identity() {
  let wts = vec![wt("alpha"), wt("beta"), wt("gamma")];
  assert_eq!(fuzzy_match_indices("", &wts), vec![0, 1, 2]);
}

#[test]
fn fuzzy_match_indices_skips_non_matches() {
  let wts = vec![wt("alpha"), wt("beta")];
  assert!(fuzzy_match_indices("zzzz", &wts).is_empty());
}
