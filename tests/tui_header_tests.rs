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

use gwm::tui::header_line;
use gwm::tui::theme::Theme;
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
  let line = header_line("gwm-cli", "/Users/me/Projects/gwm-cli", false, 120, &Theme::default());
  let text = plain(&line);
  assert!(text.contains(&version_token()), "missing version: {}", text);
  assert!(text.contains("gwm-cli"), "missing repo name: {}", text);
  assert!(text.contains("/Users/me/Projects/gwm-cli"), "missing workdir: {}", text);
}

#[test]
fn header_fits_on_a_single_line_within_the_given_width() {
  let line = header_line("gwm-cli", "/Users/me/Projects/gwm-cli", false, 120, &Theme::default());
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
  );
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
  let line = header_line("gwm-cli", "/Users/me/Projects/gwm-cli", false, 120, &Theme::default());
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
  let line = header_line("gwm-cli", "/Users/me/Projects/gwm-cli", false, 120, &Theme::default());
  let text = plain(&line);
  assert!(
    text.trim_end().ends_with(&format!("gwm {}", version_token())),
    "version chip must end the header row: {text:?}"
  );
}

#[test]
fn picker_chip_present_only_in_picker_mode() {
  let off = header_line("gwm-cli", "/tmp/x", false, 120, &Theme::default());
  assert!(
    !plain(&off).to_lowercase().contains("picker"),
    "picker chip leaked outside picker mode: {}",
    plain(&off)
  );
  let on = header_line("gwm-cli", "/tmp/x", true, 120, &Theme::default());
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
  );
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
  let line = header_line("gwm-cli", "/tmp/x", false, 0, &Theme::default());
  assert_eq!(display_width(&line), 0, "zero width must produce nothing");
  assert!(!plain(&line).contains('\n'));
}

#[test]
fn control_chars_never_break_the_single_line_contract() {
  // A pathological workdir with embedded newline/tab must not split the row.
  let line = header_line("gwm-cli", "/tmp/a\nb\tc", false, 120, &Theme::default());
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
      let line = header_line(repo, path, false, w, &theme);
      assert!(
        painted_line(&line) <= w,
        "{repo:?} at {w} columns: header painted {} cells: {:?}",
        painted_line(&line),
        plain(&line)
      );
    }
  }
}
