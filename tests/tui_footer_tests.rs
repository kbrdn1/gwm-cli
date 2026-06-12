//! Contract tests for the single-line statusline builder (`footer_line`).
//!
//! Issue #180: the footer must render on ONE line, present its key hints as
//! reverse-video badge chips, and keep the status message (the action log)
//! visible at the right edge — hints are what gets truncated when the
//! terminal is narrow, never the log.
//!
//! `footer_line` is a pure builder (like `header_line` / `help_lines`) so the
//! layout contract is pinned here without spinning up a ratatui backend.

use gwm::tui::theme::Theme;
use gwm::tui::{footer_line, status_line, HintContext};
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;

/// Flatten a `Line` back to its rendered plain text by concatenating every
/// span's content — what the user sees on the row, minus styling.
fn plain(line: &Line<'_>) -> String {
  line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn display_width(line: &Line<'_>) -> usize {
  plain(line).chars().count()
}

const HINTS: &[(&str, &str)] = &[
  ("n", "new"),
  ("d", "del"),
  ("b", "boot"),
  ("o", "open"),
  ("y", "yank"),
  ("l", "git"),
  ("R", "review"),
  ("v", "sidebar"),
  ("Tab", "focus"),
  ("/", "filter"),
  ("?", "help"),
  ("q", "quit"),
];

#[test]
fn footer_fits_on_a_single_line_within_the_given_width() {
  let line = footer_line(HINTS, "press ? for help", 120, &Theme::default());
  // No wrapping: the rendered text never exceeds the width handed in.
  assert!(
    display_width(&line) <= 120,
    "footer width {} exceeded 120: {:?}",
    display_width(&line),
    plain(&line)
  );
  // And it is genuinely a single line — no embedded newline.
  assert!(!plain(&line).contains('\n'));
}

#[test]
fn status_is_pinned_at_the_end_and_kept_intact_when_wide() {
  let line = footer_line(HINTS, "opened foo", 120, &Theme::default());
  let text = plain(&line);
  // The log lives at the very end of the row, in priority.
  assert!(
    text.trim_end().ends_with("[opened foo]"),
    "status not pinned at end: {text:?}"
  );
}

#[test]
fn hints_are_truncated_with_ellipsis_when_narrow_but_status_survives() {
  let line = footer_line(HINTS, "opened foo", 40, &Theme::default());
  let text = plain(&line);
  assert!(display_width(&line) <= 40, "overflowed 40: {text:?}");
  // The status (log) is preserved …
  assert!(text.contains("[opened foo]"), "status dropped: {text:?}");
  assert!(text.trim_end().ends_with("[opened foo]"), "status not at end: {text:?}");
  // … while the hint list is cut short with an ellipsis marker.
  assert!(text.contains('…'), "expected truncation ellipsis: {text:?}");
}

#[test]
fn status_has_absolute_priority_when_space_is_tiny() {
  // Width barely fits the status: no hint labels should appear, but the log
  // must still be shown (possibly clipped), never sacrificed for hints.
  let line = footer_line(HINTS, "x", 5, &Theme::default());
  let text = plain(&line);
  assert!(display_width(&line) <= 5, "overflowed 5: {text:?}");
  assert!(text.contains("[x]") || text.contains('x'), "status missing: {text:?}");
  assert!(!text.contains("new"), "hint label leaked into a tiny footer: {text:?}");
}

#[test]
fn keys_render_as_accent_bold_binds_not_badges() {
  // Issue #279: the footer hints drop the reverse-video badge for a
  // herdr-style "accent bind + muted action" treatment. The key reads in
  // the accent colour + BOLD with no REVERSED box.
  let line = footer_line(
    HINTS,
    "ready",
    120,
    &Theme {
      accent: Color::Magenta,
      muted: Color::Gray,
      ..Theme::default()
    },
  );
  // A bind span: accent-coloured, bold, NOT reversed, carrying the key.
  let bind = line
    .spans
    .iter()
    .find(|s| s.style.fg == Some(Color::Magenta) && s.content.as_ref() == "n")
    .expect("a bare accent 'n' bind span");
  assert!(
    bind.style.add_modifier.contains(Modifier::BOLD),
    "bind is bold: {bind:?}"
  );
  assert!(
    !line
      .spans
      .iter()
      .any(|s| s.style.add_modifier.contains(Modifier::REVERSED)),
    "no hint span should be a reverse-video badge anymore"
  );
  // Labels are still present so the row stays self-documenting.
  assert!(plain(&line).contains("new"));
}

#[test]
fn newlines_in_status_never_break_the_single_line_contract() {
  // Action logs are sometimes error strings that embed `\n` / `\r`
  // (e.g. a multi-line template error). With `Wrap` disabled a raw newline
  // would still split the row in two — the builder must neutralise them so
  // the footer stays one visual line (PR #183 review).
  let line = footer_line(HINTS, "template error:\nbad line\r\nsecond", 80, &Theme::default());
  let text = plain(&line);
  assert!(!text.contains('\n'), "status newline leaked into footer: {text:?}");
  assert!(!text.contains('\r'), "status CR leaked into footer: {text:?}");
  assert!(display_width(&line) <= 80, "overflowed 80: {text:?}");
}

#[test]
fn zero_width_emits_an_empty_line_without_overflowing() {
  // Degenerate terminal width: the row must not be wider than asked. The
  // pre-fix `trunc()` floor returned "…" (width 1) for width 0 (PR #183 review).
  let line = footer_line(HINTS, "anything", 0, &Theme::default());
  assert_eq!(
    display_width(&line),
    0,
    "footer overflowed a zero width: {:?}",
    plain(&line)
  );
}

// ---------------------------------------------------------------------------
// Contextual statusbar (issue #217): a context chip on the left, an optional
// loading spinner, the contextual hints in the middle, and the action log
// pinned right with absolute priority.
// ---------------------------------------------------------------------------

#[test]
fn status_line_shows_context_chip_at_the_left() {
  let line = status_line(
    "worktrees",
    HINTS,
    "ready",
    None,
    120,
    &Theme {
      accent: Color::Magenta,
      focus: Color::Green,
      ..Theme::default()
    },
  );
  let text = plain(&line);
  // The context label leads the row.
  assert!(
    text.trim_start().starts_with("worktrees") || text.contains("worktrees"),
    "context chip missing at the left: {text:?}"
  );
  // …and the context label stays a reversed focus chip — it's a "where am
  // I" anchor, not a hint, so it keeps the badge treatment (issue #279).
  let has_ctx_chip = line.spans.iter().any(|s| {
    s.style.add_modifier.contains(Modifier::REVERSED)
      && s.style.fg == Some(Color::Green)
      && s.content.contains("worktrees")
  });
  assert!(has_ctx_chip, "context label is not a reversed focus chip: {text:?}");
  // Hint binds, by contrast, are now flat accent-bold glyphs (no badge).
  assert!(
    line.spans.iter().any(|s| {
      !s.style.add_modifier.contains(Modifier::REVERSED)
        && s.style.fg == Some(Color::Magenta)
        && s.style.add_modifier.contains(Modifier::BOLD)
        && s.content.as_ref() == "n"
    }),
    "hint binds should be flat accent-bold glyphs while context uses a focus chip: {text:?}"
  );
}

#[test]
fn status_line_renders_the_loading_spinner_when_present() {
  let with = status_line("status", HINTS, "loading", Some("⠋"), 120, &Theme::default());
  assert!(plain(&with).contains('⠋'), "spinner glyph missing when loading");
  // Absent when not loading.
  let without = status_line("status", HINTS, "idle", None, 120, &Theme::default());
  assert!(!plain(&without).contains('⠋'), "spinner glyph leaked when not loading");
}

#[test]
fn status_line_pins_the_log_right_and_fits_width() {
  let line = status_line("worktrees", HINTS, "opened foo", None, 120, &Theme::default());
  let text = plain(&line);
  assert!(display_width(&line) <= 120, "overflowed 120: {text:?}");
  assert!(!text.contains('\n'), "statusline must stay one row");
  assert!(
    text.trim_end().ends_with("[opened foo]"),
    "log not pinned at end: {text:?}"
  );
}

#[test]
fn status_line_keeps_context_and_log_when_narrow_dropping_hints() {
  // Tight width: hints get truncated but both the context chip and the log
  // survive — they are the load-bearing signals.
  let line = status_line("worktrees", HINTS, "busy", Some("⠙"), 44, &Theme::default());
  let text = plain(&line);
  assert!(display_width(&line) <= 44, "overflowed 44: {text:?}");
  assert!(text.contains("worktrees"), "context dropped under pressure: {text:?}");
  assert!(text.contains("[busy]"), "log dropped under pressure: {text:?}");
}

#[test]
fn link_prompt_status_line_keeps_short_log_at_80_cols() {
  use gwm::tui::keymap::Keymap;
  let km = Keymap::defaults();
  let resolved = HintContext::LinkPrompt.resolve(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults());
  let hints: Vec<(&str, &str)> = resolved.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();

  let line = status_line("link", &hints, "pick", None, 80, &Theme::default());
  let text = plain(&line);

  assert!(display_width(&line) <= 80, "overflowed 80: {text:?}");
  assert!(text.contains("[pick]"), "short Link status was clipped: {text:?}");
}

#[test]
fn hint_context_exposes_label_and_hints() {
  use gwm::tui::keymap::Keymap;
  // The same context source feeds the help subtitle and the statusbar chip.
  assert_eq!(HintContext::Worktrees.label(), "worktrees");
  assert_eq!(HintContext::Status.label(), "status");
  assert_eq!(HintContext::Picker.label(), "switch");
  assert_eq!(HintContext::Create.label(), "create");
  assert_eq!(HintContext::Confirm.label(), "confirm");
  let km = Keymap::defaults();
  for ctx in [
    HintContext::Worktrees,
    HintContext::Status,
    HintContext::Picker,
    HintContext::Create,
    HintContext::Confirm,
    HintContext::OpenMenu,
    HintContext::LinkPrompt,
    HintContext::CommandPalette,
  ] {
    assert!(
      !ctx
        .resolve(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults())
        .is_empty(),
      "context {:?} must advertise hints",
      ctx.label()
    );
  }
}

#[test]
fn worktrees_and_status_hints_advertise_the_command_logs_key() {
  // Issue #226: the statusbar which-key must advertise `3 logs` so the
  // Command Logs overlay is discoverable, alongside the `1`/`2` pane keys.
  use gwm::tui::keymap::Keymap;
  let km = Keymap::defaults();
  for ctx in [HintContext::Worktrees, HintContext::Status] {
    let resolved = ctx.resolve(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults());
    assert!(
      resolved.iter().any(|(k, l)| k == "3" && l == "logs"),
      "context {:?} must advertise the `3 logs` hint: {resolved:?}",
      ctx.label()
    );
  }
}

#[test]
fn worktrees_and_status_hints_advertise_the_settings_panel_key() {
  // Issue #232 / #279: the statusbar which-key must advertise `4 settings`
  // so the Settings panel is discoverable, alongside the `1`/`2`/`3` pane
  // keys (renamed from `config` when the panel became editable).
  use gwm::tui::keymap::Keymap;
  let km = Keymap::defaults();
  for ctx in [HintContext::Worktrees, HintContext::Status] {
    let resolved = ctx.resolve(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults());
    assert!(
      resolved.iter().any(|(k, l)| k == "4" && l == "settings"),
      "context {:?} must advertise the `4 settings` hint: {resolved:?}",
      ctx.label()
    );
  }
}

#[test]
fn status_hints_resolve_user_rebindings() {
  // Issue #217 review (P2): the statusbar must show the *live* binding, not
  // the hard-coded default — the keymap actions are rebindable. `fetch` is
  // `F` by default; rebind it and the resolved hint follows.
  use gwm::tui::keymap::{Action, KeyStroke, Keymap};
  let mut km = Keymap::defaults();
  let default = HintContext::Status.resolve(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults());
  assert!(
    default.iter().any(|(k, l)| k == "F" && l == "fetch"),
    "default status hints should advertise the `F` fetch binding: {default:?}"
  );

  km.apply_override(Action::FetchGithub, vec![KeyStroke::parse_chord("Ctrl+g").unwrap()])
    .unwrap();
  let resolved = HintContext::Status.resolve(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults());
  assert!(
    resolved.iter().any(|(k, l)| k == "Ctrl+g" && l == "fetch"),
    "rebinding fetch_github must change the statusbar hint key: {resolved:?}"
  );
  assert!(
    !resolved.iter().any(|(k, _)| k == "F"),
    "the stale default `F` must not linger after the rebind: {resolved:?}"
  );
}

#[test]
fn worktrees_hints_are_grouped_lifecycle_then_act_then_navigate_then_global() {
  // The footer is truncated right-to-left when narrow, so the order is also a
  // priority: actions are grouped by family (worktree lifecycle → act on the
  // selected worktree → find / navigate panes → global) with the most-used
  // verb of each family first.
  use gwm::tui::keymap::Keymap;
  let labels: Vec<String> = HintContext::Worktrees
    .resolve(&Keymap::defaults(), &gwm::tui::modal_keymap::ModalKeymap::defaults())
    .into_iter()
    .map(|(_, l)| l)
    .collect();
  assert_eq!(
    labels,
    vec![
      "new", "del", "boot", // lifecycle
      "open", "git", "review", "yank", // act on the selected worktree
      "filter", "status", "logs", "settings", // find / navigate
      "help", "quit", // global
    ],
    "worktrees footer hints must follow the grouped order"
  );
}

#[test]
fn status_hints_are_grouped_read_then_sidebar_then_navigate_then_global() {
  use gwm::tui::keymap::Keymap;
  let labels: Vec<String> = HintContext::Status
    .resolve(&Keymap::defaults(), &gwm::tui::modal_keymap::ModalKeymap::defaults())
    .into_iter()
    .map(|(_, l)| l)
    .collect();
  assert_eq!(
    labels,
    vec![
      "scroll",
      "fetch", // read the status pane
      "mode",
      "layout", // sidebar mode / layout
      "worktrees",
      "filter",
      "logs",
      "settings", // navigate panes
      "help",
      "quit", // global
    ],
    "status footer hints must follow the grouped order"
  );
}

#[test]
fn confirm_hints_include_delete_branch_toggle_binding() {
  use gwm::tui::keymap::Keymap;
  let resolved = HintContext::Confirm.resolve(&Keymap::defaults(), &gwm::tui::modal_keymap::ModalKeymap::defaults());
  assert!(
    resolved.iter().any(|(k, l)| k == "D" && l == "branch"),
    "Delete Worktree hints should advertise the branch toggle binding: {resolved:?}"
  );
}

#[test]
fn report_close_hint_resolves_user_rebinding() {
  // #219 review (P3): the report overlay footer advertised a literal
  // `Enter/Esc` even after `[tui.keys.report] close` was rebound — the event
  // loop already routes close through the modal keymap, so the footer must
  // follow it instead of printing a stale key.
  use gwm::tui::keymap::Keymap;
  use gwm::tui::modal_keymap::{parse_single, ModalAction, ModalKeymap};
  let km = Keymap::defaults();
  let default = HintContext::Report.resolve(&km, &ModalKeymap::defaults());
  assert!(
    default.iter().any(|(k, l)| k == "Esc" && l == "close"),
    "default report footer must advertise the primary `Esc` close: {default:?}"
  );

  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(ModalAction::ReportClose, vec![parse_single("x").unwrap()])
    .unwrap();
  let resolved = HintContext::Report.resolve(&km, &modal);
  assert!(
    resolved.iter().any(|(k, l)| k == "x" && l == "close"),
    "rebinding report close must change the footer hint: {resolved:?}"
  );
  assert!(
    !resolved.iter().any(|(k, _)| k == "Enter/Esc"),
    "the stale literal `Enter/Esc` must not linger after the rebind: {resolved:?}"
  );
}

#[test]
fn link_input_number_drops_fetch_hint_when_shadowed() {
  // #219 review (P3): in the number-input stage the footer advertises the
  // global `F fetch` fallback, but if a modal verb is rebound onto `F` the
  // event loop resolves `F` as that verb first, leaving the fetch hint
  // pointing at an unreachable action. The footer must drop a global hint
  // whose key is shadowed by a modal binding in the active context.
  use gwm::tui::keymap::Keymap;
  use gwm::tui::modal_keymap::{parse_single, ModalAction, ModalKeymap};
  let km = Keymap::defaults();

  let default = HintContext::LinkInputNumber.resolve(&km, &ModalKeymap::defaults());
  assert!(
    default.iter().any(|(k, l)| k == "F" && l == "fetch"),
    "by default `F fetch` is reachable during number input: {default:?}"
  );

  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(ModalAction::LinkInputSubmit, vec![parse_single("F").unwrap()])
    .unwrap();
  let resolved = HintContext::LinkInputNumber.resolve(&km, &modal);
  assert!(
    !resolved.iter().any(|(_, l)| l == "fetch"),
    "the shadowed `F fetch` hint must be dropped: {resolved:?}"
  );
  assert!(
    resolved.iter().any(|(k, l)| k == "F" && l == "submit"),
    "F now resolves as submit and must show through: {resolved:?}"
  );
}

#[test]
fn help_close_hint_resolves_user_rebinding() {
  // #219 review (P3): same staleness on the Keybindings overlay — rebinding
  // `[tui.keys.help] close` must show through the footer (scroll/pan pairs
  // stay literal because no single resolved key captures `j/k` / `h/l`).
  use gwm::tui::keymap::Keymap;
  use gwm::tui::modal_keymap::{parse_single, ModalAction, ModalKeymap};
  let km = Keymap::defaults();
  let default = HintContext::Help.resolve(&km, &ModalKeymap::defaults());
  assert!(
    default.iter().any(|(k, l)| k == "Esc" && l == "close"),
    "default help footer must advertise the primary `Esc` close: {default:?}"
  );

  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(ModalAction::HelpClose, vec![parse_single("x").unwrap()])
    .unwrap();
  let resolved = HintContext::Help.resolve(&km, &modal);
  assert!(
    resolved.iter().any(|(k, l)| k == "x" && l == "close"),
    "rebinding help close must change the footer hint: {resolved:?}"
  );
  assert!(
    !resolved.iter().any(|(k, _)| k == "Esc/q"),
    "the stale literal `Esc/q` must not linger after the rebind: {resolved:?}"
  );
}
