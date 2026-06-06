//! Contract tests for the single-line statusline builder (`footer_line`).
//!
//! Issue #180: the footer must render on ONE line, present its key hints as
//! reverse-video badge chips, and keep the status message (the action log)
//! visible at the right edge — hints are what gets truncated when the
//! terminal is narrow, never the log.
//!
//! `footer_line` is a pure builder (like `header_title` / `help_lines`) so the
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
fn keys_render_as_reverse_video_chips_on_the_accent_colour() {
  let line = footer_line(
    HINTS,
    "ready",
    120,
    &Theme {
      accent: Color::Magenta,
      ..Theme::default()
    },
  );
  // At least one span is a chip: reverse-video, accent-coloured, carrying a key.
  let has_chip = line.spans.iter().any(|s| {
    s.style.add_modifier.contains(Modifier::REVERSED) && s.style.fg == Some(Color::Magenta) && s.content.contains('n')
  });
  assert!(has_chip, "no reverse-video accent chip found in footer spans");
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
  // …and it is a reversed accent chip, like the hint badges.
  let has_ctx_chip = line.spans.iter().any(|s| {
    s.style.add_modifier.contains(Modifier::REVERSED)
      && s.style.fg == Some(Color::Green)
      && s.content.contains("worktrees")
  });
  assert!(has_ctx_chip, "context label is not a reversed focus chip: {text:?}");
  assert!(
    line
      .spans
      .iter()
      .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
      .any(|s| s.style.fg == Some(Color::Magenta) && s.content.contains(" n ")),
    "hint key badges should keep the accent colour while context uses focus: {text:?}"
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
  let resolved = HintContext::LinkPrompt.resolve(&km);
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
      !ctx.resolve(&km).is_empty(),
      "context {:?} must advertise hints",
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
  let default = HintContext::Status.resolve(&km);
  assert!(
    default.iter().any(|(k, l)| k == "F" && l == "fetch"),
    "default status hints should advertise the `F` fetch binding: {default:?}"
  );

  km.apply_override(Action::FetchGithub, vec![KeyStroke::parse_chord("Ctrl+g").unwrap()])
    .unwrap();
  let resolved = HintContext::Status.resolve(&km);
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
fn confirm_hints_include_delete_branch_toggle_binding() {
  use gwm::tui::keymap::Keymap;
  let resolved = HintContext::Confirm.resolve(&Keymap::defaults());
  assert!(
    resolved.iter().any(|(k, l)| k == "p" && l == "branch"),
    "Delete Worktree hints should advertise the branch toggle binding: {resolved:?}"
  );
}
