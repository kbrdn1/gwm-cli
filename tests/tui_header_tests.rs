//! Contract tests for the styled TUI header builder (`header_line`).
//!
//! Issue #185: the top line must stop being one flat cyan string and instead
//! present a clear visual hierarchy — a reverse-video version chip (the same
//! chip language as the #180 footer), the repo name in bold, and the working
//! directory dimmed as secondary context. An optional `picker` chip flags the
//! `gwm switch` picker session.
//!
//! `header_line` is a pure, width-driven builder (like `footer_line`) so the
//! layout contract is pinned here without spinning up a ratatui backend.

use gwm::tui::theme::Theme;
use gwm::tui::{header_line, COMMAND_LOGS_ICON, SETTINGS_ICON};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};

/// Flatten a `Line` back to its rendered plain text by concatenating every
/// span's content — what the user sees on the row, minus styling.
fn plain(line: &Line<'_>) -> String {
  line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn display_width(line: &Line<'_>) -> usize {
  plain(line).chars().count()
}

/// First span whose content contains `needle`, for asserting on its style.
fn span_with<'a, 'b>(line: &'a Line<'b>, needle: &str) -> Option<&'a Span<'b>> {
  line.spans.iter().find(|s| s.content.contains(needle))
}

fn version_token() -> String {
  env!("CARGO_PKG_VERSION").to_string()
}

#[test]
fn header_surfaces_version_repo_and_path_when_wide() {
  let line = header_line("gwm-cli", "/Users/me/Projects/gwm-cli", false, 120, &Theme::default()).line;
  let text = plain(&line);
  assert!(text.contains(&version_token()), "missing version: {}", text);
  assert!(text.contains("gwm-cli"), "missing repo name: {}", text);
  assert!(text.contains("/Users/me/Projects/gwm-cli"), "missing workdir: {}", text);
}

#[test]
fn header_fits_on_a_single_line_within_the_given_width() {
  let line = header_line("gwm-cli", "/Users/me/Projects/gwm-cli", false, 120, &Theme::default()).line;
  assert!(
    display_width(&line) <= 120,
    "header width {} exceeded 120: {:?}",
    display_width(&line),
    plain(&line)
  );
  assert!(!plain(&line).contains('\n'), "header must be a single line");
}

#[test]
fn version_renders_as_a_reverse_video_chip_on_the_accent_colour() {
  let line = header_line(
    "gwm-cli",
    "/tmp/x",
    false,
    120,
    &Theme {
      accent: Color::Magenta,
      ..Theme::default()
    },
  )
  .line;
  let chip = span_with(&line, "gwm ").expect("version chip span present");
  assert_eq!(chip.style.fg, Some(Color::Magenta), "chip not painted on accent");
  assert!(
    chip.style.add_modifier.contains(Modifier::REVERSED),
    "version chip must be reverse-video like the footer chips"
  );
  assert!(
    chip.style.add_modifier.contains(Modifier::BOLD),
    "version chip must be bold"
  );
}

#[test]
fn current_dir_name_is_a_leading_badge_and_path_is_dimmed() {
  let line = header_line("gwm-cli", "/Users/me/Projects/gwm-cli", false, 120, &Theme::default()).line;
  let repo = span_with(&line, "gwm-cli").expect("repo span present");
  assert!(
    repo.style.add_modifier.contains(Modifier::REVERSED),
    "current dir name must render as a leading badge"
  );
  assert!(
    repo.style.add_modifier.contains(Modifier::BOLD),
    "current dir name badge must be bold"
  );
  let path = span_with(&line, "/Users/me/Projects/gwm-cli").expect("path span present");
  assert_eq!(
    path.style.fg,
    Some(Color::DarkGray),
    "path must be dimmed as secondary context"
  );
}

#[test]
fn version_chip_is_pinned_to_the_end_when_wide() {
  let line = header_line("gwm-cli", "/Users/me/Projects/gwm-cli", false, 120, &Theme::default()).line;
  let text = plain(&line);
  assert!(
    text.trim_end().ends_with(&format!("gwm {}", version_token())),
    "version chip must end the header row: {text:?}"
  );
}

#[test]
fn picker_chip_present_only_in_picker_mode() {
  let off = header_line("gwm-cli", "/tmp/x", false, 120, &Theme::default()).line;
  assert!(
    !plain(&off).to_lowercase().contains("picker"),
    "picker chip leaked outside picker mode: {}",
    plain(&off)
  );
  let on = header_line("gwm-cli", "/tmp/x", true, 120, &Theme::default()).line;
  let chip = span_with(&on, "picker").expect("picker chip present in picker mode");
  assert!(
    chip.style.add_modifier.contains(Modifier::REVERSED),
    "picker chip must be a reverse-video chip"
  );
}

#[test]
fn narrow_width_drops_path_but_keeps_version_chip_and_repo() {
  // Wide enough for the version chip + repo name, but not the long path.
  let width = format!(" gwm {} ", version_token()).chars().count() + 2 + " gwm-cli ".len() + 4;
  let line = header_line(
    "gwm-cli",
    "/Users/me/some/really/long/path/that/will/not/fit/gwm-cli",
    false,
    width,
    &Theme::default(),
  )
  .line;
  let text = plain(&line);
  assert!(
    display_width(&line) <= width,
    "header overflowed at width {}: {:?}",
    width,
    text
  );
  assert!(
    text.contains(&version_token()),
    "version chip dropped too early: {}",
    text
  );
  assert!(text.contains("gwm-cli"), "repo name dropped too early: {}", text);
  assert!(
    !text.contains("really/long/path"),
    "path should be dropped/truncated under width pressure: {}",
    text
  );
}

#[test]
fn zero_width_emits_an_empty_line_without_overflowing() {
  let line = header_line("gwm-cli", "/tmp/x", false, 0, &Theme::default()).line;
  assert_eq!(display_width(&line), 0, "zero width must produce nothing");
  assert!(!plain(&line).contains('\n'));
}

#[test]
fn control_chars_never_break_the_single_line_contract() {
  // A pathological workdir with embedded newline/tab must not split the row.
  let line = header_line("gwm-cli", "/tmp/a\nb\tc", false, 120, &Theme::default()).line;
  assert!(!plain(&line).contains('\n'), "newline leaked into header row");
  assert!(!plain(&line).contains('\t'), "tab leaked into header row");
}

// --- the row never paints past its width (issue #563) ----------------------

/// Cells the renderer paints for `s`. The oracle here, because the builder
/// budgets with `chars().count()` and asserting on that would agree with it
/// whatever it did.
fn painted(s: &str) -> usize {
  let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 400, 1));
  let (x, _) = buf.set_stringn(0, 0, s, 400, ratatui::style::Style::default());
  usize::from(x)
}

fn painted_line(line: &Line<'_>) -> usize {
  line.spans.iter().map(|s| painted(&s.content)).sum()
}

#[test]
fn the_header_never_paints_past_its_width_on_wide_glyphs() {
  // A repo directory named in CJK is 1 character and 2 columns per glyph, and
  // the row budgets in characters: at 80 columns the header painted 102, so
  // the version chip it pins right went off the terminal. The discriminant is
  // width per character, not the `unicode-width` divergence #562 chased, so
  // Arabic is not a fixture here: it reads one column per letter both ways.
  let theme = Theme::default();
  for (repo, path) in [
    ("作業作業作業作業作業", "~/dev/作業作業作業作業作業作業"),
    ("plain", "~/dev/ｶﾞｶﾞｶﾞｶﾞｶﾞｶﾞｶﾞｶﾞｶﾞｶﾞ"),
    ("🚀🚀🚀🚀🚀", "~/dev/🚀🚀🚀🚀🚀🚀🚀🚀"),
  ] {
    for w in [80usize, 100, 120] {
      let line = header_line(repo, path, false, w, &theme).line;
      assert!(
        painted_line(&line) <= w,
        "{repo:?} at {w} columns: header painted {} cells: {:?}",
        painted_line(&line),
        plain(&line)
      );
    }
  }
}

/// See `tests/tui_footer_tests.rs` for the same set and the same reasoning:
/// `Cf`, not `Cc`, so `char::is_control` matches none of them (#502).
const BIDI_CONTROLS: &[char] = &[
  '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}',
  '\u{2067}', '\u{2068}', '\u{2069}',
];

#[test]
fn a_bidi_control_in_the_repo_or_path_never_reaches_the_row() {
  // The header neutralised `char::is_control` only, so a directory named with
  // a format character reordered how the row reads. Unlike a ref name this
  // needs no fetch: a directory on disk can carry one, and the header shows
  // both its name and the working path.
  //
  // Pre-dates this branch, `dev` leaks the same characters. Found by a Codex
  // review of #563.
  for c in BIDI_CONTROLS {
    for (repo, path) in [
      (format!("re{c}po"), "~/dev/x".to_string()),
      ("repo".into(), format!("~/dev/x{c}y")),
    ] {
      let line = header_line(&repo, &path, false, 120, &Theme::default()).line;
      assert!(
        !plain(&line).contains(*c),
        "the header replayed U+{:04X} from {:?}",
        *c as u32,
        (&repo, &path)
      );
    }
  }
}

// ---- Panel affordances (issue #624) ---------------------------------------

/// Column the first occurrence of `needle` is painted at, measured the way
/// the terminal measures: by walking the spans and asking ratatui how wide
/// each one paints. Not `chars()` — `header_line`'s own arithmetic counts
/// characters (#563), and neither affordance glyph is ASCII.
fn painted_col_of(line: &Line<'_>, needle: &str) -> Option<usize> {
  let mut col = 0usize;
  for s in &line.spans {
    if let Some(i) = s.content.find(needle) {
      return Some(col + painted(&s.content[..i]));
    }
    col += painted(&s.content);
  }
  None
}

#[test]
fn the_header_carries_both_panel_affordances_left_of_the_version_chip() {
  let h = header_line("gwm-cli", "/Users/me/Projects/gwm-cli", false, 120, &Theme::default());
  let text = plain(&h.line);

  let logs = text.find(COMMAND_LOGS_ICON).expect("no command-logs affordance");
  let settings = text.find(SETTINGS_ICON).expect("no settings affordance");
  let version = text.find(&version_token()).expect("no version chip");

  assert!(logs < settings, "the transcript panel comes first: {text:?}");
  assert!(
    settings < version,
    "both affordances sit left of the pinned version chip: {text:?}"
  );
}

/// The discriminating one. Asserting "clicking the reported range opens
/// Settings" is self-fulfilling when the range comes from the code under
/// test, so this walks the row and measures where the glyph *actually*
/// lands, then checks the reported range against that measurement.
///
/// The range must also cover the glyph's trailing pad cell: both glyphs are
/// East-Asian-Ambiguous, so a terminal may paint either of them two cells
/// wide, and a range that only covered the first cell would miss half the
/// clicks. Two cells reserved is the repo convention for a non-ASCII glyph
/// (`NOTE_ICON`, #595; the Settings tab strip).
#[test]
fn the_reported_affordance_columns_are_where_the_glyphs_are_painted() {
  let theme = Theme::default();
  for w in [80usize, 100, 120, 200] {
    let h = header_line("gwm-cli", "/Users/me/dev/gwm-cli", false, w, &theme);
    let logs = h.logs.clone().unwrap_or_else(|| panic!("no logs range at {w} columns"));
    let settings = h
      .settings
      .clone()
      .unwrap_or_else(|| panic!("no settings range at {w} columns"));

    let logs_col = painted_col_of(&h.line, COMMAND_LOGS_ICON).expect("glyph missing");
    let settings_col = painted_col_of(&h.line, SETTINGS_ICON).expect("glyph missing");

    assert_eq!(logs.start as usize, logs_col, "logs range starts off the glyph at {w}");
    assert_eq!(
      settings.start as usize, settings_col,
      "settings range starts off the glyph at {w}"
    );
    assert_eq!(logs.len(), 2, "the range covers the glyph and its pad cell at {w}");
    assert_eq!(settings.len(), 2, "the range covers the glyph and its pad cell at {w}");
    assert!(
      settings.end as usize <= w,
      "the settings range ran off a {w}-column row"
    );
  }
}

/// The floor case. Without it the suite above passes vacuously on a narrow
/// terminal by never getting there: what has to hold is that the row still
/// carries the version chip when the affordances no longer fit, and that the
/// ranges say so rather than pointing at columns nothing was painted on.
#[test]
fn a_row_too_narrow_for_the_affordances_drops_them_and_keeps_the_version_chip() {
  let theme = Theme::default();
  let version = version_token();
  // Wide enough for ` gwm <version> ` and the repo badge, not for six more
  // cells of affordance.
  let w = version.chars().count() + 6 + 4;
  let h = header_line("gwm-cli", "/Users/me/dev/gwm-cli", false, w, &theme);

  assert!(
    h.logs.is_none(),
    "affordance kept on a {w}-column row: {:?}",
    plain(&h.line)
  );
  assert!(h.settings.is_none());
  assert!(
    !plain(&h.line).contains(COMMAND_LOGS_ICON),
    "range dropped but the glyph was still painted"
  );
  assert!(
    plain(&h.line).contains(&version),
    "the version chip stays pinned: {:?}",
    plain(&h.line)
  );
}

/// Sacrifice order. The path is secondary context and goes first; the
/// affordances are the only on-screen sign the two panels exist, so they
/// outlive it.
#[test]
fn the_path_is_sacrificed_before_the_affordances() {
  let theme = Theme::default();
  let long = "/Users/me/Projects/some/deeply/nested/place/gwm-cli";
  let h = header_line("gwm-cli", long, false, 60, &theme);
  let text = plain(&h.line);

  assert!(!text.contains(long), "the path should have been truncated: {text:?}");
  assert!(
    h.logs.is_some() && h.settings.is_some(),
    "the affordances outlive the path: {text:?}"
  );
}
