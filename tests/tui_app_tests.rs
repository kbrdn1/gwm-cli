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
