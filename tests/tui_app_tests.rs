mod common;

use common::init_repo;
use gwm::naming::BRANCH_TYPES;
use gwm::tui::{App, Field, View};

fn make_app() -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let app = App::new_at(Some(dir.path())).unwrap();
  (dir, app)
}

#[test]
fn new_loads_main_worktree() {
  let (_dir, app) = make_app();
  assert_eq!(app.worktrees.len(), 1);
  assert!(app.worktrees[0].is_main);
}

#[test]
fn enter_create_initializes_form() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  assert_eq!(app.view, View::Create);
  assert_eq!(app.create_field, Field::Type);
  assert!(app.create_issue.is_empty());
  assert!(app.create_desc.is_empty());
}

#[test]
fn create_field_navigation_loops() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_next_field();
  assert_eq!(app.create_field, Field::Issue);
  app.create_next_field();
  assert_eq!(app.create_field, Field::Desc);
  app.create_next_field();
  assert_eq!(app.create_field, Field::Type);
  app.create_prev_field();
  assert_eq!(app.create_field, Field::Desc);
}

#[test]
fn create_type_navigation_loops() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_prev_type();
  assert_eq!(app.create_type_index, BRANCH_TYPES.len() - 1);
  app.create_next_type();
  assert_eq!(app.create_type_index, 0);
}

#[test]
fn create_push_only_digits_on_issue() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_field = Field::Issue;
  for c in "12a3".chars() {
    app.create_push_char(c);
  }
  assert_eq!(app.create_issue, "123");
}

#[test]
fn create_push_accepts_desc_chars() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_field = Field::Desc;
  for c in "foo-bar".chars() {
    app.create_push_char(c);
  }
  assert_eq!(app.create_desc, "foo-bar");
  app.create_pop_char();
  assert_eq!(app.create_desc, "foo-ba");
}

#[test]
fn enter_confirm_delete_refuses_main() {
  let (_dir, mut app) = make_app();
  app.enter_confirm_delete();
  assert_eq!(app.view, View::List, "main worktree should not allow delete view");
}

#[test]
fn toggle_delete_branch_flips() {
  let (_dir, mut app) = make_app();
  assert!(!app.delete_branch_on_remove);
  app.toggle_delete_branch();
  assert!(app.delete_branch_on_remove);
}

#[test]
fn next_prev_with_single_entry_stays_put() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.next();
  assert_eq!(app.list_state.selected(), Some(0));
  app.prev();
  assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn refresh_keeps_selection_in_bounds() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(5));
  app.refresh().unwrap();
  assert_eq!(app.list_state.selected(), Some(0));
}

// ---- sidebar / focus / vim motions ---------------------------------------

#[test]
fn sidebar_open_by_default() {
  let (_dir, app) = make_app();
  assert!(
    app.sidebar_open,
    "sidebar should default to open (will be hidden when narrow)"
  );
  assert!(!app.sidebar_focused, "focus defaults to the worktree list");
}

#[test]
fn toggle_sidebar_flips_open_flag() {
  let (_dir, mut app) = make_app();
  let before = app.sidebar_open;
  app.toggle_sidebar();
  assert_eq!(app.sidebar_open, !before);
  app.toggle_sidebar();
  assert_eq!(app.sidebar_open, before);
}

#[test]
fn toggle_sidebar_when_closed_resets_focus_to_list() {
  let (_dir, mut app) = make_app();
  app.sidebar_focused = true;
  app.sidebar_open = true;
  app.toggle_sidebar(); // close
  assert!(!app.sidebar_open);
  assert!(
    !app.sidebar_focused,
    "closing the sidebar must drop focus back to the list"
  );
}

#[test]
fn toggle_focus_only_works_when_sidebar_open() {
  let (_dir, mut app) = make_app();
  app.sidebar_open = false;
  app.toggle_focus();
  assert!(!app.sidebar_focused, "focus cannot move to a hidden sidebar");

  app.sidebar_open = true;
  app.toggle_focus();
  assert!(app.sidebar_focused);
  app.toggle_focus();
  assert!(!app.sidebar_focused);
}

#[test]
fn first_selects_first_worktree() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.first();
  assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn last_selects_last_worktree() {
  let (_dir, mut app) = make_app();
  app.last();
  let expected = app.worktrees.len().saturating_sub(1);
  assert_eq!(app.list_state.selected(), Some(expected));
}

#[test]
fn handle_g_motion_tracks_pending_then_jumps_to_first() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  // First `g` arms the motion but does not move.
  assert!(!app.pending_g);
  app.handle_g();
  assert!(app.pending_g, "first 'g' must arm the gg sequence");
  // Second `g` jumps to first and disarms.
  app.handle_g();
  assert!(!app.pending_g, "second 'g' completes gg and disarms");
  assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn pending_g_resets_on_other_key() {
  let (_dir, mut app) = make_app();
  app.handle_g();
  assert!(app.pending_g);
  app.cancel_pending_motion();
  assert!(!app.pending_g, "any non-g keypress must drop the pending motion");
}

#[test]
fn sidebar_scroll_clamps_to_zero() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.sidebar_scroll, 0);
  app.sidebar_scroll_up();
  assert_eq!(app.sidebar_scroll, 0, "scrolling up from 0 stays at 0");

  // The renderer normally publishes a max bound; simulate enough room for scroll.
  app.sidebar_max_scroll = 5;
  app.sidebar_scroll_down();
  assert_eq!(app.sidebar_scroll, 1);
  app.sidebar_scroll_up();
  assert_eq!(app.sidebar_scroll, 0);
}

#[test]
fn sidebar_scroll_clamps_at_max() {
  // The renderer sets `sidebar_max_scroll`. Scrolling past it must stop there
  // so the user can't push the panel content entirely off-screen.
  let (_dir, mut app) = make_app();
  app.sidebar_max_scroll = 3;
  app.sidebar_scroll_down();
  app.sidebar_scroll_down();
  app.sidebar_scroll_down();
  assert_eq!(app.sidebar_scroll, 3);
  app.sidebar_scroll_down();
  assert_eq!(app.sidebar_scroll, 3, "scrolling beyond max must clamp");
}

#[test]
fn focus_routes_navigation_to_sidebar() {
  // When sidebar is focused, next()/prev() should NOT move the list selection.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.sidebar_open = true;
  app.sidebar_focused = true;
  app.sidebar_max_scroll = 5; // pretend the renderer has populated this

  app.next();
  assert_eq!(
    app.list_state.selected(),
    Some(0),
    "list must stay put when sidebar has focus"
  );
  assert!(
    app.sidebar_scroll >= 1,
    "next() must scroll the sidebar when it has focus"
  );

  app.prev();
  assert_eq!(app.list_state.selected(), Some(0));
  assert_eq!(app.sidebar_scroll, 0, "prev() scrolled back up");
}

#[test]
fn next_prev_invalidate_sidebar_cache() {
  // Moving selection must drop any cached sidebar content so the new
  // worktree's preview is recomputed on the next frame.
  let (_dir, mut app) = make_app();
  app.sidebar_cache = Some((std::path::PathBuf::from("/tmp/x"), vec![]));
  app.next();
  assert!(app.sidebar_cache.is_none(), "next() must invalidate the sidebar cache");

  app.sidebar_cache = Some((std::path::PathBuf::from("/tmp/x"), vec![]));
  app.prev();
  assert!(app.sidebar_cache.is_none(), "prev() must invalidate the sidebar cache");
}

#[test]
fn refresh_invalidates_sidebar_cache() {
  let (_dir, mut app) = make_app();
  app.sidebar_cache = Some((std::path::PathBuf::from("/tmp/x"), vec![]));
  app.refresh().unwrap();
  assert!(app.sidebar_cache.is_none());
}
