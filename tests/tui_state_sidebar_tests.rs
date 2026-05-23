//! Unit tests for the pure `SidebarState` sub-struct (issue #127, part
//! 5/6 of #102).
//!
//! Exercises the sidebar visibility / focus / scroll / cache state in
//! isolation — `SidebarState` owns `open`, `focused`, `scroll`,
//! `max_scroll`, and the `cache` of pre-rendered sections. The `App`
//! orchestrator owns the side-effecting wrappers (status bar updates,
//! `refresh_link()` after navigation); this module's tests pin the
//! pure-state contract.
//!
//! Navigation contract (the load-bearing reason for the extraction —
//! the previous `App` had 4+ verbatim repetitions of the
//! `sidebar_scroll = 0; invalidate_sidebar_cache();` pair across
//! `next`, `prev`, `first`, `last`, see #102):
//!
//! - `on_navigation()` resets `scroll` to 0 AND drops the cached
//!   sections so the next render recomputes against the freshly
//!   selected worktree. Callers in `App` pair it with `refresh_link()`
//!   in a single `App::on_navigation()` wrapper so the triple cannot
//!   drift back into duplicated literals.

use gwm::tui::state::sidebar::SidebarState;
use gwm::tui::SidebarSections;
use std::path::PathBuf;

// ---- Construction ---------------------------------------------------------

#[test]
fn default_state_is_open_unfocused_zero_scroll() {
  // Matches the previous `App::new_at` defaults verbatim so the
  // extraction is observably a no-op for the renderer.
  let s = SidebarState::new();
  assert!(s.open, "sidebar defaults to open (renderer hides on narrow terminals)");
  assert!(!s.focused, "focus defaults to the worktree list");
  assert_eq!(s.scroll, 0);
  assert_eq!(s.max_scroll, 0);
  assert!(s.cache.is_none(), "cache starts cold");
}

// ---- Navigation contract --------------------------------------------------

#[test]
fn on_navigation_resets_scroll_to_zero() {
  let mut s = SidebarState::new();
  s.max_scroll = 10;
  s.scroll = 5;
  s.on_navigation();
  assert_eq!(s.scroll, 0, "on_navigation must reset scroll to top");
}

#[test]
fn on_navigation_invalidates_cache() {
  let mut s = SidebarState::new();
  s.cache = Some((PathBuf::from("/tmp/x"), SidebarSections::default()));
  s.on_navigation();
  assert!(
    s.cache.is_none(),
    "on_navigation must drop the cached sections so the new selection re-renders"
  );
}

#[test]
fn on_navigation_does_not_touch_open_or_focused() {
  // Navigation moves selection within the existing layout; it must NOT
  // toggle sidebar visibility or focus. Only the dedicated toggle
  // methods do that.
  let mut s = SidebarState::new();
  s.open = false;
  s.focused = true; // contrived but exercises the invariant
  s.on_navigation();
  assert!(!s.open, "on_navigation must not flip the open flag");
  assert!(s.focused, "on_navigation must not flip the focused flag");
}

// ---- Scroll API -----------------------------------------------------------

#[test]
fn scroll_down_clamps_at_max() {
  let mut s = SidebarState::new();
  s.max_scroll = 3;
  s.scroll_down();
  s.scroll_down();
  s.scroll_down();
  assert_eq!(s.scroll, 3);
  s.scroll_down();
  assert_eq!(s.scroll, 3, "scroll_down beyond max_scroll must clamp");
}

#[test]
fn scroll_down_with_zero_max_stays_at_zero() {
  // The renderer publishes max_scroll = 0 when the sidebar isn't shown
  // or there's no scrollable content. Scrolling must be a no-op.
  let mut s = SidebarState::new();
  assert_eq!(s.max_scroll, 0);
  s.scroll_down();
  assert_eq!(s.scroll, 0);
}

#[test]
fn scroll_up_saturates_at_zero() {
  let mut s = SidebarState::new();
  s.scroll_up();
  assert_eq!(s.scroll, 0, "scroll_up from 0 must stay at 0 (no underflow)");
}

#[test]
fn scroll_up_after_scroll_down_returns_to_zero() {
  let mut s = SidebarState::new();
  s.max_scroll = 5;
  s.scroll_down();
  s.scroll_down();
  assert_eq!(s.scroll, 2);
  s.scroll_up();
  s.scroll_up();
  assert_eq!(s.scroll, 0);
  s.scroll_up();
  assert_eq!(s.scroll, 0, "subsequent scroll_up still saturates");
}

// ---- Visibility / focus invariants ----------------------------------------

#[test]
fn toggle_open_flips_the_flag() {
  let mut s = SidebarState::new();
  let before = s.open;
  s.toggle_open();
  assert_eq!(s.open, !before);
  s.toggle_open();
  assert_eq!(s.open, before);
}

#[test]
fn toggle_open_when_closing_drops_focus() {
  // A hidden sidebar can't be focused — closing it must drop focus
  // back to the list so subsequent `j` / `k` walks the worktree table.
  let mut s = SidebarState::new();
  s.focused = true;
  s.open = true;
  s.toggle_open();
  assert!(!s.open);
  assert!(!s.focused, "closing the sidebar must clear the focus flag");
}

#[test]
fn toggle_focus_is_a_noop_when_closed() {
  let mut s = SidebarState::new();
  s.open = false;
  s.toggle_focus();
  assert!(!s.focused, "focus cannot move to a hidden sidebar");
}

#[test]
fn toggle_focus_flips_when_open() {
  let mut s = SidebarState::new();
  s.open = true;
  s.toggle_focus();
  assert!(s.focused);
  s.toggle_focus();
  assert!(!s.focused);
}

// ---- Explicit cache flush -------------------------------------------------

#[test]
fn invalidate_drops_cache_keeps_scroll() {
  // `invalidate()` is the standalone cache flush used outside the
  // navigation path (e.g. `filter_push_char` re-narrows the visible
  // set but doesn't move the cursor — scroll state must survive).
  let mut s = SidebarState::new();
  s.cache = Some((PathBuf::from("/tmp/x"), SidebarSections::default()));
  s.scroll = 4;
  s.max_scroll = 10;
  s.invalidate();
  assert!(s.cache.is_none());
  assert_eq!(s.scroll, 4, "plain invalidate must NOT touch scroll");
  assert_eq!(s.max_scroll, 10, "plain invalidate must NOT touch max_scroll");
}
