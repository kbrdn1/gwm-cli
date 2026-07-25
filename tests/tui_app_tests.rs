mod common;

use common::init_repo;
use gwm::naming::BRANCH_TYPES;
use gwm::tui::theme::Theme;
use gwm::tui::{
  branch_name_color, filled_cells_for_progress, freshness_color, panel_border_color, pr_badge_color, App,
  ConfirmKeyAction, CountdownTickOutcome, Field, View,
};
use gwm::worktree::{BranchStatus, WorktreeInfo};
use ratatui::style::Color;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Process-global lock guarding every test in this binary that mutates
/// `std::env`. `set_var` / `remove_var` are `unsafe` because the libc
/// calls aren't thread-safe; under `cargo test`'s default thread pool,
/// two env-mutating tests running in parallel can race and trigger UB.
/// Every test fn here that touches env vars MUST take this lock before
/// any `set_var` / `remove_var` (the trust-gate tests and the GitHub
/// PR-detection refresh test, #181). Mirrors the same pattern in
/// `trust_tests.rs` / `history_tests.rs`.
fn env_lock() -> &'static Mutex<()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  LOCK.get_or_init(|| Mutex::new(()))
}

/// Build a synthetic worktree row for state-machine tests that need a known
/// list shape (fuzzy filter ranking, multi-row navigation). Lets the test
/// drive the filter without going through real `git2::worktree_add` calls.
fn worktree_fixture(name: &str) -> WorktreeInfo {
  WorktreeInfo {
    name: name.into(),
    id: name.into(),
    path: PathBuf::from(format!("/tmp/gwm-test/{}", name)),
    branch: Some(format!("feat/#0-{}", name)),
    head: None,
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
    link: gwm::github::BranchLink::empty(),
    issue_state: None,
    pr_state: None,
    age: None,
  }
}

fn make_app() -> (tempfile::TempDir, App) {
  let (dir, _) = init_repo();
  let app = App::new_at_layered(Some(dir.path()), None).unwrap();
  (dir, app)
}

/// Index of `field` within the Settings TUI tab. Looked up rather than
/// hardcoded: inserting a field (as #365 did with `sidebar_orientation`)
/// silently shifts every literal index below it, which is how two numeric-
/// input tests started editing the wrong row.
fn tui_field_index(field: gwm::tui::SettingField) -> usize {
  gwm::tui::SettingsTab::Tui
    .fields()
    .iter()
    .position(|f| *f == field)
    .unwrap_or_else(|| panic!("{field:?} is not exposed in the Settings TUI tab"))
}

#[test]
fn focus_status_opens_and_focuses_the_sidebar() {
  // Issue #217: pressing `2` (focus_status) must open the sidebar if it was
  // closed and move the navigation focus onto it.
  let (_dir, mut app) = make_app();
  app.sidebar.open = false;
  app.sidebar.focused = false;
  app.focus_status();
  assert!(app.sidebar.open, "focus_status must open a closed sidebar");
  assert!(app.sidebar.focused, "focus_status must focus the sidebar");
}

#[test]
fn focus_worktrees_releases_sidebar_focus() {
  // Pressing `1` (focus_worktrees) returns navigation focus to the table so
  // `j` / `k` walk the worktree list, leaving the sidebar open but unfocused.
  let (_dir, mut app) = make_app();
  app.sidebar.open = true;
  app.sidebar.focused = true;
  app.focus_worktrees();
  assert!(!app.sidebar.focused, "focus_worktrees must release sidebar focus");
}

#[test]
fn enter_command_logs_opens_the_overlay_syncs_and_resets_scroll() {
  // Issue #226: `3` opens the Command Logs modal. Opening must (1) switch
  // to the overlay view, (2) sync the global command log into owned state,
  // and (3) reset the scroll cursor so a previously-scrolled session starts
  // fresh at the top.
  use gwm::command_log::{self, CommandLogEntry, CommandStatus};
  use std::time::Duration;

  let sentinel = "gwm-enter-cmdlog-9a1c";
  command_log::record(CommandLogEntry {
    command: format!("gh pr list # {sentinel}"),
    duration: Duration::from_millis(1),
    status: CommandStatus::Exited(Some(0)),
    output: String::new(),
  });

  let (_dir, mut app) = make_app();
  app.command_logs.scroll = 9;
  app.command_logs.x_scroll = 3;
  app.enter_command_logs();

  assert_eq!(app.view, View::CommandLogs);
  assert_eq!(app.command_logs.scroll, 0, "scroll resets on open");
  assert_eq!(app.command_logs.x_scroll, 0, "horizontal scroll resets on open");
  assert!(
    app.command_logs.entries.iter().any(|e| e.command.contains(sentinel)),
    "opening the overlay snapshots the global command log"
  );
}

#[test]
fn enter_config_panel_opens_resolves_rows_and_resets_scroll() {
  // Issue #232: `4` opens the Configuration panel. Opening must (1) switch
  // to the overlay view, (2) resolve the effective config into owned rows,
  // and (3) reset the scroll cursor so a previously-scrolled session starts
  // fresh at the top.
  use gwm::config::ConfigSource;

  let (_dir, mut app) = make_app();
  app.config_panel.scroll = 9;
  app.config_panel.x_scroll = 3;
  app.enter_config_panel();

  assert_eq!(app.view, View::Config);
  assert_eq!(app.config_panel.scroll, 0, "scroll resets on open");
  assert_eq!(app.config_panel.x_scroll, 0, "horizontal scroll resets on open");
  assert!(
    !app.config_panel.rows.is_empty(),
    "opening resolves the effective config into rows"
  );
  // The fixture has no repo `.gwm.toml` and no global config, so every
  // resolved value is a built-in default.
  let base = app
    .config_panel
    .rows
    .iter()
    .find(|r| r.key == "worktree.base")
    .expect("worktree.base resolved");
  assert_eq!(base.source, ConfigSource::Default);
}

// ── Keys tab: in-TUI keymap editor (issue #294) ────────────────────────────

#[test]
fn enter_config_panel_builds_the_keys_tab_rows() {
  use gwm::tui::keymap::Action;
  use gwm::tui::modal_keymap::ModalAction;

  let (_dir, mut app) = make_app();
  app.enter_config_panel();

  let expected = Action::all().count() + ModalAction::all().count();
  assert_eq!(
    app.config_panel.key_rows.len(),
    expected,
    "opening the panel enumerates every global + modal binding"
  );
}

#[test]
fn capturing_a_global_chord_rebinds_it_live_and_writes_the_file() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::{Action, ChordResolution, KeyStroke};
  use gwm::tui::{KeyTarget, SettingsTab};

  let (dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::Quit))
    .expect("quit row present");
  app.config_panel.selected = idx;

  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE));
  app.commit_key_capture();

  // The live keymap reflects the rebind without a restart.
  let q = KeyStroke::new(KeyCode::Char('Q'), KeyModifiers::NONE);
  assert_eq!(app.keymap.lookup(&[q]), ChordResolution::Matched(Action::Quit));

  // …and it round-trips to disk under `[tui.keys]`.
  let raw = std::fs::read_to_string(dir.path().join(".gwm.toml")).unwrap();
  assert!(raw.contains("quit = [\"Q\"]"), "binding persisted: {raw}");
}

#[test]
fn capturing_a_modal_verb_rebinds_it_live() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::{KeyContext, ModalAction};
  use gwm::tui::{KeyTarget, SettingsTab};

  let (_dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Modal(ModalAction::ConfirmConfirm))
    .expect("confirm row present");
  app.config_panel.selected = idx;

  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
  app.commit_key_capture();

  let o = KeyStroke::new(KeyCode::Char('o'), KeyModifiers::NONE);
  assert_eq!(
    app.modal_keymap.resolve(KeyContext::Confirm, &o),
    Some(ModalAction::ConfirmConfirm),
    "the modal verb fires on its new key immediately"
  );
}

#[test]
fn an_invalid_rebind_is_rejected_and_leaves_the_binding_live() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::{Action, ChordResolution, KeyStroke};
  use gwm::tui::{KeyTarget, SettingsTab};

  let (_dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::Refresh))
    .expect("refresh row present");
  app.config_panel.selected = idx;

  // `g` is a strict prefix of `top`'s default `g g` chord — a prefix
  // collision the validate-before-write gate must reject.
  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
  app.commit_key_capture();

  assert!(app.status.starts_with("keys:"), "error surfaced: {}", app.status);
  // The previous binding survives — `f` still refreshes.
  let f = KeyStroke::new(KeyCode::Char('f'), KeyModifiers::NONE);
  assert_eq!(app.keymap.lookup(&[f]), ChordResolution::Matched(Action::Refresh));
}

#[test]
fn cancelling_a_capture_writes_nothing() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::SettingsTab;

  let (dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  app.config_panel.selected = 0;

  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
  app.config_panel.cancel_capture();

  assert!(app.config_panel.capture.is_none(), "capture cleared on cancel");
  assert!(
    !dir.path().join(".gwm.toml").exists(),
    "a cancelled capture must not write the file"
  );
}

#[test]
fn a_cross_layer_conflict_rolls_back_and_does_not_brick_the_config() {
  // Codex #297 review (P2): `set_array_at` validates only the file it writes.
  // A rebind that is valid in the project file alone but collides with the
  // global layer once merged must NOT be left on disk — otherwise the next
  // launch's layered load fails and the repo config is bricked.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::config::Config;
  use gwm::config_cli::set_array_at;
  use gwm::tui::keymap::{Action, ChordResolution, KeyStroke};
  use gwm::tui::{App, KeyTarget, SettingsLayer, SettingsTab};

  let (repo, _) = init_repo();
  let home = tempfile::tempdir().unwrap();
  let global = home.path().join("gwm").join("config.toml");
  // The global layer binds `top` to a two-stroke chord.
  set_array_at(&global, "tui.keys.top", &["z z".to_string()]).unwrap();

  let mut app = App::new_at_layered(Some(repo.path()), Some(&global)).unwrap();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  app.config_panel.layer = SettingsLayer::Project; // write the repo file
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::Refresh))
    .unwrap();
  app.config_panel.selected = idx;

  // `z` alone is a prefix of the global `z z` — valid in the repo file by
  // itself, invalid once the layers merge.
  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
  app.commit_key_capture();

  assert!(app.status.starts_with("keys:"), "rejection surfaced: {}", app.status);
  // The merged config still loads — nothing was left broken on disk.
  assert!(
    Config::load_layered(repo.path(), Some(&global)).is_ok(),
    "the layered config must still load after a rolled-back rebind"
  );
  let repo_toml = repo.path().join(".gwm.toml");
  if repo_toml.exists() {
    let raw = std::fs::read_to_string(&repo_toml).unwrap();
    assert!(!raw.contains("refresh"), "the rejected rebind was rolled back: {raw}");
  }
  // The previous binding survives — `f` still refreshes.
  let f = KeyStroke::new(KeyCode::Char('f'), KeyModifiers::NONE);
  assert_eq!(app.keymap.lookup(&[f]), ChordResolution::Matched(Action::Refresh));
}

#[test]
fn a_shadowed_global_key_rebind_warns() {
  // Codex #297 review (P3): editing the global layer for a key the repo
  // overrides leaves the effective binding unchanged (repo wins). Mirror
  // `apply_setting` and flag the shadow rather than reporting a clean set.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::config_cli::set_array_at;
  use gwm::tui::keymap::Action;
  use gwm::tui::{App, KeyTarget, SettingsLayer, SettingsTab};

  let (repo, _) = init_repo();
  let home = tempfile::tempdir().unwrap();
  let global = home.path().join("gwm").join("config.toml");
  // The repo pins `quit` to `x`, so a global rebind of `quit` is shadowed.
  set_array_at(&repo.path().join(".gwm.toml"), "tui.keys.quit", &["x".to_string()]).unwrap();

  let mut app = App::new_at_layered(Some(repo.path()), Some(&global)).unwrap();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  app.config_panel.layer = SettingsLayer::Global;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::Quit))
    .unwrap();
  app.config_panel.selected = idx;

  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
  app.commit_key_capture();

  assert!(
    app.status.contains("shadowed"),
    "a shadowed global rebind must warn: {}",
    app.status
  );
}

#[test]
fn physical_enter_stays_reserved_even_with_a_custom_config_edit_submit() {
  // Codex #297 review (P2): rebinding `config.edit.submit` to e.g. Ctrl+s must
  // not make the *physical* Enter assignable in capture — it stays a reserved
  // control regardless of the config.edit lookup.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::config_cli::set_array_at;
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::{KeyContext, ModalAction};
  use gwm::tui::{App, KeyTarget, SettingsTab};

  let (repo, _) = init_repo();
  set_array_at(
    &repo.path().join(".gwm.toml"),
    "tui.keys.modal.config.edit.submit",
    &["Ctrl+s".to_string()],
  )
  .unwrap();

  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Modal(ModalAction::ConfirmConfirm))
    .unwrap();
  app.config_panel.selected = idx;
  app.config_panel.begin_capture();

  // Physical Enter is reserved: ignored, not captured — capture stays armed.
  app.handle_capture_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
  assert!(app.config_panel.capture.is_some(), "physical Enter must stay reserved");
  let enter = KeyStroke::new(KeyCode::Enter, KeyModifiers::NONE);
  assert_ne!(
    app.modal_keymap.resolve(KeyContext::Confirm, &enter),
    Some(ModalAction::ConfirmConfirm),
    "Enter must not have become the binding"
  );
}

#[test]
fn a_failed_write_to_an_already_invalid_shadowed_file_rolls_back() {
  // Codex #297 review (P2): when the target file is invalid on its own but
  // loaded because the bad value is shadowed by another layer,
  // `write_and_validate` writes the edit then returns Err (recovery path). The
  // commit must roll back so a rebind reported as failed never persists.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::Action;
  use gwm::tui::{App, KeyTarget, SettingsLayer, SettingsTab};

  let (repo, _) = init_repo();
  let home = tempfile::tempdir().unwrap();
  let global = home.path().join("gwm").join("config.toml");
  std::fs::create_dir_all(global.parent().unwrap()).unwrap();
  // Global is invalid on its own (non-numeric countdown)…
  std::fs::write(&global, "[tui]\nconfirm_countdown_secs = \"abc\"\n").unwrap();
  // …but the repo overrides it with a valid value, so the merged config loads.
  std::fs::write(repo.path().join(".gwm.toml"), "[tui]\nconfirm_countdown_secs = 4\n").unwrap();

  let mut app = App::new_at_layered(Some(repo.path()), Some(&global)).unwrap();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  app.config_panel.layer = SettingsLayer::Global; // write the invalid file
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::Quit))
    .unwrap();
  app.config_panel.selected = idx;

  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE));
  app.commit_key_capture();

  assert!(app.status.starts_with("keys:"), "failure surfaced: {}", app.status);
  let raw = std::fs::read_to_string(&global).unwrap();
  assert!(
    !raw.contains("quit"),
    "a failed write must be rolled back, not persisted: {raw}"
  );
}

#[test]
fn modal_capture_reserves_enter_and_backspace_as_controls() {
  // Codex #297 review (P2): Enter / Backspace must stay reserved capture
  // controls even for a single-stroke modal target — they can't be captured
  // as a binding (hand-edit for those), matching the documented controls.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::{KeyContext, ModalAction};
  use gwm::tui::{KeyTarget, SettingsTab};

  let (_dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Modal(ModalAction::ConfirmConfirm))
    .unwrap();
  app.config_panel.selected = idx;
  app.config_panel.begin_capture();

  // Enter and Backspace are ignored — the capture stays armed, nothing bound.
  app.handle_capture_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
  assert!(
    app.config_panel.capture.is_some(),
    "Enter must not capture a modal verb"
  );
  app.handle_capture_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
  assert!(
    app.config_panel.capture.is_some(),
    "Backspace must not capture a modal verb"
  );
  let enter = KeyStroke::new(KeyCode::Enter, KeyModifiers::NONE);
  assert_ne!(
    app.modal_keymap.resolve(KeyContext::Confirm, &enter),
    Some(ModalAction::ConfirmConfirm),
    "Enter must not have become the binding"
  );

  // A real key auto-commits the single-stroke modal capture.
  app.handle_capture_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
  assert!(app.config_panel.capture.is_none(), "a real key commits the capture");
  let o = KeyStroke::new(KeyCode::Char('o'), KeyModifiers::NONE);
  assert_eq!(
    app.modal_keymap.resolve(KeyContext::Confirm, &o),
    Some(ModalAction::ConfirmConfirm)
  );
}

#[test]
fn global_capture_commits_on_enter_and_pops_on_backspace() {
  // The global (multi-stroke) path: real keys accumulate, Backspace drops the
  // last, Enter commits — all through the testable handler.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::{Action, ChordResolution, KeyStroke};
  use gwm::tui::{KeyTarget, SettingsTab};

  let (_dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::Refresh))
    .unwrap();
  app.config_panel.selected = idx;
  app.config_panel.begin_capture();

  app.handle_capture_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
  app.handle_capture_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
  assert!(
    app.config_panel.capture.is_some(),
    "global chord accumulates, no auto-commit"
  );
  app.handle_capture_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)); // drop 'z'
  app.handle_capture_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // commit "x"

  assert!(app.config_panel.capture.is_none(), "Enter commits the global chord");
  let x = KeyStroke::new(KeyCode::Char('x'), KeyModifiers::NONE);
  assert_eq!(app.keymap.lookup(&[x]), ChordResolution::Matched(Action::Refresh));
}

#[test]
fn an_unbind_shadowed_by_another_layer_warns() {
  // Codex #297 review (P2): an empty capture (unbind) on a low-precedence
  // layer is shadowed when a higher layer still binds the action — warn rather
  // than report a clean `unbound`.
  use gwm::config_cli::set_array_at;
  use gwm::tui::keymap::Action;
  use gwm::tui::{App, KeyTarget, SettingsLayer, SettingsTab};

  let (repo, _) = init_repo();
  let home = tempfile::tempdir().unwrap();
  let global = home.path().join("gwm").join("config.toml");
  set_array_at(&repo.path().join(".gwm.toml"), "tui.keys.quit", &["x".to_string()]).unwrap();

  let mut app = App::new_at_layered(Some(repo.path()), Some(&global)).unwrap();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  app.config_panel.layer = SettingsLayer::Global;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::Quit))
    .unwrap();
  app.config_panel.selected = idx;

  // Empty capture → unbind written to the global layer.
  app.config_panel.begin_capture();
  app.commit_key_capture();

  assert!(
    app.status.contains("unbound"),
    "status reports the unbind: {}",
    app.status
  );
  assert!(
    app.status.contains("shadowed"),
    "a shadowed unbind must warn: {}",
    app.status
  );
}

#[test]
fn a_cross_layer_alias_shadow_is_detected_and_warned() {
  // Codex #297 review (P2, cross-layer): the legacy alias lives in the OTHER
  // layer (global `open_menu`) while the canonical `browse_links` is rebound in
  // the project layer. We don't edit the global file, so the alias survives the
  // merge and shadows the new key — the commit must detect the ineffective
  // rebind and warn rather than report a clean success.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::config_cli::set_array_at;
  use gwm::tui::keymap::{Action, ChordResolution, KeyStroke};
  use gwm::tui::{App, KeyTarget, SettingsLayer, SettingsTab};

  let (repo, _) = init_repo();
  let home = tempfile::tempdir().unwrap();
  let global = home.path().join("gwm").join("config.toml");
  set_array_at(&global, "tui.keys.open_menu", &["B".to_string()]).unwrap();

  let mut app = App::new_at_layered(Some(repo.path()), Some(&global)).unwrap();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  app.config_panel.layer = SettingsLayer::Project;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::BrowseLinks))
    .unwrap();
  app.config_panel.selected = idx;

  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
  app.commit_key_capture();

  assert!(
    app.status.contains("shadowed"),
    "a cross-layer alias shadow must warn: {}",
    app.status
  );
  // The new key really doesn't fire — the global alias `B` still wins.
  let z = KeyStroke::new(KeyCode::Char('z'), KeyModifiers::NONE);
  assert_ne!(app.keymap.lookup(&[z]), ChordResolution::Matched(Action::BrowseLinks));
}

#[test]
fn rebinding_an_aliased_action_strips_the_legacy_alias() {
  // Codex #297 review (P2): a pre-#290 config carrying `tui.keys.open_menu`
  // (alias for `browse_links`) would, after a rebind writes the canonical
  // `browse_links`, re-apply the alias last in the sorted override walk and
  // silently shadow the new key. The commit must strip the alias so the new
  // binding actually wins.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::config_cli::set_array_at;
  use gwm::tui::keymap::{Action, ChordResolution, KeyStroke};
  use gwm::tui::{App, KeyTarget, SettingsLayer, SettingsTab};

  let (repo, _) = init_repo();
  let toml = repo.path().join(".gwm.toml");
  set_array_at(&toml, "tui.keys.open_menu", &["B".to_string()]).unwrap();

  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Keys;
  app.config_panel.layer = SettingsLayer::Project;
  let idx = app
    .config_panel
    .key_rows
    .iter()
    .position(|r| r.target == KeyTarget::Global(Action::BrowseLinks))
    .unwrap();
  app.config_panel.selected = idx;

  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
  app.commit_key_capture();

  let raw = std::fs::read_to_string(&toml).unwrap();
  assert!(!raw.contains("open_menu"), "the legacy alias was stripped: {raw}");
  // The new key actually wins — the alias can no longer shadow it on reload.
  let z = KeyStroke::new(KeyCode::Char('z'), KeyModifiers::NONE);
  assert_eq!(app.keymap.lookup(&[z]), ChordResolution::Matched(Action::BrowseLinks));
}

#[test]
fn hint_context_follows_focus() {
  // Issue #217: the statusbar chip + help subtitle read the live focus. The
  // worktrees pane is the default; focusing the sidebar switches to Status.
  use gwm::tui::HintContext;
  let (_dir, mut app) = make_app();
  app.focus_worktrees();
  assert_eq!(app.hint_context(), HintContext::Worktrees);
  app.focus_status();
  assert_eq!(app.hint_context(), HintContext::Status);
}

#[test]
fn focused_panel_border_wears_the_theme_focus_colour() {
  // #185: the focus-swappable panel borders (worktree list ↔ sidebar,
  // toggled with Tab) must paint with the theme `focus` role, not a
  // hardcoded cyan — otherwise the focused "tab" ignores the active
  // palette (e.g. the Claude orange default). Unfocused panels stay
  // muted.
  let theme = Theme::default();
  assert_eq!(
    panel_border_color(true, &theme),
    theme.focus,
    "focused panel must wear the theme focus colour"
  );
  assert_eq!(
    panel_border_color(false, &theme),
    theme.muted,
    "unfocused panel wears the theme muted role (#170)"
  );
}

#[test]
fn new_loads_main_worktree() {
  let (_dir, app) = make_app();
  assert_eq!(app.worktrees.len(), 1);
  assert!(app.worktrees[0].is_main);
}

#[test]
fn enter_create_opens_focused_on_the_issue_field() {
  // Issue #217 UX: the modal opens focused on Issue (not the cycle-only
  // Type field) so the very first keypress edits text instead of being a
  // silent no-op — the trap that read as "typing is broken". The Type
  // field keeps its sensible default (index 0) and stays reachable via
  // Shift-Tab / the field rotation.
  let (_dir, mut app) = make_app();
  app.enter_create();
  assert_eq!(app.view, View::Create);
  assert_eq!(app.create_form.field, Field::Issue);
  assert_eq!(app.create_form.type_index, 0, "type keeps its default");
  assert!(app.create_form.issue.is_empty());
  assert!(app.create_form.desc.is_empty());
}

#[test]
fn create_field_navigation_loops() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  // Pin the start to Type so this exercises the full rotation contract
  // independently of where the modal opens its focus (#217).
  app.create_form.field = Field::Type;
  app.create_next_field();
  assert_eq!(app.create_form.field, Field::Issue);
  app.create_next_field();
  assert_eq!(app.create_form.field, Field::Desc);
  app.create_next_field();
  assert_eq!(app.create_form.field, Field::Type);
  app.create_prev_field();
  assert_eq!(app.create_form.field, Field::Desc);
}

#[test]
fn create_type_navigation_loops() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_prev_type();
  assert_eq!(app.create_form.type_index, BRANCH_TYPES.len() - 1);
  app.create_next_type();
  assert_eq!(app.create_form.type_index, 0);
}

#[test]
fn backspace_stays_a_reserved_eraser_in_the_create_form() {
  // Codex review #456 (iteration 7): a modal rebind like
  // `[tui.keys.modal.create] cancel = ["Backspace"]` used to resolve
  // BEFORE the typing fallback, so Backspace cancelled the form and the
  // text fields lost their eraser. The physical Backspace is a reserved
  // editing control on the input fields now — same contract as the
  // Keys-tab capture's reserved Esc / Enter / Backspace.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  use gwm::tui::CreateKey;
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(ModalAction::CreateCancel, KeyStroke::parse_chord("Backspace").unwrap())
    .unwrap();
  app.enter_create();
  app.create_form.field = Field::Issue;
  app.create_push_char('4');
  app.create_push_char('2');
  let out = app.handle_create_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
  assert_ne!(
    out,
    CreateKey::Cancel,
    "Backspace must not cancel while a text field is focused"
  );
  assert_eq!(app.create_form.issue, "4", "Backspace must erase the last digit");
}

#[test]
fn backspace_stays_a_reserved_eraser_in_the_link_number_input() {
  // Same reserved-control contract for the link prompt's number stage
  // (Codex review #456): `[tui.keys.modal.link.input_number] cancel =
  // ["Backspace"]` must not leave the digits without an eraser.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  use gwm::tui::LinkPromptKey;
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(
      ModalAction::LinkInputCancel,
      KeyStroke::parse_chord("Backspace").unwrap(),
    )
    .unwrap();
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
  app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
  let out = app.handle_link_prompt_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
  assert_ne!(out, LinkPromptKey::Cancel, "Backspace must not cancel the number input");
  assert_eq!(
    app.link_prompt_number_input(),
    "4",
    "Backspace must erase the last digit"
  );
}

#[test]
fn printable_keys_stay_typing_in_the_create_form_despite_rebinds() {
  // Codex review #456 (iteration 8): a modal rebind onto a printable key
  // (`cancel = ["q"]`) resolved before the typing fallback, so `q` closed
  // the form mid-word. On a text field the printable keys are reserved
  // for typing (the palette convention); modal verbs act through their
  // non-printable defaults.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  use gwm::tui::CreateKey;
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(ModalAction::CreateCancel, KeyStroke::parse_chord("q").unwrap())
    .unwrap();
  app.enter_create();
  app.create_form.field = Field::Desc;
  let out = app.handle_create_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
  assert_ne!(out, CreateKey::Cancel, "a printable rebind must not fire while typing");
  assert_eq!(app.create_form.desc, "q", "the character types into the field instead");
}

#[test]
fn printable_keys_stay_typing_in_the_link_number_input_despite_rebinds() {
  // Same reserved-typing contract for the link prompt's number stage.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  use gwm::tui::LinkPromptKey;
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(ModalAction::LinkInputCancel, KeyStroke::parse_chord("5").unwrap())
    .unwrap();
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  let out = app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
  assert_ne!(
    out,
    LinkPromptKey::Cancel,
    "a printable rebind must not fire while typing digits"
  );
  assert_eq!(
    app.link_prompt_number_input(),
    "5",
    "the digit types into the number instead"
  );
}

#[test]
fn palette_input_routes_typing_before_modal_rebinds() {
  // Codex review #456 (iteration 8): `palette.close = ["x"]` used to
  // close the palette instead of filtering. The typing route is a
  // testable App method now, called before the modal resolution: the
  // filter charset and Backspace always type / erase.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(ModalAction::CommandPaletteClose, KeyStroke::parse_chord("x").unwrap())
    .unwrap();
  app.open_command_palette();
  assert!(
    app.palette_input_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
    "a charset key is consumed as typing"
  );
  assert_eq!(app.palette.buffer(), "x", "the character filters instead of closing");
  app
    .modal_keymap
    .apply_override(
      ModalAction::CommandPaletteAccept,
      KeyStroke::parse_chord("Backspace").unwrap(),
    )
    .unwrap();
  assert!(
    app.palette_input_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
    "Backspace is consumed as the eraser"
  );
  assert_eq!(app.palette.buffer(), "", "Backspace erases despite the rebind");
}

#[test]
fn settings_editor_routes_typing_before_modal_rebinds() {
  // Same contract for the Settings value editor (Codex review #456):
  // `config.edit.submit = ["5"]` must not commit while typing a 5, and a
  // Backspace rebind must not swallow the eraser.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(ModalAction::ConfigEditSubmit, KeyStroke::parse_chord("5").unwrap())
    .unwrap();
  app.config_panel.editing = Some("4".into());
  assert!(
    app.settings_edit_input_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE)),
    "a printable key is consumed as typing"
  );
  assert_eq!(app.config_panel.editing.as_deref(), Some("45"));
  app
    .modal_keymap
    .apply_override(
      ModalAction::ConfigEditCancel,
      KeyStroke::parse_chord("Backspace").unwrap(),
    )
    .unwrap();
  assert!(
    app.settings_edit_input_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
    "Backspace is consumed as the eraser"
  );
  assert_eq!(app.config_panel.editing.as_deref(), Some("4"));
}

#[test]
fn modified_strokes_reach_the_modal_resolution_in_input_modes() {
  // Codex review #456 (iteration 9): the reserved-typing routes must not
  // swallow Ctrl/Alt-modified strokes — the parser accepts bindings like
  // `close = ["Alt+x"]` or `accept = ["Ctrl+Backspace"]` and they have to
  // stay reachable while typing. Only unmodified legitimate input is
  // reserved.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  assert!(
    !app.palette_input_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
    "Alt+x must fall through to the modal resolution"
  );
  assert!(
    !app.palette_input_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)),
    "Ctrl+Backspace must fall through to the modal resolution"
  );
  app.config_panel.editing = Some("1".into());
  assert!(
    !app.settings_edit_input_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
    "Alt+x must fall through in the settings editor"
  );
  assert!(
    !app.settings_edit_input_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)),
    "Ctrl+Backspace must fall through in the settings editor"
  );
}

#[test]
fn settings_editor_reinjects_unresolved_modified_characters() {
  // Codex review #456 (iteration 10): AltGr/Option printables arrive as
  // Char + ALT on some keyboards. They are not reserved typing (a bound
  // Alt+x must stay reachable), but when the modal resolution comes up
  // empty the character IS typing and must reach the buffer — the old
  // fallback pushed it, the modifier-aware route dropped it, making
  // characters like @ or { impossible to type.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, mut app) = make_app();
  app.config_panel.editing = Some("a".into());
  app.handle_settings_edit_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::ALT));
  assert_eq!(
    app.config_panel.editing.as_deref(),
    Some("a@"),
    "an unresolved AltGr character must still type"
  );
}

#[test]
fn shifted_uppercase_never_counts_as_palette_typing() {
  // Codex review #456 (iteration 10): kitty-style terminals report an
  // uppercase X as Char('x') + SHIFT — the very case KeyStroke::from_event
  // normalises. The palette only takes lowercase input, so that stroke is
  // NOT canonical typing and must fall through to the modal resolution
  // (a binding on "X" stays reachable on every terminal encoding).
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  assert!(
    !app.palette_input_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::SHIFT)),
    "a shifted letter is an uppercase, not palette input"
  );
  assert_eq!(app.palette.buffer(), "", "nothing must have been typed");
}

#[test]
fn create_push_only_digits_on_issue() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_form.field = Field::Issue;
  for c in "12a3".chars() {
    app.create_push_char(c);
  }
  assert_eq!(app.create_form.issue, "123");
}

#[test]
fn create_push_accepts_desc_chars() {
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_form.field = Field::Desc;
  for c in "foo-bar".chars() {
    app.create_push_char(c);
  }
  assert_eq!(app.create_form.desc, "foo-bar");
  app.create_pop_char();
  assert_eq!(app.create_form.desc, "foo-ba");
}

#[test]
fn enter_confirm_delete_refuses_main() {
  let (_dir, mut app) = make_app();
  app.enter_confirm_delete();
  assert_eq!(app.view, View::List, "main worktree should not allow delete view");
}

#[test]
fn toggle_delete_branch_flips() {
  let (_dir, mut app) = make_app();
  assert!(!app.delete_branch_on_remove);
  app.toggle_delete_branch();
  assert!(app.delete_branch_on_remove);
}

#[test]
fn next_prev_with_single_entry_stays_put() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.next();
  assert_eq!(app.list_state.selected(), Some(0));
  app.prev();
  assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn refresh_keeps_selection_in_bounds() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(5));
  app.refresh().unwrap();
  assert_eq!(app.list_state.selected(), Some(0));
}

// ---- sidebar / focus / vim motions ---------------------------------------

#[test]
fn sidebar_open_by_default() {
  let (_dir, app) = make_app();
  assert!(
    app.sidebar.open,
    "sidebar should default to open (will be hidden when narrow)"
  );
  assert!(!app.sidebar.focused, "focus defaults to the worktree list");
}

#[test]
fn toggle_sidebar_flips_open_flag() {
  let (_dir, mut app) = make_app();
  let before = app.sidebar.open;
  app.toggle_sidebar();
  assert_eq!(app.sidebar.open, !before);
  app.toggle_sidebar();
  assert_eq!(app.sidebar.open, before);
}

#[test]
fn toggle_sidebar_when_closed_resets_focus_to_list() {
  let (_dir, mut app) = make_app();
  app.sidebar.focused = true;
  app.sidebar.open = true;
  app.toggle_sidebar(); // close
  assert!(!app.sidebar.open);
  assert!(
    !app.sidebar.focused,
    "closing the sidebar must drop focus back to the list"
  );
}

#[test]
fn toggle_focus_only_works_when_sidebar_open() {
  let (_dir, mut app) = make_app();
  app.sidebar.open = false;
  app.toggle_focus();
  assert!(!app.sidebar.focused, "focus cannot move to a hidden sidebar");

  app.sidebar.open = true;
  app.toggle_focus();
  assert!(app.sidebar.focused);
  app.toggle_focus();
  assert!(!app.sidebar.focused);
}

#[test]
fn first_selects_first_worktree() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.first();
  assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn last_selects_last_worktree() {
  let (_dir, mut app) = make_app();
  app.last();
  let expected = app.worktrees.len().saturating_sub(1);
  assert_eq!(app.list_state.selected(), Some(expected));
}

#[test]
fn handle_g_motion_tracks_pending_then_jumps_to_first() {
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  // First `g` arms the motion but does not move.
  assert!(!app.pending_g);
  app.handle_g();
  assert!(app.pending_g, "first 'g' must arm the gg sequence");
  // Second `g` jumps to first and disarms.
  app.handle_g();
  assert!(!app.pending_g, "second 'g' completes gg and disarms");
  assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn pending_g_resets_on_other_key() {
  let (_dir, mut app) = make_app();
  app.handle_g();
  assert!(app.pending_g);
  app.cancel_pending_motion();
  assert!(!app.pending_g, "any non-g keypress must drop the pending motion");
}

#[test]
fn sidebar_scroll_clamps_to_zero() {
  let (_dir, mut app) = make_app();
  assert_eq!(app.sidebar.scroll, 0);
  app.sidebar_scroll_up();
  assert_eq!(app.sidebar.scroll, 0, "scrolling up from 0 stays at 0");

  // The renderer normally publishes a max bound; simulate enough room for scroll.
  app.sidebar.max_scroll = 5;
  app.sidebar_scroll_down();
  assert_eq!(app.sidebar.scroll, 1);
  app.sidebar_scroll_up();
  assert_eq!(app.sidebar.scroll, 0);
}

#[test]
fn sidebar_scroll_clamps_at_max() {
  // The renderer sets `sidebar.max_scroll`. Scrolling past it must stop there
  // so the user can't push the panel content entirely off-screen.
  let (_dir, mut app) = make_app();
  app.sidebar.max_scroll = 3;
  app.sidebar_scroll_down();
  app.sidebar_scroll_down();
  app.sidebar_scroll_down();
  assert_eq!(app.sidebar.scroll, 3);
  app.sidebar_scroll_down();
  assert_eq!(app.sidebar.scroll, 3, "scrolling beyond max must clamp");
}

#[test]
fn wt_scroll_requires_status_focus() {
  // Issue #437: `J` / `K` scroll the Working Tree pane only while the
  // status pane holds the navigation focus — same gate as `j` / `k`
  // routing in `next()` / `prev()`. Unfocused, the keys are inert so
  // they stay free for future list-view use.
  let (_dir, mut app) = make_app();
  app.sidebar.wt_max_scroll = 5;
  assert!(!app.sidebar.focused);
  app.wt_scroll_down();
  assert_eq!(
    app.sidebar.wt_scroll, 0,
    "unfocused sidebar must not scroll the Working Tree"
  );

  app.focus_status();
  app.wt_scroll_down();
  assert_eq!(app.sidebar.wt_scroll, 1);
}

#[test]
fn wt_scroll_clamps_between_zero_and_max() {
  // The renderer republishes `wt_max_scroll` every frame; scrolling must
  // clamp against it (down) and saturate at zero (up) exactly like the
  // Recent Commits offset.
  let (_dir, mut app) = make_app();
  app.focus_status();
  app.wt_scroll_up();
  assert_eq!(app.sidebar.wt_scroll, 0, "scrolling up from 0 stays at 0");

  app.sidebar.wt_max_scroll = 2;
  app.wt_scroll_down();
  app.wt_scroll_down();
  app.wt_scroll_down();
  assert_eq!(app.sidebar.wt_scroll, 2, "scrolling beyond max must clamp");
  app.wt_scroll_up();
  assert_eq!(app.sidebar.wt_scroll, 1);
}

#[test]
fn navigation_resets_wt_scroll() {
  // Selection change → new worktree → the previous Working Tree offset is
  // meaningless. Same reset contract as the Recent Commits scroll.
  let (_dir, mut app) = make_app();
  app.sidebar.wt_max_scroll = 5;
  app.sidebar.wt_scroll = 3;
  app.on_navigation();
  assert_eq!(
    app.sidebar.wt_scroll, 0,
    "navigation must reset the Working Tree scroll"
  );
}

#[test]
fn cycle_mode_resets_wt_scroll() {
  // Stashes mode renders no Working Tree section; a stale offset would
  // land on unrelated content when toggling back to Commits.
  let (_dir, mut app) = make_app();
  app.sidebar.wt_max_scroll = 5;
  app.sidebar.wt_scroll = 3;
  app.sidebar.cycle_mode();
  assert_eq!(
    app.sidebar.wt_scroll, 0,
    "mode toggle must reset the Working Tree scroll"
  );
}

#[test]
fn section_heights_fit_naturally_with_commits_absorbing_slack() {
  // Issue #438: when everything fits, each variable section keeps its
  // natural height (content + 2 borders) and Recent Commits absorbs the
  // remaining space — the exact behaviour the old `Min(3)` constraint
  // produced, now pinned through the pure solver.
  use gwm::tui::state::sidebar::split_section_heights;
  assert_eq!(split_section_heights(60, 3, 10, 20), (5, 12, 43));
}

#[test]
fn section_heights_guarantee_floor_and_share_proportionally_on_overflow() {
  // Overflow: the scrollable sections are guaranteed their floor —
  // min(natural, 7) for Working Tree (validation feedback on PR #455: the
  // 5-line floor read too small in the field), min(natural, 5) for Recent
  // Commits — and share the surplus proportionally to content size, capped
  // at natural height, residue cascading to Recent Commits first. Agents
  // cannot scroll so it keeps its natural height outright (see
  // section_heights_never_clamp_the_agents_pane).
  // available=21, agents=6 (natural 8, kept), wt=30 (natural 32, floor 7),
  // commits=50 (natural 52, floor 5): base 20, surplus 1 → give =
  // 1*len/86 = (0, 0, 0), residue 1 → commits. Sum == available exactly.
  use gwm::tui::state::sidebar::split_section_heights;
  assert_eq!(split_section_heights(21, 6, 30, 50), (8, 7, 6));
}

#[test]
fn section_heights_never_clamp_the_agents_pane() {
  // Codex review on PR #454: the Agents pane has no scroll — clamping it
  // below its content permanently hides the trailing "+N more" row (3
  // pinned rows + the overflow indicator = 4 content rows max, bounded by
  // agent_pane_lines). A non-scrollable section keeps its natural height
  // even when the column overflows; only the scrollable sections clamp.
  use gwm::tui::state::sidebar::split_section_heights;
  let (agents, wt, commits) = split_section_heights(21, 4, 30, 50);
  assert_eq!(agents, 6, "agents must keep natural height (4 rows + borders)");
  assert_eq!((agents, wt, commits), (6, 8, 7));
}

#[test]
fn section_heights_keep_empty_sections_collapsed() {
  // A section with no content keeps its collapse behaviour: Agents hidden
  // when no session, Working Tree at 0 when the tree is clean. The
  // collapsed section never eats a 5-line floor.
  use gwm::tui::state::sidebar::split_section_heights;
  assert_eq!(split_section_heights(40, 0, 5, 10), (0, 7, 33));
  assert_eq!(split_section_heights(30, 2, 0, 8), (4, 0, 26));
}

#[test]
fn section_heights_degrade_commits_first_on_tiny_terminal() {
  // available below the floors' sum: hand out what exists with Recent
  // Commits served first (the historical always-visible section), then
  // Working Tree, then Agents. Sum must never exceed the available height.
  use gwm::tui::state::sidebar::split_section_heights;
  assert_eq!(split_section_heights(8, 6, 30, 50), (0, 3, 5));
}

#[test]
fn section_heights_survive_empty_commits_under_overflow() {
  // Codex review on PR #454: with an empty history (natural 2) the commits
  // floor used to be raised to 3, breaking the floor <= natural invariant
  // the sharing math relies on — `nat - floor` underflowed and panicked in
  // debug builds. The natural height of commits is now floored at 3 (the
  // old `Min(3)` rendered an empty bordered panel at 3 lines anyway), so
  // the invariant holds and the split stays additive.
  use gwm::tui::state::sidebar::split_section_heights;
  assert_eq!(split_section_heights(8, 0, 5, 0), (0, 5, 3));
}

#[test]
fn section_heights_give_everything_to_commits_when_alone() {
  // No agents, clean tree, empty history: Recent Commits keeps the whole
  // column, matching the pre-#438 rendering of an empty bottom panel.
  use gwm::tui::state::sidebar::split_section_heights;
  assert_eq!(split_section_heights(20, 0, 0, 0), (0, 0, 20));
}

#[test]
fn focus_routes_navigation_to_sidebar() {
  // When sidebar is focused, next()/prev() should NOT move the list selection.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.sidebar.open = true;
  app.sidebar.focused = true;
  app.sidebar.max_scroll = 5; // pretend the renderer has populated this

  app.next();
  assert_eq!(
    app.list_state.selected(),
    Some(0),
    "list must stay put when sidebar has focus"
  );
  assert!(
    app.sidebar.scroll >= 1,
    "next() must scroll the sidebar when it has focus"
  );

  app.prev();
  assert_eq!(app.list_state.selected(), Some(0));
  assert_eq!(app.sidebar.scroll, 0, "prev() scrolled back up");
}

#[test]
fn next_prev_invalidate_sidebar_cache() {
  // Moving selection must drop any cached sidebar content so the new
  // worktree's preview is recomputed on the next frame.
  let (_dir, mut app) = make_app();
  app.sidebar.cache = Some((
    (
      std::path::PathBuf::from("/tmp/x"),
      gwm::tui::state::sidebar::SidebarMode::Commits,
    ),
    Default::default(),
  ));
  app.next();
  assert!(app.sidebar.cache.is_none(), "next() must invalidate the sidebar cache");

  app.sidebar.cache = Some((
    (
      std::path::PathBuf::from("/tmp/x"),
      gwm::tui::state::sidebar::SidebarMode::Commits,
    ),
    Default::default(),
  ));
  app.prev();
  assert!(app.sidebar.cache.is_none(), "prev() must invalidate the sidebar cache");
}

#[test]
fn refresh_invalidates_sidebar_cache() {
  let (_dir, mut app) = make_app();
  app.sidebar.cache = Some((
    (
      std::path::PathBuf::from("/tmp/x"),
      gwm::tui::state::sidebar::SidebarMode::Commits,
    ),
    Default::default(),
  ));
  app.refresh().unwrap();
  assert!(app.sidebar.cache.is_none());
}

#[test]
fn on_navigation_resets_scroll_and_invalidates_sidebar_cache() {
  // Pre-extraction, `next`, `prev`, `first`, `last` each repeated the
  // verbatim triple `sidebar_scroll = 0; invalidate_sidebar_cache();
  // refresh_link();`. Issue #127 collapses the first two pieces into
  // `SidebarState::on_navigation` and pairs them with `refresh_link()`
  // inside `App::on_navigation`, so the next time a navigation method
  // needs the reset, it goes through this single entry point.
  //
  // This integration test asserts the two observable pieces from this
  // fixture: scroll resets to 0, cache drops. `refresh_link()` also
  // runs (it's wired into `App::on_navigation`) but can't be observed
  // here — the test repo has no GitHub remote, so the link stays
  // `BranchLink::empty()` whether `refresh_link()` ran or not. The
  // unit tests for `SidebarState::on_navigation` cover the sub-struct
  // half of the contract in isolation.
  let (_dir, mut app) = make_app();
  app.sidebar.scroll = 7;
  app.sidebar.cache = Some((
    (
      std::path::PathBuf::from("/tmp/x"),
      gwm::tui::state::sidebar::SidebarMode::Commits,
    ),
    Default::default(),
  ));

  app.on_navigation();

  assert_eq!(app.sidebar.scroll, 0, "on_navigation must reset scroll to 0");
  assert!(
    app.sidebar.cache.is_none(),
    "on_navigation must drop the cached sidebar sections"
  );
}

// ---- fuzzy filter (issue #21) -------------------------------------------

#[test]
fn filter_state_defaults_to_inactive_and_empty() {
  let (_dir, app) = make_app();
  assert!(!app.filter.active, "filter must default to inactive");
  assert!(app.filter.query().is_empty(), "filter query must default to empty");
}

#[test]
fn enter_filter_activates_capture_and_disarms_gg() {
  let (_dir, mut app) = make_app();
  app.handle_g(); // arm `gg`
  assert!(app.pending_g);

  app.enter_filter();
  assert!(app.filter.active);
  assert!(
    !app.pending_g,
    "opening the filter bar must drop any half-typed gg motion"
  );
}

#[test]
fn enter_filter_drops_sidebar_focus() {
  // regression: PR #44 Copilot review — sidebar focus survived `/` and broke
  // j/k navigation after Enter committed the filter.
  // Regression for the Copilot review on PR #44: if the sidebar held focus
  // when the user hit `/`, after `exit_filter_keep` (Enter) the focus would
  // still be on the sidebar, so `j` / `k` would scroll it instead of walking
  // the filtered worktrees — contradicting the documented "navigation
  // returns to the table" contract. Opening the filter bar must therefore
  // pre-emptively pull focus back to the list.
  let (_dir, mut app) = make_app();
  app.sidebar.open = true;
  app.sidebar.focused = true;

  app.enter_filter();
  assert!(
    !app.sidebar.focused,
    "opening the filter bar must hand focus back to the list"
  );
}

#[test]
fn enter_filter_preserves_existing_query() {
  // Hitting `/` on a sticky filter re-opens the bar so the user can refine
  // it; only Esc clears.
  let (_dir, mut app) = make_app();
  app.filter.set_query("auth".into());
  app.enter_filter();
  assert_eq!(app.filter.query(), "auth");
  assert!(app.filter.active);
}

#[test]
fn filter_push_char_appends_to_query() {
  let (_dir, mut app) = make_app();
  app.enter_filter();
  for c in "tui".chars() {
    app.filter_push_char(c);
  }
  assert_eq!(app.filter.query(), "tui");
}

#[test]
fn filter_pop_char_removes_last_char() {
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter.set_query("tuix".into());
  app.filter_pop_char();
  assert_eq!(app.filter.query(), "tui");
}

#[test]
fn filter_pop_char_on_empty_is_noop() {
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter_pop_char();
  assert_eq!(app.filter.query(), "");
  assert!(app.filter.active, "popping an empty query must not exit filter mode");
}

#[test]
fn exit_filter_keep_disables_capture_keeps_query() {
  // Enter behaviour: filter sticks, navigation returns to the list.
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter.set_query("auth".into());
  app.exit_filter_keep();
  assert!(!app.filter.active);
  assert_eq!(app.filter.query(), "auth", "Enter must not wipe the query");
}

#[test]
fn exit_filter_cancel_clears_query() {
  // Esc behaviour: full list back, query gone.
  let (_dir, mut app) = make_app();
  app.enter_filter();
  app.filter.set_query("auth".into());
  app.exit_filter_cancel();
  assert!(!app.filter.active);
  assert!(app.filter.query().is_empty(), "Esc must clear the query");
}

#[test]
fn filtered_indices_returns_all_when_query_empty() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("beta"),
    worktree_fixture("gamma"),
  ];
  let idx: Vec<usize> = app.filtered_indices().to_vec();
  assert_eq!(idx, vec![0, 1, 2], "empty query is the identity over worktrees");
}

#[test]
fn filtered_indices_keeps_only_matching_worktrees() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("feat-1-tui-search"),
    worktree_fixture("feat-2-cli-completions"),
    worktree_fixture("fix-3-locked-worktree"),
  ];
  app.filter.set_query("tui".into());

  let idx: Vec<usize> = app.filtered_indices().to_vec();
  let names: Vec<&str> = idx.iter().map(|&i| app.worktrees[i].name.as_str()).collect();
  assert_eq!(
    names,
    vec!["feat-1-tui-search"],
    "only the worktree whose name contains 'tui' should match"
  );
}

#[test]
fn filtered_indices_supports_subsequence_match() {
  // The candidate must NOT contain the query as a contiguous substring —
  // otherwise a regression that downgrades the fuzzy matcher to plain
  // substring matching would still pass. Spread the query characters across
  // the haystack so only the subsequence path can score it.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    // 'a'-'u'-'t'-'h' appear in order but separated by other characters;
    // there is no literal `auth` substring anywhere in this name.
    worktree_fixture("a-foo-u-bar-t-baz-h-qux"),
    worktree_fixture("chore-1-bump-deps"),
  ];
  app.filter.set_query("auth".into());
  // Sanity-guard the precondition: if a future refactor introduces an `auth`
  // substring into the fixture, the test would silently degrade to a
  // substring check again.
  assert!(
    !app.worktrees[0].name.contains("auth"),
    "fixture must not contain 'auth' as a substring or the test stops covering subsequence"
  );

  let idx: Vec<usize> = app.filtered_indices().to_vec();
  assert_eq!(idx.len(), 1);
  assert_eq!(app.worktrees[idx[0]].name, "a-foo-u-bar-t-baz-h-qux");
}

#[test]
fn filtered_indices_ranks_substring_above_subsequence() {
  // Issue #21 contract: "exact substring match > prefix match > sub-sequence
  // match". Verify by feeding a candidate that contains the literal needle
  // and another that only matches via gaps; the contiguous one must rank
  // first in the returned index list.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    // subsequence: a-u-t-h spread out across the name
    worktree_fixture("a-zzz-u-yyy-t-xxx-h"),
    // direct substring of "auth"
    worktree_fixture("auth-service"),
  ];
  app.filter.set_query("auth".into());

  let idx: Vec<usize> = app.filtered_indices().to_vec();
  assert!(!idx.is_empty(), "at least the substring candidate must match");
  assert_eq!(
    app.worktrees[idx[0]].name, "auth-service",
    "contiguous substring must outrank a spread subsequence"
  );
}

#[test]
fn filtered_indices_skips_when_no_match() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("alpha"), worktree_fixture("beta")];
  app.filter.set_query("zzzz".into());
  assert!(app.filtered_indices().is_empty());
}

#[test]
fn selected_returns_filtered_worktree() {
  // `selected()` must resolve the table state's index through the filter map
  // back to the underlying worktree, not blindly index into `worktrees`.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("authentication"),
    worktree_fixture("beta"),
  ];
  app.filter.set_query("auth".into());
  app.list_state.select(Some(0));

  let sel = app.selected().expect("filtered selection must resolve");
  assert_eq!(sel.name, "authentication");
}

#[test]
fn selected_returns_none_when_filter_matches_nothing() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("alpha"), worktree_fixture("beta")];
  app.filter.set_query("zzzz".into());
  app.list_state.select(Some(0));
  assert!(app.selected().is_none());
}

#[test]
fn next_navigates_within_filtered_subset_and_wraps() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("foo-a"),
    worktree_fixture("foo-b"),
  ];
  app.filter.set_query("foo".into());
  app.list_state.select(Some(0));

  app.next();
  assert_eq!(app.list_state.selected(), Some(1));
  app.next();
  assert_eq!(
    app.list_state.selected(),
    Some(0),
    "wrap-around to start of filtered subset"
  );
}

#[test]
fn prev_navigates_within_filtered_subset_and_wraps() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("foo-a"),
    worktree_fixture("foo-b"),
  ];
  app.filter.set_query("foo".into());
  app.list_state.select(Some(0));

  app.prev();
  assert_eq!(app.list_state.selected(), Some(1), "wrap-around backwards");
}

#[test]
fn first_and_last_jump_inside_filtered_subset() {
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("foo-1"),
    worktree_fixture("beta"),
    worktree_fixture("foo-2"),
    worktree_fixture("gamma"),
  ];
  app.filter.set_query("foo".into());
  app.list_state.select(Some(1));

  app.first();
  assert_eq!(app.list_state.selected(), Some(0));
  app.last();
  assert_eq!(
    app.list_state.selected(),
    Some(1),
    "last must use the filtered length (2 matches → index 1)"
  );
}

#[test]
fn filter_push_clamps_selection_when_subset_shrinks() {
  // User starts on the second match, then types more so only the first stays;
  // selection must clamp instead of dangling past the new end.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("foo-bar"), worktree_fixture("foo-baz-xx")];
  app.filter.set_query("foo".into());
  app.list_state.select(Some(1));

  // Typing more reduces the match set to just "foo-bar".
  for c in "-bar".chars() {
    app.filter_push_char(c);
  }
  let filtered: Vec<usize> = app.filtered_indices().to_vec();
  assert_eq!(filtered.len(), 1, "only foo-bar should still match foo-bar");
  assert_eq!(
    app.list_state.selected(),
    Some(0),
    "selection must clamp to inside the new filtered subset"
  );
}

#[test]
fn exit_filter_cancel_restores_full_list_selection() {
  // After clearing the filter, the original full list comes back and a
  // selection past the filtered len must remain valid for the larger set.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),
    worktree_fixture("foo"),
    worktree_fixture("beta"),
  ];
  app.filter.set_query("foo".into());
  app.list_state.select(Some(0));

  app.exit_filter_cancel();
  assert!(app.filter.query().is_empty());
  assert_eq!(app.filtered_indices(), vec![0, 1, 2]);
  // Selection from the filtered view (index 0) remains a valid index in the
  // full list (index 0). The clamp logic doesn't move it forward, only back.
  assert_eq!(app.list_state.selected(), Some(0));
}

// ---- picker mode (issue #22) --------------------------------------------

#[test]
fn picker_mode_defaults_to_false() {
  // The regular `gwm` TUI entry point must not behave like a picker. The
  // create / delete / bootstrap actions stay reachable, and `picker_result`
  // is unset until an explicit picker session asks for it.
  let (_dir, app) = make_app();
  assert!(!app.picker_mode, "default App must not be in picker mode");
  assert!(
    app.picker_result.is_none(),
    "no path is selected until the user confirms"
  );
}

#[test]
fn new_picker_at_enables_picker_mode() {
  // `gwm switch` enters the TUI through this constructor; the picker flag
  // is what drives the event loop into "Enter confirms, n/d/b are inert".
  let (dir, _) = init_repo();
  let app = App::new_picker_at_layered(Some(dir.path()), None).unwrap();
  assert!(app.picker_mode, "new_picker_at must set picker_mode=true");
}

#[test]
fn new_picker_at_opens_filter_bar() {
  // Per issue #22: "switch could open the filter bar immediately on
  // startup". A user invoking `gwm switch` already knows they want to
  // narrow the list; opening the bar saves one keystroke.
  let (dir, _) = init_repo();
  let app = App::new_picker_at_layered(Some(dir.path()), None).unwrap();
  assert!(app.filter.active, "picker mode must open with the filter bar active");
}

#[test]
fn picker_confirm_records_selected_path() {
  // Enter in picker mode commits the highlighted worktree path so the
  // event loop can return it to the caller (which prints it to stdout
  // for the `cd "$(gwm switch)"` flow).
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.list_state.select(Some(0));
  let expected = app.selected().expect("test fixture must have a worktree").path.clone();

  app.picker_confirm();
  assert_eq!(
    app.picker_result,
    Some(expected),
    "picker_confirm must record the selected worktree's path"
  );
}

#[test]
fn picker_confirm_with_no_selection_keeps_result_none() {
  // If the filter wipes the list down to zero matches, hitting Enter must
  // not crash and must not record a bogus path. The event loop is then
  // free to keep the TUI open (or break with None, which is the caller's
  // call).
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.worktrees.clear();
  app.list_state.select(None);

  app.picker_confirm();
  assert!(
    app.picker_result.is_none(),
    "picker_confirm with no selection must leave picker_result unset"
  );
}

#[test]
fn picker_confirm_outside_picker_mode_is_inert() {
  // Defensive: `picker_confirm` shouldn't poison the regular TUI flow if
  // it's ever wired into the wrong event branch. Only picker mode reacts
  // to Enter by recording a path; the normal `Enter = copy path to status
  // bar` behaviour is left to its own handler.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  assert!(!app.picker_mode);

  app.picker_confirm();
  assert!(
    app.picker_result.is_none(),
    "picker_confirm outside picker mode must not record a path"
  );
}

// Copilot review (PR #53): the event loop unconditionally `break`s after
// `picker_confirm()` in picker mode, even when no worktree is selected.
// That turns Enter-on-an-empty-filter-result into an exit-1 surprise.
// The fix surfaces a `picker_should_exit` flag so the loop can keep
// running until the user actually picks something.

#[test]
fn picker_should_exit_defaults_false() {
  let (_dir, app) = make_app();
  assert!(!app.picker_should_exit, "newly-built App must not signal a picker exit");
}

#[test]
fn picker_confirm_with_selection_signals_exit() {
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.list_state.select(Some(0));
  app.picker_confirm();
  assert!(
    app.picker_should_exit,
    "successful picker_confirm must signal the event loop to exit"
  );
  assert!(app.picker_result.is_some());
}

#[test]
fn picker_confirm_without_selection_does_not_signal_exit() {
  // regression: PR #53 Copilot review — picker_confirm with an empty filter
  // result unconditionally broke the event loop and exited 1.
  // When the fuzzy filter narrows the list down to zero matches, Enter
  // must keep the TUI open so the user can back-space and try again,
  // not exit with code 1.
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.worktrees.clear();
  app.list_state.select(None);

  app.picker_confirm();
  assert!(
    !app.picker_should_exit,
    "picker_confirm with no selection must NOT signal exit"
  );
  assert!(app.picker_result.is_none());
}

#[test]
fn picker_confirm_without_selection_reports_status() {
  // regression: PR #53 — Enter on an empty filter result was silently
  // swallowed; no status-bar hint surfaced the no-match state.
  // The user needs feedback explaining why Enter was inert. Surface a
  // status-bar hint so the no-match case isn't silently swallowed.
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.worktrees.clear();
  app.list_state.select(None);

  app.picker_confirm();
  assert!(
    app.status.to_lowercase().contains("no") || app.status.to_lowercase().contains("nothing"),
    "picker_confirm with no selection must update the status bar (got: {:?})",
    app.status
  );
}

// Copilot review (PR #53): in picker mode, `Esc` during an active filter
// only clears the filter — but the picker footer reads `esc:cancel`, so
// the documented contract is "Esc cancels the picker". Add an explicit
// `picker_cancel` that signals exit without recording a path so the
// event loop can route filter-mode Esc to the picker contract.

#[test]
fn picker_cancel_signals_exit_without_path() {
  // regression: PR #53 — picker footer reads `esc:cancel` but Esc-in-filter
  // only cleared the query; the picker contract wasn't honored.
  let (_dir, mut app) = make_app();
  app.picker_mode = true;
  app.list_state.select(Some(0));

  app.picker_cancel();
  assert!(
    app.picker_should_exit,
    "picker_cancel must signal the event loop to exit"
  );
  assert!(
    app.picker_result.is_none(),
    "picker_cancel must NOT record a path (Esc is the no-pick exit)"
  );
}

#[test]
fn picker_cancel_outside_picker_mode_is_inert() {
  // Defensive: like `picker_confirm`, `picker_cancel` should be a no-op
  // when wired into the wrong branch — the regular TUI's Esc semantics
  // are handled elsewhere.
  let (_dir, mut app) = make_app();
  assert!(!app.picker_mode);

  app.picker_cancel();
  assert!(
    !app.picker_should_exit,
    "picker_cancel outside picker mode must not flip the exit flag"
  );
}

// ---- Confirm countdown state machine (issue #30) -------------------------
//
// The countdown only arms when `delete_branch_on_remove` is ON AND the
// configured `confirm_countdown_secs` is non-zero. In every other case
// (off, or countdown_secs = 0) the modal stays single-keystroke. These
// tests pin every branch of that dispatch so a regression on either knob
// fails loudly.
//
// Time injection: `confirm_press_y`, `tick_confirm_countdown`, and the
// progress / remaining getters all take an `Instant` parameter so the
// tests can step through the countdown without sleeping. The event loop
// passes `Instant::now()` at the real call sites.

#[test]
fn countdown_total_zero_when_delete_branch_off() {
  let (_dir, app) = make_app();
  assert!(!app.delete_branch_on_remove);
  assert_eq!(app.confirm_countdown_total(), Duration::ZERO);
  assert!(!app.confirm_is_countdown_mode());
}

#[test]
fn countdown_total_matches_config_when_delete_branch_on() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  assert!(app.delete_branch_on_remove);
  assert_eq!(app.confirm_countdown_total(), Duration::from_secs(3));
  assert!(app.confirm_is_countdown_mode());
}

#[test]
fn countdown_total_zero_when_config_says_zero() {
  let (_dir, mut app) = make_app();
  app.config.tui.confirm_countdown_secs = 0;
  app.toggle_delete_branch();
  assert_eq!(app.confirm_countdown_total(), Duration::ZERO);
  assert!(
    !app.confirm_is_countdown_mode(),
    "countdown_secs=0 must fall back to the classic modal even when delete_branch is armed"
  );
}

#[test]
fn confirm_press_y_in_classic_mode_fires_immediately() {
  let (_dir, mut app) = make_app();
  // delete_branch OFF → classic flow
  let action = app.confirm_press_y(Instant::now());
  assert_eq!(action, ConfirmKeyAction::FireNow);
  assert!(!app.confirm.is_armed(), "classic mode must never set the timer");
}

#[test]
fn confirm_press_y_in_countdown_mode_arms_the_timer() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let now = Instant::now();
  let action = app.confirm_press_y(now);
  assert_eq!(action, ConfirmKeyAction::Armed);
  assert!(app.confirm.is_armed());
  // Anchor is exactly `now`: zero elapsed means progress is 0.0. This
  // pins the anchor through the public API instead of reading the
  // (now private since #131) raw `started_at` field.
  assert_eq!(app.confirm.progress(now, app.confirm_countdown_total()), 0.0);
}

#[test]
fn confirm_press_y_a_second_time_disarms_the_timer() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  let t1 = t0 + Duration::from_millis(500);
  let action = app.confirm_press_y(t1);
  assert_eq!(action, ConfirmKeyAction::Disarmed);
  assert!(!app.confirm.is_armed());
}

#[test]
fn countdown_status_uses_rebound_confirm_keys() {
  // #219 review (P3): the armed/disarmed countdown status copy hard-coded
  // `y` / `Esc`. When the confirm context's confirm/cancel verbs are rebound,
  // the instructions must name the live keys, not the defaults.
  use gwm::tui::modal_keymap::{parse_single, ModalAction};
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(ModalAction::ConfirmConfirm, vec![parse_single("c").unwrap()])
    .unwrap();
  app
    .modal_keymap
    .apply_override(ModalAction::ConfirmCancel, vec![parse_single("x").unwrap()])
    .unwrap();
  app.toggle_delete_branch();

  let t0 = Instant::now();
  app.confirm_press_y(t0); // arms
  assert!(
    app.status.contains("press c again or x to cancel"),
    "armed copy must use the rebound confirm/cancel keys: {}",
    app.status
  );

  let action = app.confirm_press_y(t0 + Duration::from_millis(500)); // disarms
  assert_eq!(action, ConfirmKeyAction::Disarmed);
  assert!(
    app.status.contains("press c to re-arm"),
    "disarmed copy must use the rebound confirm key: {}",
    app.status
  );
}

#[test]
fn countdown_status_omits_an_unbound_cancel_key() {
  // #219 review (P2): with `[tui.keys.modal.confirm] cancel = []`, Esc no
  // longer cancels — the armed status must not advertise a phantom cancel key
  // (it would tell the user to press a key that does nothing while the delete
  // timer runs). Drop it instead of falling back to the literal.
  use gwm::tui::modal_keymap::ModalAction;
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(ModalAction::ConfirmCancel, vec![])
    .unwrap();
  app.toggle_delete_branch();
  app.confirm_press_y(Instant::now()); // arms
  assert!(
    !app.status.contains("Esc") && !app.status.contains("to cancel"),
    "armed status must not advertise an unbound cancel key: {}",
    app.status
  );
  assert!(
    app.status.contains("press y again"),
    "the still-bound confirm key must remain in the copy: {}",
    app.status
  );
}

#[test]
fn confirm_dismiss_resets_timer_and_returns_to_list() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  app.view = View::Confirm;
  app.confirm_press_y(Instant::now());
  assert!(app.confirm.is_armed());
  app.confirm_dismiss();
  assert_eq!(app.view, View::List);
  assert!(!app.confirm.is_armed(), "Esc/n must always disarm the countdown");
}

#[test]
fn tick_unarmed_is_noop() {
  let (_dir, mut app) = make_app();
  let outcome = app.tick_confirm_countdown(Instant::now());
  assert_eq!(outcome, CountdownTickOutcome::NotArmed);
}

#[test]
fn tick_before_duration_is_pending() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  let outcome = app.tick_confirm_countdown(t0 + Duration::from_millis(1500));
  assert_eq!(outcome, CountdownTickOutcome::Pending);
  assert!(app.confirm.is_armed(), "pending tick must not clear the timer");
}

#[test]
fn tick_at_duration_signals_ready_to_fire() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  let outcome = app.tick_confirm_countdown(t0 + Duration::from_secs(3));
  assert_eq!(outcome, CountdownTickOutcome::ReadyToFire);
}

#[test]
fn tick_past_duration_signals_ready_to_fire() {
  // Tick rate is 200ms (mod.rs), so the elapsed value handed to the App
  // can overshoot the configured duration slightly. The state machine
  // must still report ReadyToFire, not Pending.
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  let outcome = app.tick_confirm_countdown(t0 + Duration::from_millis(3500));
  assert_eq!(outcome, CountdownTickOutcome::ReadyToFire);
}

#[test]
fn countdown_progress_grows_with_elapsed() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  assert!((app.confirm_countdown_progress(t0) - 0.0).abs() < 1e-9);
  let mid = app.confirm_countdown_progress(t0 + Duration::from_millis(1500));
  assert!((0.49..=0.51).contains(&mid), "got progress = {mid}");
  let done = app.confirm_countdown_progress(t0 + Duration::from_secs(10));
  assert!((done - 1.0).abs() < 1e-9, "progress clamps to 1.0; got {done}");
}

#[test]
fn countdown_remaining_secs_counts_down_to_zero() {
  let (_dir, mut app) = make_app();
  app.toggle_delete_branch();
  let t0 = Instant::now();
  app.confirm_press_y(t0);
  assert_eq!(app.confirm_countdown_remaining_secs(t0), 3);
  // Anywhere strictly inside [2s, 3s) still rounds up to "1s left".
  assert_eq!(
    app.confirm_countdown_remaining_secs(t0 + Duration::from_millis(2500)),
    1
  );
  assert_eq!(app.confirm_countdown_remaining_secs(t0 + Duration::from_secs(3)), 0);
}

// Regression for Copilot review on PR #66: the gauge previously used
// `round()`, which rendered a fully-filled 10-cell bar at progress 0.95
// even though `confirm_delete` only fires at progress 1.0. The user saw
// "bar full" but the action hadn't happened yet — a false-positive
// signal on the destructive path. The fix: floor the cell count and
// keep the last cell empty until progress actually hits 1.0.

#[test]
fn filled_cells_zero_at_progress_zero() {
  assert_eq!(filled_cells_for_progress(0.0, 10), 0);
}

#[test]
fn filled_cells_full_only_at_progress_one() {
  assert_eq!(filled_cells_for_progress(1.0, 10), 10);
}

#[test]
fn filled_cells_below_one_keeps_last_cell_empty() {
  // 0.95 used to round() up to 10 cells (full bar) — that's the bug.
  // It must now floor to 9 (or less), reserving the last cell for the
  // "action fires" moment.
  assert_eq!(filled_cells_for_progress(0.95, 10), 9);
  // 0.99 even closer to full — still must not paint the last cell.
  assert!(filled_cells_for_progress(0.99, 10) < 10);
  // 0.999_999 same story — float weirdness must not flip the last cell.
  assert!(filled_cells_for_progress(0.999_999, 10) < 10);
}

#[test]
fn filled_cells_clamps_above_one() {
  // Float drift on an overshooting tick (200ms poll past N×1000ms) can
  // hand a progress > 1.0; the bar must clamp at the cell count.
  assert_eq!(filled_cells_for_progress(1.5, 10), 10);
}

#[test]
fn filled_cells_floors_partial_progress() {
  // 0.55 with 10 cells → 5.5 floor → 5 (not 6 from rounding).
  assert_eq!(filled_cells_for_progress(0.55, 10), 5);
  // 0.5 exact → 5 cells.
  assert_eq!(filled_cells_for_progress(0.5, 10), 5);
}

// ---- Issue / PR linking (issue #67) -------------------------------------

use gwm::github::{CiState, IssueState, IssueStatus, LinkSource, PrState, PrStatus};
use gwm::tui::{GitHubFetchState, LinkPromptStage, LinkTarget};

fn make_app_on_branch(name: &str) -> (tempfile::TempDir, git2::Repository, App) {
  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch(name, &head, false).unwrap();
  }
  repo.set_head(&format!("refs/heads/{}", name)).unwrap();
  let app = App::new_at_layered(Some(dir.path()), None).unwrap();
  (dir, repo, app)
}

#[test]
fn current_link_reflects_branch_name_auto_detect() {
  let (_dir, _repo, app) = make_app_on_branch("feat/#42-tui-search");
  let link = app.current_link();
  assert_eq!(link.issue, Some(42));
  assert_eq!(link.issue_source, LinkSource::BranchName);
  assert_eq!(link.pr, None);
}

#[test]
fn enter_open_menu_transitions_view() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.enter_open_menu();
  assert_eq!(app.view, View::OpenMenu);
}

#[test]
fn open_menu_selection_toggles_like_link_prompt() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.enter_open_menu();
  assert_eq!(app.open_menu_selected, LinkTarget::Issue);
  app.open_menu_toggle_selection();
  assert_eq!(app.open_menu_selected, LinkTarget::Pr);
  app.open_menu_toggle_selection();
  assert_eq!(app.open_menu_selected, LinkTarget::Issue);
}

#[test]
fn open_menu_choose_issue_returns_url_when_linked_and_slug_available() {
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  app.enter_open_menu();
  let url = app.open_menu_pick(LinkTarget::Issue).unwrap();
  assert_eq!(url, "https://github.com/kbrdn1/gwm-cli/issues/42");
  // After picking, the view is back to the list.
  assert_eq!(app.view, View::List);
}

#[test]
fn open_menu_pick_returns_none_when_no_link() {
  let (_dir, repo, mut app) = make_app_on_branch("random-branch");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  app.enter_open_menu();
  let url = app.open_menu_pick(LinkTarget::Pr);
  assert!(url.is_none());
  // Status bar should hint why.
  assert!(
    app.status.to_lowercase().contains("no pr"),
    "status should mention missing PR link: {}",
    app.status
  );
  // Codex review on PR #292 (P3): the "press X to link" hint must point at
  // LinkPrompt's real binding (`i` since #290), not the stale `L` (now
  // LazyGitFullscreen).
  assert!(
    app.status.contains("press i to link"),
    "status must use LinkPrompt's real chord, not the stale `L`: {}",
    app.status
  );
}

#[test]
fn link_open_modal_lines_include_available_links_without_refresh_button() {
  use gwm::tui::{link_open_modal_lines, LinkTarget};
  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("random-branch", &head, false).unwrap();
  }
  repo.set_head("refs/heads/random-branch").unwrap();
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.random-branch.gwm-issue", "42").unwrap();
    cfg.set_str("branch.random-branch.gwm-pr", "7").unwrap();
  }
  let app = App::new_at_layered(Some(dir.path()), None).unwrap();

  let text = link_open_modal_lines(&app, "Open in Browser", Some(LinkTarget::Issue))
    .into_iter()
    .map(|line| spans_to_text(&line.spans))
    .collect::<Vec<_>>()
    .join("\n");

  assert!(text.contains("Issue #42"), "Issue summary missing: {text:?}");
  assert!(text.contains("PR"), "PR summary missing: {text:?}");
  assert!(text.contains("#7"), "PR number missing: {text:?}");
  assert!(
    !text.contains("Refresh"),
    "refresh should be advertised in the hint row, not as a third action button: {text:?}"
  );
}

#[test]
fn enter_link_prompt_starts_at_choose_target() {
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  assert_eq!(app.view, View::LinkPrompt);
  assert_eq!(app.link_prompt_stage(), LinkPromptStage::ChooseTarget);
  assert!(app.link_prompt_number_input().is_empty());
}

#[test]
fn link_prompt_status_copy_stays_footer_sized() {
  // The statusbar pins `app.status` at the right edge. Long modal-control
  // prose gets clipped at 80 columns, so Link prompt status copy should stay
  // short; the modal itself owns the detailed key hints.
  const MAX_STATUS_CHARS: usize = 4;
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");

  app.enter_link_prompt();
  assert!(
    app.status.chars().count() <= MAX_STATUS_CHARS,
    "choose-target status is too long for the footer: {:?}",
    app.status
  );

  app.link_prompt_choose(LinkTarget::Issue);
  assert!(
    app.status.chars().count() <= MAX_STATUS_CHARS,
    "issue-input status is too long for the footer: {:?}",
    app.status
  );

  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Pr);
  assert!(
    app.status.chars().count() <= MAX_STATUS_CHARS,
    "pr-input status is too long for the footer: {:?}",
    app.status
  );
}

#[test]
fn link_prompt_choose_issue_advances_to_input() {
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  assert_eq!(app.link_prompt_stage(), LinkPromptStage::InputNumber);
}

#[test]
fn link_prompt_only_accepts_digits() {
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  for c in "12a3".chars() {
    app.link_prompt_push_char(c);
  }
  assert_eq!(app.link_prompt_number_input(), "123");
}

#[test]
fn link_prompt_submit_writes_branch_config() {
  let (_dir, repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  for c in "42".chars() {
    app.link_prompt_push_char(c);
  }
  app.link_prompt_submit().unwrap();
  // After submit, view returns to list and the link is persisted.
  assert_eq!(app.view, View::List);
  let cfg = repo.config().unwrap();
  let v = cfg.get_string("branch.random-branch.gwm-issue").unwrap();
  assert_eq!(v, "42");
}

#[test]
fn link_prompt_cancel_returns_to_list() {
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.link_prompt_cancel();
  assert_eq!(app.view, View::List);
}

#[test]
fn enter_link_prompt_opens_with_issue_highlighted() {
  // Issue #217: ChooseTarget is a vertical selectable list that opens
  // highlighting Issue (the common case).
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  assert_eq!(app.link_prompt_selected(), LinkTarget::Issue);
}

#[test]
fn link_prompt_key_jk_moves_the_highlight_without_committing() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::LinkPromptKey;
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  assert!(matches!(
    app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
    LinkPromptKey::Handled
  ));
  assert_eq!(app.link_prompt_selected(), LinkTarget::Pr, "j moves the highlight down");
  assert_eq!(
    app.link_prompt_stage(),
    LinkPromptStage::ChooseTarget,
    "moving commits nothing"
  );
  app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
  assert_eq!(app.link_prompt_selected(), LinkTarget::Issue, "k moves it back");
}

#[test]
fn link_prompt_key_enter_links_the_highlighted_target() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::LinkPromptKey;
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)); // highlight Pr
  assert!(matches!(
    app.handle_link_prompt_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    LinkPromptKey::Handled
  ));
  assert_eq!(
    app.link_prompt_stage(),
    LinkPromptStage::InputNumber,
    "Enter commits + advances"
  );
  assert_eq!(
    app.link_prompt_target(),
    Some(LinkTarget::Pr),
    "it links the highlighted row"
  );
}

#[test]
fn link_prompt_key_i_and_p_remain_direct_picks() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
  assert_eq!(app.link_prompt_stage(), LinkPromptStage::InputNumber);
  assert_eq!(app.link_prompt_target(), Some(LinkTarget::Pr), "p picks PR directly");

  app.enter_link_prompt(); // reset
  app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
  assert_eq!(
    app.link_prompt_target(),
    Some(LinkTarget::Issue),
    "i picks Issue directly"
  );
}

#[test]
fn link_prompt_key_digits_then_enter_requests_submit() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::LinkPromptKey;
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  app.handle_link_prompt_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)); // link highlighted Issue
  for c in "4a2".chars() {
    app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
  }
  assert_eq!(
    app.link_prompt_number_input(),
    "42",
    "non-digits dropped during InputNumber"
  );
  assert!(matches!(
    app.handle_link_prompt_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    LinkPromptKey::Submit
  ));
}

#[test]
fn link_prompt_key_esc_requests_cancel() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::LinkPromptKey;
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  assert!(matches!(
    app.handle_link_prompt_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    LinkPromptKey::Cancel
  ));
}

#[test]
fn link_prompt_key_fetch_requests_refresh() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::LinkPromptKey;
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  assert!(matches!(
    app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::NONE)),
    LinkPromptKey::Refresh
  ));
}

#[test]
fn github_fetch_state_default_is_idle() {
  let (_dir, _repo, app) = make_app_on_branch("feat/#42-tui-search");
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn apply_fetch_results_loads_issue_and_pr_state() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let issue = IssueStatus {
    number: 42,
    title: "TUI search".into(),
    state: IssueState::Open,
    url: "https://example.test".into(),
    labels: vec!["feature".into()],
    updated_at: "2026-05-19T00:00:00Z".into(),
  };
  let pr = PrStatus {
    number: 61,
    title: "feat(tui): search".into(),
    state: PrState::Draft,
    url: "https://example.test/pr".into(),
    updated_at: "2026-05-19T00:00:00Z".into(),
    checks_passed: 2,
    checks_total: 3,
    ci: CiState::Running,
    checks: vec![],
  };
  app.apply_issue_fetch_result(Ok(issue.clone()));
  app.apply_pr_fetch_result(Ok(pr.clone()));
  // Post-#138 the cache is keyed by number; the App-level
  // `*_fetch_state()` wrappers resolve via the current link, so they
  // surface the linked issue (42 from the branch name) here. The PR
  // (#61) isn't linked on this branch — read it via the keyed
  // accessor on the underlying `GitHubFetch` directly.
  match app.issue_fetch_state() {
    GitHubFetchState::Loaded(_) => {}
    other => panic!("expected Loaded for linked issue 42, got {:?}", other),
  }
  match app.github.pr_fetch_state(61) {
    GitHubFetchState::Loaded(_) => {}
    other => panic!("expected Loaded for stamped pr 61, got {:?}", other),
  }
}

#[test]
fn loaded_issue_status_persists_title_for_no_fetch_startup() {
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.apply_issue_fetch_result(Ok(IssueStatus {
    number: 42,
    title: "Persisted issue title".into(),
    state: IssueState::Open,
    url: "https://example.test/issues/42".into(),
    labels: vec![],
    updated_at: String::new(),
  }));

  let link = gwm::github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.issue, Some(42));
  assert_eq!(link.issue_title.as_deref(), Some("Persisted issue title"));
}

#[test]
fn enter_ci_checks_opens_the_overlay_with_one_row_per_check() {
  // Issue #436: the CI checks overlay lists every statusCheckRollup entry of
  // the linked PR — one row per check, order preserved, the details URL kept
  // as the row meta so Enter can open it in the browser.
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 1,
    checks_total: 2,
    ci: CiState::Failing,
    checks: vec![
      PrCheck {
        name: "test (ubuntu-latest)".into(),
        outcome: CheckOutcome::Failing,
        url: Some("https://example.test/actions/runs/1/job/2".into()),
        workflow_name: None,
        started_at: None,
        completed_at: None,
      },
      PrCheck {
        name: "rustfmt".into(),
        outcome: CheckOutcome::Passing,
        url: None,
        workflow_name: None,
        started_at: None,
        completed_at: None,
      },
    ],
  }));

  app.enter_ci_checks();

  assert_eq!(app.view, View::DetailOverlay);
  assert_eq!(app.detail_overlay.kind, DetailKind::CiChecks);
  assert_eq!(app.detail_overlay.rows.len(), 2);
  assert_eq!(app.detail_overlay.rows[0].value, "test (ubuntu-latest)");
  assert_eq!(
    app.detail_overlay.rows[0].meta.as_deref(),
    Some("https://example.test/actions/runs/1/job/2")
  );
  assert_eq!(app.detail_overlay.rows[1].value, "rustfmt");
  assert_eq!(app.detail_overlay.rows[1].meta, None);
}

#[test]
fn pr_summary_line_advertises_the_ci_checks_key_after_the_indicator() {
  // #436 validation feedback: the PR line's CI indicator ends with the
  // resolved key that opens the checks overlay — `… CI passing 10/10 [c]` —
  // mirroring the pane titles' `[F]` / `[a]` convention. No indicator
  // (CiState::None) → no hint.
  let mk = |ci: gwm::github::CiState, passed: u32, total: u32| gwm::github::PrStatus {
    number: 9,
    title: "x".into(),
    state: gwm::github::PrState::Open,
    url: String::new(),
    checks_passed: passed,
    checks_total: total,
    ci,
    checks: vec![],
    updated_at: String::new(),
  };
  let theme = Theme::default();
  let line = pr_summary_line(
    9,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Loaded(mk(gwm::github::CiState::Passing, 10, 10)),
    80,
    &theme,
    Some("c"),
  );
  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect::<String>();
  assert!(
    text.contains("10/10 [c]"),
    "the CI trailing must end with the key hint: {text}"
  );

  let line = pr_summary_line(
    9,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Loaded(mk(gwm::github::CiState::None, 0, 0)),
    80,
    &theme,
    Some("c"),
  );
  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect::<String>();
  assert!(!text.contains("[c]"), "no CI indicator, no hint: {text}");
}

#[test]
fn pr_line_ci_hint_follows_the_focus_context() {
  // Codex review on PR #455: with the worktrees pane focused, `c` opens the
  // rename modal — advertising it next to the CI indicator was a lie. The
  // hint resolves dynamically: the contextual `c` (EditWorktree's chord)
  // while the status pane holds the focus, the global `ci_checks` binding
  // (`C`) otherwise.
  use gwm::github::{CheckOutcome, PrCheck};
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 10,
    checks_total: 10,
    ci: CiState::Passing,
    checks: vec![PrCheck {
      name: "ci".into(),
      outcome: CheckOutcome::Passing,
      url: None,
      workflow_name: None,
      started_at: None,
      completed_at: None,
    }],
  }));

  let text_of = |app: &App| -> String {
    gwm::tui::github_status_lines(app, 120)
      .iter()
      .flat_map(|l| l.spans.iter())
      .map(|s| s.content.as_ref())
      .collect()
  };

  app.focus_worktrees();
  let unfocused = text_of(&app);
  assert!(
    unfocused.contains("10/10 [C]"),
    "worktrees focus advertises the global ci_checks binding: {unfocused}"
  );

  app.focus_status();
  let focused = text_of(&app);
  assert!(
    focused.contains("10/10 [c]"),
    "status focus advertises the contextual c: {focused}"
  );

  // Codex review #455 (P2): with `edit_worktree` explicitly unbound the
  // contextual key is gone, but the global `ci_checks` binding still opens
  // the overlay — advertise it instead of dropping the hint entirely.
  app
    .keymap
    .apply_override(gwm::tui::keymap::Action::EditWorktree, vec![])
    .unwrap();
  let unbound = text_of(&app);
  assert!(
    unbound.contains("10/10 [C]"),
    "unbound edit_worktree falls back to the global ci_checks key: {unbound}"
  );
}

#[test]
fn terminal_check_without_completion_shows_no_duration() {
  // Codex review #455 (P2): "in flight" was decided on a missing
  // completed_at, so a TERMINAL StatusContext carrying a start but no end
  // read as still active ("2m…") — and froze there, since the duration
  // tick only rebuilds while an outcome is Running. The elapsed form now
  // requires the Running outcome; a terminal check with an unknown end
  // shows no duration at all, just its workflow.
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::ci_check_rows;
  let now: std::time::SystemTime = chrono::DateTime::parse_from_rfc3339("2026-07-24T14:53:06Z")
    .unwrap()
    .into();
  let checks = vec![PrCheck {
    name: "status-ctx".into(),
    outcome: CheckOutcome::Passing,
    url: None,
    workflow_name: Some("external".into()),
    started_at: Some("2026-07-24T14:51:06Z".into()),
    completed_at: None,
  }];
  let rows = ci_check_rows(&checks, now);
  assert_eq!(
    rows[0].extra.as_deref(),
    Some("external"),
    "a terminal check with no completion timestamp shows no duration"
  );
}

#[test]
fn ci_check_rows_carry_workflow_and_duration_details() {
  // #436 validation feedback: each row surfaces the workflow name and the
  // run duration in `extra` — completed runs show the exact span, running
  // ones the elapsed time with an ellipsis, a legacy StatusContext (no
  // metadata) none at all.
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::ci_check_rows;
  let now: std::time::SystemTime = chrono::DateTime::parse_from_rfc3339("2026-07-24T14:53:06Z")
    .unwrap()
    .into();
  let checks = vec![
    PrCheck {
      name: "test".into(),
      outcome: CheckOutcome::Passing,
      url: None,
      workflow_name: Some("ci".into()),
      started_at: Some("2026-07-24T14:51:06Z".into()),
      completed_at: Some("2026-07-24T14:52:24Z".into()),
    },
    PrCheck {
      name: "fmt".into(),
      outcome: CheckOutcome::Running,
      url: None,
      workflow_name: Some("ci".into()),
      started_at: Some("2026-07-24T14:51:06Z".into()),
      completed_at: None,
    },
    PrCheck {
      name: "security/scan".into(),
      outcome: CheckOutcome::Failing,
      url: None,
      workflow_name: None,
      started_at: None,
      completed_at: None,
    },
  ];
  let rows = ci_check_rows(&checks, now);
  assert_eq!(rows[0].extra.as_deref(), Some("ci · 1m18s"));
  assert_eq!(rows[1].extra.as_deref(), Some("ci · 2m…"));
  assert_eq!(rows[2].extra, None);
}

#[test]
fn enter_ci_checks_without_checks_stays_on_the_list_with_a_status_hint() {
  // No linked PR (or a PR with an empty rollup): the overlay would be an
  // empty void — surface the situation on the status bar instead.
  let (_dir, mut app) = make_app();
  app.enter_ci_checks();
  assert_ne!(app.view, View::DetailOverlay, "no overlay without checks");
  assert!(
    app.status.contains("CI checks"),
    "status bar must explain why nothing opened: {}",
    app.status
  );
}

#[test]
fn ci_filter_enter_on_a_urlless_check_leaves_the_filter_and_signals_it() {
  // Codex review on PR #455: Enter inside the `f` filter on a check with no
  // details URL silently dropped back to the list — the List-mode Enter
  // path reports "no details URL", the filter path must too. A void query
  // with no match at all keeps the filter open instead.
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::{ci_check_rows, DetailKind, DetailMode};
  let (_dir, mut app) = make_app();
  let checks = vec![PrCheck {
    name: "legacy/scan".into(),
    outcome: CheckOutcome::Failing,
    url: None,
    workflow_name: None,
    started_at: None,
    completed_at: None,
  }];
  let rows = ci_check_rows(&checks, std::time::SystemTime::now());
  app.detail_overlay.open(DetailKind::CiChecks, "CI Checks".into(), rows);
  app.ci_input_open();

  assert_eq!(app.ci_input_selected_url(), None, "no URL on the selected check");
  assert_eq!(
    app.detail_overlay.mode,
    DetailMode::List,
    "a picked row leaves the filter even without a URL"
  );

  app.ci_input_open();
  app.ci_input_push('z');
  assert_eq!(app.ci_input_selected_url(), None);
  assert_eq!(
    app.detail_overlay.mode,
    DetailMode::Input,
    "no match under the query keeps the filter open"
  );
}

#[test]
fn edit_worktree_action_routes_to_ci_checks_when_status_focused() {
  // Issue #436: `c` is contextual, same dispatch mechanism that turns j/k
  // into sidebar scroll — worktrees context keeps the rename modal, status
  // context opens the CI checks overlay.
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: 1,
    ci: CiState::Running,
    checks: vec![PrCheck {
      name: "ci".into(),
      outcome: CheckOutcome::Running,
      url: None,
      workflow_name: None,
      started_at: None,
      completed_at: None,
    }],
  }));

  // Codex review on PR #455: the contextual routing lives on the KEY path
  // only — a pure pre-resolution the event loop applies before run_action.
  // The palette's `edit-worktree` entry must stay a rename everywhere, so
  // accept_command_palette returns the action unresolved.
  use gwm::tui::keymap::Action;
  app.focus_status();
  assert_eq!(
    app.resolve_contextual_action(Action::EditWorktree),
    Action::CiChecks,
    "status focus routes the edit-worktree KEY to the CI overlay"
  );
  assert_eq!(
    app.resolve_contextual_action(Action::Down),
    Action::Down,
    "other actions pass through untouched"
  );

  app.focus_worktrees();
  assert_eq!(
    app.resolve_contextual_action(Action::EditWorktree),
    Action::EditWorktree,
    "worktrees context keeps the rename on c"
  );

  // The resolved CiChecks action opens the overlay as before.
  app.focus_status();
  app.enter_ci_checks();
  assert_eq!(app.view, View::DetailOverlay);
  assert_eq!(app.detail_overlay.kind, DetailKind::CiChecks);
}

#[test]
fn ci_overlay_refreshes_its_rows_when_a_pr_fetch_lands() {
  // Validation feedback on PR #455: `f` inside the overlay re-fetches the
  // PR; the landing must refresh the open CI overlay in place (same
  // convention as the agents landing), keeping the kind and clamping the
  // selection to the new row count. The result is injected through the
  // spine + drain — the path a real background worker takes — not the
  // `apply_pr_fetch_result` seam: the first cut of this feature rebuilt
  // the rows only in the seam, so the pane refreshed but the overlay
  // never did (field-caught on the local install).
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::DetailKind;
  use gwm::tui::TaskMsg;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mk_check = |name: &str, outcome: CheckOutcome| PrCheck {
    name: name.into(),
    outcome,
    url: None,
    workflow_name: None,
    started_at: None,
    completed_at: None,
  };
  let mk_status = |number: u64, checks: Vec<PrCheck>| PrStatus {
    number,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: format!("https://example.test/pull/{}", number),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: checks.len() as u32,
    ci: CiState::Running,
    checks,
  };
  app.apply_pr_fetch_result(Ok(mk_status(
    61,
    vec![
      mk_check("a", CheckOutcome::Running),
      mk_check("b", CheckOutcome::Running),
    ],
  )));

  app.enter_ci_checks();
  app.detail_overlay.select_next();
  assert_eq!(app.detail_overlay.selected, 1);

  // `f` claims a spine slot; the worker's result comes back over the
  // task channel and is applied by the drain.
  let generation = request_github_pr(&mut app, 61);
  app
    .task_result_sender()
    .send(TaskMsg::GithubPr(
      generation,
      61,
      Ok(mk_status(61, vec![mk_check("a", CheckOutcome::Passing)])),
    ))
    .unwrap();
  app.drain_task_results();

  assert_eq!(app.detail_overlay.kind, DetailKind::CiChecks);
  assert_eq!(
    app.detail_overlay.rows.len(),
    1,
    "the drained landing refreshes the rows in place"
  );
  assert_eq!(app.detail_overlay.rows[0].value, "a");
  assert_eq!(app.detail_overlay.selected, 0, "the selection clamps to the new count");

  // A landing for a *different* PR (the worktree-wide bulk prefetch) must
  // not clobber the open overlay's rows.
  let generation = request_github_pr(&mut app, 62);
  app
    .task_result_sender()
    .send(TaskMsg::GithubPr(
      generation,
      62,
      Ok(mk_status(
        62,
        vec![
          mk_check("x", CheckOutcome::Failing),
          mk_check("y", CheckOutcome::Failing),
        ],
      )),
    ))
    .unwrap();
  app.drain_task_results();
  assert_eq!(
    app.detail_overlay.rows.len(),
    1,
    "another PR's landing must not clobber the linked PR's rows"
  );
  assert_eq!(app.detail_overlay.rows[0].value, "a");
}

#[test]
fn ci_overlay_closes_when_a_refresh_lands_an_empty_rollup() {
  // Codex review #455 (P2, twice): a refresh can land an empty rollup — a
  // new commit was just pushed and the workflows have not started yet.
  // Blanking the rows while leaving the overlay open produced exactly the
  // empty overlay `enter_ci_checks` refuses to open; close it and say why.
  // The result goes through the spine + drain (the real worker path): the
  // end-of-drain `report_github_refresh_status` used to overwrite the
  // close message with "github status refreshed", so the status assertion
  // below pins the whole drain, not just the landing helper.
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::TaskMsg;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mk_status = |checks: Vec<PrCheck>| PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: checks.len() as u32,
    ci: CiState::Running,
    checks,
  };
  app.apply_pr_fetch_result(Ok(mk_status(vec![PrCheck {
    name: "a".into(),
    outcome: CheckOutcome::Running,
    url: None,
    workflow_name: None,
    started_at: None,
    completed_at: None,
  }])));
  app.enter_ci_checks();
  assert_eq!(app.view, View::DetailOverlay);

  let generation = request_github_pr(&mut app, 61);
  app
    .task_result_sender()
    .send(TaskMsg::GithubPr(generation, 61, Ok(mk_status(vec![]))))
    .unwrap();
  app.drain_task_results();
  assert_eq!(app.view, View::List, "an empty landing closes the overlay");
  assert!(
    app.status.contains("no CI checks"),
    "the close message survives the end-of-drain refresh report: {}",
    app.status
  );
}

/// An App with a linked PR 61, its fetch landed and the CI overlay open —
/// the shared setup of the two identity-change tests below.
fn app_with_open_ci_overlay_on_pr_61() -> (tempfile::TempDir, git2::Repository, App) {
  use gwm::github::{CheckOutcome, PrCheck};
  let (dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: 1,
    ci: CiState::Running,
    checks: vec![PrCheck {
      name: "a".into(),
      outcome: CheckOutcome::Running,
      url: None,
      workflow_name: None,
      started_at: None,
      completed_at: None,
    }],
  }));
  app.enter_ci_checks();
  assert_eq!(app.view, View::DetailOverlay);
  (dir, repo, app)
}

#[test]
fn refresh_closes_the_ci_overlay_when_the_pr_link_is_gone() {
  // Codex review #455 (P2): with a non-explicit link the refresh re-probes
  // the PR detection, and a persisted detection can come back None —
  // leaving no PR at all. The open overlay then shows checks for a PR the
  // link no longer carries, and with nothing to fetch no landing will ever
  // refresh or close it. The refresh handles the identity change up front.
  let (_dir, _repo, mut app) = app_with_open_ci_overlay_on_pr_61();
  // Simulate the re-probe dropping the detection (the in-refresh gh probe
  // is not reachable from a test): the link no longer carries a PR.
  app.github.link.pr = None;

  app.refresh_github_status();
  assert_eq!(
    app.view,
    View::List,
    "a refresh with no linked PR closes the stale CI overlay"
  );
}

#[test]
fn refresh_closes_the_ci_overlay_when_the_detected_pr_changes() {
  // Codex review #455 (P2, second round): the None-only guard missed the
  // re-detection CHANGING the PR (#61 → #62) — the old PR's checks stayed
  // up during the new fetch, and forever if it failed, with Enter opening
  // a stale check URL. The overlay carries the PR number that opened it,
  // and a refresh whose link disagrees closes it up front.
  let (_dir, _repo, mut app) = app_with_open_ci_overlay_on_pr_61();
  // Simulate the re-probe re-detecting a different PR.
  app.github.link.pr = Some(62);

  app.refresh_github_status();
  assert_eq!(
    app.view,
    View::List,
    "a refresh onto a different PR closes the stale CI overlay"
  );
}

#[test]
fn link_refresh_closes_the_ci_overlay_when_the_link_moves() {
  // Codex review #455 (P2, third round): `refresh_github_status` is not
  // the only path that mutates the link — `refresh_link` runs on
  // navigation and on the auto-refresh relist (which can move the
  // selection when the current worktree disappears), all while the
  // overlay is up. The identity guard fires there too.
  let (_dir, repo, mut app) = app_with_open_ci_overlay_on_pr_61();
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 62).unwrap();

  app.refresh_link();
  assert_eq!(
    app.view,
    View::List,
    "a link refresh onto a different PR closes the stale CI overlay"
  );
}

#[test]
fn ci_checks_refuse_to_open_on_a_stale_workspace_selection() {
  // Codex review #455 (P2): in workspace mode a failed `Repository::open`
  // for the selected row leaves `github.link` and its cache on the
  // previously active repo. Opening the overlay then would show — and
  // `Enter` would browse — the OLD repo's checks. Refuse instead, the
  // same contract as the project-layer keymap editor (#304).
  let (_dir, _repo, mut app) = app_with_open_ci_overlay_on_pr_61();
  app.close_detail_overlay();
  app.workspace_active_stale = true;

  app.enter_ci_checks();
  assert_eq!(app.view, View::List, "a stale selection must not open the overlay");
  assert!(
    app.status.contains("unavailable"),
    "the status line explains the refusal: {}",
    app.status
  );
}

#[test]
fn modal_ci_refresh_reapplies_the_workspace_stale_guard() {
  // Codex review #455 (P1): the modal dispatch calls the refresh directly,
  // bypassing run_action's `workspace_active_stale && is_repo_mutating`
  // guard. A selection gone stale AFTER the overlay opened (an async
  // relist landing on a repo `Repository::open` can no longer activate)
  // must not fetch — and persist PR metadata — through the previously
  // active repo's slug and handle. The overlay closes instead.
  let (_dir, _repo, mut app) = app_with_open_ci_overlay_on_pr_61();
  app.workspace_active_stale = true;

  app.ci_checks_refresh();
  assert_eq!(
    app.view,
    View::List,
    "a stale refresh closes the overlay instead of fetching"
  );
  assert!(
    app.status.contains("unavailable"),
    "the status line explains the close: {}",
    app.status
  );
}

#[test]
fn ci_filter_cursor_clamps_when_a_refresh_shrinks_the_matches() {
  // Codex review #455 (P2): `f` then `/` before the result lands, and the
  // new rollup has fewer matches than the cursor position — set_rows only
  // clamped `selected`, leaving `input_selected` out of bounds: no
  // highlight, Enter returning None, a stuck filter until several `Up`
  // presses. The filter cursor clamps against the new match count too.
  use gwm::github::{CheckOutcome, PrCheck};
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mk_check = |name: &str| PrCheck {
    name: name.into(),
    outcome: CheckOutcome::Passing,
    url: None,
    workflow_name: None,
    started_at: None,
    completed_at: None,
  };
  let mk_status = |checks: Vec<PrCheck>| PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: checks.len() as u32,
    checks_total: checks.len() as u32,
    ci: CiState::Passing,
    checks,
  };
  app.apply_pr_fetch_result(Ok(mk_status(vec![
    mk_check("check-a"),
    mk_check("check-b"),
    mk_check("check-c"),
  ])));
  app.enter_ci_checks();
  app.ci_input_open();
  app.ci_input_push('c');
  app.ci_input_next();
  app.ci_input_next();
  assert_eq!(
    app.detail_overlay.input_selected, 2,
    "the cursor sits on the last of 3 matches"
  );

  app.apply_pr_fetch_result(Ok(mk_status(vec![mk_check("check-a")])));
  assert_eq!(app.ci_input_matches().len(), 1, "one match remains after the landing");
  assert_eq!(
    app.detail_overlay.input_selected, 0,
    "the filter cursor clamps to the shrunk match list"
  );
}

#[test]
fn ci_overlay_ticks_running_check_durations() {
  // Codex review #455 (P2): a Running check's elapsed duration was
  // formatted once when the rows were built, then froze until the next
  // `f`. The poll-cadence tick rebuilds the rows from the cached PR state
  // while a check is still running — and stays a no-op once every check
  // is terminal, so idle frames do no churn.
  use gwm::github::{CheckOutcome, PrCheck};
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let started = (chrono::Utc::now() - chrono::Duration::seconds(90)).to_rfc3339();
  let mk_status = |outcome: CheckOutcome, started_at: Option<String>| PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: 1,
    ci: CiState::Running,
    checks: vec![PrCheck {
      name: "a".into(),
      outcome,
      url: None,
      workflow_name: Some("ci".into()),
      started_at,
      completed_at: None,
    }],
  };
  app.apply_pr_fetch_result(Ok(mk_status(CheckOutcome::Running, Some(started.clone()))));
  app.enter_ci_checks();
  app.detail_overlay.rows[0].extra = Some("frozen".into());

  app.tick_ci_overlay_durations();
  assert_ne!(
    app.detail_overlay.rows[0].extra.as_deref(),
    Some("frozen"),
    "a running check's duration is recomputed on the tick"
  );

  // Terminal outcomes: the tick must not rebuild anything.
  app.apply_pr_fetch_result(Ok(mk_status(CheckOutcome::Passing, Some(started))));
  app.detail_overlay.rows[0].extra = Some("frozen".into());
  app.tick_ci_overlay_durations();
  assert_eq!(
    app.detail_overlay.rows[0].extra.as_deref(),
    Some("frozen"),
    "no running check, no per-tick rebuild"
  );
}

#[test]
fn ci_overlay_ticks_survive_a_pr_cache_invalidation() {
  // Codex review #455 (P2): the tick read the checks back from the PR
  // fetch cache, so an invalidation while the overlay was up — a
  // workspace refresh_link with no bulk refetch, or a failed manual
  // refresh — silently killed the clock: the ellipsis still claimed an
  // active check but the duration froze for good. The overlay carries
  // its own checks now; the tick runs on what the overlay displays.
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::TaskKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let started = (chrono::Utc::now() - chrono::Duration::seconds(90)).to_rfc3339();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: 1,
    ci: CiState::Running,
    checks: vec![PrCheck {
      name: "a".into(),
      outcome: CheckOutcome::Running,
      url: None,
      workflow_name: Some("ci".into()),
      started_at: Some(started),
      completed_at: None,
    }],
  }));
  app.enter_ci_checks();

  // The cache is flushed while the overlay is up (same identity, so the
  // identity guard keeps it open).
  app.tasks.invalidate_matching(TaskKind::is_github);
  app.github.invalidate();

  app.detail_overlay.rows[0].extra = Some("frozen".into());
  app.tick_ci_overlay_durations();
  assert_ne!(
    app.detail_overlay.rows[0].extra.as_deref(),
    Some("frozen"),
    "the duration clock survives a PR cache invalidation"
  );
}

#[test]
fn pr_line_ci_hint_is_hidden_in_picker_mode() {
  // Codex review #455 (P2): in picker mode (`gwm switch`) run_action drops
  // Action::CiChecks — printable keys feed the filter — so the PR line
  // must not advertise a key that does nothing.
  use gwm::github::{CheckOutcome, PrCheck};
  let (dir, repo, _app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  let mut app = App::new_picker_at_layered(Some(dir.path()), None).unwrap();
  assert!(app.picker_mode);
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 10,
    checks_total: 10,
    ci: CiState::Passing,
    checks: vec![PrCheck {
      name: "ci".into(),
      outcome: CheckOutcome::Passing,
      url: None,
      workflow_name: None,
      started_at: None,
      completed_at: None,
    }],
  }));
  let text: String = gwm::tui::github_status_lines(&app, 120)
    .iter()
    .flat_map(|l| l.spans.iter())
    .map(|s| s.content.as_ref())
    .collect();
  assert!(text.contains("10/10"), "the CI indicator itself stays: {text}");
  assert!(
    !text.contains("10/10 ["),
    "picker mode must not advertise the dead ci_checks key: {text}"
  );
}

#[test]
fn ci_checks_refresh_and_filter_mirror_the_list_view_keys() {
  // Validation feedback on PR #455 (2026-07-24): inside the overlay `f`
  // re-fetches and `/` filters — the exact keys the list view uses for
  // refresh and filter. Rebindable under [tui.keys.modal.ci_checks].
  use gwm::tui::modal_keymap::{ModalAction, ModalKeymap};
  let modal = ModalKeymap::defaults();
  assert_eq!(modal.primary_key(ModalAction::CiChecksRefresh).as_deref(), Some("f"));
  assert_eq!(modal.primary_key(ModalAction::CiChecksFilter).as_deref(), Some("/"));
}

#[test]
fn agent_snapshot_landing_does_not_clobber_the_ci_overlay() {
  // Codex review on PR #455: an agents overlay interrupted without a close
  // (an async task flipping the view) leaves detail_overlay_target set; a
  // detection landing while the CI overlay is later open used to rebuild
  // the rows as agent sessions while the kind stayed CiChecks — Enter then
  // tried to open a session id as a URL. The landing rebuild is now gated
  // on DetailKind::Agents and enter_ci_checks drops the stale target.
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::DetailKind;
  use gwm::tui::TaskKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 1,
    checks_total: 1,
    ci: CiState::Passing,
    checks: vec![PrCheck {
      name: "ci".into(),
      outcome: CheckOutcome::Passing,
      url: None,
      workflow_name: None,
      started_at: None,
      completed_at: None,
    }],
  }));

  // Agents overlay opened (captures the target), then interrupted without
  // a close — the exact hole the review describes.
  app.open_agent_overlay();
  app.view = View::Report;

  app.focus_status();
  app.enter_ci_checks();
  assert_eq!(app.detail_overlay.kind, DetailKind::CiChecks);
  assert_eq!(app.detail_overlay.rows[0].value, "ci");

  let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
  assert!(app.apply_agent_snapshot(
    generation,
    std::collections::BTreeMap::new(),
    None,
    std::collections::BTreeMap::new()
  ));
  assert_eq!(
    app.detail_overlay.kind,
    DetailKind::CiChecks,
    "kind must survive the landing"
  );
  assert_eq!(
    app.detail_overlay.rows[0].value, "ci",
    "the CI rows must not be replaced by agent rows"
  );
}

#[test]
fn pr_line_ci_hint_disappears_when_the_binding_is_removed() {
  // Codex review on PR #455: `ci_checks = []` (or `edit_worktree = []` in
  // the status context) unbinds the action — the PR line must then drop
  // the key suffix instead of advertising a dead key.
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::keymap::Action;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "CI checks fixture".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 1,
    checks_total: 1,
    ci: CiState::Passing,
    checks: vec![PrCheck {
      name: "ci".into(),
      outcome: CheckOutcome::Passing,
      url: None,
      workflow_name: None,
      started_at: None,
      completed_at: None,
    }],
  }));
  app.focus_worktrees();
  app.keymap.apply_override(Action::CiChecks, Vec::new()).unwrap();

  let text: String = gwm::tui::github_status_lines(&app, 120)
    .iter()
    .flat_map(|l| l.spans.iter())
    .map(|s| s.content.as_ref())
    .collect();
  assert!(
    text.contains("1/1") && !text.contains("1/1 ["),
    "an unbound ci_checks must drop the key suffix: {text}"
  );
}

#[test]
fn enter_ci_checks_error_resolves_the_fetch_key() {
  // Codex review on PR #455: the "fetch (F) first" hint hard-coded `F`;
  // the message now resolves the active fetch_github binding and drops
  // the parenthetical entirely when the action is unbound.
  use gwm::tui::keymap::Action;
  let (_dir, mut app) = make_app();
  app.enter_ci_checks();
  assert!(
    app.status.contains("(F)"),
    "default binding shows in the hint: {}",
    app.status
  );

  app.keymap.apply_override(Action::FetchGithub, Vec::new()).unwrap();
  app.enter_ci_checks();
  assert!(
    !app.status.contains('(') && app.status.contains("fetch"),
    "unbound fetch drops the parenthetical: {}",
    app.status
  );
}

#[test]
fn loaded_explicit_pr_status_persists_title_for_no_fetch_startup() {
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "Persisted explicit PR title".into(),
    state: PrState::Open,
    url: "https://example.test/pull/61".into(),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: 0,
    ci: CiState::None,
    checks: vec![],
  }));

  let link = gwm::github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.pr, Some(61));
  assert_eq!(link.pr_source, LinkSource::Explicit);
  assert_eq!(link.pr_title.as_deref(), Some("Persisted explicit PR title"));
}

#[test]
fn loaded_detected_pr_status_persists_detected_title_for_no_fetch_startup() {
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::persist_detected_pr(&repo, "feat/#42-tui-search", 77).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 77,
    title: "Persisted detected PR title".into(),
    state: PrState::Merged,
    url: "https://example.test/pull/77".into(),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: 0,
    ci: CiState::None,
    checks: vec![],
  }));

  let link = gwm::github::read_link(&repo, "feat/#42-tui-search").unwrap();
  assert_eq!(link.pr, Some(77));
  assert_eq!(link.pr_source, LinkSource::Detected);
  assert_eq!(link.pr_title.as_deref(), Some("Persisted detected PR title"));
}

#[test]
fn github_status_lines_show_persisted_titles_before_fetch() {
  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feat/#42-tui-search", &head, false).unwrap();
    let mut cfg = repo.config().unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-issue-title", "Startup issue title")
      .unwrap();
    cfg.set_str("branch.feat/#42-tui-search.gwm-pr-detected", "77").unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-detected-title", "Startup PR title")
      .unwrap();
  }
  repo.set_head("refs/heads/feat/#42-tui-search").unwrap();
  let app = App::new_at_layered(Some(dir.path()), None).unwrap();

  let text = gwm::tui::github_status_lines(&app, 120)
    .into_iter()
    .map(|line| spans_to_text(&line.spans))
    .collect::<Vec<_>>()
    .join("\n");

  assert!(text.contains("Startup issue title"), "issue title missing: {text}");
  assert!(text.contains("Startup PR title"), "PR title missing: {text}");
}

#[test]
fn github_status_lines_show_persisted_state_before_fetch() {
  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feat/#42-tui-search", &head, false).unwrap();
  }
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  {
    let mut cfg = repo.config().unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-issue-title", "Closed issue")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-issue-state", "closed")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-title", "Closed PR")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-state", "closed")
      .unwrap();
  }
  repo.set_head("refs/heads/feat/#42-tui-search").unwrap();
  let app = App::new_at_layered(Some(dir.path()), None).unwrap();

  let lines = gwm::tui::github_status_lines(&app, 120);
  let text = lines
    .iter()
    .map(|line| spans_to_text(&line.spans))
    .collect::<Vec<_>>()
    .join("\n");
  assert!(text.contains(" closed "), "persisted issue state missing: {text}");
  assert!(text.contains("Closed issue"), "persisted issue title missing: {text}");
  assert!(text.contains("Closed PR"), "persisted PR title missing: {text}");

  let theme = Theme::default();
  assert_eq!(
    lines[0].spans[0].style.fg,
    Some(gwm::tui::issue_badge_color(IssueState::Closed, &theme)),
    "persisted issue icon should use the persisted state colour"
  );
  assert_eq!(
    lines[1].spans[0].style.fg,
    Some(gwm::tui::pr_badge_color(PrState::Closed, &theme)),
    "persisted PR icon should use the persisted state colour"
  );
}

#[test]
fn github_status_lines_keep_persisted_state_visible_while_loading() {
  use gwm::tui::FetchKey;

  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feat/#42-tui-search", &head, false).unwrap();
  }
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  {
    let mut cfg = repo.config().unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-issue-title", "Closed issue")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-issue-state", "closed")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-title", "Merged PR")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-state", "merged")
      .unwrap();
  }
  repo.set_head("refs/heads/feat/#42-tui-search").unwrap();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  app.github.mark_loading(FetchKey::Issue(42));
  app.github.mark_loading(FetchKey::Pr(61));

  let lines = gwm::tui::github_status_lines(&app, 120);
  let text = lines
    .iter()
    .map(|line| spans_to_text(&line.spans))
    .collect::<Vec<_>>()
    .join("\n");
  assert!(
    text.contains(" closed "),
    "loading issue line should keep cached state: {text}"
  );
  assert!(
    text.contains(" merged "),
    "loading PR line should keep cached state: {text}"
  );
  assert!(
    text.contains("loading"),
    "loading line should still disclose the refresh: {text}"
  );

  let theme = Theme::default();
  assert_eq!(
    lines[0].spans[0].style.fg,
    Some(gwm::tui::issue_badge_color(IssueState::Closed, &theme)),
    "loading issue icon should keep the persisted state colour"
  );
  assert_eq!(
    lines[1].spans[0].style.fg,
    Some(gwm::tui::pr_badge_color(PrState::Merged, &theme)),
    "loading PR icon should keep the persisted state colour"
  );
}

fn sample_issue(n: u64) -> gwm::github::IssueStatus {
  sample_issue_titled(n, "x")
}

fn sample_issue_titled(n: u64, title: &str) -> gwm::github::IssueStatus {
  gwm::github::IssueStatus {
    number: n,
    title: title.into(),
    state: gwm::github::IssueState::Open,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  }
}

/// Drive the spine exactly as `refresh_github_status` does for one issue
/// fetch: claim a generation and flip the cache to `Loading`. Returns the
/// generation the (simulated) worker must tag its result with. Mirrors the
/// real spawn path without an OS thread or a real `gh`.
fn request_github_issue(app: &mut gwm::tui::App, n: u64) -> u64 {
  use gwm::tui::{FetchKey, TaskKind};
  let generation = app
    .tasks
    .request(TaskKind::GithubIssue(n))
    .expect("a cold GitHub issue slot must hand out a generation");
  app.github.mark_loading(FetchKey::Issue(n));
  generation
}

/// PR-side counterpart to [`request_github_issue`].
fn request_github_pr(app: &mut gwm::tui::App, n: u64) -> u64 {
  use gwm::tui::{FetchKey, TaskKind};
  let generation = app
    .tasks
    .request(TaskKind::GithubPr(n))
    .expect("a cold GitHub PR slot must hand out a generation");
  app.github.mark_loading(FetchKey::Pr(n));
  generation
}

/// Invalidate the GitHub side exactly as navigation / explicit `F` does:
/// drop any in-flight worker on the spine AND flush the result cache (the
/// navigation invariant — the two always move together).
fn invalidate_github_for_test(app: &mut gwm::tui::App) {
  use gwm::tui::TaskKind;
  app.tasks.invalidate_matching(TaskKind::is_github);
  app.github.invalidate();
}

#[test]
fn stale_github_fetch_result_loses_to_a_newer_generation() {
  // Codex adversarial-review (PR #260) finding, fixed by #255: pre-spine the
  // GitHub fetch deduped on a per-key `inflight` HashSet with NO generation,
  // so two workers for the SAME key (request → invalidate → request) were
  // indistinguishable. Whichever result drained first claimed the single
  // slot; if the STALE worker reported before the FRESH one, the stale data
  // was stamped and the fresh result dropped. On the spine each fetch owns a
  // generation, so the fresh (newer-generation) result wins regardless of
  // arrival order.
  use gwm::tui::TaskMsg;
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");

  // Worker A claims the first generation for Issue(42).
  let gen_a = request_github_issue(&mut app, 42);
  // User navigates away and back (or presses F again): the slot is freed and
  // the generation bumped, then Worker B claims a fresh generation.
  invalidate_github_for_test(&mut app);
  let gen_b = request_github_issue(&mut app, 42);
  assert_ne!(gen_a, gen_b, "the second fetch must own a distinct generation");

  // Both workers finish. Drain FRESH (B) FIRST, STALE (A) LAST — the
  // discriminating order: plain last-write-wins would let the stale result
  // clobber the fresh one, so only a working generation guard (drop A as
  // superseded) yields FRESH. The reverse order would pass even with a
  // broken always-apply guard, so it can't catch a regression.
  let tx = app.task_result_sender();
  tx.send(TaskMsg::GithubIssue(gen_b, 42, Ok(sample_issue_titled(42, "FRESH"))))
    .unwrap();
  tx.send(TaskMsg::GithubIssue(gen_a, 42, Ok(sample_issue_titled(42, "STALE"))))
    .unwrap();
  app.drain_task_results();

  match app.issue_fetch_state() {
    GitHubFetchState::Loaded(s) => assert_eq!(
      s.title, "FRESH",
      "the fresh (newer-generation) result must win the retry race, not the stale one"
    ),
    other => panic!("expected Loaded(FRESH), got {:?}", other),
  }
}

#[test]
fn a_simultaneous_refresh_keeps_its_status_over_the_github_report() {
  // Behaviour-preserving guard (issue #255): pre-spine the event loop drained
  // the GitHub channel before the task channel, so when a worktree refresh and
  // a GitHub fetch completed on the same tick, `apply_refreshed_worktrees`'
  // "refreshed — N" message ran last and stood. Now both drain in one pass; the
  // post-loop GitHub report is gated on `!refresh_applied` to preserve that.
  use gwm::tui::{TaskKind, TaskMsg};
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");

  // A GitHub fetch and a worktree refresh are both in flight.
  let g_gen = request_github_issue(&mut app, 42);
  let r_gen = app
    .tasks
    .request(TaskKind::RefreshWorktrees)
    .expect("a cold refresh slot must hand out a generation");

  // Both land on the same drain.
  let tx = app.task_result_sender();
  tx.send(TaskMsg::GithubIssue(g_gen, 42, Ok(sample_issue(42)))).unwrap();
  tx.send(TaskMsg::RefreshWorktrees(r_gen, Ok(Vec::new()))).unwrap();
  app.drain_task_results();

  assert!(
    app.status.starts_with("refreshed —"),
    "the refresh message must win a simultaneous completion (pre-#255 order), got {:?}",
    app.status
  );
}

#[test]
fn drain_applies_async_github_result() {
  // Issue #217 (threading on the spine since #255): a result delivered
  // off-thread (over the task channel) is applied by `drain_task_results`,
  // flipping the Loading state to Loaded.
  use gwm::tui::TaskMsg;
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  // Claim a generation + mark Loading exactly as `refresh_github_status` would.
  let generation = request_github_issue(&mut app, 42);
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Loading));
  assert!(app.is_github_loading(), "request must mark the app as loading");

  // A background worker reports back; we inject through the same channel.
  app
    .task_result_sender()
    .send(TaskMsg::GithubIssue(generation, 42, Ok(sample_issue(42))))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "drain must report it applied a result");
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Loaded(_)));
  assert!(!app.is_github_loading(), "no fetch should be inflight after draining");
}

#[test]
fn drain_drops_async_result_invalidated_mid_flight() {
  // The #138 guarantee on the spine (issue #255): a result whose generation
  // was bumped by an intervening navigation/invalidate is dropped rather
  // than stamped into the now-active worktree's cache.
  use gwm::tui::TaskMsg;
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let generation = request_github_issue(&mut app, 42);
  // User navigates away → the cache is flushed and the spine slot bumped.
  invalidate_github_for_test(&mut app);
  // The late shell-out result arrives after the invalidate, tagged with the
  // now-stale generation.
  app
    .task_result_sender()
    .send(TaskMsg::GithubIssue(generation, 42, Ok(sample_issue(42))))
    .unwrap();
  app.drain_task_results();
  assert!(
    matches!(app.issue_fetch_state(), GitHubFetchState::Idle),
    "a result invalidated mid-flight must be dropped, not applied"
  );
}

#[test]
fn drain_is_a_noop_with_no_pending_results() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  assert!(!app.drain_task_results(), "empty channel must report nothing applied");
}

#[test]
fn drain_does_not_report_when_only_stale_results_arrive() {
  // Issue #217 review (P2), preserved on the spine: a result whose
  // generation was bumped (the user navigated away) is dropped by the
  // generation guard; the drain must NOT then stamp "github status
  // refreshed" over the current status message.
  use gwm::tui::TaskMsg;
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let generation = request_github_issue(&mut app, 42);
  invalidate_github_for_test(&mut app); // navigated away → slot bumped
  app.status = "path: /somewhere/else".into();
  app
    .task_result_sender()
    .send(TaskMsg::GithubIssue(generation, 42, Ok(sample_issue(42))))
    .unwrap();

  let applied = app.drain_task_results();

  assert!(!applied, "a dropped stale result must not count as applied");
  assert_eq!(
    app.status, "path: /somewhere/else",
    "a stale result must not overwrite the current status message"
  );
}

#[test]
fn hint_context_prioritises_an_open_modal_over_pane_focus() {
  // Issue #217 review (P2): when a modal is open the statusbar must show the
  // modal's context, not the pane behind it. Pressing `n` in the create form
  // types text — advertising the worktrees `n new` hint there is misleading.
  use gwm::tui::{HintContext, View};
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.focus_status(); // pane focus would otherwise resolve to Status
  app.view = View::Create;
  assert_eq!(app.hint_context(), HintContext::Create);
  app.view = View::Confirm;
  assert_eq!(app.hint_context(), HintContext::Confirm);
  app.view = View::CommandPalette;
  assert_eq!(app.hint_context(), HintContext::CommandPalette);
  // Back on the list, the pane focus is honoured again.
  app.view = View::List;
  assert_eq!(app.hint_context(), HintContext::Status);
}

#[test]
fn apply_fetch_error_stores_error_state() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.apply_issue_fetch_result(Err("gh not found".into()));
  match app.issue_fetch_state() {
    GitHubFetchState::Error(msg) => assert!(msg.contains("gh"), "msg = {}", msg),
    other => panic!("expected Error, got {:?}", other),
  }
}

#[test]
fn refresh_link_invalidates_fetch_state() {
  // After the user changes selection or the branch link changes, any
  // previously fetched status no longer applies. The state must reset.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.apply_issue_fetch_result(Err("e".into()));
  app.refresh_link();
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn next_resets_fetch_state_on_selection_change() {
  // PR #68 Copilot review: selection change must invalidate the cached
  // GitHub fetch state, otherwise the right-panel Issue/PR block shows
  // stale data from the previously selected worktree.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("alt"));
  app.list_state.select(Some(0));
  app.apply_issue_fetch_result(Err("stale".into()));
  app.next();
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn prev_resets_fetch_state_on_selection_change() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("alt"));
  app.list_state.select(Some(0));
  app.apply_pr_fetch_result(Err("stale".into()));
  app.prev();
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn first_resets_fetch_state_on_selection_change() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("alt"));
  app.list_state.select(Some(1));
  app.apply_issue_fetch_result(Err("stale".into()));
  app.first();
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn last_resets_fetch_state_on_selection_change() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("alt"));
  app.list_state.select(Some(0));
  app.apply_issue_fetch_result(Err("stale".into()));
  app.last();
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn filter_clamping_resets_fetch_state_when_selection_moves() {
  // When typing into the filter narrows the visible set so much that the
  // current selection no longer points at the same worktree, the link
  // cache must invalidate too. Otherwise the right-panel block lies.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.worktrees.push(worktree_fixture("zzz-unique"));
  app.list_state.select(Some(1));
  app.apply_issue_fetch_result(Err("stale".into()));
  app.enter_filter();
  app.filter_push_char('z'); // only the second fixture matches
                             // The selection survives but the link cache should still reset because
                             // the filter operation can drop selection back to index 0 on the
                             // filtered subset. The contract: any selection-state mutation refreshes.
  assert!(matches!(app.issue_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn edit_worktree_failure_replaces_the_loading_status() {
  // Codex review on PR #292 (P3): a recoverable rename failure keeps the modal
  // open (edit_failure set) but must not leave the status bar on the
  // "renaming worktree…" loading label, which reads as still-in-progress.
  use gwm::tui::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let generation = app
    .tasks
    .request(TaskKind::EditWorktree)
    .expect("a fresh task generation");
  app.status = TaskKind::EditWorktree.loading_label().into();
  let tx = app.task_result_sender();
  tx.send(TaskMsg::EditWorktree(
    generation,
    Err("target path already exists".into()),
  ))
  .unwrap();
  app.drain_task_results();

  assert_ne!(
    app.status,
    TaskKind::EditWorktree.loading_label(),
    "the loading label must be replaced after a failure"
  );
  assert!(
    app.status.contains("target path already exists"),
    "status must surface the rename failure: {}",
    app.status
  );
  assert_eq!(
    app.edit_failure.as_deref(),
    Some("target path already exists"),
    "the modal keeps the failure for inline display"
  );
}

#[test]
fn fullscreen_child_stdout_routes_to_tty_only_when_gwm_stdout_is_captured() {
  // Codex review on PR #292 (P2): a fullscreen launcher (L/O/R) must keep its
  // stdout off the `cd "$(gwm)"` command-substitution pipe. Policy: reroute to
  // the tty exactly when gwm's own stdout is NOT a terminal (captured). The
  // actual /dev/tty open is env-dependent I/O; the decision is pure and pinned
  // here.
  assert!(
    gwm::tui::wants_child_stdout_on_tty(false),
    "captured stdout (pipe) → child stdout must be rerouted to the tty"
  );
  assert!(
    !gwm::tui::wants_child_stdout_on_tty(true),
    "a real tty stdout → inherit, no reroute"
  );
}

#[test]
fn reselect_by_path_maps_the_renamed_row_through_an_active_filter() {
  // Codex review on PR #292 (P2): after a rename with a filter active, the
  // cursor must land on the renamed row via its slot in the *filtered* list,
  // not its raw index — a raw index points at a different visible row or none.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![
    worktree_fixture("alpha"),     // raw 0 — filtered out by "zz"
    worktree_fixture("beta-zz"),   // raw 1 — filtered slot 0
    worktree_fixture("target-zz"), // raw 2 — filtered slot 1
  ];
  app.enter_filter();
  for c in "zz".chars() {
    app.filter_push_char(c);
  }

  app.reselect_by_path(&PathBuf::from("/tmp/gwm-test/target-zz"));

  // The raw index (2) overflows the 2-row filtered list; only the mapped
  // filtered slot (1) lands on the renamed worktree.
  assert_eq!(
    app.list_state.selected(),
    Some(1),
    "selection must be the filtered slot, not the raw index"
  );
  assert_eq!(
    app.selected().expect("a selection").name,
    "target-zz",
    "cursor must land on the renamed row through the filter"
  );
}

#[test]
fn refresh_worktrees_resets_fetch_state() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.apply_pr_fetch_result(Err("stale".into()));
  app.refresh().unwrap();
  assert!(matches!(app.pr_fetch_state(), GitHubFetchState::Idle));
}

#[test]
fn refresh_github_status_message_reflects_partial_failure() {
  // PR #68 Copilot review: when one of the fetches fails the status line
  // must not claim "refreshed" — the user should see something hinting
  // the refresh was incomplete.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  // Simulate the result of a refresh where the issue fetch failed but
  // (for the sake of this test) the PR fetch went through. Apply the
  // failure directly — we can't shell out to `gh` in unit tests.
  app.apply_issue_fetch_result(Err("gh: connection refused".into()));
  let pr = gwm::github::PrStatus {
    number: 1,
    title: "x".into(),
    state: gwm::github::PrState::Open,
    url: "https://example.test/pr".into(),
    updated_at: "".into(),
    checks_passed: 0,
    checks_total: 0,
    ci: CiState::None,
    checks: vec![],
  };
  app.apply_pr_fetch_result(Ok(pr));
  // Now call the same status-rendering logic the refresh would have run.
  app.report_github_refresh_status();
  assert!(
    !app.status.contains("refreshed"),
    "status must not claim 'refreshed' on partial failure: {}",
    app.status
  );
  assert!(
    app.status.to_lowercase().contains("error") || app.status.to_lowercase().contains("fail"),
    "status should mention failure: {}",
    app.status
  );
}

#[cfg(unix)]
#[test]
fn refresh_github_status_auto_detects_pr_for_unlinked_branch() {
  use std::os::unix::fs::PermissionsExt;

  // A branch with no issue number in its name and no explicit PR link.
  // Pre-#181 refresh_github_status bailed with "nothing linked"; now it
  // detects the branch's PR via `gh pr list` and surfaces it with
  // source `Detected`. Gated #[cfg(unix)] — the cross-OS gh contract is
  // covered by the pure github_tests + the cli_binary status E2E (which
  // ships a Windows fake-gh too).
  let (dir, repo, mut app) = make_app_on_branch("detect-me");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  // Re-resolve the slug now that the remote exists.
  app.refresh_link();

  // Write a fake `gh` (detecting PR `n` via both `pr list` and `pr view`)
  // to its own path. Two distinct scripts — never one rewritten in place —
  // because the first refresh spawns an off-thread `pr view` worker (issue
  // #255) that may still be executing its script when the second refresh
  // fires. Truncating a script mid-exec raced that worker (`ETXTBSY` on
  // Linux → the re-detect's spawn fails → `None`): the #248 flake. Keeping
  // every script write-once / exec-many removes the race on any OS.
  let write_gh = |path: &std::path::Path, n: u64| {
    std::fs::write(
      path,
      format!(
        "#!/bin/sh\n\
         if [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then\n\
           printf '%s' '[{{\"number\":{n}}}]'\n\
         elif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n\
           printf '%s' '{{\"number\":{n},\"title\":\"x\",\"state\":\"OPEN\",\"isDraft\":false,\"url\":\"https://example.test/pull/{n}\"}}'\n\
         fi\n"
      ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
  };
  let gh_first = dir.path().join("fake-gh-128");
  let gh_second = dir.path().join("fake-gh-200");
  write_gh(&gh_first, 128);
  write_gh(&gh_second, 200);

  // Serialise against the other env-mutating tests in this binary.
  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: env mutation is guarded by `env_lock()` above; GWM_GH is
  // restored at the end of the test, before returning.
  unsafe {
    std::env::set_var("GWM_GH", &gh_first);
  }

  // First refresh: nothing linked → detect PR #128.
  app.refresh_github_status();
  assert_eq!(app.current_link().pr, Some(128));
  assert_eq!(app.current_link().pr_source, LinkSource::Detected);
  // Table-snapshot sync (Codex review #284): the table renders from
  // `self.worktrees[*].link`, not the live `github.link`, so a detection
  // must be mirrored onto the selected row or its PR pastille stays white
  // until a separate relist.
  assert_eq!(
    app.selected().map(|w| w.link.pr),
    Some(Some(128)),
    "the selected row snapshot must reflect the detected PR immediately"
  );

  // The branch's PR changed (e.g. closed + reopened as #200). A detected
  // link is "resolved live", so a second refresh must re-detect rather
  // than stick to #128 (issue #181 — Copilot review on PR #184). Point at
  // the second script — written once, never the file the first refresh's
  // worker is still execing — so the re-detect can't race that exec.
  unsafe {
    std::env::set_var("GWM_GH", &gh_second);
  }
  app.refresh_github_status();

  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  assert_eq!(
    app.current_link().pr,
    Some(200),
    "a detected PR must re-resolve on refresh"
  );
  assert_eq!(app.current_link().pr_source, LinkSource::Detected);

  // Wiring guard (issue #283): the detection must be PERSISTED to the git
  // config, not just held in memory — that is what lets the no-fetch table
  // read path colour the PR pastille on every row. Read the link straight
  // from the repo (bypassing the in-memory `app.github.link`) so dropping
  // `persist_detected_pr` from `refresh_github_status` turns this red.
  let persisted = gwm::github::read_link(&repo, "detect-me").unwrap();
  assert_eq!(
    persisted.pr,
    Some(200),
    "the detected PR must be persisted so the table read path sees it"
  );
  assert_eq!(persisted.pr_source, LinkSource::Detected);
}

#[test]
fn refresh_keeps_persisted_pr_when_no_remote_slug() {
  // Codex review #284: pressing `F` when the probe cannot run at all (no
  // origin remote → `link_slug` is None) must NOT blank a persisted
  // detection. The unconditional `clear_detected_pr()` used to wipe the
  // in-memory link before the skipped probe could restore it.
  let (_dir, repo, mut app) = make_app_on_branch("detect-me");
  // No origin remote is configured, so there is no slug to probe with.
  gwm::github::persist_detected_pr(&repo, "detect-me", 128).unwrap();
  app.refresh_link();
  assert_eq!(
    app.current_link().pr,
    Some(128),
    "precondition: the persisted detection loads into memory"
  );

  app.refresh_github_status();

  assert_eq!(
    app.current_link().pr,
    Some(128),
    "a refresh that cannot probe must keep the persisted detection"
  );
  assert_eq!(app.current_link().pr_source, LinkSource::Detected);
}

#[cfg(unix)]
#[test]
fn refresh_keeps_persisted_pr_when_gh_detection_fails() {
  use std::os::unix::fs::PermissionsExt;

  // Codex review #284: a transient `gh` failure (missing binary, offline,
  // rate limit) must NOT wipe a previously persisted detection — pressing
  // `F` offline should keep showing the PR, not blank it.
  let (dir, repo, mut app) = make_app_on_branch("detect-me");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  app.refresh_link();

  // A `gh` that detects PR #128, then a `gh` that always fails (exit 1).
  let gh_ok = dir.path().join("fake-gh-ok");
  std::fs::write(
    &gh_ok,
    "#!/bin/sh\n\
     if [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then\n\
       printf '%s' '[{\"number\":128}]'\n\
     elif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n\
       printf '%s' '{\"number\":128,\"title\":\"x\",\"state\":\"OPEN\",\"isDraft\":false,\"url\":\"https://example.test/pull/128\"}'\n\
     fi\n",
  )
  .unwrap();
  let gh_fail = dir.path().join("fake-gh-fail");
  std::fs::write(&gh_fail, "#!/bin/sh\nexit 1\n").unwrap();
  for p in [&gh_ok, &gh_fail] {
    let mut perms = std::fs::metadata(p).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(p, perms).unwrap();
  }

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: env mutation guarded by `env_lock()`; GWM_GH restored below.
  unsafe {
    std::env::set_var("GWM_GH", &gh_ok);
  }
  app.refresh_github_status();
  assert_eq!(app.current_link().pr, Some(128), "first refresh detects #128");

  // Now `gh` fails. The probe did not prove the PR vanished.
  unsafe {
    std::env::set_var("GWM_GH", &gh_fail);
  }
  app.refresh_github_status();

  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  // The persisted key survives a failed probe...
  let persisted = gwm::github::read_link(&repo, "detect-me").unwrap();
  assert_eq!(
    persisted.pr,
    Some(128),
    "a failed gh probe must not wipe the persisted detection"
  );
  assert_eq!(persisted.pr_source, LinkSource::Detected);
  // ...and stays visible in memory rather than blanking the pane.
  assert_eq!(
    app.current_link().pr,
    Some(128),
    "the pane must keep the still-valid PR after a failed probe"
  );
}

#[cfg(unix)]
#[test]
fn read_link_with_pr_detection_refreshes_a_persisted_detection() {
  use std::os::unix::fs::PermissionsExt;

  // Codex review #284: a persisted detection (#283) must NOT make the live
  // CLI detection path (`gwm status` / `gwm list --detect-pr`) authoritative.
  // It must still re-run `gh pr list` so a PR that was replaced since the
  // last TUI `F` is reflected — only an explicit `gwm link --pr` pins it.
  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("detect-me", &head, false).unwrap();
  }

  // Stale persisted detection: #128.
  gwm::github::persist_detected_pr(&repo, "detect-me", 128).unwrap();

  // Live `gh` now reports #200 for the branch.
  let gh = dir.path().join("fake-gh-200");
  std::fs::write(
    &gh,
    "#!/bin/sh\n\
     if [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then\n\
       printf '%s' '[{\"number\":200}]'\n\
     fi\n",
  )
  .unwrap();
  let mut perms = std::fs::metadata(&gh).unwrap().permissions();
  perms.set_mode(0o755);
  std::fs::set_permissions(&gh, perms).unwrap();

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: env mutation guarded by `env_lock()`; GWM_GH restored below.
  unsafe {
    std::env::set_var("GWM_GH", &gh);
  }
  let link = gwm::github::read_link_with_pr_detection(&repo, "detect-me", "kbrdn1/gwm-cli").unwrap();
  // The live path must reconcile the persisted cache (#284), not just memory,
  // so no-fetch consumers (table at startup, `gwm open pr`) don't resurrect
  // the stale #128. Capture the stored value before the explicit link below.
  let reconciled = gwm::github::read_link(&repo, "detect-me").unwrap();

  // Explicit override still wins even over a live re-detection.
  gwm::github::link_pr(&repo, "detect-me", 61).unwrap();
  let explicit = gwm::github::read_link_with_pr_detection(&repo, "detect-me", "kbrdn1/gwm-cli").unwrap();

  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  assert_eq!(
    link.pr,
    Some(200),
    "live detection must override the stale persisted #128"
  );
  assert_eq!(link.pr_source, LinkSource::Detected);
  assert_eq!(
    reconciled.pr,
    Some(200),
    "the live detection must rewrite the persisted cache to the fresh number"
  );
  assert_eq!(explicit.pr, Some(61), "an explicit link still wins over live detection");
  assert_eq!(explicit.pr_source, LinkSource::Explicit);
}

#[cfg(unix)]
#[test]
fn read_link_with_pr_detection_keeps_title_when_detected_pr_is_unchanged() {
  use std::os::unix::fs::PermissionsExt;

  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("detect-me", &head, false).unwrap();
  }

  gwm::github::persist_detected_pr(&repo, "detect-me", 128).unwrap();
  gwm::github::persist_detected_pr_title(&repo, "detect-me", "Cached detected title").unwrap();

  let gh = dir.path().join("fake-gh-128");
  std::fs::write(
    &gh,
    "#!/bin/sh\n\
     if [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then\n\
       printf '%s' '[{\"number\":128}]'\n\
     fi\n",
  )
  .unwrap();
  let mut perms = std::fs::metadata(&gh).unwrap().permissions();
  perms.set_mode(0o755);
  std::fs::set_permissions(&gh, perms).unwrap();

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: env mutation guarded by `env_lock()`; GWM_GH restored below.
  unsafe {
    std::env::set_var("GWM_GH", &gh);
  }
  let link = gwm::github::read_link_with_pr_detection(&repo, "detect-me", "kbrdn1/gwm-cli").unwrap();

  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  assert_eq!(link.pr, Some(128));
  assert_eq!(link.pr_source, LinkSource::Detected);
  assert_eq!(link.pr_title.as_deref(), Some("Cached detected title"));
}

#[cfg(unix)]
#[test]
fn read_link_with_pr_detection_clears_persisted_cache_when_pr_vanished() {
  use std::os::unix::fs::PermissionsExt;

  // Codex review #284: when the live probe proves the PR is gone (`Ok(None)`),
  // the live CLI path must clear the persisted cache too, otherwise a no-fetch
  // read (`read_link`) would resurrect the stale stored number.
  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("detect-me", &head, false).unwrap();
  }
  gwm::github::persist_detected_pr(&repo, "detect-me", 128).unwrap();

  // Live `gh pr list` returns an empty array — the PR no longer exists.
  let gh = dir.path().join("fake-gh-empty");
  std::fs::write(
    &gh,
    "#!/bin/sh\n\
     if [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then\n\
       printf '%s' '[]'\n\
     fi\n",
  )
  .unwrap();
  let mut perms = std::fs::metadata(&gh).unwrap().permissions();
  perms.set_mode(0o755);
  std::fs::set_permissions(&gh, perms).unwrap();

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: env mutation guarded by `env_lock()`; GWM_GH restored below.
  unsafe {
    std::env::set_var("GWM_GH", &gh);
  }
  let link = gwm::github::read_link_with_pr_detection(&repo, "detect-me", "kbrdn1/gwm-cli").unwrap();
  let stored = gwm::github::read_link(&repo, "detect-me").unwrap();
  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  assert_eq!(link.pr, None, "a vanished PR resolves to no PR live");
  assert_eq!(
    stored.pr, None,
    "the persisted cache must be cleared so no-fetch reads don't resurrect it"
  );
}

// ---- Configurable launchers (issue #75) --------------------------------
//
// The `R` key in the worktree-list view now triggers the [review]
// launcher; the previous "refresh GitHub status" action moves to `F`.
// `f` becomes the worktree refresh (previously `r`). These tests pin
// the new methods that back those keybindings; the actual key
// dispatch sits in `src/tui/mod.rs` and is exercised by the
// behaviour we assert here.

#[test]
fn prepare_review_returns_none_when_no_review_configured() {
  // Default config has no [review] block ⇒ pressing `R` must surface
  // a status-bar hint, not spawn anything.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let plan = app.prepare_review();
  assert!(plan.is_none(), "no [review] config ⇒ no launcher plan");
  let s = app.status.to_lowercase();
  assert!(
    s.contains("review") && (s.contains("not configured") || s.contains("not set") || s.contains("gwm.toml")),
    "status bar must explain why R was inert: {}",
    app.status
  );
}

#[test]
fn prepare_review_skips_when_no_changes_and_flag_on() {
  // skip_when_no_changes defaults to true. A fresh repo has zero
  // commits past `main` ⇒ R must short-circuit with the documented
  // "no changes to review" status line.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.config.review.command = Some("lumen diff {base}..{head}".into());
  app.config.review.fullscreen = Some(true);
  // The branch was created from HEAD (main), and we have made no new
  // commits — count_commits_ahead must be 0.
  let plan = app.prepare_review();
  assert!(plan.is_none(), "no commits past base + skip_when_no_changes ⇒ skip");
  let s = app.status.to_lowercase();
  assert!(
    s.contains("no changes"),
    "status bar must say 'no changes': {}",
    app.status
  );
  assert!(
    app.status.contains("main") || app.status.contains("dev"),
    "status should name the resolved base: {}",
    app.status
  );
}

#[test]
fn prepare_review_returns_plan_when_configured_and_diff_exists() {
  // The full happy path: [review].command set, head ≠ base (one extra
  // commit), skip_when_no_changes off so we don't have to bother
  // creating a diff. The plan must surface the expanded argv with all
  // placeholders substituted.
  let (dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.config.review.command = Some("reviewer --base {base} --head {head}".into());
  app.config.review.skip_when_no_changes = false;
  // Force a known base so the test isn't sensitive to the chain order.
  app.config.review.default_base = Some("main".into());
  // Drop the upstream / gwm-base config so default_base wins.
  let mut cfg = repo.config().unwrap();
  let _ = cfg.remove("branch.feat/#42-tui-search.gwm-base");

  let plan = app.prepare_review().expect("configured + no skip ⇒ plan present");
  let argv = &plan.expanded.argv;
  assert_eq!(argv[0], "reviewer");
  assert_eq!(argv[1], "--base");
  assert_eq!(argv[2], "main");
  assert_eq!(argv[3], "--head");
  assert_eq!(argv[4], "feat/#42-tui-search");
  // The cwd must be the worktree path so the spawned tool sees the
  // selected branch's working tree as `.`. Use `paths_equal` so the
  // macOS `/private/var` ↔ `/var` symlink doesn't flake the assertion.
  assert!(
    common::paths_equal(&plan.cwd, dir.path()),
    "plan.cwd = {} vs dir = {}",
    plan.cwd.display(),
    dir.path().display()
  );
}

#[test]
fn prepare_review_respects_default_base_chain() {
  // `branch.<n>.gwm-base` (set by gwm create) must outrank
  // `[review].default_base`. Recorded as `main` by the fixture.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  // Manually record gwm-base since the test fixture didn't go through
  // worktree::add. The base chain reads from this key.
  gwm::launcher::write_gwm_base(&app.repo, "feat/#42-tui-search", "release-3.x").unwrap();
  app.config.review.command = Some("echo {base}".into());
  app.config.review.skip_when_no_changes = false;
  app.config.review.default_base = Some("trunk".into());

  let plan = app.prepare_review().expect("must resolve");
  assert_eq!(
    plan.expanded.argv,
    vec!["echo", "release-3.x"],
    "gwm-base must win over [review].default_base"
  );
}

#[test]
fn prepare_git_tui_default_uses_lazygit() {
  // Backwards-compat: no `[git_tui]` block ⇒ `l` still runs
  // `lazygit -p <path>` fullscreen.
  let (dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let plan = app.prepare_git_tui().expect("git_tui has a default");
  let argv = &plan.expanded.argv;
  assert_eq!(argv[0], "lazygit");
  assert_eq!(argv[1], "-p");
  // The path the launcher injects is the *currently selected* worktree;
  // make_app_on_branch only creates the main, so the default selection
  // is the main repo's path. Use `paths_equal` to dodge the macOS
  // `/private/var` ↔ `/var` mismatch (same trick as elsewhere in the
  // suite).
  assert!(
    common::paths_equal(std::path::Path::new(&argv[2]), dir.path()),
    "argv[2] = {} vs dir = {}",
    argv[2],
    dir.path().display()
  );
  assert!(plan.fullscreen, "lazygit defaults to fullscreen");
}

#[test]
fn prepare_git_tui_uses_user_command_when_set() {
  // [git_tui].command override must beat the lazygit default.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.config.git_tui.command = Some("gitui -d {path}".into());
  app.config.git_tui.fullscreen = Some(false);
  let plan = app.prepare_git_tui().expect("must resolve");
  assert_eq!(plan.expanded.argv[0], "gitui");
  assert_eq!(plan.expanded.argv[1], "-d");
  assert!(!plan.fullscreen, "user opted out of fullscreen");
}

#[test]
fn refresh_github_status_message_celebrates_full_success() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let issue = gwm::github::IssueStatus {
    number: 42,
    title: "x".into(),
    state: gwm::github::IssueState::Open,
    url: "https://example.test".into(),
    labels: vec![],
    updated_at: "".into(),
  };
  app.apply_issue_fetch_result(Ok(issue));
  app.report_github_refresh_status();
  assert!(
    app.status.to_lowercase().contains("refreshed") || app.status.to_lowercase().contains("ok"),
    "all-green refresh should signal success: {}",
    app.status
  );
}

// ---- Issue #73: lazygit-style color helpers --------------------------------
// Three pure functions live in `tui/ui.rs` and are re-exported through
// `tui/mod.rs` so the table-driven tests below can pin the contract.
// Visual regressions land here first, before showing up in screenshots.

#[test]
fn branch_name_color_codes_synced_branch_as_green() {
  let synced = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 0,
    behind: 0,
    unknown: false,
  };
  assert_eq!(branch_name_color(&synced, &Theme::default()), Color::Green);
}

#[test]
fn branch_name_color_codes_dirty_branch_as_red() {
  // Dirty = local working copy has uncommitted work. Lazygit doesn't surface
  // dirty in its branches view (it has a dedicated files view), but for a
  // worktree manager the most important signal is "this worktree has work
  // not yet captured anywhere" — red flags that hard.
  let dirty = BranchStatus {
    is_dirty: true,
    has_upstream: true,
    ahead: 0,
    behind: 0,
    unknown: false,
  };
  assert_eq!(branch_name_color(&dirty, &Theme::default()), Color::Red);
}

#[test]
fn branch_name_color_codes_ahead_or_behind_as_yellow() {
  let ahead = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 3,
    behind: 0,
    unknown: false,
  };
  let behind = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 0,
    behind: 2,
    unknown: false,
  };
  assert_eq!(branch_name_color(&ahead, &Theme::default()), Color::Yellow);
  assert_eq!(branch_name_color(&behind, &Theme::default()), Color::Yellow);
}

#[test]
fn branch_name_color_codes_unpublished_branch_as_magenta() {
  // No upstream + nothing dirty = branch exists locally only. Mirrors
  // lazygit's "?" magenta marker for RemoteBranchNotStoredLocally —
  // distinct from "synced" (green) and from "behind" (yellow) so the user
  // can tell at a glance whether they've pushed yet.
  let unpublished = BranchStatus {
    is_dirty: false,
    has_upstream: false,
    ahead: 0,
    behind: 0,
    unknown: false,
  };
  assert_eq!(branch_name_color(&unpublished, &Theme::default()), Color::Magenta);
}

#[test]
fn branch_name_color_codes_unknown_status_as_darkgray() {
  let unknown = BranchStatus {
    unknown: true,
    ..BranchStatus::default()
  };
  assert_eq!(branch_name_color(&unknown, &Theme::default()), Color::DarkGray);
}

#[test]
fn freshness_color_picks_green_for_recent_branches() {
  assert_eq!(freshness_color(Duration::from_secs(0), &Theme::default()), Color::Green);
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 3), &Theme::default()),
    Color::Green
  );
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 6 + 3600 * 23), &Theme::default()),
    Color::Green
  );
}

#[test]
fn freshness_color_picks_yellow_for_one_to_four_week_branches() {
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 7), &Theme::default()),
    Color::Yellow
  );
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 15), &Theme::default()),
    Color::Yellow
  );
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 29 + 3600 * 23), &Theme::default()),
    Color::Yellow
  );
}

#[test]
fn freshness_color_picks_darkgray_for_stale_branches() {
  // Branches older than a month read as "stale" — gwm encourages cleanup
  // via `gwm doctor`, so the colour reinforces the prompt.
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 30), &Theme::default()),
    Color::DarkGray
  );
  assert_eq!(
    freshness_color(Duration::from_secs(86_400 * 365), &Theme::default()),
    Color::DarkGray
  );
}

#[test]
fn pr_badge_color_maps_each_state_to_its_lazygit_palette() {
  // Mirrors `pkg/gui/presentation/branches.go::WithPrColor` — open=green,
  // draft=darkgray, merged=magenta, closed=red. The actual lazygit RGB
  // shades are slightly off-palette for terminal themes; we use the
  // 16-color names so the dots respect the user's colour scheme.
  assert_eq!(pr_badge_color(PrState::Open, &Theme::default()), Color::Green);
  assert_eq!(pr_badge_color(PrState::Draft, &Theme::default()), Color::DarkGray);
  assert_eq!(pr_badge_color(PrState::Merged, &Theme::default()), Color::Magenta);
  assert_eq!(pr_badge_color(PrState::Closed, &Theme::default()), Color::Red);
}

// Ensure the IssueState variants stay accessible — once `branch_name_color`
// and the rest land, the sidebar's badge function will need to fall back to
// an issue-derived colour when no PR is linked. The compile-time check below
// catches a stale import without polluting the runtime tests.
#[test]
fn issue_state_variants_compile() {
  let _ = IssueState::Open;
  let _ = IssueState::Closed;
}

// ---- Issue #73: configurable `o:` open key ---------------------------------
// `App::resolve_open_target` returns an `OpenTarget` that the event loop
// dispatches on (suspend-and-spawn for shell/editor, OS opener for finder).
// Pure resolution — no side effects, no spawn — so the test can pin the
// command resolution under every config / env combination.

use gwm::config::{TuiOpenConfig, TuiOpenMode};
use gwm::tui::OpenTarget;

#[test]
fn resolve_open_target_returns_none_when_nothing_selected() {
  let (_dir, mut app) = make_app();
  app.list_state.select(None);
  assert!(app.resolve_open_target().is_none());
}

#[test]
fn resolve_open_target_defaults_to_shell_mode() {
  // Default config (`mode = "shell"`) + a worktree selected → Shell variant
  // carrying the worktree path. The exact command depends on env / fallback;
  // here we only pin the variant + path so the test isn't $SHELL-dependent.
  let (_dir, app) = make_app();
  let target = app
    .resolve_open_target()
    .expect("main worktree should always be selectable");
  match target {
    OpenTarget::Shell { path, command } => {
      assert_eq!(path, app.worktrees[0].path);
      assert!(!command.is_empty(), "shell command must never be empty");
    }
    other => panic!("expected Shell variant, got {:?}", other),
  }
}

#[test]
fn resolve_open_target_honours_shell_cmd_override() {
  // `shell_cmd = "/usr/bin/fish"` must beat `$SHELL` — that's the whole
  // point of the override. Use a sentinel that's unlikely to be the
  // ambient shell to make the assertion deterministic.
  let (_dir, mut app) = make_app();
  app.config.tui.open = TuiOpenConfig {
    mode: TuiOpenMode::Shell,
    shell_cmd: Some("/sentinel/shell".into()),
    editor_cmd: None,
  };
  match app.resolve_open_target().unwrap() {
    OpenTarget::Shell { command, .. } => assert_eq!(command, "/sentinel/shell"),
    other => panic!("expected Shell, got {:?}", other),
  }
}

#[test]
fn resolve_open_target_uses_editor_mode_when_configured() {
  let (_dir, mut app) = make_app();
  app.config.tui.open = TuiOpenConfig {
    mode: TuiOpenMode::Editor,
    shell_cmd: None,
    editor_cmd: Some("hx".into()),
  };
  match app.resolve_open_target().unwrap() {
    OpenTarget::Editor { path, command } => {
      assert_eq!(path, app.worktrees[0].path);
      assert_eq!(command, "hx");
    }
    other => panic!("expected Editor, got {:?}", other),
  }
}

// ---- Issue #73: `y: yank` clipboard support -------------------------------
// `App::yank_selected_path` returns the path to push into the clipboard,
// or None when nothing's selected. The actual shell-out (pbcopy / wl-copy
// / xclip / clip) is tested manually — CI machines don't necessarily
// have any of these installed, so we keep the test scope to the pure
// resolution step and rely on the smoke test in the TUI for the rest.

#[test]
fn yank_selected_path_returns_path_for_selected_worktree() {
  let (_dir, app) = make_app();
  let path = app.yank_selected_path().expect("main worktree must be yankable");
  assert_eq!(path, app.worktrees[0].path);
}

#[test]
fn yank_selected_path_returns_none_when_nothing_selected() {
  let (_dir, mut app) = make_app();
  app.list_state.select(None);
  assert!(app.yank_selected_path().is_none());
}

// ---- Issue #283: table marker pastilles -----------------------------------
// The marker column (first cell) renders two slots separated by `/`: linked
// Issue/PR slots use a pastille, empty slots use `-`. The main worktree keeps
// its `★`. The table is the no-fetch read path until GitHub status has been
// fetched, at which point Issue/PR rows can carry their loaded state colours.

/// Pull each span's `(content, fg)` out of a `table_marker` line so the
/// pastille assertions read as a flat list.
fn marker_cells(line: &ratatui::text::Line<'_>) -> Vec<(String, Option<Color>)> {
  line
    .spans
    .iter()
    .map(|s| (s.content.as_ref().to_string(), s.style.fg))
    .collect()
}

#[test]
fn table_marker_for_main_worktree_is_a_yellow_star() {
  use gwm::github::BranchLink;
  let mut w = worktree_fixture("main");
  w.is_main = true;
  w.link = BranchLink::empty();
  let line = gwm::tui::table_marker(&w, &Theme::default());
  assert_eq!(marker_cells(&line), vec![("★".to_string(), Some(Color::Yellow))]);
}

#[test]
fn table_marker_paints_green_issue_and_violet_pr_pastilles() {
  use gwm::github::{BranchLink, LinkSource};
  let mut w = worktree_fixture("feat-1");
  w.is_main = false;
  w.link = BranchLink {
    issue: Some(42),
    pr: Some(43),
    issue_title: None,
    pr_title: None,
    issue_state: None,
    pr_state: None,
    issue_source: LinkSource::BranchName,
    pr_source: LinkSource::Detected,
  };
  let line = gwm::tui::table_marker(&w, &Theme::default());
  assert_eq!(
    marker_cells(&line),
    vec![
      ("●".to_string(), Some(Color::Green)),    // issue linked → clean role
      ("/".to_string(), Some(Color::DarkGray)), // muted separator
      ("●".to_string(), Some(Color::Magenta)),  // pr linked → locked role
    ]
  );
}

#[test]
fn table_marker_issue_only_leaves_the_pr_slot_as_dash() {
  use gwm::github::{BranchLink, LinkSource};
  let mut w = worktree_fixture("feat-1");
  w.is_main = false;
  w.link = BranchLink {
    issue: Some(42),
    pr: None,
    issue_title: None,
    pr_title: None,
    issue_state: None,
    pr_state: None,
    issue_source: LinkSource::BranchName,
    pr_source: LinkSource::None,
  };
  let line = gwm::tui::table_marker(&w, &Theme::default());
  let cells = marker_cells(&line);
  assert_eq!(cells[0].1, Some(Color::Green), "issue dot green");
  assert_eq!(cells[2].0, "-", "empty pr slot uses a dash");
  assert_eq!(cells[2].1, Some(Color::White), "empty pr dash white");
}

#[test]
fn table_marker_issue_pastille_uses_loaded_closed_issue_state() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let mut w = worktree_fixture("feat-1");
  w.branch = Some("feat/#42-tui-search".into());
  w.link = app.current_link().clone();
  app.worktrees = vec![w];
  app.list_state.select(Some(0));

  app.apply_issue_fetch_result(Ok(IssueStatus {
    number: 42,
    title: "Done".into(),
    state: IssueState::Closed,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  }));

  let theme = Theme::default();
  let line = gwm::tui::table_marker(&app.worktrees[0], &theme);
  let cells = marker_cells(&line);
  assert_eq!(
    cells[0].1,
    Some(gwm::tui::issue_badge_color(IssueState::Closed, &theme)),
    "closed issue dot should use the closed issue state colour"
  );
}

#[test]
fn table_marker_pr_pastille_uses_loaded_closed_pr_state() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  let mut w = worktree_fixture("feat-1");
  w.branch = Some("feat/#42-tui-search".into());
  w.link = gwm::github::BranchLink {
    issue: None,
    pr: Some(61),
    issue_title: None,
    pr_title: None,
    issue_state: None,
    pr_state: None,
    issue_source: LinkSource::None,
    pr_source: LinkSource::Explicit,
  };
  app.github.link = w.link.clone();
  app.worktrees = vec![w];
  app.list_state.select(Some(0));

  app.apply_pr_fetch_result(Ok(PrStatus {
    number: 61,
    title: "Closed".into(),
    state: PrState::Closed,
    url: String::new(),
    updated_at: String::new(),
    checks_passed: 0,
    checks_total: 0,
    ci: CiState::None,
    checks: vec![],
  }));

  let theme = Theme::default();
  let line = gwm::tui::table_marker(&app.worktrees[0], &theme);
  let cells = marker_cells(&line);
  assert_eq!(
    cells[2].1,
    Some(gwm::tui::pr_badge_color(PrState::Closed, &theme)),
    "closed PR dot should use the loaded PR state colour"
  );
}

#[test]
fn table_marker_uses_persisted_issue_and_pr_state_on_startup() {
  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feat/#42-tui-search", &head, false).unwrap();
  }
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  {
    let mut cfg = repo.config().unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-issue-state", "closed")
      .unwrap();
    cfg
      .set_str("branch.feat/#42-tui-search.gwm-pr-state", "closed")
      .unwrap();
  }
  repo.set_head("refs/heads/feat/#42-tui-search").unwrap();
  let app = App::new_at_layered(Some(dir.path()), None).unwrap();

  let theme = Theme::default();
  let mut listed = app.worktrees[0].clone();
  listed.is_main = false;
  let cells = marker_cells(&gwm::tui::table_marker(&listed, &theme));
  assert_eq!(
    cells[0].1,
    Some(gwm::tui::issue_badge_color(IssueState::Closed, &theme)),
    "issue marker should reuse persisted issue state after restart"
  );
  assert_eq!(
    cells[2].1,
    Some(gwm::tui::pr_badge_color(PrState::Closed, &theme)),
    "PR marker should reuse persisted PR state after restart"
  );
}

#[test]
fn table_marker_pr_only_leaves_the_issue_slot_as_dash() {
  use gwm::github::{BranchLink, LinkSource};
  let mut w = worktree_fixture("feat-1");
  w.is_main = false;
  w.link = BranchLink {
    issue: None,
    pr: Some(43),
    issue_title: None,
    pr_title: None,
    issue_state: None,
    pr_state: None,
    issue_source: LinkSource::None,
    pr_source: LinkSource::Detected,
  };
  let line = gwm::tui::table_marker(&w, &Theme::default());
  let cells = marker_cells(&line);
  assert_eq!(cells[0].0, "-", "empty issue slot uses a dash");
  assert_eq!(cells[0].1, Some(Color::White), "empty issue dash white");
  assert_eq!(cells[2].1, Some(Color::Magenta), "pr dot violet");
}

#[test]
fn table_marker_unlinked_non_main_is_two_white_dashes() {
  use gwm::github::BranchLink;
  let mut w = worktree_fixture("feat-1");
  w.is_main = false;
  w.link = BranchLink::empty();
  let line = gwm::tui::table_marker(&w, &Theme::default());
  let cells = marker_cells(&line);
  assert_eq!(cells[0].0, "-", "empty issue slot uses a dash");
  assert_eq!(cells[0].1, Some(Color::White), "empty issue dash white");
  assert_eq!(cells[1].0, "/", "muted separator between slots");
  assert_eq!(cells[2].0, "-", "empty pr slot uses a dash");
  assert_eq!(cells[2].1, Some(Color::White), "empty pr dash white");
}

#[test]
fn yank_candidates_for_current_platform_is_non_empty() {
  // Whatever the host OS, the candidate list must offer at least one
  // tool to try — empty would mean `y` could never succeed even with a
  // working pbcopy / xclip installed.
  assert!(
    !gwm::tui::clipboard_candidates().is_empty(),
    "clipboard candidates must include at least one tool for this OS"
  );
}

#[test]
fn resolve_open_target_uses_finder_mode_for_legacy_behaviour() {
  let (_dir, mut app) = make_app();
  app.config.tui.open = TuiOpenConfig {
    mode: TuiOpenMode::Finder,
    shell_cmd: None,
    editor_cmd: None,
  };
  match app.resolve_open_target().unwrap() {
    OpenTarget::Finder { path } => assert_eq!(path, app.worktrees[0].path),
    other => panic!("expected Finder, got {:?}", other),
  }
}

// ---- Sidebar sections (Option C — bordered subsections, no Commands block) ----

use gwm::tui::build_sidebar_sections;

fn detailed_worktree_fixture() -> WorktreeInfo {
  WorktreeInfo {
    name: "api-rest".into(),
    id: "api-rest".into(),
    path: PathBuf::from("/Users/test/cc-worktree/api-rest"),
    branch: Some("feat/#42-api-rest".into()),
    head: Some("08d1029f1234567890abcdef".into()),
    is_main: true,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus {
      is_dirty: false,
      has_upstream: true,
      ahead: 0,
      behind: 0,
      unknown: false,
    },
    link: gwm::github::BranchLink::empty(),
    issue_state: None,
    pr_state: None,
    age: None,
  }
}

fn section_text(lines: &[ratatui::text::Line<'static>]) -> String {
  lines
    .iter()
    .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
    .collect::<Vec<_>>()
    .join("")
}

#[test]
fn sidebar_sections_omit_commands_block() {
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    None,
    &Theme::default(),
  );
  let all = format!(
    "{}\n{}\n{}",
    section_text(&sections.worktree),
    section_text(&sections.working_tree),
    section_text(&sections.recent_commits),
  );
  assert!(
    !all.contains("Commands"),
    "the Commands cheat-sheet block must be removed (lives in ? help); got: {}",
    all
  );
  assert!(
    !all.contains("Bootstrap worktree"),
    "help-overlay phrasing must not leak into the sidebar: {}",
    all
  );
  assert!(
    !all.contains("Toggle this sidebar"),
    "help-overlay phrasing must not leak into the sidebar: {}",
    all
  );
}

#[test]
fn sidebar_sections_omit_inline_section_headers() {
  // The new layout puts section titles on the Block borders, so the inline
  // `Basic Settings:` / `Recent commits:` / `Working tree:` headers must
  // disappear from the content lines.
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    None,
    &Theme::default(),
  );
  let all = format!(
    "{}\n{}\n{}",
    section_text(&sections.worktree),
    section_text(&sections.working_tree),
    section_text(&sections.recent_commits),
  );
  assert!(!all.contains("Basic Settings:"), "got: {}", all);
  assert!(!all.contains("Recent commits:"), "got: {}", all);
  assert!(!all.contains("Working tree:"), "got: {}", all);
}

#[test]
fn sidebar_worktree_section_is_compact_identity() {
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    None,
    &Theme::default(),
  );
  let text = section_text(&sections.worktree);

  assert!(text.contains("api-rest"), "name on top line: {}", text);
  assert!(text.contains("feat/#42-api-rest"), "branch shown: {}", text);
  assert!(text.contains("08d1029"), "short head shown: {}", text);
  assert!(
    text.contains("synced") || text.contains("✓"),
    "synced state badge shown: {}",
    text
  );
  assert!(
    text.contains("main") || text.contains("★"),
    "main badge shown: {}",
    text
  );
}

#[test]
fn sidebar_worktree_section_short_enough_for_compact_layout() {
  // Compact identity block: name, branch · head, badges, path → 4 lines target.
  // Allow ≤5 to leave headroom for variable badges.
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    None,
    &Theme::default(),
  );
  assert!(
    sections.worktree.len() <= 5,
    "compact worktree block must stay ≤5 lines (target 4), got {}: {:?}",
    sections.worktree.len(),
    sections.worktree.iter().map(section_text_single).collect::<Vec<_>>()
  );
}

fn section_text_single(l: &ratatui::text::Line<'static>) -> String {
  l.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn sidebar_diff_line_renders_counts_in_theme_roles() {
  // A passed `DiffLineStat` surfaces a `Diff +<ins> -<del>` line whose
  // insertions wear the `untracked` role and deletions the `prunable`
  // role. Unique non-default `Rgb` values pin the wiring (a `Color::Green`
  // hardcode would pass against defaults but fail here — #170/#211 rule).
  let w = detailed_worktree_fixture();
  let theme = Theme {
    untracked: Color::Rgb(10, 20, 30),
    prunable: Color::Rgb(40, 50, 60),
    ..Theme::default()
  };
  let diff = gwm::worktree::DiffLineStat {
    insertions: 12,
    deletions: 4,
  };
  let sections = build_sidebar_sections(&w, gwm::tui::state::sidebar::SidebarMode::Commits, Some(diff), &theme);

  let diff_line = sections
    .worktree
    .iter()
    .find(|l| section_text_single(l).contains("Diff"))
    .expect("identity card must carry a Diff line when a stat is supplied");
  let ins = diff_line.spans.iter().find(|s| s.content.contains("+12")).unwrap();
  assert_eq!(
    ins.style.fg,
    Some(Color::Rgb(10, 20, 30)),
    "insertions wear `untracked`"
  );
  let del = diff_line.spans.iter().find(|s| s.content.contains("-4")).unwrap();
  assert_eq!(del.style.fg, Some(Color::Rgb(40, 50, 60)), "deletions wear `prunable`");
}

#[test]
fn sidebar_diff_line_absent_for_empty_or_missing_stat() {
  // No stat (`None`) and an all-zero stat both leave the card without a
  // Diff line — the `+0 -0` case is suppressed.
  let w = detailed_worktree_fixture();
  for diff in [None, Some(gwm::worktree::DiffLineStat::default())] {
    let sections = build_sidebar_sections(
      &w,
      gwm::tui::state::sidebar::SidebarMode::Commits,
      diff,
      &Theme::default(),
    );
    assert!(
      !sections
        .worktree
        .iter()
        .any(|l| section_text_single(l).contains("Diff")),
      "no Diff line should render for {diff:?}"
    );
  }
}

// ---- working_tree_status_line (issue #179, recoloured in #287) -------------
// The whole row is painted by the file's change category so it matches the
// Working Tree footer count it belongs to:
//   - created (`??` / `A`)     → green
//   - modified (`M`, `R`, …)   → yellow
//   - deleted (`D`)            → red
// (The pre-#287 staged-vs-worktree cyan `X`-column split is gone.)
use gwm::tui::working_tree_status_line;

fn filename_span_fg(line: &ratatui::text::Line<'static>, needle: &str) -> Option<Color> {
  line
    .spans
    .iter()
    .find(|s| s.content.contains(needle))
    .unwrap_or_else(|| panic!("no span carrying {:?} in {:?}", needle, section_text_single(line)))
    .style
    .fg
}

#[test]
fn working_tree_status_line_preserves_raw_text() {
  // Only Span styling is added — the rendered text must read back
  // byte-for-byte identical to the raw `git status --short` line.
  for raw in [
    "A  staged.rs",
    "AM both.rs",
    " M tracked.rs",
    " D gone.rs",
    "?? untracked.rs",
    "R  old.rs -> new.rs",
  ] {
    assert_eq!(
      section_text_single(&working_tree_status_line(raw, &Theme::default())),
      raw,
      "raw text preserved for {:?}",
      raw
    );
  }
}

#[test]
fn working_tree_status_line_added_is_green() {
  // `A` (added / staged) is a *created* file → green, status code + name.
  let line = working_tree_status_line("A  staged.rs", &Theme::default());
  assert_eq!(line.spans[0].style.fg, Some(Color::Green), "added code → green");
  assert_eq!(
    filename_span_fg(&line, "staged.rs"),
    Some(Color::Green),
    "added filename → green"
  );
}

#[test]
fn working_tree_status_line_modified_is_yellow() {
  let line = working_tree_status_line(" M tracked.rs", &Theme::default());
  assert_eq!(line.spans[0].style.fg, Some(Color::Yellow), "modified code → yellow");
  assert_eq!(
    filename_span_fg(&line, "tracked.rs"),
    Some(Color::Yellow),
    "modified filename → yellow"
  );
}

#[test]
fn working_tree_status_line_deleted_is_red() {
  // `D` (deleted) → red, matching the footer's deleted segment (issue #287).
  for raw in ["D  gone.rs", " D gone.rs"] {
    let line = working_tree_status_line(raw, &Theme::default());
    assert_eq!(line.spans[0].style.fg, Some(Color::Red), "deleted code → red: {raw:?}");
    assert_eq!(
      filename_span_fg(&line, "gone.rs"),
      Some(Color::Red),
      "deleted filename → red: {raw:?}"
    );
  }
}

#[test]
fn working_tree_status_line_untracked_is_green() {
  let line = working_tree_status_line("?? untracked.rs", &Theme::default());
  assert_eq!(line.spans[0].style.fg, Some(Color::Green), "untracked code → green");
  assert_eq!(
    filename_span_fg(&line, "untracked.rs"),
    Some(Color::Green),
    "untracked filename → green"
  );
}

#[test]
fn working_tree_status_line_handles_multibyte_leading_chars() {
  // The helper is `pub` (exported for these tests), so a non-git caller can
  // feed it arbitrary input. Splitting on byte offsets (`raw[0..1]`) would
  // panic mid-codepoint when the first chars are multi-byte UTF-8. Split on
  // char boundaries instead — no panic, and the exact text is preserved.
  let raw = "éM café.rs"; // X='é' (2 bytes), Y='M', sep=' ', path="café.rs"
  let line = working_tree_status_line(raw, &Theme::default());
  assert_eq!(
    section_text_single(&line),
    raw,
    "multi-byte text preserved without panic"
  );
}

#[test]
fn working_tree_status_line_added_then_modified_is_green() {
  // `AM`: created wins over a later modification (precedence created >
  // deleted > modified), so the whole row is green — matching the bucket
  // the file is counted in.
  let line = working_tree_status_line("AM both.rs", &Theme::default());
  assert_eq!(line.spans[0].style.fg, Some(Color::Green), "AM (created wins) → green");
  assert_eq!(
    filename_span_fg(&line, "both.rs"),
    Some(Color::Green),
    "AM filename → green"
  );
}

#[test]
fn sidebar_worktree_section_skips_irrelevant_badges() {
  // A non-main, unlocked, non-prunable worktree should NOT advertise
  // those flags — only the ones that are true add visual noise.
  let mut w = detailed_worktree_fixture();
  w.is_main = false;
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    None,
    &Theme::default(),
  );
  let text = section_text(&sections.worktree);
  assert!(
    !text.contains("★ main"),
    "non-main worktree must not show ★ main: {}",
    text
  );
  assert!(
    !text.contains("locked"),
    "unlocked worktree must not show locked badge: {}",
    text
  );
  assert!(
    !text.contains("prunable"),
    "non-prunable worktree must not show prunable badge: {}",
    text
  );
}

#[test]
fn sidebar_worktree_badge_uses_divergence_sigil_when_ahead() {
  // Regression: PR #70 review (Copilot) flagged that a non-dirty/non-unknown
  // status always rendered `✓ <label>`, which produced misleading badges like
  // `✓ ↑2` for branches that were *ahead* of upstream. The `✓` is reserved
  // for synced / clean — divergence must use `↑` / `↓` / `⇅` (or no sigil).
  let mut w = detailed_worktree_fixture();
  w.status = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 2,
    behind: 0,
    unknown: false,
  };
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    None,
    &Theme::default(),
  );
  let badge = section_text_single(&sections.worktree[2]);
  assert!(
    !badge.contains("✓"),
    "ahead-only branch must not display the synced/clean ✓ sigil: {}",
    badge
  );
  assert!(badge.contains("↑2"), "ahead label must still be visible: {}", badge);
}

#[test]
fn sidebar_worktree_badge_uses_divergence_sigil_when_behind() {
  let mut w = detailed_worktree_fixture();
  w.status = BranchStatus {
    is_dirty: false,
    has_upstream: true,
    ahead: 0,
    behind: 3,
    unknown: false,
  };
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    None,
    &Theme::default(),
  );
  let badge = section_text_single(&sections.worktree[2]);
  assert!(
    !badge.contains("✓"),
    "behind-only branch must not display the synced/clean ✓ sigil: {}",
    badge
  );
  assert!(badge.contains("↓3"), "behind label must still be visible: {}", badge);
}

#[test]
fn sidebar_worktree_badge_keeps_check_sigil_when_synced() {
  // Sanity: the fixture has `has_upstream=true, ahead=0, behind=0` so the
  // synced label *should* still display `✓`. Guards against an over-eager
  // fix that would drop the sigil everywhere.
  let w = detailed_worktree_fixture();
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    None,
    &Theme::default(),
  );
  let badge = section_text_single(&sections.worktree[2]);
  assert!(badge.contains("✓"), "synced branch must keep the ✓ sigil: {}", badge);
  assert!(badge.contains("synced"), "label must still say synced: {}", badge);
}

// ---- tilde_compress path-boundary safety (PR #70 review, Copilot) -------

use gwm::tui::tilde_compress_with_home;

#[test]
fn tilde_compress_does_not_slice_across_path_boundaries() {
  // Regression: PR #70 review (Copilot) flagged that string `strip_prefix`
  // on the rendered home dir would slice into longer directory names that
  // merely *start with* the same characters — e.g. home `/home/al` would
  // turn `/home/alice/repo` into `~ice/repo`. The compression must only
  // fire when the prefix ends at a path separator boundary.
  let home = std::path::Path::new("/home/al");
  assert_eq!(
    tilde_compress_with_home("/home/alice/repo", home),
    "/home/alice/repo",
    "must not slice across the `alice` directory name"
  );
}

#[test]
fn tilde_compress_compresses_exact_home_match() {
  let home = std::path::Path::new("/home/alice");
  assert_eq!(tilde_compress_with_home("/home/alice", home), "~");
  assert_eq!(tilde_compress_with_home("/home/alice/repo", home), "~/repo");
  assert_eq!(tilde_compress_with_home("/home/alice/repo/sub", home), "~/repo/sub");
}

#[test]
fn tilde_compress_falls_back_when_path_outside_home() {
  let home = std::path::Path::new("/home/alice");
  assert_eq!(tilde_compress_with_home("/var/log/x", home), "/var/log/x");
  // Sibling directory starting with the same letters → no match.
  assert_eq!(tilde_compress_with_home("/home/alicent/x", home), "/home/alicent/x");
}

// ---- Issue / PR summary line width budgeting ----------------------------

use gwm::tui::{issue_summary_line, pr_summary_line};

fn line_visible_width(line: &ratatui::text::Line<'static>) -> usize {
  line.spans.iter().map(|s| s.content.chars().count()).sum()
}

#[test]
fn github_status_idle_body_does_not_render_fetch_prompt() {
  // The fetch affordance lives in the pane title (`Issue / PR [F]`) and in
  // statusbar/modal hints, not as a body row competing with issue/PR data.
  let (_dir, _repo, app) = make_app_on_branch("feat/#42-tui-search");
  let lines = gwm::tui::github_status_lines(&app, 80);
  let text: String = lines
    .iter()
    .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref().to_string()))
    .collect::<Vec<_>>()
    .join(" ");
  assert!(
    !text.contains("press "),
    "fetch prompt should not render inside the Issue/PR body: {text}"
  );
}

#[test]
fn github_status_no_link_hint_uses_the_real_link_prompt_chord() {
  // Regression (#290 keymap redesign): the "no link" hint hardcoded `L`,
  // which pre-#290 opened the link menu but is now LazyGitFullscreen. The
  // hint must derive LinkPrompt's real chord (`i`) from the keymap so it
  // never drifts from the binding (and tracks `[tui.keys]` overrides).
  // A branch with no `#N` auto-detects no issue, so the link is empty and
  // the no-link row renders.
  let (_dir, _repo, app) = make_app_on_branch("scratch");
  let text: String = gwm::tui::github_status_lines(&app, 120)
    .iter()
    .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref().to_string()))
    .collect::<Vec<_>>()
    .join("");
  assert!(text.contains("no link"), "the no-link hint should render: {text}");
  assert!(
    text.contains("press i to link"),
    "hint must use LinkPrompt's real chord, not the stale `L`: {text}"
  );
}

#[test]
fn github_status_loading_uses_the_animated_spinner_frame() {
  use gwm::tui::FetchKey;
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.github.mark_loading(FetchKey::Issue(42));

  let first = gwm::tui::github_status_lines(&app, 80)
    .into_iter()
    .map(|line| spans_to_text(&line.spans))
    .collect::<Vec<_>>()
    .join("\n");
  app.spinner.tick();
  let second = gwm::tui::github_status_lines(&app, 80)
    .into_iter()
    .map(|line| spans_to_text(&line.spans))
    .collect::<Vec<_>>()
    .join("\n");

  assert!(first.contains("loading"), "loading label missing: {first:?}");
  assert_ne!(first, second, "loading rows should animate with the App spinner");
}

#[test]
fn issue_summary_line_truncates_loaded_state_to_budget() {
  // Regression: with a 48-column sidebar, a fully-loaded issue line was
  // `#67 (auto) [open] <40-char title>` ≈ 58 chars, overflowing the
  // Issue/PR block and forcing ratatui's `Wrap` to push the title onto a
  // second visual row that the layout's `Constraint::Length` didn't
  // budget for. The Loaded variant must keep the head + badge prefix
  // intact and trim the title so the total ≤ max_width.
  let status = gwm::github::IssueStatus {
    number: 828,
    title:
      "Stats: subscriptions distribution across schools and individual customers (very long title to force truncation)"
        .into(),
    state: gwm::github::IssueState::Open,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  };
  let line = issue_summary_line(
    828,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Loaded(status),
    30,
    &Theme::default(),
  );
  let width = line_visible_width(&line);
  assert!(
    width <= 30,
    "loaded issue line must fit in 30 cols, got {}: {:?}",
    width,
    line.spans.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>()
  );
  // Ellipsis confirms the title was actually trimmed (not just clipped).
  let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(joined.ends_with('…'), "expected trailing ellipsis: {}", joined);
}

#[test]
fn pr_summary_line_truncates_loaded_state_to_budget() {
  let status = gwm::github::PrStatus {
    number: 70,
    title: "feat(tui): redesign Details sidebar with bordered subsections and four cards".into(),
    state: gwm::github::PrState::Open,
    url: String::new(),
    checks_passed: 3,
    checks_total: 3,
    ci: CiState::Passing,
    checks: vec![],
    updated_at: String::new(),
  };
  let line = pr_summary_line(
    70,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Loaded(status),
    35,
    &Theme::default(),
    None,
  );
  let width = line_visible_width(&line);
  assert!(
    width <= 35,
    "loaded PR line must fit in 35 cols, got {}: {:?}",
    width,
    line.spans.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>()
  );
}

#[test]
fn issue_summary_line_keeps_short_title_intact() {
  // Sanity: budget large enough → no truncation, no spurious ellipsis.
  let status = gwm::github::IssueStatus {
    number: 1,
    title: "short".into(),
    state: gwm::github::IssueState::Open,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  };
  let line = issue_summary_line(
    1,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Loaded(status),
    80,
    &Theme::default(),
  );
  let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    joined.contains("short"),
    "short title must not be truncated: {}",
    joined
  );
  assert!(
    !joined.contains('…'),
    "no ellipsis when budget exceeds content: {}",
    joined
  );
}

#[test]
fn issue_summary_line_truncates_error_state_to_budget() {
  let line = issue_summary_line(
    42,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Error(
      "gh: API rate limit exceeded for user, retry after 60s with exponential backoff please".into(),
    ),
    30,
    &Theme::default(),
  );
  let width = line_visible_width(&line);
  assert!(width <= 30, "error line must fit in 30 cols, got {}", width);
}

// ---- Issue #283: pane icons + source / state chips ----------------------

fn span_with<'a>(line: &'a ratatui::text::Line<'a>, needle: &str) -> Option<&'a ratatui::text::Span<'a>> {
  line.spans.iter().find(|s| s.content.contains(needle))
}

#[test]
fn issue_summary_line_leads_with_the_issue_icon() {
  let line = issue_summary_line(
    7,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Idle,
    80,
    &Theme::default(),
  );
  assert!(
    line.spans[0].content.contains(gwm::tui::ISSUE_ICON),
    "issue pane line must lead with the issue nerdfont glyph: {:?}",
    line.spans[0].content
  );
}

#[test]
fn issue_summary_line_icon_has_trailing_space_only() {
  let line = issue_summary_line(
    7,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Idle,
    80,
    &Theme::default(),
  );
  assert_eq!(
    line.spans[0].content.as_ref(),
    format!("{}  ", gwm::tui::ISSUE_ICON),
    "issue icon segment should leave two spaces after the glyph only"
  );
}

#[test]
fn issue_summary_line_loaded_icon_uses_issue_state_color() {
  let status = gwm::github::IssueStatus {
    number: 7,
    title: "x".into(),
    state: gwm::github::IssueState::Closed,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  };
  let theme = Theme::default();
  let line = issue_summary_line(
    7,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Loaded(status),
    80,
    &theme,
  );
  assert_eq!(
    line.spans[0].style.fg,
    Some(gwm::tui::issue_badge_color(gwm::github::IssueState::Closed, &theme)),
    "loaded issue icon should reuse the issue state badge role"
  );
}

#[test]
fn issue_summary_line_idle_icon_stays_muted() {
  let theme = Theme::default();
  let line = issue_summary_line(
    7,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Idle,
    80,
    &theme,
  );
  assert_eq!(
    line.spans[0].style.fg,
    Some(theme.muted),
    "idle issue icon stays neutral"
  );
}

#[test]
fn pr_summary_line_leads_with_the_pr_icon() {
  let status = gwm::github::PrStatus {
    number: 9,
    title: "x".into(),
    state: gwm::github::PrState::Open,
    url: String::new(),
    checks_passed: 0,
    checks_total: 0,
    ci: CiState::None,
    checks: vec![],
    updated_at: String::new(),
  };
  let line = pr_summary_line(
    9,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Loaded(status),
    80,
    &Theme::default(),
    None,
  );
  assert!(
    line.spans[0].content.contains(gwm::tui::PR_ICON),
    "pr pane line must lead with the pr nerdfont glyph: {:?}",
    line.spans[0].content
  );
}

#[test]
fn pr_summary_line_icon_has_trailing_space_only() {
  let line = pr_summary_line(
    9,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Idle,
    80,
    &Theme::default(),
    None,
  );
  assert_eq!(
    line.spans[0].content.as_ref(),
    format!("{}  ", gwm::tui::PR_ICON),
    "PR icon segment should leave two spaces after the glyph only"
  );
}

#[test]
fn pr_summary_line_loaded_icon_uses_pr_state_color() {
  let status = gwm::github::PrStatus {
    number: 9,
    title: "x".into(),
    state: gwm::github::PrState::Merged,
    url: String::new(),
    checks_passed: 0,
    checks_total: 0,
    ci: CiState::None,
    checks: vec![],
    updated_at: String::new(),
  };
  let theme = Theme::default();
  let line = pr_summary_line(
    9,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Loaded(status),
    80,
    &theme,
    None,
  );
  assert_eq!(
    line.spans[0].style.fg,
    Some(gwm::tui::pr_badge_color(gwm::github::PrState::Merged, &theme)),
    "loaded PR icon should reuse the PR state badge role"
  );
}

#[test]
fn pr_summary_line_loaded_renders_ci_indicator_when_checks_present() {
  // Issue #299: a PR with a failing rollup must surface a coloured CI
  // indicator (icon + label + N/M), not just the bare count.
  let status = gwm::github::PrStatus {
    number: 9,
    title: "x".into(),
    state: gwm::github::PrState::Open,
    url: String::new(),
    checks_passed: 1,
    checks_total: 2,
    ci: gwm::github::CiState::Failing,
    checks: vec![],
    updated_at: String::new(),
  };
  let theme = Theme::default();
  let line = pr_summary_line(
    9,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Loaded(status),
    80,
    &theme,
    None,
  );
  let ci = span_with(&line, "CI").expect("a CI indicator span");
  assert!(
    ci.content.contains("failing") && ci.content.contains("1/2"),
    "CI indicator must carry the failing label and count, got {:?}",
    ci.content
  );
  assert_eq!(
    ci.style.fg,
    Some(theme.prunable),
    "a failing CI indicator must paint with the prunable (red) role"
  );
}

#[test]
fn pr_summary_line_loaded_omits_ci_indicator_when_no_checks() {
  let status = gwm::github::PrStatus {
    number: 9,
    title: "x".into(),
    state: gwm::github::PrState::Open,
    url: String::new(),
    checks_passed: 0,
    checks_total: 0,
    ci: gwm::github::CiState::None,
    checks: vec![],
    updated_at: String::new(),
  };
  let line = pr_summary_line(
    9,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Loaded(status),
    80,
    &Theme::default(),
    None,
  );
  let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    !joined.contains("CI"),
    "a PR with no checks must not render any CI indicator: {}",
    joined
  );
}

#[test]
fn ci_indicator_maps_states_to_status_roles() {
  let theme = Theme::default();
  // None renders nothing.
  assert!(gwm::tui::ci_indicator(gwm::github::CiState::None, 0, 0, &theme).is_none());
  // Passing → clean (green), failing → prunable (red), running → dirty (yellow).
  let (txt, col) = gwm::tui::ci_indicator(gwm::github::CiState::Passing, 9, 9, &theme).unwrap();
  assert!(txt.contains("passing") && txt.contains("9/9"));
  assert_eq!(col, theme.clean);
  let (txt, col) = gwm::tui::ci_indicator(gwm::github::CiState::Failing, 7, 9, &theme).unwrap();
  assert!(txt.contains("failing") && txt.contains("7/9"));
  assert_eq!(col, theme.prunable);
  let (txt, col) = gwm::tui::ci_indicator(gwm::github::CiState::Running, 8, 9, &theme).unwrap();
  assert!(txt.contains("running") && txt.contains("8/9"));
  assert_eq!(col, theme.dirty);
}

#[test]
fn pr_summary_line_renders_detected_source_as_a_reverse_video_chip() {
  use ratatui::style::Modifier;
  let status = gwm::github::PrStatus {
    number: 9,
    title: "x".into(),
    state: gwm::github::PrState::Open,
    url: String::new(),
    checks_passed: 0,
    checks_total: 0,
    ci: CiState::None,
    checks: vec![],
    updated_at: String::new(),
  };
  let line = pr_summary_line(
    9,
    gwm::github::LinkSource::Detected,
    &GitHubFetchState::Loaded(status),
    80,
    &Theme::default(),
    None,
  );
  let chip = span_with(&line, "detected").expect("a 'detected' source chip span");
  assert!(
    chip.style.add_modifier.contains(Modifier::REVERSED),
    "the source chip must use the version-badge reverse-video treatment"
  );
}

#[test]
fn issue_summary_line_renders_auto_source_as_a_reverse_video_chip() {
  use ratatui::style::Modifier;
  let line = issue_summary_line(
    7,
    gwm::github::LinkSource::BranchName,
    &GitHubFetchState::Idle,
    80,
    &Theme::default(),
  );
  let chip = span_with(&line, "auto").expect("an 'auto' source chip span");
  assert!(
    chip.style.add_modifier.contains(Modifier::REVERSED),
    "the source chip must use the version-badge reverse-video treatment"
  );
}

#[test]
fn explicit_link_renders_no_source_chip() {
  let line = issue_summary_line(
    7,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Idle,
    80,
    &Theme::default(),
  );
  let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    !text.contains("auto"),
    "explicit link must not show an auto chip: {text}"
  );
  assert!(
    !text.contains("detected"),
    "explicit link must not show a detected chip: {text}"
  );
}

#[test]
fn issue_summary_line_state_badge_is_a_reverse_video_chip() {
  use ratatui::style::Modifier;
  let status = gwm::github::IssueStatus {
    number: 7,
    title: "x".into(),
    state: gwm::github::IssueState::Open,
    url: String::new(),
    labels: vec![],
    updated_at: String::new(),
  };
  let line = issue_summary_line(
    7,
    gwm::github::LinkSource::Explicit,
    &GitHubFetchState::Loaded(status),
    80,
    &Theme::default(),
  );
  let chip = span_with(&line, "open").expect("an 'open' state chip span");
  assert!(
    chip.style.add_modifier.contains(Modifier::REVERSED),
    "the state badge must use the version-badge reverse-video treatment"
  );
}

// ---- Recent Commits panel: lazygit-style fill + clip (issue #71) ---------

use gwm::tui::{recent_commits_lines, RECENT_COMMITS_LIMIT};

fn add_commits(repo: &git2::Repository, count: usize) {
  use git2::Signature;
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  for i in 0..count {
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo
      .commit(Some("HEAD"), &sig, &sig, &format!("commit-{}", i), &tree, &[&parent])
      .unwrap();
  }
}

fn worktree_pointing_at_dir(dir: &std::path::Path) -> WorktreeInfo {
  WorktreeInfo {
    name: "test".into(),
    id: "test".into(),
    path: dir.to_path_buf(),
    branch: Some("main".into()),
    head: None,
    is_main: true,
    is_locked: false,
    is_prunable: false,
    status: BranchStatus::default(),
    link: gwm::github::BranchLink::empty(),
    issue_state: None,
    pr_state: None,
    age: None,
  }
}

#[test]
fn recent_commits_lines_respects_limit_when_repo_has_more() {
  let (dir, repo) = init_repo();
  add_commits(&repo, 14); // 15 total commits (1 seed + 14)
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 5, &Theme::default());
  assert_eq!(
    lines.len(),
    5,
    "limit=5 must produce exactly 5 lines, got {}",
    lines.len()
  );
}

#[test]
fn recent_commits_lines_returns_all_when_under_limit() {
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 100, &Theme::default());
  assert_eq!(
    lines.len(),
    1,
    "init_repo has 1 commit, asking for 100 should still return 1, got {}",
    lines.len()
  );
}

#[test]
fn recent_commits_lines_reuses_cached_rows_for_unchanged_head() {
  let (dir, repo) = init_repo();
  add_commits(&repo, 3);
  let mut w = worktree_pointing_at_dir(dir.path());
  w.head = Some(repo.head().unwrap().target().unwrap().to_string());

  let first = recent_commits_lines(&w, 4, &Theme::default());
  let first_text: Vec<String> = first
    .iter()
    .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect())
    .collect();
  drop(repo);

  std::fs::rename(dir.path().join(".git"), dir.path().join(".git.hidden")).unwrap();
  let second = recent_commits_lines(&w, 4, &Theme::default());
  let second_text: Vec<String> = second
    .iter()
    .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect())
    .collect();

  assert_eq!(
    second_text, first_text,
    "unchanged head+limit should reuse cached recent commit rows instead of re-reading the repo"
  );
}

#[test]
fn recent_commits_cache_is_scoped_to_worktree_path() {
  let (dir, repo) = init_repo();
  let mut cached = worktree_pointing_at_dir(dir.path());
  cached.head = Some(repo.head().unwrap().target().unwrap().to_string());

  let first = recent_commits_lines(&cached, 1, &Theme::default());
  let first_text: String = first[0].spans.iter().map(|span| span.content.as_ref()).collect();

  let other = tempfile::TempDir::new().unwrap();
  let mut same_oid_different_path = worktree_pointing_at_dir(other.path());
  same_oid_different_path.head = cached.head.clone();

  let second = recent_commits_lines(&same_oid_different_path, 1, &Theme::default());
  let second_text: String = second[0].spans.iter().map(|span| span.content.as_ref()).collect();

  assert!(
    second_text.starts_with("! "),
    "same OID in a different worktree path must miss the cache, got: {}",
    second_text
  );
  assert_ne!(
    second_text, first_text,
    "recent commit cache must not leak rows across repositories that share an OID"
  );
}

#[test]
#[allow(clippy::assertions_on_constants)] // intentional const pin
fn recent_commits_default_limit_fills_modern_terminal_heights() {
  // Regression: the previous hardcoded limit of 10 left the bottom of tall
  // sidebars empty. On a 50-line terminal, the Recent Commits block gets
  // ~12–18 rows after the small fixed sections take their slice; on a
  // 100-line terminal it gets ~70+. Keep the default generous so the
  // block fills the panel without re-shelling git on scroll. A future
  // contributor that lowers the constant will trip this test.
  assert!(
    RECENT_COMMITS_LIMIT >= 50,
    "RECENT_COMMITS_LIMIT must be ≥ 50 to fill a typical sidebar, got {}",
    RECENT_COMMITS_LIMIT
  );
}

#[test]
fn build_sidebar_sections_fetches_up_to_default_recent_commits_limit() {
  // Wire-up smoke: build_sidebar_sections must use RECENT_COMMITS_LIMIT (or
  // higher) so the cached section is dense enough to fill a tall panel.
  let (dir, repo) = init_repo();
  add_commits(&repo, 30); // 31 total commits
  let w = worktree_pointing_at_dir(dir.path());
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    None,
    &Theme::default(),
  );
  assert_eq!(
    sections.recent_commits.len(),
    31,
    "expected all 31 commits to be cached (default limit ≥ 50 ≥ 31), got {}",
    sections.recent_commits.len()
  );
}

// ---- lazygit-style row format (hash + initials + subject) ----------------

use gwm::tui::{author_initials, COMMIT_HASH_DISPLAY_LEN};

#[test]
fn author_initials_two_word_name_picks_first_letters_of_each() {
  assert_eq!(author_initials("Kylian Bardini"), "KB");
  assert_eq!(author_initials("Jesse Duffield"), "JD");
}

#[test]
fn author_initials_single_word_takes_first_two_chars() {
  assert_eq!(author_initials("Linus"), "Li");
  assert_eq!(author_initials("kb"), "kb");
}

#[test]
fn author_initials_three_or_more_words_only_uses_first_two() {
  // Lazygit caps the result at 2 chars regardless of token count.
  assert_eq!(author_initials("Jean-Paul Marie Dupont"), "JM");
}

#[test]
fn author_initials_strips_leading_whitespace() {
  assert_eq!(author_initials("  Kylian Bardini"), "KB");
}

#[test]
fn author_initials_empty_returns_empty() {
  assert_eq!(author_initials(""), "");
  assert_eq!(author_initials("   "), "");
}

#[test]
fn author_initials_takes_first_unicode_scalar_per_token() {
  // Documented divergence from lazygit (PR #72 Copilot review): gwm's
  // `author_initials` uses `str::chars()` and slices on Unicode scalar
  // values, NOT grapheme clusters. Single-scalar emoji ("🦀") survive
  // intact, but multi-scalar grapheme clusters like the French flag
  // "🇫🇷" (two regional indicators) are split — only the first scalar
  // makes it into the initials. Pinning this so a future contributor
  // doesn't quietly break it by adopting a grapheme-aware crate.
  assert_eq!(author_initials("🦀 Crab"), "🦀C");
  assert_eq!(
    author_initials("🇫🇷 Bardini"),
    "🇫B",
    "two-scalar grapheme cluster (flag) is intentionally split per scalar"
  );
}

// ---- Graph node markers (○ commit, ◎ merge) -----------------------------

fn add_merge_commit(repo: &git2::Repository) -> git2::Oid {
  use git2::Signature;
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();

  // Start from current HEAD. Create a sibling branch with one commit, then
  // merge it back into HEAD with a true 2-parent merge commit.
  let base = repo.head().unwrap().peel_to_commit().unwrap();
  let branch_name = "tmp-side";
  repo.branch(branch_name, &base, false).unwrap();
  // Side commit.
  let side_oid = {
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo
      .commit(
        Some(&format!("refs/heads/{}", branch_name)),
        &sig,
        &sig,
        "side",
        &tree,
        &[&base],
      )
      .unwrap()
  };
  let side = repo.find_commit(side_oid).unwrap();
  // Merge commit on HEAD with two parents.
  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  repo
    .commit(
      Some("HEAD"),
      &sig,
      &sig,
      "merge: side into trunk",
      &tree,
      &[&base, &side],
    )
    .unwrap()
}

#[test]
fn commit_row_carries_parent_hashes() {
  // Regression: the renderer needs the parent count to pick ○ vs ◎.
  // git_log_with_author must surface the parent list, not flatten it.
  let (dir, repo) = init_repo();
  add_merge_commit(&repo); // adds two extra commits (side + merge)
  let rows = gwm::worktree::git_log_with_author(dir.path(), 10).unwrap();
  // First row = merge commit (HEAD), should have 2 parents.
  assert!(!rows.is_empty(), "expected at least 1 commit");
  assert_eq!(
    rows[0].parents.len(),
    2,
    "HEAD is a merge commit and must surface both parents, got {:?}",
    rows[0].parents
  );
  // The seed `init` commit has no parent.
  let seed = rows
    .iter()
    .find(|r| r.subject == "init")
    .expect("seed commit must be in log");
  assert!(
    seed.parents.is_empty(),
    "seed commit has no parents, got {:?}",
    seed.parents
  );
}

#[test]
fn recent_commits_line_marks_merge_commit_with_bullseye() {
  // U+25CE ◎ is named "BULLSEYE" in Unicode — it is what lazygit uses
  // for `MergeSymbol`. The previous test name said "diamond" which
  // was geometrically wrong; this name pins the actual glyph.
  let (dir, repo) = init_repo();
  add_merge_commit(&repo);
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 10, &Theme::default());
  // Find the merge commit row by subject; assert it carries ◎ somewhere.
  let merge = lines
    .iter()
    .find(|l| {
      let joined: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
      joined.contains("merge: side into trunk")
    })
    .expect("merge row must be present");
  let joined: String = merge.spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    joined.contains('◎'),
    "merge row must carry the ◎ bullseye marker, got: {}",
    joined
  );
}

// ---- Commit graph topology (lazygit port — pipes + connectors) ---------

use gwm::tui::commit_graph::{box_drawing_chars, build_pipe_sets, render_commits, render_pipe_set, test_row, PipeKind};

fn spans_to_text(spans: &[ratatui::text::Span<'static>]) -> String {
  spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
#[allow(clippy::type_complexity)]
fn graph_glyph_table_matches_lazygit_truth_table() {
  // 16-case ground truth ported verbatim from
  // `lazygit/pkg/gui/presentation/graph/cell.go::getBoxDrawingChars`.
  // If any of these flip, the entire graph rendering will silently shift.
  let cases: &[((bool, bool, bool, bool), (char, char))] = &[
    ((true, true, true, true), ('│', '─')),
    ((true, true, true, false), ('│', ' ')),
    ((true, true, false, true), ('│', '─')),
    ((true, true, false, false), ('│', ' ')),
    ((true, false, true, true), ('┴', '─')),
    ((true, false, true, false), ('╯', ' ')),
    ((true, false, false, true), ('╰', '─')),
    ((true, false, false, false), ('╵', ' ')),
    ((false, true, true, true), ('┬', '─')),
    ((false, true, true, false), ('╮', ' ')),
    ((false, true, false, true), ('╭', '─')),
    ((false, true, false, false), ('╷', ' ')),
    ((false, false, true, true), ('─', '─')),
    ((false, false, true, false), ('─', ' ')),
    ((false, false, false, true), ('╶', '─')),
    ((false, false, false, false), (' ', ' ')),
  ];
  for &((u, d, l, r), expected) in cases {
    assert_eq!(
      box_drawing_chars(u, d, l, r),
      expected,
      "case ({}, {}, {}, {}) — expected {:?}",
      u,
      d,
      l,
      r,
      expected
    );
  }
}

#[test]
fn graph_linear_history_emits_single_column_circles() {
  // Three commits, each pointing at the next: c (parent b) → b (parent a) → a (no parent).
  let rows = vec![test_row("c", &["b"]), test_row("b", &["a"]), test_row("a", &[])];
  let graphs = render_commits(&rows, &Theme::default());
  assert_eq!(graphs.len(), 3);
  // Each row should be a 2-cell render (one column → 2 chars).
  for (idx, g) in graphs.iter().enumerate() {
    let text = spans_to_text(g);
    assert!(
      text.contains('○'),
      "linear history row {} must carry a ○ node, got {:?}",
      idx,
      text
    );
    assert!(
      !text.contains('◎'),
      "linear history row {} must NOT carry a ◎ merge node, got {:?}",
      idx,
      text
    );
  }
}

#[test]
fn graph_merge_commit_carries_bullseye_and_branch_corners() {
  // Topology:
  //   c (merge: parents = a, b)
  //   b (parent a)         ← side branch
  //   a (no parent)        ← trunk root
  let rows = vec![test_row("c", &["a", "b"]), test_row("b", &["a"]), test_row("a", &[])];
  let graphs = render_commits(&rows, &Theme::default());
  // Row 0 = merge commit, must carry ◎.
  let merge_text = spans_to_text(&graphs[0]);
  assert!(merge_text.contains('◎'), "merge row must carry ◎, got {:?}", merge_text);
  // Row 1 (the side branch) must spawn somewhere outside column 0 —
  // there should be a `╮` corner on row 0 to drop the second parent
  // into a fresh column.
  assert!(
    merge_text.contains('╮') || merge_text.contains('─'),
    "merge row must carry a corner / horizontal stroke into the new branch column, got {:?}",
    merge_text
  );
}

#[test]
fn graph_pipe_set_first_commit_seeds_starts_pipe() {
  // Internal invariant: the first row's pipe set must contain a STARTS
  // pipe for the first commit, regardless of how many parents it has.
  let rows = vec![test_row("a", &["b"])];
  let pipes = build_pipe_sets(&rows);
  assert_eq!(pipes.len(), 1);
  assert!(
    pipes[0]
      .iter()
      .any(|p| p.kind == PipeKind::Starts && p.from_hash == test_row("a", &[]).hash),
    "first row must contain a STARTS pipe whose from_hash is the commit itself, got {:?}",
    pipes[0]
  );
}

#[test]
fn graph_pipe_set_merge_commit_emits_extra_starts_per_parent() {
  // A merge with 2 parents should emit 2 STARTS pipes whose from_pos is
  // the commit's column and to_pos points at distinct columns.
  let rows = vec![test_row("c", &["a", "b"]), test_row("b", &["a"]), test_row("a", &[])];
  let pipes = build_pipe_sets(&rows);
  let row0 = &pipes[0];
  let starts: Vec<_> = row0.iter().filter(|p| p.kind == PipeKind::Starts).collect();
  assert_eq!(
    starts.len(),
    2,
    "merge row must emit 2 STARTS pipes (one per parent), got {} ({:?})",
    starts.len(),
    starts
  );
}

#[test]
fn graph_render_pipe_set_empty_input_returns_empty() {
  let graphs = render_commits(&[], &Theme::default());
  assert!(graphs.is_empty());
}

#[test]
fn graph_row_width_is_deterministic_on_commit_list() {
  // The graph width is `2 * (max_pos + 1)` chars, derived from pipe
  // topology — it must NOT depend on terminal width or external state.
  // Snapshot the linear-history width so a regression caught quickly.
  let rows = vec![test_row("c", &["b"]), test_row("b", &["a"]), test_row("a", &[])];
  let graphs = render_commits(&rows, &Theme::default());
  for g in &graphs {
    let text = spans_to_text(g);
    let chars = text.chars().count();
    assert!(
      (1..=8).contains(&chars),
      "linear-history row must render in ≤ 8 chars, got {} ({:?})",
      chars,
      text
    );
  }
}

#[test]
fn graph_render_pipe_set_handles_single_pipe_starts() {
  use gwm::tui::commit_graph::Pipe;
  let from = test_row("a", &[]);
  let to = test_row("b", &[]);
  let pipes = vec![Pipe {
    from_pos: 0,
    to_pos: 0,
    from_hash: from.hash,
    to_hash: to.hash,
    kind: PipeKind::Starts,
  }];
  let spans = render_pipe_set(&pipes, &Theme::default());
  let text = spans_to_text(&spans);
  // Cell 0: ○ + filler (space, since right has no neighbor)
  assert!(text.starts_with('○'), "expected ○ glyph at column 0, got {:?}", text);
}

#[test]
fn recent_commits_line_marks_normal_commit_with_open_circle() {
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 1, &Theme::default());
  let joined: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    joined.contains('○'),
    "non-merge row must carry the ○ marker, got: {}",
    joined
  );
  assert!(
    !joined.contains('◎'),
    "non-merge row must NOT carry the ◎ marker, got: {}",
    joined
  );
}

#[test]
fn recent_commits_line_starts_with_short_hash() {
  // The first span of every row must be a hash of exactly
  // COMMIT_HASH_DISPLAY_LEN hex chars (8 by default, matching lazygit).
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 1, &Theme::default());
  assert_eq!(lines.len(), 1, "init_repo should produce 1 commit");
  let head_span = lines[0]
    .spans
    .first()
    .expect("commit row must carry at least the hash span");
  assert_eq!(
    head_span.content.chars().count(),
    COMMIT_HASH_DISPLAY_LEN,
    "expected hash span of {} chars, got {:?}",
    COMMIT_HASH_DISPLAY_LEN,
    head_span.content
  );
  // Must be all-hex.
  assert!(
    head_span.content.chars().all(|c| c.is_ascii_hexdigit()),
    "expected hex hash, got {:?}",
    head_span.content
  );
}

#[test]
fn recent_commits_line_includes_author_initials_after_hash() {
  // init_repo signs commits as "gwm-test" — a single token → first 2 chars.
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 1, &Theme::default());
  let joined: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
  // Initials live as a styled span after the hash + double space.
  assert!(
    joined.contains("gw"),
    "expected 'gw' initials from 'gwm-test', got {:?}",
    joined
  );
}

#[test]
fn recent_commits_line_carries_subject_unclipped() {
  // The renderer relies on ratatui's view-level clip — `recent_commits_lines`
  // itself must NOT pre-truncate the subject (otherwise scrollback / wider
  // sidebars would lose information). Verify the full subject is preserved.
  let (dir, _repo) = init_repo();
  let w = worktree_pointing_at_dir(dir.path());
  let lines = recent_commits_lines(&w, 1, &Theme::default());
  let joined: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
  assert!(
    joined.contains("init"),
    "expected the seed 'init' subject, got {:?}",
    joined
  );
  assert!(
    !joined.contains('…'),
    "must not pre-emptively truncate with ellipsis: {:?}",
    joined
  );
}

// --- TOFU trust gate, TUI side (issue #95, PR #113 follow-up) -----------
//
// `check_trust_for_bootstrap` is the silent gate shared by
// `submit_create` and `bootstrap_selected`. These tests exercise it
// directly — driving the full event loop through `submit_create` would
// require a real worktree base + `worktree::add`, which is orthogonal
// to the security policy under test. The two `submit_create_*` /
// `bootstrap_selected_*` integration tests at the bottom of this
// section verify the actual call-site wiring.

use gwm::trust::TrustMode;

/// Build an App whose workdir already has a `.gwm.toml` ready to be
/// hashed by the gate. Returns the dir keepalive + the App (with
/// `trust_mode = Prompt` by default).
fn app_with_config(toml_body: &str) -> (tempfile::TempDir, App) {
  let (dir, _repo) = init_repo();
  std::fs::write(dir.path().join(".gwm.toml"), toml_body).unwrap();
  let app = App::new_at_layered(Some(dir.path()), None).unwrap();
  (dir, app)
}

fn toml_basic_string(path: &std::path::Path) -> String {
  path.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
}

#[test]
fn tui_gate_passes_when_no_gwm_toml_present() {
  // The CLI gate is a no-op when `.gwm.toml` doesn't exist — same
  // contract on the TUI side. Construction with `init_repo`'s empty
  // workdir already covers this.
  let (_dir, app) = make_app();
  assert!(
    matches!(app.check_trust_for_bootstrap(), Ok(None)),
    "no .gwm.toml → gate must clear"
  );
}

#[test]
fn tui_gate_passes_on_empty_bootstrap_surface() {
  // A `.gwm.toml` with only `[worktree]` (no executable surface)
  // carries no RCE risk. Prompting in that case would just train
  // the user to mash `y` — same UX bug the CLI gate avoids.
  let (_dir, app) = app_with_config(
    r#"[worktree]
base = "/tmp/never-used"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"
"#,
  );
  assert!(
    matches!(app.check_trust_for_bootstrap(), Ok(None)),
    "empty surface → gate must clear"
  );
}

#[test]
fn tui_gate_refuses_untrusted_config_in_prompt_mode() {
  // Default `TrustMode::Prompt`: a `.gwm.toml` declaring a bootstrap
  // command, no ledger entry → the gate refuses with a status-bar
  // message. The CLI in this same position would prompt; the TUI
  // can't (alternate-screen + no modal yet), so it points the user
  // at the CLI gate / env bypass.
  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");
  // Serialise against the other env-mutating tests in this binary
  // (the PR-detection refresh test also mutates env, #181).
  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  let prior_allow = std::env::var("GWM_ALLOW_BOOTSTRAP").ok();
  // SAFETY: env mutation is guarded by `env_lock()` above.
  unsafe {
    std::env::set_var("GWM_TRUST_LEDGER", &ledger);
    std::env::remove_var("GWM_ALLOW_BOOTSTRAP");
  }

  let (_dir, app) = app_with_config(
    r#"[[bootstrap.command]]
name = "x"
run  = "true"
"#,
  );

  match app.check_trust_for_bootstrap() {
    Ok(Some(msg)) => {
      assert!(
        msg.contains("not in trust ledger"),
        "refuse message must point at the gate (got: {})",
        msg
      );
      assert!(
        msg.contains("--allow-bootstrap") || msg.contains("GWM_ALLOW_BOOTSTRAP"),
        "refuse message must surface the bypass options (got: {})",
        msg
      );
    }
    other => panic!("expected refuse, got {:?}", other),
  }

  // SAFETY: restoration paired with the set/remove above.
  unsafe {
    match prior_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
    match prior_allow {
      Some(v) => std::env::set_var("GWM_ALLOW_BOOTSTRAP", v),
      None => std::env::remove_var("GWM_ALLOW_BOOTSTRAP"),
    }
  }
}

#[test]
fn tui_gate_clears_under_allow_mode() {
  // `--allow-bootstrap` (resolved to `TrustMode::Allow` at the
  // entrypoint) bypasses the gate. Threading it through
  // `with_trust_mode` is the whole reason for this PR follow-up —
  // pin the wiring down.
  let (_dir, app) = app_with_config(
    r#"[[bootstrap.command]]
name = "x"
run  = "true"
"#,
  );
  let app = app.with_trust_mode(TrustMode::Allow);
  assert!(
    matches!(app.check_trust_for_bootstrap(), Ok(None)),
    "Allow mode → gate must clear regardless of ledger state"
  );
}

#[test]
fn tui_gate_refuses_under_deny_mode_even_with_safe_config() {
  // `--deny-bootstrap` is the forensic mode: refuse even if there's
  // nothing scary. The empty-surface short-circuit comes BEFORE the
  // Deny check inside `evaluate`, so a truly empty surface still
  // clears — Deny mode is meaningful only when a real surface
  // exists. Document that with the same fixture as the prompt-mode
  // refuse test (a config with a bootstrap.command).
  let (_dir, app) = app_with_config(
    r#"[[bootstrap.command]]
name = "x"
run  = "true"
"#,
  );
  let app = app.with_trust_mode(TrustMode::Deny);
  let outcome = app.check_trust_for_bootstrap();
  match outcome {
    Ok(Some(msg)) => assert!(
      msg.contains("--deny-bootstrap"),
      "deny refuse must name the flag (got: {})",
      msg
    ),
    other => panic!("expected deny refuse, got {:?}", other),
  }
}

#[test]
fn tui_submit_create_aborts_on_untrusted_config() {
  // End-to-end: `submit_create` reads the gate, sets the status bar
  // verbatim from the refuse message, and crucially does NOT call
  // `worktree::add` — meaning no orphaned worktree dir lands on
  // disk. We pin that postcondition by checking the resolved
  // worktree path is absent after the call.
  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");
  let base_dir = tempfile::TempDir::new().unwrap();
  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  let prior_allow = std::env::var("GWM_ALLOW_BOOTSTRAP").ok();
  // SAFETY: see comment on `tui_gate_refuses_untrusted_config_in_prompt_mode`.
  unsafe {
    std::env::set_var("GWM_TRUST_LEDGER", &ledger);
    std::env::remove_var("GWM_ALLOW_BOOTSTRAP");
  }

  let body = format!(
    r#"[worktree]
base = "{base}"
path_pattern = "{{type}}-{{issue}}-{{desc}}"
branch_pattern = "{{type}}/#{{issue}}-{{desc}}"

[[bootstrap.command]]
name = "echo"
run  = "echo would-have-run"
"#,
    base = toml_basic_string(base_dir.path()),
  );
  let (_dir, mut app) = app_with_config(&body);

  // Drive `submit_create` end-to-end: type=feat (index in the
  // resolved branch_types), issue + desc filled in.
  let feat_idx = app
    .branch_types
    .iter()
    .position(|t| t.name == "feat")
    .expect("`feat` is in BRANCH_TYPES defaults");
  app.create_form.type_index = feat_idx;
  app.create_form.issue = "42".into();
  app.create_form.desc = "untrusted-creates".into();

  // Must succeed (no Err — Err would crash out of the event loop) and
  // the gate must have set a refuse message.
  app.submit_create().expect("submit_create must surface a soft refusal");

  assert!(
    app.status.contains("not in trust ledger"),
    "status must reflect the gate refusal (got: {})",
    app.status
  );
  let would_have_been = base_dir.path().join("feat-42-untrusted-creates");
  assert!(
    !would_have_been.exists(),
    "worktree dir MUST NOT be created when the gate refuses (got: {})",
    would_have_been.display()
  );

  // SAFETY: env restoration paired with set/remove above.
  unsafe {
    match prior_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
    match prior_allow {
      Some(v) => std::env::set_var("GWM_ALLOW_BOOTSTRAP", v),
      None => std::env::remove_var("GWM_ALLOW_BOOTSTRAP"),
    }
  }
  let _ = BRANCH_TYPES; // keep the import live; the indirect lookup above relies on the default list.
}

// ---- Issue #276: create worktree on the async-task spine ------------------

fn fill_create_form(app: &mut App, issue: &str, desc: &str) {
  let feat_idx = app
    .branch_types
    .iter()
    .position(|t| t.name == "feat")
    .expect("`feat` is in branch types");
  app.create_form.type_index = feat_idx;
  app.create_form.issue = issue.into();
  app.create_form.desc = desc.into();
}

#[test]
fn submit_create_starts_async_create_and_keeps_create_modal_open() {
  use gwm::tui::state::async_task::TaskKind;

  let base_dir = tempfile::TempDir::new().unwrap();
  let body = format!(
    r#"[worktree]
base = "{base}"
path_pattern = "{{type}}-{{issue}}-{{desc}}"
branch_pattern = "{{type}}/#{{issue}}-{{desc}}"
"#,
    base = toml_basic_string(base_dir.path()),
  );
  let (_dir, mut app) = app_with_config(&body);
  app.view = View::Create;
  fill_create_form(&mut app, "276", "async-create");

  app
    .submit_create()
    .expect("submit_create must only enqueue async create");

  assert_eq!(app.view, View::Create, "the modal stays open while create runs");
  assert!(
    app.tasks.is_loading(TaskKind::CreateWorktree),
    "create must claim an async loading slot"
  );
  assert_eq!(app.status, TaskKind::CreateWorktree.loading_label());

  // A real async worktree create + bootstrap drains in well under 100ms on a
  // dev box, but a loaded Windows CI runner can take longer. The old 500ms
  // budget (50 × 10ms) was too tight and flaked (#328); 3s is generous enough
  // to absorb a slow runner while still bailing fast if the task truly hangs.
  for _ in 0..300 {
    if app.drain_task_results() {
      break;
    }
    std::thread::sleep(Duration::from_millis(10));
  }
  assert!(
    !app.tasks.is_loading(TaskKind::CreateWorktree),
    "background create should drain before temp dirs are dropped"
  );
}

#[test]
fn drain_applies_async_create_result_and_flips_to_report_view() {
  use gwm::bootstrap::{BootstrapReport, StepResult};
  use gwm::tui::{CreateWorktreeResult, TaskKind, TaskMsg};

  let (_dir, mut app) = make_app();
  let generation = app.tasks.request(TaskKind::CreateWorktree).unwrap();
  app.view = View::Create;

  app
    .task_result_sender()
    .send(TaskMsg::CreateWorktree(
      generation,
      Ok(CreateWorktreeResult {
        branch: "feat/#276-async-create".into(),
        created: PathBuf::from("/tmp/gwm-created"),
        report: BootstrapReport {
          steps: vec![StepResult::ok("post_create hook")],
        },
      }),
    ))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "a live create result must be applied");
  assert_eq!(app.view, View::Report);
  assert!(app.report.is_some(), "create report is shown in the Report view");
  assert!(
    app.status.contains("created feat/#276-async-create @ /tmp/gwm-created"),
    "status reports the created branch and path: {:?}",
    app.status
  );
  assert!(!app.tasks.is_loading(TaskKind::CreateWorktree));
}

#[test]
fn drain_create_failure_stays_in_create_and_reports_status() {
  use gwm::tui::{TaskKind, TaskMsg};

  let (_dir, mut app) = make_app();
  let generation = app.tasks.request(TaskKind::CreateWorktree).unwrap();
  app.view = View::Create;

  app
    .task_result_sender()
    .send(TaskMsg::CreateWorktree(generation, Err("branch already exists".into())))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "a create failure still clears the live slot");
  assert_eq!(app.view, View::Create);
  assert_eq!(app.status, "create failed: branch already exists");
  assert!(!app.tasks.is_loading(TaskKind::CreateWorktree));
}

#[test]
fn drain_drops_a_superseded_create_result() {
  use gwm::bootstrap::{BootstrapReport, StepResult};
  use gwm::tui::{CreateWorktreeResult, TaskKind, TaskMsg};

  let (_dir, mut app) = make_app();
  let stale = app.tasks.request(TaskKind::CreateWorktree).unwrap();
  app.tasks.invalidate(TaskKind::CreateWorktree);
  app.view = View::Create;
  app.status = "untouched".into();

  app
    .task_result_sender()
    .send(TaskMsg::CreateWorktree(
      stale,
      Ok(CreateWorktreeResult {
        branch: "feat/#276-stale".into(),
        created: PathBuf::from("/tmp/stale"),
        report: BootstrapReport {
          steps: vec![StepResult::ok("stale")],
        },
      }),
    ))
    .unwrap();
  app.drain_task_results();

  assert_eq!(app.view, View::Create);
  assert_eq!(app.status, "untouched");
  assert!(app.report.is_none());
}

#[test]
fn tui_bootstrap_selected_aborts_on_untrusted_config() {
  // Counterpart of the submit_create test — the `b` keybinding
  // (re-run bootstrap on an existing worktree) takes the same gate.
  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");
  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  let prior_allow = std::env::var("GWM_ALLOW_BOOTSTRAP").ok();
  // SAFETY: same rationale as the previous test.
  unsafe {
    std::env::set_var("GWM_TRUST_LEDGER", &ledger);
    std::env::remove_var("GWM_ALLOW_BOOTSTRAP");
  }

  let (_dir, mut app) = app_with_config(
    r#"[[bootstrap.command]]
name = "echo"
run  = "echo trapped"
"#,
  );

  // Seed a fake selection so `selected()` returns Some — otherwise
  // `bootstrap_selected` short-circuits before reaching the gate
  // with "nothing selected".
  app.worktrees = vec![worktree_fixture("dummy")];
  app.list_state.select(Some(0));

  app.bootstrap_selected();

  assert!(
    app.status.contains("not in trust ledger"),
    "status must reflect the gate refusal (got: {})",
    app.status
  );

  // SAFETY: env restoration paired with set/remove above.
  unsafe {
    match prior_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
    match prior_allow {
      Some(v) => std::env::set_var("GWM_ALLOW_BOOTSTRAP", v),
      None => std::env::remove_var("GWM_ALLOW_BOOTSTRAP"),
    }
  }
}

// ---- Issue #256: bootstrap on the async-task spine ----------------------
//
// `bootstrap_selected` claims a `TaskKind::Bootstrap` generation and spawns
// a worker (after the synchronous TOFU gate above); the worker's
// `TaskMsg::Bootstrap` is applied by `drain_task_results`. These tests pin
// the drain side — the result handling and late-drop guard — without a real
// OS thread, mirroring the GitHub / sync drain tests.

#[test]
fn bootstrap_selected_with_no_selection_reports_and_does_not_load() {
  // The early guard runs before the trust gate and the spawn: with nothing
  // selected there is no worktree to bootstrap, so no generation is claimed.
  let (_dir, mut app) = make_app();
  app.worktrees.clear();
  app.bootstrap_selected();
  assert_eq!(app.status, "nothing selected");
  assert!(!app.is_task_loading(), "no worktree selected → no task claimed");
}

#[test]
fn drain_applies_async_bootstrap_report_and_flips_to_report_view() {
  use gwm::bootstrap::{BootstrapReport, StepResult};
  use gwm::tui::{TaskKind, TaskMsg};
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-x");

  // Claim a generation exactly as `bootstrap_selected` would after the gate.
  let generation = app
    .tasks
    .request(TaskKind::Bootstrap)
    .expect("a cold bootstrap slot must hand out a generation");
  assert!(app.is_task_loading(), "request must mark the app as loading");

  let report = BootstrapReport {
    steps: vec![StepResult::ok("copy .env"), StepResult::ok("post_create hook")],
  };
  app
    .task_result_sender()
    .send(TaskMsg::Bootstrap(generation, Ok(report)))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "a live bootstrap result must be applied");
  assert_eq!(app.view, View::Report, "completion flips to the Report view");
  assert!(app.report.is_some(), "the report is stored for the Report view");
  assert_eq!(app.status, "bootstrap ok");
  assert!(!app.is_task_loading(), "completion clears the in-flight slot");
}

#[test]
fn drain_bootstrap_report_with_a_failed_step_says_had_failures() {
  use gwm::bootstrap::{BootstrapReport, StepResult};
  use gwm::tui::{TaskKind, TaskMsg};
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-x");

  let generation = app.tasks.request(TaskKind::Bootstrap).unwrap();
  let report = BootstrapReport {
    steps: vec![
      StepResult::ok("copy .env"),
      StepResult::failed("post_create hook", "exit 1"),
    ],
  };
  app
    .task_result_sender()
    .send(TaskMsg::Bootstrap(generation, Ok(report)))
    .unwrap();
  app.drain_task_results();

  assert_eq!(app.view, View::Report, "a partial failure still shows the report");
  assert_eq!(app.status, "bootstrap had failures");
}

#[test]
fn a_late_bootstrap_result_is_dropped_and_keeps_the_list_view() {
  use gwm::bootstrap::{BootstrapReport, StepResult};
  use gwm::tui::{TaskKind, TaskMsg};
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-x");

  // A worker is in flight, then an invalidate bumps the generation (e.g. a
  // second `b` press coalesced after an invalidate) — the stale worker's
  // result must not flip the view to its now-superseded report.
  let stale = app.tasks.request(TaskKind::Bootstrap).unwrap();
  app.tasks.invalidate(TaskKind::Bootstrap);
  app
    .task_result_sender()
    .send(TaskMsg::Bootstrap(
      stale,
      Ok(BootstrapReport {
        steps: vec![StepResult::ok("stale")],
      }),
    ))
    .unwrap();
  app.drain_task_results();

  assert_eq!(app.view, View::List, "a dropped late result must not flip the view");
  assert!(app.report.is_none(), "a dropped late result must not store a report");
}

#[test]
fn drain_bootstrap_error_reports_status_and_does_not_flip_to_report() {
  use gwm::tui::{TaskKind, TaskMsg};
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-x");

  let generation = app.tasks.request(TaskKind::Bootstrap).unwrap();
  app
    .task_result_sender()
    .send(TaskMsg::Bootstrap(generation, Err("disk full".into())))
    .unwrap();
  app.drain_task_results();

  assert_eq!(
    app.view,
    View::List,
    "a failed bootstrap stays on the list, no Report to show"
  );
  assert!(app.report.is_none());
  assert_eq!(app.status, "bootstrap error: disk full");
}

#[test]
fn drain_bootstrap_report_flips_to_report_even_from_another_view() {
  // The bootstrap is async now (issue #256): between the `b` press and the
  // result, the user may have navigated elsewhere (e.g. opened the create
  // form). A live result still flips to the Report view — the user asked for
  // the bootstrap, so its outcome takes the screen. This pins the
  // always-flip choice (vs only flipping from the list view); a behaviour
  // change from the old synchronous path, which had no such window.
  use gwm::bootstrap::{BootstrapReport, StepResult};
  use gwm::tui::{TaskKind, TaskMsg};
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-x");

  let generation = app.tasks.request(TaskKind::Bootstrap).unwrap();
  app.view = View::Create;
  app
    .task_result_sender()
    .send(TaskMsg::Bootstrap(
      generation,
      Ok(BootstrapReport {
        steps: vec![StepResult::ok("copy .env")],
      }),
    ))
    .unwrap();
  app.drain_task_results();

  assert_eq!(
    app.view,
    View::Report,
    "a live bootstrap result takes the screen even mid-create"
  );
}

// ---- Issue #106: LinkTarget canonical location --------------------------
//
// The `LinkTarget` enum was duplicated between `cli.rs` and
// `tui/app.rs`. Both call sites should now resolve to the SAME type
// (re-exported from a single module) so a value created on the CLI
// boundary can be handed to a TUI function without an `as` cast or
// a manual conversion.

#[test]
fn link_target_is_canonical_across_cli_and_tui() {
  // If `cli::LinkTarget` and `tui::LinkTarget` are the same type,
  // a value assigned from one accessor binds to the other without
  // a conversion call.
  let from_cli: gwm::cli::LinkTarget = gwm::cli::LinkTarget::Issue;
  let from_tui: gwm::tui::LinkTarget = from_cli;
  assert_eq!(from_tui, gwm::tui::LinkTarget::Issue);

  let from_tui: gwm::tui::LinkTarget = gwm::tui::LinkTarget::Pr;
  let from_cli: gwm::cli::LinkTarget = from_tui;
  assert_eq!(from_cli, gwm::cli::LinkTarget::Pr);
}

#[test]
fn fresh_app_confirm_modal_focuses_cancel() {
  // #187: the App wires the confirm modal's default button focus to
  // Cancel, so the destructive `[ Confirm ]` is never the button a
  // stray Enter lands on when the modal first opens.
  use gwm::tui::ConfirmButton;
  let (_dir, app) = make_app();
  assert_eq!(app.confirm.focused_button(), ConfirmButton::Cancel);
}

#[test]
fn fresh_app_spinner_starts_at_first_frame() {
  // #187: the App owns a Spinner loader initialised to its first frame.
  use gwm::tui::state::spinner::DOT_FRAMES;
  let (_dir, app) = make_app();
  assert_eq!(app.spinner.glyph(DOT_FRAMES), DOT_FRAMES[0]);
}

#[test]
fn help_scroll_clamps_between_zero_and_max() {
  // Issue #217 follow-up: the Keybindings overlay scrolls when the help
  // outgrows the modal. `help_max_scroll` is published by the renderer
  // each frame; the offset clamps to `[0, max]` and resets on (re)open.
  let (_dir, mut app) = make_app();
  app.enter_help();
  assert_eq!(app.view, View::Help);
  assert_eq!(app.help_scroll, 0, "a freshly opened help starts at the top");

  // Simulate the renderer having measured 3 rows of overflow.
  app.help_max_scroll = 3;
  app.help_scroll_down();
  app.help_scroll_down();
  assert_eq!(app.help_scroll, 2);
  app.help_scroll_down();
  app.help_scroll_down();
  assert_eq!(app.help_scroll, 3, "scroll-down clamps at the published max");

  app.help_scroll_up();
  assert_eq!(app.help_scroll, 2);
  for _ in 0..10 {
    app.help_scroll_up();
  }
  assert_eq!(app.help_scroll, 0, "scroll-up clamps at the top");

  // Re-opening help resets the offset.
  app.help_scroll = 2;
  app.enter_help();
  assert_eq!(app.help_scroll, 0, "(re)opening help returns to the top");
}

#[test]
fn help_horizontal_scroll_clamps_between_zero_and_max() {
  let (_dir, mut app) = make_app();
  app.enter_help();
  assert_eq!(app.help_x_scroll, 0);

  app.help_max_x_scroll = 2;
  app.help_scroll_right();
  assert_eq!(app.help_x_scroll, 1);
  app.help_scroll_right();
  app.help_scroll_right();
  assert_eq!(app.help_x_scroll, 2, "scroll-right clamps at the published max");

  app.help_scroll_left();
  assert_eq!(app.help_x_scroll, 1);
  app.help_scroll_left();
  app.help_scroll_left();
  assert_eq!(app.help_x_scroll, 0, "scroll-left clamps at the left edge");

  app.help_x_scroll = 2;
  app.enter_help();
  assert_eq!(app.help_x_scroll, 0, "(re)opening help returns to the left edge");
}

#[test]
fn create_key_typing_appends_to_the_focused_text_field() {
  // Issue #217 follow-up: the create key handling is an `App` method so the
  // typing path is unit-testable (not just `push_char` in isolation).
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::CreateKey;
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_form.field = Field::Desc;
  for c in "my-feat".chars() {
    assert!(matches!(
      app.handle_create_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
      CreateKey::Handled
    ));
  }
  assert_eq!(app.create_form.desc, "my-feat");
}

#[test]
fn create_key_rejects_issue_letters_with_status_feedback() {
  // Issue #220 visual pass: the modal opens on the digits-only Issue field.
  // A stray letter must not leak into Desc, but it also must not look like
  // typing is broken; the status bar explains the contract.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::CreateKey;
  let (_dir, mut app) = make_app();
  app.enter_create();
  assert_eq!(app.create_form.field, Field::Issue);

  assert!(matches!(
    app.handle_create_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
    CreateKey::Handled
  ));
  assert!(app.create_form.issue.is_empty());
  assert!(
    app.create_form.desc.is_empty(),
    "non-digit Issue input must stay on Issue and never append to Desc"
  );
  assert!(
    app.status.contains("digits"),
    "status should explain the digits-only Issue field, got {:?}",
    app.status
  );

  app.handle_create_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
  assert_eq!(app.create_form.issue, "7");
  assert!(app.create_form.desc.is_empty());
}

#[test]
fn create_key_hl_cycles_the_type_only_when_type_is_focused() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, mut app) = make_app();
  app.enter_create();
  // h/l type cycling only fires while the Type field is focused; pin it
  // here since the modal now opens on Issue (#217).
  app.create_form.field = Field::Type;
  app.handle_create_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
  assert_eq!(app.create_form.type_index, 1, "l advances the type");
  app.handle_create_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
  assert_eq!(app.create_form.type_index, 0, "h steps back");
  // On a text field, h / l are literal input, not type cycling — otherwise
  // we'd recreate the very "can't type these letters" bug we're avoiding.
  app.create_form.field = Field::Desc;
  app.handle_create_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
  app.handle_create_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
  assert_eq!(app.create_form.desc, "hl");
  assert_eq!(app.create_form.type_index, 0, "type stays put while editing desc");
}

#[test]
fn create_key_enter_advances_then_submits_on_desc() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::CreateKey;
  let (_dir, mut app) = make_app();
  app.enter_create();
  app.create_form.field = Field::Issue;
  assert!(matches!(
    app.handle_create_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    CreateKey::Handled
  ));
  assert_eq!(
    app.create_form.field,
    Field::Desc,
    "Enter off the desc field advances focus"
  );
  assert!(matches!(
    app.handle_create_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    CreateKey::Submit
  ));
}

#[test]
fn create_key_esc_requests_cancel() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::CreateKey;
  let (_dir, mut app) = make_app();
  app.enter_create();
  assert!(matches!(
    app.handle_create_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    CreateKey::Cancel
  ));
}

// ---------------------------------------------------------------------------
// Async-task layer — off-thread worktree refresh (issue #231)
// ---------------------------------------------------------------------------

#[test]
fn drain_applies_async_refresh_result() {
  // Issue #231: a worktree list refresh delivered off-thread (over the task
  // channel) is applied by `drain_task_results`, swapping in the fresh list
  // and clearing the loading slot — the deterministic analogue of the
  // background worker, with no real OS thread (the flaky-thread-test trap).
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  // Claim the slot exactly as `request_refresh` would, without spawning.
  let generation = app.tasks.request(TaskKind::RefreshWorktrees).unwrap();
  assert!(app.is_task_loading(), "request must mark the app as loading");

  let fresh = vec![worktree_fixture("alpha"), worktree_fixture("beta")];
  app
    .task_result_sender()
    .send(TaskMsg::RefreshWorktrees(generation, Ok(fresh)))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "drain must report it applied a result");
  assert_eq!(app.worktrees.len(), 2, "the fresh list replaces the old one");
  assert!(!app.is_task_loading(), "no task should be inflight after draining");
  assert!(
    app.status.contains("refreshed"),
    "status reports the refresh outcome: {:?}",
    app.status
  );
}

// ---- sidebar off-thread rebuild (issue #343) -----------------------------
// The details sidebar's git subprocesses moved off the render path onto the
// `TaskRunner` (`TaskKind::Sidebar`). These pin the decision + drain contract
// deterministically — claim the slot / inject a `TaskMsg::Sidebar` / drain,
// never a real OS worker thread (the #248 flaky trap).

#[test]
fn maybe_refresh_sidebar_is_a_noop_when_the_cache_is_current() {
  use gwm::tui::state::async_task::TaskKind;
  use gwm::tui::SidebarSections;
  let (_dir, mut app) = make_app();
  let w = app.selected().expect("a worktree is selected").clone();
  let mode = app.sidebar.mode;
  // Cache already built for this selection + mode → nothing to rebuild.
  app.sidebar.cache = Some(((w.path.clone(), mode), SidebarSections::default()));

  app.maybe_refresh_sidebar();

  assert!(
    !app.tasks.is_loading(TaskKind::Sidebar),
    "a cache current for the selection must not spawn a rebuild"
  );
}

#[test]
fn maybe_refresh_sidebar_coalesces_a_held_navigation_onto_one_worker() {
  // The debounce: while one rebuild is in flight, every subsequent tick (a
  // held `j` with a stale cache) coalesces onto it instead of claiming a new
  // generation. Proven by draining the ORIGINAL generation's result — a
  // re-request would have bumped the generation and this would be dropped.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  use gwm::tui::SidebarSections;
  let (_dir, mut app) = make_app();
  let gen = app
    .tasks
    .request(TaskKind::Sidebar)
    .expect("cold slot claims a generation");
  app.sidebar.cache = None; // stale → maybe_refresh would want to rebuild

  // Several event-loop ticks fire while the worker runs.
  app.maybe_refresh_sidebar();
  app.maybe_refresh_sidebar();

  let path = app.selected().unwrap().path.clone();
  let mode = app.sidebar.mode;
  app
    .task_result_sender()
    .send(TaskMsg::Sidebar(gen, path.clone(), mode, SidebarSections::default()))
    .unwrap();
  assert!(
    app.drain_task_results(),
    "the original worker's result must still apply — the ticks coalesced, they did not re-request"
  );
  assert!(
    matches!(&app.sidebar.cache, Some(((p, _), _)) if *p == path),
    "the coalesced worker's payload lands in the cache"
  );
}

#[test]
fn drain_applies_a_sidebar_rebuild_and_clears_the_slot() {
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  use gwm::tui::state::sidebar::SidebarMode;
  use gwm::tui::SidebarSections;
  let (_dir, mut app) = make_app();
  let gen = app.tasks.request(TaskKind::Sidebar).unwrap();
  let path = PathBuf::from("/tmp/gwm-test/alpha");
  let mode = SidebarMode::Commits;

  app
    .task_result_sender()
    .send(TaskMsg::Sidebar(gen, path.clone(), mode, SidebarSections::default()))
    .unwrap();
  assert!(app.drain_task_results(), "drain reports it applied the sidebar payload");

  assert!(
    matches!(&app.sidebar.cache, Some(((p, m), _)) if *p == path && *m == mode),
    "the payload is stored under the (path, mode) it was built for"
  );
  assert!(
    !app.tasks.is_loading(TaskKind::Sidebar),
    "the slot is cleared once the result applies"
  );
}

#[test]
fn drain_drops_a_superseded_sidebar_rebuild() {
  // A selection that moved (or a mutation) bumped the generation mid-flight;
  // the stale worker's late payload must be discarded, not stored.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  use gwm::tui::state::sidebar::SidebarMode;
  use gwm::tui::SidebarSections;
  let (_dir, mut app) = make_app();
  let stale = app.tasks.request(TaskKind::Sidebar).unwrap();
  app.tasks.invalidate(TaskKind::Sidebar); // superseded
  app.sidebar.cache = None;

  app
    .task_result_sender()
    .send(TaskMsg::Sidebar(
      stale,
      PathBuf::from("/tmp/gwm-test/ghost"),
      SidebarMode::Commits,
      SidebarSections::default(),
    ))
    .unwrap();
  app.drain_task_results();

  assert!(
    app.sidebar.cache.is_none(),
    "a superseded sidebar payload must be dropped, not stored (the #138 guard)"
  );
}

#[test]
fn refresh_invalidates_an_inflight_sidebar_rebuild() {
  // Advisor #3 / issue #343: a synchronous `refresh()` (create / delete /
  // sync / report-close) re-lists worktrees, so an in-flight sidebar rebuild
  // was reading *pre-mutation* git state. `refresh()` must bump the Sidebar
  // generation so that late payload is dropped by the drain.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  use gwm::tui::SidebarSections;
  let (_dir, mut app) = make_app();
  let stale = app.tasks.request(TaskKind::Sidebar).unwrap();
  let path = app.selected().unwrap().path.clone();
  let mode = app.sidebar.mode;

  app.refresh().unwrap();

  app
    .task_result_sender()
    .send(TaskMsg::Sidebar(stale, path, mode, SidebarSections::default()))
    .unwrap();
  app.drain_task_results();

  assert!(
    app.sidebar.cache.is_none(),
    "refresh() must drop a pre-mutation sidebar payload so it can't clobber the fresh preview"
  );
}

#[test]
fn drain_async_refresh_invalidates_an_inflight_sidebar_rebuild() {
  // codex review (PR #351): the ASYNC refresh drains re-read git state via
  // `apply_refreshed_worktrees`, so an in-flight sidebar rebuild is now reading
  // pre-refresh data. Its late payload must be dropped — the invalidate lives
  // in the shared tail, not just the synchronous `refresh()`, else the async
  // `r` / auto-refresh path would store stale sections under the current key
  // and render them as fresh until the next navigation.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  use gwm::tui::SidebarSections;
  let (_dir, mut app) = make_app();
  // A sidebar rebuild is in flight...
  let stale_sidebar = app.tasks.request(TaskKind::Sidebar).unwrap();
  let path = app.selected().unwrap().path.clone();
  let mode = app.sidebar.mode;
  // ...when an async worktree refresh lands and re-lists.
  let refresh_gen = app.tasks.request(TaskKind::RefreshWorktrees).unwrap();
  app
    .task_result_sender()
    .send(TaskMsg::RefreshWorktrees(
      refresh_gen,
      Ok(vec![worktree_fixture("alpha")]),
    ))
    .unwrap();
  assert!(app.drain_task_results(), "the async refresh applies");

  // The pre-refresh sidebar worker reports late.
  app
    .task_result_sender()
    .send(TaskMsg::Sidebar(stale_sidebar, path, mode, SidebarSections::default()))
    .unwrap();
  app.drain_task_results();

  assert!(
    app.sidebar.cache.is_none(),
    "the async refresh must drop the pre-refresh sidebar payload, not store it as current"
  );
}

#[test]
fn maybe_refresh_sidebar_skips_a_hidden_sidebar() {
  // codex review (PR #351): a hidden sidebar is not drawn, so rebuilding its
  // preview would run git work for an invisible panel — the pre-#343 behaviour
  // was to do none. Guard on `sidebar.open`.
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  app.sidebar.open = false;
  app.sidebar.cache = None; // would otherwise trigger a rebuild

  app.maybe_refresh_sidebar();

  assert!(
    !app.tasks.is_loading(TaskKind::Sidebar),
    "a hidden sidebar must do no preview work"
  );
}

#[cfg(unix)]
#[test]
fn worktree_refresh_fetches_issue_and_pr_status_for_every_linked_worktree() {
  use gwm::github::BranchLink;
  use gwm::tui::{TaskKind, TaskMsg};
  use std::os::unix::fs::PermissionsExt;

  let (dir, repo, mut app) = make_app_on_branch("feat/#42-selected");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  let fake_gh = dir.path().join("fake-gh-refresh-all");
  std::fs::write(
    &fake_gh,
    "#!/bin/sh\n\
     kind=\"$1\"\n\
     number=\"$3\"\n\
     if [ \"$kind\" = \"issue\" ] && [ \"$2\" = \"view\" ]; then\n\
       printf '{\"number\":%s,\"title\":\"issue %s\",\"state\":\"CLOSED\",\"url\":\"https://example.test/issues/%s\",\"labels\":[],\"updatedAt\":\"2026-06-09T00:00:00Z\"}' \"$number\" \"$number\" \"$number\"\n\
     elif [ \"$kind\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n\
       printf '{\"number\":%s,\"title\":\"pr %s\",\"state\":\"MERGED\",\"isDraft\":false,\"url\":\"https://example.test/pull/%s\",\"updatedAt\":\"2026-06-09T00:00:00Z\",\"statusCheckRollup\":[]}' \"$number\" \"$number\" \"$number\"\n\
     else\n\
       exit 2\n\
     fi\n",
  )
  .unwrap();
  let mut perms = std::fs::metadata(&fake_gh).unwrap().permissions();
  perms.set_mode(0o755);
  std::fs::set_permissions(&fake_gh, perms).unwrap();

  let linked = |name: &str, branch: &str, issue: u64, pr: u64| {
    let mut w = worktree_fixture(name);
    w.branch = Some(branch.into());
    w.link = BranchLink {
      issue: Some(issue),
      pr: Some(pr),
      issue_title: None,
      pr_title: None,
      issue_state: None,
      pr_state: None,
      issue_source: LinkSource::Explicit,
      pr_source: LinkSource::Explicit,
    };
    w
  };
  for (branch, issue, pr) in [("feat/#42-selected", 42, 61), ("feat/#283-other", 283, 286)] {
    gwm::github::link_issue(&repo, branch, issue).unwrap();
    gwm::github::link_pr(&repo, branch, pr).unwrap();
  }
  let fresh = vec![
    linked("selected", "feat/#42-selected", 42, 61),
    linked("other", "feat/#283-other", 283, 286),
  ];

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: env mutation is guarded by `env_lock()` and restored before the
  // test returns. The worker captures this path on the main thread.
  unsafe {
    std::env::set_var("GWM_GH", &fake_gh);
  }

  let generation = app.tasks.request(TaskKind::RefreshWorktrees).unwrap();
  app
    .task_result_sender()
    .send(TaskMsg::RefreshWorktrees(generation, Ok(fresh)))
    .unwrap();
  assert!(app.drain_task_results(), "the worktree refresh result must apply");

  // Issue #425: do NOT assert `tasks.is_loading(GithubIssue(42))` here. Applying
  // the refresh requests four fetches, each spawning a worker that runs the
  // `fake_gh` shell script above. On an idle runner those workers can finish and
  // have their results consumed inside the very `drain_task_results()` call
  // above, so the tasks are already out of `running` by the time the assertion
  // reads it — the test would be racing the workers it just spawned. Whether a
  // task is still in `running` at an arbitrary instant is an implementation
  // timing detail, not a contract.
  //
  // The contract ("refresh fetches issue and PR status for every linked
  // worktree") is asserted below, and asserted more strongly: the marker colours
  // and the persisted link titles/states can only be right if every fetch was
  // both requested and completed, on the non-selected row as well.

  for _ in 0..200 {
    if !app.tasks.is_loading(TaskKind::GithubIssue(42))
      && !app.tasks.is_loading(TaskKind::GithubPr(61))
      && !app.tasks.is_loading(TaskKind::GithubIssue(283))
      && !app.tasks.is_loading(TaskKind::GithubPr(286))
    {
      break;
    }
    std::thread::sleep(Duration::from_millis(10));
    app.drain_task_results();
  }

  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  let theme = Theme::default();
  for (idx, issue, pr) in [(0, 42, 61), (1, 283, 286)] {
    let cells = marker_cells(&gwm::tui::table_marker(&app.worktrees[idx], &theme));
    assert_eq!(
      cells[0].1,
      Some(gwm::tui::issue_badge_color(IssueState::Closed, &theme)),
      "issue #{issue} marker should reflect the fetched closed state"
    );
    assert_eq!(
      cells[2].1,
      Some(gwm::tui::pr_badge_color(PrState::Merged, &theme)),
      "PR #{pr} marker should reflect the fetched merged state"
    );
  }

  for (branch, issue_title, pr_title) in [
    ("feat/#42-selected", "issue 42", "pr 61"),
    ("feat/#283-other", "issue 283", "pr 286"),
  ] {
    let link = gwm::github::read_link(&repo, branch).unwrap();
    assert_eq!(
      link.issue_title.as_deref(),
      Some(issue_title),
      "issue title should persist on branch {branch}"
    );
    assert_eq!(
      link.issue_state,
      Some(IssueState::Closed),
      "issue state should persist on branch {branch}"
    );
    assert_eq!(
      link.pr_title.as_deref(),
      Some(pr_title),
      "PR title should persist on branch {branch}"
    );
    assert_eq!(
      link.pr_state,
      Some(PrState::Merged),
      "PR state should persist on branch {branch}"
    );
  }
}

#[cfg(unix)]
#[test]
fn app_startup_fetches_issue_and_pr_status_for_linked_worktrees() {
  use gwm::tui::TaskKind;
  use std::os::unix::fs::PermissionsExt;

  let (dir, repo) = init_repo();
  {
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    repo.branch("feat/#42-startup", &head, false).unwrap();
  }
  repo.set_head("refs/heads/feat/#42-startup").unwrap();
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  gwm::github::link_pr(&repo, "feat/#42-startup", 61).unwrap();

  let fake_gh = dir.path().join("fake-gh-startup-refresh");
  std::fs::write(
    &fake_gh,
    "#!/bin/sh\n\
     kind=\"$1\"\n\
     number=\"$3\"\n\
     if [ \"$kind\" = \"issue\" ] && [ \"$2\" = \"view\" ]; then\n\
       printf '{\"number\":%s,\"title\":\"issue %s\",\"state\":\"OPEN\",\"url\":\"https://example.test/issues/%s\",\"labels\":[],\"updatedAt\":\"2026-06-09T00:00:00Z\"}' \"$number\" \"$number\" \"$number\"\n\
     elif [ \"$kind\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n\
       printf '{\"number\":%s,\"title\":\"pr %s\",\"state\":\"OPEN\",\"isDraft\":false,\"url\":\"https://example.test/pull/%s\",\"updatedAt\":\"2026-06-09T00:00:00Z\",\"statusCheckRollup\":[]}' \"$number\" \"$number\" \"$number\"\n\
     else\n\
       exit 2\n\
     fi\n",
  )
  .unwrap();
  let mut perms = std::fs::metadata(&fake_gh).unwrap().permissions();
  perms.set_mode(0o755);
  std::fs::set_permissions(&fake_gh, perms).unwrap();

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: env mutation is guarded by `env_lock()` and the TUI captures the
  // program path before spawning its initial GitHub workers.
  unsafe {
    std::env::set_var("GWM_GH", &fake_gh);
  }

  let app = App::new_at_layered(Some(dir.path()), None).unwrap();

  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  assert!(
    app.tasks.is_loading(TaskKind::GithubIssue(42)),
    "startup should fetch the linked issue immediately"
  );
  assert!(
    app.tasks.is_loading(TaskKind::GithubPr(61)),
    "startup should fetch the linked PR immediately"
  );
}

#[cfg(unix)]
#[test]
fn github_refresh_fetches_only_the_current_link_without_relisting_worktrees() {
  use gwm::github::BranchLink;
  use gwm::tui::TaskKind;
  use std::os::unix::fs::PermissionsExt;

  let (dir, _repo, mut app) = make_app_on_branch("feat/#42-selected");
  let fake_gh = dir.path().join("fake-gh-current-only");
  std::fs::write(
    &fake_gh,
    "#!/bin/sh\n\
     kind=\"$1\"\n\
     number=\"$3\"\n\
     if [ \"$kind\" = \"issue\" ] && [ \"$2\" = \"view\" ]; then\n\
       printf '{\"number\":%s,\"title\":\"issue %s\",\"state\":\"OPEN\",\"url\":\"https://example.test/issues/%s\",\"labels\":[],\"updatedAt\":\"2026-06-09T00:00:00Z\"}' \"$number\" \"$number\" \"$number\"\n\
     elif [ \"$kind\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n\
       printf '{\"number\":%s,\"title\":\"pr %s\",\"state\":\"OPEN\",\"isDraft\":false,\"url\":\"https://example.test/pull/%s\",\"updatedAt\":\"2026-06-09T00:00:00Z\",\"statusCheckRollup\":[]}' \"$number\" \"$number\" \"$number\"\n\
     else\n\
       exit 2\n\
     fi\n",
  )
  .unwrap();
  let mut perms = std::fs::metadata(&fake_gh).unwrap().permissions();
  perms.set_mode(0o755);
  std::fs::set_permissions(&fake_gh, perms).unwrap();

  let mut selected = worktree_fixture("selected");
  selected.branch = Some("feat/#42-selected".into());
  selected.link = BranchLink {
    issue: Some(42),
    pr: Some(61),
    issue_title: None,
    pr_title: None,
    issue_state: None,
    pr_state: None,
    issue_source: LinkSource::Explicit,
    pr_source: LinkSource::Explicit,
  };
  let mut other = worktree_fixture("other");
  other.branch = Some("feat/#283-other".into());
  other.link = BranchLink {
    issue: Some(283),
    pr: Some(286),
    issue_title: None,
    pr_title: None,
    issue_state: None,
    pr_state: None,
    issue_source: LinkSource::Explicit,
    pr_source: LinkSource::Explicit,
  };
  app.github.link = selected.link.clone();
  app.github.link_slug = Some("kbrdn1/gwm-cli".into());
  app.worktrees = vec![selected, other];
  app.list_state.select(Some(0));
  let names_before: Vec<String> = app.worktrees.iter().map(|w| w.name.clone()).collect();

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: guarded by `env_lock()` and restored before returning.
  unsafe {
    std::env::set_var("GWM_GH", &fake_gh);
  }

  app.refresh_github_status();

  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  assert!(
    !app.tasks.is_loading(TaskKind::RefreshWorktrees),
    "GitHub refresh must not relist worktrees"
  );
  assert_eq!(
    app.worktrees.iter().map(|w| w.name.clone()).collect::<Vec<_>>(),
    names_before,
    "GitHub refresh must leave the worktree list untouched"
  );
  assert!(app.tasks.is_loading(TaskKind::GithubIssue(42)));
  assert!(app.tasks.is_loading(TaskKind::GithubPr(61)));
  assert!(
    !app.tasks.is_loading(TaskKind::GithubIssue(283)),
    "GitHub refresh must not fetch a non-selected row's issue"
  );
  assert!(
    !app.tasks.is_loading(TaskKind::GithubPr(286)),
    "GitHub refresh must not fetch a non-selected row's PR"
  );
}

#[test]
fn drain_drops_async_refresh_invalidated_mid_flight() {
  // Issue #231 carries the #138 guarantee onto the generic spine: a refresh
  // result whose generation was bumped by an intervening invalidate is
  // dropped, leaving the current list untouched.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let before = app.worktrees.len();
  let stale = app.tasks.request(TaskKind::RefreshWorktrees).unwrap();
  // The run is superseded (e.g. a fresh `f` or a future invalidation hook).
  app.tasks.invalidate(TaskKind::RefreshWorktrees);
  // The late worker reports back after the bump.
  app
    .task_result_sender()
    .send(TaskMsg::RefreshWorktrees(stale, Ok(vec![worktree_fixture("ghost")])))
    .unwrap();
  app.drain_task_results();
  assert_eq!(
    app.worktrees.len(),
    before,
    "a refresh invalidated mid-flight must be dropped, not applied"
  );
}

#[test]
fn drain_async_refresh_failure_surfaces_status_without_touching_the_list() {
  // Off-thread refresh converts what used to be a fatal `refresh()?` (which
  // tore down the event loop) into a graceful status message; the list is
  // left as-is so the UI keeps showing the last good state.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let before = app.worktrees.len();
  let generation = app.tasks.request(TaskKind::RefreshWorktrees).unwrap();
  app
    .task_result_sender()
    .send(TaskMsg::RefreshWorktrees(generation, Err("boom".into())))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "a failure still counts as a drained result");
  assert_eq!(app.worktrees.len(), before, "a failed refresh leaves the list intact");
  assert!(!app.is_task_loading(), "the slot clears even on failure");
  assert!(
    app.status.contains("boom"),
    "the error reaches the status bar: {:?}",
    app.status
  );
}

fn sync_report_integrated(behind: usize) -> gwm::sync::SyncReport {
  gwm::sync::SyncReport {
    branch: "feat/#258-x".into(),
    upstream: "origin/main".into(),
    strategy: gwm::sync::SyncStrategy::Rebase,
    ahead_before: 0,
    behind_before: behind,
    action: gwm::sync::SyncAction::Integrated,
  }
}

#[test]
fn drain_applies_sync_report_and_reports_the_outcome() {
  // Issue #258: a `gwm sync` result delivered off-thread is applied by
  // `drain_task_results`, which re-lists the worktrees (so the new
  // ahead/behind shows) and reports the sync outcome on the status bar.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  // Claim the Sync slot exactly as `request_sync` would, without spawning.
  let generation = app.tasks.request(TaskKind::Sync).unwrap();
  assert!(app.is_task_loading(), "request must mark the app as loading");

  app
    .task_result_sender()
    .send(TaskMsg::Sync(generation, "alpha".into(), Ok(sync_report_integrated(3))))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "drain must report it applied a result");
  assert!(!app.is_task_loading(), "the sync slot clears after draining");
  assert!(
    app.status.contains("rebased 3 commits"),
    "status reports the sync outcome, not the refresh line: {:?}",
    app.status
  );
}

#[test]
fn drain_sync_failure_surfaces_on_the_status_bar() {
  // A refused/failed sync (dirty tree, no upstream, conflicts) surfaces as a
  // status message rather than tearing anything down.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let generation = app.tasks.request(TaskKind::Sync).unwrap();
  app
    .task_result_sender()
    .send(TaskMsg::Sync(
      generation,
      "alpha".into(),
      Err("branch 'feat/#258-x' has no upstream configured".into()),
    ))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "a failure still counts as a drained result");
  assert!(!app.is_task_loading(), "the slot clears even on failure");
  assert!(
    app.status.contains("sync failed") && app.status.contains("no upstream"),
    "the sync error reaches the status bar: {:?}",
    app.status
  );
}

#[test]
fn drain_drops_a_superseded_sync_result() {
  // The #138 guard on the sync path: a worker whose generation was bumped by
  // an intervening invalidate is dropped, leaving the status untouched.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let stale = app.tasks.request(TaskKind::Sync).unwrap();
  app.tasks.invalidate(TaskKind::Sync);
  app.status = "untouched".into();
  app
    .task_result_sender()
    .send(TaskMsg::Sync(stale, "alpha".into(), Ok(sync_report_integrated(2))))
    .unwrap();
  app.drain_task_results();
  assert_eq!(
    app.status, "untouched",
    "a sync result invalidated mid-flight must be dropped, not reported"
  );
}

#[test]
fn drain_delete_worktree_success_returns_to_list_and_reports_removed_target() {
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let generation = app.tasks.request(TaskKind::DeleteWorktree).unwrap();
  app.view = View::Confirm;
  app.delete_failure = Some("old failure".into());

  app
    .task_result_sender()
    .send(TaskMsg::DeleteWorktree(
      generation,
      "alpha".into(),
      "/tmp/alpha".into(),
      Ok(()),
    ))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "delete result should be applied");
  assert!(!app.is_delete_worktree_loading(), "delete slot clears after success");
  assert_eq!(app.view, View::List);
  assert!(app.delete_failure.is_none(), "old failure is cleared after success");
  assert!(
    app.status.contains("removed alpha") && app.status.contains("/tmp/alpha"),
    "status reports the removed target: {:?}",
    app.status
  );
}

#[test]
fn drain_delete_worktree_failure_stays_in_confirm_and_records_failure() {
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let generation = app.tasks.request(TaskKind::DeleteWorktree).unwrap();
  app.view = View::Confirm;

  app
    .task_result_sender()
    .send(TaskMsg::DeleteWorktree(
      generation,
      "alpha".into(),
      "/tmp/alpha".into(),
      Err("permission denied".into()),
    ))
    .unwrap();
  let applied = app.drain_task_results();

  assert!(applied, "delete failure should still be applied");
  assert!(!app.is_delete_worktree_loading(), "delete slot clears after failure");
  assert_eq!(app.view, View::Confirm);
  assert_eq!(app.delete_failure.as_deref(), Some("permission denied"));
  assert!(
    app.status.contains("delete failed") && app.status.contains("permission denied"),
    "status reports the delete failure: {:?}",
    app.status
  );
}

#[test]
fn drain_drops_a_superseded_delete_worktree_result() {
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let stale = app.tasks.request(TaskKind::DeleteWorktree).unwrap();
  app.tasks.invalidate(TaskKind::DeleteWorktree);
  app.view = View::Confirm;
  app.status = "untouched".into();

  app
    .task_result_sender()
    .send(TaskMsg::DeleteWorktree(
      stale,
      "alpha".into(),
      "/tmp/alpha".into(),
      Ok(()),
    ))
    .unwrap();
  app.drain_task_results();

  assert_eq!(app.view, View::Confirm);
  assert_eq!(app.status, "untouched");
}

#[test]
fn request_sync_coalesces_onto_an_inflight_run() {
  // A second `S` press while a sync is already in flight must not spawn a
  // second rebase — `request_sync` coalesces and returns early (zero threads).
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  // The main worktree is selected by default, so request_sync gets past the
  // selection check and reaches the coalesce branch.
  let generation = app.tasks.request(TaskKind::Sync).unwrap();
  app.request_sync(); // coalesced — no panic, no second worker
  assert!(app.is_task_loading());
  assert!(
    app.tasks.complete(TaskKind::Sync, generation),
    "the original sync is still authoritative after a coalesced press"
  );
}

#[test]
fn request_sync_with_no_selection_reports_and_does_not_claim_a_slot() {
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  // Drop the selection so there is no worktree to sync.
  app.list_state.select(None);
  app.request_sync();
  assert!(
    !app.tasks.is_loading(TaskKind::Sync),
    "with nothing selected, request_sync must not claim a sync slot"
  );
  assert!(
    app.status.contains("no worktree selected"),
    "request_sync reports the missing selection: {:?}",
    app.status
  );
}

#[test]
fn request_refresh_coalesces_onto_an_inflight_run() {
  // A second `f` press while a refresh is already in flight must not spawn a
  // second worker — `request_refresh` coalesces and returns early (so this
  // test spawns zero threads).
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  let generation = app.tasks.request(TaskKind::RefreshWorktrees).unwrap();
  app.request_refresh(); // coalesced — no panic, no second worker
  assert!(app.is_task_loading());
  assert!(
    app.tasks.complete(TaskKind::RefreshWorktrees, generation),
    "the original run is still the authoritative one after a coalesced press"
  );
}

#[test]
fn auto_refresh_triggers_after_default_interval_without_resetting_selection() {
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("alpha"), worktree_fixture("beta")];
  app.list_state.select(Some(1));
  let start = Instant::now();
  app.last_auto_refresh_at = start;

  assert!(
    !app.maybe_auto_refresh(start + Duration::from_secs(59)),
    "default interval is 60s, so 59s must not refresh"
  );
  assert_eq!(app.list_state.selected(), Some(1), "selection stays put before refresh");

  assert!(
    app.maybe_auto_refresh(start + Duration::from_secs(60)),
    "60s default interval should trigger a worktree refresh"
  );
  assert!(
    app.tasks.is_loading(TaskKind::RefreshWorktrees),
    "auto-refresh uses the async refresh task"
  );
  assert_eq!(
    app.list_state.selected(),
    Some(1),
    "requesting auto-refresh must not reset the user's selection"
  );
}

#[test]
fn auto_refresh_advances_timer_when_refresh_is_already_inflight() {
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  let start = Instant::now();
  app.last_auto_refresh_at = start;
  let generation = app.tasks.request(TaskKind::RefreshWorktrees).unwrap();
  let elapsed = start + Duration::from_secs(60);

  assert!(
    !app.maybe_auto_refresh(elapsed),
    "an in-flight refresh coalesces instead of spawning a second worker"
  );
  assert_eq!(
    app.last_auto_refresh_at, elapsed,
    "coalescing still advances the timer to avoid an immediate follow-up refresh"
  );
  assert!(
    app.tasks.complete(TaskKind::RefreshWorktrees, generation),
    "the original refresh remains authoritative"
  );
  assert!(
    !app.maybe_auto_refresh(elapsed + Duration::from_secs(1)),
    "the next event-loop tick should not immediately start another auto-refresh"
  );
}

#[test]
fn auto_refresh_zero_is_disabled() {
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  app.config.tui.auto_refresh_secs = 0;
  let start = Instant::now();
  app.last_auto_refresh_at = start;

  assert!(
    !app.maybe_auto_refresh(start + Duration::from_secs(3600)),
    "auto_refresh_secs = 0 disables periodic refresh"
  );
  assert!(
    !app.tasks.is_loading(TaskKind::RefreshWorktrees),
    "disabled auto-refresh must not claim a refresh task"
  );
}

#[test]
fn quit_waits_while_a_sync_task_is_in_flight() {
  use gwm::tui::state::async_task::TaskKind;

  let (_dir, mut app) = make_app();
  app.should_quit = true;
  app.tasks.request(TaskKind::Sync).unwrap();

  assert!(!app.can_quit_now());
}

#[test]
fn quit_waits_while_a_create_worktree_task_is_in_flight() {
  use gwm::tui::state::async_task::TaskKind;

  let (_dir, mut app) = make_app();
  app.should_quit = true;
  app.tasks.request(TaskKind::CreateWorktree).unwrap();

  assert!(!app.can_quit_now());
  app.defer_quit_for_mutating_task();
  assert_eq!(app.status, "finishing creating worktree before quit…");
}

#[test]
fn quit_waits_while_a_bootstrap_task_is_in_flight() {
  use gwm::tui::state::async_task::TaskKind;

  let (_dir, mut app) = make_app();
  app.should_quit = true;
  app.tasks.request(TaskKind::Bootstrap).unwrap();

  assert!(!app.can_quit_now());
}

#[test]
fn quit_waits_while_a_delete_worktree_task_is_in_flight() {
  use gwm::tui::state::async_task::TaskKind;

  let (_dir, mut app) = make_app();
  app.should_quit = true;
  app.tasks.request(TaskKind::DeleteWorktree).unwrap();

  assert!(!app.can_quit_now());
  app.defer_quit_for_mutating_task();
  assert_eq!(app.status, "finishing deleting worktree before quit…");
}

#[test]
fn quit_does_not_wait_for_read_only_tasks() {
  use gwm::tui::state::async_task::TaskKind;

  let (_dir, mut app) = make_app();
  app.should_quit = true;
  app.tasks.request(TaskKind::RefreshWorktrees).unwrap();
  app.tasks.request(TaskKind::GithubIssue(42)).unwrap();
  app.tasks.request(TaskKind::GithubPr(7)).unwrap();

  assert!(app.can_quit_now());
}

#[test]
fn quit_waiting_status_explains_the_deferred_exit() {
  use gwm::tui::state::async_task::TaskKind;

  let (_dir, mut app) = make_app();
  app.should_quit = true;
  app.tasks.request(TaskKind::Bootstrap).unwrap();

  assert!(!app.can_quit_now());
  app.defer_quit_for_mutating_task();

  assert_eq!(app.status, "finishing bootstrapping before quit…");
}

#[test]
fn sync_refresh_invalidates_an_inflight_async_refresh() {
  // Issue #231 race guard: a synchronous `refresh()` (the create / delete /
  // report-close path) must bump the task generation so a still-in-flight
  // async refresh — spawned with a *pre-mutation* snapshot — is dropped
  // rather than clobbering the authoritative post-mutation list.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  // An async refresh is in flight (claimed exactly as `request_refresh` would).
  let stale = app.tasks.request(TaskKind::RefreshWorktrees).unwrap();
  // A delete/create lands and re-lists synchronously while the worker runs.
  app.refresh().unwrap();
  let authoritative = app.worktrees.len();
  // The pre-mutation worker now reports its stale snapshot.
  app
    .task_result_sender()
    .send(TaskMsg::RefreshWorktrees(stale, Ok(vec![worktree_fixture("ghost")])))
    .unwrap();
  app.drain_task_results();
  assert_eq!(
    app.worktrees.len(),
    authoritative,
    "the stale pre-mutation snapshot must not replace the sync-refreshed list"
  );
  assert!(
    !app.worktrees.iter().any(|w| w.name == "ghost"),
    "the dropped result's payload must never reach the list"
  );
}

// ---------------------------------------------------------------------------
// Editable Settings panel — apply-live + persistence (issue #279)
// ---------------------------------------------------------------------------

#[test]
fn activate_choice_setting_persists_project_layer_and_applies_live() {
  use gwm::config::{Config, SidebarPosition};
  use gwm::tui::SettingsTab;

  let (dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Tui;
  app.config_panel.selected = 0; // sidebar position
  assert_eq!(app.config.tui.sidebar_position, SidebarPosition::Right);

  // Cycle the choice: right → left, written to the project `.gwm.toml` and
  // applied live.
  app.activate_selected_setting();

  assert_eq!(
    app.config.tui.sidebar_position,
    SidebarPosition::Left,
    "live config updated"
  );
  assert_eq!(
    app.sidebar.position,
    SidebarPosition::Left,
    "live sidebar position re-seeded"
  );

  let written = std::fs::read_to_string(dir.path().join(".gwm.toml")).unwrap();
  assert!(
    written.contains("sidebar_position"),
    "edit persisted to .gwm.toml: {written}"
  );
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(
    cfg.tui.sidebar_position,
    SidebarPosition::Left,
    "edit round-trips through a fresh layered load"
  );
}

#[test]
fn sidebar_orientation_is_seeded_from_config_at_construction() {
  // #365: `[tui] sidebar_orientation` drives the launch layout. Without
  // the seed the key parses but the TUI ignores it, which is the bug the
  // issue reports.
  use gwm::config::SidebarOrientation;

  let (repo, _) = init_repo();
  std::fs::write(
    repo.path().join(".gwm.toml"),
    r#"
[tui]
sidebar_orientation = "side-by-side"
"#,
  )
  .unwrap();

  let app = App::new_at_layered(Some(repo.path()), None).unwrap();
  assert_eq!(
    app.sidebar.orientation,
    SidebarOrientation::SideBySide,
    "config orientation must reach the live sidebar state"
  );
}

#[test]
fn sidebar_orientation_defaults_to_stacked_without_config() {
  // Guards the #217 launch layout against an accidental flip while
  // wiring the knob up (#365).
  use gwm::config::SidebarOrientation;

  let (_dir, app) = make_app();
  assert_eq!(app.sidebar.orientation, SidebarOrientation::Stacked);
}

#[test]
fn activate_sidebar_orientation_setting_persists_and_reseeds_live() {
  use gwm::config::{Config, SidebarOrientation};
  use gwm::tui::{SettingField, SettingsTab};

  let (dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Tui;
  app.config_panel.selected = tui_field_index(SettingField::SidebarOrientation);
  assert_eq!(app.config.tui.sidebar_orientation, SidebarOrientation::Stacked);

  // Cycle the choice: the write-back must round-trip through a fresh load
  // *and* re-seed the live sidebar, same contract as sidebar_position.
  app.activate_selected_setting();

  assert_ne!(
    app.config.tui.sidebar_orientation,
    SidebarOrientation::Stacked,
    "live config updated"
  );
  assert_eq!(
    app.sidebar.orientation, app.config.tui.sidebar_orientation,
    "live sidebar orientation re-seeded from the edited config"
  );

  let written = std::fs::read_to_string(dir.path().join(".gwm.toml")).unwrap();
  assert!(
    written.contains("sidebar_orientation"),
    "edit persisted to .gwm.toml: {written}"
  );
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(
    cfg.tui.sidebar_orientation, app.config.tui.sidebar_orientation,
    "edit round-trips through a fresh layered load"
  );
}

#[test]
fn committing_numeric_input_persists_the_typed_value() {
  use gwm::config::Config;
  use gwm::tui::{SettingField, SettingsTab};

  let (dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Tui;
  app.config_panel.selected = tui_field_index(SettingField::ConfirmCountdown);

  // Arm the input, retype "5", commit.
  app.activate_selected_setting();
  assert!(
    app.config_panel.editing.is_some(),
    "Enter on a Uint field opens the input"
  );
  app.config_panel.editing = Some("5".into());
  app.commit_settings_edit();

  assert!(app.config_panel.editing.is_none(), "commit closes the input");
  assert_eq!(app.config.tui.confirm_countdown_secs, 5, "live config updated");
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.confirm_countdown_secs, 5, "typed value persisted");
}

#[test]
fn committing_auto_refresh_secs_persists_the_typed_value() {
  use gwm::config::Config;
  use gwm::tui::{SettingField, SettingsTab};

  let (dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Tui;
  app.config_panel.selected = tui_field_index(SettingField::AutoRefreshSecs);

  app.activate_selected_setting();
  assert!(
    app.config_panel.editing.is_some(),
    "Enter on auto refresh opens the numeric input"
  );
  app.config_panel.editing = Some("0".into());
  app.commit_settings_edit();

  assert_eq!(app.config.tui.auto_refresh_secs, 0, "live config updated");
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.tui.auto_refresh_secs, 0, "typed value persisted");
}

#[test]
fn committing_text_input_persists_a_worktree_pattern() {
  use gwm::config::Config;
  use gwm::tui::SettingsTab;

  let (dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Worktree;
  app.config_panel.selected = 0; // base directory (Text input)

  app.activate_selected_setting();
  assert!(
    app.config_panel.editing.is_some(),
    "Enter on a Text field opens the input"
  );
  app.config_panel.editing = Some("{home}/custom-wt/{repo}".into());
  app.commit_settings_edit();

  assert_eq!(
    app.config.worktree.base, "{home}/custom-wt/{repo}",
    "live config updated"
  );
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.worktree.base, "{home}/custom-wt/{repo}", "text value persisted");
}

#[test]
fn committing_numeric_looking_text_persists_as_a_string() {
  // Review P2: a Text field whose value looks like a number must round-trip
  // as a string through the typed load, not be coerced to an int.
  use gwm::config::Config;
  use gwm::tui::SettingsTab;

  let (dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Worktree;
  app.config_panel.selected = 0; // base directory (Text input)

  app.activate_selected_setting();
  app.config_panel.editing = Some("404".into());
  app.commit_settings_edit();

  assert_eq!(app.config.worktree.base, "404", "live config keeps the text value");
  let cfg = Config::load_layered(dir.path(), None).unwrap();
  assert_eq!(cfg.worktree.base, "404", "numeric-looking text persisted as a string");
}

#[test]
fn command_logs_transcript_is_newest_first_and_empty_when_blank() {
  use gwm::command_log::{CommandLogEntry, CommandStatus};
  use std::time::Duration;

  let (_dir, mut app) = make_app();
  // Empty transcript when nothing has run.
  assert!(app.command_logs_transcript().is_empty());

  app.command_logs.entries = vec![
    CommandLogEntry {
      command: "first cmd".into(),
      duration: Duration::from_millis(10),
      status: CommandStatus::Exited(Some(0)),
      output: "ok".into(),
    },
    CommandLogEntry {
      command: "second cmd".into(),
      duration: Duration::from_millis(20),
      status: CommandStatus::Exited(Some(2)),
      output: "boom".into(),
    },
  ];
  let t = app.command_logs_transcript();
  assert!(
    t.contains("$ first cmd") && t.contains("$ second cmd"),
    "both argv present: {t}"
  );
  // Newest-first: the last-pushed entry leads the transcript.
  assert!(
    t.find("second cmd").unwrap() < t.find("first cmd").unwrap(),
    "newest entry must come first: {t}"
  );
  assert!(t.contains("→ exit 2"), "non-zero exit is recorded: {t}");
  assert!(
    t.contains("boom") && t.contains("ok"),
    "captured output is included: {t}"
  );
}

#[test]
fn activate_is_a_noop_on_the_read_only_all_tab() {
  use gwm::tui::SettingsTab;
  let (dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::All;
  app.activate_selected_setting();
  // Nothing written: the All tab has no editable field.
  assert!(
    !dir.path().join(".gwm.toml").exists(),
    "the read-only All tab must not write anything"
  );
}

// ---- Edit-worktree modal (rename, #290) ----------------------------------

#[test]
fn enter_edit_worktree_prefills_create_form_from_branch() {
  // `c` opens the rename modal by decomposing the current branch into the
  // Create form (Type / Issue / Desc), so renaming is symmetric with create.
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("fix/#42-broken-thing".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::Edit);
  assert_eq!(app.create_form.issue, "42");
  assert_eq!(app.create_form.desc, "broken-thing");
  assert_eq!(
    app.branch_types[app.create_form.type_index].name, "fix",
    "the type selector must point at the parsed branch type"
  );
  assert_eq!(app.edit_original_branch.as_deref(), Some("fix/#42-broken-thing"));
  assert!(
    app.edit_original_path.is_some(),
    "the original path is captured for git worktree move"
  );
}

#[test]
fn enter_edit_worktree_rejects_unparseable_branch() {
  // A branch that isn't `<type>/#<issue>-<desc>` (e.g. `main`) can't be
  // decomposed into the form, so the modal refuses to open.
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("main".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::List, "unparseable branch must not open the modal");
  assert!(app.edit_original_branch.is_none());
}

#[test]
fn cancel_edit_worktree_resets_state() {
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#1-x".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit);

  app.cancel_edit_worktree();

  assert_eq!(app.view, View::List);
  assert!(app.edit_original_branch.is_none());
  assert!(app.edit_original_path.is_none());
}

#[test]
fn request_push_refuses_while_another_mutation_runs() {
  // Codex review on PR #292: pressing `P` while a different mutating task is
  // in flight must not start a concurrent push in the same worktree.
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#1-x".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  // A sync is already running (a different mutating kind).
  app.tasks.request(TaskKind::Sync);

  app.request_push();

  assert!(
    !app.tasks.is_loading(TaskKind::Push),
    "push must not start while a sync is in flight"
  );
  assert!(
    app.status.contains("before pushing"),
    "status must explain the block: {}",
    app.status
  );
}

#[test]
fn request_pull_refuses_while_another_mutation_runs() {
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#1-x".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.tasks.request(TaskKind::Bootstrap);

  app.request_pull();

  assert!(
    !app.tasks.is_loading(TaskKind::Pull),
    "pull must not start while a bootstrap is in flight"
  );
  assert!(
    app.status.contains("before pulling"),
    "status must explain the block: {}",
    app.status
  );
}

#[test]
fn enter_edit_worktree_refuses_unconfigured_branch_type() {
  // Codex review on PR #292: a branch whose parsed type is not in branch_types
  // must refuse to open the rename modal rather than silently preselecting the
  // first configured type (which Enter would then rename the branch to).
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  // `zzz` is a well-formed <type>/#<issue>-<desc> but not a configured type.
  wt.branch = Some("zzz/#7-thing".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::List, "unconfigured type must not open the modal");
  assert!(app.edit_original_branch.is_none());
  assert!(
    app.status.contains("not configured"),
    "status must explain: {}",
    app.status
  );
}

#[test]
fn request_sync_refuses_while_another_mutation_runs() {
  // Codex review on PR #292: the concurrent-mutation guard is centralized, so
  // sync also refuses to start while a different mutating task is in flight.
  use gwm::tui::state::async_task::TaskKind;
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#1-x".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.tasks.request(TaskKind::Pull);

  app.request_sync();

  assert!(
    !app.tasks.is_loading(TaskKind::Sync),
    "sync must not start while a pull is in flight"
  );
  assert!(
    app.status.contains("before syncing"),
    "status must explain: {}",
    app.status
  );
}

// --- contextual modal rebinding (issue #219) -----------------------------

#[test]
fn create_modal_honours_a_rebound_submit_key() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  use gwm::tui::CreateKey;

  let (_dir, mut app) = make_app();
  // Rebind create.submit from Enter to F2.
  app
    .modal_keymap
    .apply_override(
      ModalAction::CreateSubmit,
      vec![KeyStroke::new(KeyCode::F(2), KeyModifiers::empty())],
    )
    .unwrap();

  // Advance Type -> Issue -> Desc using the default `next_field` (Tab).
  app.handle_create_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
  app.handle_create_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
  assert_eq!(app.create_form.field, Field::Desc);

  // The rebound key submits…
  assert_eq!(
    app.handle_create_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
    CreateKey::Submit
  );
  // …and the old default Enter no longer submits (it is unbound now).
  assert_eq!(
    app.handle_create_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    CreateKey::Handled
  );
}

#[test]
fn create_modal_type_cycle_keys_stay_literal_on_text_fields() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::CreateKey;

  let (_dir, mut app) = make_app();
  // On the Desc field, `l` is literal text — it must NOT cycle the type
  // (the default next_type binding includes `l`, gated on the Type field).
  app.handle_create_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // Issue
  app.handle_create_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)); // Desc
  assert_eq!(app.create_form.field, Field::Desc);
  let before = app.create_form.type_index;
  assert_eq!(
    app.handle_create_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)),
    CreateKey::Handled
  );
  assert_eq!(
    app.create_form.type_index, before,
    "type must not cycle while typing a description"
  );
  assert!(
    app.create_form.desc.contains('l'),
    "`l` must reach the description buffer as literal text"
  );
}

#[test]
fn resolve_modal_reflects_a_confirm_rebind() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::{KeyContext, ModalAction};

  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(
      ModalAction::ConfirmConfirm,
      vec![KeyStroke::new(KeyCode::Char('o'), KeyModifiers::empty())],
    )
    .unwrap();
  // The inline confirm routing resolves through App::resolve_modal, so a
  // rebind is observable there: `o` now means confirm, `y` no longer does.
  assert_eq!(
    app.resolve_modal(
      KeyContext::Confirm,
      KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)
    ),
    Some(ModalAction::ConfirmConfirm)
  );
  assert_eq!(
    app.resolve_modal(
      KeyContext::Confirm,
      KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)
    ),
    None
  );
}

#[test]
fn link_input_number_context_advertises_its_own_hints() {
  // #219 review: while typing the number, the hints must resolve submit /
  // cancel from `[tui.keys.modal.link.input_number]` (including a rebind), not the
  // choose-target keys.
  use crossterm::event::{KeyCode, KeyModifiers};
  use gwm::tui::keymap::{KeyStroke, Keymap};
  use gwm::tui::modal_keymap::{ModalAction, ModalKeymap};
  use gwm::tui::HintContext;

  let mut modal = ModalKeymap::defaults();
  modal
    .apply_override(
      ModalAction::LinkInputSubmit,
      vec![KeyStroke::new(KeyCode::Char('x'), KeyModifiers::empty())],
    )
    .unwrap();
  let resolved = HintContext::LinkInputNumber.resolve(&Keymap::defaults(), &modal);
  assert!(
    resolved.iter().any(|(k, l)| l == "submit" && k == "x"),
    "submit hint must show the rebound key, got {resolved:?}"
  );
  assert!(
    !resolved.iter().any(|(_, l)| l == "kind" || l == "move"),
    "the input-number stage must not advertise choose-target hints: {resolved:?}"
  );
}

#[test]
fn hint_context_switches_to_link_input_number_while_typing() {
  use gwm::tui::HintContext;
  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app.enter_link_prompt();
  assert_eq!(app.hint_context(), HintContext::LinkPrompt, "choose-target stage");
  // Commit a target → InputNumber stage; the statusbar context must follow.
  app.link_prompt_choose(LinkTarget::Issue);
  assert_eq!(app.link_prompt_stage(), LinkPromptStage::InputNumber);
  assert_eq!(app.hint_context(), HintContext::LinkInputNumber, "number-input stage");
}

#[test]
fn link_modal_binding_on_fetch_key_wins_over_fetch_fallback() {
  // #293 review: the global fetch shortcut is a FALLBACK after the stage
  // context, so a contextual binding on that key is reachable. Rebinding the
  // number-input submit onto `F` (also the default fetch key) must submit,
  // not refresh.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  use gwm::tui::LinkPromptKey;

  let (_dir, _repo, mut app) = make_app_on_branch("random-branch");
  app
    .modal_keymap
    .apply_override(
      ModalAction::LinkInputSubmit,
      vec![KeyStroke::new(KeyCode::Char('F'), KeyModifiers::empty())],
    )
    .unwrap();
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue); // → InputNumber stage
  assert!(
    matches!(
      app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::NONE)),
      LinkPromptKey::Submit
    ),
    "a contextual binding on the fetch key must win over the fetch fallback"
  );
}

// ── Exec picker overlay (issue #325) ──────────────────────────────────────

/// An `App` over a fresh repo whose `.gwm.toml` carries `toml`, so the exec
/// config is loaded at construction. The selected worktree is the main one
/// (the repo root) — a valid `cwd` for an exec run.
fn app_with_gwm_toml(toml: &str) -> (tempfile::TempDir, App) {
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gwm.toml"), toml).unwrap();
  let app = App::new_at_layered(Some(repo.path()), None).unwrap();
  (repo, app)
}

#[test]
fn exec_picker_refuses_to_open_with_no_profiles() {
  let (repo, _) = init_repo();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_exec_picker();
  assert_eq!(app.view, View::List, "no profiles ⇒ no transition");
  assert!(
    app.status.contains("exec.profiles"),
    "the status explains why nothing opened: {}",
    app.status
  );
}

#[test]
fn exec_picker_opens_and_lists_profiles_sorted() {
  let (_repo, mut app) = app_with_gwm_toml(
    "[exec.profiles.test]\ncommand = [\"cargo\", \"test\"]\n\n[exec.profiles.build]\ncommand = [\"cargo\", \"build\"]\n",
  );
  app.enter_exec_picker();
  assert_eq!(app.view, View::ExecPicker);
  // `BTreeMap` key order ⇒ alphabetical: build, then test.
  assert_eq!(app.exec_picker.profiles(), &["build".to_string(), "test".to_string()]);
  assert_eq!(app.exec_picker.selected_profile(), Some("build"));
}

#[test]
fn exec_picker_navigation_moves_the_highlight() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::ExecPickerKey;
  let (_repo, mut app) =
    app_with_gwm_toml("[exec.profiles.a]\ncommand = [\"true\"]\n\n[exec.profiles.b]\ncommand = [\"true\"]\n");
  app.enter_exec_picker();
  let down = app.handle_exec_picker_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
  assert_eq!(down, ExecPickerKey::Handled);
  assert_eq!(app.exec_picker.selected_profile(), Some("b"));
  app.handle_exec_picker_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
  assert_eq!(app.exec_picker.selected_profile(), Some("a"));
}

#[test]
fn exec_picker_enter_submits_and_esc_cancels() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::ExecPickerKey;
  let (_repo, mut app) = app_with_gwm_toml("[exec.profiles.a]\ncommand = [\"true\"]\n");
  app.enter_exec_picker();
  assert_eq!(
    app.handle_exec_picker_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    ExecPickerKey::Submit
  );
  assert_eq!(
    app.handle_exec_picker_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    ExecPickerKey::Cancel
  );
}

#[test]
fn exec_picker_resolves_the_highlighted_profile_to_its_argv() {
  let (_repo, mut app) = app_with_gwm_toml("[exec.profiles.build]\ncommand = [\"cargo\", \"build\", \"--release\"]\n");
  app.enter_exec_picker();
  let expected_cwd = app.selected().unwrap().path.clone();
  let (argv, cwd) = app.exec_picker_resolve().expect("a valid profile resolves");
  assert_eq!(
    argv,
    vec!["cargo".to_string(), "build".to_string(), "--release".to_string()],
    "argv is the frozen command array verbatim (no shell)"
  );
  assert_eq!(cwd, expected_cwd, "cwd is the selected worktree's path");
}

#[test]
fn exec_picker_close_returns_to_the_list() {
  let (_repo, mut app) = app_with_gwm_toml("[exec.profiles.a]\ncommand = [\"true\"]\n");
  app.enter_exec_picker();
  assert_eq!(app.view, View::ExecPicker);
  app.close_exec_picker();
  assert_eq!(app.view, View::List);
}

// ── Clean overlay (issue #325) ────────────────────────────────────────────

#[test]
fn clean_overlay_scans_gitignored_artifacts() {
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "target/\n").unwrap();
  std::fs::create_dir(repo.path().join("target")).unwrap();
  std::fs::write(repo.path().join("target").join("blob"), vec![0u8; 2048]).unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  assert_eq!(app.view, View::CleanReport);
  let reclaim = app.clean_overlay.reclaim().expect("the worktree was scanned");
  assert!(
    reclaim.artifacts.iter().any(|a| a.rel == "target"),
    "the git-ignored target/ is counted: {:?}",
    reclaim.artifacts
  );
  assert!(app.clean_overlay.total_bytes() >= 2048);
}

#[test]
fn clean_overlay_gate_skips_non_gitignored_artifacts() {
  let (repo, _) = init_repo();
  // No `.gitignore` ⇒ `target/` is NOT ignored ⇒ the safety gate preserves it.
  std::fs::create_dir(repo.path().join("target")).unwrap();
  std::fs::write(repo.path().join("target").join("blob"), vec![0u8; 1024]).unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  let reclaim = app.clean_overlay.reclaim().expect("scanned");
  assert!(reclaim.artifacts.is_empty(), "a non-ignored target/ is never counted");
  assert!(
    app.clean_overlay.skipped().contains(&"target".to_string()),
    "and is reported as skipped: {:?}",
    app.clean_overlay.skipped()
  );
  assert_eq!(app.clean_overlay.total_bytes(), 0);
}

#[test]
fn clean_overlay_delete_reclaims_only_the_gated_dir() {
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "build/\n").unwrap();
  std::fs::create_dir(repo.path().join("build")).unwrap();
  std::fs::write(repo.path().join("build").join("out"), vec![0u8; 512]).unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  assert!(repo.path().join("build").exists());
  app.clean_overlay_delete();
  assert!(!repo.path().join("build").exists(), "the build dir was reclaimed");
  assert_eq!(app.view, View::List, "and the overlay closed");
  assert!(
    app.status.contains("reclaimed"),
    "status reports the reclaim: {}",
    app.status
  );
}

#[test]
fn clean_confirm_arms_then_is_ready_after_the_countdown() {
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "dist/\n").unwrap();
  std::fs::create_dir(repo.path().join("dist")).unwrap();
  std::fs::write(repo.path().join("dist").join("x"), vec![0u8; 100]).unwrap();
  std::fs::write(repo.path().join(".gwm.toml"), "[tui]\nconfirm_countdown_secs = 3\n").unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  let t0 = Instant::now();
  assert_eq!(app.clean_confirm_press(t0), ConfirmKeyAction::Armed);
  assert!(app.clean_overlay.confirm.is_armed());
  assert_eq!(
    app.tick_clean_countdown(t0 + Duration::from_secs(1)),
    CountdownTickOutcome::Pending
  );
  assert_eq!(
    app.tick_clean_countdown(t0 + Duration::from_secs(3)),
    CountdownTickOutcome::ReadyToFire
  );
}

#[test]
fn clean_confirm_with_zero_countdown_fires_immediately() {
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "node_modules/\n").unwrap();
  std::fs::create_dir(repo.path().join("node_modules")).unwrap();
  std::fs::write(repo.path().join("node_modules").join("y"), vec![0u8; 64]).unwrap();
  std::fs::write(repo.path().join(".gwm.toml"), "[tui]\nconfirm_countdown_secs = 0\n").unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  assert_eq!(app.clean_confirm_press(Instant::now()), ConfirmKeyAction::FireNow);
}

#[test]
fn clean_confirm_is_a_noop_when_nothing_to_reclaim() {
  let (repo, _) = init_repo(); // no artifact dirs ⇒ empty scan
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  assert!(app.clean_overlay.is_empty_scan());
  assert_eq!(app.clean_confirm_press(Instant::now()), ConfirmKeyAction::Disarmed);
  assert!(app.status.contains("nothing to reclaim"), "status: {}", app.status);
}

#[test]
fn clean_overlay_profile_picker_rescans_per_profile() {
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "cache/\nout/\n").unwrap();
  std::fs::create_dir(repo.path().join("cache")).unwrap();
  std::fs::write(repo.path().join("cache").join("c"), vec![0u8; 100]).unwrap();
  std::fs::create_dir(repo.path().join("out")).unwrap();
  std::fs::write(repo.path().join("out").join("o"), vec![0u8; 200]).unwrap();
  std::fs::write(
    repo.path().join(".gwm.toml"),
    "[clean.profiles.a]\ndirs = [\"cache\"]\n\n[clean.profiles.b]\ndirs = [\"out\"]\n",
  )
  .unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  // Opens on the `(default)` choice (no --profile) — built-in set here, which
  // doesn't include `cache` / `out`.
  assert_eq!(app.clean_overlay.selected_profile(), None);
  // Cycle to profile `a` → re-scans for `cache` only.
  app.clean_overlay_next();
  assert_eq!(app.clean_overlay.selected_profile(), Some("a"));
  let a = app.clean_overlay.reclaim().unwrap();
  assert!(a.artifacts.iter().any(|x| x.rel == "cache"));
  assert!(!a.artifacts.iter().any(|x| x.rel == "out"));
  // Cycle to profile `b` → re-scans for `out`.
  app.clean_overlay_next();
  assert_eq!(app.clean_overlay.selected_profile(), Some("b"));
  let b = app.clean_overlay.reclaim().unwrap();
  assert!(b.artifacts.iter().any(|x| x.rel == "out"));
  assert!(!b.artifacts.iter().any(|x| x.rel == "cache"));
}

#[test]
fn clean_overlay_close_returns_to_the_list() {
  let (repo, _) = init_repo();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  assert_eq!(app.view, View::CleanReport);
  app.close_clean_overlay();
  assert_eq!(app.view, View::List);
}

#[test]
fn clean_overlay_opens_on_the_no_profile_default_choice() {
  // The overlay always opens on the `(default)` / no-`--profile` choice, so
  // its first preview matches `gwm clean` — even when the repo defines named
  // profiles that sort before it.
  let (repo, _) = init_repo();
  std::fs::write(
    repo.path().join(".gwm.toml"),
    "[clean.profiles.aggressive]\ndirs = [\"target\"]\n",
  )
  .unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  assert_eq!(
    app.clean_overlay.selected_profile(),
    None,
    "opens on the no-profile choice"
  );
  assert_eq!(app.clean_overlay.choice_labels().first().copied(), Some("(default)"));
  assert!(
    app.clean_overlay.has_profiles(),
    "a named profile makes the picker worth showing"
  );
}

#[test]
fn clean_overlay_default_choice_uses_builtins_without_a_default_profile() {
  // Codex #333 review: a repo with only a non-`default` profile must not make
  // the built-in set unreachable. The `(default)` choice resolves to the
  // built-in `target` / … set (matching `gwm clean` with no --profile), NOT
  // the alphabetically-first profile.
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "target/\ncoverage/\n").unwrap();
  // `target` is a built-in; `coverage` is only reachable via the named profile.
  std::fs::create_dir(repo.path().join("target")).unwrap();
  std::fs::write(repo.path().join("target").join("t"), vec![0u8; 128]).unwrap();
  std::fs::create_dir(repo.path().join("coverage")).unwrap();
  std::fs::write(repo.path().join("coverage").join("c"), vec![0u8; 256]).unwrap();
  std::fs::write(
    repo.path().join(".gwm.toml"),
    "[clean.profiles.coverage]\ndirs = [\"coverage\"]\n",
  )
  .unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  // The `(default)` choice scans the built-in set → finds `target`, not `coverage`.
  let r = app.clean_overlay.reclaim().unwrap();
  assert!(
    r.artifacts.iter().any(|a| a.rel == "target"),
    "built-in target/ is reachable"
  );
  assert!(
    !r.artifacts.iter().any(|a| a.rel == "coverage"),
    "the named profile is not the default"
  );
}

#[test]
fn clean_overlay_delete_revalidates_the_gate_just_before_removing() {
  // Codex #333 review (TOCTOU): the overlay scans on open, but the delete can
  // fire seconds later (countdown / overlay left open). If a dir turned unsafe
  // meanwhile, the delete must re-gate and preserve it — not destroy the now
  // tracked file the stale snapshot still lists.
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "target/\n").unwrap();
  std::fs::create_dir(repo.path().join("target")).unwrap();
  std::fs::write(repo.path().join("target").join("blob"), vec![0u8; 256]).unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  // The scan saw a safe (ignored, untracked) `target/`.
  assert!(app
    .clean_overlay
    .reclaim()
    .unwrap()
    .artifacts
    .iter()
    .any(|a| a.rel == "target"));
  // Now force-track a file under it — the snapshot is stale.
  std::process::Command::new("git")
    .arg("-C")
    .arg(repo.path())
    .args(["add", "-f", "target/blob"])
    .status()
    .unwrap();
  app.clean_overlay_delete();
  // The re-gate catches the now-tracked dir and refuses to remove it.
  assert!(
    repo.path().join("target").exists(),
    "a directory that became tracked after the scan must not be reclaimed"
  );
  assert_eq!(app.view, View::List, "the overlay still closes");
}

#[test]
fn exec_picker_runs_in_the_open_time_worktree_and_config_after_a_drift() {
  // Codex #333: an auto-refresh can drift the live selection AND (workspace)
  // swap the active config while the picker is open. Resolve must run in the
  // captured worktree against the captured `[exec]` config — not the live
  // ones.
  let (_repo, mut app) = app_with_gwm_toml("[exec.profiles.a]\ncommand = [\"echo\", \"hi\"]\n");
  let opened = app.selected().unwrap().path.clone();
  app.enter_exec_picker();
  // Drift: the list AND the live config moved out from under the overlay.
  app.worktrees = vec![worktree_fixture("other")];
  app.config.exec.profiles.clear();
  let (argv, cwd) = app
    .exec_picker_resolve()
    .expect("resolves against the captured cfg + cwd");
  assert_eq!(argv, vec!["echo".to_string(), "hi".to_string()]);
  assert_eq!(cwd, opened, "runs in the open-time worktree, not the drifted selection");
}

#[test]
fn clean_overlay_deletes_the_open_time_target_and_config_after_a_drift() {
  // Codex #333: the clean delete must reclaim the previewed worktree using the
  // captured `[clean]` config, even if the live selection AND config drifted
  // (workspace refresh) meanwhile.
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "build/\n").unwrap();
  std::fs::create_dir(repo.path().join("build")).unwrap();
  std::fs::write(repo.path().join("build").join("o"), vec![0u8; 64]).unwrap();
  std::fs::write(
    repo.path().join(".gwm.toml"),
    "[clean.profiles.x]\ndirs = [\"build\"]\n",
  )
  .unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  app.clean_overlay_next(); // select profile `x` (dirs = build)
  assert_eq!(app.clean_overlay.selected_profile(), Some("x"));
  // Drift: replace the list AND drop the live config's profile.
  app.worktrees = vec![worktree_fixture("other")];
  app.config.clean.profiles.clear();
  app.clean_overlay_delete();
  assert!(
    !repo.path().join("build").exists(),
    "reclaimed via the captured target + config despite the drift"
  );
}

#[test]
fn exec_picker_pins_a_worktree_relative_program_to_the_target() {
  // Codex #333: a `[exec.profiles].command` like `./run.sh` must resolve
  // against the captured worktree (like the CLI's resolve_program), not gwm's
  // own cwd.
  let (_repo, mut app) = app_with_gwm_toml("[exec.profiles.run]\ncommand = [\"./run.sh\", \"--ci\"]\n");
  let wt = app.selected().unwrap().path.clone();
  app.enter_exec_picker();
  let (argv, _cwd) = app.exec_picker_resolve().expect("resolves");
  // Same anchoring the CLI exec path applies — an absolute path under the
  // worktree, not gwm's own cwd.
  assert_eq!(
    argv[0],
    gwm::exec::resolve_program(&wt, "./run.sh").to_string_lossy(),
    "the relative executable is pinned to the worktree"
  );
  assert!(std::path::Path::new(&argv[0]).is_absolute(), "and is absolute");
  assert_eq!(argv[1], "--ci", "the args are passed through unchanged");
}

#[test]
fn exec_picker_leaves_a_bare_command_for_path_lookup() {
  // A bare command (no path separator) must NOT be anchored — it relies on
  // PATH resolution inside the worktree.
  let (_repo, mut app) = app_with_gwm_toml("[exec.profiles.t]\ncommand = [\"cargo\", \"test\"]\n");
  app.enter_exec_picker();
  let (argv, _cwd) = app.exec_picker_resolve().expect("resolves");
  assert_eq!(argv, vec!["cargo".to_string(), "test".to_string()]);
}

#[test]
fn clean_countdown_is_pinned_to_the_open_time_config() {
  // Codex #333: the safety delay is captured at open, so a workspace config
  // swap (e.g. to a repo with confirm_countdown_secs = 0) can't erase it
  // while the overlay is armed.
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gwm.toml"), "[tui]\nconfirm_countdown_secs = 3\n").unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  // Drift: a refresh swapped in a repo with no safety delay.
  app.config.tui.confirm_countdown_secs = 0;
  assert_eq!(
    app.clean_countdown_total(),
    Duration::from_secs(3),
    "the safety delay captured at open survives a live config swap"
  );
}

#[test]
fn destructive_overlay_open_flags_exec_and_clean_views() {
  // Codex #333: the run loop suspends auto-refresh / active-repo sync while a
  // destructive overlay is open, gating on this predicate so the captured
  // exec/clean target can't drift mid-overlay.
  let (_repo, mut app) = make_app();
  assert!(
    !app.destructive_overlay_open(),
    "list view is not a destructive overlay"
  );
  app.view = View::ExecPicker;
  assert!(app.destructive_overlay_open());
  app.view = View::CleanReport;
  assert!(app.destructive_overlay_open());
  // The delete-confirm modal is not in this class — it has no captured target
  // to protect and reads the live selection by design.
  app.view = View::Confirm;
  assert!(!app.destructive_overlay_open());
}

#[test]
fn clean_overlay_noop_profile_move_keeps_the_countdown_armed() {
  // Codex #333: with only the `(default)` choice, `j`/`k` are no-ops and must
  // NOT re-scan — a re-scan resets the ConfirmModal, silently disarming a
  // pending reclaim while the status bar still reads "armed".
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "build/\n").unwrap();
  std::fs::create_dir(repo.path().join("build")).unwrap();
  std::fs::write(repo.path().join("build").join("o"), vec![0u8; 64]).unwrap();
  std::fs::write(repo.path().join(".gwm.toml"), "[tui]\nconfirm_countdown_secs = 3\n").unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  assert!(!app.clean_overlay.has_profiles(), "only the (default) choice exists");
  assert_eq!(app.clean_confirm_press(Instant::now()), ConfirmKeyAction::Armed);
  app.clean_overlay_next();
  assert!(app.clean_overlay.confirm.is_armed(), "a no-op move must not disarm");
  app.clean_overlay_prev();
  assert!(
    app.clean_overlay.confirm.is_armed(),
    "prev no-op must not disarm either"
  );
}

#[test]
fn clean_overlay_real_profile_change_disarms_the_countdown() {
  // The complement: actually changing the target re-requires confirmation.
  let (repo, _) = init_repo();
  std::fs::write(repo.path().join(".gitignore"), "a/\nb/\n").unwrap();
  for d in ["a", "b"] {
    std::fs::create_dir(repo.path().join(d)).unwrap();
    std::fs::write(repo.path().join(d).join("x"), vec![0u8; 64]).unwrap();
  }
  std::fs::write(
    repo.path().join(".gwm.toml"),
    "[tui]\nconfirm_countdown_secs = 3\n\n[clean.profiles.pa]\ndirs = [\"a\"]\n\n[clean.profiles.pb]\ndirs = [\"b\"]\n",
  )
  .unwrap();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  app.enter_clean_overlay();
  assert!(app.clean_overlay.has_profiles());
  // The `(default)` choice scans built-ins (empty here), so move to `pa`
  // (which reclaims `a`) before arming.
  app.clean_overlay_next(); // (default) → pa
  assert_eq!(app.clean_overlay.selected_profile(), Some("pa"));
  assert_eq!(app.clean_confirm_press(Instant::now()), ConfirmKeyAction::Armed);
  app.clean_overlay_next(); // pa → pb: a real change
  assert!(
    !app.clean_overlay.confirm.is_armed(),
    "changing the target re-requires confirmation"
  );
}

// -- Agent session pane (issue #408) --------------------------------------

mod agent_sessions_pane {
  use super::*;
  use gwm::agent_sessions::{AgentKind, AgentSession, Freshness, WorktreeAgents};
  use gwm::tui::{agent_cell_label, TaskKind};
  use std::collections::BTreeMap;
  use std::path::PathBuf;
  use std::time::{Duration, SystemTime};

  fn snapshot_for(path: &str, kind: AgentKind, age_secs: u64) -> BTreeMap<String, WorktreeAgents> {
    let mut map = BTreeMap::new();
    map.insert(
      path.to_string(),
      WorktreeAgents {
        sessions: vec![AgentSession {
          kind,
          cwd: PathBuf::from(path),
          last_activity: SystemTime::now() - Duration::from_secs(age_secs),
          ended: false,
          id: "s1".into(),
          name: None,
        }],
      },
    );
    map
  }

  #[test]
  fn agent_snapshot_applies_on_live_generation_and_coalesces() {
    let (_d, mut app) = make_app();
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    // A second request while one is in flight coalesces (the debounce).
    assert!(app.tasks.request(TaskKind::AgentSessions).is_none());
    let map = snapshot_for("/w/one", AgentKind::ClaudeCode, 10);
    assert!(app.apply_agent_snapshot(generation, map.clone(), None, BTreeMap::new()));
    assert_eq!(app.agent_snapshot.as_ref(), Some(&map));
  }

  #[test]
  fn same_set_refresh_keeps_an_in_flight_detection_alive() {
    // Codex review round P (P2): every refresh used to invalidate the
    // AgentSessions slot unconditionally — with auto_refresh_secs shorter
    // than a scan of a large store, each tick freed the slot while the
    // scan thread kept running, spawned a concurrent scan and dropped the
    // previous result as stale: scans piled up and no snapshot ever
    // landed. A refresh that re-lists the SAME worktree set must keep the
    // in-flight run authoritative; only a genuinely different set drops it.
    let (_d, mut app) = make_app();
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    app.refresh().unwrap();
    assert!(
      app.apply_agent_snapshot(generation, BTreeMap::new(), None, BTreeMap::new()),
      "an unchanged worktree set left the in-flight detection authoritative"
    );
  }

  #[test]
  fn branch_flip_refresh_drops_the_in_flight_detection() {
    // Codex review round Q (P2): pins live in BRANCH config, so a checkout
    // that switches branch without changing path moved the pins key — a
    // same-path-only staleness gate would keep showing the OLD branch's
    // pins for up to 30 s. The (path, branch) key drops the in-flight run.
    let (_d, mut app) = make_app();
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    {
      let head = app.repo.head().unwrap().peel_to_commit().unwrap();
      app.repo.branch("flipped", &head, false).unwrap();
      app.repo.set_head("refs/heads/flipped").unwrap();
    }
    app.refresh().unwrap();
    assert!(
      !app.apply_agent_snapshot(generation, BTreeMap::new(), None, BTreeMap::new()),
      "a branch flip at constant path invalidates the in-flight detection"
    );
  }

  #[test]
  fn summary_only_snapshot_keeps_the_previous_pool() {
    // Round Q: the periodic tick is summary-only (`None` pool) — it must
    // never wipe the candidates an open attach prompt is filtering.
    let (_d, mut app) = make_app();
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    assert!(app.apply_agent_snapshot(
      generation,
      BTreeMap::new(),
      Some(vec![AgentSession {
        kind: AgentKind::Codex,
        cwd: PathBuf::from("/w/one"),
        last_activity: SystemTime::now(),
        ended: false,
        id: "pool-keep".into(),
        name: None,
      }]),
      BTreeMap::new(),
    ));
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    assert!(app.apply_agent_snapshot(generation, BTreeMap::new(), None, BTreeMap::new()));
    assert_eq!(
      app.agent_all_sessions.len(),
      1,
      "the pool survived the summary-only landing"
    );
    assert_eq!(app.agent_all_sessions[0].id, "pool-keep");
  }

  #[test]
  fn opening_the_attach_prompt_starts_a_pool_refresh() {
    // Round Q: the full sweep runs when the prompt opens, not on the
    // periodic tick — observable as an in-flight AgentSessions run that
    // coalesces any further request.
    let (_d, mut app) = make_app();
    app.open_agent_overlay();
    app.open_agent_input();
    assert!(
      app.tasks.request(TaskKind::AgentSessions).is_none(),
      "the pool refresh is in flight right after the prompt opened"
    );
  }

  #[test]
  fn prompt_open_defers_the_pool_scan_behind_an_in_flight_run() {
    // Codex review round R (P2): invalidating the in-flight periodic run
    // only freed the SLOT — its thread kept walking the store while the
    // prompt's full scan started, doubling the I/O. Opening the prompt
    // while a run is in flight must instead queue the pool scan: the
    // periodic result still lands, and the full scan chains after it.
    let (_d, mut app) = make_app();
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    app.open_agent_overlay();
    app.open_agent_input();
    // The in-flight periodic run was NOT invalidated…
    assert!(
      app.apply_agent_snapshot(generation, BTreeMap::new(), None, BTreeMap::new()),
      "the periodic run stays authoritative under the queued pool scan"
    );
    // …and its landing chained the queued full scan.
    assert!(
      app.tasks.request(TaskKind::AgentSessions).is_none(),
      "the pool scan is in flight right after the periodic landing"
    );
  }

  #[test]
  fn closing_the_prompt_cancels_the_queued_pool_scan() {
    // Codex review round T (P2): open-then-close the attach prompt while
    // a periodic run is in flight left `agent_pool_wanted` set — the
    // landing then chained the full foreign-dir sweep with no prompt left
    // to consume it. The chain only fires while the prompt is still open.
    let (_d, mut app) = make_app();
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    app.open_agent_overlay();
    app.open_agent_input(); // queued behind the in-flight run
    app.agent_input_cancel(); // …and abandoned before it landed
    assert!(app.apply_agent_snapshot(generation, BTreeMap::new(), None, BTreeMap::new()));
    assert!(
      app.tasks.request(TaskKind::AgentSessions).is_some(),
      "no orphan pool scan chained after the prompt closed"
    );
  }

  #[test]
  fn agent_snapshot_stale_generation_is_dropped() {
    let (_d, mut app) = make_app();
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    let live = snapshot_for("/w/one", AgentKind::Codex, 10);
    assert!(app.apply_agent_snapshot(generation, live.clone(), None, BTreeMap::new()));
    // A new run starts, then a refresh invalidates it mid-flight.
    let stale = app.tasks.request(TaskKind::AgentSessions).unwrap();
    app.tasks.invalidate(TaskKind::AgentSessions);
    assert!(!app.apply_agent_snapshot(stale, BTreeMap::new(), None, BTreeMap::new()));
    // The last authoritative snapshot survives.
    assert_eq!(app.agent_snapshot.as_ref(), Some(&live));
  }

  #[test]
  fn agent_cell_is_empty_without_snapshot_or_sessions() {
    // No snapshot yet (startup) and no matched sessions both render nothing —
    // no placeholder noise (spec US1 scenario 5).
    assert!(agent_cell_label(None, SystemTime::now()).is_none());
    let empty = WorktreeAgents::default();
    assert!(agent_cell_label(Some(&empty), SystemTime::now()).is_none());
  }

  #[test]
  fn agent_cell_shows_top_agent_with_freshness() {
    let now = SystemTime::now();
    let agents = WorktreeAgents {
      sessions: vec![
        AgentSession {
          kind: AgentKind::ClaudeCode,
          cwd: PathBuf::from("/w/one"),
          last_activity: now - Duration::from_secs(10),
          ended: false,
          id: "new".into(),
          name: None,
        },
        AgentSession {
          kind: AgentKind::Vibe,
          cwd: PathBuf::from("/w/one"),
          last_activity: now - Duration::from_secs(4000),
          ended: false,
          id: "old".into(),
          name: None,
        },
      ],
    };
    let (label, freshness) = agent_cell_label(Some(&agents), now).unwrap();
    assert_eq!(label, "claude");
    assert_eq!(freshness, Freshness::Active);

    let idle_only = WorktreeAgents {
      sessions: vec![AgentSession {
        kind: AgentKind::Opencode,
        cwd: PathBuf::from("/w/one"),
        last_activity: now - Duration::from_secs(4000),
        ended: false,
        id: "old".into(),
        name: None,
      }],
    };
    let (label, freshness) = agent_cell_label(Some(&idle_only), now).unwrap();
    assert_eq!(label, "opencode");
    assert_eq!(freshness, Freshness::Idle);
  }

  #[test]
  fn agents_for_looks_up_by_worktree_path() {
    let (_d, mut app) = make_app();
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    let path = app.worktrees[0].path.to_string_lossy().to_string();
    let map = snapshot_for(&path, AgentKind::ClaudeCode, 10);
    assert!(app.apply_agent_snapshot(generation, map, None, BTreeMap::new()));
    let w = app.worktrees[0].clone();
    assert!(app.agents_for(&w).is_some());
    assert_eq!(app.agents_for(&w).unwrap().top().unwrap().id, "s1");
  }
}

// -- Agent detail overlay (issue #408, US2) --------------------------------

mod agent_detail_overlay {
  use super::*;
  use gwm::agent_sessions::{AgentKind, AgentSession, WorktreeAgents};
  use gwm::tui::state::detail_overlay::{agent_detail_rows, DetailRole};
  use gwm::tui::{TaskKind, View};
  use std::collections::BTreeMap;
  use std::path::PathBuf;
  use std::time::{Duration, SystemTime};

  fn seeded_app_with_sessions() -> (tempfile::TempDir, App) {
    let (dir, mut app) = make_app();
    let now = SystemTime::now();
    let path = app.worktrees[0].path.to_string_lossy().to_string();
    let mut map = BTreeMap::new();
    map.insert(
      path.clone(),
      WorktreeAgents {
        sessions: vec![
          AgentSession {
            kind: AgentKind::ClaudeCode,
            cwd: PathBuf::from(&path),
            last_activity: now - Duration::from_secs(10),
            ended: false,
            id: "newest-session".into(),
            name: None,
          },
          AgentSession {
            kind: AgentKind::Codex,
            cwd: PathBuf::from(&path),
            last_activity: now - Duration::from_secs(4000),
            ended: false,
            id: "older-session".into(),
            name: None,
          },
        ],
      },
    );
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    assert!(app.apply_agent_snapshot(generation, map, None, BTreeMap::new()));
    (dir, app)
  }

  #[test]
  fn open_lists_sessions_most_recent_first() {
    let (_d, mut app) = seeded_app_with_sessions();
    app.open_agent_overlay();
    assert_eq!(app.view, View::DetailOverlay);
    let rows = &app.detail_overlay.rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].label, "claude");
    assert_eq!(rows[0].role, DetailRole::Active);
    assert_eq!(rows[1].label, "codex");
    assert_eq!(rows[1].role, DetailRole::Muted);
    // Value carries freshness + a human-readable recency, not raw timestamps,
    // and the FULL id (user feedback: 8-char truncation was useless for attach).
    assert!(rows[0].value.contains("active"));
    assert!(rows[1].value.contains("idle"));
    assert!(
      rows[0].value.contains("newest-session"),
      "full id expected: {}",
      rows[0].value
    );
    // meta carries the session id for attach/detach on the selected row.
    assert_eq!(rows[0].meta.as_deref(), Some("newest-session"));
    // User feedback 2026-07-22: capitalized title, no worktree name suffix.
    assert_eq!(app.detail_overlay.title, "Agent Sessions");
  }

  #[test]
  fn open_on_sessionless_worktree_states_it_rather_than_empty() {
    let (_d, mut app) = make_app();
    app.open_agent_overlay();
    assert_eq!(app.view, View::DetailOverlay);
    let rows = &app.detail_overlay.rows;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].value.contains("no agent session found"));
    assert_eq!(rows[0].role, DetailRole::Muted);
  }

  #[test]
  fn attach_on_an_empty_list_falls_through_to_the_by_id_prompt() {
    // User feedback 2026-07-22: with "no agent session found" there is
    // nothing to select, so `a` must open the attach-by-id prompt instead
    // of dead-ending on a status error.
    use gwm::tui::state::detail_overlay::DetailMode;
    let (_d, mut app) = make_app();
    app.open_agent_overlay();
    app.attach_selected_agent();
    assert_eq!(app.detail_overlay.mode, DetailMode::Input);
  }

  #[test]
  fn close_restores_the_list_untouched() {
    let (_d, mut app) = seeded_app_with_sessions();
    let selected_before = app.list_state.selected();
    app.open_agent_overlay();
    app.close_detail_overlay();
    assert_eq!(app.view, View::List);
    assert_eq!(app.list_state.selected(), selected_before);
  }

  #[test]
  fn detail_rows_are_generic_label_value_role_triples() {
    // The mapping is pure and content-agnostic: any consumer can build rows.
    let rows = agent_detail_rows(None, &[], SystemTime::now());
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    // The triple shape is the reuse contract for the future rich view.
    let _label: &String = &row.label;
    let _value: &String = &row.value;
    let _role: &DetailRole = &row.role;
    let _meta: &Option<String> = &row.meta;
  }

  #[test]
  fn selection_starts_at_zero_moves_and_clamps() {
    let (_d, mut app) = seeded_app_with_sessions();
    app.open_agent_overlay();
    assert_eq!(app.detail_overlay.selected, 0);
    app.detail_overlay.select_next();
    assert_eq!(app.detail_overlay.selected, 1);
    app.detail_overlay.select_next(); // clamps at last row
    assert_eq!(app.detail_overlay.selected, 1);
    app.detail_overlay.select_prev();
    app.detail_overlay.select_prev(); // clamps at zero
    assert_eq!(app.detail_overlay.selected, 0);
  }

  #[test]
  fn rows_prefer_the_session_name_over_the_id() {
    // User feedback: a named session displays its name, not the uuid.
    let now = SystemTime::now();
    let agents = WorktreeAgents {
      sessions: vec![AgentSession {
        kind: AgentKind::ClaudeCode,
        cwd: PathBuf::from("/w/one"),
        last_activity: now - Duration::from_secs(10),
        ended: false,
        id: "a7820111-8232".into(),
        name: Some("fix the login timeout bug".into()),
      }],
    };
    let rows = agent_detail_rows(Some(&agents), &[], now);
    assert!(
      rows[0].value.contains("fix the login timeout bug"),
      "got {}",
      rows[0].value
    );
    assert!(
      !rows[0].value.contains("a7820111"),
      "id must yield to the name: {}",
      rows[0].value
    );
    // The id still rides meta for attach.
    assert_eq!(rows[0].meta.as_deref(), Some("a7820111-8232"));
  }

  #[test]
  fn attach_writes_the_pin_into_the_current_branch_after_a_flip() {
    // Codex review round U (P2): the overlay captured (path, branch) at
    // open; a branch flipped externally while it stayed open meant attach
    // wrote `branch.<old>.gwm-agent-pin`. The write must re-resolve the
    // CURRENT branch from the captured path.
    let (_d, mut app) = seeded_app_with_sessions();
    app.open_agent_overlay();
    {
      let head = app.repo.head().unwrap().peel_to_commit().unwrap();
      app.repo.branch("flipped", &head, false).unwrap();
      app.repo.set_head("refs/heads/flipped").unwrap();
    }
    app.refresh().unwrap(); // worktrees now carry the new branch
    app.attach_selected_agent();
    let pins = gwm::github::agent_pins(&app.repo, "flipped").unwrap();
    assert_eq!(pins, vec!["newest-session"], "the pin landed in the CURRENT branch");
  }

  #[test]
  fn pin_change_during_an_in_flight_scan_chains_instead_of_racing() {
    // Codex review round U (P2): attach/detach invalidated the
    // AgentSessions slot while its thread was still walking the store —
    // the next tick spawned a second concurrent scan (same hazard as
    // rounds P/R). With a run in flight the refresh queues: the landing
    // stays authoritative, the fresh pins survive it, and the re-scan
    // chains after.
    let (_d, mut app) = seeded_app_with_sessions();
    app.open_agent_overlay();
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    app.attach_selected_agent(); // pin written while the scan is in flight
    let branch = app.worktrees[0].branch.clone().unwrap();
    assert!(!gwm::github::agent_pins(&app.repo, &branch).unwrap().is_empty());
    // The in-flight run still lands (not invalidated) — with PRE-change
    // pins that must not clobber the fresh map…
    let stale_pins = BTreeMap::new();
    assert!(app.apply_agent_snapshot(generation, BTreeMap::new(), None, stale_pins));
    assert!(
      app.agent_pins.values().flatten().any(|sid| sid == "newest-session"),
      "the fresh pin survived the stale landing: {:?}",
      app.agent_pins
    );
    // …and the queued re-detection is due (snapshot cleared, slot free).
    assert!(
      app.tasks.request(TaskKind::AgentSessions).is_some(),
      "the slot is free for the chained re-scan"
    );
  }

  #[test]
  fn attach_pins_the_selected_session_and_marks_the_row() {
    let (_d, mut app) = seeded_app_with_sessions();
    app.open_agent_overlay();
    app.detail_overlay.select_next(); // select "older-session"
    app.attach_selected_agent();
    // Pin persisted in branch config for the target worktree's branch.
    let branch = app.worktrees[0].branch.clone().unwrap();
    let pins = gwm::github::agent_pins(&app.repo, &branch).unwrap();
    assert_eq!(pins, vec!["older-session"]);
    // The row now carries the pinned marker.
    let row = &app.detail_overlay.rows[app.detail_overlay.selected];
    assert!(row.value.contains("pinned"), "got {}", row.value);
  }

  #[test]
  fn attach_accumulates_pins_and_detach_removes_only_the_selected_one() {
    // User feedback 2026-07-22: several agents can work one worktree, so a
    // second attach ADDS a pin (it used to replace), and `d` unpins only
    // the selected session.
    let (_d, mut app) = seeded_app_with_sessions();
    app.open_agent_overlay();
    app.attach_selected_agent(); // pin "newest-session"
    app.detail_overlay.select_next();
    app.attach_selected_agent(); // pin "older-session" TOO
    let branch = app.worktrees[0].branch.clone().unwrap();
    assert_eq!(
      gwm::github::agent_pins(&app.repo, &branch).unwrap(),
      vec!["newest-session", "older-session"]
    );
    // Both rows carry the marker.
    assert!(app.detail_overlay.rows.iter().all(|r| r.value.contains("pinned")));

    // Detach on the selected (older) row removes only that pin.
    app.detach_selected_agent();
    assert_eq!(
      gwm::github::agent_pins(&app.repo, &branch).unwrap(),
      vec!["newest-session"]
    );
    assert!(app.detail_overlay.rows[0].value.contains("pinned"));
    assert!(!app.detail_overlay.rows[1].value.contains("pinned"));
  }

  #[test]
  fn detach_clears_the_pin() {
    let (_d, mut app) = seeded_app_with_sessions();
    app.open_agent_overlay();
    app.attach_selected_agent();
    let branch = app.worktrees[0].branch.clone().unwrap();
    assert!(!gwm::github::agent_pins(&app.repo, &branch).unwrap().is_empty());
    app.detach_selected_agent();
    assert!(gwm::github::agent_pins(&app.repo, &branch).unwrap().is_empty());
    let row = &app.detail_overlay.rows[0];
    assert!(!row.value.contains("pinned"), "got {}", row.value);
  }
}

// -- Agents sidebar pane (issue #408, user feedback 2026-07-22) ------------

mod agent_pane {
  use gwm::agent_sessions::{AgentKind, AgentSession, WorktreeAgents};
  use gwm::tui::theme::Theme;
  use gwm::tui::{agent_pane_lines, agents_pane_title};
  use std::path::PathBuf;
  use std::time::{Duration, SystemTime};

  fn line_text(line: &ratatui::text::Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
  }

  fn mk(kind: AgentKind, age: u64, id: &str, name: Option<&str>) -> AgentSession {
    AgentSession {
      kind,
      cwd: PathBuf::from("/w/one"),
      last_activity: SystemTime::now() - Duration::from_secs(age),
      ended: false,
      id: id.into(),
      name: name.map(str::to_string),
    }
  }

  #[test]
  fn pane_lists_only_pinned_sessions_preferring_names() {
    // User feedback 2026-07-22: the sidebar pane is the *deliberate* view —
    // only pinned sessions show there; the full detected list lives in the
    // `a` overlay.
    let now = SystemTime::now();
    let agents = WorktreeAgents {
      sessions: vec![
        mk(AgentKind::ClaudeCode, 10, "uuid-1", Some("fix login bug")),
        mk(AgentKind::Codex, 400, "uuid-2", None),
      ],
    };
    let pinned = ["uuid-1".to_string()];
    let lines = agent_pane_lines(Some(&agents), &pinned, now, &Theme::default());
    assert_eq!(lines.len(), 1, "only the pinned session shows");
    let first = line_text(&lines[0]);
    assert!(first.contains("claude"), "got {first}");
    assert!(first.contains("active"), "got {first}");
    assert!(first.contains("fix login bug"), "got {first}");
    assert!(!first.contains("uuid-1"), "name must replace the id: {first}");
  }

  #[test]
  fn pane_caps_at_three_pinned_sessions_with_an_overflow_line() {
    let now = SystemTime::now();
    let agents = WorktreeAgents {
      sessions: (0..5)
        .map(|i| mk(AgentKind::ClaudeCode, 1000 + i, &format!("uuid-{i}"), None))
        .collect(),
    };
    let pinned: Vec<String> = (0..5).map(|i| format!("uuid-{i}")).collect();
    let lines = agent_pane_lines(Some(&agents), &pinned, now, &Theme::default());
    assert_eq!(lines.len(), 4, "3 pinned sessions + overflow line");
    let overflow = line_text(&lines[3]);
    assert!(overflow.contains("+2"), "got {overflow}");
  }

  #[test]
  fn pane_is_empty_without_pins_so_the_block_collapses() {
    let now = SystemTime::now();
    assert!(agent_pane_lines(None, &[], now, &Theme::default()).is_empty());
    // Detected-but-unpinned sessions do NOT surface in the pane (user
    // feedback 2026-07-22) — they stay in the overlay until pinned.
    let agents = WorktreeAgents {
      sessions: vec![mk(AgentKind::ClaudeCode, 10, "uuid-1", None)],
    };
    assert!(agent_pane_lines(Some(&agents), &[], now, &Theme::default()).is_empty());
  }

  #[test]
  fn pane_title_advertises_the_overlay_key() {
    let km = gwm::tui::keymap::Keymap::defaults();
    let title = agents_pane_title(&km);
    assert!(title.contains("Agents"), "got {title}");
    assert!(title.contains('a'), "resolved overlay key expected: {title}");
  }
}

// -- Detail overlay footer hints (user feedback 2026-07-22) ----------------

mod agent_overlay_hints {
  use gwm::tui::keymap::Keymap;
  use gwm::tui::modal_keymap::ModalKeymap;
  use gwm::tui::theme::Theme;
  use gwm::tui::{modal_hint_for_context, HintContext};

  #[test]
  fn detail_footer_advertises_select_attach_detach_close() {
    let line = modal_hint_for_context(
      HintContext::Detail,
      &Keymap::defaults(),
      &ModalKeymap::defaults(),
      &Theme::default(),
    );
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    for needle in ["select", "attach", "detach", "close"] {
      assert!(text.contains(needle), "hint must advertise '{needle}', got: {text}");
    }
    // The resolved default keys ride along.
    assert!(text.contains('a'), "attach key expected: {text}");
    assert!(text.contains('d'), "detach key expected: {text}");
  }
}

// -- Overlay refresh + attach-by-id input (user feedback 2026-07-22 #2) ----

mod agent_overlay_input {
  use super::*;
  use gwm::agent_sessions::{AgentKind, AgentSession, WorktreeAgents};
  use gwm::tui::state::detail_overlay::{filter_sessions, DetailMode};
  use gwm::tui::TaskKind;
  use std::collections::BTreeMap;
  use std::path::PathBuf;
  use std::time::{Duration, SystemTime};

  fn session(kind: AgentKind, id: &str, name: Option<&str>) -> AgentSession {
    AgentSession {
      kind,
      cwd: PathBuf::from("/elsewhere"),
      last_activity: SystemTime::now() - Duration::from_secs(10),
      ended: false,
      id: id.into(),
      name: name.map(str::to_string),
    }
  }

  #[test]
  fn snapshot_landing_rebuilds_the_open_overlay_rows() {
    // User feedback: after attach/detach the async re-detection lands but
    // the open overlay kept its stale rows until reopened.
    let (_d, mut app) = make_app();
    app.open_agent_overlay();
    assert!(app.detail_overlay.rows[0].value.contains("no agent session"));

    let path = app.worktrees[0].path.to_string_lossy().to_string();
    let mut map = BTreeMap::new();
    map.insert(
      path,
      WorktreeAgents {
        sessions: vec![session(AgentKind::Codex, "fresh-1", None)],
      },
    );
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    assert!(app.apply_agent_snapshot(generation, map, None, BTreeMap::new()));
    assert_eq!(app.detail_overlay.rows.len(), 1);
    assert!(app.detail_overlay.rows[0].value.contains("fresh-1"));
  }

  #[test]
  fn filter_matches_id_name_and_kind_case_insensitively() {
    let all = vec![
      session(AgentKind::Codex, "019f6b95-abcd", Some("review feature flags")),
      session(AgentKind::ClaudeCode, "a7820111-uuid", Some("fix login")),
      session(AgentKind::Vibe, "vibe-1", None),
    ];
    let ids = |q: &str| -> Vec<String> { filter_sessions(&all, q).into_iter().map(|s| s.id.clone()).collect() };
    assert_eq!(ids("019f"), ["019f6b95-abcd"]);
    assert_eq!(ids("LOGIN"), ["a7820111-uuid"]);
    assert_eq!(ids("vibe"), ["vibe-1"]);
    assert_eq!(ids("").len(), 3, "empty query lists everything");
    assert!(ids("zzz").is_empty());
  }

  #[test]
  fn input_mode_attaches_the_highlighted_candidate() {
    let (_d, mut app) = make_app();
    // Seed the global session pool with an unmatched session.
    let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
    assert!(app.apply_agent_snapshot(
      generation,
      BTreeMap::new(),
      Some(vec![session(AgentKind::Codex, "pool-42", Some("refactor auth"))]),
      BTreeMap::new(),
    ));
    app.open_agent_overlay();
    app.open_agent_input();
    assert_eq!(app.detail_overlay.mode, DetailMode::Input);
    app.agent_input_push('p');
    app.agent_input_push('o');
    app.agent_input_push('o');
    app.agent_input_push('l');
    app.agent_input_submit();
    // Back to the list, pin persisted for the target worktree's branch.
    assert_eq!(app.detail_overlay.mode, DetailMode::List);
    let branch = app.worktrees[0].branch.clone().unwrap();
    let pins = gwm::github::agent_pins(&app.repo, &branch).unwrap();
    assert_eq!(pins, vec!["pool-42"]);
  }

  #[test]
  fn input_mode_escape_returns_to_list_without_pinning() {
    let (_d, mut app) = make_app();
    app.open_agent_overlay();
    app.open_agent_input();
    app.agent_input_push('x');
    app.agent_input_cancel();
    assert_eq!(app.detail_overlay.mode, DetailMode::List);
    let branch = app.worktrees[0].branch.clone().unwrap();
    assert!(gwm::github::agent_pins(&app.repo, &branch).unwrap().is_empty());
  }

  #[test]
  fn input_mode_unknown_id_reports_and_stays_in_input() {
    let (_d, mut app) = make_app();
    app.open_agent_overlay();
    app.open_agent_input();
    for c in "nope".chars() {
      app.agent_input_push(c);
    }
    app.agent_input_submit();
    assert_eq!(app.detail_overlay.mode, DetailMode::Input, "stay for correction");
    assert!(app.status.contains("no agent session"), "got {}", app.status);
  }
}
