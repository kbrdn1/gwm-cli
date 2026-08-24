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
fn worktrees_hints_advertise_the_note_key() {
  // Issue #515: `N` acts on the selected worktree the way `x` / `a` /
  // `r` do, so it belongs in the same statusbar family. Shipped without
  // it, the note is reachable only by reading `?` or the docs, which is
  // exactly the discoverability the which-key exists to provide.
  use gwm::tui::keymap::Keymap;
  let km = Keymap::defaults();
  let resolved = HintContext::Worktrees.resolve(&km, &gwm::tui::modal_keymap::ModalKeymap::defaults());
  assert!(
    resolved.iter().any(|(k, l)| k == "N" && l == "note"),
    "the worktrees context must advertise the `N note` hint: {resolved:?}"
  );
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
      "new", "del", "mark", "boot", // lifecycle
      "open", "git", "exec", "agents", "note", "review", "yank", // act on the selected worktree
      "filter", "status", "logs", "settings", // find / navigate
      "help", "quit", // global
    ],
    "worktrees footer hints must follow the grouped order (#453 re-audit: \
     exec and agent sessions joined the act family; clean / mux / macros \
     stay overlay-only — the footer is a teaser, `?` is the manual)"
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
      "wt scroll", // #437: Working Tree pane scroll
      "fetch",     // read the status pane
      "ci checks", // #436: `c` routes to the CI checks overlay here
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
  // `Enter/Esc` even after `[tui.keys.modal.report] close` was rebound — the event
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
  // `[tui.keys.modal.help] close` must show through the footer (scroll/pan pairs
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

// --- the row never paints past its width (issue #563) ----------------------

/// Cells the renderer paints for `s`: the cursor `set_stringn` leaves behind.
/// The oracle for every budget assertion below, because the builders measure
/// with `chars().count()` and asserting on that would agree with them whatever
/// they did.
fn painted(s: &str) -> usize {
  let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 400, 1));
  let (x, _) = buf.set_stringn(0, 0, s, 400, ratatui::style::Style::default());
  usize::from(x)
}

fn painted_line(line: &Line<'_>) -> usize {
  line.spans.iter().map(|s| painted(&s.content)).sum()
}

fn plain_line(line: &Line<'_>) -> String {
  line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn the_statusbar_never_paints_past_its_width_on_ascii() {
  // Pure ASCII, so this is not about the measure: the `…` marker costs two
  // columns whenever anything precedes it (a space, then the glyph), and the
  // hint budget only ever reserved one. `footer_line` reserves both, which is
  // why it does not have this. Asserted as an invariant over a band of widths
  // rather than one case, since the overflow only shows where the truncation
  // lands.
  for w in 20usize..=60 {
    let line = status_line("worktrees", HINTS, "plain message", None, w, &Theme::default());
    assert!(
      painted_line(&line) <= w,
      "at {w} columns the statusbar painted {} cells: {:?}",
      painted_line(&line),
      plain_line(&line)
    );
  }
}

#[test]
fn neither_footer_nor_statusbar_paints_past_its_width_on_wide_glyphs() {
  // The action log is the segment that arrives wide: it carries branch names,
  // paths and error blobs. Both rows budget in characters and pin the log
  // right, so a CJK log under-counted by half and pushed itself off the
  // terminal. Measured before the fix: 100 cells into an 80-column row.
  //
  // CJK, halfwidth katakana and emoji are the fixtures because the measure at
  // fault is `chars().count()`; the discriminant is columns per character.
  let theme = Theme::default();
  for status in [
    "作業作業作業作業作業作業作業作業作業作業",
    "created ｶﾞｶﾞｶﾞｶﾞｶﾞ",
    "pushed 🚀🚀🚀🚀🚀🚀",
  ] {
    for w in [40usize, 60, 80, 120] {
      let footer = footer_line(HINTS, status, w, &theme);
      assert!(
        painted_line(&footer) <= w,
        "footer at {w} columns painted {} cells: {:?}",
        painted_line(&footer),
        plain_line(&footer)
      );
      let bar = status_line("worktrees", HINTS, status, None, w, &theme);
      assert!(
        painted_line(&bar) <= w,
        "statusbar at {w} columns painted {} cells: {:?}",
        painted_line(&bar),
        plain_line(&bar)
      );
    }
  }
}

/// Every character carrying the Unicode `Bidi_Control` property. They are
/// `Cf`, not `Cc`, so `char::is_control` matches none of them, which is the
/// whole reason they need naming (#502).
const BIDI_CONTROLS: &[char] = &[
  '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}',
  '\u{2067}', '\u{2068}', '\u{2069}',
];

#[test]
fn a_bidi_control_in_the_action_log_never_reaches_the_row() {
  // Both rows neutralised `char::is_control`, which is `Cc` only, so the
  // format characters that reorder how a terminal *renders* the text around
  // them went straight through. The log carries branch names and paths, and
  // git's ref rules refuse the ASCII controls but not these, so one arrives
  // with a fetch rather than being typed. The row can then read in an order
  // the bytes do not have, which is the guarantee #506 exists to give at every
  // width-constrained sink.
  //
  // Pre-dates this branch: `dev` leaks the same characters at the same widths.
  // Found by a Codex review of #563 and fixed here rather than left behind,
  // since the fix is the sanitiser these rows already had half of.
  for c in BIDI_CONTROLS {
    let status = format!("opened feat/{c}danger");
    for w in [30usize, 60, 120] {
      let footer = footer_line(HINTS, &status, w, &Theme::default());
      assert!(
        !plain_line(&footer).contains(*c),
        "the footer replayed U+{:04X} from the action log at {w} columns",
        *c as u32
      );
      let bar = status_line("worktrees", HINTS, &status, None, w, &Theme::default());
      assert!(
        !plain_line(&bar).contains(*c),
        "the statusbar replayed U+{:04X} from the action log at {w} columns",
        *c as u32
      );
    }
  }
}

// ── the note editor's mode line (#557, install pass) ───────────────────────

#[test]
fn the_note_footer_names_the_mode_it_is_in() {
  // The bar under the modal is the context line: it already carries the
  // pane name, so the mode belongs in the same slot rather than in a hint.
  // With the mode turned off there is no state to name.
  assert_eq!(HintContext::NoteNormal.label(), "note · NORMAL");
  assert_eq!(HintContext::NoteInsert.label(), "note · INSERT");
  assert_eq!(HintContext::Note.label(), "note");
}

#[test]
fn the_normal_mode_footer_lists_the_motions() {
  // `?` is a printable inside the editor, so the help overlay cannot be
  // reached from it: this bar is the only place the vim verbs are ever
  // spelled out.
  use gwm::tui::keymap::Keymap;
  let resolved = HintContext::NoteNormal.resolve(&Keymap::defaults(), &gwm::tui::modal_keymap::ModalKeymap::defaults());
  for (key, label) in [
    ("hjkl", "move"),
    ("w/b/e", "word"),
    ("i/a/o", "insert"),
    ("x/dd", "delete"),
  ] {
    assert!(
      resolved.iter().any(|(k, l)| k == key && l == label),
      "normal mode must advertise `{key} {label}`: {resolved:?}"
    );
  }
  assert!(
    resolved.iter().any(|(k, l)| k == "Esc" && l == "save & close"),
    "and the way out, which is what `Esc` does from normal mode: {resolved:?}"
  );
}

#[test]
fn the_insert_mode_footer_does_not_promise_esc_saves() {
  // The one verb whose meaning the mode changes. A bar that still said
  // "save & close" here would send the user's first `Esc` somewhere it
  // does not go.
  use gwm::tui::keymap::Keymap;
  let resolved = HintContext::NoteInsert.resolve(&Keymap::defaults(), &gwm::tui::modal_keymap::ModalKeymap::defaults());
  assert!(
    resolved.iter().any(|(k, l)| k == "Esc" && l == "normal mode"),
    "insert mode must say where `Esc` goes: {resolved:?}"
  );
  assert!(
    !resolved.iter().any(|(_, l)| l == "save & close"),
    "and must not promise the gesture it no longer performs: {resolved:?}"
  );
}

#[test]
fn the_note_bullet_hint_follows_the_chord_that_reaches_the_app() {
  // `Ctrl+l` never arrives inside tmux (tmux.nvim binds the whole
  // `Ctrl+h/j/k/l` pane set), so the advertised chord is the one gwm can
  // actually receive.
  use gwm::tui::keymap::Keymap;
  for ctx in [HintContext::Note, HintContext::NoteNormal, HintContext::NoteInsert] {
    let resolved = ctx.resolve(&Keymap::defaults(), &gwm::tui::modal_keymap::ModalKeymap::defaults());
    assert!(
      resolved.iter().any(|(k, l)| k == "Ctrl+u" && l == "bullet"),
      "{:?} must advertise `Ctrl+u bullet`: {resolved:?}",
      ctx.label()
    );
  }
}

#[test]
fn the_help_overlay_teaches_the_normal_mode_verbs() {
  // `?` is a printable inside the note editor, so the overlay cannot be
  // opened from it: whoever wants to read the vim verbs at leisure reads
  // them here, on the surface reachable from the list.
  use gwm::tui::{help_rows, keymap::Keymap, HelpRow};
  let rows = help_rows(
    &Keymap::defaults(),
    &gwm::tui::modal_keymap::ModalKeymap::defaults(),
    HintContext::Help,
  );
  let keys: Vec<String> = rows
    .iter()
    .filter_map(|r| match r {
      HelpRow::Entry { keys, .. } => Some(keys.clone()),
      _ => None,
    })
    .collect();
  for verb in ["h j k l", "w b e", "gg G", "x dd", "i I a A o O"] {
    assert!(
      keys.iter().any(|k| k == verb),
      "the help overlay must document the normal-mode `{verb}` row: {keys:?}"
    );
  }
  assert!(
    keys.iter().any(|k| k == "Ctrl+u"),
    "and the bullet chord as bound, not as it once was: {keys:?}"
  );
}
