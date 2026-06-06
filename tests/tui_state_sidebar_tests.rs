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

use gwm::config::SidebarPosition;
use gwm::tui::state::sidebar::{
  ResolvedSidebarLayout, SidebarMode, SidebarOrientation, SidebarState, SIDEBAR_MIN_WIDTH,
};
use gwm::tui::SidebarSections;
use std::path::PathBuf;

// ---- Construction ---------------------------------------------------------

#[test]
fn default_state_is_open_unfocused_zero_scroll() {
  // Matches the previous `App::new_at` defaults verbatim so the
  // extraction is observably a no-op for the renderer.
  let s = SidebarState::new();
  assert!(
    s.open,
    "sidebar defaults to open (renderer stacks it under the table on narrow terminals)"
  );
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
  s.cache = Some((
    (PathBuf::from("/tmp/x"), SidebarMode::Commits),
    SidebarSections::default(),
  ));
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

// ---- Mode toggle (issue #34) ----------------------------------------------

#[test]
fn default_mode_is_commits() {
  // The historical sidebar showed `git log --oneline` + `git status --short`.
  // Mode = Commits is the only behaviour that pre-existed, so it must
  // remain the default after the toggle lands.
  let s = SidebarState::new();
  assert_eq!(s.mode, SidebarMode::Commits);
}

#[test]
fn cycle_mode_flips_commits_to_stashes_and_back() {
  let mut s = SidebarState::new();
  s.cycle_mode();
  assert_eq!(s.mode, SidebarMode::Stashes);
  s.cycle_mode();
  assert_eq!(s.mode, SidebarMode::Commits);
}

#[test]
fn cycle_mode_resets_scroll() {
  // Switching modes presents fresh content with its own length; the
  // scroll offset from the previous mode is meaningless. Reset to 0
  // matches the on_navigation contract and avoids the user landing
  // halfway through a panel they did not scroll.
  let mut s = SidebarState::new();
  s.max_scroll = 8;
  s.scroll = 4;
  s.cycle_mode();
  assert_eq!(s.scroll, 0, "cycle_mode must reset scroll back to top");
}

#[test]
fn cache_is_keyed_by_path_and_mode() {
  // Per the issue note "Same caching pattern as commits/status mode" —
  // the cache key carries the active mode so toggling does not leak a
  // stale render across modes. Storing both modes for the same path
  // would be possible but adds memory pressure for marginal gain;
  // keying by `(path, mode)` makes a re-toggle reshell git, which is
  // documented behaviour.
  let mut s = SidebarState::new();
  let path = PathBuf::from("/tmp/wt-a");
  s.cache = Some(((path.clone(), SidebarMode::Commits), SidebarSections::default()));
  // Cycling mode invalidates the cache because the key changes.
  s.cycle_mode();
  assert!(
    s.cache.is_none(),
    "cycle_mode must drop the cached sections for the previous mode"
  );
}

#[test]
fn invalidate_drops_cache_keeps_scroll() {
  // `invalidate()` is the standalone cache flush used outside the
  // navigation path (e.g. `filter_push_char` re-narrows the visible
  // set but doesn't move the cursor — scroll state must survive).
  let mut s = SidebarState::new();
  s.cache = Some((
    (PathBuf::from("/tmp/x"), SidebarMode::Commits),
    SidebarSections::default(),
  ));
  s.scroll = 4;
  s.max_scroll = 10;
  s.invalidate();
  assert!(s.cache.is_none());
  assert_eq!(s.scroll, 4, "plain invalidate must NOT touch scroll");
  assert_eq!(s.max_scroll, 10, "plain invalidate must NOT touch max_scroll");
}

// ---- Responsive layout + position (issue #188) ----------------------------

#[test]
fn default_position_is_right_orientation_is_stacked() {
  // Sidebar on the right; default orientation is now `Stacked` (issue #217)
  // so the status pane sits under the worktrees table by default. `App`
  // overrides `position` from `[tui] sidebar_position` at construction.
  let s = SidebarState::new();
  assert_eq!(s.position, SidebarPosition::Right);
  assert_eq!(s.orientation, SidebarOrientation::Stacked);
}

#[test]
fn resolve_layout_hidden_when_closed_regardless_of_width() {
  let mut s = SidebarState::new();
  s.open = false;
  assert_eq!(s.resolve_layout(200), ResolvedSidebarLayout::Hidden);
  assert_eq!(s.resolve_layout(40), ResolvedSidebarLayout::Hidden);
}

#[test]
fn resolve_layout_auto_is_side_by_side_at_or_above_min_width() {
  let mut s = SidebarState::new(); // open, Right
  s.orientation = SidebarOrientation::Auto; // opt into width-driven layout
  assert_eq!(
    s.resolve_layout(SIDEBAR_MIN_WIDTH),
    ResolvedSidebarLayout::SideBySide { sidebar_left: false },
    "exactly at the threshold counts as wide"
  );
  assert_eq!(
    s.resolve_layout(SIDEBAR_MIN_WIDTH + 50),
    ResolvedSidebarLayout::SideBySide { sidebar_left: false }
  );
}

#[test]
fn resolve_layout_auto_stacks_below_min_width() {
  // The headline #188 change: narrow no longer hides the sidebar, it
  // stacks it under the table.
  let mut s = SidebarState::new();
  s.orientation = SidebarOrientation::Auto;
  assert_eq!(s.resolve_layout(SIDEBAR_MIN_WIDTH - 1), ResolvedSidebarLayout::Stacked);
  assert_eq!(s.resolve_layout(0), ResolvedSidebarLayout::Stacked);
}

#[test]
fn resolve_layout_auto_honours_left_position() {
  let mut s = SidebarState::new();
  s.orientation = SidebarOrientation::Auto;
  s.position = SidebarPosition::Left;
  assert_eq!(
    s.resolve_layout(SIDEBAR_MIN_WIDTH),
    ResolvedSidebarLayout::SideBySide { sidebar_left: true }
  );
  // Position is irrelevant to the stacked layout.
  assert_eq!(s.resolve_layout(SIDEBAR_MIN_WIDTH - 1), ResolvedSidebarLayout::Stacked);
}

#[test]
fn resolve_layout_forced_side_by_side_ignores_narrow_width() {
  let mut s = SidebarState::new();
  s.orientation = SidebarOrientation::SideBySide;
  assert_eq!(
    s.resolve_layout(20),
    ResolvedSidebarLayout::SideBySide { sidebar_left: false },
    "a forced side-by-side stays beside the table even when narrow"
  );
}

#[test]
fn resolve_layout_forced_stacked_ignores_wide_width() {
  let mut s = SidebarState::new();
  s.orientation = SidebarOrientation::Stacked;
  assert_eq!(
    s.resolve_layout(300),
    ResolvedSidebarLayout::Stacked,
    "a forced stack stays stacked even on a wide terminal"
  );
}

#[test]
fn cycle_orientation_walks_auto_side_by_side_stacked_and_wraps() {
  // The cycle order itself is unchanged (Auto → SideBySide → Stacked → …);
  // only the default starting point moved to `Stacked` (issue #217).
  let mut s = SidebarState::new();
  assert_eq!(s.orientation, SidebarOrientation::Stacked);
  s.cycle_orientation();
  assert_eq!(s.orientation, SidebarOrientation::Auto);
  s.cycle_orientation();
  assert_eq!(s.orientation, SidebarOrientation::SideBySide);
  s.cycle_orientation();
  assert_eq!(
    s.orientation,
    SidebarOrientation::Stacked,
    "cycle wraps back to Stacked"
  );
}

#[test]
fn toggle_position_flips_left_right() {
  let mut s = SidebarState::new();
  assert_eq!(s.position, SidebarPosition::Right);
  s.toggle_position();
  assert_eq!(s.position, SidebarPosition::Left);
  s.toggle_position();
  assert_eq!(s.position, SidebarPosition::Right);
}

#[test]
fn split_percentages_favour_the_status_pane_vertically_and_the_table_horizontally() {
  // Issue #217 ratios: stacked (vertical) gives the status pane the lion's
  // share (42% table / 58% status) so commits + issue/PR breathe; side-by-side
  // keeps the table dominant (55% / 45%). Hidden has no split.
  assert_eq!(ResolvedSidebarLayout::Stacked.split_percentages(), Some((42, 58)));
  assert_eq!(
    ResolvedSidebarLayout::SideBySide { sidebar_left: false }.split_percentages(),
    Some((55, 45))
  );
  assert_eq!(
    ResolvedSidebarLayout::SideBySide { sidebar_left: true }.split_percentages(),
    Some((55, 45)),
    "the table/sidebar ratio is independent of which side the sidebar sits on"
  );
  assert_eq!(ResolvedSidebarLayout::Hidden.split_percentages(), None);
}

#[test]
fn orientation_label_is_human_readable() {
  assert_eq!(SidebarOrientation::Auto.label(), "auto");
  assert_eq!(SidebarOrientation::SideBySide.label(), "side-by-side");
  assert_eq!(SidebarOrientation::Stacked.label(), "stacked");
}
