//! State transitions the mouse drives (issue #624).
//!
//! No terminal and no ratatui backend: the geometry is pushed into the map by
//! hand, exactly as the renderer pushes it, and what is pinned here is what
//! `App` does with a hit.

mod common;

use common::init_repo;
use gwm::tui::keymap::Action;
use gwm::tui::mouse::{MouseKind, PaneId, RowList, Spot};
use gwm::tui::{App, LinkTarget, MouseOutcome, SettingsTab, View};
use gwm::worktree::{BranchStatus, WorktreeInfo};
use ratatui::layout::Rect;
use std::path::PathBuf;

fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
  Rect {
    x,
    y,
    width: w,
    height: h,
  }
}

fn worktree_fixture(name: &str) -> WorktreeInfo {
  WorktreeInfo {
    name: name.into(),
    id: name.into(),
    path: PathBuf::from(format!("/tmp/gwm-test/{}", name)),
    branch: Some(format!("feat/#0-{}", name)),
    head: None,
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
    link: gwm::github::BranchLink::empty(),
    issue_state: None,
    pr_state: None,
    age: None,
    has_note: false,
  }
}

/// An app holding `n` worktrees with the cursor on the first row, and a table
/// strip published from y=3 the way `draw_list` publishes it.
fn app_with_rows(n: usize) -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  app.worktrees = (0..n).map(|i| worktree_fixture(&format!("wt-{i}"))).collect();
  app.filter.invalidate();
  app.list_state.select(Some(0));
  app.mouse.push_rows(rect(0, 3, 80, n as u16), RowList::Worktrees, 0, n);
  (dir, app)
}

/// The parity obligation. A click lands on an arbitrary row, so it cannot
/// reuse `next`'s arithmetic — but it must reuse its *transition*:
/// `on_navigation` is what drops the sidebar's scroll and invalidates the
/// cached git preview, and a cursor that moved without it leaves the sidebar
/// painting the worktree that used to be selected.
///
/// Driven against the same app in both directions rather than against a list
/// of cases: three `j`s and one click on row three have to leave the same
/// state, whatever that state is.
#[test]
fn a_click_leaves_the_same_state_as_walking_there_with_j() {
  let (_d1, mut clicked) = app_with_rows(6);
  let (_d2, mut walked) = app_with_rows(6);

  for _ in 0..3 {
    walked.next();
  }
  assert_eq!(clicked.handle_mouse(MouseKind::Click, 10, 6), MouseOutcome::Handled);

  assert_eq!(clicked.list_state.selected(), Some(3));
  assert_eq!(clicked.list_state.selected(), walked.list_state.selected());
  assert_eq!(clicked.sidebar.scroll, walked.sidebar.scroll);
  assert_eq!(clicked.sidebar.cache.is_none(), walked.sidebar.cache.is_none());
}

/// The half a raw `list_state.select` would skip.
#[test]
fn a_click_drops_the_sidebar_scroll_and_the_cached_preview() {
  let (_d, mut app) = app_with_rows(6);
  app.sidebar.scroll = 7;

  app.handle_mouse(MouseKind::Click, 10, 5);

  assert_eq!(app.list_state.selected(), Some(2));
  assert_eq!(app.sidebar.scroll, 0, "the preview of another worktree scrolled to 0");
  assert!(app.sidebar.cache.is_none(), "the cached preview was not invalidated");
}

#[test]
fn a_click_below_the_last_worktree_selects_nothing() {
  let (_d, mut app) = app_with_rows(3);
  // Strip published for three rows in a taller area: `draw_list` bounds the
  // strip by the item count, so y=6 is past it.
  app.mouse = Default::default();
  app.mouse.push_rows(rect(0, 3, 80, 10), RowList::Worktrees, 0, 3);

  assert_eq!(app.handle_mouse(MouseKind::Click, 10, 9), MouseOutcome::Ignored);
  assert_eq!(app.list_state.selected(), Some(0));
}

/// A scrolled table. `TableState::offset()` is what the renderer publishes,
/// and every reported index is shifted by it — the case a short list can
/// never exercise.
#[test]
fn a_click_on_a_scrolled_table_selects_the_row_that_was_drawn() {
  let (_d, mut app) = app_with_rows(200);
  app.mouse = Default::default();
  // Ten rows of viewport, scrolled so worktree 120 sits on the first one.
  app.mouse.push_rows(rect(0, 3, 80, 10), RowList::Worktrees, 120, 200);

  app.handle_mouse(MouseKind::Click, 10, 5);

  assert_eq!(app.list_state.selected(), Some(122));
}

#[test]
fn clicking_a_row_takes_the_focus_back_to_the_table() {
  let (_d, mut app) = app_with_rows(4);
  app.sidebar.open = true;
  app.sidebar.focused = true;

  app.handle_mouse(MouseKind::Click, 10, 4);

  assert!(
    !app.sidebar.focused,
    "picking a row is also a statement about which pane is meant"
  );
  assert_eq!(app.list_state.selected(), Some(1));
}

#[test]
fn clicking_a_pane_focuses_it() {
  let (_d, mut app) = app_with_rows(4);
  app.sidebar.open = true;
  app.mouse.push_pane(rect(0, 12, 80, 8), PaneId::Status);
  app.mouse.push_pane(rect(0, 1, 80, 2), PaneId::Worktrees);

  app.handle_mouse(MouseKind::Click, 10, 13);
  assert!(app.sidebar.focused, "a click on the sidebar focuses it");

  app.handle_mouse(MouseKind::Click, 10, 1);
  assert!(!app.sidebar.focused, "a click on the worktrees pane focuses it back");
}

/// The stated deviation from the issue, pinned so it is a decision rather
/// than an accident: the wheel acts on what the pointer is over, not on what
/// holds the focus.
#[test]
fn the_wheel_over_the_table_moves_the_table_even_when_the_sidebar_has_focus() {
  let (_d, mut app) = app_with_rows(6);
  app.sidebar.open = true;
  app.sidebar.focused = true;
  app.sidebar.max_scroll = 10;
  app.sidebar.scroll = 4;

  app.handle_mouse(MouseKind::WheelDown, 10, 3);

  assert_eq!(app.list_state.selected(), Some(1), "the table moved");
  // `select_row` resets the sidebar scroll, so what is pinned here is that
  // the wheel did not scroll the sidebar *instead of* moving the table.
  assert_eq!(app.sidebar.scroll, 0);
}

#[test]
fn the_wheel_over_the_sidebar_scrolls_it_on_its_own_axis() {
  let (_d, mut app) = app_with_rows(4);
  app.sidebar.open = true;
  app.sidebar.max_scroll = 10;
  app.sidebar.wt_max_scroll = 10;
  app.mouse.push_pane(rect(0, 12, 80, 8), PaneId::Status);
  app.mouse.push_pane(rect(0, 14, 80, 3), PaneId::WorkingTree);

  app.handle_mouse(MouseKind::WheelDown, 10, 13);
  assert_eq!(app.sidebar.scroll, 1);
  assert_eq!(app.sidebar.wt_scroll, 0);

  app.handle_mouse(MouseKind::WheelDown, 10, 15);
  assert_eq!(app.sidebar.wt_scroll, 1, "the Working Tree section has its own axis");
  assert_eq!(app.sidebar.scroll, 1, "and moving it leaves the column where it was");
}

#[test]
fn the_header_affordances_fire_the_actions_their_digits_fire() {
  let (_d, mut app) = app_with_rows(2);
  app.mouse.push_spot(rect(70, 0, 2, 1), Spot::CommandLogs);
  app.mouse.push_spot(rect(73, 0, 2, 1), Spot::Settings);

  assert_eq!(
    app.handle_mouse(MouseKind::Click, 71, 0),
    MouseOutcome::Action(Action::CommandLogs)
  );
  assert_eq!(
    app.handle_mouse(MouseKind::Click, 74, 0),
    MouseOutcome::Action(Action::ConfigPanel)
  );
}

#[test]
fn a_wheel_over_a_button_does_nothing() {
  let (_d, mut app) = app_with_rows(2);
  app.mouse.push_spot(rect(70, 0, 2, 1), Spot::Settings);

  assert_eq!(app.handle_mouse(MouseKind::WheelDown, 71, 0), MouseOutcome::Ignored);
  assert_eq!(app.handle_mouse(MouseKind::WheelUp, 71, 0), MouseOutcome::Ignored);
}

#[test]
fn the_close_button_asks_the_event_loop_for_the_escape_path() {
  let (_d, mut app) = app_with_rows(2);
  app.mouse.push_spot(rect(76, 4, 3, 1), Spot::CloseModal);

  assert_eq!(
    app.handle_mouse(MouseKind::Click, 77, 4),
    MouseOutcome::CloseModal,
    "the button must not close the modal itself — Esc already owns the teardown"
  );
}

/// A modal is a lid. Two things it must not do: fall through to the row under
/// it, and close on a stray click inside itself.
#[test]
fn a_click_on_a_modal_body_neither_falls_through_nor_closes_it() {
  let (_d, mut app) = app_with_rows(6);
  app.mouse.push_pane(rect(4, 2, 70, 16), PaneId::Modal);

  assert_eq!(app.handle_mouse(MouseKind::Click, 10, 6), MouseOutcome::Handled);
  assert_eq!(
    app.list_state.selected(),
    Some(0),
    "the row under the modal must not have been selected"
  );
}

#[test]
fn the_wheel_over_a_modal_body_scrolls_the_modal_that_is_open() {
  let (_d, mut app) = app_with_rows(2);
  app.mouse.push_pane(rect(4, 2, 70, 16), PaneId::Modal);
  app.view = View::CommandLogs;
  app.command_logs.max_scroll = 20;

  app.handle_mouse(MouseKind::WheelDown, 10, 6);
  assert_eq!(app.command_logs.scroll, 1);

  app.view = View::Help;
  app.help_max_scroll = 20;
  app.handle_mouse(MouseKind::WheelDown, 10, 6);
  assert_eq!(app.help_scroll, 1);
  assert_eq!(app.command_logs.scroll, 1, "the other modal did not move");
}

#[test]
fn clicking_a_settings_tab_switches_to_it() {
  let (_d, mut app) = app_with_rows(2);
  app
    .mouse
    .push_spot(rect(20, 5, 8, 1), Spot::ConfigTab(SettingsTab::Keys));
  assert_ne!(app.config_panel.tab, SettingsTab::Keys);

  app.handle_mouse(MouseKind::Click, 22, 5);

  assert_eq!(app.config_panel.tab, SettingsTab::Keys);
}

#[test]
fn clicking_a_modal_row_moves_that_listing_and_no_other() {
  let (_d, mut app) = app_with_rows(6);
  app.mouse.push_rows(rect(4, 8, 60, 2), RowList::OpenMenu, 0, 2);

  app.handle_mouse(MouseKind::Click, 10, 9);

  assert_eq!(app.open_menu_selected, LinkTarget::Pr);
  assert_eq!(
    app.list_state.selected(),
    Some(0),
    "the worktree cursor stayed where it was"
  );
}

#[test]
fn the_counts_footer_opens_the_full_size_working_tree() {
  let (_d, mut app) = app_with_rows(2);
  app.mouse.push_spot(rect(0, 18, 40, 1), Spot::WtCounts);

  assert_eq!(
    app.handle_mouse(MouseKind::Click, 10, 18),
    MouseOutcome::Action(Action::WorkingTree)
  );
}

#[test]
fn a_click_where_the_frame_published_nothing_is_ignored() {
  let (_d, mut app) = app_with_rows(4);

  assert_eq!(app.handle_mouse(MouseKind::Click, 10, 40), MouseOutcome::Ignored);
  assert_eq!(app.handle_mouse(MouseKind::WheelDown, 10, 40), MouseOutcome::Ignored);
  assert_eq!(app.list_state.selected(), Some(0));
}

// ---- The capture toggle ----------------------------------------------------

#[test]
fn the_mouse_starts_captured_and_the_toggle_flips_it_both_ways() {
  let (_d, mut app) = app_with_rows(2);
  assert!(app.mouse_capture, "gwm reads mouse events out of the box");

  app.toggle_mouse_capture();
  assert!(!app.mouse_capture);
  assert!(
    app.status.contains("select text"),
    "the status has to say what was bought: {:?}",
    app.status
  );
  assert!(
    app.status.contains(&app.keymap.keys_display(Action::ToggleMouse)),
    "and which key brings it back: {:?}",
    app.status
  );

  app.toggle_mouse_capture();
  assert!(app.mouse_capture);
}
