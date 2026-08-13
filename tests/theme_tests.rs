//! Tests for the configurable TUI theme (issue #33).
//!
//! Covers the `Theme` struct + role-based color resolution: defaults
//! match the pre-#33 hardcoded scheme, built-in presets resolve by
//! name, per-role overrides win over preset values, every supported
//! color syntax (named / indexed / hex) parses cleanly.

use gwm::tui::theme::{parse_color, preset_names, Theme};
use ratatui::style::Color;

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

#[test]
fn default_theme_matches_pre_issue_33_scheme() {
  // The hardcoded palette pre-#33 was: cyan for focus/accent, green
  // for branch/clean, yellow for dirty/main, magenta for locked,
  // red for prunable, dark gray for muted. The default Theme must
  // be observationally equivalent so users who never write a
  // `[theme]` block see no change — a different look (e.g. the
  // Claude orange) is opt-in via the `claude-dark` preset, not the
  // default.
  let t = Theme::default();
  assert_eq!(t.focus, Color::Cyan);
  assert_eq!(t.accent, Color::Cyan);
  assert_eq!(t.branch, Color::Green);
  assert_eq!(t.clean, Color::Green);
  assert_eq!(t.dirty, Color::Yellow);
  assert_eq!(t.main, Color::Yellow);
  assert_eq!(t.locked, Color::Magenta);
  assert_eq!(t.prunable, Color::Red);
  assert_eq!(t.muted, Color::DarkGray);
  // #210: the chrome roles default to their pre-#170 structural literals
  // so a `[theme]`-less config still paints the worktree name white and
  // the table path grey.
  assert_eq!(t.name, Color::White, "name role default → White");
  assert_eq!(t.path, Color::Gray, "path role default → Gray");
  // #211: the git-status families default to the cyan/yellow/green they
  // used to borrow from accent/dirty/clean, so the working-tree panel is
  // unchanged for a `[theme]`-less config.
  assert_eq!(t.staged, Color::Cyan, "staged role default → Cyan");
  assert_eq!(t.modified, Color::Yellow, "modified role default → Yellow");
  assert_eq!(t.untracked, Color::Green, "untracked role default → Green");
  // #545: the compact-mode section header fill. Indexed rather than `Rgb`
  // so it survives a terminal without truecolor — a translucent white
  // does not exist in ANSI 256, which is the fill the mock-up asked for.
  assert_eq!(
    t.section_bg,
    Color::Indexed(236),
    "section_bg role default → indexed 236"
  );
}

#[test]
fn section_bg_never_collides_with_selection_bg() {
  // #545 acceptance: row selection already paints a background. Once
  // section headers gain one too, the two must stay tellable apart —
  // otherwise a header reads as a selected row. Checked on the default
  // theme *and* every shipped preset, because the pair is chosen
  // per-palette and a copy-paste slip is exactly how they'd converge.
  let mut themes = vec![("default", Theme::default())];
  for name in preset_names() {
    themes.push((name, Theme::preset(name).expect("listed preset must resolve")));
  }
  for (name, t) in themes {
    assert_ne!(
      t.section_bg, t.selection_bg,
      "theme {:?}: section header fill must differ from the selection background",
      name
    );
  }
}

#[test]
fn apply_override_replaces_git_status_roles() {
  // #211: the git-status families must be overridable independently of the
  // accent/dirty/clean roles they used to borrow.
  let mut t = Theme::default();
  t.apply_override("staged", "magenta").unwrap();
  t.apply_override("modified", "#123456").unwrap();
  t.apply_override("untracked", "200").unwrap();
  assert_eq!(t.staged, Color::Magenta, "staged override wins");
  assert_eq!(t.modified, Color::Rgb(0x12, 0x34, 0x56), "modified override wins");
  assert_eq!(t.untracked, Color::Indexed(200), "untracked override wins");
  // The roles they used to borrow are untouched — proving the decoupling.
  assert_eq!(t.accent, Color::Cyan, "accent untouched by staged override");
  assert_eq!(t.dirty, Color::Yellow, "dirty untouched by modified override");
  assert_eq!(t.clean, Color::Green, "clean untouched by untracked override");
}

#[test]
fn apply_override_replaces_section_bg_role() {
  // #545: users on an unusual terminal background need to retune the
  // compact header fill without forking a preset.
  let mut t = Theme::default();
  t.apply_override("section_bg", "#1f2335").unwrap();
  assert_eq!(t.section_bg, Color::Rgb(0x1f, 0x23, 0x35), "section_bg override wins");
  assert_eq!(
    t.selection_bg,
    Color::DarkGray,
    "selection_bg untouched by section_bg override"
  );
}

#[test]
fn apply_override_replaces_name_and_path_roles() {
  // #210: the new chrome roles must be overridable like any other role.
  let mut t = Theme::default();
  t.apply_override("name", "#abcdef").unwrap();
  t.apply_override("path", "240").unwrap();
  assert_eq!(t.name, Color::Rgb(0xab, 0xcd, 0xef), "name override wins");
  assert_eq!(t.path, Color::Indexed(240), "path override wins");
  // Other roles untouched.
  assert_eq!(t.focus, Color::Cyan);
  assert_eq!(t.muted, Color::DarkGray);
}

#[test]
fn claude_dark_preset_ports_the_palette() {
  // #185: a full port of the Claude dark palette
  // (`~/.claude/.../claude-dark-palette.md`) into gwm's role scheme.
  // Pin the signature mappings so a regression in the port is caught.
  let t = Theme::preset("claude-dark").expect("claude-dark preset must resolve");
  assert_eq!(t.accent, Color::Rgb(0xD4, 0x82, 0x5D), "primary orange");
  assert_eq!(t.focus, Color::Rgb(0xC1, 0x5F, 0x3C), "orange-dark / focused borders");
  assert_eq!(t.branch, Color::Rgb(0x86, 0xE8, 0x9A), "success green");
  assert_eq!(t.clean, Color::Rgb(0x86, 0xE8, 0x9A), "success green");
  assert_eq!(t.dirty, Color::Rgb(0xFF, 0xDF, 0x61), "warning yellow");
  assert_eq!(t.main, Color::Rgb(0xFF, 0xDF, 0x61), "warning yellow");
  assert_eq!(t.locked, Color::Rgb(0xC7, 0x9B, 0xFF), "special purple");
  assert_eq!(t.prunable, Color::Rgb(0xFF, 0x7A, 0x7A), "error red");
  // #214: the chrome roles map to the palette's `--text` / `--text-dim`
  // tokens, not the improvised warm greys #210 introduced (corrected before
  // any release — both #210 and #214 land in the same unreleased cycle).
  assert_eq!(t.name, Color::Rgb(0xE0, 0xE0, 0xE0), "--text (primary text)");
  assert_eq!(t.path, Color::Rgb(0xB0, 0xB0, 0xB0), "--text-dim (secondary text)");
  // The alias `claude` resolves to the same theme.
  assert_eq!(Theme::preset("claude"), Some(t), "`claude` aliases `claude-dark`");
}

#[test]
fn claude_dark_is_listed_among_presets() {
  assert!(
    preset_names().contains(&"claude-dark"),
    "claude-dark must be discoverable via `gwm theme list`"
  );
}

// ---------------------------------------------------------------------------
// Built-in presets
// ---------------------------------------------------------------------------

#[test]
fn preset_names_lists_at_least_one_builtin() {
  // Shipping at least one preset is the whole point of the framework
  // — without one, `gwm theme show <name>` has no payload and the
  // user has to copy hex codes from somewhere. We don't pin the
  // exact list (presets can be added over time); we pin that the
  // list is non-empty and that every name resolves.
  let names = preset_names();
  assert!(!names.is_empty(), "at least one built-in preset must be shipped");
  for name in names {
    assert!(
      Theme::preset(name).is_some(),
      "preset {:?} listed in preset_names() must resolve",
      name
    );
  }
}

#[test]
fn unknown_preset_returns_none() {
  assert!(Theme::preset("does-not-exist").is_none());
}

#[test]
fn preset_produces_a_theme_different_from_default() {
  // A preset that exactly matched the default would be a useless
  // listing. Pin that at least one role differs so the framework
  // actually delivers contrast — without dictating which preset is
  // checked, since the catalog can grow.
  let names = preset_names();
  let any_differs = names.iter().any(|&name| {
    let preset = Theme::preset(name).unwrap();
    preset != Theme::default()
  });
  assert!(
    any_differs,
    "at least one preset must differ from the default (otherwise the framework ships no actual themes)"
  );
}

// ---------------------------------------------------------------------------
// Overrides
// ---------------------------------------------------------------------------

#[test]
fn apply_override_replaces_a_single_role() {
  let mut t = Theme::default();
  t.apply_override("focus", "red").unwrap();
  assert_eq!(t.focus, Color::Red, "override must win for the targeted role");
  // Other roles untouched.
  assert_eq!(t.accent, Color::Cyan);
  assert_eq!(t.branch, Color::Green);
}

#[test]
fn apply_override_rejects_unknown_role() {
  let mut t = Theme::default();
  let err = t.apply_override("phantom", "red").unwrap_err();
  assert!(
    err.to_string().to_lowercase().contains("phantom"),
    "expected message to name the bad role, got: {err}"
  );
}

#[test]
fn apply_override_rejects_unparsable_color() {
  let mut t = Theme::default();
  let err = t.apply_override("focus", "not_a_color").unwrap_err();
  assert!(
    err.to_string().to_lowercase().contains("not_a_color"),
    "expected message to name the bad color, got: {err}"
  );
}

// ---------------------------------------------------------------------------
// Color parsing
// ---------------------------------------------------------------------------

#[test]
fn parse_named_color() {
  assert_eq!(parse_color("cyan").unwrap(), Color::Cyan);
  assert_eq!(parse_color("Cyan").unwrap(), Color::Cyan, "case-insensitive");
  assert_eq!(parse_color("red").unwrap(), Color::Red);
  assert_eq!(parse_color("dark_gray").unwrap(), Color::DarkGray);
}

#[test]
fn parse_indexed_color() {
  // 256-color palette: numeric strings parse as `Color::Indexed`.
  let c = parse_color("220").unwrap();
  assert_eq!(c, Color::Indexed(220));
}

#[test]
fn parse_hex_color() {
  let c = parse_color("#89b4fa").unwrap();
  assert_eq!(c, Color::Rgb(0x89, 0xb4, 0xfa));
}

#[test]
fn parse_rejects_garbage() {
  assert!(parse_color("").is_err());
  assert!(parse_color("not_a_color").is_err());
  assert!(parse_color("#zzz").is_err());
  assert!(parse_color("256").is_err()); // indices 0-255 only
}
