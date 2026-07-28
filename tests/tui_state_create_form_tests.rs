//! Unit tests for the pure `CreateForm` sub-struct (issue #123).
//!
//! Exercises the input form state in isolation — the form owns `field`
//! / `type_index` / `issue` / `desc`, exposes focus rotation, type
//! cycling, character push/pop. The `App` orchestrator owns the
//! side-effecting `submit_create` which wires the form's resolved values
//! into `BranchSpec` and dispatches `worktree::add` + `bootstrap::run` on
//! the async task spine.

use gwm::tui::state::create_form::{CreateForm, Field, Mode, MAX_DESC_LEN, MAX_ISSUE_LEN, MAX_NAME_LEN};

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

#[test]
fn push_char_caps_the_issue_field_length() {
  // Issue #217 follow-up: the inputs grow a length cap so the resolved
  // branch name stays within GitHub's git-ref limits. The issue number is
  // bounded to `MAX_ISSUE_LEN` digits.
  let mut form = CreateForm::new();
  form.field = Field::Issue;
  for _ in 0..(MAX_ISSUE_LEN + 20) {
    form.push_char('9');
  }
  assert_eq!(
    form.issue.chars().count(),
    MAX_ISSUE_LEN,
    "issue must not grow past the cap"
  );
}

#[test]
fn push_char_caps_the_desc_field_length() {
  // The description (slug) is bounded to `MAX_DESC_LEN` so the
  // `<type>/#<issue>-<desc>` branch name stays under the 255-byte git ref
  // limit.
  let mut form = CreateForm::new();
  form.field = Field::Desc;
  for _ in 0..(MAX_DESC_LEN + 50) {
    form.push_char('a');
  }
  assert_eq!(
    form.desc.chars().count(),
    MAX_DESC_LEN,
    "desc must not grow past the cap"
  );
}

// --- free-form mode (issue #416) ----------------------------------------

#[test]
fn the_form_opens_in_structured_mode() {
  let f = CreateForm::new();
  assert_eq!(f.mode, Mode::Structured);
}

#[test]
fn toggling_switches_mode_and_focuses_the_only_field_of_the_target_mode() {
  let mut f = CreateForm::new();
  f.toggle_mode();
  assert_eq!(f.mode, Mode::Freeform);
  assert_eq!(f.field, Field::Name, "free-form has a single field, focus it");

  f.toggle_mode();
  assert_eq!(f.mode, Mode::Structured);
  assert_eq!(f.field, Field::Issue, "back to the field `enter_create` opens on");
}

/// Toggling is exploratory — a user flipping modes to see the other form
/// must not lose what they already typed.
#[test]
fn toggling_preserves_what_was_already_typed_on_both_sides() {
  let mut f = CreateForm::new();
  f.field = Field::Desc;
  for c in "tui-search".chars() {
    f.push_char(c);
  }
  f.toggle_mode();
  for c in "spike-redis".chars() {
    f.push_char(c);
  }
  assert_eq!(f.name, "spike-redis");

  f.toggle_mode();
  assert_eq!(f.desc, "tui-search", "the structured slug survived the round trip");
  f.toggle_mode();
  assert_eq!(f.name, "spike-redis", "and so did the free-form name");
}

/// One field means field rotation has nowhere to go — Tab must not walk
/// focus onto Type or Issue, which free-form mode does not present.
#[test]
fn field_rotation_is_a_no_op_in_freeform_mode() {
  let mut f = CreateForm::new();
  f.toggle_mode();
  f.next_field();
  assert_eq!(f.field, Field::Name);
  f.prev_field();
  assert_eq!(f.field, Field::Name);
}

#[test]
fn freeform_accepts_characters_the_slug_field_would_also_take() {
  let mut f = CreateForm::new();
  f.toggle_mode();
  for c in "Spike_Redis 2".chars() {
    f.push_char(c);
  }
  assert_eq!(
    f.name, "Spike_Redis 2",
    "validation happens on submit, not per keystroke"
  );
  f.pop_char();
  assert_eq!(f.name, "Spike_Redis ");
}

#[test]
fn the_freeform_name_is_length_capped_like_the_other_text_fields() {
  let mut f = CreateForm::new();
  f.toggle_mode();
  for _ in 0..(MAX_NAME_LEN + 10) {
    f.push_char('x');
  }
  assert_eq!(f.name.chars().count(), MAX_NAME_LEN);
}

#[test]
fn reset_returns_to_structured_mode_and_clears_the_name() {
  let mut f = CreateForm::new();
  f.toggle_mode();
  for c in "spike".chars() {
    f.push_char(c);
  }
  f.reset();
  assert_eq!(f.mode, Mode::Structured);
  assert!(f.name.is_empty());
}
