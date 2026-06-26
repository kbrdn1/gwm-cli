//! Unit tests for pure layout helpers in `tui::ui` (issue #187 review
//! follow-up): the middle-ellipsizer that keeps a long path readable in
//! the confirm modal, and the badge-group width used to align the help
//! overlay's per-chord key badges.

use gwm::bootstrap::{BootstrapReport, StepResult};
use gwm::tui::keymap::{Action, KeyStroke, Keymap};
use gwm::tui::state::sidebar::SidebarMode;
use gwm::tui::theme::Theme;
use gwm::tui::ConfirmButton;
use gwm::tui::{
  badge_group_width, bootstrap_report_lines, centered_abs, confirm_buttons_line, create_buttons_line, ellipsize_middle,
  field_input_line, link_prompt_modal_width, link_target_line, modal_hint_line, pane_counter, recent_items_pane_title,
  status_pane_title, type_selector_line, working_tree_counts_footer, working_tree_pane_title,
  working_tree_status_counts, worktrees_pane_title, WorkingTreeCounts, WT_CREATED_ICON, WT_DELETED_ICON,
  WT_MODIFIED_ICON,
};
use gwm::tui::{
  confirm_delete_branch_line, confirm_detail_line, delete_worktree_title, help_body_section_color, help_entry_line,
  help_section_style, issue_pr_pane_title,
};
use ratatui::layout::Rect;
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
fn badge_group_width_single_chord_is_the_bare_chord_width() {
  // Issue #279: chords are flat accent-bold glyphs now, no `` key `` box —
  // so a group's width is the bare chord width, not chord + 2 pad.
  assert_eq!(badge_group_width("q"), 1);
  assert_eq!(badge_group_width("Ctrl-C"), 6);
}

#[test]
fn badge_group_width_splits_comma_chords_with_a_single_space() {
  // `j, Down` renders as `j Down` (flat): 1 + one separator space + 4 → 6.
  assert_eq!(badge_group_width("j, Down"), 1 + 1 + 4);
  // `g g` is a *single* sequential chord (space inside, no comma) → `g g` = 3.
  assert_eq!(badge_group_width("g g"), 3);
}

#[test]
fn badge_group_width_unbound_is_the_bare_placeholder_width() {
  let expected = "(unbound)".chars().count();
  assert_eq!(badge_group_width("(unbound)"), expected);
  assert_eq!(badge_group_width(""), expected);
}

#[test]
fn help_entry_line_renders_flat_accent_chords_not_badges() {
  // Issue #279: the keybindings body drops the reverse-video chord badge
  // for flat accent-bold glyphs (herdr-style). The label stays readable.
  let theme = Theme {
    accent: Color::Magenta,
    ..Theme::default()
  };
  let line = help_entry_line("j, Down", "next", 10, &theme);
  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(text.contains("next"), "label missing: {text:?}");
  // The first chord renders as a bare `j` accent-bold span — no padding box.
  let chord = line
    .spans
    .iter()
    .find(|s| s.content.as_ref() == "j")
    .expect("a bare 'j' chord span");
  assert_eq!(chord.style.fg, Some(Color::Magenta), "chord wears the accent");
  assert!(
    chord.style.add_modifier.contains(Modifier::BOLD),
    "chord is bold: {chord:?}"
  );
  assert!(
    !line
      .spans
      .iter()
      .any(|s| s.style.add_modifier.contains(Modifier::REVERSED)),
    "no chord span should be a reverse-video badge anymore"
  );
}

// ---------------------------------------------------------------------------
// Worktrees pane title + counter (issue #217)
// ---------------------------------------------------------------------------

/// Flatten a `Line` into its visible text (span contents concatenated).
fn title_text(line: &ratatui::text::Line<'_>) -> String {
  line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn worktrees_pane_title_unfiltered_shows_total_with_focus_index() {
  // No filter (empty query, not active) → the `(N)` counter is the full
  // worktree count, and the pane carries the `[1]` focus mnemonic (focusable
  // with the `1` key). Casing is fixed to `Worktrees`.
  let line = worktrees_pane_title("", false, 5, 5, Color::Yellow);
  assert_eq!(title_text(&line), " [1] Worktrees (5) ");
}

#[test]
fn worktrees_pane_title_active_filter_shows_query_cursor_and_ratio() {
  // Issue #262: the live `/` filter renders in the pane title. While typing
  // (active), the title carries the `/query`, a block cursor, and the
  // `(visible/total)` ratio so the user sees how much the filter narrowed.
  let line = worktrees_pane_title("au", true, 3, 5, Color::Yellow);
  assert_eq!(title_text(&line), " [1] Worktrees /au\u{2588} (3/5) ");
}

#[test]
fn worktrees_pane_title_sticky_filter_shows_query_without_cursor() {
  // Sticky (committed) filter: the query stays visible in the title for
  // context, but with no cursor (the bar is closed) and a compact form — no
  // oversized hint.
  let line = worktrees_pane_title("au", false, 3, 5, Color::Yellow);
  assert_eq!(title_text(&line), " [1] Worktrees /au (3/5) ");
}

#[test]
fn worktrees_pane_title_active_empty_query_shows_prompt_and_total() {
  // Just opened the bar with an empty buffer: the `/` prompt + cursor show,
  // but the counter stays the `(total)` form (an empty query matches all).
  let line = worktrees_pane_title("", true, 5, 5, Color::Yellow);
  assert_eq!(title_text(&line), " [1] Worktrees /\u{2588} (5) ");
}

#[test]
fn worktrees_pane_title_paints_the_slash_in_the_filter_colour() {
  // The `/` prompt keeps its dedicated filter colour (historically the
  // `dirty` role) so it reads as an editable affordance, not chrome.
  let line = worktrees_pane_title("au", true, 3, 5, Color::Yellow);
  let slash = line
    .spans
    .iter()
    .find(|s| s.content.as_ref() == "/")
    .expect("title must carry a styled '/' prompt span");
  assert_eq!(slash.style.fg, Some(Color::Yellow));
}

#[test]
fn status_pane_title_carries_the_focus_index() {
  // The sidebar reads as the `[2] Status` pane (focusable with `2`),
  // mirroring `[1] Worktrees`.
  assert_eq!(status_pane_title(), " [2] Status ");
}

#[test]
fn sidebar_subpane_titles_surface_live_bindings() {
  let mut km = Keymap::defaults();
  assert_eq!(issue_pr_pane_title(&km), " Issue / PR [F] ");
  assert_eq!(working_tree_pane_title(&km), " Working Tree [R] ");
  assert_eq!(
    recent_items_pane_title(SidebarMode::Commits, &km),
    " Recent Commits [L] "
  );

  km.apply_override(Action::FetchGithub, vec![KeyStroke::parse_chord("Ctrl+g").unwrap()])
    .unwrap();
  km.apply_override(
    Action::ReviewFullscreen,
    vec![KeyStroke::parse_chord("Ctrl+r").unwrap()],
  )
  .unwrap();
  km.apply_override(
    Action::LazyGitFullscreen,
    vec![KeyStroke::parse_chord("Ctrl+l").unwrap()],
  )
  .unwrap();

  assert_eq!(issue_pr_pane_title(&km), " Issue / PR [Ctrl+g] ");
  assert_eq!(working_tree_pane_title(&km), " Working Tree [Ctrl+r] ");
  assert_eq!(
    recent_items_pane_title(SidebarMode::Commits, &km),
    " Recent Commits [Ctrl+l] "
  );
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

#[test]
fn modal_hint_line_renders_accent_bind_then_muted_action() {
  // Issue #279: hints drop the reverse-video badge for a herdr-style
  // "accent bind + space + muted action" treatment. The key span carries
  // the accent colour + BOLD (no REVERSED box); the label reads muted.
  let theme = Theme {
    accent: Color::Magenta,
    muted: Color::Gray,
    ..Theme::default()
  };
  let line = modal_hint_line(&[("F", "fetch"), ("Esc", "close")], &theme);
  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(text.contains("fetch"), "hint label missing: {text:?}");
  // The bind is the bare key glyph — no surrounding padding box.
  let key = line
    .spans
    .iter()
    .find(|s| s.content.as_ref() == "F")
    .expect("a bare 'F' bind span (no badge padding)");
  assert_eq!(key.style.fg, Some(Color::Magenta), "bind wears the accent");
  assert!(
    key.style.add_modifier.contains(Modifier::BOLD),
    "bind is bold for emphasis"
  );
  assert!(
    !key.style.add_modifier.contains(Modifier::REVERSED),
    "hints no longer use a reverse-video badge: {key:?}"
  );
  // The action reads in the muted role.
  let label = line
    .spans
    .iter()
    .find(|s| s.content.contains("fetch"))
    .expect("a fetch label span");
  assert_eq!(label.style.fg, Some(Color::Gray), "action reads muted");
}

#[test]
fn bootstrap_report_lines_keep_step_logs_as_pane_rows() {
  let report = BootstrapReport {
    steps: vec![
      StepResult::ok_with_detail("cargo fetch", "done"),
      StepResult::skipped("direnv allow", "when false"),
    ],
  };
  let lines = bootstrap_report_lines(Some(&report), &Theme::default());
  let text: String = lines
    .iter()
    .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
    .collect::<Vec<_>>()
    .join("\n");

  assert!(text.contains("cargo fetch"), "step label missing: {text:?}");
  assert!(text.contains("done"), "step detail missing: {text:?}");
  assert!(text.contains("direnv allow"), "skipped label missing: {text:?}");
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
fn rename_buttons_say_rename_not_create() {
  // Codex review on PR #292 (P3): the `c` modal is titled "Rename Worktree"
  // and Enter renames, so its primary button must read "Rename", not the
  // create overlay's "Create".
  let line = gwm::tui::rename_buttons_line(Color::Magenta, Color::Gray);
  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(text.contains("Rename"), "missing Rename label: {text:?}");
  assert!(text.contains("Cancel"), "missing Cancel label: {text:?}");
  assert!(
    !text.contains("Create"),
    "the rename modal must not say Create: {text:?}"
  );
  let primary = line
    .spans
    .iter()
    .find(|s| s.content.contains("Rename"))
    .expect("a Rename span");
  assert!(
    primary.style.add_modifier.contains(Modifier::REVERSED),
    "primary Rename button must be the reversed chip"
  );
  assert_eq!(primary.style.fg, Some(Color::Magenta), "Rename chip carries the accent");
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
fn link_target_keys_track_rebinding_per_context() {
  // #219 review (P3): the Issue / PR direct-pick chips hard-coded `i` / `p`.
  // They must resolve from the active context's modal bindings so a rebind of
  // `[tui.keys.modal.link.choose_target]` (or `[tui.keys.modal.open_menu]`) shows through,
  // and the two contexts stay independent (the whole point of #219).
  use gwm::tui::link_target_keys;
  use gwm::tui::modal_keymap::{parse_single, ModalAction, ModalKeymap};
  use gwm::tui::HintContext;

  assert_eq!(
    link_target_keys(HintContext::LinkPrompt, &ModalKeymap::defaults()),
    ("i".to_string(), "p".to_string()),
    "defaults must keep the historical i / p direct-pick keys"
  );
  assert_eq!(
    link_target_keys(HintContext::OpenMenu, &ModalKeymap::defaults()),
    ("i".to_string(), "p".to_string()),
  );

  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(ModalAction::LinkChooseIssue, vec![parse_single("x").unwrap()])
    .unwrap();
  assert_eq!(
    link_target_keys(HintContext::LinkPrompt, &modal),
    ("x".to_string(), "p".to_string()),
    "rebinding the link choose-target issue key must show through the chip"
  );
  assert_eq!(
    link_target_keys(HintContext::OpenMenu, &modal),
    ("i".to_string(), "p".to_string()),
    "the open-menu chips are an independent context and must not change"
  );
}

#[test]
fn config_edit_footer_hints_track_rebinding() {
  // #219 review (P2): the Settings panel edit footer printed a fixed
  // `Enter save / Esc cancel`. Once `[tui.keys.modal.config.edit]` is rebound the
  // handler stops treating Enter/Esc as save/cancel, so the footer must
  // resolve those hints from the ConfigEdit* modal bindings too.
  use gwm::tui::config_edit_footer_hints;
  use gwm::tui::modal_keymap::{parse_single, ModalAction, ModalKeymap};

  assert_eq!(
    config_edit_footer_hints(&ModalKeymap::defaults()),
    vec![
      ("Enter".to_string(), "save".to_string()),
      ("Esc".to_string(), "cancel".to_string()),
    ],
    "default settings edit footer must read Enter save / Esc cancel"
  );

  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(ModalAction::ConfigEditSubmit, vec![parse_single("Ctrl+s").unwrap()])
    .unwrap();
  assert_eq!(
    config_edit_footer_hints(&modal),
    vec![
      ("Ctrl+s".to_string(), "save".to_string()),
      ("Esc".to_string(), "cancel".to_string()),
    ],
    "rebinding config.edit submit must change the save hint"
  );

  // Unbinding a verb drops it rather than advertising a phantom key.
  let mut unbound = ModalKeymap::defaults();
  unbound.apply_override(ModalAction::ConfigEditCancel, vec![]).unwrap();
  let hints = config_edit_footer_hints(&unbound);
  assert!(
    !hints.iter().any(|(_, l)| l == "cancel"),
    "an unbound cancel must drop from the settings edit footer: {hints:?}"
  );
}

#[test]
fn config_nav_footer_hints_track_rebinding() {
  // #219 review (P3): the Settings panel *nav* footer (non-edit) still printed
  // hard-coded Tab / L / Esc / Enter / Space. Resolve the single-key verbs
  // (section / layer / close / activate) from the Config modal bindings; the
  // j/k scroll pair stays literal (no single resolved key captures it).
  use gwm::tui::config_nav_footer_hints;
  use gwm::tui::modal_keymap::{parse_single, ModalAction, ModalKeymap};
  use gwm::tui::{FieldKind, SettingsTab};

  let all = config_nav_footer_hints(&ModalKeymap::defaults(), SettingsTab::All, None);
  assert_eq!(
    all[0],
    ("j/k".to_string(), "scroll".to_string()),
    "All tab leads with the literal scroll pair"
  );
  assert!(all.iter().any(|(k, l)| k == "Tab" && l == "section"));
  assert!(all.iter().any(|(k, l)| k == "L" && l == "layer"));
  assert!(all.iter().any(|(k, l)| k == "Esc" && l == "close"));

  // An editable field advertises `edit`; a Choice field advertises `cycle`.
  let editable = config_nav_footer_hints(&ModalKeymap::defaults(), SettingsTab::Tui, Some(FieldKind::Text));
  assert!(
    editable.iter().any(|(_, l)| l == "edit"),
    "editable field footer: {editable:?}"
  );
  let choice = config_nav_footer_hints(&ModalKeymap::defaults(), SettingsTab::Tui, Some(FieldKind::Choice));
  assert!(
    choice.iter().any(|(_, l)| l == "cycle"),
    "choice field footer: {choice:?}"
  );

  // Rebinding close + next_tab shows through.
  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(ModalAction::ConfigClose, vec![parse_single("x").unwrap()])
    .unwrap();
  modal
    .apply_override(ModalAction::ConfigNextTab, vec![parse_single("n").unwrap()])
    .unwrap();
  let rebound = config_nav_footer_hints(&modal, SettingsTab::All, None);
  assert!(
    rebound.iter().any(|(k, l)| k == "x" && l == "close"),
    "rebound close: {rebound:?}"
  );
  assert!(
    rebound.iter().any(|(k, l)| k == "n" && l == "section"),
    "rebound section: {rebound:?}"
  );
  assert!(
    !rebound.iter().any(|(k, _)| k == "Tab" || k == "Esc"),
    "stale Tab / Esc must not linger after the rebind: {rebound:?}"
  );

  // Keys tab (issue #294): `activate` advertises `rebind`, not `edit`/`cycle`.
  let keys = config_nav_footer_hints(&ModalKeymap::defaults(), SettingsTab::Keys, None);
  assert!(
    keys.iter().any(|(_, l)| l == "rebind"),
    "Keys tab footer advertises rebind: {keys:?}"
  );
}

#[test]
fn config_capture_footer_hints_differ_by_capture_kind() {
  // Issue #294: while capturing, the footer resolves cancel (+ save for a
  // multi-stroke global chord) from the ConfigEdit modal bindings; a
  // single-stroke modal capture auto-commits, so it advertises the live
  // prompt instead of a save verb.
  use gwm::tui::config_capture_footer_hints;
  use gwm::tui::modal_keymap::{parse_single, ModalAction, ModalKeymap};

  let global = config_capture_footer_hints(&ModalKeymap::defaults(), false);
  assert!(global.iter().any(|(k, l)| k == "Enter" && l == "save"), "{global:?}");
  assert!(
    global.iter().any(|(k, l)| k == "Backspace" && l == "delete"),
    "{global:?}"
  );
  assert!(global.iter().any(|(k, l)| k == "Esc" && l == "cancel"), "{global:?}");

  let modal = config_capture_footer_hints(&ModalKeymap::defaults(), true);
  assert!(
    modal.iter().any(|(_, l)| l == "bind"),
    "single-stroke prompt: {modal:?}"
  );
  assert!(modal.iter().any(|(_, l)| l == "cancel"), "{modal:?}");
  assert!(
    !modal.iter().any(|(_, l)| l == "save"),
    "a single-stroke capture auto-commits, no save verb: {modal:?}"
  );

  // A rebind of the config.edit cancel verb shows through.
  let mut mk = ModalKeymap::defaults();
  mk.apply_override(ModalAction::ConfigEditCancel, vec![parse_single("q").unwrap()])
    .unwrap();
  let rebound = config_capture_footer_hints(&mk, false);
  assert!(rebound.iter().any(|(k, l)| k == "q" && l == "cancel"), "{rebound:?}");
}

#[test]
fn command_logs_footer_hints_track_rebinding() {
  // #219 review (P3): the Command Logs overlay footer hard-coded j/k, g/G, y,
  // Esc. Resolve copy / close from the CommandLogs modal bindings (movement
  // pairs stay literal, as on Help).
  use gwm::tui::command_logs_footer_hints;
  use gwm::tui::modal_keymap::{parse_single, ModalAction, ModalKeymap};

  let default = command_logs_footer_hints(&ModalKeymap::defaults());
  assert!(default.iter().any(|(k, l)| k == "j/k" && l == "scroll"));
  assert!(default.iter().any(|(k, l)| k == "g/G" && l == "top/bottom"));
  assert!(default.iter().any(|(k, l)| k == "y" && l == "copy"));
  assert!(default.iter().any(|(k, l)| k == "Esc" && l == "close"));

  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(ModalAction::CommandLogsCopy, vec![parse_single("c").unwrap()])
    .unwrap();
  modal
    .apply_override(ModalAction::CommandLogsClose, vec![parse_single("x").unwrap()])
    .unwrap();
  let rebound = command_logs_footer_hints(&modal);
  assert!(
    rebound.iter().any(|(k, l)| k == "c" && l == "copy"),
    "rebound copy: {rebound:?}"
  );
  assert!(
    rebound.iter().any(|(k, l)| k == "x" && l == "close"),
    "rebound close: {rebound:?}"
  );
  assert!(
    !rebound.iter().any(|(k, l)| k == "y" && l == "copy"),
    "stale `y copy` must not linger after the rebind: {rebound:?}"
  );
}

#[test]
fn link_prompt_width_stays_compact_on_wide_terminals() {
  assert_eq!(link_prompt_modal_width(80), 64);
  assert_eq!(
    link_prompt_modal_width(120),
    72,
    "Link/Open prompts should be wide enough for Issue/PR summaries but still cap on wide terminals"
  );
}

#[test]
fn help_section_style_uses_body_section_colour() {
  let style = help_section_style(Color::Green);
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

#[test]
fn delete_worktree_title_replaces_confirm_delete() {
  assert_eq!(delete_worktree_title(), "Delete Worktree");
}

#[test]
fn confirm_delete_branch_line_renders_title_case_key_and_value_badges() {
  let line = confirm_delete_branch_line(false, "p", 13, Color::Magenta, Color::Gray);
  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    text.starts_with("Delete Branch"),
    "delete branch label should be Title Case: {text:?}"
  );
  let key = line.spans.iter().find(|s| s.content.contains("p")).expect("key badge");
  assert!(
    key.style.add_modifier.contains(Modifier::REVERSED),
    "toggle key should render as a badge: {key:?}"
  );
  let value = line
    .spans
    .iter()
    .find(|s| s.content.contains("false"))
    .expect("value badge");
  assert!(
    value.style.add_modifier.contains(Modifier::REVERSED),
    "boolean state should render as a badge: {value:?}"
  );
}

#[test]
fn link_target_buttons_keep_equal_visual_widths() {
  let issue = link_target_line("i", "Issue", true, Color::Magenta, Color::Gray);
  let pr = link_target_line("p", "Pull Request", true, Color::Magenta, Color::Gray);
  let issue_chip = issue
    .spans
    .iter()
    .find(|s| s.content.contains("Issue"))
    .expect("issue chip");
  let pr_chip = pr
    .spans
    .iter()
    .find(|s| s.content.contains("Pull Request"))
    .expect("pr chip");
  assert_eq!(
    issue_chip.content.chars().count(),
    pr_chip.content.chars().count(),
    "Link/Open action buttons should align to the same width"
  );
}

#[test]
fn help_body_section_colour_is_distinct_from_subtitle_colour() {
  let theme = Theme {
    branch: Color::Green,
    locked: Color::Blue,
    ..Theme::default()
  };
  assert_eq!(help_body_section_color(&theme), Color::Blue);
}

// `centered_abs` (issue #243, plan P7): the absolute-width centering shared by
// the open-menu and link-prompt modals, extracted from two byte-for-byte
// identical `Rect{}` blocks. Characterisation pins the contract so the
// extraction (and the `centered_h` delegation) stays behaviour-preserving.

#[test]
fn centered_abs_centers_a_box_that_fits() {
  // Nominal: 40×10 box in a 100×40 area → centred both axes.
  assert_eq!(
    centered_abs(
      40,
      10,
      Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40
      }
    ),
    Rect {
      x: 30,
      y: 15,
      width: 40,
      height: 10
    }
  );
}

#[test]
fn centered_abs_honours_a_non_zero_area_origin() {
  // The area's own x/y offset is added to the centred position.
  assert_eq!(
    centered_abs(
      20,
      6,
      Rect {
        x: 5,
        y: 3,
        width: 50,
        height: 20
      }
    ),
    Rect {
      x: 20,
      y: 10,
      width: 20,
      height: 6
    }
  );
}

#[test]
fn centered_abs_caps_width_wider_than_the_area() {
  // Width larger than the area is clamped to the area width; x collapses to 0.
  assert_eq!(
    centered_abs(
      150,
      10,
      Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40
      }
    ),
    Rect {
      x: 0,
      y: 15,
      width: 100,
      height: 10
    }
  );
}

#[test]
fn centered_abs_caps_height_taller_than_the_area() {
  // Edge case the inline modal blocks relied on: a box taller than the
  // terminal is clamped to the area height and pinned to the top (y = area.y),
  // because `saturating_sub` floors the vertical gap at 0.
  assert_eq!(
    centered_abs(
      40,
      50,
      Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40
      }
    ),
    Rect {
      x: 30,
      y: 0,
      width: 40,
      height: 40
    }
  );
}

// ---- working_tree_status_counts / footer (issue #287) ----------------------

#[test]
fn working_tree_status_counts_buckets_each_file_once() {
  // One NUL-delimited (`-z`) record per porcelain family; created wins over
  // deleted wins over modified so every record increments exactly one
  // counter. The rename (`R`) carries a trailing source field (`orig.rs`)
  // that the shared parser drops, so it still counts once.
  let status = "?? new.rs\0 M mod.rs\0 D del.rs\0A  added.rs\0AM both.rs\0R  renamed.rs\0orig.rs\0";
  let c = working_tree_status_counts(status);
  assert_eq!(c.created, 3, "?? + A + AM → created: {c:?}");
  assert_eq!(c.modified, 2, " M + R → modified: {c:?}");
  assert_eq!(c.deleted, 1, " D → deleted: {c:?}");
}

#[test]
fn working_tree_status_counts_empty_string_is_clean() {
  assert!(working_tree_status_counts("").is_empty());
}

#[test]
fn working_tree_counts_footer_is_none_when_all_zero() {
  // A clean tree must produce no footer at all (rather than a bare ` 0 `).
  let counts = WorkingTreeCounts::default();
  assert!(working_tree_counts_footer(&counts, &Theme::default()).is_none());
}

#[test]
fn working_tree_counts_footer_shows_only_nonzero_colored_segments() {
  let counts = WorkingTreeCounts {
    created: 3,
    modified: 0,
    deleted: 1,
  };
  let line = working_tree_counts_footer(&counts, &Theme::default()).expect("non-empty counts → footer");

  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    text.contains(WT_CREATED_ICON) && text.contains('3'),
    "created segment shown: {text:?}"
  );
  assert!(
    text.contains(WT_DELETED_ICON) && text.contains('1'),
    "deleted segment shown: {text:?}"
  );
  assert!(
    !text.contains(WT_MODIFIED_ICON),
    "a zero count must be omitted entirely: {text:?}"
  );

  // Colour roles must be wired to the *theme*, not hardcoded literals.
  // Drive a theme whose `untracked` / `prunable` are unique non-default
  // `Rgb` values and assert those exact colours land — a `Color::Green`
  // hardcode (which equals the default `untracked`) would pass against the
  // default theme but fail here (mirrors the #170/#211 audit rule).
  let theme = Theme {
    untracked: Color::Rgb(1, 2, 3),
    prunable: Color::Rgb(4, 5, 6),
    ..Theme::default()
  };
  let line = working_tree_counts_footer(&counts, &theme).unwrap();
  let created_span = line.spans.iter().find(|s| s.content.contains(WT_CREATED_ICON)).unwrap();
  assert_eq!(
    created_span.style.fg,
    Some(Color::Rgb(1, 2, 3)),
    "created paints the `untracked` role"
  );
  let deleted_span = line.spans.iter().find(|s| s.content.contains(WT_DELETED_ICON)).unwrap();
  assert_eq!(
    deleted_span.style.fg,
    Some(Color::Rgb(4, 5, 6)),
    "deleted paints the `prunable` role"
  );
}

#[test]
fn picker_window_keeps_the_selection_visible() {
  // #325 overlay polish: the picker scrolls to keep the highlight in view.
  use gwm::tui::picker_window;
  // Fits within the budget → the whole list, no scroll.
  assert_eq!(picker_window(3, 0, 5), (0, 3));
  assert_eq!(picker_window(5, 4, 5), (0, 5));
  // Overflows → a `max`-row window clamped to the bounds, selection inside.
  assert_eq!(picker_window(10, 0, 4), (0, 4));
  assert_eq!(picker_window(10, 9, 4), (6, 10));
  let (s, e) = picker_window(10, 5, 4);
  assert!(s <= 5 && 5 < e && e - s == 4, "selection in a 4-row window: {s}..{e}");
  // Degenerate inputs are safe.
  assert_eq!(picker_window(0, 0, 5), (0, 0));
  assert_eq!(picker_window(5, 2, 0), (0, 5));
}

#[test]
fn reclaim_size_color_is_a_magnitude_heatmap() {
  // #325 overlay polish: green (small) → yellow (medium) → red (large).
  use gwm::tui::reclaim_size_color;
  use gwm::tui::theme::Theme;
  let t = Theme::default();
  const MIB: u64 = 1024 * 1024;
  assert_eq!(reclaim_size_color(0, &t), t.clean, "zero → green");
  assert_eq!(reclaim_size_color(10 * MIB, &t), t.clean, "small → green");
  assert_eq!(reclaim_size_color(50 * MIB, &t), t.dirty, "50 MiB boundary → yellow");
  assert_eq!(reclaim_size_color(200 * MIB, &t), t.dirty, "medium → yellow");
  assert_eq!(reclaim_size_color(500 * MIB, &t), t.prunable, "500 MiB boundary → red");
  assert_eq!(reclaim_size_color(3 * 1024 * MIB, &t), t.prunable, "large → red");
}

#[test]
fn clean_dir_icon_matches_the_ecosystem() {
  // #334 polish: each reclaimable dir gets a nerd-font glyph matched to its
  // ecosystem; unknown names fall back to the generic folder. Leading dots
  // are ignored so `.venv` matches like `venv`.
  use gwm::tui::clean_dir_icon;
  let folder = clean_dir_icon("some-unknown-dir");
  assert_ne!(clean_dir_icon("node_modules"), folder, "node_modules has its own icon");
  assert_ne!(clean_dir_icon("target"), folder, "target (Rust) has its own icon");
  assert_eq!(
    clean_dir_icon(".venv"),
    clean_dir_icon("venv"),
    "leading dot is ignored"
  );
  assert_eq!(clean_dir_icon(".cache"), clean_dir_icon("cache"));
  // Distinct ecosystems get distinct glyphs.
  assert_ne!(clean_dir_icon("node_modules"), clean_dir_icon("target"));
  assert_ne!(clean_dir_icon("vendor"), clean_dir_icon("venv"));
  // Every glyph is a single non-empty token.
  for d in [
    "node_modules",
    "target",
    "vendor",
    ".venv",
    "dist",
    ".cache",
    "coverage",
    "whatever",
  ] {
    assert!(!clean_dir_icon(d).is_empty(), "icon for {d:?} must be non-empty");
  }
}

#[test]
fn overlay_modal_width_is_wider_but_clamped() {
  // #334 polish: the exec/clean overlays use more horizontal space than the
  // link-prompt modal on a roomy terminal, but stay readable / clamped.
  use gwm::tui::{link_prompt_modal_width, overlay_modal_width};
  // On a wide terminal it is meaningfully wider than the 72-col link modal.
  assert!(overlay_modal_width(160) > link_prompt_modal_width(160));
  assert!(overlay_modal_width(120) >= 72);
  // Clamped: never wider than the terminal, never past the 88 ceiling.
  assert!(overlay_modal_width(300) <= 88);
  assert!(overlay_modal_width(40) <= 40);
  assert!(overlay_modal_width(50) >= 45, "narrow terminals still get a usable box");
}
