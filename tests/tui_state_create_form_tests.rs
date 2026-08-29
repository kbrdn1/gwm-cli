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
fn next_field_rotates_through_the_fields_the_default_pattern_asks_for() {
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
fn the_freeform_name_cap_is_the_validator_s_own_limit_so_nothing_legal_is_truncated() {
  // The form's cap and `WorktreeName::freeform`'s must be the same number:
  // when the form stopped at 200 characters and the validator accepted 255
  // bytes, a 201-character name was silently truncated and submitted as a
  // *different* branch — the one thing "the name becomes the branch verbatim"
  // rules out (Codex review on PR #474). Counted in bytes, like the
  // validator, not in characters.
  assert_eq!(
    MAX_NAME_LEN,
    gwm::naming::MAX_DIR_COMPONENT_BYTES,
    "the form must not stop short of what the validator accepts"
  );
  let mut f = CreateForm::new();
  f.toggle_mode();
  for _ in 0..(MAX_NAME_LEN + 10) {
    f.push_char('x');
  }
  assert_eq!(f.name.len(), MAX_NAME_LEN);

  // And a name that uses the whole budget legally must survive the round
  // trip — the form's job is not to truncate it. (A single 255-byte segment
  // is refused by the `.lock` rule, which is a *ref* limit, not the
  // directory one this cap enforces; split it so the final segment is short.)
  let mut g = CreateForm::new();
  g.toggle_mode();
  for _ in 0..251 {
    g.push_char('x');
  }
  for c in "/yyy".chars() {
    g.push_char(c);
  }
  assert_eq!(g.name.len(), MAX_NAME_LEN);
  assert!(
    gwm::naming::WorktreeName::freeform(&g.name).is_ok(),
    "whatever the form let through must still validate"
  );
}

#[test]
fn a_multibyte_name_is_capped_on_bytes_not_characters() {
  // `é` is two bytes: counting characters would let the buffer grow past the
  // 255-byte path component the directory actually has to fit in.
  let mut f = CreateForm::new();
  f.toggle_mode();
  for _ in 0..MAX_NAME_LEN {
    f.push_char('é');
  }
  assert!(
    f.name.len() <= MAX_NAME_LEN,
    "byte length must stay within the cap, got {}",
    f.name.len()
  );
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

// --- token-driven fields (issue #418) ------------------------------------
//
// The form used to present the canonical triple whatever the repo's patterns
// said. Issue #418's proposal has three parts and two of them shipped ahead of
// it: the live branch/dir preview came with #217's follow-up and #416, and
// `Ctrl-T` came with #416. What was left is this — the field set and the focus
// order, derived from the patterns instead of hardcoded.

use gwm::tui::state::create_form::fields_for;

/// A pattern that writes no issue number must not present an Issue field.
///
/// Not a cosmetic point. `BranchSpec::validate_against` refuses an empty issue,
/// so on a `{type}/{desc}` repo the old form demanded a number, then expanded a
/// pattern that has nowhere to put it: the value was mandatory *and* discarded,
/// which made the TUI create path unusable on that convention.
#[test]
fn a_pattern_without_an_issue_token_presents_no_issue_field() {
  assert_eq!(
    fields_for(&["{type}/{desc}", "{type}-{desc}", "{home}/wt"]),
    [Field::Type, Field::Desc]
  );
}

/// Focus order is the pattern's order, not the canonical triple's.
#[test]
fn the_field_order_follows_the_pattern_rather_than_the_canonical_triple() {
  let mut form = CreateForm::new();
  form.set_fields(fields_for(&["{desc}-{issue}", "{desc}-{issue}", "~/wt"]));
  form.reset();

  assert_eq!(form.field, Field::Desc, "the pattern leads with the description");
  form.next_field();
  assert_eq!(form.field, Field::Issue);
  form.next_field();
  assert_eq!(form.field, Field::Desc, "wraps within the pattern's own fields");
  form.prev_field();
  assert_eq!(form.field, Field::Issue, "and backwards too");
}

/// Rotation must never land on a field the pattern does not render. Walking one
/// full turn plus one is enough to catch a rotation that still enumerates the
/// hardcoded triple.
#[test]
fn rotation_never_lands_on_a_field_the_pattern_omits() {
  for patterns in [
    &["{type}/{desc}", "{type}-{desc}", "~/wt"][..],
    &["#{issue}-{desc}", "{issue}-{desc}", "~/wt"][..],
    &["{type}/#{issue}", "{type}-{issue}", "~/wt"][..],
  ] {
    let expected = fields_for(patterns);
    let mut form = CreateForm::new();
    form.set_fields(expected.clone());
    form.reset();
    for _ in 0..(expected.len() + 1) {
      assert!(
        expected.contains(&form.field),
        "{:?} rotated onto {:?}, which it does not present",
        patterns,
        form.field
      );
      form.next_field();
    }
  }
}

/// Every entry point has to land on a field the pattern renders. Each one used
/// to name a field literally — `reset` chose Type, `toggle_mode` chose Issue,
/// and the rename modal chose Desc — so each is a separate way to focus an
/// input that is not on screen, where typing goes nowhere.
#[test]
fn every_entry_point_focuses_a_field_the_pattern_actually_presents() {
  // A pattern omitting each of the three in turn, so no single hardcoded
  // choice can pass all three cases.
  for patterns in [
    &["{type}/{desc}", "{type}-{desc}", "~/wt"][..],
    &["#{issue}-{desc}", "{issue}-{desc}", "~/wt"][..],
    &["{type}/#{issue}", "{type}-{issue}", "~/wt"][..],
  ] {
    let expected = fields_for(patterns);
    let mut form = CreateForm::new();
    form.set_fields(expected.clone());

    form.reset();
    assert!(
      expected.contains(&form.field),
      "{:?}: reset focused {:?}",
      patterns,
      form.field
    );

    assert!(
      expected.contains(&form.entry_field()),
      "{:?}: entry_field is {:?}",
      patterns,
      form.entry_field()
    );
    assert!(
      expected.contains(&form.last_field()),
      "{:?}: last_field is {:?}",
      patterns,
      form.last_field()
    );

    // Free-form and back: the return leg used to name Issue unconditionally.
    form.toggle_mode();
    assert_eq!(form.field, Field::Name);
    form.toggle_mode();
    assert!(
      expected.contains(&form.field),
      "{:?}: toggling back focused {:?}",
      patterns,
      form.field
    );
  }
}

/// The form opens on the first field the user *types into*: Type is cycled
/// rather than typed, so opening there makes the first keypress a silent no-op
/// (#217). That rule has to survive being generalised.
#[test]
fn the_entry_field_skips_the_cycle_only_type_selector() {
  let mut form = CreateForm::new();
  assert_eq!(
    form.entry_field(),
    Field::Issue,
    "the default pattern still opens on Issue"
  );

  form.set_fields(fields_for(&["{type}/{desc}", "{type}-{desc}", "~/wt"]));
  assert_eq!(form.entry_field(), Field::Desc, "Type is skipped, Desc is next");

  // A pattern whose only editable token is the type has nowhere else to go.
  form.set_fields(fields_for(&["{type}/fixed", "{type}-fixed", "~/wt"]));
  assert_eq!(form.entry_field(), Field::Type);
}

/// A field the pattern omits must not accept keystrokes either. Rotation alone
/// is not enough: `App` seeds `field` directly on several paths, and a buffer
/// filled behind a field that is never drawn is a value the user cannot see,
/// cannot correct, and (where the pattern does carry the token elsewhere) may
/// still be written.
#[test]
fn typing_into_a_field_the_pattern_omits_is_a_no_op() {
  let mut form = CreateForm::new();
  form.set_fields(fields_for(&["{type}/{desc}", "{type}-{desc}", "~/wt"]));
  form.field = Field::Issue;
  form.push_char('4');
  form.push_char('2');
  assert!(
    form.issue.is_empty(),
    "the Issue field is not presented, so it takes nothing"
  );

  form.issue.push_str("42");
  form.pop_char();
  assert_eq!(form.issue, "42", "and it gives nothing back either");
}

/// A pattern set with no editable token at all is degenerate (every worktree
/// would get the same branch name, and git refuses the second), but it must not
/// panic the form.
#[test]
fn a_pattern_set_with_no_editable_token_leaves_the_form_inert_rather_than_panicking() {
  let mut form = CreateForm::new();
  form.set_fields(fields_for(&["wip", "wip", "~/wt"]));
  assert!(form.fields().is_empty());
  form.reset();
  form.next_field();
  form.prev_field();
  form.push_char('x');
  assert!(form.issue.is_empty() && form.desc.is_empty());
}

/// `set_fields` carries the repo's configuration, not user input — a reset
/// clears what was typed and must leave the field set standing.
#[test]
fn reset_clears_the_buffers_but_keeps_the_configured_field_set() {
  let mut form = CreateForm::new();
  let expected = fields_for(&["{desc}-{issue}", "{desc}-{issue}", "~/wt"]);
  form.set_fields(expected.clone());
  form.desc.push_str("thing");
  form.reset();
  assert_eq!(form.fields(), expected.as_slice());
  assert!(form.desc.is_empty());
}

// ---- Mode::FromIssue (issue #625) ---------------------------------------

#[test]
fn enter_from_issue_opens_on_the_number_and_clears_what_was_typed() {
  let mut form = CreateForm::new();
  form.desc.push_str("stale");
  form.name.push_str("stale");
  form.type_index = 2;

  form.enter_from_issue();

  assert_eq!(form.mode, Mode::FromIssue);
  assert_eq!(form.field, Field::Issue);
  assert!(form.issue.is_empty());
  assert!(form.desc.is_empty(), "a stale slug would survive the derivation");
  assert_eq!(form.awaiting_issue, None);
}

#[test]
fn from_issue_accepts_the_number_even_when_the_patterns_write_none() {
  // A `{type}/{desc}` repo discards the number from the branch, so `Issue`
  // is not in its field list — and this mode still has to collect it, since
  // the number is what gets fetched rather than what gets written. Reading
  // the pattern's field list here would make the form untypeable.
  let mut form = CreateForm::new();
  form.set_fields(vec![Field::Type, Field::Desc]);
  form.enter_from_issue();

  form.push_char('5');
  form.push_char('9');
  form.push_char('4');

  assert_eq!(form.issue, "594");
}

#[test]
fn from_issue_has_one_field_so_rotation_stays_put() {
  let mut form = CreateForm::new();
  form.enter_from_issue();

  form.next_field();
  assert_eq!(form.field, Field::Issue);
  form.prev_field();
  assert_eq!(form.field, Field::Issue);
}

#[test]
fn from_issue_toggles_back_to_the_structured_triple() {
  // One key out of a mode the user opened by name, rather than a three-way
  // cycle that would make the toggle unpredictable.
  let mut form = CreateForm::new();
  form.enter_from_issue();

  form.toggle_mode();

  assert_eq!(form.mode, Mode::Structured);
  assert_eq!(form.field, form.entry_field());
}

#[test]
fn reset_clears_the_awaited_issue() {
  // A form reopened while a fetch is still in flight must not adopt that
  // fetch's answer: `reset` is what `enter_create` calls.
  let mut form = CreateForm::new();
  form.enter_from_issue();
  form.awaiting_issue = Some(594);

  form.reset();

  assert_eq!(form.awaiting_issue, None);
  assert_eq!(form.mode, Mode::Structured);
}
