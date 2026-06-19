//! State-machine tests for the exec profile picker overlay (issue #325).
//!
//! The picker is pure state — no PTY, no config beyond the sorted
//! profile-name list handed in at open time — so these run ratatui-free
//! on every platform.

use gwm::tui::ExecPicker;
use std::path::PathBuf;

fn names(v: &[&str]) -> Vec<String> {
  v.iter().map(|s| s.to_string()).collect()
}

/// A stand-in worktree path captured at open time.
fn wt() -> PathBuf {
  PathBuf::from("/tmp/gwm-test/wt")
}

#[test]
fn opens_empty_by_default() {
  let p = ExecPicker::new();
  assert!(p.is_empty());
  assert_eq!(p.selected_profile(), None);
  assert!(p.profiles().is_empty());
}

#[test]
fn open_populates_and_resets_highlight_to_top() {
  let mut p = ExecPicker::new();
  p.open(names(&["build", "test", "lint"]), wt());
  assert!(!p.is_empty());
  assert_eq!(p.profiles().len(), 3);
  assert_eq!(p.selected_index(), 0);
  assert_eq!(p.selected_profile(), Some("build"));
  // The target worktree is captured at open so Enter runs there even if the
  // live selection drifts (#333).
  assert_eq!(p.cwd(), Some(wt().as_path()));
}

#[test]
fn next_advances_then_wraps_to_top() {
  let mut p = ExecPicker::new();
  p.open(names(&["build", "test"]), wt());
  p.next();
  assert_eq!(p.selected_profile(), Some("test"));
  p.next(); // wraps back to the first row
  assert_eq!(p.selected_profile(), Some("build"));
}

#[test]
fn prev_retreats_then_wraps_to_bottom() {
  let mut p = ExecPicker::new();
  p.open(names(&["build", "test", "lint"]), wt());
  p.prev(); // wraps to the last row
  assert_eq!(p.selected_profile(), Some("lint"));
  p.prev();
  assert_eq!(p.selected_profile(), Some("test"));
}

#[test]
fn navigation_is_noop_when_empty() {
  let mut p = ExecPicker::new();
  p.next();
  p.prev();
  assert!(p.is_empty());
  assert_eq!(p.selected_index(), 0);
  assert_eq!(p.selected_profile(), None);
}

#[test]
fn reopen_resets_highlight_and_replaces_profiles() {
  let mut p = ExecPicker::new();
  p.open(names(&["a", "b", "c"]), wt());
  p.next();
  p.next();
  assert_eq!(p.selected_index(), 2);
  p.open(names(&["x", "y"]), wt());
  assert_eq!(p.selected_index(), 0);
  assert_eq!(p.profiles(), &["x".to_string(), "y".to_string()]);
  assert_eq!(p.selected_profile(), Some("x"));
}
