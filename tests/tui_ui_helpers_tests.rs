//! Unit tests for pure layout helpers in `tui::ui` (issue #187 review
//! follow-up): the middle-ellipsizer that keeps a long path readable in
//! the confirm modal, and the badge-group width used to align the help
//! overlay's per-chord key badges.

use gwm::bootstrap::{BootstrapReport, StepResult};
use gwm::tui::keymap::{Action, KeyStroke, Keymap};
use gwm::tui::state::sidebar::SidebarMode;
use gwm::tui::theme::{preset_names, Theme};
use gwm::tui::ConfirmButton;
use gwm::tui::{
  badge_group_width, bootstrap_report_lines, centered_abs, compact_header_fill, compact_header_line,
  compact_header_style, confirm_buttons_line, create_buttons_line, ellipsize_middle, field_input_line,
  form_field_scroll, link_prompt_modal_width, link_target_line, modal_hint_line, pad_cells, pane_counter,
  recent_items_pane_title, status_pane_title, type_selector_line, working_tree_counts_footer, working_tree_pane_title,
  working_tree_status_counts, worktrees_pane_title, WorkingTreeCounts, WT_CREATED_ICON, WT_DELETED_ICON,
  WT_MODIFIED_ICON,
};
use gwm::tui::{
  confirm_delete_branch_line, confirm_detail_line, delete_worktree_title, help_body_section_color, help_entry_line,
  help_section_style, issue_pr_pane_title,
};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

/// Cells the renderer actually paints for `s`: the cursor `set_stringn`
/// leaves behind. The oracle for every cell-budget assertion here that
/// `unicode-width` alone would get wrong.
fn painted(s: &str) -> usize {
  let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 120, 1));
  let (x, _) = buf.set_stringn(0, 0, s, 120, Style::default());
  usize::from(x)
}

/// Same oracle for a whole `Line`, span by span — which is how ratatui draws
/// one. Not `Line::width()`: that sums `Span::width()`, i.e.
/// `UnicodeWidthStr::width`, which is the measure under test rather than the
/// one the renderer applies (issue #562).
fn painted_line(line: &ratatui::text::Line<'_>) -> usize {
  line.spans.iter().map(|s| painted(&s.content)).sum()
}

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
  assert_eq!(painted(&out), 20, "must fit exactly within max");
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
fn ellipsize_middle_never_slices_a_codepoint() {
  // Multi-byte segments must not be sliced mid-codepoint. These accents
  // are one cell each, so the budget spends exactly.
  let s = "~/Projets/dépôt-très-long/branche-accentuée-éàü";
  let out = ellipsize_middle(s, 15);
  assert_eq!(painted(&out), 15);
  assert!(out.contains('…'));
}

#[test]
fn ellipsize_middle_measures_in_terminal_cells_not_chars() {
  // Issue #554. Every caller's budget is the width of a ratatui rect, in
  // cells. A CJK path counts one char per two columns drawn, so a char
  // count judged this short, returned it whole, and ratatui clipped the
  // tail the middle ellipsis exists to keep. Same measure, and the same
  // per-glyph walk, as `compact_header_line`.
  let s = "/tmp/作業ディレクトリ/深い/入れ子/TAIL";
  assert!(s.chars().count() < 30, "the fixture must fit the budget in chars");
  assert!(painted(s) > 30, "and overflow it in cells");

  let out = ellipsize_middle(s, 30);
  assert!(painted(&out) <= 30, "must fit the cell budget: {} cells", painted(&out));
  assert!(out.contains('…'), "must carry the middle ellipsis: {out}");
  assert!(out.starts_with("/tmp"), "keeps the head: {out}");
  assert!(out.ends_with("TAIL"), "keeps the tail: {out}");
}

#[test]
fn ellipsize_middle_drops_a_wide_glyph_rather_than_half_drawing_it() {
  // A 2-cell glyph against a 1-cell remainder is dropped whole, so the
  // result is `<= max` cells rather than exactly `max` — half a glyph is
  // not a thing a terminal can draw. The budget is never overspent.
  for max in 2..=12 {
    let out = ellipsize_middle("作業ディレクトリの名前", max);
    assert!(
      painted(&out) <= max,
      "max={max} overspent: {out:?} is {} cells",
      painted(&out)
    );
  }
}

#[test]
fn ellipsize_middle_measures_sequences_whole_not_char_by_char() {
  // Codex review on PR #561. `unicode-width` reads `"*\u{FE0F}"` as 2 cells
  // but its two chars in isolation as 1 and 0, so a per-char sum undercounts
  // every variation-selector sequence. Measured before the fix: this string
  // budgeted at 30 came back 59 cells wide.
  let s = "*\u{FE0F}".repeat(20);
  assert_eq!(painted(&s), 40, "the fixture must overflow the budget");
  for max in 2..=30 {
    let out = ellipsize_middle(&s, max);
    assert!(
      painted(&out) <= max,
      "max={max} overspent: {out:?} is {} cells",
      painted(&out)
    );
  }
}

#[test]
fn ellipsize_middle_cuts_on_grapheme_boundaries() {
  // Codex review on PR #561, second pass. Walking codepoints let the cut
  // fall between a base and its combining mark: `("作作\u{0301}", 3)` came
  // back as `"…\u{0301}"`, an accent landing on the ellipsis and not one
  // character of the path kept.
  let out = ellipsize_middle("作作\u{0301}", 3);
  assert!(painted(&out) <= 3, "{out:?} is {} cells", painted(&out));
  assert!(
    !out.starts_with('…') || out.chars().nth(1) != Some('\u{0301}'),
    "a combining mark must not be left to attach to the ellipsis: {out:?}"
  );
  // A tail that keeps the accented glyph keeps its base with it.
  let out = ellipsize_middle("/tmp/a/e\u{0301}", 5);
  assert!(painted(&out) <= 5, "{out:?} is {} cells", painted(&out));
  assert!(
    !out.contains('\u{0301}') || out.contains("e\u{0301}"),
    "the mark travels with its base: {out:?}"
  );
}

#[test]
fn ellipsize_middle_budgets_the_cells_the_renderer_paints() {
  // Codex review on PR #561, third pass, verified against `set_stringn`
  // rather than taken on the report. `UnicodeWidthStr::width` on the whole
  // string undercounts twice over, and both cases are real text:
  //
  //   "لالالا"   unicode-width 3, painted 6 (lam-alef reads as a ligature)
  //   "ｶﾞｶﾞｶﾞ"   unicode-width 3, painted 6 (U+FF9E is Grapheme_Extend, but
  //              terminals give the halfwidth dakuten its own cell)
  //
  // Either one sailed through the early return and overflowed the frame.
  for s in ["لالالا", "ｶﾞｶﾞｶﾞ", "ﾊﾟﾊﾟﾊﾟﾊﾟ", "لا/tmp/لالا"] {
    assert!(
      painted(s) > s.width(),
      "the fixture must be one unicode-width gets wrong: {s:?}"
    );
    for max in 2..=painted(s) {
      let out = ellipsize_middle(s, max);
      assert!(
        painted(&out) <= max,
        "{s:?} at max={max} came back {} painted cells: {out:?}",
        painted(&out)
      );
    }
  }
}

#[test]
fn ellipsize_middle_survives_a_control_character() {
  // Codex review on PR #561, fourth pass. `CellWidth::cell_width` carries a
  // `debug_assert!` that a one-byte ASCII grapheme is not a control: ratatui
  // filters controls in `set_stringn` before measuring, so the assert states
  // the contract rather than leaving it implied. `ellipsize_middle` does not
  // sanitise (that is `trunc`'s job, #506) and a Unix path may legally hold a
  // newline, so measuring one directly panicked every debug build, this suite
  // included.
  let s = "/tmp/gwm\ttest/a\nb/一二三四五六七八九十/TAIL";
  let out = ellipsize_middle(s, 20);
  assert!(painted(&out) <= 20, "{out:?} paints {} cells", painted(&out));
  assert!(out.ends_with("TAIL"), "keeps the tail: {out:?}");
  // A control measures nothing, exactly as the renderer treats it, so it
  // never spends budget a drawable glyph could have had.
  assert_eq!(painted("a\tb"), painted("ab"));
}

#[test]
fn pad_cells_pads_to_the_cells_the_renderer_paints() {
  // Same measure on the padding side: `pad_cells` undercounting means the
  // right-pinned size column of the `clean` report leaves the frame.
  for s in ["لا", "ｶﾞ", "作業", "ab"] {
    let out = pad_cells(s, 10);
    assert_eq!(painted(&out), 10, "{s:?} padded to {:?}", out);
  }
}

#[test]
fn pad_cells_fills_a_row_by_cells_so_a_pinned_column_stays_put() {
  // The other half of #554. A picker row and a reclaim row pad the
  // ellipsized value to the column width; `{:<w$}` counts chars, so once
  // the value is cell-measured a wide-glyph row got padded past its budget
  // and pushed the right-pinned size column off the frame.
  assert_eq!(painted(&pad_cells("ab", 5)), 5);
  let wide = pad_cells("作業", 8);
  assert_eq!(painted(&wide), 8, "4 cells of text, 4 of padding: {wide:?}");
  // Never trims: a value already at or over the column keeps its cells.
  assert_eq!(pad_cells("作業ディレクトリ", 4), "作業ディレクトリ");
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
  let line = worktrees_pane_title("", false, 5, 5, Color::Yellow, false);
  assert_eq!(title_text(&line), " [1] Worktrees (5) ");
}

#[test]
fn worktrees_pane_title_active_filter_shows_query_cursor_and_ratio() {
  // Issue #262: the live `/` filter renders in the pane title. While typing
  // (active), the title carries the `/query`, a block cursor, and the
  // `(visible/total)` ratio so the user sees how much the filter narrowed.
  let line = worktrees_pane_title("au", true, 3, 5, Color::Yellow, false);
  assert_eq!(title_text(&line), " [1] Worktrees /au\u{2588} (3/5) ");
}

#[test]
fn worktrees_pane_title_sticky_filter_shows_query_without_cursor() {
  // Sticky (committed) filter: the query stays visible in the title for
  // context, but with no cursor (the bar is closed) and a compact form — no
  // oversized hint.
  let line = worktrees_pane_title("au", false, 3, 5, Color::Yellow, false);
  assert_eq!(title_text(&line), " [1] Worktrees /au (3/5) ");
}

#[test]
fn worktrees_pane_title_active_empty_query_shows_prompt_and_total() {
  // Just opened the bar with an empty buffer: the `/` prompt + cursor show,
  // but the counter stays the `(total)` form (an empty query matches all).
  let line = worktrees_pane_title("", true, 5, 5, Color::Yellow, false);
  assert_eq!(title_text(&line), " [1] Worktrees /\u{2588} (5) ");
}

#[test]
fn worktrees_pane_title_paints_the_slash_in_the_filter_colour() {
  // The `/` prompt keeps its dedicated filter colour (historically the
  // `dirty` role) so it reads as an editable affordance, not chrome.
  let line = worktrees_pane_title("au", true, 3, 5, Color::Yellow, false);
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
  assert_eq!(status_pane_title(false), " [2] Status ");
}

#[test]
fn compact_titles_keep_the_bracket_shape_and_shout_the_label() {
  // Issue #545 + validation feedback on PR #546: compact only changes
  // the *case*, never the shape. The chord stays bracketed and keeps its
  // side — leading for a focusable pane (`[1]`, `[2]`), trailing for a
  // sub-pane — because that is how every other surface in the TUI writes
  // a key. Uppercase is what marks the line as chrome now that no rule
  // delimits it.
  let km = Keymap::defaults();
  assert_eq!(status_pane_title(true), " [2] STATUS ");
  assert_eq!(issue_pr_pane_title(&km, true), " ISSUE / PR [F] ");
  assert_eq!(working_tree_pane_title(&km, true), " WORKING TREE [R] ");
  assert_eq!(
    recent_items_pane_title(SidebarMode::Commits, &km, true),
    " RECENT COMMITS [L] "
  );
  assert_eq!(
    recent_items_pane_title(SidebarMode::Stashes, &km, true),
    " STASHES [L] "
  );
  // Same shape as the bordered form, case aside — the property the
  // feedback asked for, stated as one assertion rather than five.
  for (compact, bordered) in [
    (issue_pr_pane_title(&km, true), issue_pr_pane_title(&km, false)),
    (working_tree_pane_title(&km, true), working_tree_pane_title(&km, false)),
  ] {
    assert_eq!(
      compact.to_uppercase(),
      bordered.to_uppercase(),
      "compact must not reorder or re-punctuate the title"
    );
  }
}

#[test]
fn compact_header_line_fills_the_width_and_right_aligns_the_counter() {
  // Issue #545: the counter moves out of the bottom rule (which no longer
  // exists) onto the right of the header line, so a section spends one row
  // on chrome instead of two. The line is padded to the full width because
  // the fill has to reach the right edge — a header that stops at its text
  // reads as a stray highlighted word, not as a section boundary.
  let line = compact_header_line(
    ratatui::text::Line::from(" 1 WORKTREES "),
    Some(ratatui::text::Line::from(" 3 of 5 ")),
    30,
    Style::default(),
  );
  let text = title_text(&line);
  assert_eq!(text.chars().count(), 30, "header must span the pane width: {text:?}");
  assert!(text.starts_with(" 1 WORKTREES "), "title leads: {text:?}");
  assert!(text.ends_with(" 3 of 5 "), "counter is flushed right: {text:?}");
}

#[test]
fn compact_header_line_without_a_counter_still_spans_the_width() {
  let line = compact_header_line(ratatui::text::Line::from(" 2 STATUS "), None, 18, Style::default());
  let text = title_text(&line);
  assert_eq!(text.chars().count(), 18, "got {text:?}");
  assert!(text.starts_with(" 2 STATUS "), "got {text:?}");
}

#[test]
fn compact_header_line_drops_the_counter_before_the_title() {
  // A narrow pane cannot show both. The title carries the focus mnemonic
  // and says *what* the section is, so it is the half that survives; the
  // counter is the first thing cut, then the title itself is truncated.
  let line = compact_header_line(
    ratatui::text::Line::from(" 1 WORKTREES "),
    Some(ratatui::text::Line::from(" 3 of 5 ")),
    14,
    Style::default(),
  );
  let text = title_text(&line);
  assert_eq!(text.chars().count(), 14, "got {text:?}");
  assert!(
    !text.contains("of"),
    "counter dropped rather than overlapping: {text:?}"
  );

  let squeezed = compact_header_line(ratatui::text::Line::from(" 1 WORKTREES "), None, 6, Style::default());
  let text = title_text(&squeezed);
  assert_eq!(text.chars().count(), 6, "never overflows the pane: {text:?}");
}

/// A compact header carrying both span kinds: the pane name, which has no
/// colour of its own, and the filter `/` prompt, which does.
fn compact_header(focused: bool, theme: &Theme) -> ratatui::text::Line<'static> {
  let title = ratatui::text::Line::from(vec![
    ratatui::text::Span::raw(" 1 WORKTREES "),
    ratatui::text::Span::styled("/", Style::default().fg(Color::Yellow)),
  ]);
  compact_header_line(title, None, 30, compact_header_style(focused, theme))
}

/// The span of `line` that *is* `needle`, or failing that the one that
/// contains it. Exact first so a one-character needle keeps naming the span
/// it was written for even if a title later grows the same character.
fn header_span(line: &ratatui::text::Line<'static>, needle: &str) -> ratatui::text::Span<'static> {
  line
    .spans
    .iter()
    .find(|s| s.content.as_ref() == needle)
    .or_else(|| line.spans.iter().find(|s| s.content.contains(needle)))
    .unwrap_or_else(|| panic!("span {needle:?} in {:?}", title_text(line)))
    .clone()
}

#[test]
fn compact_header_trades_its_two_roles_on_focus_and_never_dims() {
  // #605: the header stops carrying focus as a *dimming*. The two states
  // trade the same pair of roles instead — `accent` text on the quiet
  // `section_bg` band when inactive, dark `section_bg` text on the
  // `accent` band when focused — so a pane's name never goes secondary.
  // `muted` is in neither: it is how you find the pane to `Tab` into.
  //
  // Over every palette, because one theme cannot discriminate the claim:
  // the default has `accent == focus`, so only its inactive header proves
  // anything, while `claude-dark` separates all three of `accent` /
  // `focus` / `muted` and pins both states.
  let mut themes = vec![("default", Theme::default())];
  for name in preset_names() {
    themes.push((name, Theme::preset(name).expect("listed preset must resolve")));
  }
  for (name, theme) in themes {
    let inactive = header_span(&compact_header(false, &theme), "WORKTREES").style;
    let focused = header_span(&compact_header(true, &theme), "WORKTREES").style;

    assert_eq!(
      inactive.fg,
      Some(theme.accent),
      "theme {name:?}: the inactive header is accent text over the section band"
    );
    assert_eq!(
      focused.fg,
      Some(theme.section_bg),
      "theme {name:?}: the focused header is dark text over the accent band"
    );
    for (state, style) in [("focused", focused), ("inactive", inactive)] {
      assert_ne!(
        style.fg,
        Some(theme.muted),
        "theme {name:?} / {state}: a pane's name is never the muted role"
      );
      assert_eq!(
        style.bg, None,
        "theme {name:?} / {state}: the band comes from `Chrome::fill`, not from the text style"
      );
    }
  }
}

#[test]
fn a_coloured_span_keeps_its_colour_on_either_band() {
  // The filter `/` prompt and the Working Tree per-category counts encode
  // a category, not focus, so neither band may repaint them — the header
  // style is *patched* onto a span rather than replacing it.
  let theme = Theme::preset("claude-dark").expect("preset must resolve");
  for (state, focused) in [("focused", true), ("inactive", false)] {
    let style = header_span(&compact_header(focused, &theme), "/").style;
    assert_eq!(
      style.fg,
      Some(Color::Yellow),
      "{state}: an already-coloured span keeps its own colour"
    );
  }
}

/// WCAG relative luminance, and the contrast ratio between two colours.
/// Only meaningful for `Rgb`, which is what every shipped preset uses for
/// the two roles the band is mixed from.
fn contrast(a: Color, b: Color) -> f64 {
  fn luminance(c: Color) -> f64 {
    let Color::Rgb(r, g, b) = c else {
      panic!("contrast is only defined on Rgb, got {c:?}");
    };
    let chan = |v: u8| {
      let v = v as f64 / 255.0;
      if v <= 0.03928 {
        v / 12.92
      } else {
        ((v + 0.055) / 1.055).powf(2.4)
      }
    };
    0.2126 * chan(r) + 0.7152 * chan(g) + 0.0722 * chan(b)
  }
  let (x, y) = (luminance(a), luminance(b));
  (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn the_focused_header_band_is_accent_pulled_down_toward_the_section_tone() {
  // The band is `accent` darkened toward `section_bg`, not `accent` itself
  // (too strong at full strength) and not `focus` (the border tone, which
  // is more saturated — the half of "too strong" that darkening alone does
  // not fix). Per preset, per channel: it sits between the two roles and
  // leans on `accent`, so it stays recognisably the header's colour.
  for name in preset_names() {
    let theme = Theme::preset(name).expect("listed preset must resolve");
    let (Color::Rgb(ar, ag, ab), Color::Rgb(gr, gg, gb)) = (theme.accent, theme.section_bg) else {
      panic!("preset {name:?} must carry RGB for both roles, or the mix silently falls back");
    };
    let band = compact_header_fill(&theme);
    let Color::Rgb(br, bg, bb) = band else {
      panic!("preset {name:?}: an RGB pair must mix to RGB");
    };

    assert_ne!(
      band, theme.accent,
      "preset {name:?}: the band is pulled down, not the accent role itself"
    );
    assert_ne!(
      band, theme.section_bg,
      "preset {name:?}: it has to differ from the inactive band, or it signals nothing"
    );
    assert_ne!(
      band, theme.selection_bg,
      "preset {name:?}: and it is never the cursor row's background"
    );
    for (chan, a, g, b) in [("r", ar, gr, br), ("g", ag, gg, bg), ("b", ab, gb, bb)] {
      let (lo, hi) = if a < g { (a, g) } else { (g, a) };
      assert!(
        (lo..=hi).contains(&b),
        "preset {name:?} / {chan}: the band must sit between the two roles, got {b} outside {lo}..={hi}"
      );
      assert!(
        b.abs_diff(a) <= b.abs_diff(g),
        "preset {name:?} / {chan}: it leans on accent, or it stops reading as the header's colour"
      );
    }
  }
}

#[test]
fn the_band_stays_legible_under_the_dark_text_written_on_it() {
  // This is the floor on how far the band may be darkened. The focused
  // header writes `section_bg` on it, so the two have to keep the 3:1 that
  // WCAG asks of bold display text — below it the text role would have to
  // change with the mix, and a passing colour test would not notice.
  // `claude-dark` is the tight one at 3.1:1, so this is not vacuous.
  for name in preset_names() {
    let theme = Theme::preset(name).expect("listed preset must resolve");
    let ratio = contrast(compact_header_fill(&theme), theme.section_bg);
    assert!(
      ratio >= 3.0,
      "preset {name:?}: band vs its own text is {ratio:.2}:1, under the 3:1 floor"
    );
  }
}

#[test]
fn a_theme_without_rgb_keeps_the_accent_role_as_its_band() {
  // The default theme's `accent` is an ANSI name whose value belongs to
  // the terminal and its `section_bg` a palette index: there are no
  // components to mix. Falling back to `accent` keeps a coloured band;
  // falling back to a grey would put that theme back where #605 started.
  let theme = Theme::default();
  assert_eq!(
    compact_header_fill(&theme),
    theme.accent,
    "with nothing to mix the band stays the accent role"
  );
}

#[test]
fn compact_header_weight_tracks_focus_across_the_whole_line() {
  // #605: weight is what the header line adds on top of the fill, and it
  // reaches *every* span, coloured ones included, so one header line runs
  // one rule rather than two side by side.
  let theme = Theme::default();
  let focused = compact_header(true, &theme);
  let unfocused = compact_header(false, &theme);

  for needle in ["WORKTREES", "/"] {
    assert!(
      header_span(&focused, needle)
        .style
        .add_modifier
        .contains(Modifier::BOLD),
      "focused: {needle:?} is bold"
    );
    assert!(
      !header_span(&unfocused, needle)
        .style
        .add_modifier
        .contains(Modifier::BOLD),
      "unfocused: {needle:?} is not bold"
    );
  }
}

#[test]
fn compact_titles_still_track_a_rebound_chord() {
  // The chord in a compact header is resolved live, exactly like the
  // bracketed one — a user who rebinds `F` must see the new key lead
  // the header rather than a stale literal.
  let mut km = Keymap::defaults();
  km.apply_override(Action::FetchGithub, vec![KeyStroke::parse_chord("Ctrl+g").unwrap()])
    .unwrap();
  assert_eq!(issue_pr_pane_title(&km, true), " ISSUE / PR [Ctrl+g] ");
}

#[test]
fn sidebar_subpane_titles_surface_live_bindings() {
  let mut km = Keymap::defaults();
  assert_eq!(issue_pr_pane_title(&km, false), " Issue / PR [F] ");
  assert_eq!(working_tree_pane_title(&km, false), " Working Tree [R] ");
  assert_eq!(
    recent_items_pane_title(SidebarMode::Commits, &km, false),
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

  assert_eq!(issue_pr_pane_title(&km, false), " Issue / PR [Ctrl+g] ");
  assert_eq!(working_tree_pane_title(&km, false), " Working Tree [Ctrl+r] ");
  assert_eq!(
    recent_items_pane_title(SidebarMode::Commits, &km, false),
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

#[test]
fn list_pane_counter_appends_the_mark_count() {
  // #484: only `d` reads the mark set, so the footer has to carry it or a
  // cursor-row verb would look like it ignored the selection.
  use gwm::tui::{delete_batch_title, list_pane_counter};
  assert_eq!(
    list_pane_counter(3, 12, 0).as_deref(),
    Some(" 3 of 12 "),
    "no mark, no change from the pre-#484 counter"
  );
  assert_eq!(list_pane_counter(3, 12, 2).as_deref(), Some(" 3 of 12 · 2 marked "));
  assert_eq!(list_pane_counter(0, 0, 2), None, "nothing visible, no footer");

  assert_eq!(delete_batch_title(1), "Delete Worktree");
  assert_eq!(delete_batch_title(3), "Delete 3 Worktrees");
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

  // A rebind of the config.edit cancel verb shows through (a modified
  // stroke — bare printables are refused as reserved typing since the
  // #456 review, iteration 13).
  let mut mk = ModalKeymap::defaults();
  mk.apply_override(ModalAction::ConfigEditCancel, vec![parse_single("Alt+q").unwrap()])
    .unwrap();
  let rebound = config_capture_footer_hints(&mk, false);
  assert!(
    rebound.iter().any(|(k, l)| k == "Alt+q" && l == "cancel"),
    "{rebound:?}"
  );
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

// ---- form_field_scroll (issue #553) ----------------------------------------

#[test]
fn form_field_scroll_stays_put_while_the_focused_row_fits() {
  // The whole point of deriving the offset from focus: a form that fits its
  // frame renders exactly as it did before #553, at offset 0. The last row a
  // `height`-row viewport shows is `height - 1`, so that one still scrolls
  // nothing.
  for focus in 0..8usize {
    assert_eq!(form_field_scroll(focus, 8), 0, "row {focus} fits an 8-row viewport");
  }
}

#[test]
fn form_field_scroll_pans_the_minimum_to_reveal_the_focused_row() {
  // One row past the viewport pans by exactly one: the focused field lands on
  // the last visible row, keeping as much of the form above it on screen as
  // the frame allows.
  assert_eq!(form_field_scroll(8, 8), 1);
  assert_eq!(form_field_scroll(9, 8), 2);
  // The rename form's own numbers: `Desc` sits on row 9 of 10, and a 120x16
  // terminal leaves the body 8 rows.
  assert_eq!(form_field_scroll(9, 8), 2, "rename at 120x16");
  // ...and 4 rows at 120x12.
  assert_eq!(form_field_scroll(9, 4), 6, "rename at 120x12");
}

#[test]
fn form_field_scroll_survives_a_zero_row_viewport() {
  // A frame so short the body layout resolves to nothing: `Constraint::Min(1)`
  // still yields 0 rows once the four fixed rows have taken the space. There
  // is nothing to reveal, and the arithmetic must not underflow.
  assert_eq!(form_field_scroll(0, 0), 1);
  assert_eq!(form_field_scroll(9, 0), 10);
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

#[test]
fn a_modal_never_shrinks_when_the_terminal_grows() {
  // #550. Both helpers used to branch on `term_width <= 80` to spend a bigger
  // percentage on a small terminal, which made width NON-MONOTONIC: dragging
  // a pane from 80 to 81 columns collapsed the link prompt by 16 columns and
  // the exec/clean/detail overlay by 22. A modal may stop growing; it must
  // never get narrower because the terminal got wider.
  use gwm::tui::{link_prompt_modal_width, overlay_modal_width, rich_view_modal_width};
  for w in 20u16..300 {
    for (name, f) in [
      ("link_prompt_modal_width", link_prompt_modal_width as fn(u16) -> u16),
      ("overlay_modal_width", overlay_modal_width as fn(u16) -> u16),
      ("rich_view_modal_width", rich_view_modal_width as fn(u16) -> u16),
    ] {
      let (here, next) = (f(w), f(w + 1));
      assert!(
        next >= here,
        "{name}: growing the terminal from {w} to {} cols shrank the modal from {here} to {next} cols",
        w + 1
      );
    }
  }
}

#[test]
fn the_width_policy_is_monotonic_and_bounded_for_any_knobs() {
  // Every one of the eight distinct knob sets in use, the two the wrappers
  // above cover included. The property belongs to the policy, not to its
  // callers: whatever (pct, min, max) a future overlay picks, its width must
  // never shrink as the terminal grows, never break its ceiling, and never
  // reach the frame edge.
  use gwm::tui::modal_width;
  for (pct, min_cols, max_cols) in [
    (40, 40, 64),  // confirm, nothing-selected fallback
    (60, 64, 72),  // open-menu / link prompt
    (60, 64, 96),  // help, config, command palette
    (62, 64, 88),  // confirm, destructive summary
    (62, 72, 88),  // exec picker, clean, detail
    (70, 56, 72),  // create, rename
    (80, 64, 96),  // bootstrap report
    (80, 72, 120), // rich PR / issue view (#551)
  ] {
    let mut previous = 0u16;
    for w in 20u16..=300 {
      let got = modal_width(w, pct, min_cols, max_cols);
      assert!(
        got >= previous,
        "({pct}%, [{min_cols}, {max_cols}]): {w} cols gave {got}, narrower than the {previous} before it"
      );
      assert!(
        got <= max_cols,
        "({pct}%, [{min_cols}, {max_cols}]): {w} cols broke the ceiling with {got}"
      );
      assert!(
        got <= w.saturating_sub(4),
        "({pct}%, [{min_cols}, {max_cols}]): {w} cols gave {got}, under 2 columns of margin per side"
      );
      previous = got;
    }
  }
}

#[test]
fn a_modal_always_leaves_a_margin_inside_the_frame() {
  // #550: the floor that kills the seam above must not let a modal grow into
  // the frame edge on a narrow terminal — the border would hug column 0.
  use gwm::tui::{link_prompt_modal_width, overlay_modal_width, rich_view_modal_width};
  for w in 20u16..=300 {
    for (name, f) in [
      ("link_prompt_modal_width", link_prompt_modal_width as fn(u16) -> u16),
      ("overlay_modal_width", overlay_modal_width as fn(u16) -> u16),
      ("rich_view_modal_width", rich_view_modal_width as fn(u16) -> u16),
    ] {
      let got = f(w);
      assert!(
        got <= w.saturating_sub(4),
        "{name}: at {w} cols the modal is {got} wide, leaving under 2 columns of margin per side"
      );
    }
  }
}

#[test]
fn compact_header_line_measures_in_terminal_cells_not_chars() {
  // Codex review, PR #546: the header was padded with `chars().count()`,
  // which counts one for a wide character that ratatui draws in two
  // cells. A filter query containing CJK or an emoji therefore produced a
  // line wider than the pane — the right-aligned counter fell off the
  // edge — because the padding was computed against an undercount.
  //
  // Asserted against what `set_stringn` paints, not against `Line::width()`
  // — that one sums `Span::width()`, the measure the helper itself uses, so
  // it would agree with a wrong implementation (issue #562).
  let title = ratatui::text::Line::from(" [1] WORKTREES /界 ");
  let line = compact_header_line(title, Some(ratatui::text::Line::from(" 3 of 5 ")), 40, Style::default());
  assert_eq!(
    painted_line(&line),
    40,
    "the header must span exactly the pane width in cells, got {}: {:?}",
    painted_line(&line),
    title_text(&line)
  );
}

#[test]
fn compact_header_line_truncates_wide_glyphs_by_cell_budget() {
  // Same measure on the narrow path: a title of wide glyphs alone must be
  // cut to the cell budget, never past it. Cutting by char count would
  // leave a line twice as wide as the pane.
  let title = ratatui::text::Line::from("界界界界界界界界");
  let line = compact_header_line(title, None, 9, Style::default());
  assert!(
    painted_line(&line) <= 9,
    "must never exceed the pane width in cells, got {}",
    painted_line(&line)
  );
}

/// Titles `unicode-width` reads narrower than the renderer paints them, with
/// the two measures spelled out. CJK is deliberately absent: `UnicodeWidthStr`
/// and `CellWidth` agree on it, which is why #546 shipped `Span::width()` and
/// why every fixture above stays green either way.
const UNDERCOUNTED: &[(&str, usize, usize)] = &[
  // Lam-alef is a ligature to `unicode-width`; the renderer walks graphemes
  // and paints both letters.
  ("لالالالالا", 5, 10),
  // U+FF9E carries `Grapheme_Extend`, so `unicode-width` gives it no cell,
  // but a terminal draws the halfwidth dakuten in one and ratatui adds it back.
  ("ｶﾞｶﾞｶﾞｶﾞｶﾞ", 5, 10),
];

#[test]
fn compact_header_line_pads_against_the_cells_the_renderer_paints() {
  for (title, narrow, wide) in UNDERCOUNTED {
    assert_eq!(
      (UnicodeWidthStr::width(*title), painted(title)),
      (*narrow, *wide),
      "{title:?} must be a case the two measures disagree on, or this proves nothing"
    );
    let counter = ratatui::text::Line::from(" 3 of 5 ");
    let line = compact_header_line(
      ratatui::text::Line::from(title.to_string()),
      Some(counter),
      20,
      Style::default(),
    );
    // Padding computed against the undercount leaves the line wider than the
    // pane, which pushes the right-aligned counter off it.
    assert_eq!(
      painted_line(&line),
      20,
      "{title:?}: header painted {} cells into a 20-cell pane: {:?}",
      painted_line(&line),
      title_text(&line)
    );
  }
}

#[test]
fn compact_header_line_truncates_by_the_cells_the_renderer_paints() {
  for (title, narrow, wide) in UNDERCOUNTED {
    // Narrower than what gets painted, wider than what `unicode-width` reads:
    // the truncation branch is only entered at all once the measure is right.
    let width = (narrow + wide) / 2;
    let line = compact_header_line(
      ratatui::text::Line::from(title.to_string()),
      None,
      width as u16,
      Style::default(),
    );
    assert!(
      painted_line(&line) <= width,
      "{title:?}: title painted {} cells into a {width}-cell pane: {:?}",
      painted_line(&line),
      title_text(&line)
    );
  }
}

#[test]
fn compact_header_line_truncates_sequences_whole_not_char_by_char() {
  // The truncation branch used to step `UnicodeWidthChar::width` per char. A
  // variation selector reads 0 there while the sequence it completes paints 2,
  // so every one of these was free and the whole title survived its budget.
  let title = "*\u{FE0F}*\u{FE0F}*\u{FE0F}*\u{FE0F}*\u{FE0F}";
  assert_eq!(painted(title), 10, "fixture must paint two cells per sequence");
  let line = compact_header_line(ratatui::text::Line::from(title), None, 5, Style::default());
  assert!(
    painted_line(&line) <= 5,
    "title painted {} cells into a 5-cell pane: {:?}",
    painted_line(&line),
    title_text(&line)
  );
}

// ---- the modal height policy (issue #569) ---------------------------------

#[test]
fn modal_height_sizes_to_content_between_its_bounds() {
  use gwm::tui::modal_height;

  // The counterpart of `modal_width`, and deliberately not its mirror: the
  // input is the content's own row count, not a percentage. A short overlay
  // is simply short, whereas a narrow one truncates its text, which is why
  // width interpolates a percentage and height does not.
  let term = 60;
  assert_eq!(
    modal_height(term, 14, 10, 30),
    14,
    "content between the bounds is taken as-is"
  );
  assert_eq!(modal_height(term, 3, 10, 30), 10, "a short tab still gets a usable box");
  assert_eq!(modal_height(term, 200, 10, 30), 30, "a long one stops at the ceiling");
}

#[test]
fn modal_height_never_reaches_the_frame_edge() {
  use gwm::tui::modal_height;

  // The one property that does carry over from `modal_width`: two rows of
  // margin per side, so a modal reads as a modal instead of repainting the
  // whole frame. `centered_abs` only clamps to the frame, which is why the
  // create form painted its border on rows 0 and 13 of a 14-row terminal.
  for term in 6..40u16 {
    let h = modal_height(term, 200, 10, 30);
    assert!(
      h <= term.saturating_sub(4).max(1),
      "term={term}: height {h} leaves no margin"
    );
    assert!(h >= 1, "term={term}: height must stay renderable");
  }
}

#[test]
fn modal_height_is_monotonic_in_its_content() {
  // No seam: one more row of content never yields a shorter box. The width
  // policy exists because its predecessors branched on `term_width <= 80`
  // and lost that property (#550); this one starts with it.
  let mut prev = 0;
  for rows in 0..60u16 {
    let h = gwm::tui::modal_height(80, rows, 10, 30);
    assert!(h >= prev, "content {rows} produced {h} after {prev}");
    prev = h;
  }
}

#[test]
fn the_rich_view_gets_a_wider_box_than_the_shared_overlay() {
  // Issue #551. The detail overlay's 88-column ceiling was chosen for the
  // clean report, whose rows are an icon, a directory name and a size
  // pinned right — a wider box only stretches the gap between the two
  // columns. The rich PR / issue view puts PROSE in the same box, and
  // prose is the one payload that keeps earning columns: on a 200-column
  // terminal the shared policy left more than half the screen unused
  // while the description was cut at `… 85 more lines`.
  use gwm::tui::{overlay_modal_width, rich_view_modal_width};
  assert!(
    rich_view_modal_width(200) > overlay_modal_width(200),
    "the rich view must claim more of a wide terminal than the shared overlay"
  );
  // Still a modal, not a takeover: capped well short of the frame.
  assert!(rich_view_modal_width(400) <= 120);
  // A narrow terminal keeps the shared floor rather than gaining one of
  // its own — the two policies must not cross over.
  assert!(rich_view_modal_width(60) >= overlay_modal_width(60));
}

#[test]
fn every_markdown_role_is_painted_differently_from_plain_text() {
  // The other half of `tests/tui_markdown_tests.rs` (issue #551). That file
  // asserts the parse produces the right roles; this one asserts the roles
  // reach the screen as something the eye can tell apart. A parse that
  // produces perfect segments nobody colours differently is a feature that
  // is dead on screen with the suite green.
  //
  // Written as an exhaustive `match` with no `_` arm so a role added later
  // does not compile until someone decides how it is painted.
  use gwm::tui::markdown_style;
  use gwm::tui::state::markdown::Emphasis;
  let theme = gwm::tui::theme::Theme::default();
  let plain = markdown_style(Emphasis::Plain, &theme);

  for role in [
    Emphasis::Plain,
    Emphasis::Bold,
    Emphasis::Italic,
    Emphasis::BoldItalic,
    Emphasis::Code,
    Emphasis::Strike,
    Emphasis::Link,
    Emphasis::Heading,
    Emphasis::Quote,
    Emphasis::Marker,
    Emphasis::Success,
    Emphasis::Failure,
    Emphasis::Running,
    Emphasis::Notice,
    Emphasis::Muted,
    Emphasis::Branch,
  ] {
    let style = markdown_style(role, &theme);
    match role {
      // Plain text is the baseline it is measured against.
      Emphasis::Plain => assert_eq!(style, plain),
      Emphasis::Bold
      | Emphasis::Italic
      | Emphasis::BoldItalic
      | Emphasis::Code
      | Emphasis::Strike
      | Emphasis::Link
      | Emphasis::Heading
      | Emphasis::Quote
      | Emphasis::Marker
      | Emphasis::Success
      | Emphasis::Failure
      | Emphasis::Running
      | Emphasis::Notice
      | Emphasis::Muted
      | Emphasis::Branch => assert_ne!(style, plain, "{role:?} must not be painted exactly like plain prose"),
    }
  }
}

#[test]
fn the_metadata_roles_resolve_to_the_status_panes_own_colours() {
  // The rich view's metadata block claims to colour a fact the way the
  // Status pane colours it (issue #551). That claim is only true if both
  // sides land on the SAME theme role — asserted against `pr_badge_color`
  // and `issue_badge_color` themselves rather than against a colour spelled
  // out twice, which would agree with a wrong implementation.
  use gwm::github::{IssueState, PrState};
  use gwm::tui::state::markdown::Emphasis;
  use gwm::tui::{issue_badge_color, markdown_style, pr_badge_color};
  let theme = gwm::tui::theme::Theme::default();
  let fg = |e: Emphasis| markdown_style(e, &theme).fg.expect("a role paints a foreground");

  for (state, role) in [
    (PrState::Open, Emphasis::Success),
    (PrState::Draft, Emphasis::Muted),
    (PrState::Merged, Emphasis::Notice),
    (PrState::Closed, Emphasis::Failure),
  ] {
    assert_eq!(
      fg(role),
      pr_badge_color(state, &theme),
      "{state:?} reads as one colour in the pane and another in the overlay"
    );
  }
  for (state, role) in [
    (IssueState::Open, Emphasis::Success),
    (IssueState::Closed, Emphasis::Notice),
  ] {
    assert_eq!(fg(role), issue_badge_color(state, &theme), "{state:?}");
  }
}
