//! Unit tests for the pure `CreateForm` sub-struct (issue #123).
//!
//! Exercises the input form state in isolation — the form owns `field`
//! / `type_index` / `issue` / `desc`, exposes focus rotation, type
//! cycling, character push/pop. The `App` orchestrator owns the
//! side-effecting `submit_create` which wires the form's resolved values
//! into `BranchSpec` + `worktree::add` + `bootstrap::run`.

use gwm::tui::state::create_form::{CreateForm, Field};

#[test]
fn reset_returns_form_to_initial_state() {
  let mut form = CreateForm::new();
  form.issue.push_str("42");
  form.desc.push_str("foo");
  form.type_index = 2;
  form.field = Field::Desc;

  form.reset();

  assert_eq!(form.field, Field::Type);
  assert_eq!(form.type_index, 0);
  assert!(form.issue.is_empty());
  assert!(form.desc.is_empty());
}

#[test]
fn next_field_rotates_type_to_issue_to_desc_to_type() {
  let mut form = CreateForm::new();
  assert_eq!(form.field, Field::Type);
  form.next_field();
  assert_eq!(form.field, Field::Issue);
  form.next_field();
  assert_eq!(form.field, Field::Desc);
  form.next_field();
  assert_eq!(form.field, Field::Type, "wraps back to Type");
}

#[test]
fn prev_field_rotates_in_reverse() {
  let mut form = CreateForm::new();
  form.prev_field();
  assert_eq!(form.field, Field::Desc, "Type -> Desc on prev (wraps)");
  form.prev_field();
  assert_eq!(form.field, Field::Issue);
  form.prev_field();
  assert_eq!(form.field, Field::Type);
}

#[test]
fn next_type_wraps_at_branch_types_len() {
  let mut form = CreateForm::new();
  form.next_type(3);
  assert_eq!(form.type_index, 1);
  form.next_type(3);
  assert_eq!(form.type_index, 2);
  form.next_type(3);
  assert_eq!(form.type_index, 0, "wraps");
}

#[test]
fn prev_type_wraps_at_zero() {
  let mut form = CreateForm::new();
  form.prev_type(3);
  assert_eq!(form.type_index, 2, "0 -> last");
  form.prev_type(3);
  assert_eq!(form.type_index, 1);
  form.prev_type(3);
  assert_eq!(form.type_index, 0);
}

#[test]
fn next_and_prev_type_noop_on_empty_types() {
  // Empty allow-list edge case: the form must not panic on % 0.
  let mut form = CreateForm::new();
  form.next_type(0);
  assert_eq!(form.type_index, 0);
  form.prev_type(0);
  assert_eq!(form.type_index, 0);
}

#[test]
fn push_char_only_accepts_digits_on_issue_field() {
  // Branch convention: `<type>/#<digits>-<slug>`. The issue field
  // restricts to digits so the slug parser never sees garbage; the desc
  // field accepts any input (slug normalisation happens in BranchSpec).
  let mut form = CreateForm::new();
  form.field = Field::Issue;
  form.push_char('1');
  form.push_char('a');
  form.push_char('2');
  assert_eq!(form.issue, "12", "non-digit chars must be dropped on Issue field");
}

#[test]
fn push_char_on_desc_accepts_any_printable() {
  let mut form = CreateForm::new();
  form.field = Field::Desc;
  for c in "foo-bar".chars() {
    form.push_char(c);
  }
  assert_eq!(form.desc, "foo-bar");
}

#[test]
fn push_char_on_type_field_is_noop() {
  // Type is selected via next_type / prev_type, not typed — chars must
  // not bleed into any string field when Type is focused.
  let mut form = CreateForm::new();
  form.field = Field::Type;
  form.push_char('x');
  assert!(form.issue.is_empty());
  assert!(form.desc.is_empty());
}

#[test]
fn pop_char_removes_last_character_on_active_field() {
  let mut form = CreateForm::new();
  form.issue.push_str("42");
  form.desc.push_str("foo");

  form.field = Field::Issue;
  form.pop_char();
  assert_eq!(form.issue, "4");

  form.field = Field::Desc;
  form.pop_char();
  assert_eq!(form.desc, "fo");
}

#[test]
fn pop_char_on_empty_is_noop() {
  let mut form = CreateForm::new();
  form.field = Field::Desc;
  form.pop_char();
  assert!(form.desc.is_empty());
}
