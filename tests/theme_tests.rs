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
  // `[theme]` block see no change.
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
