//! Contract tests for the contextual modal keymap (issue #219).
//!
//! Ratatui-free: every assertion drives the pure [`ModalKeymap`] model so
//! the routing contract is pinned without spawning a terminal.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gwm::tui::keymap::{KeyStroke, Source};
use gwm::tui::modal_keymap::{KeyContext, ModalAction, ModalKeymap};

fn stroke(code: KeyCode) -> KeyStroke {
  KeyStroke::new(code, KeyModifiers::empty())
}

fn ch(c: char) -> KeyStroke {
  KeyStroke::new(KeyCode::Char(c), KeyModifiers::empty())
}

// ── context / verb roundtrip ───────────────────────────────────────────────

#[test]
fn config_path_roundtrips_through_from_config_path() {
  for ctx in KeyContext::all() {
    assert_eq!(
      KeyContext::from_config_path(ctx.config_path()),
      Some(*ctx),
      "config_path {:?} must roundtrip",
      ctx.config_path()
    );
  }
}

#[test]
fn link_stages_use_dotted_config_paths() {
  assert_eq!(KeyContext::LinkChooseTarget.config_path(), "link.choose_target");
  assert_eq!(KeyContext::LinkInputNumber.config_path(), "link.input_number");
  assert_eq!(KeyContext::ConfigEdit.config_path(), "config.edit");
}

#[test]
fn from_context_verb_resolves_known_verbs_and_rejects_unknown() {
  assert_eq!(
    ModalAction::from_context_verb(KeyContext::Confirm, "confirm"),
    Some(ModalAction::ConfirmConfirm)
  );
  assert_eq!(
    ModalAction::from_context_verb(KeyContext::Create, "submit"),
    Some(ModalAction::CreateSubmit)
  );
  // A verb that exists in another context must not leak across.
  assert_eq!(ModalAction::from_context_verb(KeyContext::Confirm, "submit"), None);
  assert_eq!(ModalAction::from_context_verb(KeyContext::Create, "confirm"), None);
}

#[test]
fn every_action_reports_its_owning_context() {
  for action in ModalAction::all() {
    let ctx = action.context();
    assert_eq!(
      ModalAction::from_context_verb(ctx, action.verb()),
      Some(action),
      "{:?} must resolve from its own (context, verb)",
      action
    );
  }
}

// ── defaults resolution ────────────────────────────────────────────────────

#[test]
fn defaults_resolve_the_historical_keys() {
  let km = ModalKeymap::defaults();

  // confirm: y / Enter / n / Esc / h / l / Tab
  assert_eq!(
    km.resolve(KeyContext::Confirm, &ch('y')),
    Some(ModalAction::ConfirmConfirm)
  );
  assert_eq!(
    km.resolve(KeyContext::Confirm, &stroke(KeyCode::Enter)),
    Some(ModalAction::ConfirmActivate)
  );
  assert_eq!(
    km.resolve(KeyContext::Confirm, &ch('n')),
    Some(ModalAction::ConfirmCancel)
  );
  assert_eq!(
    km.resolve(KeyContext::Confirm, &stroke(KeyCode::Esc)),
    Some(ModalAction::ConfirmCancel)
  );
  assert_eq!(
    km.resolve(KeyContext::Confirm, &ch('h')),
    Some(ModalAction::ConfirmFocusConfirm)
  );
  assert_eq!(
    km.resolve(KeyContext::Confirm, &stroke(KeyCode::Tab)),
    Some(ModalAction::ConfirmToggleFocus)
  );

  // create: Tab / BackTab / Enter / Esc, and h/l type cycling
  assert_eq!(
    km.resolve(KeyContext::Create, &stroke(KeyCode::Tab)),
    Some(ModalAction::CreateNextField)
  );
  assert_eq!(
    km.resolve(KeyContext::Create, &stroke(KeyCode::BackTab)),
    Some(ModalAction::CreatePrevField)
  );
  assert_eq!(
    km.resolve(KeyContext::Create, &ch('l')),
    Some(ModalAction::CreateNextType)
  );

  // link stages
  assert_eq!(
    km.resolve(KeyContext::LinkChooseTarget, &ch('i')),
    Some(ModalAction::LinkChooseIssue)
  );
  assert_eq!(
    km.resolve(KeyContext::LinkInputNumber, &stroke(KeyCode::Enter)),
    Some(ModalAction::LinkInputSubmit)
  );
}

#[test]
fn unbound_stroke_resolves_to_none() {
  let km = ModalKeymap::defaults();
  // `z` is bound in no modal context.
  assert_eq!(km.resolve(KeyContext::Confirm, &ch('z')), None);
  // digits are free text in link.input_number, not a verb.
  assert_eq!(km.resolve(KeyContext::LinkInputNumber, &ch('4')), None);
}

#[test]
fn same_key_means_different_verbs_in_different_contexts() {
  let km = ModalKeymap::defaults();
  // Enter is `submit` in create but `activate` in confirm — the whole point.
  assert_eq!(
    km.resolve(KeyContext::Create, &stroke(KeyCode::Enter)),
    Some(ModalAction::CreateSubmit)
  );
  assert_eq!(
    km.resolve(KeyContext::Confirm, &stroke(KeyCode::Enter)),
    Some(ModalAction::ConfirmActivate)
  );
  // `i` is `issue` in both link.choose_target and open_menu — independent.
  assert_eq!(
    km.resolve(KeyContext::LinkChooseTarget, &ch('i')),
    Some(ModalAction::LinkChooseIssue)
  );
  assert_eq!(
    km.resolve(KeyContext::OpenMenu, &ch('i')),
    Some(ModalAction::OpenMenuIssue)
  );
}

// ── overrides ──────────────────────────────────────────────────────────────

#[test]
fn override_replaces_keys_for_a_verb() {
  let mut km = ModalKeymap::defaults();
  km.apply_override(ModalAction::ConfirmConfirm, vec![ch('o')]).unwrap();
  // new key fires
  assert_eq!(
    km.resolve(KeyContext::Confirm, &ch('o')),
    Some(ModalAction::ConfirmConfirm)
  );
  // old default `y` no longer fires
  assert_eq!(km.resolve(KeyContext::Confirm, &ch('y')), None);
}

#[test]
fn override_can_unbind_with_empty_vec() {
  let mut km = ModalKeymap::defaults();
  km.apply_override(ModalAction::ConfirmConfirm, vec![]).unwrap();
  assert_eq!(km.resolve(KeyContext::Confirm, &ch('y')), None);
}

#[test]
fn user_override_wins_over_a_default_in_the_same_context() {
  // Bind confirm's `cancel` to `y` — which defaults to `confirm`. The
  // default `confirm` binding must silently vacate `y`; no conflict.
  let mut km = ModalKeymap::defaults();
  km.apply_override(ModalAction::ConfirmCancel, vec![ch('y'), stroke(KeyCode::Esc)])
    .unwrap();
  assert_eq!(
    km.resolve(KeyContext::Confirm, &ch('y')),
    Some(ModalAction::ConfirmCancel)
  );
}

#[test]
fn user_vs_user_conflict_in_same_context_is_rejected() {
  let mut km = ModalKeymap::defaults();
  km.apply_override(ModalAction::ConfirmConfirm, vec![ch('x')]).unwrap();
  let err = km
    .apply_override(ModalAction::ConfirmCancel, vec![ch('x')])
    .expect_err("same key for two verbs in one context must conflict");
  let msg = err.to_string();
  assert!(msg.contains("conflict"), "message should mention conflict: {msg}");
  assert!(msg.contains("confirm"), "message should name the context: {msg}");
}

#[test]
fn cross_context_reuse_is_not_a_conflict() {
  let mut km = ModalKeymap::defaults();
  // Bind `x` in confirm and in create — different contexts, both fine.
  // (`next_type` is the create verb that stays bindable on a bare
  // letter: it lives on the typing-free Type field.)
  km.apply_override(ModalAction::ConfirmConfirm, vec![ch('x')]).unwrap();
  km.apply_override(ModalAction::CreateNextType, vec![ch('x')]).unwrap();
  assert_eq!(
    km.resolve(KeyContext::Confirm, &ch('x')),
    Some(ModalAction::ConfirmConfirm)
  );
  assert_eq!(
    km.resolve(KeyContext::Create, &ch('x')),
    Some(ModalAction::CreateNextType)
  );
}

#[test]
fn toggle_mode_refuses_a_bare_printable_the_text_fields_would_swallow() {
  // #456's contract: `handle_create_key` routes an unmodified printable into
  // the focused text field BEFORE resolving the modal verb, so `toggle_mode =
  // ["x"]` would be accepted by config and then never fire — and in free-form
  // mode, where `Name` is the only field, it would leave no way back to the
  // structured form. `Ctrl+t` is the default precisely because of this, but
  // the guard has to cover the verb too (Codex review on PR #474).
  let mut km = ModalKeymap::defaults();
  let err = km
    .apply_override(ModalAction::CreateToggleMode, vec![ch('x')])
    .expect_err("a bare printable must be refused for toggle_mode");
  assert!(
    err.to_string().contains("reserved for typing"),
    "the message must explain why: {err}"
  );
  // Ctrl-modified stays bindable — that is the escape hatch the default uses.
  km.apply_override(
    ModalAction::CreateToggleMode,
    vec![KeyStroke::new(KeyCode::Char('y'), KeyModifiers::CONTROL)],
  )
  .expect("a Ctrl-modified stroke is not typing input");
}

#[test]
fn override_marks_source_as_user_config() {
  let mut km = ModalKeymap::defaults();
  km.apply_override(ModalAction::ConfirmConfirm, vec![ch('o')]).unwrap();
  let binding = km
    .list()
    .iter()
    .find(|b| b.action == ModalAction::ConfirmConfirm)
    .unwrap();
  assert_eq!(binding.source, Source::UserConfig);
  // an untouched verb stays Default
  let other = km
    .list()
    .iter()
    .find(|b| b.action == ModalAction::ConfirmCancel)
    .unwrap();
  assert_eq!(other.source, Source::Default);
}

// ── single-stroke contract ─────────────────────────────────────────────────

#[test]
fn parse_single_accepts_one_stroke_rejects_chords() {
  use gwm::tui::modal_keymap::parse_single;
  assert!(parse_single("Enter").is_ok());
  assert!(parse_single("Ctrl+s").is_ok());
  let err = parse_single("g g").expect_err("multi-stroke chord must be rejected for modals");
  assert!(
    err.to_string().contains("single keystroke"),
    "message should explain the single-stroke rule: {err}"
  );
}

#[test]
fn parse_single_rejects_the_reserved_ctrl_c() {
  // #219 review (P2): Ctrl+C is the emergency quit handled in `run_app` before
  // any modal lookup, so a modal binding to it would never fire. Reject it at
  // parse time rather than advertise an unreachable action in `gwm tui keys`.
  use gwm::tui::modal_keymap::parse_single;
  for s in ["Ctrl+c", "Ctrl+C"] {
    let err = parse_single(s).expect_err("Ctrl+C must be rejected as reserved");
    assert!(
      err.to_string().contains("Ctrl+C") || err.to_string().to_lowercase().contains("reserved"),
      "message should explain the reserved key: {err}"
    );
  }
}

// ── listing ────────────────────────────────────────────────────────────────

#[test]
fn bindings_for_returns_only_that_contexts_verbs() {
  let km = ModalKeymap::defaults();
  let confirm = km.bindings_for(KeyContext::Confirm);
  assert!(confirm.iter().all(|b| b.action.context() == KeyContext::Confirm));
  // confirm has seven verbs (#551 added `cycle_method`)
  assert_eq!(confirm.len(), 7);
}

// ── BackTab / Shift-Tab terminal disagreement (issue #219 review) ──────────

#[test]
fn backtab_with_shift_modifier_resolves_like_plain_backtab() {
  // Some terminals report Shift-Tab as `KeyCode::BackTab` WITH a SHIFT
  // modifier; others as bare `BackTab`. Both mean the same keystroke, so the
  // modal resolver must fire the prev-field / prev-tab verb bound to the
  // modifier-less `"BackTab"` default in either case. Without normalisation
  // the shifted variant compared unequal and the default silently did
  // nothing — a regression vs the old hard-coded `KeyCode::BackTab` branch.
  let km = ModalKeymap::defaults();
  let shifted = KeyStroke::from_event(&KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
  assert_eq!(
    km.resolve(KeyContext::Create, &shifted),
    Some(ModalAction::CreatePrevField)
  );
  assert_eq!(
    km.resolve(KeyContext::Config, &shifted),
    Some(ModalAction::ConfigPrevTab)
  );
}

#[test]
fn detail_context_binds_o_to_the_resume_pane_verb() {
  // Issue #591. `o` was unbound in the agents context, which is why it was
  // available; in the list view it is `Action::TerminalPty`, so the letter
  // already reads as "open a terminal here".
  let km = ModalKeymap::defaults();
  assert_eq!(
    km.resolve(KeyContext::Detail, &ch('o')),
    Some(ModalAction::DetailOpenPane)
  );
  // It joins the other three under `[tui.keys.modal.detail]`, so it stays
  // rebindable rather than being a hardcoded key in the dispatch.
  assert_eq!(
    ModalAction::from_context_verb(KeyContext::Detail, "open_pane"),
    Some(ModalAction::DetailOpenPane)
  );
  // The verb it must not have stolen: `a` is still the pin.
  assert_eq!(
    km.resolve(KeyContext::Detail, &ch('a')),
    Some(ModalAction::DetailAttach)
  );
}
