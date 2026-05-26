//! End-to-end test for the `[theme]` config → `App.theme` pipeline
//! (issue #33). The unit-level invariants on `Theme` live in
//! `tests/theme_tests.rs`; this file pins that the pipeline reaches
//! the running TUI.

mod common;

use common::init_repo;
use gwm::tui::theme::Theme;
use gwm::tui::App;
use ratatui::style::Color;

#[test]
fn app_theme_defaults_when_no_config() {
  // Fresh repo with no `.gwm.toml` → `App.theme == Theme::default()`.
  // The pre-#33 hardcoded palette is preserved verbatim.
  let (dir, _) = init_repo();
  let app = App::new_at(Some(dir.path())).unwrap();
  assert_eq!(app.theme, Theme::default());
  assert_eq!(app.theme.focus, Color::Cyan);
}

#[test]
fn app_theme_reflects_preset_from_gwm_toml() {
  // Writing `[theme] preset = "catppuccin"` in `.gwm.toml` must
  // make `App.theme` differ from the default. We assert the
  // difference rather than a specific hex so the test stays
  // resilient to preset palette tweaks.
  let (dir, _) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[theme]
preset = "catppuccin"
"#,
  )
  .unwrap();
  let app = App::new_at(Some(dir.path())).unwrap();
  assert_ne!(app.theme, Theme::default(), "preset must change at least one role");
}

#[test]
fn app_theme_reflects_per_role_override_from_gwm_toml() {
  let (dir, _) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[theme]
focus = "red"
"#,
  )
  .unwrap();
  let app = App::new_at(Some(dir.path())).unwrap();
  assert_eq!(app.theme.focus, Color::Red);
  // Other roles untouched.
  assert_eq!(app.theme.branch, Theme::default().branch);
}
