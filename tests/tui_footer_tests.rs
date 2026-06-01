//! Contract tests for the single-line statusline builder (`footer_line`).
//!
//! Issue #180: the footer must render on ONE line, present its key hints as
//! reverse-video badge chips, and keep the status message (the action log)
//! visible at the right edge — hints are what gets truncated when the
//! terminal is narrow, never the log.
//!
//! `footer_line` is a pure builder (like `header_title` / `help_lines`) so the
//! layout contract is pinned here without spinning up a ratatui backend.

use gwm::tui::footer_line;
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
  let line = footer_line(HINTS, "press ? for help", 120, Color::Cyan);
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
  let line = footer_line(HINTS, "opened foo", 120, Color::Cyan);
  let text = plain(&line);
  // The log lives at the very end of the row, in priority.
  assert!(
    text.trim_end().ends_with("[opened foo]"),
    "status not pinned at end: {text:?}"
  );
}

#[test]
fn hints_are_truncated_with_ellipsis_when_narrow_but_status_survives() {
  let line = footer_line(HINTS, "opened foo", 40, Color::Cyan);
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
  let line = footer_line(HINTS, "x", 5, Color::Cyan);
  let text = plain(&line);
  assert!(display_width(&line) <= 5, "overflowed 5: {text:?}");
  assert!(text.contains("[x]") || text.contains('x'), "status missing: {text:?}");
  assert!(!text.contains("new"), "hint label leaked into a tiny footer: {text:?}");
}

#[test]
fn keys_render_as_reverse_video_chips_on_the_accent_colour() {
  let line = footer_line(HINTS, "ready", 120, Color::Magenta);
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
  let line = footer_line(HINTS, "template error:\nbad line\r\nsecond", 80, Color::Cyan);
  let text = plain(&line);
  assert!(!text.contains('\n'), "status newline leaked into footer: {text:?}");
  assert!(!text.contains('\r'), "status CR leaked into footer: {text:?}");
  assert!(display_width(&line) <= 80, "overflowed 80: {text:?}");
}

#[test]
fn zero_width_emits_an_empty_line_without_overflowing() {
  // Degenerate terminal width: the row must not be wider than asked. The
  // pre-fix `trunc()` floor returned "…" (width 1) for width 0 (PR #183 review).
  let line = footer_line(HINTS, "anything", 0, Color::Cyan);
  assert_eq!(
    display_width(&line),
    0,
    "footer overflowed a zero width: {:?}",
    plain(&line)
  );
}
