//! Unit tests for pure layout helpers in `tui::ui` (issue #187 review
//! follow-up): the middle-ellipsizer that keeps a long path readable in
//! the confirm modal, and the badge-group width used to align the help
//! overlay's per-chord key badges.

use gwm::tui::ConfirmButton;
use gwm::tui::{
  badge_group_width, confirm_buttons_line, create_buttons_line, ellipsize_middle, field_input_line, link_choose_hint,
  link_input_hint, link_prompt_modal_width, link_target_line, pane_counter, status_pane_title, type_selector_line,
  worktrees_pane_title,
};
use gwm::tui::{confirm_detail_line, help_section_style};
use ratatui::style::{Color, Modifier, Style};

#[test]
fn ellipsize_middle_returns_input_when_it_fits() {
  assert_eq!(ellipsize_middle("short", 10), "short");
  // Exactly `max` is left untouched (no ellipsis when nothing is cut).
  assert_eq!(ellipsize_middle("exactly10!", 10), "exactly10!");
}

#[test]
fn ellipsize_middle_keeps_head_and_tail_around_the_ellipsis() {
  // A long path keeps both its root and the worktree name at the end.
  let s = "/Users/kb/Projects/Flippad/worktrees/chore-880-drop-deprecated";
  let out = ellipsize_middle(s, 20);
  assert_eq!(out.chars().count(), 20, "must fit exactly within max");
  assert!(out.contains('…'), "must carry the middle ellipsis: {out}");
  assert!(out.starts_with("/Users"), "keeps the head: {out}");
  // Tail length is `max - 1 - ceil((max-1)/2)` chars, so a suffix that
  // fits inside it survives (the head/tail split is the contract, not a
  // specific cut point).
  assert!(out.ends_with("ecated"), "keeps the tail: {out}");
}

#[test]
fn ellipsize_middle_degrades_to_a_single_ellipsis_when_too_narrow() {
  assert_eq!(ellipsize_middle("anything", 1), "…");
  assert_eq!(ellipsize_middle("anything", 0), "…");
}

#[test]
fn ellipsize_middle_counts_chars_not_bytes() {
  // Multi-byte segments must not be sliced mid-codepoint, and the budget
  // is measured in chars.
  let s = "~/Projets/dépôt-très-long/branche-accentuée-éàü";
  let out = ellipsize_middle(s, 15);
  assert_eq!(out.chars().count(), 15);
  assert!(out.contains('…'));
}

#[test]
fn badge_group_width_single_chord_is_chord_plus_two_pad() {
  // ` q ` → 1 + 2.
  assert_eq!(badge_group_width("q"), 3);
  // ` Ctrl-C ` → 6 + 2.
  assert_eq!(badge_group_width("Ctrl-C"), 8);
}

#[test]
fn badge_group_width_splits_comma_chords_into_separate_badges() {
  // `j, Down` renders as `[ j ] [ Down ]`:
  //   ` j ` = 3, one separator space, ` Down ` = 6  → 10.
  assert_eq!(badge_group_width("j, Down"), 3 + 1 + 6);
  // `g g` is a *single* sequential chord (space inside, no comma) → one
  // badge ` g g ` = 5.
  assert_eq!(badge_group_width("g g"), 5);
}

#[test]
fn badge_group_width_unbound_renders_one_muted_badge() {
  let expected = "(unbound)".chars().count() + 2;
  assert_eq!(badge_group_width("(unbound)"), expected);
  assert_eq!(badge_group_width(""), expected);
}

// ---------------------------------------------------------------------------
// Worktrees pane title + counter (issue #217)
// ---------------------------------------------------------------------------

#[test]
fn worktrees_pane_title_unfiltered_shows_total_with_focus_index() {
  // No active filter → the `(N)` counter is the full worktree count, and the
  // pane carries the `[1]` focus mnemonic (focusable with the `1` key). The
  // casing is fixed to `Worktrees` (was lowercase `worktrees`).
  assert_eq!(worktrees_pane_title(true, 5, 5), " [1] Worktrees (5) ");
}

#[test]
fn worktrees_pane_title_filtered_shows_visible_over_total() {
  // Active filter → `(visible/total)` so the user sees how much the filter
  // narrowed the list.
  assert_eq!(worktrees_pane_title(false, 3, 5), " [1] Worktrees (3/5) ");
}

#[test]
fn status_pane_title_carries_the_focus_index() {
  // The sidebar reads as the `[2] Status` pane (focusable with `2`),
  // mirroring `[1] Worktrees`.
  assert_eq!(status_pane_title(), " [2] Status ");
}

#[test]
fn pane_counter_is_blank_when_nothing_visible() {
  // Empty list → no `N of M` footer at all (mirrors the Recent Commits
  // section, which drops its counter when there is nothing to scroll).
  assert_eq!(pane_counter(0, 0), None);
}

#[test]
fn pane_counter_formats_selected_of_visible() {
  // Bottom-right footer of the worktrees pane, lazygit-style: the 1-based
  // selected position over the visible count (e.g. `3 of 12`).
  assert_eq!(pane_counter(3, 12).as_deref(), Some(" 3 of 12 "));
}

// ---------------------------------------------------------------------------
// Confirm modal buttons (issue #217)
// ---------------------------------------------------------------------------

#[test]
fn confirm_buttons_render_as_chips_without_brackets() {
  // Issue #217: the confirm modal buttons are flat coloured chips
  // (` Confirm ` / ` Cancel `), not the pre-#217 `[ Confirm ]` / `[ Cancel ]`
  // bracket pairs. The focused button wears the reversed-accent chip; the
  // idle one reads muted.
  let line = confirm_buttons_line(ConfirmButton::Cancel, Color::Magenta, Color::Gray);
  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(text.contains("Confirm"), "missing Confirm label: {text:?}");
  assert!(text.contains("Cancel"), "missing Cancel label: {text:?}");
  assert!(!text.contains('['), "square bracket leaked into a chip: {text:?}");
  assert!(!text.contains(']'), "square bracket leaked into a chip: {text:?}");

  // The focused button (Cancel here — the safe default) is the reversed chip.
  let cancel = line
    .spans
    .iter()
    .find(|s| s.content.contains("Cancel"))
    .expect("a Cancel span");
  assert!(
    cancel.style.add_modifier.contains(Modifier::REVERSED),
    "focused Cancel button must be a reversed chip"
  );
  let confirm = line
    .spans
    .iter()
    .find(|s| s.content.contains("Confirm"))
    .expect("a Confirm span");
  assert!(
    !confirm.style.add_modifier.contains(Modifier::REVERSED),
    "idle Confirm button must not be reversed"
  );
}

// ---------------------------------------------------------------------------
// Create modal: buttons, horizontal type selector, single-line bg inputs
// (issue #217 follow-up — the modal polish pass)
// ---------------------------------------------------------------------------

#[test]
fn create_buttons_render_as_chips_with_create_highlighted() {
  // The create overlay grows a button row mirroring the confirm modal's
  // flat coloured chips (no `[ ]` brackets). Unlike confirm — whose safe
  // default is Cancel — the non-destructive create primes `Create` as the
  // reversed-accent chip; `Cancel` reads muted.
  let line = create_buttons_line(Color::Magenta, Color::Gray);
  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(text.contains("Create"), "missing Create label: {text:?}");
  assert!(text.contains("Cancel"), "missing Cancel label: {text:?}");
  assert!(!text.contains('['), "square bracket leaked into a chip: {text:?}");
  assert!(!text.contains(']'), "square bracket leaked into a chip: {text:?}");

  let create = line
    .spans
    .iter()
    .find(|s| s.content.contains("Create"))
    .expect("a Create span");
  assert!(
    create.style.add_modifier.contains(Modifier::REVERSED),
    "primary Create button must be the reversed chip"
  );
  assert_eq!(create.style.fg, Some(Color::Magenta), "Create chip carries the accent");
  let cancel = line
    .spans
    .iter()
    .find(|s| s.content.contains("Cancel"))
    .expect("a Cancel span");
  assert!(
    !cancel.style.add_modifier.contains(Modifier::REVERSED),
    "idle Cancel button must not be reversed"
  );
}

#[test]
fn type_selector_shows_horizontal_arrows_and_focus_accent() {
  // The branch-type field is a horizontal `‹ name ›` selector (was a
  // bordered up/down box). Focused, the selected type reads in the accent.
  let focused = type_selector_line("type", "feat", "a new feature", true, Color::Magenta, Color::Gray);
  let text: String = focused.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    text.contains('‹') && text.contains('›'),
    "horizontal arrows missing: {text:?}"
  );
  assert!(text.contains("type"), "field label missing: {text:?}");
  assert!(text.contains("feat"), "type name missing: {text:?}");
  assert!(text.contains("a new feature"), "description missing: {text:?}");

  let name = focused
    .spans
    .iter()
    .find(|s| s.content.contains("feat"))
    .expect("a type-name span");
  assert_eq!(name.style.fg, Some(Color::Magenta), "focused type reads in the accent");
  assert!(
    name.style.add_modifier.contains(Modifier::BOLD),
    "focused type name is bold"
  );
  // The focused selection reads as a reversed-accent chip (like the
  // confirm / create buttons) so it stands out as an editable control.
  assert!(
    name.style.add_modifier.contains(Modifier::REVERSED),
    "focused type name is a reversed chip"
  );

  // Idle → the name is not painted in the accent.
  let idle = type_selector_line("type", "feat", "x", false, Color::Magenta, Color::Gray);
  let iname = idle
    .spans
    .iter()
    .find(|s| s.content.contains("feat"))
    .expect("a type-name span");
  assert_ne!(
    iname.style.fg,
    Some(Color::Magenta),
    "idle type must not wear the accent"
  );
}

#[test]
fn field_input_fills_a_single_row_with_a_background() {
  // The issue / description fields are single-row inputs with a background
  // surface (was a 3-row bordered box). Idle shows the surface bg and no
  // cursor; focused brightens to the accent bg and shows a `_` cursor.
  let idle = field_input_line("issue", "123", false, 20, Color::Magenta, Color::Gray, Color::DarkGray);
  let text: String = idle.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(text.contains("issue"), "label missing: {text:?}");
  assert!(text.contains("123"), "value missing: {text:?}");
  assert!(!text.contains('_'), "idle field must not show a cursor: {text:?}");
  let val = idle
    .spans
    .iter()
    .find(|s| s.content.contains("123"))
    .expect("a value span");
  assert_eq!(val.style.bg, Some(Color::DarkGray), "idle input wears the surface bg");

  let focused = field_input_line("issue", "123", true, 20, Color::Magenta, Color::Gray, Color::DarkGray);
  let ftext: String = focused.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(ftext.contains('_'), "focused field shows a cursor: {ftext:?}");
  let fval = focused
    .spans
    .iter()
    .find(|s| s.content.contains("123"))
    .expect("a value span");
  assert_eq!(fval.style.bg, Some(Color::Magenta), "focused input bg = accent");
}

#[test]
fn link_target_line_highlights_the_selected_row() {
  // The link prompt's ChooseTarget step is a vertical selectable list
  // (#217): each row shows its direct-pick key + label, and the
  // highlighted row reads in the accent while the others stay muted.
  let selected = link_target_line("i", "Issue", true, Color::Magenta, Color::Gray);
  let stext: String = selected.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(stext.contains("Issue"), "label missing: {stext:?}");
  assert!(stext.contains('i'), "pick key missing: {stext:?}");
  assert!(
    selected.spans.iter().any(|s| s.style.fg == Some(Color::Magenta)),
    "the selected row must read in the accent: {stext:?}"
  );
  let selected_chip = selected
    .spans
    .iter()
    .find(|s| s.content.contains("Issue"))
    .expect("selected label span");
  assert!(
    selected_chip.style.add_modifier.contains(Modifier::REVERSED),
    "selected link row must use the same reversed chip treatment as modal buttons: {selected_chip:?}"
  );
  assert!(
    !stext.contains('›'),
    "the selected link row should read as a chip, not a marker-prefixed list item: {stext:?}"
  );

  let idle = link_target_line("p", "Pull Request", false, Color::Magenta, Color::Gray);
  let itext: String = idle.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(itext.contains("Pull Request"), "label missing: {itext:?}");
  assert!(
    idle.spans.iter().all(|s| s.style.fg != Some(Color::Magenta)),
    "an unselected row must not wear the accent: {itext:?}"
  );
}

#[test]
fn link_prompt_width_stays_compact_on_wide_terminals() {
  assert_eq!(link_prompt_modal_width(80), 40);
  assert_eq!(
    link_prompt_modal_width(120),
    42,
    "Link prompt should cap instead of growing to half the terminal"
  );
}

#[test]
fn link_prompt_hints_fit_the_80_col_modal_budget() {
  // At 80 columns the compact Link modal remains 40 columns wide, and the
  // rounded border + horizontal padding leave 34 cells for content. The
  // visual smoke caught the previous long hints clipping at exactly this
  // common terminal width.
  const INNER_WIDTH_AT_80_COLS: usize = 34;

  for hint in [link_choose_hint(), link_input_hint()] {
    assert!(
      hint.chars().count() <= INNER_WIDTH_AT_80_COLS,
      "Link prompt hint clips at 80 cols: {hint:?}"
    );
  }
}

#[test]
fn help_section_style_uses_body_section_colour() {
  let style = help_section_style(Color::Magenta, Color::Green);
  assert_eq!(
    style.fg,
    Some(Color::Green),
    "body section headings should not reuse the modal title accent"
  );
  assert!(style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn confirm_detail_line_aligns_label_column() {
  let value_style = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
  let line = confirm_detail_line("branch", "feat/#220", 8, Color::Gray, value_style);
  assert_eq!(line.spans[0].content.as_ref(), "branch    ");
  assert_eq!(line.spans[0].style.fg, Some(Color::Gray));
  assert_eq!(line.spans[1].content.as_ref(), "feat/#220");
  assert_eq!(line.spans[1].style, value_style);
}
