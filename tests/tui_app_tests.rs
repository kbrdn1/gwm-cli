mod common;

use common::init_repo;
use gwm::naming::BRANCH_TYPES;
use gwm::tui::keymap::Action;
use gwm::tui::theme::Theme;
use gwm::tui::{
  branch_name_color, filled_cells_for_progress, freshness_color, panel_border_color, pr_badge_color, App,
  ConfirmKeyAction, CountdownTickOutcome, Field, NoteKey, ToggleStroke, View,
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
    has_note: false,
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
  // The global layer binds `top` to a two-stroke chord. `u` rather than `z`
  // since #484 moved `cycle_sidebar_layout` onto `z`, and a shipped default
  // is a prefix conflict for any chord starting with the same key.
  set_array_at(&global, "tui.keys.top", &["u u".to_string()]).unwrap();

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

  // `u` alone is a prefix of the global `u u` — valid in the repo file by
  // itself, invalid once the layers merge.
  app.config_panel.begin_capture();
  app.push_key_capture(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
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
fn a_settings_edit_drops_the_sidebar_cache() {
  // #547: the folded / labelled shape is baked into the *cached* payload,
  // and the cache is keyed by (path, mode) alone — neither of which a
  // settings edit changes. Without an explicit drop, toggling
  // `status_one_line` from the panel leaves the old shape on screen until
  // the user navigates away and back, i.e. reads as a toggle that did
  // nothing. The theme has the same exposure (it colours every span in
  // there), which is why the drop is unconditional rather than per-field.
  use gwm::tui::{App, SettingField, SettingsLayer};

  let (repo, _) = init_repo();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  let w = app.selected().expect("a worktree is selected").clone();
  let mode = app.sidebar.mode;
  let sections = gwm::tui::build_sidebar_payload(&w, mode, &app.config.doctor.trunks, &app.theme, false);
  app.sidebar.cache = Some(((w.path.clone(), mode), sections));

  app.config_panel.layer = SettingsLayer::Project;
  app.apply_setting(SettingField::StatusOneLine, "true");

  assert!(
    app.config.tui.status_one_line,
    "the edit reached the live config: {}",
    app.status
  );
  assert!(
    app.sidebar.cache.is_none(),
    "the pre-edit payload must be dropped so the new shape is rebuilt"
  );
}

#[test]
fn a_settings_edit_invalidates_an_inflight_sidebar_rebuild() {
  // Codex review, PR #556 (P2): dropping the cache is only half the pair.
  // A worker spawned *before* the edit carries the pre-edit config and theme;
  // its generation is still current, so `drain_task_results` accepts the
  // payload and stores it under the same (path, mode) key. `maybe_refresh_sidebar`
  // then reads a warm cache and never rebuilds — the toggle looks ignored
  // again, this time through the race rather than the cache.
  //
  // Same pairing `apply_refreshed_worktrees` makes for the #343 hazard:
  // `sidebar.invalidate()` and `tasks.invalidate(TaskKind::Sidebar)` travel
  // together or not at all.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  use gwm::tui::{App, SettingField, SettingsLayer, SidebarSections};

  let (repo, _) = init_repo();
  let mut app = App::new_at_layered(Some(repo.path()), None).unwrap();
  let w = app.selected().expect("a worktree is selected").clone();
  let mode = app.sidebar.mode;
  let stale = app.tasks.request(TaskKind::Sidebar).expect("no rebuild in flight yet");

  app.config_panel.layer = SettingsLayer::Project;
  app.apply_setting(SettingField::StatusOneLine, "false");

  // The pre-edit worker lands after the write, for the *current* selection.
  app
    .task_result_sender()
    .send(TaskMsg::Sidebar(
      stale,
      w.path.clone(),
      mode,
      SidebarSections::default(),
    ))
    .unwrap();
  app.drain_task_results();

  assert!(
    app.sidebar.cache.is_none(),
    "a rebuild that started before the edit carries the pre-edit shape — it must be dropped"
  );
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
  // editing control on the input fields (and such rebinds are refused at
  // config time since iteration 14) — this route is the runtime
  // backstop.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::CreateKey;
  let (_dir, mut app) = make_app();
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
  // Codex review #456: Backspace is reserved typing on the number stage
  // (a colliding rebind is refused at config time).
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::LinkPromptKey;
  let (_dir, mut app) = make_app();
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
  app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
  let out = app.handle_link_prompt_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
  assert!(matches!(out, LinkPromptKey::Handled), "Backspace is plain typing");
  assert_eq!(app.link_prompt_number_input(), "4", "Backspace erases the last digit");
}

#[test]
fn printable_keys_stay_typing_in_the_create_form_despite_rebinds() {
  // Codex review #456 (iteration 8): a modal rebind onto a printable key
  // (`cancel = ["q"]`) resolved before the typing fallback, so `q` closed
  // the form mid-word. On a text field the printable keys are reserved
  // for typing (the palette convention, and such rebinds are refused at
  // config time since iteration 14) — this route is the runtime
  // backstop. Type-cycling verbs keep bare letters: they only fire on
  // the Type field, so `next_type = ["q"]` must not steal the letter
  // from a text field either.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  use gwm::tui::CreateKey;
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(ModalAction::CreateNextType, KeyStroke::parse_chord("q").unwrap())
    .unwrap();
  app.enter_create();
  app.create_form.field = Field::Desc;
  let before = app.create_form.type_index;
  let out = app.handle_create_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
  assert_ne!(out, CreateKey::Cancel, "a printable rebind must not fire while typing");
  assert_eq!(app.create_form.desc, "q", "the character types into the field instead");
  assert_eq!(
    app.create_form.type_index, before,
    "the type must not cycle from a text field"
  );
}

#[test]
fn printable_keys_stay_typing_in_the_link_number_input_despite_rebinds() {
  // Codex review #456: digits are reserved typing on the number stage
  // (a colliding rebind like cancel = ["5"] is refused at config time);
  // non-digits still reach the modal resolution (#293 contract).
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::LinkPromptKey;
  let (_dir, mut app) = make_app();
  app.enter_link_prompt();
  app.link_prompt_choose(LinkTarget::Issue);
  let out = app.handle_link_prompt_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
  assert!(matches!(out, LinkPromptKey::Handled), "a digit is plain typing");
  assert_eq!(app.link_prompt_number_input(), "5", "the digit types into the number");
}

#[test]
fn palette_input_routes_typing_before_modal_rebinds() {
  // Codex review #456: the palette's filter charset and Backspace are
  // reserved typing, routed through a testable App method before the
  // modal resolution (bindings that would collide are refused at config
  // time — see reserved_typing_keys_cannot_be_rebound_in_input_contexts).
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  assert!(
    app.palette_input_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
    "a charset key is consumed as typing"
  );
  assert_eq!(app.palette.buffer(), "x", "the character filters");
  assert!(
    app.palette_input_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
    "Backspace is consumed as the eraser"
  );
  assert_eq!(app.palette.buffer(), "", "Backspace erases");
}

#[test]
fn settings_editor_routes_typing_before_modal_rebinds() {
  // Same contract for the Settings value editor (Codex review #456):
  // printables and Backspace are consumed as typing before the modal
  // resolution. Rebinds on those keys are refused at config time
  // (iteration 13) — this route is the runtime backstop behind that
  // validation.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, mut app) = make_app();
  app.config_panel.editing = Some("4".into());
  assert!(
    app.settings_edit_input_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE)),
    "a printable key is consumed as typing"
  );
  assert_eq!(app.config_panel.editing.as_deref(), Some("45"));
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
fn settings_editor_reinjects_unresolved_modified_backspace() {
  // Codex review #456 (iteration 14): before the reserved-typing routes,
  // every KeyCode::Backspace erased — including Alt/Ctrl+Backspace. The
  // modifier-aware route let those reach the modal resolution (correct,
  // a bound Ctrl+Backspace must fire), but the empty-resolution fallback
  // only reinjected Char, so an UNBOUND modified Backspace stopped
  // erasing. Parity restored: it pops one character again.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, mut app) = make_app();
  app.config_panel.editing = Some("ab".into());
  app.handle_settings_edit_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
  assert_eq!(
    app.config_panel.editing.as_deref(),
    Some("a"),
    "an unresolved Alt+Backspace must still erase"
  );
  app.handle_settings_edit_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL));
  assert_eq!(
    app.config_panel.editing.as_deref(),
    Some(""),
    "an unresolved Ctrl+Backspace must still erase"
  );
}

#[test]
fn palette_reinjects_unresolved_modified_backspace() {
  // Same parity for the command palette (Codex review #456, iteration
  // 14): an UNBOUND Alt/Ctrl+Backspace falls through the modal
  // resolution and must still erase, exactly like the pre-#456 routing.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let (_dir, mut app) = make_app();
  app.open_command_palette();
  app.palette_input_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
  app.palette_input_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
  app.palette_unresolved_fallback(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
  assert_eq!(
    app.palette.buffer(),
    "a",
    "an unresolved Alt+Backspace must still erase"
  );
  // The charset reinjection (AltGr parity) keeps working through the
  // same fallback.
  app.palette_unresolved_fallback(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT));
  assert_eq!(
    app.palette.buffer(),
    "ac",
    "an unresolved AltGr charset character still types"
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
fn a_character_the_numeric_field_refuses_is_not_claimed_as_typing() {
  // Codex review #456 (iteration 11): on a numeric field push_edit_char
  // refuses non-digits, but settings_edit_input_key still claimed them as
  // consumed. A character the field refuses is not typing; it falls
  // through to the modal resolution (where only non-typing strokes can be
  // bound since iteration 13, so nothing fires and the reinjection drops
  // it) — the buffer must stay untouched either way.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::{SettingField, SettingsTab};
  let (_dir, mut app) = make_app();
  app.config_panel.tab = SettingsTab::Tui;
  app.config_panel.selected = 0;
  while app.config_panel.selected_field() != Some(SettingField::ConfirmCountdown) {
    app.config_panel.selected += 1;
    assert!(
      app.config_panel.selected < 100,
      "ConfirmCountdown not found in the Tui tab"
    );
  }
  app.config_panel.begin_edit("4");
  assert!(
    !app.settings_edit_input_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
    "a character the numeric field refuses must fall through to the resolution"
  );
  app.handle_settings_edit_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
  assert_eq!(
    app.config_panel.editing.as_deref(),
    Some("4"),
    "the refused character must not land in the numeric buffer"
  );
}

#[test]
fn reserved_typing_keys_cannot_be_rebound_in_input_contexts() {
  // Codex review #456 (iterations 12-13): a VALID config like
  // `[tui.keys.modal.palette] close = ["x"]` replaced Esc, then the
  // reserved typing consumed `x` — the overlay had no exit left short of
  // Ctrl-C killing the whole TUI. Bindings the typing routes would
  // swallow are refused up front, at config time: every verb of the
  // always-typing contexts (palette, link number input), the whole
  // ConfigEdit context (it only exists while a value edit consumes
  // printables and Backspace), and CreateSubmit (submitting only works
  // from the Description field, where all printables are typing). The
  // other create verbs stay bindable on bare letters — they act on the
  // Type field, which takes no text input.
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::{ModalAction, ModalKeymap};
  let mut modal = ModalKeymap::defaults();
  for (action, chord) in [
    (ModalAction::CommandPaletteClose, "x"),
    (ModalAction::CommandPaletteAccept, "Backspace"),
    (ModalAction::LinkInputCancel, "5"),
    (ModalAction::LinkInputSubmit, "Backspace"),
    (ModalAction::ConfigEditSubmit, "5"),
    (ModalAction::ConfigEditCancel, "x"),
    (ModalAction::ConfigEditCancel, "Backspace"),
    (ModalAction::CreateSubmit, "s"),
    (ModalAction::CreateSubmit, "Backspace"),
    // Iteration 14: cancel / field navigation must stay reachable from
    // the create form's TEXT fields, where bare printables and Backspace
    // are typing — only the type-cycling verbs live on a typing-free
    // field and keep bare letters.
    (ModalAction::CreateCancel, "q"),
    (ModalAction::CreateCancel, "Backspace"),
    (ModalAction::CreateNextField, "Backspace"),
    (ModalAction::CreatePrevField, "a"),
  ] {
    assert!(
      modal
        .apply_override(action, KeyStroke::parse_chord(chord).unwrap())
        .is_err(),
      "{action:?} = [{chord:?}] must be refused — the key is reserved typing input"
    );
  }
  for (action, chord) in [
    (ModalAction::CreateNextType, "n"),
    (ModalAction::CreateCancel, "Alt+q"),
    (ModalAction::ConfigEditSubmit, "Alt+s"),
    (ModalAction::CommandPaletteClose, "Alt+x"),
  ] {
    assert!(
      modal
        .apply_override(action, KeyStroke::parse_chord(chord).unwrap())
        .is_ok(),
      "{action:?} = [{chord:?}] must stay bindable"
    );
  }
}

#[test]
fn link_number_footer_drops_a_fetch_binding_swallowed_by_typing() {
  // Codex review #456 (iteration 13): with the global `fetch_github`
  // rebound to a digit or Backspace, the number-input stage consumes the
  // key as typing before the fetch fallback — the shortcut cannot fire
  // there. The footer must drop the dead hint rather than advertise it
  // (the same convention as a key shadowed by a modal binding).
  use crossterm::event::{KeyCode, KeyModifiers};
  use gwm::tui::keymap::{Action, KeyStroke, Keymap};
  use gwm::tui::modal_keymap::ModalKeymap;
  use gwm::tui::HintContext;

  let modal = ModalKeymap::defaults();
  let default = HintContext::LinkInputNumber.resolve(&Keymap::defaults(), &modal);
  assert!(
    default.iter().any(|(_, l)| l == "fetch"),
    "the default fetch binding must be advertised: {default:?}"
  );
  let mut km = Keymap::defaults();
  km_apply(&mut km, Action::FetchGithub, KeyCode::Char('5'));
  let resolved = HintContext::LinkInputNumber.resolve(&km, &modal);
  assert!(
    !resolved.iter().any(|(_, l)| l == "fetch"),
    "a fetch binding swallowed by digit typing must not be advertised: {resolved:?}"
  );
  let mut km = Keymap::defaults();
  km_apply(&mut km, Action::FetchGithub, KeyCode::Backspace);
  let resolved = HintContext::LinkInputNumber.resolve(&km, &modal);
  assert!(
    !resolved.iter().any(|(_, l)| l == "fetch"),
    "a fetch binding swallowed by the eraser must not be advertised: {resolved:?}"
  );

  fn km_apply(km: &mut Keymap, action: Action, code: KeyCode) {
    km.apply_override(action, vec![vec![KeyStroke::new(code, KeyModifiers::empty())]])
      .unwrap();
  }
}

#[test]
fn full_edit_buffer_still_consumes_valid_characters() {
  // Codex review #456 (iteration 12): at the buffer limit push_edit_char
  // refused even VALID characters, so the stroke leaked through to the
  // modal resolution mid-typing. A type-accepted character is consumed
  // as a no-op at the limit; only a character the field refuses outright
  // falls through.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::{SettingField, SettingsTab};
  let (_dir, mut app) = make_app();
  app.config_panel.tab = SettingsTab::Tui;
  app.config_panel.selected = 0;
  while app.config_panel.selected_field() != Some(SettingField::ConfirmCountdown) {
    app.config_panel.selected += 1;
    assert!(
      app.config_panel.selected < 100,
      "ConfirmCountdown not found in the Tui tab"
    );
  }
  app.config_panel.begin_edit("999"); // at the 3-char limit
  assert!(
    app.settings_edit_input_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE)),
    "a valid digit at the limit is a consumed no-op — it must not leak to the resolution"
  );
  assert_eq!(
    app.config_panel.editing.as_deref(),
    Some("999"),
    "the buffer must stay at its limit"
  );
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
  assert_eq!(split_section_heights(60, 2, 3, 10, 20), (5, 12, 43));
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
  assert_eq!(split_section_heights(21, 2, 6, 30, 50), (8, 7, 6));
}

#[test]
fn section_heights_never_clamp_the_agents_pane() {
  // Codex review on PR #454: the Agents pane has no scroll — clamping it
  // below its content permanently hides the trailing "+N more" row (3
  // pinned rows + the overflow indicator = 4 content rows max, bounded by
  // agent_pane_lines). A non-scrollable section keeps its natural height
  // even when the column overflows; only the scrollable sections clamp.
  use gwm::tui::state::sidebar::split_section_heights;
  let (agents, wt, commits) = split_section_heights(21, 2, 4, 30, 50);
  assert_eq!(agents, 6, "agents must keep natural height (4 rows + borders)");
  assert_eq!((agents, wt, commits), (6, 8, 7));
}

#[test]
fn section_heights_keep_empty_sections_collapsed() {
  // A section with no content keeps its collapse behaviour: Agents hidden
  // when no session, Working Tree at 0 when the tree is clean. The
  // collapsed section never eats a 5-line floor.
  use gwm::tui::state::sidebar::split_section_heights;
  assert_eq!(split_section_heights(40, 2, 0, 5, 10), (0, 7, 33));
  assert_eq!(split_section_heights(30, 2, 2, 0, 8), (4, 0, 26));
}

#[test]
fn section_heights_degrade_commits_first_on_tiny_terminal() {
  // available below the floors' sum: hand out what exists with Recent
  // Commits served first (the historical always-visible section), then
  // Working Tree, then Agents. Sum must never exceed the available height.
  use gwm::tui::state::sidebar::split_section_heights;
  assert_eq!(split_section_heights(8, 2, 6, 30, 50), (0, 3, 5));
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
  assert_eq!(split_section_heights(8, 2, 0, 5, 0), (0, 5, 3));
}

#[test]
fn section_heights_give_everything_to_commits_when_alone() {
  // No agents, clean tree, empty history: Recent Commits keeps the whole
  // column, matching the pre-#438 rendering of an empty bottom panel.
  use gwm::tui::state::sidebar::split_section_heights;
  assert_eq!(split_section_heights(20, 2, 0, 0, 0), (0, 0, 20));
}

#[test]
fn stacked_table_pane_asks_for_what_it_draws() {
  // Issue #545: the pane reserved its percentage share whatever the row
  // count, so a five-worktree screen showed a column of blank rows above
  // a scrolling sidebar. It now asks for `rows + header + chrome` and the
  // sidebar takes back the rest.
  use gwm::tui::state::sidebar::stacked_table_height;
  // 5 worktrees, compact chrome, a 16-row quota: 5 + 1 header + 1 header
  // fill = 7, well under the quota, so 9 rows go to the sidebar.
  assert_eq!(stacked_table_height(16, 5, 1), 7);
  // Same list bordered: two rules instead of one filled header.
  assert_eq!(stacked_table_height(16, 5, 2), 8);
}

#[test]
fn stacked_table_pane_never_grows_past_its_quota() {
  // A long list must not push the sidebar off the screen: the share stays
  // the ceiling and the pane scrolls beyond it, exactly as before.
  use gwm::tui::state::sidebar::stacked_table_height;
  assert_eq!(stacked_table_height(16, 200, 1), 16);
  // Degenerate quota (a terminal too short to split) hands back the quota,
  // never a larger value the layout could not honour.
  assert_eq!(stacked_table_height(0, 5, 1), 0);
}

#[test]
fn section_heights_hand_the_saved_rows_back_in_compact_mode() {
  // Issue #545: compact mode replaces the two box rules with a single
  // filled header, so a section's chrome costs 1 row instead of 2. The
  // whole point of the mode is that those rows come back as content —
  // pinned here against the bordered baseline of
  // `section_heights_fit_naturally_with_commits_absorbing_slack`, same
  // inputs, chrome = 1.
  use gwm::tui::state::sidebar::split_section_heights;
  let bordered = split_section_heights(60, 2, 3, 10, 20);
  let compact = split_section_heights(60, 1, 3, 10, 20);
  assert_eq!(bordered, (5, 12, 43), "bordered baseline unchanged");
  assert_eq!(
    compact,
    (4, 11, 45),
    "each section sheds a chrome row, commits absorbs them"
  );
  // The column is fully used either way — a compact section must not
  // leave a blank row where its bottom rule used to be.
  assert_eq!(compact.0 + compact.1 + compact.2, 60);
}

#[test]
fn section_heights_scale_their_floors_with_the_chrome() {
  // The overflow floors are "chrome + N content rows", not the literals
  // 7 / 5: in compact mode a 7-row floor would hand Working Tree six
  // content rows where the bordered mode gives five, silently making the
  // denser layout *taller*. Same inputs as
  // `section_heights_guarantee_floor_and_share_proportionally_on_overflow`.
  use gwm::tui::state::sidebar::split_section_heights;
  let (agents, wt, commits) = split_section_heights(21, 1, 6, 30, 50);
  assert_eq!(
    (agents, wt, commits),
    (7, 7, 7),
    "floors follow the chrome (wt 1+5, commits 1+3)"
  );
  assert_eq!(agents + wt + commits, 21, "the split stays additive");
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
fn mux_pane_status_reports_the_multiplexers_own_refusal() {
  // Issue #588, second Codex pass. The spawn used to inherit both pipes,
  // which let a failing multiplexer draw its error over the ratatui frame;
  // sending them to `/dev/null` fixed that and traded it for a status bar
  // that said "opened" whatever happened. herdr answers a refusal with a
  // non-zero exit and a JSON body on stdout, so the message is built from
  // whichever stream spoke.
  let ok = gwm::tui::mux_pane_status("feat-7-foo", "pane", true, "{\"result\":{}}", "");
  assert_eq!(ok, "opened feat-7-foo in new pane");

  // The noun is a parameter because `t` no longer always opens a pane:
  // `mux_pane_direction = "window"` opens a tmux window or a zellij/herdr
  // tab, and a status bar that still said "pane" would be describing the
  // key rather than the screen (#589).
  let ok = gwm::tui::mux_pane_status("feat-7-foo", "tab", true, "", "");
  assert_eq!(ok, "opened feat-7-foo in new tab");

  let err = gwm::tui::mux_pane_status(
    "feat-7-foo",
    "pane",
    false,
    "{\"error\":{\"message\":\"unknown workspace w9Z\"}}\n",
    "",
  );
  assert!(
    err.contains("unknown workspace w9Z"),
    "the multiplexer's own words must reach the status bar, got: {}",
    err
  );
  assert!(!err.contains('\n'), "the status bar is one line, got: {}", err);

  // stderr wins when both spoke: tmux and zellij put their diagnostics
  // there, and it is the more specific of the two.
  let err = gwm::tui::mux_pane_status("feat-7-foo", "pane", false, "some stdout", "no server running");
  assert!(
    err.contains("no server running"),
    "expected the stderr text, got: {}",
    err
  );

  // A refusal with nothing on either stream still has to read as a failure,
  // not as a success with an empty reason.
  let quiet = gwm::tui::mux_pane_status("feat-7-foo", "pane", false, "", "");
  assert!(
    !quiet.starts_with("opened"),
    "a silent non-zero exit is still a failure, got: {}",
    quiet
  );
}

#[test]
fn mux_pane_without_a_selection_says_so_and_spawns_nothing() {
  // Issue #588. `t` on an empty list (or a filter that matches nothing) must
  // refuse on the status bar rather than reach the multiplexer with no path.
  let (_dir, mut app) = make_app();
  app.worktrees.clear();
  app.list_state.select(None);
  app.open_in_mux_pane_from(None, None, Some("1".into()), None);
  assert_eq!(
    app.status, "no worktree selected",
    "the selection gate comes before the multiplexer probe"
  );
}

#[test]
fn mux_pane_with_no_multiplexer_names_all_three_variables() {
  // The hint is the only thing a user gets when `t` does nothing, so it has
  // to name what gwm actually looked for. Before #588 it said `$TMUX /
  // $ZELLIJ`, which reads as "gwm has no idea what you are running" to
  // someone sitting in a herdr pane.
  //
  // The three values are passed in rather than removed from the environment:
  // `$TMUX` is also read by the clipboard path, so rewriting it here would
  // pull every yank test in this binary under the env lock.
  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("feat-7-foo")];
  app.list_state.select(Some(0));
  app.open_in_mux_pane_from(None, None, None, None);
  assert!(
    app.status.contains("$TMUX") && app.status.contains("$ZELLIJ") && app.status.contains("$HERDR_ENV"),
    "the hint must name all three probes, got: {}",
    app.status
  );
}

#[test]
fn the_mux_knobs_pick_the_mode_the_t_key_builds() {
  // `t` reads `[tui] mux_open_in` and `mux_pane_direction` (#608 / #589).
  // The spawn itself is not reachable from a test (it shells out to a
  // multiplexer that is not on the runner), so this pins the pure steps the
  // key runs on: the two config values it resolves, and the argv they
  // produce for the backend the cascade would have picked.
  use gwm::config::MuxTarget;
  use gwm::multiplexer::{build_command, spawn_noun, Multiplexer, SpawnMode, SplitDirection};

  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("feat-7-foo")];
  app.list_state.select(Some(0));
  let path = app.worktrees[0].path.clone();
  let mode_of = |app: &gwm::tui::App| app.config.tui.mux_open_in.spawn_mode(app.config.tui.mux_pane_direction);

  // Default: a pane on the right, where 1.9 and earlier left the choice to
  // the backend and tmux answered "below".
  assert_eq!(app.config.tui.mux_open_in, MuxTarget::Pane);
  assert_eq!(app.config.tui.mux_pane_direction, SplitDirection::Right);
  let mode = mode_of(&app);
  assert_eq!(mode, SpawnMode::Split(SplitDirection::Right));
  let argv = build_command(Multiplexer::Tmux, "feat-7-foo", &path, mode, None).unwrap();
  assert_eq!(argv[1], "split-window");
  assert_eq!(argv[2], "-h", "the default must reach tmux as `-h`, got: {:?}", argv);
  assert_eq!(spawn_noun(Multiplexer::Tmux, mode), "pane");

  app.config.tui.mux_pane_direction = SplitDirection::Down;
  let argv = build_command(Multiplexer::Tmux, "feat-7-foo", &path, mode_of(&app), None).unwrap();
  assert_eq!(argv[2], "-v", "`down` must reach tmux as `-v`, got: {:?}", argv);

  // `tab` is the whole-screen target: a tmux window, a zellij or herdr tab.
  // The direction is still set and must be ignored rather than leak.
  app.config.tui.mux_open_in = MuxTarget::Tab;
  let mode = mode_of(&app);
  assert_eq!(mode, SpawnMode::Window);
  let argv = build_command(Multiplexer::Tmux, "feat-7-foo", &path, mode, None).unwrap();
  assert_eq!(argv[1], "new-window", "`tab` must not split, got: {:?}", argv);
  assert!(
    !argv.iter().any(|a| a == "-v" || a == "-h"),
    "a leftover direction must not reach a window, got: {:?}",
    argv
  );
  assert_eq!(spawn_noun(Multiplexer::Tmux, mode), "window");
  assert_eq!(spawn_noun(Multiplexer::Zellij, mode), "tab");
  assert_eq!(spawn_noun(Multiplexer::Herdr, mode), "tab");

  // `workspace` is herdr's level and nobody else's: the other two refuse
  // instead of opening a tab, so the setting cannot describe something that
  // did not happen (#608).
  app.config.tui.mux_open_in = MuxTarget::Workspace;
  let mode = mode_of(&app);
  assert_eq!(mode, SpawnMode::Workspace);
  let argv = build_command(Multiplexer::Herdr, "feat-7-foo", &path, mode, None).unwrap();
  assert_eq!(argv[1], "workspace");
  assert_eq!(argv[2], "create");
  assert_eq!(spawn_noun(Multiplexer::Herdr, mode), "workspace");
  for mux in [Multiplexer::Tmux, Multiplexer::Zellij] {
    assert!(
      build_command(mux, "feat-7-foo", &path, mode, None).is_err(),
      "{mux:?} has no workspace level to open"
    );
  }
}

#[test]
fn the_t_key_puts_a_refused_target_on_the_status_bar() {
  // The refusal has to reach the user: `t` under `mux_open_in = "workspace"`
  // inside tmux opens nothing, and a silent no-op reads as a broken key.
  use gwm::config::MuxTarget;

  let (_dir, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("feat-7-foo")];
  app.list_state.select(Some(0));
  app.config.tui.mux_open_in = MuxTarget::Workspace;
  app.open_in_mux_pane_from(Some("/tmp/tmux-501/default,1,0".into()), None, None, None);
  assert!(
    app.status.contains("tmux") && app.status.contains("workspace"),
    "the status must name the backend and the level it cannot open, got: {}",
    app.status
  );
}

// ---------------------------------------------------------------------------
// Issue #591: `o` on the agents overlay resumes the session in a mux pane
// ---------------------------------------------------------------------------

/// An agents overlay open on one worktree carrying one detected session.
/// The snapshot is seeded directly rather than through `apply_agent_snapshot`
/// because what is under test is the overlay's `o`, not the landing path.
fn app_with_agent_overlay(kind: gwm::agent_sessions::AgentKind, id: &str, age_secs: u64) -> (tempfile::TempDir, App) {
  use gwm::agent_sessions::{AgentSession, WorktreeAgents};
  use std::collections::BTreeMap;
  let (dir, mut app) = make_app();
  let w = worktree_fixture("feat-591-foo");
  let mut map = BTreeMap::new();
  map.insert(
    gwm::agent_sessions::path_display_key(&w.path),
    WorktreeAgents {
      sessions: vec![AgentSession {
        kind,
        cwd: w.path.clone(),
        last_activity: std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs),
        ended: false,
        id: id.into(),
        name: None,
      }],
    },
  );
  app.agent_snapshot = Some(map);
  app.worktrees = vec![w];
  app.list_state.select(Some(0));
  app.open_agent_overlay();
  (dir, app)
}

#[test]
fn agent_pane_with_no_multiplexer_names_all_three_variables() {
  // #591 is multiplexer-only BY DESIGN: no PTY-overlay fallback, because the
  // point is to put the session next to gwm and an overlay that covers gwm
  // is not that. So the refusal is all the user gets, and it names what gwm
  // looked for — the same sentence `t` shows, not the macro path's shorter
  // "no multiplexer".
  let (_d, mut app) = app_with_agent_overlay(gwm::agent_sessions::AgentKind::ClaudeCode, "s1", 10);
  app.open_selected_agent_pane_from(None, None, None, None);
  assert!(
    app.status.contains("$TMUX") && app.status.contains("$ZELLIJ") && app.status.contains("$HERDR_ENV"),
    "the hint must name all three probes, got: {}",
    app.status
  );
}

#[test]
fn agent_pane_inside_herdr_plans_the_sequenced_round_trip() {
  // #591 after the herdr measurement. herdr takes no command in the argv
  // that opens the container, which is why `macro_refusal` refuses it for a
  // `[tui.macro*]`. It is not unable to run one: the command goes in
  // afterwards through the pane id the response carries (#599).
  //
  // Planned rather than driven: the sequenced path spawns a worker that
  // talks to a real herdr, and a test must not open a pane in the developer's
  // session. What the worker does with this plan is covered by the parser
  // tests in `multiplexer_tests.rs` and by the drain test below.
  use gwm::agent_sessions::{AgentKind, AgentSession};
  let session = AgentSession {
    kind: AgentKind::Codex,
    cwd: std::path::PathBuf::from("/tmp/gwm-test/feat-591-foo"),
    last_activity: std::time::SystemTime::now(),
    ended: true,
    id: "s1".into(),
    name: None,
  };
  let plan = gwm::tui::plan_agent_pane(
    &session,
    std::path::Path::new("/tmp/gwm-test/feat-591-foo"),
    &gwm::config::TuiConfig::default(),
    None,
    None,
    Some("1".into()),
    None,
  )
  .expect("herdr can resume, it just needs two steps");
  let gwm::tui::AgentPanePlan::Sequenced { open, line, noun } = plan else {
    panic!("herdr must plan the sequenced round trip, not a one-shot argv");
  };
  assert_eq!(open[0], "herdr");
  assert!(
    open.iter().any(|a| a == "split") && open.iter().any(|a| a == "/tmp/gwm-test/feat-591-foo"),
    "the container opens in the overlay's worktree, got: {open:?}"
  );
  assert!(
    !open.iter().any(|a| a.contains("codex resume")),
    "the command must NOT ride the opening argv: herdr ignores it there, got: {open:?}"
  );
  assert_eq!(line, "codex resume s1", "the line is typed in afterwards");
  assert_eq!(noun, "pane");
}

#[test]
fn agent_pane_under_a_zellij_tab_is_still_refused() {
  // The refusal families herdr no longer belongs to are untouched: a zellij
  // TAB takes no trailing command in any shape, and there is no pane id to
  // type into afterwards either.
  let (_d, mut app) = app_with_agent_overlay(gwm::agent_sessions::AgentKind::Opencode, "s1", 10);
  app.config.tui.mux_open_in = gwm::config::MuxTarget::Tab;
  app.open_selected_agent_pane_from(None, Some("0".into()), None, None);
  assert!(
    app.status.contains("zellij") && app.status.contains("no command"),
    "expected the zellij-tab refusal, got: {}",
    app.status
  );
}

#[test]
fn agent_pane_worker_result_reaches_the_status_bar() {
  // The herdr path answers through the task drain, so the worker's wording
  // is what the user reads. Both arms, because a failure at step 2 or 3
  // leaves a container open and saying "opened" there would be a lie.
  let (_d, mut app) = make_app();
  let generation = app.tasks.request(gwm::tui::TaskKind::AgentPane).unwrap();
  app.apply_agent_pane_result(generation, Ok("opened codex session in new pane".into()));
  assert_eq!(app.status, "opened codex session in new pane");

  let generation = app.tasks.request(gwm::tui::TaskKind::AgentPane).unwrap();
  app.apply_agent_pane_result(generation, Err("herdr refused the resume: no such pane".into()));
  assert!(
    app.status.contains("no such pane") && !app.status.starts_with("opened"),
    "a refusal must not read as a success, got: {}",
    app.status
  );

  // A superseded worker cannot clobber a newer one's status.
  let stale = 0;
  app.status = "current".into();
  app.apply_agent_pane_result(stale, Ok("late arrival".into()));
  assert_eq!(app.status, "current", "a stale generation is dropped");
}

#[test]
fn agent_pane_refuses_a_zellij_tab_because_the_level_is_a_setting_now() {
  // `o` reads `[tui] mux_open_in` like `t` does (#608), so its refusals are
  // the whole `macro_refusal` set, not just herdr: a zellij TAB takes no
  // trailing command either. Hardcoding the herdr sentence would make `o`
  // lie under a setting the user can already flip for `t`.
  let (_d, mut app) = app_with_agent_overlay(gwm::agent_sessions::AgentKind::Opencode, "s1", 10);
  app.config.tui.mux_open_in = gwm::config::MuxTarget::Tab;
  app.open_selected_agent_pane_from(None, Some("0".into()), None, None);
  assert!(
    app.status.contains("zellij") && app.status.contains("no command"),
    "expected the zellij-tab refusal, got: {}",
    app.status
  );
}

#[test]
fn agent_pane_without_a_selected_session_refuses() {
  // `agent_detail_rows` emits a `no agent session found` placeholder with no
  // meta. `attach` deliberately falls through to the attach-by-id prompt
  // there; `o` has nothing to resume, so it refuses instead — opening a
  // prompt from a key that promises a pane would be a different verb.
  let (_d, mut app) = make_app();
  app.worktrees = vec![worktree_fixture("feat-591-foo")];
  app.list_state.select(Some(0));
  app.open_agent_overlay();
  app.open_selected_agent_pane_from(None, None, Some("1".into()), None);
  assert!(
    app.status.contains("no agent session"),
    "expected a refusal naming the missing session, got: {}",
    app.status
  );
  assert_eq!(
    app.detail_overlay.mode,
    gwm::tui::state::detail_overlay::DetailMode::List,
    "`o` must not open the attach-by-id prompt the way `a` does"
  );
}

#[test]
fn agent_pane_opens_at_the_overlay_target_not_the_recorded_cwd() {
  // The defect the obvious reading of #591 ships. The overlay lists PINNED
  // sessions too, and a pin exists exactly when the recorded directory names
  // the wrong tree — `gwm agents attach` right after `gwm create` is the
  // documented workflow, so this is the common row, not the exotic one.
  // `overlay_pins` leaves `cwd` alone "purely as provenance", and for a
  // pinned Claude session resolved by the id sweep it is the slug directory
  // under `~/.claude/projects`: resuming there drops the agent inside its own
  // artefact store.
  use gwm::agent_sessions::{AgentKind, AgentSession};
  let session = AgentSession {
    kind: AgentKind::ClaudeCode,
    cwd: std::path::PathBuf::from("/home/u/.claude/projects/-home-u-main-checkout"),
    last_activity: std::time::SystemTime::now(),
    ended: true,
    id: "s1".into(),
    name: None,
  };
  let target = std::path::Path::new("/tmp/gwm-test/feat-591-foo");
  let plan = gwm::tui::plan_agent_pane(
    &session,
    target,
    &gwm::config::TuiConfig::default(),
    Some("/tmp/sock,1,0".into()),
    None,
    None,
    None,
  )
  .expect("tmux takes a command");
  let gwm::tui::AgentPanePlan::OneShot { argv, .. } = plan else {
    panic!("tmux carries its command in the opening argv");
  };
  assert!(
    argv.iter().any(|a| a == "/tmp/gwm-test/feat-591-foo"),
    "the pane must open in the worktree the overlay is about, got: {argv:?}"
  );
  assert!(
    !argv.iter().any(|a| a.contains(".claude/projects")),
    "the recorded cwd is provenance, not a place to run an agent: {argv:?}"
  );
  // And it resumes the session rather than landing a bare shell there.
  assert_eq!(argv.last().map(String::as_str), Some("claude -r s1"), "got: {argv:?}");
}

#[test]
#[cfg(unix)]
fn agent_pane_spawn_hands_the_planned_command_to_the_multiplexer() {
  // Copilot review on PR #610: every other test here stops at a refusal or
  // at pure argv planning, so a regression that plans correctly and then
  // never launches — or launches with the wrong cwd, or builds the status
  // from the wrong session — would pass. This one drives the real spawn
  // against a recording fake `tmux`, the same shape `cli_binary.rs` uses for
  // the CLI verb.
  //
  // Unix-only for the same reason the `glab` fake is: what is under test is
  // gwm's argv and status, and a `.cmd` shim would mostly exercise `cmd.exe`
  // quoting rules instead.
  use std::os::unix::fs::PermissionsExt;

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let fake = tempfile::TempDir::new().unwrap();
  let log = fake.path().join("argv.log");
  let tmux = fake.path().join("tmux");
  std::fs::write(
    &tmux,
    format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\n", log.display()),
  )
  .unwrap();
  let mut perms = std::fs::metadata(&tmux).unwrap().permissions();
  perms.set_mode(0o755);
  std::fs::set_permissions(&tmux, perms).unwrap();

  // An IDLE session (2h old), so the status must NOT carry the live warning.
  let (_d, mut app) = app_with_agent_overlay(gwm::agent_sessions::AgentKind::ClaudeCode, "s1", 7200);

  let previous = std::env::var("PATH").ok();
  // SAFETY: env mutation is guarded by `env_lock()` above.
  unsafe {
    std::env::set_var(
      "PATH",
      format!("{}:{}", fake.path().display(), previous.clone().unwrap_or_default()),
    );
  }
  app.open_selected_agent_pane_from(Some("/tmp/tmux-501/default,1,0".into()), None, None, None);
  unsafe {
    match previous {
      Some(v) => std::env::set_var("PATH", v),
      None => std::env::remove_var("PATH"),
    }
  }

  let argv = std::fs::read_to_string(&log).unwrap_or_default();
  assert!(
    argv.contains("split-window"),
    "the planned argv must actually reach the multiplexer, got: {argv:?} / status: {}",
    app.status
  );
  assert!(
    argv.contains("claude -r s1"),
    "the pane must run the RESUME command, not a bare shell, got: {argv:?}"
  );
  assert!(
    argv.contains("/tmp/gwm-test/feat-591-foo"),
    "the pane must open in the overlay's worktree, got: {argv:?}"
  );
  // The status is built from the spawn's own outcome and this session's
  // freshness, which is the wiring the planning tests cannot see.
  assert_eq!(
    app.status, "opened claude session in new pane",
    "an idle session opens without the live warning"
  );
}

#[test]
fn agent_pane_status_warns_when_the_session_is_still_active() {
  // A session with `ended = true` resumes without comment: that is what
  // resume is for. A LIVE one is the interesting case — resuming it in a
  // second pane while it runs elsewhere may fork or refuse depending on the
  // tool, so the status says so rather than leaving a silent second pane.
  let live = gwm::tui::agent_pane_status("claude", "pane", true, true, "", "");
  assert!(
    live.contains("still active"),
    "a live session must be flagged, got: {}",
    live
  );
  let idle = gwm::tui::agent_pane_status("claude", "pane", false, true, "", "");
  assert!(
    !idle.contains("still active"),
    "an idle session resumes without a warning, got: {}",
    idle
  );
  assert!(idle.contains("claude"), "the status names the backend, got: {}", idle);
  // The noun comes from `spawn_noun`, so `mux_open_in = "tab"` does not leave
  // the status describing a pane the user is not looking at (#589).
  let tab = gwm::tui::agent_pane_status("claude", "tab", false, true, "", "");
  assert!(tab.contains("tab") && !tab.contains("pane"), "got: {}", tab);

  // A refusal keeps the multiplexer's own words and gains no warning: what
  // failed is the spawn, and "still active elsewhere" would read as a cause.
  let refused = gwm::tui::agent_pane_status("claude", "pane", true, false, "", "no server running");
  assert!(
    refused.contains("no server running") && !refused.contains("still active"),
    "got: {}",
    refused
  );
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
  // "refreshed: N" message ran last and stood. Now both drain in one pass; the
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
    app.status.starts_with("refreshed:"),
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
    detail: Default::default(),
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
  // Re-resolve the slug now that the remote exists.
  // NOTE: `refresh_link` is deliberately called *after* `GWM_GH` is set.
  // Since #419 the forge captures its program at construction, which is
  // what keeps the TUI's off-thread fetch from re-reading the environment
  // (the #217 contract). Resolving the link before the override would pin
  // the real `gh` and never see the fake.
  app.refresh_link();

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
  // Re-resolve so the swapped `GWM_GH` is picked up — see the note above.
  app.refresh_link();
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
  // `refresh_link` after each override: since #419 the forge captures its
  // program at construction, which is what keeps the off-thread fetch from
  // re-reading the environment (the #217 contract). Swapping `GWM_GH`
  // without re-resolving would keep calling the previous fake.
  app.refresh_link();
  app.refresh_github_status();
  assert_eq!(app.current_link().pr, Some(128), "first refresh detects #128");

  // Now `gh` fails. The probe did not prove the PR vanished.
  unsafe {
    std::env::set_var("GWM_GH", &gh_fail);
  }
  app.refresh_link();
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
  let link = gwm::github::read_link_with_pr_detection(
    &repo,
    "detect-me",
    // Built here, after `GWM_GH` is set: the backend captures the
    // program at construction (issue #419 keeps #217's off-thread
    // contract by resolving the env once, up front).
    gwm::forge::for_kind(
      gwm::forge::ForgeKind::GitHub,
      gwm::forge::parse_remote_url("https://github.com/kbrdn1/gwm-cli").unwrap(),
    )
    .as_ref(),
  )
  .unwrap();
  // The live path must reconcile the persisted cache (#284), not just memory,
  // so no-fetch consumers (table at startup, `gwm open pr`) don't resurrect
  // the stale #128. Capture the stored value before the explicit link below.
  let reconciled = gwm::github::read_link(&repo, "detect-me").unwrap();

  // Explicit override still wins even over a live re-detection.
  gwm::github::link_pr(&repo, "detect-me", 61).unwrap();
  let explicit = gwm::github::read_link_with_pr_detection(
    &repo,
    "detect-me",
    // Built here, after `GWM_GH` is set: the backend captures the
    // program at construction (issue #419 keeps #217's off-thread
    // contract by resolving the env once, up front).
    gwm::forge::for_kind(
      gwm::forge::ForgeKind::GitHub,
      gwm::forge::parse_remote_url("https://github.com/kbrdn1/gwm-cli").unwrap(),
    )
    .as_ref(),
  )
  .unwrap();

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
  let link = gwm::github::read_link_with_pr_detection(
    &repo,
    "detect-me",
    // Built here, after `GWM_GH` is set: the backend captures the
    // program at construction (issue #419 keeps #217's off-thread
    // contract by resolving the env once, up front).
    gwm::forge::for_kind(
      gwm::forge::ForgeKind::GitHub,
      gwm::forge::parse_remote_url("https://github.com/kbrdn1/gwm-cli").unwrap(),
    )
    .as_ref(),
  )
  .unwrap();

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
  let link = gwm::github::read_link_with_pr_detection(
    &repo,
    "detect-me",
    // Built here, after `GWM_GH` is set: the backend captures the
    // program at construction (issue #419 keeps #217's off-thread
    // contract by resolving the env once, up front).
    gwm::forge::for_kind(
      gwm::forge::ForgeKind::GitHub,
      gwm::forge::parse_remote_url("https://github.com/kbrdn1/gwm-cli").unwrap(),
    )
    .as_ref(),
  )
  .unwrap();
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
    detail: Default::default(),
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
fn table_marker_paints_both_unfetched_pastilles_white() {
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
      ("●".to_string(), Some(Color::White)),    // issue linked, unfetched → name role
      ("/".to_string(), Some(Color::DarkGray)), // muted separator
      ("●".to_string(), Some(Color::White)),    // pr linked, unfetched → name role too (#596)
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
  assert_eq!(cells[0].1, Some(Color::White), "unfetched issue dot white (#596)");
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
    detail: Default::default(),
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
    detail: Default::default(),
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
  assert_eq!(cells[2].1, Some(Color::White), "unfetched pr dot white (#596)");
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
    has_note: false,
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
    true,
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
    true,
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
    true,
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
    true,
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

// ---- `[tui] status_one_line` — the folded Status row (issue #547) ---------

/// A fixture carrying every foldable value at once: branch, head, a dirty
/// state, a non-empty diff, and a measurable age. The labelled block spends
/// four rows on these; the fold spends one.
fn foldable_worktree_fixture() -> WorktreeInfo {
  let mut w = detailed_worktree_fixture();
  w.is_main = false;
  w.age = Some(std::time::Duration::from_secs(3 * 24 * 60 * 60));
  w.status = BranchStatus {
    is_dirty: true,
    has_upstream: true,
    ahead: 0,
    behind: 0,
    unknown: false,
  };
  w
}

#[test]
fn status_fold_carries_every_value_of_the_labelled_block() {
  let w = foldable_worktree_fixture();
  let diff = gwm::worktree::DiffLineStat {
    insertions: 12,
    deletions: 4,
  };
  let row = section_text_single(&gwm::tui::folded_status_line(&w, Some(&diff), &Theme::default()));

  for needle in ["feat/#42-api-rest", "08d1029", "dirty", "+12", "-4", "3d"] {
    assert!(row.contains(needle), "folded row must carry {needle:?}: {row}");
  }
  // The labels are what the fold buys back — four of them, one per row.
  for label in ["Branch ", "Created", "Diff ", "State "] {
    assert!(!row.contains(label), "folded row must drop the {label:?} label: {row}");
  }
}

#[test]
fn status_fold_orders_identity_first_and_age_last() {
  // The sidebar renders without `Wrap`, so a row wider than the pane is
  // hard-clipped on the right: segment order *is* the width policy (open
  // question 2 of #547). Identity leads, `Created` trails, because age is
  // the value the pane can most afford to lose.
  let w = foldable_worktree_fixture();
  let diff = gwm::worktree::DiffLineStat {
    insertions: 12,
    deletions: 4,
  };
  let row = section_text_single(&gwm::tui::folded_status_line(&w, Some(&diff), &Theme::default()));
  let at = |needle: &str| {
    row
      .find(needle)
      .unwrap_or_else(|| panic!("{needle:?} missing from {row}"))
  };

  assert!(at("feat/#42-api-rest") < at("08d1029"), "branch before head: {row}");
  assert!(at("08d1029") < at("dirty"), "head before state: {row}");
  assert!(at("dirty") < at("+12"), "state before diff: {row}");
  assert!(at("+12") < at("3d"), "diff before age — age clips first: {row}");
}

#[test]
fn status_fold_keeps_the_theme_roles_of_the_labelled_block() {
  // The fold is a change of shape, not of colour: every segment keeps the
  // role it wears in the labelled block. Unique `Rgb` values so a hardcoded
  // `Color::Red` cannot pass here (the #170/#211 rule).
  let theme = Theme {
    prunable: Color::Rgb(40, 50, 60),
    untracked: Color::Rgb(10, 20, 30),
    dirty: Color::Rgb(70, 80, 90),
    ..Theme::default()
  };
  let w = foldable_worktree_fixture();
  let diff = gwm::worktree::DiffLineStat {
    insertions: 12,
    deletions: 4,
  };
  let line = gwm::tui::folded_status_line(&w, Some(&diff), &theme);
  let fg = |needle: &str| -> Option<Color> {
    line
      .spans
      .iter()
      .find(|s| s.content.contains(needle))
      .unwrap_or_else(|| panic!("no span carrying {needle:?} in {}", section_text_single(&line)))
      .style
      .fg
  };

  assert_eq!(fg("feat/#42-api-rest"), Some(theme.prunable), "dirty branch → prunable");
  assert_eq!(fg("08d1029"), Some(theme.dirty), "short head → dirty role");
  assert_eq!(fg("+12"), Some(theme.untracked), "insertions → untracked");
  assert_eq!(fg("-4"), Some(theme.prunable), "deletions → prunable");
}

#[test]
fn status_fold_skips_the_segments_the_labelled_block_skips() {
  // No head, no diff, no age → those segments are absent rather than
  // rendered empty, exactly as the labelled block omits their rows.
  let mut w = foldable_worktree_fixture();
  w.head = None;
  w.age = None;
  let row = section_text_single(&gwm::tui::folded_status_line(&w, None, &Theme::default()));

  assert!(row.contains("feat/#42-api-rest"), "branch survives: {row}");
  assert!(row.contains("dirty"), "state survives: {row}");
  assert!(!row.contains('+'), "no diff segment without a stat: {row}");
  assert!(
    !row.ends_with('·') && !row.contains("· ·"),
    "no dangling separator: {row}"
  );
}

#[test]
fn status_one_line_folds_the_identity_block_to_two_rows() {
  let w = foldable_worktree_fixture();
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    Some(gwm::worktree::DiffLineStat {
      insertions: 12,
      deletions: 4,
    }),
    &Theme::default(),
    true,
  );

  assert_eq!(
    sections.worktree.len(),
    2,
    "folded status + path, nothing else: {:?}",
    sections.worktree.iter().map(section_text_single).collect::<Vec<_>>()
  );
  let status = section_text_single(&sections.worktree[0]);
  for needle in ["feat/#42-api-rest", "08d1029", "dirty", "+12", "3d"] {
    assert!(
      status.contains(needle),
      "row 1 is the folded status, not an empty line — missing {needle:?}: {status}"
    );
  }
  assert!(
    section_text_single(&sections.worktree[1]).contains("Path"),
    "the path keeps its own labelled row: {}",
    section_text_single(&sections.worktree[1])
  );
}

#[test]
fn status_one_line_off_keeps_the_labelled_block() {
  let w = foldable_worktree_fixture();
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    Some(gwm::worktree::DiffLineStat {
      insertions: 12,
      deletions: 4,
    }),
    &Theme::default(),
    false,
  );
  let text = section_text(&sections.worktree);

  assert_eq!(sections.worktree.len(), 5, "branch, created, diff, state, path: {text}");
  for label in ["Branch", "Created", "Diff", "State", "Path"] {
    assert!(text.contains(label), "{label} row still labelled: {text}");
  }
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
  let sections = build_sidebar_sections(
    &w,
    gwm::tui::state::sidebar::SidebarMode::Commits,
    Some(diff),
    &theme,
    false,
  );

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
      false,
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
    true,
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
    false,
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
    false,
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
    false,
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

#[test]
fn tilde_compress_tolerates_a_trailing_separator_on_home() {
  // `HOME=/home/alice/` is legal, and `dirs::home_dir()` keeps the separator
  // verbatim rather than normalising it away — measured, not assumed:
  // `HOME=/home/alice/` yields `Some("/home/alice/")` and `HOME=/home/alice//`
  // yields `Some("/home/alice//")`.
  //
  // Left in, the prefix strips `/home/alice/repo` down to `repo`, the boundary
  // check then sees no leading separator and refuses, and every surface stays
  // absolute for exactly those users.
  for home in ["/home/alice/", "/home/alice//"] {
    let home = std::path::Path::new(home);
    assert_eq!(
      tilde_compress_with_home("/home/alice/repo", home),
      "~/repo",
      "home={home:?}"
    );
    assert_eq!(tilde_compress_with_home("/home/alice", home), "~", "home={home:?}");
    // The boundary guard survives the trim: a longer sibling is still refused.
    assert_eq!(
      tilde_compress_with_home("/home/alicent/x", home),
      "/home/alicent/x",
      "home={home:?}"
    );
  }
}

#[cfg(windows)]
#[test]
fn tilde_compress_matches_across_the_two_windows_separators() {
  // The two sources spell the same path differently, which is why a
  // byte-for-byte prefix match never fired here: `WorktreeInfo::path` comes
  // from libgit2, which emits `/` even on Windows, while `dirs::home_dir()`
  // returns `\`. Compression was therefore a silent no-op on the platform,
  // for the header and the sidebar as much as for the table's `PATH` column.
  //
  // `#[cfg(windows)]` rather than unconditional, because the behaviour under
  // test *is* the platform's: on Unix a backslash is an ordinary character in
  // a directory name, and accepting it as a boundary would reopen the slice
  // PR #70 closed. The runner is the only oracle, so this one is proven by CI.
  let home = std::path::Path::new(r"C:\Users\alice");
  assert_eq!(tilde_compress_with_home("C:/Users/alice/repo", home), "~/repo");
  assert_eq!(tilde_compress_with_home(r"C:\Users\alice\repo", home), r"~\repo");
  assert_eq!(tilde_compress_with_home("C:/Users/alice", home), "~");
  // The boundary guard still holds across spellings.
  assert_eq!(
    tilde_compress_with_home("C:/Users/alicent/x", home),
    "C:/Users/alicent/x",
    "a longer sibling must not be sliced just because the separators differ"
  );
}

// ---- how a worktree path is spelled on screen (issue #568) ----------------

use gwm::tui::display_path_with_home;

#[test]
fn a_displayed_path_compresses_home_like_the_header_does() {
  // Issue #568: the header already tilde-compressed, the table printed the
  // same value raw, so `$HOME` was re-spent on every row of a column that is
  // `Fill(1)` and vanishes first.
  let home = std::path::Path::new("/home/alice");
  assert_eq!(
    display_path_with_home("/home/alice/gwm-demo/worktrees/acme-api/", home),
    // Trailing separator kept: `w.path` carries one in production, and the
    // column has no business rewriting the value beyond the prefix.
    "~/gwm-demo/worktrees/acme-api/"
  );
  assert_eq!(display_path_with_home("/var/lib/acme", home), "/var/lib/acme");
}

#[test]
fn a_displayed_path_compresses_before_it_sanitises() {
  // The order is not interchangeable, which is the whole reason this pair
  // exists rather than the two helpers being composed at each call site.
  // Compression matches the home prefix byte for byte, so it has to run on
  // the raw path: sanitising first rewrites whatever `$HOME` itself carries
  // into `?`, the prefix stops matching the real `dirs::home_dir()`, and
  // compression silently stops firing for exactly the users whose home is
  // hostile. Sanitise-first would yield `/home/al?ice/wt` here.
  let home = std::path::Path::new("/home/al\u{202E}ice");
  assert_eq!(display_path_with_home("/home/al\u{202E}ice/wt", home), "~/wt");
}

#[test]
fn a_displayed_path_still_sanitises_what_compression_leaves_behind() {
  // Compressing must not become a way past the sink: the tail the tilde does
  // not swallow is the part a hostile worktree directory name arrives in
  // (issue #506), and the table cell is still outside `trunc`'s funnel.
  let home = std::path::Path::new("/home/alice");
  assert_eq!(
    display_path_with_home("/home/alice/wt\u{202E}x", home),
    "~/wt?x",
    "a bidi control below home must not ride the tilde into the cell"
  );
  assert_eq!(
    display_path_with_home("/var/wt\u{202E}x", home),
    "/var/wt?x",
    "an uncompressed path must be sanitised exactly as before"
  );
}

// ---- Issue / PR summary line width budgeting ----------------------------

use gwm::tui::{issue_summary_line, pr_summary_line};

fn line_visible_width(line: &ratatui::text::Line<'static>) -> usize {
  // The cells `set_stringn` paints, span by span, which is how ratatui draws a
  // `Line`. Not `chars().count()`, the measure the builders under test use:
  // that would agree with them whatever they did (issue #563).
  line.spans.iter().map(|s| painted(&s.content)).sum()
}

fn painted(s: &str) -> usize {
  let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 200, 1));
  let (x, _) = buf.set_stringn(0, 0, s, 200, ratatui::style::Style::default());
  usize::from(x)
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    detail: Default::default(),
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
    has_note: false,
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
    true,
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
  let _env = env_lock().lock().unwrap_or_else(|e| e.into_inner());
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
  let _env = env_lock().lock().unwrap_or_else(|e| e.into_inner());
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
  let _env = env_lock().lock().unwrap_or_else(|e| e.into_inner());
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
  let _env = env_lock().lock().unwrap_or_else(|e| e.into_inner());
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
  let _env = env_lock().lock().unwrap_or_else(|e| e.into_inner());
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
  let _env = env_lock().lock().unwrap_or_else(|e| e.into_inner());
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
  use gwm::tui::state::async_task::{DeleteBatchOutcome, TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let generation = app.tasks.request(TaskKind::DeleteWorktree).unwrap();
  app.view = View::Confirm;
  app.delete_failure = Some("old failure".into());

  app
    .task_result_sender()
    .send(TaskMsg::DeleteWorktree(
      generation,
      DeleteBatchOutcome {
        removed: vec![("alpha".into(), "/tmp/alpha".into())],
        failed: vec![],
        warnings: vec![],
      },
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
  use gwm::tui::state::async_task::{DeleteBatchOutcome, DeleteFailure, TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let generation = app.tasks.request(TaskKind::DeleteWorktree).unwrap();
  app.view = View::Confirm;

  app
    .task_result_sender()
    .send(TaskMsg::DeleteWorktree(
      generation,
      DeleteBatchOutcome {
        removed: vec![],
        failed: vec![DeleteFailure {
          id: "alpha".into(),
          path: "/tmp/alpha".into(),
          error: "permission denied".into(),
        }],
        warnings: vec![],
      },
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
  use gwm::tui::state::async_task::{DeleteBatchOutcome, TaskKind, TaskMsg};
  let (_dir, mut app) = make_app();
  let stale = app.tasks.request(TaskKind::DeleteWorktree).unwrap();
  app.tasks.invalidate(TaskKind::DeleteWorktree);
  app.view = View::Confirm;
  app.status = "untouched".into();

  app
    .task_result_sender()
    .send(TaskMsg::DeleteWorktree(
      stale,
      DeleteBatchOutcome {
        removed: vec![("alpha".into(), "/tmp/alpha".into())],
        failed: vec![],
        warnings: vec![],
      },
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
  // Looked up rather than hard-coded: the tab's order is a design call
  // and moved when #545 put `layout` at the top. An index literal made
  // this test fail for a reason that had nothing to do with what it
  // checks.
  app.config_panel.selected = SettingsTab::Tui
    .fields()
    .iter()
    .position(|f| *f == gwm::tui::SettingField::SidebarPosition)
    .expect("the TUI tab must offer sidebar position");
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

/// Issue #479's conversion table, all four cells. `WorktreeName` already knows
/// how each shape becomes a branch and a directory, so the rename target is
/// built exactly the way `submit_create` builds the create target, and the
/// table is two code paths rather than four.
///
/// The worker is never reached here — `spawn_edit_worktree` needs a real repo —
/// so what these assert is the *target* the form composes, read back off the
/// task the submit requested.
#[test]
fn the_rename_form_composes_a_target_for_each_of_the_four_conversions() {
  use gwm::tui::state::create_form::Mode;

  // free-form → free-form: the name is the branch, verbatim.
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("spike-redis");
  wt.branch = Some("spike-redis".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.create_form.mode, Mode::Freeform);
  app.create_form.name = "spike-valkey".into();
  assert_eq!(
    app.edit_target().expect("a legal free-form name composes a target"),
    ("spike-valkey".to_string(), "spike-valkey".to_string()),
    "free-form → free-form writes the name verbatim as branch and directory"
  );

  // free-form → structured: the pattern is applied after the fact.
  app.create_form.mode = Mode::Structured;
  app.create_form.type_index = app
    .branch_types
    .iter()
    .position(|t| t.name == "feat")
    .expect("`feat` is a built-in branch type");
  app.create_form.issue = "42".into();
  app.create_form.desc = "cache-layer".into();
  assert_eq!(
    app.edit_target().expect("a complete triple composes a target"),
    ("feat/#42-cache-layer".to_string(), "feat-42-cache-layer".to_string()),
    "free-form → structured promotes the spike into the convention"
  );

  // structured → free-form: the name is the branch, verbatim, again.
  let (_dir2, mut app) = make_app();
  let mut wt = worktree_fixture("feat-42-cache-layer");
  wt.branch = Some("feat/#42-cache-layer".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.create_form.mode, Mode::Structured);
  app.create_form.mode = Mode::Freeform;
  app.create_form.name = "spike-again".into();
  assert_eq!(
    app.edit_target().expect("composes"),
    ("spike-again".to_string(), "spike-again".to_string()),
    "structured → free-form drops out of the convention on purpose"
  );

  // structured → structured: unchanged from before #479.
  app.create_form.mode = Mode::Structured;
  app.create_form.desc = "other-desc".into();
  assert_eq!(
    app.edit_target().expect("composes"),
    ("feat/#42-other-desc".to_string(), "feat-42-other-desc".to_string()),
    "structured → structured is exactly what #290 did"
  );
}

#[test]
fn the_rename_form_refuses_a_free_form_name_create_would_refuse() {
  // Same validator, so a name one form turns away the other turns away too:
  // `WorktreeName::freeform` is the single set of rules (#416), and a rename
  // that accepted more than create would produce worktrees create could never
  // have made.
  use gwm::tui::state::create_form::Mode;
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("spike-redis");
  wt.branch = Some("spike-redis".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.create_form.mode, Mode::Freeform);

  for refused in ["HEAD", "spike-{issue}", "-spike", "..", " spike"] {
    app.create_form.name = refused.into();
    assert!(
      gwm::naming::WorktreeName::freeform(refused).is_err(),
      "`{}` is documented as refused by create",
      refused
    );
    app.edit_failure = None;
    app
      .submit_edit_worktree()
      .expect("a refusal is a form failure, not an error");
    assert_eq!(app.view, View::Edit, "the form stays open on `{}`", refused);
    assert!(
      app.edit_failure.is_some(),
      "`{}` must be refused by rename too, with a reason",
      refused
    );
  }
}

#[test]
fn the_seeded_description_respects_the_bound_typing_it_would_have() {
  // Codex review on PR #485. A free-form name may run to
  // `MAX_DIR_COMPONENT_BYTES` (255), while the description field caps typed
  // input at `MAX_DESC_LEN` (200) — and that cap exists so
  // `<type>/#<issue>-<desc>` cannot exceed git's ref limit. `push_char` is the
  // only place it was enforced, so seeding the field wrote past a bound no
  // keystroke could have crossed, and `BranchSpec` has no length check to
  // catch it downstream.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::state::create_form::{Mode, MAX_DESC_LEN};
  let long = "a".repeat(250);
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("spike");
  wt.branch = Some(long.clone());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.create_form.mode, Mode::Freeform);
  assert_eq!(app.create_form.name, long, "the whole name is a legal free-form branch");

  app.handle_create_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));

  assert_eq!(app.create_form.mode, Mode::Structured);
  assert!(
    app.create_form.desc.chars().count() <= MAX_DESC_LEN,
    "the seed must not write past the bound typing enforces: {} chars",
    app.create_form.desc.chars().count()
  );
}

#[test]
fn a_corrected_rename_is_not_held_back_by_the_previous_attempt() {
  // Codex review on PR #485. The frozen-segment guard reports through
  // `edit_failure`, and the call site read that same field back to decide
  // whether to stop — so an *earlier* failure still sitting there stopped the
  // next submit too, whatever the user had just fixed, and the form could only
  // be unstuck by closing and reopening it. The guard returns its own verdict
  // now, so the only thing that can stop a submit is that submit.
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#42-my-desc".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit);

  // A first submit that fails on its own terms: an empty description is not a
  // branch `BranchSpec` will build.
  app.create_form.desc = String::new();
  app
    .submit_edit_worktree()
    .expect("a refusal is a form failure, not an error");
  assert!(app.edit_failure.is_some(), "an empty description is refused");

  // Fixing it has to be enough.
  app.create_form.desc = "other-desc".into();
  app.submit_edit_worktree().expect("submits");
  assert_eq!(
    app.edit_failure, None,
    "a corrected form must not be held back by the previous attempt's message"
  );
}

#[test]
fn the_rename_modal_opens_a_free_form_worktree_in_free_form_mode() {
  // Issue #479. `worktree_spec` starts with `branch_parser.parse(branch)?`, so a
  // name the user chose on purpose does not parse and the modal used to refuse
  // outright. It is the one shape the rename form exists for and the one shape
  // it turned away: renaming a spike that outlived its name meant `git branch -m`
  // plus `git worktree move` by hand, two commands that have to agree.
  use gwm::tui::state::create_form::Mode;
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("spike-redis");
  wt.branch = Some("spike-redis".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(
    app.view,
    View::Edit,
    "the modal opens instead of refusing: {}",
    app.status
  );
  assert_eq!(app.create_form.mode, Mode::Freeform);
  assert_eq!(
    app.create_form.name, "spike-redis",
    "prefilled with the current name, so the common edit is a small one"
  );
  assert_eq!(app.create_form.field, Field::Name, "free-form's only field");
}

#[test]
fn ctrl_t_flips_the_rename_modal_between_modes() {
  // Issue #479 inverts a decision from #416, and the reasoning is kept rather
  // than deleted. The rename modal reuses `handle_create_key` (`tui/mod.rs`,
  // #290), so #416's free-form toggle was already *reachable* from it — and
  // was swallowed, because `draw_edit_worktree` did not render the `Name`
  // field and `submit_edit_worktree` did not read it, so toggling sent
  // keystrokes into an invisible buffer and made Enter submit the untouched
  // triple (Codex review on PR #474). #479 supplies the two missing halves, so
  // the verb does what its name says instead of being suppressed.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::state::create_form::Mode;
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("fix/#42-broken-thing".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));
  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit);
  assert_eq!(app.create_form.mode, Mode::Structured, "a branch the pattern reads");

  app.handle_create_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));

  assert_eq!(
    app.create_form.mode,
    Mode::Freeform,
    "the verb flips the rename modal now"
  );
  assert_eq!(
    app.create_form.field,
    Field::Name,
    "focus lands on the field the target mode renders"
  );
  assert_eq!(
    app.create_form.name, "fix/#42-broken-thing",
    "converting to free-form seeds the current branch verbatim, the only obvious answer"
  );

  // …and back, without losing what the structured side held: #416 keeps both
  // buffers side by side precisely so a round trip costs nothing.
  app.handle_create_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
  assert_eq!(app.create_form.mode, Mode::Structured);
  assert_eq!(
    app.create_form.desc, "broken-thing",
    "the structured buffer survives the round trip, and `t` never leaked into it as literal input"
  );
}

#[test]
fn the_mode_status_names_the_live_toggle_key_not_the_default() {
  // Same contract as the confirm countdown (#219 review): never advertise a
  // key that is no longer bound. The status hard-coded `ctrl-t`, so rebinding
  // `toggle_mode = ["Ctrl+y"]` left the overlay telling the user to press a
  // combination that does nothing (Codex review on PR #474).
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::keymap::KeyStroke;
  use gwm::tui::modal_keymap::ModalAction;
  let (_dir, mut app) = make_app();
  app
    .modal_keymap
    .apply_override(
      ModalAction::CreateToggleMode,
      vec![KeyStroke::new(KeyCode::Char('y'), KeyModifiers::CONTROL)],
    )
    .expect("Ctrl+y is bindable");
  app.enter_create();

  app.handle_create_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));

  assert!(
    app.status.to_lowercase().contains("ctrl-y") || app.status.to_lowercase().contains("ctrl+y"),
    "the status must name the live binding: {}",
    app.status
  );
  assert!(
    !app.status.to_lowercase().contains("ctrl-t"),
    "the status must not advertise the vacated default: {}",
    app.status
  );
}

#[test]
fn ctrl_t_still_flips_the_create_modal_into_free_form_mode() {
  // The counterpart: scoping the verb to `View::Create` must not disarm it
  // where it belongs.
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::state::create_form::Mode;
  let (_dir, mut app) = make_app();
  app.enter_create();
  assert_eq!(app.view, View::Create);

  app.handle_create_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));

  assert_eq!(app.create_form.mode, Mode::Freeform);
  assert_eq!(app.create_form.field, Field::Name);
}

#[test]
fn enter_edit_worktree_opens_an_unparseable_branch_in_free_form() {
  // Inverted by #479, and the reasoning is kept: a branch that is not
  // `<type>/#<issue>-<desc>` could not be decomposed into the form, so the
  // modal refused to open at all. It now has a mode that needs no
  // decomposition, so the same branch opens in it.
  use gwm::tui::state::create_form::Mode;
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("main".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::Edit, "free-form mode needs no decomposition");
  assert_eq!(app.create_form.mode, Mode::Freeform);
  assert_eq!(app.edit_original_branch.as_deref(), Some("main"));
}

#[test]
fn enter_edit_worktree_refuses_the_main_worktree() {
  // #479 removes an accidental protection and has to replace it deliberately.
  // The main checkout's branch is normally unparseable (`main`, `dev`), so the
  // parse failure used to turn the rename modal away from it — for the wrong
  // reason, but with the right result. Free-form mode parses nothing, so that
  // side effect is gone and the guard has to be the explicit one `enter_confirm_delete`
  // already has: renaming the main worktree means renaming the repo's default
  // branch, and `git worktree move` cannot move the main checkout anyway.
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("main".into());
  wt.is_main = true;
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::List, "the main worktree is not renamed from here");
  assert!(app.edit_original_branch.is_none());
  assert!(
    app.status.contains("main worktree"),
    "the refusal names what it is protecting: {}",
    app.status
  );
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
  // Codex review on PR #292: a branch whose type is not in branch_types must
  // refuse to open the rename modal rather than silently preselecting the
  // first configured type (which Enter would then rename the branch to).
  //
  // Issue #417 left this path alone on purpose. Compiling `{type}` into an
  // alternation of the configured types would have made `zzz/#7-thing` fail to
  // parse, collapsing this precise refusal into the generic "does not match
  // the pattern" one and taking the same branch away from `doctor` and
  // `commit-prefix`, which read it fine in the previous release.
  let (_dir, mut app) = make_app();
  let mut wt = worktree_fixture("foo");
  // `zzz` looks like a <type>/#<issue>-<desc> but is not a configured type.
  wt.branch = Some("zzz/#7-thing".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::List, "unconfigured type must not open the modal");
  assert!(app.edit_original_branch.is_none());
  assert!(
    app.status.contains("not configured") && app.status.contains("zzz"),
    "status must name the type and the reason: {}",
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
  let (argv, cwd, _) = app.exec_picker_resolve().expect("a valid profile resolves");
  assert_eq!(
    argv,
    vec!["cargo".to_string(), "build".to_string(), "--release".to_string()],
    "argv is the frozen command array verbatim (no shell)"
  );
  assert_eq!(cwd, expected_cwd, "cwd is the selected worktree's path");
}

#[cfg(unix)] // `[container]` is refused on Windows (host paths cannot be mirrored there).
#[test]
fn exec_picker_wraps_a_container_profile_the_same_way_the_cli_does() {
  // Issue #421: the same profile must not mean "in a container" on the CLI
  // and "on the host" in the TUI. `runtime` is explicit so the test never
  // depends on the runner having docker or podman installed.
  let (_repo, mut app) = app_with_gwm_toml(
    "[exec.profiles.ci]\ncommand = [\"cargo\", \"test\"]\n\n[exec.profiles.ci.container]\nimage = \"rust:1.90\"\nruntime = \"docker\"\n",
  );
  app.enter_exec_picker();
  let cwd = app.selected().unwrap().path.clone();
  let (argv, _, _) = app.exec_picker_resolve().expect("a container profile resolves");
  assert_eq!(argv[0], "docker", "argv[0] is the runtime the PTY overlay spawns");
  assert_eq!(argv[1], "run");
  // This overlay owns a real pty, so the container gets stdin and a terminal
  // (the fan-out on the CLI deliberately does not).
  assert!(
    argv.contains(&"-i".to_string()) && argv.contains(&"-t".to_string()),
    "the overlay allocates stdin + tty: {argv:?}"
  );
  // The selected worktree here is the main checkout, whose gitdir lives
  // inside it: one mount, its own path (the CLI dedupe, exercised through
  // the TUI path).
  let wt = cwd.components().collect::<std::path::PathBuf>().display().to_string();
  assert!(
    argv.windows(2).any(|w| w[0] == "-v" && w[1] == format!("{wt}:{wt}")),
    "the worktree path is mirrored: {argv:?}"
  );
  assert_eq!(
    argv.iter().filter(|t| *t == "-v").count(),
    1,
    "the gitdir already lives inside this worktree: {argv:?}"
  );
  assert_eq!(
    &argv[argv.len() - 2..],
    &["cargo".to_string(), "test".to_string()],
    "the profile's command is the container's CMD"
  );
}

#[cfg(unix)] // `[container]` is refused on Windows.
#[test]
fn exec_picker_hands_the_overlay_a_teardown_for_a_container_profile() {
  // The overlay kills the `docker` client on close, which leaves the
  // container running (the daemon owns it, `--rm` only fires on exit). So the
  // resolve hands the run loop the command that removes it by name.
  let (_repo, mut app) = app_with_gwm_toml(
    "[exec.profiles.ci]\ncommand = [\"cargo\", \"test\"]\n\n[exec.profiles.ci.container]\nimage = \"rust:1.90\"\nruntime = \"docker\"\n",
  );
  app.enter_exec_picker();
  let (argv, _, teardown) = app.exec_picker_resolve().expect("resolves");
  let teardown = teardown.expect("a containerised profile carries a teardown");
  let name = argv
    .windows(2)
    .find(|w| w[0] == "--name")
    .map(|w| w[1].clone())
    .expect("the run is named");
  assert_eq!(
    teardown,
    vec!["docker".to_string(), "rm".to_string(), "-f".to_string(), name],
    "the teardown removes the very container that was started"
  );
}

#[test]
fn exec_picker_leaves_a_hostless_profile_alone() {
  // The complement: no `[container]` ⇒ the argv is the command verbatim, the
  // pre-#421 behaviour.
  let (_repo, mut app) = app_with_gwm_toml("[exec.profiles.plain]\ncommand = [\"cargo\", \"test\"]\n");
  app.enter_exec_picker();
  let (argv, _, teardown) = app.exec_picker_resolve().expect("resolves");
  assert_eq!(argv, vec!["cargo".to_string(), "test".to_string()]);
  assert!(teardown.is_none(), "a host command has nothing to tear down");
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
  let (argv, cwd, _) = app
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
  let (argv, _cwd, _) = app.exec_picker_resolve().expect("resolves");
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
  let (argv, _cwd, _) = app.exec_picker_resolve().expect("resolves");
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
  // The delete-confirm modal is not in this class: since #484 it does capture
  // its targets, but by path rather than by row index, so a refresh landing
  // mid-countdown cannot retarget it and there is nothing to suspend for.
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
    let title = agents_pane_title(&km, false);
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

#[test]
fn open_menu_says_so_when_the_url_is_a_guess() {
  // A guessed SSH origin has no web host in it, so the locally built URL
  // may point at the SSH endpoint. It is still opened — refusing would
  // leave a permanently dead menu entry on an unreachable instance — but
  // the status bar stops it being a silent wrong tab (Codex review #458).
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  // A host that states its forge: `git.acme.internal` states none, and
  // `forge::resolve` now refuses to guess rather than send an
  // authenticated call there. The point here is the *guessed URL*
  // warning, which an scp-syntax remote still produces on github.com.
  repo.remote("origin", "git@github.com:team/proj.git").unwrap();
  app.enter_open_menu();

  let url = app.open_menu_pick(LinkTarget::Issue).unwrap();

  assert!(url.contains("/issues/42"), "{url}");
  assert!(app.status.contains("guessed"), "status was: {}", app.status);
}

#[test]
fn open_menu_stays_quiet_on_an_authoritative_origin() {
  // The negative control: an https origin names its web host, so the
  // built URL is not a guess and must not be flagged as one.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  app.enter_open_menu();

  app.open_menu_pick(LinkTarget::Issue).unwrap();

  assert!(!app.status.contains("guessed"), "status was: {}", app.status);
}

#[test]
fn open_menu_keeps_the_fetched_url_it_is_about_to_use() {
  // `enter_open_menu` re-read the link through the invalidating
  // `refresh_link`, wiping the very cache `cached_issue_url` reads — so
  // the server-reported `web_url` was never used and the menu always
  // built a URL locally (Codex review #458). Selection changes must
  // still invalidate; only this path is exempt, and it is safe because
  // the caches are keyed by number.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  // A host that states its forge: `git.acme.internal` states none, and
  // `forge::resolve` now refuses to guess rather than send an
  // authenticated call there. The point here is the *guessed URL*
  // warning, which an scp-syntax remote still produces on github.com.
  repo.remote("origin", "git@github.com:team/proj.git").unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(gwm::forge::IssueStatus {
    number: 42,
    title: "t".into(),
    state: gwm::forge::IssueState::Open,
    url: "https://web.acme.internal:8443/team/proj/-/issues/42".into(),
    labels: vec![],
    updated_at: String::new(),
    detail: Default::default(),
  }));

  app.enter_open_menu();
  let url = app.open_menu_pick(LinkTarget::Issue).unwrap();

  assert_eq!(url, "https://web.acme.internal:8443/team/proj/-/issues/42");
  assert!(
    !app.status.contains("guessed"),
    "a server-reported URL is not a guess: {}",
    app.status
  );
}

#[test]
fn open_menu_drops_a_cached_url_from_the_previous_origin() {
  // `reread_link` preserves the fetch caches so the open menu can use
  // the server-reported URL. Those caches are keyed by number alone, so
  // an origin move must still clear them: otherwise issue #42 on the new
  // instance opens the old tenant's #42 (Codex review #458).
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(gwm::forge::IssueStatus {
    number: 42,
    title: "t".into(),
    state: gwm::forge::IssueState::Open,
    url: "https://github.com/acme/widgets/issues/42".into(),
    labels: vec![],
    updated_at: String::new(),
    detail: Default::default(),
  }));

  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://gitlab.com/acme/widgets.git").unwrap();
  app.enter_open_menu();
  let url = app.open_menu_pick(LinkTarget::Issue).unwrap();

  assert!(
    !url.contains("github.com"),
    "the old tenant's cached URL survived the move: {url}"
  );
}

#[test]
fn enter_edit_worktree_opens_on_a_pattern_that_carries_no_issue() {
  // This test used to assert the opposite, and the reversal *is* issue #418.
  //
  // The guard it pinned (Codex review on PR #476) refused to open the rename
  // modal when a segment came back empty, because "the form cannot be
  // submitted without it": `BranchSpec::new_with_types` rejects an empty issue,
  // and inventing one would not have helped since `{type}/{desc}` has no
  // `{issue}` to write it into. Both halves were true of a form hardcoded to
  // the canonical triple.
  //
  // Token-driven, neither holds. The form presents no Issue field on this
  // pattern, `new_with_required` does not validate a segment the patterns
  // discard, and the rename is perfectly submittable — so refusing took the
  // modal away from a repo where it now works.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "{type}/{desc}".into();
  app.config.worktree.path_pattern = "{type}-{desc}".into();
  app.apply_create_form_fields();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/my-desc".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::Edit, "the form is submittable, so it must open");
  assert_eq!(app.create_form.fields(), [Field::Type, Field::Desc]);
  assert_eq!(app.create_form.desc, "my-desc");
  assert!(
    app.create_form.issue.is_empty(),
    "nothing to recover, and nothing asks for it"
  );
  assert!(
    app.edit_target().is_ok(),
    "and it composes a target: {:?}",
    app.edit_target()
  );
}

#[test]
fn enter_edit_worktree_opens_when_the_pattern_freezes_a_segment() {
  // The other side of the same guard: `feat/#{issue}-{desc}` hardcodes the
  // type, and #417 recovers it as a constant, so every segment has a value and
  // the rename is perfectly submittable. Refusing here would take the modal
  // away from a repo where it works.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#42-my-desc".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::Edit, "a fully-supplied triple must open the form");
  assert_eq!(app.create_form.issue, "42");
  assert_eq!(app.create_form.desc, "my-desc");
}

#[test]
fn submit_edit_worktree_refuses_to_change_a_segment_no_pattern_writes() {
  // Codex review on PR #476, fourth pass, narrowed by Kylian afterwards. The
  // refusal is about there being **nowhere to put the new value**, so it asks
  // both patterns, not just `branch_pattern`: a segment the branch freezes but
  // the *path* still writes has a real destination, and changing it genuinely
  // renames the directory (see the test below).
  //
  // Here neither writes the issue: `{type}-1-{desc}` on both sides. Editing
  // `1` to `2` would produce the same branch and the same directory, so the
  // submit would close the form having changed nothing at all. Saying no is
  // better than a silent no-op.
  //
  // The refusal is scoped to a frozen segment the user actually changed:
  // `feat/#{issue}-{desc}` freezes the type and its rename worked in 1.5.0, so
  // editing the issue or the description there must stay possible.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "{type}-1-{desc}".into();
  app.config.worktree.path_pattern = "{type}-1-{desc}".into();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat-1-my-desc".into());
  wt.path = app.workdir.join("feat-1-my-desc");
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit, "the frozen issue is supplied, so the form opens");
  assert_eq!(app.create_form.issue, "1");

  app.create_form.issue = "2".into();
  app
    .submit_edit_worktree()
    .expect("the refusal is a form failure, not an error");

  assert_eq!(app.view, View::Edit, "the form stays open on a refusal");
  let failure = app
    .edit_failure
    .clone()
    .expect("the refusal must be reported in the form");
  assert!(
    failure.contains("{issue}"),
    "the message must name the placeholder the pattern cannot write: {}",
    failure
  );
  // Deliberately value-free: `LoaderWidget` renders one unwrapped line, so a
  // message whose length follows user input clips at an arbitrary point. The
  // value is on screen anyway, in the modal's `From :` row.
  assert!(
    !failure.contains('1'),
    "the frozen value belongs on screen, not in a message of variable length: {}",
    failure
  );
}

#[test]
fn submit_edit_worktree_still_changes_a_segment_the_pattern_writes() {
  // The other side of the same guard, and the reason it is scoped to the
  // segment rather than to the pattern: `feat/#{issue}-{desc}` freezes the
  // type, and renaming the issue or the description under it worked before
  // #417. It has to keep working.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#42-my-desc".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit);

  app.create_form.desc = "other-desc".into();
  app.submit_edit_worktree().expect("submits");

  assert_eq!(
    app.edit_failure, None,
    "editing a segment the pattern writes is not a frozen-segment change"
  );
}

#[test]
fn submit_edit_worktree_lets_the_directory_carry_what_the_branch_cannot() {
  // Kylian's call, validating by hand on #476. `feat/#{issue}-{desc}` freezes
  // the type, so the branch will say `feat` whatever the form holds — but
  // `path_pattern` writes `{type}`, so the directory is where the type of this
  // worktree actually lives. Refusing to edit it meant a worktree created as
  // `fix` could never become `docs`, under a config that puts the type in the
  // path on purpose.
  //
  // What made the refusal look right was a preview that lied: it showed
  // `docs/#42-…` for a submit that writes `feat/#42-…`. With the preview
  // expanding the real patterns, the modal states plainly that the branch is
  // unchanged and the directory moves, so there is nothing left to protect the
  // user from. `rename_worktree` already handles this exact shape as a
  // path-only edit: same branch, so it skips every ref mutation, local and
  // remote alike.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();
  app.config.worktree.path_pattern = "{type}-{issue}-{desc}".into();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#42-login".into());
  wt.path = app.workdir.join("fix-42-login");
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit);
  assert_eq!(
    app.create_form.type_index,
    app
      .branch_types
      .iter()
      .position(|t| t.name == "fix")
      .expect("`fix` is a built-in branch type"),
    "issue #478: the type comes from the directory, the only place it exists"
  );

  app.create_form.type_index = app
    .branch_types
    .iter()
    .position(|t| t.name == "docs")
    .expect("`docs` is a built-in branch type");
  app.submit_edit_worktree().expect("submits");

  assert_eq!(
    app.edit_failure, None,
    "`path_pattern` writes {{type}}, so there is somewhere to put `docs`: {:?}",
    app.edit_failure
  );
}

#[test]
fn the_rename_form_keeps_what_only_the_worktree_directory_carries() {
  // Issue #478. `branch_pattern` freezes the type, `path_pattern` still writes
  // it, so `gwm create fix 42 x` produces the branch `feat/#42-x` and the
  // directory `fix-42-x` — and `fix` exists nowhere else. Rebuilding the
  // triple from the branch alone read the type as `feat`, so renaming the
  // description also moved the directory to `feat-42-…` and dropped what the
  // worktree was created with.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#42-x".into());
  wt.path = std::path::PathBuf::from("/tmp/gwm-test/fix-42-x");
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::Edit, "the form must open: {}", app.status);
  assert_eq!(
    app.branch_types[app.create_form.type_index].name, "fix",
    "the form must show the type the worktree was created with, not the frozen literal"
  );
  assert_eq!(app.create_form.issue, "42");
  assert_eq!(app.create_form.desc, "x");

  // Renaming the description leaves the type where the directory had it, so
  // the rename renames only what was edited.
  app.create_form.desc = "other".into();
  app.submit_edit_worktree().expect("submits");
  assert_eq!(
    app.edit_failure, None,
    "an untouched type read from the directory is not a change: {:?}",
    app.edit_failure
  );
}

#[test]
fn submit_edit_worktree_counts_worktree_base_as_a_destination() {
  // Codex review on PR #476, tenth pass. `[worktree].base` is expanded with the
  // triple too — `BranchSpec::worktree_path` feeds it `{type}` / `{issue}` /
  // `{desc}` before joining the dirname — so a `base` of `.../{type}` sorts
  // worktrees into per-type directories and changing the type moves the
  // worktree between them. That is a real rename, so the guard has to look
  // there as well: asking only `branch_pattern` and `path_pattern` refused an
  // edit that had a perfectly good destination one level up the path.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();
  app.config.worktree.path_pattern = "{issue}-{desc}".into();
  app.config.worktree.base = format!("{}/{{type}}", app.workdir.display());
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#42-login".into());
  wt.path = app.workdir.join("fix").join("42-login");
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit, "the form must open: {}", app.status);

  app.create_form.type_index = app
    .branch_types
    .iter()
    .position(|t| t.name == "docs")
    .expect("`docs` is a built-in branch type");
  app.submit_edit_worktree().expect("submits");

  assert_eq!(
    app.edit_failure, None,
    "`base` writes {{type}}, so the worktree moves from `fix/` to `docs/`: {:?}",
    app.edit_failure
  );
}

#[test]
fn the_rename_form_still_refuses_to_change_what_neither_pattern_writes() {
  // The other side of #478: reading the type from the directory does not by
  // itself make it editable. Here the *path* freezes it too — `fix-` is a
  // literal, not `{type}` — so the type is a constant on both sides and there
  // is nowhere to write `docs`. Contrast with
  // `submit_edit_worktree_lets_the_directory_carry_what_the_branch_cannot`,
  // where `path_pattern` writes `{type}` and the edit goes through.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "feat/#{issue}-{desc}".into();
  app.config.worktree.path_pattern = "fix-{issue}-{desc}".into();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#42-x".into());
  wt.path = std::path::PathBuf::from("/tmp/gwm-test/fix-42-x");
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit);

  app.create_form.type_index = app
    .branch_types
    .iter()
    .position(|t| t.name == "docs")
    .expect("docs is configured");
  app
    .submit_edit_worktree()
    .expect("the refusal is a form failure, not an error");
  assert!(
    app.edit_failure.as_deref().is_some_and(|e| e.contains("{type}")),
    "changing a type the branch pattern cannot write must be refused: {:?}",
    app.edit_failure
  );
}

#[test]
fn activating_a_workspace_repo_names_it_by_its_directory_not_its_display_label() {
  // Issue #480. `workspace::discover` suffixes a repo whose basename collides
  // with a sibling, so the second `api` is displayed as `api-2` (#304), and
  // activating it used to put that label straight into `App::repo_name` — the
  // field every formatter call expands `{repo}` with. The parser side
  // (`BranchParser::for_repo`, `github::read_link`, `lifecycle`) reads the
  // *directory* basename, and so does every `gwm create` from the CLI, so a
  // `{repo}` pattern wrote `api-2/…` that nothing could read back.
  //
  // The label is a property of the workspace's current membership, not of the
  // repo: move the sibling out and this repo becomes `api` again while its
  // branches still say `api-2`. A name persisted in git cannot depend on what
  // else sits next to it on disk, so naming uses the basename and the label
  // stays what it was built for, which is display.
  let (dir, mut app) = make_app();
  let sibling = dir.path().join("api");
  std::fs::create_dir_all(&sibling).unwrap();
  git2::Repository::init(&sibling).unwrap();

  app.workspace = Some(gwm::tui::WorkspaceState {
    root: dir.path().to_path_buf(),
    repos: vec![
      gwm::tui::RepoMeta {
        name: app.repo_name.clone(),
        workdir: app.workdir.clone(),
        config: app.config.clone(),
      },
      gwm::tui::RepoMeta {
        // The display label, deliberately different from the basename `api`.
        name: "api-2".into(),
        workdir: sibling.clone(),
        config: app.config.clone(),
      },
    ],
    row_repo: vec![0, 1],
    active: 0,
  });
  let mut wt = worktree_fixture("other");
  wt.branch = Some("api/feat/#42-x".into());
  app.worktrees = vec![worktree_fixture("own"), wt];
  app.list_state.select(Some(1));

  app.sync_active_repo();

  assert_eq!(
    app.repo_name, "api",
    "`{{repo}}` is expanded with the directory basename, the same name the CLI and the parser use"
  );
  assert_eq!(
    app.display_repo_name, "api-2",
    "the disambiguated label survives, for the header and the REPO column"
  );
}

#[test]
fn the_rename_form_reads_a_branch_with_the_same_repo_name_it_writes_one_with() {
  // Codex review on PR #476, eighth pass. In a workspace, two repos whose
  // directories share a basename are disambiguated for display — the second
  // becomes `api-2` (#304) — and `App::repo_name` holds that name. Every
  // formatter call in the rename flow already expands `{repo}` with it, so the
  // parser has to be compiled with it too; deriving it from the *real*
  // basename instead meant a `{repo}` pattern read a branch this repo could
  // never have written, and the form refused to open on a worktree it owns.
  //
  // Which name is right for `{repo}` is a separate question, and an older one:
  // `spec.branch_name(..., &self.repo_name)` predates #417. What matters here
  // is that one name is used, since parser and formatter agreeing is the whole
  // point of the issue.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "{repo}/{type}/#{issue}-{desc}".into();
  app.repo_name = "api-2".into();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("api-2/feat/#42-my-desc".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(
    app.view,
    View::Edit,
    "the form must open on a branch this repo writes: {}",
    app.status
  );
  assert_eq!(app.create_form.issue, "42");
  assert_eq!(app.create_form.desc, "my-desc");
}

#[test]
fn submit_edit_worktree_compares_a_frozen_segment_before_kebab_normalises_it() {
  // Codex review on PR #476, seventh pass. A frozen description does not have
  // to be canonical: `DESC_RE` accepts `fixed-`, and since the previous pass
  // the recovery reads it whole rather than trimming the trailing dash — which
  // is what 1.5.0 did. But the form's value goes through
  // `BranchSpec::new_with_types`, and `kebab` strips that dash, so comparing
  // the *spec* against the constant found a difference on every submit and the
  // form could never be submitted at all.
  //
  // The guard is about what the user typed, so it compares what the user
  // typed.
  //
  // Both patterns freeze the description, so the guard is the one that fires
  // here: with only `branch_pattern` freezing it, `path_pattern` would give the
  // new value a destination and the edit would be allowed.
  let (_dir, mut app) = make_app();
  app.config.worktree.branch_pattern = "{type}/#{issue}-fixed-".into();
  app.config.worktree.path_pattern = "{type}-{issue}-fixed-".into();
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/#42-fixed-".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();
  assert_eq!(
    app.view,
    View::Edit,
    "the frozen description is supplied, so the form opens"
  );
  assert_eq!(
    app.create_form.desc, "fixed-",
    "the constant is read whole, trailing dash and all"
  );

  // Change only the issue — the frozen description is untouched.
  app.create_form.issue = "43".into();
  app.submit_edit_worktree().expect("submits");
  assert_eq!(
    app.edit_failure, None,
    "an untouched frozen description must not read as a change"
  );

  // …and the guard still fires when the frozen description really is edited.
  app.enter_edit_worktree();
  app.create_form.desc = "something-else".into();
  app
    .submit_edit_worktree()
    .expect("the refusal is a form failure, not an error");
  assert!(
    app.edit_failure.as_deref().is_some_and(|e| e.contains("{desc}")),
    "editing the frozen description must still be refused: {:?}",
    app.edit_failure
  );
}

// --- token-driven create form (issue #418) --------------------------------

/// Build an `App` on a throwaway repo whose `.gwm.toml` carries the given
/// worktree patterns. `base` is included because it feeds the triple too.
fn app_with_patterns(branch: &str, path: &str, base: &str) -> (tempfile::TempDir, App) {
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    format!(
      "[worktree]\nbase = \"{}\"\nbranch_pattern = \"{}\"\npath_pattern = \"{}\"\n",
      base, branch, path
    ),
  )
  .unwrap();
  let app = App::new_at_layered(Some(dir.path()), None).expect("app builds on the fixture repo");
  (dir, app)
}

/// The form must present exactly the fields the repo's patterns ask for.
#[test]
fn the_create_form_presents_only_the_fields_the_repo_s_patterns_ask_for() {
  let (_d, app) = app_with_patterns("{type}/{desc}", "{type}-{desc}", "{repo_parent}/wt");
  assert_eq!(
    app.create_form.fields(),
    [Field::Type, Field::Desc],
    "a pattern that writes no issue number must not present an Issue field"
  );

  // The union includes `base`: a value dropped there names a real directory.
  let (_d2, app2) = app_with_patterns("{type}/{desc}", "{type}-{desc}", "{repo_parent}/wt/{issue}");
  assert_eq!(
    app2.create_form.fields(),
    [Field::Type, Field::Desc, Field::Issue],
    "base carries {{issue}}, so the form still has to collect it"
  );
}

/// The end-to-end point of #418. `BranchSpec::validate_against` refuses an
/// empty issue, so before this the form on a `{type}/{desc}` repo demanded a
/// number and then expanded patterns with nowhere to put it: mandatory *and*
/// discarded, which left the TUI create path unusable on that convention.
#[test]
fn a_pattern_without_an_issue_token_can_be_submitted_without_an_issue_number() {
  let (_d, mut app) = app_with_patterns("{type}/{desc}", "{type}-{desc}", "{repo_parent}/wt");
  app.enter_create();
  for c in "thing".chars() {
    app.create_form.push_char(c);
  }
  assert!(
    app.create_form.issue.is_empty(),
    "nothing was typed into a hidden field"
  );

  let (branch, dirname) = app
    .edit_target()
    .expect("the form must compose a target without an issue number it cannot write");
  assert_eq!(branch, "feat/thing");
  assert_eq!(dirname, "feat-thing");
}

/// A segment the patterns *do* ask for is still required — relaxing validation
/// must be scoped to what the pattern drops, not to validation in general.
#[test]
fn a_segment_the_pattern_does_ask_for_is_still_required() {
  let (_d, mut app) = app_with_patterns("{type}/#{issue}-{desc}", "{type}-{issue}-{desc}", "{repo_parent}/wt");
  app.enter_create();
  app.create_form.field = Field::Desc;
  for c in "thing".chars() {
    app.create_form.push_char(c);
  }
  assert!(
    app.edit_target().is_err(),
    "the pattern writes an issue number, so an empty one must still be refused"
  );
}

/// `enter_create` opened on `Field::Issue` literally. On a pattern without one
/// that focuses an input the renderer never draws, so the first keypress goes
/// nowhere.
#[test]
fn entering_the_create_form_focuses_a_field_the_pattern_presents() {
  let (_d, mut app) = app_with_patterns("{type}/{desc}", "{type}-{desc}", "{repo_parent}/wt");
  app.enter_create();
  assert_eq!(app.create_form.field, Field::Desc);

  // And the canonical pattern keeps #217's behaviour: skip the cycle-only Type.
  let (_d2, mut app2) = app_with_patterns("{type}/#{issue}-{desc}", "{type}-{issue}-{desc}", "{repo_parent}/wt");
  app2.enter_create();
  assert_eq!(app2.create_form.field, Field::Issue);
}

/// Enter submitted only from `Field::Desc` (or `Name`), because those were the
/// last fields of the two hardcoded modes. Making `{desc}` optional turned that
/// into a form that **cannot be submitted at all**: on `{type}/#{issue}` Enter
/// just rotated forever. A bug this PR would have introduced, not a pre-existing
/// one, and no state or render test can see it because the gate lives in the key
/// handler.
#[test]
fn enter_submits_from_the_last_field_whatever_the_pattern_calls_it() {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  use gwm::tui::CreateKey;

  let (_d, mut app) = app_with_patterns("{type}/#{issue}", "{type}-{issue}", "{repo_parent}/wt");
  app.enter_create();
  assert_eq!(app.create_form.fields(), [Field::Type, Field::Issue]);

  // Focus the last field the pattern presents, then submit.
  app.create_form.field = app.create_form.last_field();
  assert!(
    matches!(
      app.handle_create_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
      CreateKey::Submit
    ),
    "Enter on the pattern's last field must submit, not rotate"
  );

  // And from a non-final field it still advances rather than submitting.
  app.create_form.field = Field::Type;
  assert!(matches!(
    app.handle_create_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    CreateKey::Handled
  ));
  assert_eq!(app.create_form.field, Field::Issue, "it advanced instead");
}

/// The create path composes its own `BranchSpec`, separately from the rename
/// path. Relaxing validation in `worktree_name_from_form` alone left the actual
/// defect standing where the issue is about: pressing Enter on a `{type}/{desc}`
/// repo still failed with `invalid issue number ''`.
#[test]
fn submitting_the_create_form_does_not_demand_a_segment_the_patterns_discard() {
  let _env = env_lock().lock().unwrap_or_else(|e| e.into_inner());
  let (_d, mut app) = app_with_patterns("{type}/{desc}", "{type}-{desc}", "{repo_parent}/wt");
  app.enter_create();
  for c in "thing".chars() {
    app.create_form.push_char(c);
  }

  // Composition happens before the trust gate, so a validation failure surfaces
  // as an `Err` here regardless of what the gate then decides.
  let out = app.submit_create();
  assert!(
    out.is_ok(),
    "the form must compose without an issue number it cannot write: {:?}",
    out.err().map(|e| e.to_string())
  );
  assert!(
    !app.status.contains("invalid issue"),
    "and must not complain about one either: {}",
    app.status
  );
}

/// Codex review on PR #492. `enter_create` has its own status string, separate
/// from the one the free-form toggle writes, and only the second was moved off
/// `desc`. So the form opened telling the user to press enter on a field the
/// pattern does not present, while Enter actually submitted from another.
///
/// Same class as the field set itself, applied half-way: when an invariant is
/// written down, every consumer of the old hardcoded value has to move at once.
#[test]
fn the_opening_instruction_names_the_field_enter_actually_submits_from() {
  let (_d, mut app) = app_with_patterns("{type}/#{issue}", "{type}-{issue}", "{repo_parent}/wt");
  app.enter_create();
  assert!(
    app.status.contains("enter on issue"),
    "the pattern's last field is Issue, not Desc: {}",
    app.status
  );
  assert!(!app.status.contains("enter on desc"), "and Desc does not exist here");

  let (_d2, mut app2) = app_with_patterns("{type}/#{issue}-{desc}", "{type}-{issue}-{desc}", "{repo_parent}/wt");
  app2.enter_create();
  assert!(
    app2.status.contains("enter on desc"),
    "the canonical pattern is unchanged: {}",
    app2.status
  );
}

/// Codex review on PR #492, two passes, and the second reversed the first.
///
/// `worktree_spec` reads the branch and the directory name and never parses
/// `base`, so a segment carried only by `base` comes back empty. My first fix
/// treated that as "nothing to preserve" and defaulted the selector to index 0.
/// That is wrong in the most expensive way available: the value is not absent,
/// it is **on disk**. Under `base = ".../wt/{type}"` a worktree at
/// `.../wt/fix/my-desc` would open showing `feat`, and submitting without
/// touching the type would move the worktree to `.../wt/feat/`.
///
/// The rule is the conjunction, not either half: refuse when a segment some
/// pattern *writes* is one the parse did *not* recover.
#[test]
fn the_rename_form_refuses_a_segment_it_cannot_read_back_from_the_name() {
  let (_d, mut app) = app_with_patterns("{desc}", "{desc}", "{repo_parent}/wt/{type}");
  assert!(
    app.create_form.fields().contains(&Field::Type),
    "base carries {{type}}, so the form would present a selector for it"
  );
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("my-desc".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::List, "opening would overwrite the on-disk type");
  assert!(app.edit_original_branch.is_none());
  assert!(
    app.status.contains("{type}") && app.status.contains("worktree.base"),
    "the status must name the segment and where to look: {}",
    app.status
  );
}

/// The other half of that conjunction, and the #418 win it must not undo: a
/// segment **no** pattern carries is not something the form shows or asks for,
/// so its absence from the parse is not a reason to refuse.
#[test]
fn a_segment_no_pattern_carries_is_not_a_reason_to_refuse() {
  let (_d, mut app) = app_with_patterns("{type}/{desc}", "{type}-{desc}", "{repo_parent}/wt");
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("feat/my-desc".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(app.view, View::Edit, "status was: {}", app.status);
  assert!(app.create_form.issue.is_empty());
}

/// The other side of the same guard: a parsed type that is real but not
/// configured must still be refused, which is what #292 was actually about.
#[test]
fn a_parsed_but_unconfigured_type_is_still_refused() {
  let (_d, mut app) = app_with_patterns("{type}/#{issue}-{desc}", "{type}-{issue}-{desc}", "{repo_parent}/wt");
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("zzz/#7-thing".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();

  assert_eq!(
    app.view,
    View::List,
    "an unconfigured type must not be silently rewritten"
  );
  assert!(app.status.contains("zzz"), "and must be named: {}", app.status);
}

/// Codex review on PR #492, sixth pass, P1, and it disproves a claim I wrote
/// into the code two commits earlier: that `refuse_unwritable_segment_change`
/// had become unreachable in structured mode.
///
/// It is reachable, and it blocks every structured rename on a pattern set that
/// omits `{type}`. The form hides the Type field but `type_index` still points
/// at the first configured type, so the guard compares `feat` against the empty
/// type recovered from the branch, calls that a change to an unwritten segment,
/// and refuses. Nothing the user can do clears it, because the field is not on
/// screen to correct.
///
/// A hidden field has no value to defend: the guard must skip the segments the
/// form does not present rather than compare their fallbacks.
#[test]
fn a_hidden_segment_cannot_block_the_rename_it_is_not_part_of() {
  let (_d, mut app) = app_with_patterns("#{issue}-{desc}", "{issue}-{desc}", "{repo_parent}/wt");
  assert!(
    !app.create_form.fields().contains(&Field::Type),
    "no pattern carries {{type}}, so the form must not present it"
  );
  let mut wt = worktree_fixture("foo");
  wt.branch = Some("#42-my-desc".into());
  app.worktrees = vec![wt];
  app.list_state.select(Some(0));

  app.enter_edit_worktree();
  assert_eq!(app.view, View::Edit, "status was: {}", app.status);

  // Edit only what the pattern writes.
  app.create_form.field = Field::Desc;
  for _ in 0.."my-desc".len() {
    app.create_form.pop_char();
  }
  for c in "renamed".chars() {
    app.create_form.push_char(c);
  }

  let out = app.submit_edit_worktree();
  assert!(
    !app.edit_failure.as_deref().unwrap_or("").contains("{type}"),
    "the hidden type must not be read as a change: {:?}",
    app.edit_failure
  );
  assert!(
    !app.status.contains("no {type} to write"),
    "nor block the submit: {}",
    app.status
  );
  let _ = out;
}

// --- #484: multi-row selection + bulk delete -----------------------------
//
// The cursor row (`list_state`) and the marked set are two different things:
// `d` acts on the marked set when it is non-empty, on the cursor row
// otherwise. Every assertion below is on the pure state, no ratatui.

fn app_with_rows(names: &[&str]) -> (tempfile::TempDir, App) {
  let (dir, mut app) = make_app();
  app.worktrees = names.iter().map(|n| worktree_fixture(n)).collect();
  app.list_state.select(Some(0));
  (dir, app)
}

#[test]
fn toggle_select_marks_and_unmarks_the_cursor_row() {
  let (_d, mut app) = app_with_rows(&["alpha", "beta"]);
  app.toggle_select();
  assert_eq!(app.marked_count(), 1, "status was: {}", app.status);
  assert!(app.is_marked(&PathBuf::from("/tmp/gwm-test/alpha")));

  app.toggle_select();
  assert_eq!(app.marked_count(), 0, "a second press must unmark the row");
}

#[test]
fn the_main_worktree_can_never_be_marked() {
  // `d` refuses the main worktree, so marking it would build a batch with a
  // target that can only fail.
  let (_d, mut app) = app_with_rows(&["main"]);
  app.worktrees[0].is_main = true;
  app.toggle_select();
  assert_eq!(app.marked_count(), 0, "status was: {}", app.status);
  assert!(app.status.contains("main"), "and it must say why: {}", app.status);
}

#[test]
fn delete_targets_are_the_marked_rows_in_list_order() {
  let (_d, mut app) = app_with_rows(&["alpha", "beta", "gamma"]);
  // Mark gamma first, then alpha: the batch must still run in list order.
  app.list_state.select(Some(2));
  app.toggle_select();
  app.list_state.select(Some(0));
  app.toggle_select();

  let ids: Vec<String> = app.delete_targets().into_iter().map(|t| t.id).collect();
  assert_eq!(ids, vec!["alpha".to_string(), "gamma".to_string()]);
}

#[test]
fn delete_targets_fall_back_to_the_cursor_row_when_nothing_is_marked() {
  let (_d, mut app) = app_with_rows(&["alpha", "beta"]);
  app.list_state.select(Some(1));
  let ids: Vec<String> = app.delete_targets().into_iter().map(|t| t.id).collect();
  assert_eq!(ids, vec!["beta".to_string()]);
}

#[test]
fn a_mark_on_a_row_that_no_longer_exists_targets_nothing() {
  // Marks are keyed by path; a row that vanished must not survive as a
  // phantom target, and must NOT silently fall back to the cursor row
  // either — that would delete a worktree the user never marked.
  let (_d, mut app) = app_with_rows(&["alpha", "beta"]);
  app.toggle_select();
  app.worktrees.remove(0);
  app.list_state.select(Some(0));
  assert!(app.delete_targets().is_empty(), "a stale mark must target nothing");
}

#[test]
fn a_refresh_prunes_the_marks_whose_row_is_gone() {
  // The background auto-refresh must not clear a selection mid-build, but it
  // has to drop the rows that no longer exist. Refreshing against the real
  // repo replaces the synthetic rows, so every mark below is stale.
  let (_d, mut app) = app_with_rows(&["alpha", "beta"]);
  app.toggle_select();
  assert_eq!(app.marked_count(), 1);
  app.refresh().unwrap();
  assert_eq!(app.marked_count(), 0, "a mark whose row is gone must be pruned");
}

#[test]
fn opening_the_filter_clears_the_marks() {
  let (_d, mut app) = app_with_rows(&["alpha", "beta"]);
  app.toggle_select();
  app.enter_filter();
  assert_eq!(app.marked_count(), 0);
}

#[test]
fn the_manual_refresh_clears_the_marks() {
  let (_d, mut app) = app_with_rows(&["alpha", "beta"]);
  app.toggle_select();
  app.request_refresh();
  assert_eq!(app.marked_count(), 0);
}

#[test]
fn entering_the_confirm_overlay_snapshots_the_batch() {
  // The countdown can run while an auto-refresh lands and reorders the list,
  // so the targets are resolved once, at open time.
  let (_d, mut app) = app_with_rows(&["alpha", "beta", "gamma"]);
  app.toggle_select();
  app.list_state.select(Some(1));
  app.toggle_select();

  app.enter_confirm_delete();
  assert_eq!(app.view, View::Confirm, "status was: {}", app.status);
  assert_eq!(app.pending_delete().len(), 2);

  app.worktrees.clear();
  assert_eq!(app.pending_delete().len(), 2, "the snapshot must not track the list");
}

#[test]
fn the_batch_status_names_the_failures() {
  use gwm::tui::{DeleteBatchOutcome, DeleteFailure};
  let single = DeleteBatchOutcome {
    removed: vec![("alpha".into(), "/tmp/alpha".into())],
    failed: vec![],
    warnings: vec![],
  };
  assert_eq!(single.status_line(), "removed alpha (/tmp/alpha)");

  let batch = DeleteBatchOutcome {
    removed: vec![
      ("alpha".into(), "/tmp/alpha".into()),
      ("beta".into(), "/tmp/beta".into()),
    ],
    failed: vec![DeleteFailure {
      id: "gamma".into(),
      path: "/tmp/gamma".into(),
      error: "locked".into(),
    }],
    warnings: vec![],
  };
  assert_eq!(batch.status_line(), "removed 2 of 3 worktrees; failed: gamma (locked)");

  let all_ok = DeleteBatchOutcome {
    removed: vec![
      ("alpha".into(), "/tmp/alpha".into()),
      ("beta".into(), "/tmp/beta".into()),
    ],
    failed: vec![],
    warnings: vec![],
  };
  assert_eq!(all_ok.status_line(), "removed 2 worktrees");
}

#[test]
fn a_warning_rides_the_status_line_without_becoming_a_failure() {
  // Issue #521: a `post_remove` hook that aborts, or an undo-journal entry
  // that could not be written, happens around a removal that DID happen.
  // Counting it as a failure would report the opposite of what is on disk,
  // and `failure_banner` is what keeps the confirm overlay open, so a
  // warning must not reach it.
  use gwm::tui::{DeleteBatchOutcome, DeleteFailure};

  let with_warning = DeleteBatchOutcome {
    removed: vec![("alpha".into(), "/tmp/alpha".into())],
    failed: vec![],
    warnings: vec!["hook post_remove 'cleanup' failed: exited with 1".into()],
  };
  assert_eq!(
    with_warning.status_line(),
    "removed alpha (/tmp/alpha); hook post_remove 'cleanup' failed: exited with 1"
  );
  assert_eq!(
    with_warning.failure_banner(),
    None,
    "a warning must not hold the confirm overlay open"
  );

  // A real failure still owns the banner, and warnings ride along.
  let both = DeleteBatchOutcome {
    removed: vec![("alpha".into(), "/tmp/alpha".into())],
    failed: vec![DeleteFailure {
      id: "beta".into(),
      path: "/tmp/beta".into(),
      error: "locked".into(),
    }],
    warnings: vec!["journal unwritable".into()],
  };
  assert_eq!(
    both.status_line(),
    "removed 1 of 2 worktrees; failed: beta (locked); journal unwritable"
  );
  assert_eq!(both.failure_banner(), Some("/tmp/beta: locked".into()));
}

#[test]
fn a_partial_batch_narrows_the_confirm_to_the_failures_never_to_the_cursor() {
  // Codex review on PR #520 (P1). `worktree::remove` prunes the admin entry
  // BEFORE deleting the directory (#98), so a removal that fails on the
  // filesystem still drops its row from `repo.worktrees()`. The refresh in
  // the drain then prunes its mark, and a batch RECOMPUTED from an empty
  // mark set falls back to the cursor row: a second confirm would delete a
  // worktree the user never marked. The batch may only ever narrow.
  use gwm::tui::state::async_task::{DeleteBatchOutcome, DeleteFailure, TaskKind, TaskMsg};

  let (dir, repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  let doomed = base.path().join("wt-doomed");
  let bystander = base.path().join("wt-bystander");
  gwm::worktree::add(&repo, "wt-doomed", &doomed, "feat/#484-doomed", false).unwrap();
  gwm::worktree::add(&repo, "wt-bystander", &bystander, "feat/#484-bystander", false).unwrap();

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let doomed_row = app
    .worktrees
    .iter()
    .position(|w| w.name == "wt-doomed")
    .expect("the doomed worktree is listed");
  app.list_state.select(Some(doomed_row));
  app.toggle_select();
  app.enter_confirm_delete();
  assert_eq!(app.pending_delete().len(), 1, "status was: {}", app.status);
  let confirmed: Vec<std::path::PathBuf> = app.pending_delete().iter().map(|t| t.path.clone()).collect();

  // The worktree is gone from git's list (pruned) but its removal reported a
  // failure — the exact shape a mid-removal filesystem error leaves behind.
  gwm::worktree::remove(&repo, "wt-doomed", false).unwrap();
  let generation = app.tasks.request(TaskKind::DeleteWorktree).unwrap();
  app
    .task_result_sender()
    .send(TaskMsg::DeleteWorktree(
      generation,
      DeleteBatchOutcome {
        removed: vec![],
        failed: vec![DeleteFailure {
          id: "wt-doomed".into(),
          path: doomed.clone(),
          error: "directory not empty".into(),
        }],
        warnings: vec![],
      },
    ))
    .unwrap();
  app.drain_task_results();

  assert_eq!(app.view, View::Confirm, "the failure keeps the overlay open");
  for target in app.pending_delete() {
    assert!(
      confirmed.contains(&target.path),
      "the retry batch must only ever contain rows the user confirmed, got {:?}",
      target.path
    );
  }
  assert!(
    !app.pending_delete().iter().any(|t| t.path == bystander),
    "and never the row the cursor landed on after the refresh"
  );
}

#[test]
fn the_failure_banner_separates_two_repos_sharing_a_worktree_id() {
  // Codex review on PR #520 (P2): a workspace batch spans repos, and two of
  // them can hold the same worktree id. The banner is where the user goes to
  // fix things, so it names each failure by path.
  use gwm::tui::{DeleteBatchOutcome, DeleteFailure};
  let outcome = DeleteBatchOutcome {
    removed: vec![],
    failed: vec![
      DeleteFailure {
        id: "feat-1-auth".into(),
        path: "/repos/alpha/feat-1-auth".into(),
        error: "locked".into(),
      },
      DeleteFailure {
        id: "feat-1-auth".into(),
        path: "/repos/beta/feat-1-auth".into(),
        error: "dirty".into(),
      },
    ],
    warnings: vec![],
  };
  let banner = outcome.failure_banner().expect("two failures produce a banner");
  assert!(
    banner.contains("/repos/alpha/feat-1-auth") && banner.contains("/repos/beta/feat-1-auth"),
    "both repos must be tellable apart: {banner}"
  );
}

// ---- per-worktree notes (#515) -------------------------------------------

#[test]
fn opening_the_note_editor_resolves_the_file_in_the_main_git_dir() {
  let (dir, mut app) = make_app();
  app.list_state.select(Some(0));

  app.open_note_editor();

  let path = app
    .note_editor
    .as_ref()
    .expect("the main row carries a branch")
    .path
    .clone();
  // `paths_equal` canonicalises: on macOS the tempdir is `/var/...` and the
  // resolved note is `/private/var/...`, the same inode spelled two ways.
  assert!(
    common::paths_equal(
      path.parent().unwrap(),
      &dir.path().join(".git").join("gwm").join("notes")
    ),
    "the note belongs in the main checkout's git dir, got {}",
    path.display()
  );
  assert_eq!(path.file_name().unwrap(), "main.md", "keyed on the branch");
  assert!(
    path.parent().unwrap().is_dir(),
    "the parent directory must exist before $EDITOR is spawned"
  );
  assert!(
    !path.exists(),
    "the file itself is the editor's to create — quitting without saving leaves no note"
  );
}

#[test]
fn opening_a_note_on_a_detached_row_says_why() {
  // No branch, no filename. The rule `pinnable_branch` settled for the agent
  // pin, restated so the key press explains itself instead of no-oping.
  let (_dir, mut app) = make_app();
  let mut row = worktree_fixture("detached");
  row.branch = None;
  app.worktrees = vec![row];
  app.list_state.select(Some(0));

  app.open_note_editor();

  assert!(app.note_editor.is_none());
  assert_eq!(
    app.view,
    View::List,
    "no modal opened over a row that cannot carry a note"
  );
  assert!(
    app.status.contains("detached"),
    "the status bar must carry the reason, got: {}",
    app.status
  );
}

#[test]
fn opening_a_note_with_nothing_selected_says_so() {
  let (_dir, mut app) = make_app();
  app.worktrees.clear();
  app.list_state.select(None);

  app.open_note_editor();

  assert!(app.note_editor.is_none());
  assert_eq!(app.view, View::List);
  assert_eq!(app.status, "nothing selected");
}

#[test]
fn a_branch_name_no_filesystem_can_back_refuses_the_note_out_loud() {
  // git accepts `feat/bad|name`; Windows does not. Refusing beats writing a
  // file whose name means a different branch once the repo is cloned there.
  let (_dir, mut app) = make_app();
  let mut row = worktree_fixture("weird");
  row.branch = Some("feat/bad|name".into());
  app.worktrees = vec![row];
  app.list_state.select(Some(0));

  app.open_note_editor();

  assert!(app.note_editor.is_none());
  assert_eq!(app.view, View::List);
  assert!(
    app.status.contains("portable"),
    "the status bar must say the name is the problem, got: {}",
    app.status
  );
}

#[test]
fn the_marker_follows_what_the_editor_left_behind() {
  // One file read for one row on the way back from `$EDITOR` — not a full
  // `refresh()`, which would drop the mark set (#484) and re-shell every
  // row's git config to repaint a single glyph.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.open_note_editor();
  let path = app.note_editor.as_ref().unwrap().path.clone();
  assert!(!app.worktrees[0].has_note, "no note yet");

  std::fs::write(&path, "what I had just figured out\n").unwrap();
  app.sync_selected_note_marker();
  assert!(app.worktrees[0].has_note, "the marker must light up after a save");

  // Emptied rather than deleted: the marker must go back down.
  std::fs::write(&path, "\n").unwrap();
  app.sync_selected_note_marker();
  assert!(!app.worktrees[0].has_note, "a blanked note must clear the marker");
}

// ---- launcher argv splitting (#515, Codex review pass 6) ------------------

#[test]
fn a_configured_editor_command_splits_into_program_and_arguments() {
  // `$EDITOR` and `$SHELL` are shell lines by convention: git, cargo and
  // systemctl all word-split them, and `[review]` / hook `run =` lines in
  // this repo already go through `shell_words`. Handing the whole string to
  // `Command::new` looked for an executable literally named `code --wait`,
  // so `N` could never open a note for a VS Code or Sublime user.
  use gwm::tui::launch_argv;

  assert_eq!(launch_argv("vi"), vec!["vi"]);
  assert_eq!(launch_argv("code --wait"), vec!["code", "--wait"]);
  assert_eq!(launch_argv("nvim -f"), vec!["nvim", "-f"]);
  assert_eq!(launch_argv("subl -w -n"), vec!["subl", "-w", "-n"]);
}

#[test]
fn a_program_path_containing_a_space_is_quoted_not_split() {
  // The counterpart of word-splitting: a real path with a space stays one
  // token when it is quoted, which is how every other consumer of these
  // variables expects it to be written.
  use gwm::tui::launch_argv;

  assert_eq!(
    launch_argv("\"/Applications/My App/bin/edit\" --wait"),
    vec!["/Applications/My App/bin/edit", "--wait"]
  );
  assert_eq!(
    launch_argv("'/Applications/My App/bin/edit'"),
    vec!["/Applications/My App/bin/edit"]
  );
}

#[test]
fn an_unparseable_command_is_passed_through_whole() {
  // An unbalanced quote is a user typo in `.gwm.toml`. Falling back to the
  // raw string keeps the pre-existing behaviour and lets the spawn failure
  // name what was configured, instead of turning a typo into a panic or a
  // silently different program.
  use gwm::tui::launch_argv;

  assert_eq!(
    launch_argv("edit --flag \"unbalanced"),
    vec!["edit --flag \"unbalanced"]
  );
  assert_eq!(launch_argv(""), vec![""]);
}

#[test]
fn a_command_that_names_a_real_file_is_never_split() {
  // Word-splitting is POSIX, filenames are not. `shell_words` drops an
  // unprotected backslash, so `EDITOR=C:\Tools\nvim.exe` — an absolute path
  // that `Command::new` launched fine before word-splitting was introduced —
  // came back as `C:Toolsnvim.exe` and stopped launching (Codex review, PR
  // #530). A string that already names a file is not a shell line: nothing is
  // left to split, so it is handed over whole.
  use gwm::tui::launch_argv;
  let dir = tempfile::TempDir::new().unwrap();

  let spaced = dir.path().join("My Editor.sh");
  std::fs::write(&spaced, "#!/bin/sh\n").unwrap();
  let spaced = spaced.to_str().unwrap();
  assert_eq!(launch_argv(spaced), vec![spaced]);

  // The backslash is the case the splitter actually mangles. Windows has no
  // filename that can carry one, so the file that proves it is a Unix file;
  // the code path it exercises is the same one a Windows path takes.
  #[cfg(unix)]
  {
    let backslashed = dir.path().join(r"Tools\nvim");
    std::fs::write(&backslashed, "#!/bin/sh\n").unwrap();
    let backslashed = backslashed.to_str().unwrap();
    assert_eq!(launch_argv(backslashed), vec![backslashed]);
  }
}

// --- rich PR / issue view wiring (issue #420) -----------------------------

/// A PR with enough rich payload for the view to have something to render.
fn rich_pr_fixture(number: u64) -> PrStatus {
  PrStatus {
    number,
    title: "rich fixture".into(),
    state: PrState::Open,
    url: format!("https://example.test/pull/{number}"),
    updated_at: "2026-08-04T13:00:00Z".into(),
    checks_passed: 1,
    checks_total: 1,
    ci: CiState::Passing,
    checks: vec![],
    detail: gwm::forge::PrDetail {
      body: "A description worth reading.".into(),
      author: "kbrdn1".into(),
      additions: 10,
      deletions: 2,
      base_ref: "dev".into(),
      head_ref: "feat/#42-tui-search".into(),
      reviews: vec![],
      comments: vec![],
    },
  }
}

fn rich_issue_fixture(number: u64) -> gwm::github::IssueStatus {
  gwm::github::IssueStatus {
    number,
    title: "rich issue fixture".into(),
    state: gwm::github::IssueState::Open,
    url: format!("https://example.test/issues/{number}"),
    labels: vec!["feature".into()],
    updated_at: "2026-08-01T10:00:00Z".into(),
    detail: gwm::forge::IssueDetail {
      body: "The issue body.".into(),
      author: "sassman".into(),
      comments: vec![],
    },
  }
}

#[test]
fn quoting_protects_a_backslashed_path_that_takes_arguments() {
  // The escape hatch for the case the fast path above cannot see: a path that
  // does not exist on this machine, or one that carries flags. Double quotes
  // are what the doc tells the user to reach for, so the claim is pinned
  // rather than asserted in prose — POSIX only treats `\` as an escape inside
  // double quotes before `$`, `` ` ``, `"`, `\` and a newline.
  use gwm::tui::launch_argv;

  assert_eq!(
    launch_argv("\"C:\\Tools\\nvim.exe\" --clean"),
    vec!["C:\\Tools\\nvim.exe", "--clean"]
  );
}

#[test]
fn rich_view_prefers_the_linked_pr() {
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));

  app.enter_rich_view();

  assert_eq!(app.view, View::DetailOverlay);
  assert_eq!(
    app.detail_overlay.kind,
    DetailKind::RichPr,
    "with both linked, the PR is the thing being worked on"
  );
  let vals: Vec<&str> = app.detail_overlay.rows.iter().map(|r| r.value.as_str()).collect();
  assert!(vals.contains(&"kbrdn1"), "the metadata block rendered: {vals:?}");
  assert!(
    vals.iter().any(|v| v.contains("A description worth reading.")),
    "the body rendered: {vals:?}"
  );
}

#[test]
fn rich_view_falls_back_to_the_issue() {
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));

  app.enter_rich_view();

  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue);
  let vals: Vec<&str> = app.detail_overlay.rows.iter().map(|r| r.value.as_str()).collect();
  assert!(vals.iter().any(|v| v.contains("The issue body.")), "{vals:?}");
}

#[test]
fn rich_view_without_a_fetched_status_explains_instead_of_opening_blank() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.refresh_link();

  app.enter_rich_view();

  assert_eq!(app.view, View::List, "an empty overlay is a bordered void");
  assert!(
    app.status.contains("fetch"),
    "the status must name the way out: {}",
    app.status
  );
}

#[test]
fn rich_view_enter_opens_the_selected_row_url() {
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();

  let url_row = app
    .detail_overlay
    .rows
    .iter()
    .position(|r| r.label == "url")
    .expect("a url row");
  app.detail_overlay.selected = url_row;

  assert_eq!(app.rich_selected_url().as_deref(), Some("https://example.test/pull/61"));

  // An inert row (the body) exposes nothing to open.
  let body_row = app
    .detail_overlay
    .rows
    .iter()
    .position(|r| r.value.contains("A description worth reading."))
    .expect("a body row");
  app.detail_overlay.selected = body_row;
  assert_eq!(app.rich_selected_url(), None);
}

#[test]
fn a_link_change_closes_the_rich_view() {
  // Same invariant the CI checks overlay carries (Codex review #455): the
  // rows belong to the PR they were built for, and `Enter` would otherwise
  // open the previous link's URL. `is_forge_linked` is what makes the
  // guard cover every forge-backed consumer by construction.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();
  assert_eq!(app.view, View::DetailOverlay);

  gwm::github::link_pr(&repo, "feat/#42-tui-search", 62).unwrap();
  app.refresh_link();

  assert_eq!(app.view, View::List, "the overlay must not outlive its link");
}

#[test]
fn a_resize_rewraps_the_rich_view() {
  // The builder wraps against a width the App carries; a resize that never
  // reaches the App leaves rows wrapped for the old width and the renderer
  // ellipsises them.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut pr = rich_pr_fixture(61);
  pr.detail.body = "word ".repeat(120);
  app.apply_pr_fetch_result(Ok(pr));
  app.set_term_width(200);
  app.enter_rich_view();
  let wide = app.detail_overlay.rows.len();

  app.set_term_width(60);
  let narrow = app.detail_overlay.rows.len();

  assert!(
    narrow > wide,
    "a narrower terminal must produce more wrapped rows ({narrow} vs {wide})"
  );
}

#[test]
fn an_issue_standing_in_for_a_slower_pr_is_replaced_when_it_lands() {
  // The original concern (Codex review #529, first pass) replayed by hand
  // after the guard that answered it was REMOVED. It refused to open while
  // the PR was `Loading`, which guarded a symptom of the missing
  // promotion; once `sync_rich_overlay` existed the guard only produced
  // its own edge case, since `Idle` ("nobody asked yet") is
  // indistinguishable from `Loading` for a user staring at the wrong side.
  //
  // What actually had to hold is this: the user never gets STUCK on the
  // issue. Showing it meanwhile beats showing nothing.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.mark_pr_loading_for_test(61);

  app.enter_rich_view();
  assert_eq!(
    app.detail_overlay.kind,
    DetailKind::RichIssue,
    "what is available opens rather than nothing"
  );

  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));

  assert_eq!(
    app.detail_overlay.kind,
    DetailKind::RichPr,
    "and the PR claims the view the moment it lands"
  );
}

#[test]
fn an_origin_move_between_instances_closes_the_rich_view() {
  // Codex review #529: the overlay identity was keyed on the bare SLUG, so
  // an origin moving from github.com/acme/widgets to gitlab.com/acme/widgets
  // compared equal, the overlay survived, and `Enter` still opened the old
  // host's URL. Same failure `open_menu_drops_a_cached_url_from_the_previous_origin`
  // pins for the fetch caches (Codex review #458); the identity now carries
  // the full `<kind> <web origin>/<slug>`.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  repo.remote("origin", "https://github.com/acme/widgets.git").unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.enter_rich_view();
  assert_eq!(app.view, View::DetailOverlay, "precondition");

  repo.remote_delete("origin").unwrap();
  repo.remote("origin", "https://gitlab.com/acme/widgets.git").unwrap();
  app.refresh_link();

  assert_eq!(
    app.view,
    View::List,
    "the overlay outlived the instance its rows describe"
  );
}

#[test]
fn rich_view_falls_back_to_the_issue_when_the_pr_fetch_failed() {
  // The other half of the same rule: a PR whose fetch ERRORED is never
  // going to land, so refusing to show the issue would leave the user with
  // nothing. Only an in-flight PR holds the view back.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.apply_pr_fetch_result(Err("gh: not found".into()));

  app.enter_rich_view();

  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue);
}

#[test]
fn a_resize_still_rewraps_after_the_cache_was_flushed() {
  // Codex review #529: the rebuild read the fetch CACHE, so a resize while
  // a refresh was in flight found no `Loaded` and gave up. If that refresh
  // then failed, nothing ever rebuilt and the view stayed wrapped for the
  // old terminal for good. The overlay owns its source instead, the same
  // fix `ci_overlay_checks` already carries for the duration tick (#455).
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut pr = rich_pr_fixture(61);
  pr.detail.body = "word ".repeat(120);
  app.apply_pr_fetch_result(Ok(pr));
  app.set_term_width(200);
  app.enter_rich_view();
  let wide = app.detail_overlay.rows.len();

  // The `F` refresh flushes the cache before re-requesting.
  app.refresh_link();
  assert_eq!(app.view, View::DetailOverlay, "same link, the overlay stays up");

  app.set_term_width(60);

  assert!(
    app.detail_overlay.rows.len() > wide,
    "the overlay must re-wrap from its own source, not from a flushed cache"
  );
}

#[test]
fn a_landing_pr_promotes_the_rich_view_off_the_issue() {
  // Codex review #529, second pass. The invariant, written once instead of
  // patched a third time: while the rich view is open it renders the side
  // the LINK prefers, in its freshest version, title included. So a PR
  // landing takes over an issue view that was only standing in for it.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.apply_pr_fetch_result(Err("gh: transient".into()));
  app.enter_rich_view();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue, "precondition");

  // `f` succeeds this time.
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));

  assert_eq!(
    app.detail_overlay.kind,
    DetailKind::RichPr,
    "the PR must claim the view it is entitled to"
  );
  assert!(
    app.detail_overlay.title.contains("#61"),
    "the title must follow the content: {}",
    app.detail_overlay.title
  );
}

#[test]
fn a_landing_issue_does_not_displace_the_pr_view() {
  // The other direction of the same invariant: the link prefers the PR, so
  // an issue landing refreshes the view only when the issue IS the view.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichPr, "precondition");

  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));

  assert_eq!(app.detail_overlay.kind, DetailKind::RichPr);
}

#[test]
fn a_refreshed_title_follows_a_renamed_pr() {
  // Same invariant, third consequence: `f` used to replace the source and
  // the rows while keeping the title computed at open, so a PR renamed
  // upstream showed fresh content under a stale heading.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();
  assert!(app.detail_overlay.title.contains("rich fixture"), "precondition");

  let mut renamed = rich_pr_fixture(61);
  renamed.title = "renamed upstream".into();
  app.apply_pr_fetch_result(Ok(renamed));

  assert!(
    app.detail_overlay.title.contains("renamed upstream"),
    "stale heading over fresh content: {}",
    app.detail_overlay.title
  );
}

// ---- the in-TUI note editor (#515) ---------------------------------------

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Open the note editor on the main row, which always carries a branch.
fn app_with_note_open() -> (tempfile::TempDir, App) {
  let (dir, mut app) = make_app();
  // #557: the mode ships on, so the #515 editor is now the opt-out. These
  // tests are the contract for `note_vim = false`, which is a supported
  // config and not a leftover — every printable is text and one `Esc`
  // writes and closes.
  app.config.tui.note_vim = false;
  app.list_state.select(Some(0));
  app.open_note_editor();
  assert_eq!(app.view, View::Note, "the editor must be the active view");
  (dir, app)
}

#[test]
fn typing_in_the_note_editor_never_reaches_the_global_keymap() {
  // The bug this exists to stop: `d` is the global delete verb. A modal
  // that captures every printable must consume it, or writing the word
  // "done" in a note opens the delete confirm on the worktree the note is
  // about — and the second `d` is inside a running countdown.
  let (_dir, mut app) = app_with_note_open();

  for c in "done".chars() {
    app.handle_note_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
  }

  assert_eq!(app.view, View::Note, "no global verb fired");
  assert_eq!(
    app.note_editor.as_ref().unwrap().lines,
    vec!["done"],
    "every keystroke landed in the buffer"
  );
}

#[test]
fn every_global_single_key_default_is_swallowed_by_the_note_editor() {
  // Enumerated rather than sampled: `d` is the one that destroys data, but
  // any global default reaching through would be a keystroke the user
  // cannot type. Derived from the keymap so a verb added later is covered
  // without editing this test.
  let (_dir, mut app) = app_with_note_open();
  let singles: Vec<char> = app
    .keymap
    .list()
    .into_iter()
    .flat_map(|binding| binding.chords)
    .filter(|chord| chord.len() == 1)
    .filter_map(|chord| match chord[0].code {
      KeyCode::Char(c) if chord[0].modifiers.is_empty() => Some(c),
      _ => None,
    })
    .collect();
  assert!(singles.len() > 5, "the keymap should have plenty of single-key verbs");

  for c in &singles {
    app.handle_note_key(KeyEvent::new(KeyCode::Char(*c), KeyModifiers::NONE));
    assert_eq!(app.view, View::Note, "`{c}` escaped the note editor");
  }
  let typed: String = singles.iter().collect();
  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec![typed]);
}

#[test]
fn esc_writes_the_note_and_closes() {
  // Esc saves: the reflex on leaving a note is to keep it, and there is no
  // "quit without saving" to lose prose to.
  let (_dir, mut app) = app_with_note_open();
  for c in "kept".chars() {
    app.handle_note_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
  }
  let path = app.note_editor.as_ref().unwrap().path.clone();

  app.handle_note_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

  assert_eq!(app.view, View::List, "the editor closed");
  assert!(app.note_editor.is_none(), "and dropped its buffer");
  assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept\n");
}

#[test]
fn clearing_the_buffer_removes_the_note_instead_of_writing_a_blank_file() {
  // The only way to discard, so it has to actually delete: a one-byte file
  // reads as "no note" everywhere but would still sit on disk and be found
  // by `gwm doctor` once the branch is gone.
  //
  // Backspace is the gesture with the mode off (#557); `dd` is its twin in
  // the default mode, pinned by the test below.
  let (dir, mut app) = make_app();
  app.config.tui.note_vim = false;
  app.list_state.select(Some(0));
  let branch = app.selected().unwrap().branch.clone().unwrap();
  let path = gwm::notes::prepare(&git2::Repository::open(dir.path()).unwrap(), &branch)
    .unwrap()
    .unwrap();
  std::fs::write(&path, "old prose\n").unwrap();

  app.open_note_editor();
  for _ in 0..20 {
    app.handle_note_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
  }
  app.handle_note_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

  assert!(!path.exists(), "an emptied note is removed, not blanked");
}

#[test]
fn closing_an_untouched_note_does_not_rewrite_the_file() {
  // Opening a note to read it must not touch its mtime, which is what
  // `dirty` is for.
  let (dir, mut app) = make_app();
  app.list_state.select(Some(0));
  let branch = app.selected().unwrap().branch.clone().unwrap();
  let path = gwm::notes::prepare(&git2::Repository::open(dir.path()).unwrap(), &branch)
    .unwrap()
    .unwrap();
  std::fs::write(&path, "untouched\n").unwrap();
  let before = std::fs::metadata(&path).unwrap().modified().unwrap();

  app.open_note_editor();
  app.handle_note_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
  app.handle_note_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

  assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
  assert_eq!(std::fs::read_to_string(&path).unwrap(), "untouched\n");
}

#[test]
fn ctrl_e_writes_the_buffer_before_handing_the_file_to_the_editor() {
  // `$EDITOR` opens the file, not the buffer. Launching without flushing
  // would show the user their note as it was before the keys they just
  // typed, and their save would then overwrite those keys.
  let (_dir, mut app) = app_with_note_open();
  for c in "typed here".chars() {
    app.handle_note_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
  }

  let outcome = app.handle_note_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));

  let NoteKey::LaunchEditor(command, path) = outcome else {
    panic!("Ctrl+e must hand the run loop an editor command, got {outcome:?}");
  };
  assert!(!command.is_empty(), "an editor command always resolves");
  assert_eq!(std::fs::read_to_string(&path).unwrap(), "typed here\n");
  assert_eq!(app.view, View::Note, "the editor overlay stays open behind $EDITOR");
}

#[test]
fn returning_from_the_external_editor_reloads_what_it_wrote() {
  let (_dir, mut app) = app_with_note_open();
  app.handle_note_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
  let NoteKey::LaunchEditor(_, path) = app.handle_note_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL))
  else {
    panic!("expected a launch");
  };
  std::fs::write(&path, "rewritten outside\n").unwrap();

  app.reload_note_after_editor();

  assert_eq!(
    app.note_editor.as_ref().unwrap().lines,
    vec!["rewritten outside", ""],
    "the buffer shows what $EDITOR left, not what was typed before it"
  );
}

#[test]
fn a_detached_row_says_so_rather_than_opening_an_editor_on_nothing() {
  let (_dir, mut app) = make_app();
  app.worktrees[0].branch = None;
  app.list_state.select(Some(0));

  app.open_note_editor();

  assert_eq!(app.view, View::List, "no editor opened");
  assert!(app.note_editor.is_none());
  assert!(
    app.status.contains("detached"),
    "the refusal is on the status bar: {}",
    app.status
  );
}

// --- inline review comments wiring (issue #528) ---------------------------

fn one_thread() -> gwm::forge::ReviewThreads {
  gwm::forge::ReviewThreads::Threads {
    threads: vec![gwm::forge::ReviewThread {
      path: "src/tui/app.rs".into(),
      line: Some(11),
      start_line: Some(7),
      is_resolved: false,
      is_outdated: false,
      diff_hunk: "@@ -4,10 +4,11 @@\n-old\n+new".into(),
      total_comments: 1,
      comments: vec![gwm::forge::ForgeComment {
        author: "coderabbitai".into(),
        body: "This drops the guard.".into(),
        created_at: "2026-08-04T13:40:21Z".into(),
        url: Some("https://example.test/pull/61#discussion_r1".into()),
      }],
    }],
    total: 1,
  }
}

fn overlay_text(app: &gwm::tui::App) -> String {
  app
    .detail_overlay
    .rows
    .iter()
    .map(|r| r.value.clone())
    .collect::<Vec<_>>()
    .join("\n")
}

#[test]
fn landed_threads_reach_an_already_open_rich_view() {
  // Same invariant as the PR itself: while the view is open it renders
  // the freshest version of what it is showing. A second transport that
  // resolves *after* the view opened is the common case here, not an
  // edge one — the view is what triggers the request.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();
  assert!(
    !overlay_text(&app).contains("src/tui/app.rs:7-11"),
    "precondition: the threads have not landed yet"
  );

  app.apply_pr_threads_fetch_result(61, Ok(one_thread()));

  assert!(
    overlay_text(&app).contains("src/tui/app.rs:7-11"),
    "the landing never reached the open view:\n{}",
    overlay_text(&app)
  );
}

#[test]
fn a_thread_result_for_another_pr_is_not_shown() {
  // The cache is keyed by number for the reason #138 already paid for.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();

  app.apply_pr_threads_fetch_result(62, Ok(one_thread()));

  assert!(
    !overlay_text(&app).contains("src/tui/app.rs:7-11"),
    "PR 62's threads rendered under PR 61"
  );
}

#[test]
fn a_failed_thread_fetch_is_shown_not_swallowed() {
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();

  app.apply_pr_threads_fetch_result(61, Err("gh: HTTP 403".into()));

  assert!(
    overlay_text(&app).contains("gh: HTTP 403"),
    "in:\n{}",
    overlay_text(&app)
  );
}

#[test]
fn a_link_refresh_drops_cached_threads_with_everything_else() {
  // Threads live in their own cache, so the invalidation that clears the
  // PR must clear them too — otherwise a refreshed PR renders next to the
  // previous run's inline comments.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_threads_fetch_result(61, Ok(one_thread()));
  assert!(
    matches!(app.pr_threads_fetch_state(61), gwm::tui::GitHubFetchState::Loaded(_)),
    "precondition"
  );

  app.refresh_link();

  assert!(
    matches!(app.pr_threads_fetch_state(61), gwm::tui::GitHubFetchState::Idle),
    "stale threads survived the invalidation"
  );
}

#[test]
fn the_issue_view_never_grows_a_threads_section() {
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));

  app.enter_rich_view();

  assert!(!overlay_text(&app).to_lowercase().contains("inline comments"));
}

/// A `gh` stand-in that answers the review-threads GraphQL query, the PR
/// re-probe (`pr list`) and the two `view` calls a refresh makes. Written
/// once because the two tests below need the same surface.
#[cfg(unix)]
fn fake_gh_with_threads(dir: &std::path::Path) -> PathBuf {
  use std::os::unix::fs::PermissionsExt;
  let path = dir.join("fake-gh-threads");
  std::fs::write(
    &path,
    r##"#!/bin/sh
if [ "$1" = "api" ] && [ "$2" = "graphql" ]; then
  # `\\n` so printf emits a literal backslash-n: a raw newline inside a JSON
  # string is a control character and takes the whole parse down.
  printf '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":1,"nodes":[{"isResolved":false,"isOutdated":false,"path":"src/tui/app.rs","line":11,"startLine":7,"comments":{"totalCount":1,"nodes":[{"author":{"login":"coderabbitai"},"body":"This drops the guard.","diffHunk":"@@ -4,10 +4,11 @@\\n+new","createdAt":"2026-08-04T13:40:21Z","url":"https://example.test/pull/61#discussion_r1"}]}}]}}}}}'
elif [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '[{"number":61}]'
elif [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  printf '{"number":61,"title":"pr 61","state":"OPEN","isDraft":false,"url":"https://example.test/pull/61","updatedAt":"2026-06-09T00:00:00Z","statusCheckRollup":[]}'
elif [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  printf '{"number":42,"title":"issue 42","state":"OPEN","url":"https://example.test/issues/42","labels":[],"updatedAt":"2026-06-09T00:00:00Z"}'
else
  exit 2
fi
"##,
  )
  .unwrap();
  let mut perms = std::fs::metadata(&path).unwrap().permissions();
  perms.set_mode(0o755);
  std::fs::set_permissions(&path, perms).unwrap();
  path
}

/// Drain until the threads worker for `n` has landed, or give up. Mirrors
/// the polling the bulk-refresh test uses: whether a task is still in
/// `running` at an arbitrary instant is a timing detail (issue #425), so
/// the assertion is on the result, never on `is_loading`.
#[cfg(unix)]
fn settle_threads(app: &mut gwm::tui::App, n: u64) {
  for _ in 0..200 {
    app.drain_task_results();
    if !app.tasks.is_loading(gwm::tui::TaskKind::GithubPrThreads(n)) {
      break;
    }
    std::thread::sleep(Duration::from_millis(10));
  }
  app.drain_task_results();
}

#[cfg(unix)]
#[test]
fn opening_the_view_requests_the_threads_and_renders_what_lands() {
  // The one test that exercises the SPAWN path end to end: the request is
  // fired by `enter_rich_view`, runs through the real worker, and lands via
  // the drain rather than the `apply_*` seam. Everything else about the
  // threads goes through the seam, which would leave the whole spawn +
  // drain path green-but-unrun.
  let (dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  let fake_gh = fake_gh_with_threads(dir.path());

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: env mutation is guarded by `env_lock()` and restored below. The
  // backend captures this path on the main thread when it is built.
  unsafe {
    std::env::set_var("GWM_GH", &fake_gh);
  }

  app.refresh_link(); // resolves the forge now that `origin` exists
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();
  settle_threads(&mut app, 61);

  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  assert!(
    overlay_text(&app).contains("src/tui/app.rs:7-11"),
    "the view never asked, or the answer never landed:\n{}",
    overlay_text(&app)
  );
  assert!(
    overlay_text(&app).contains("This drops the guard."),
    "the chain did not survive the round trip"
  );
}

#[cfg(unix)]
#[test]
fn refreshing_the_view_asks_for_the_threads_again() {
  // `refresh_github_status` invalidates all three caches, so without a
  // re-spawn `f` would blank the section until the view was reopened. The
  // guard that re-spawns is also gated on the view still being open, which
  // `close_detail_overlay` does not express through `kind` alone.
  let (dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  let fake_gh = fake_gh_with_threads(dir.path());

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: as above.
  unsafe {
    std::env::set_var("GWM_GH", &fake_gh);
  }

  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();
  settle_threads(&mut app, 61);
  assert!(
    overlay_text(&app).contains("src/tui/app.rs:7-11"),
    "precondition: the first fetch landed"
  );

  app.rich_view_refresh();
  settle_threads(&mut app, 61);

  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  // Asserted on the CACHE, not on the rows. `refresh_github_status` does
  // not rebuild the overlay (a landing does), so the previous run's rows
  // are still on screen either way — an assertion on them stays green with
  // the re-spawn deleted, which is exactly the vacant guard this test would
  // otherwise be. `Idle` is what the invalidation leaves behind when
  // nothing asks again.
  assert!(
    matches!(app.pr_threads_fetch_state(61), gwm::tui::GitHubFetchState::Loaded(_)),
    "`f` dropped the cached threads and never asked again: {:?}",
    app.pr_threads_fetch_state(61)
  );
  assert!(
    overlay_text(&app).contains("src/tui/app.rs:7-11"),
    "and the section still renders them:\n{}",
    overlay_text(&app)
  );
}

#[test]
fn activating_layout_from_the_panel_switches_the_live_layout() {
  // Codex review, PR #546: the panel is documented as the editable
  // schema, so `bordered` — the opt-out of the layout #545 made the
  // default — must be reachable from it and take effect without a
  // relaunch.
  //
  // No `apply_*` step is needed for this one, and that is the point of
  // reading `config.tui.layout` at render time rather than mirroring it
  // onto `App`: reloading the config *is* applying it. The assertion
  // below is what proves that, so a future refactor that caches the
  // layout on `App` fails here until it wires its own apply.
  use gwm::config::TuiLayout;
  use gwm::tui::SettingsTab;

  let (_dir, mut app) = make_app();
  app.enter_config_panel();
  app.config_panel.tab = SettingsTab::Tui;
  app.config_panel.selected = SettingsTab::Tui
    .fields()
    .iter()
    .position(|f| *f == gwm::tui::SettingField::Layout)
    .expect("the TUI tab must offer the layout field");
  assert_eq!(app.config.tui.layout, TuiLayout::Compact, "default is compact");

  app.activate_selected_setting();
  assert_eq!(
    app.config.tui.layout,
    TuiLayout::Bordered,
    "cycling the choice must reach the live config"
  );

  // And back, so the cycle is a cycle rather than a one-way door.
  app.activate_selected_setting();
  assert_eq!(app.config.tui.layout, TuiLayout::Compact);
}

#[test]
fn every_panel_choice_survives_the_write_it_triggers() {
  // Codex review, PR #546: `dim_unfocused` was classed `FieldKind::Choice`,
  // which routes the write through `set_string_at` and produced
  // `dim_unfocused = "true"` — a string where serde wants a bool. The load
  // then failed and the setting never changed.
  //
  // The existing round-trip guard could not catch it: it hand-lists four
  // fields while claiming to cover "every Choice field", so a fifth was
  // invisible to it. This one enumerates from the panel itself — every
  // tab, every field it offers — and exercises the real write path
  // (`activate_selected_setting`) rather than simulating the TOML, so it
  // covers how the value is spelled as well as what it says.
  use gwm::tui::{FieldKind, SettingsTab};

  for tab in SettingsTab::ALL {
    for (index, field) in tab.fields().iter().enumerate() {
      if matches!(field.kind(), FieldKind::Text | FieldKind::Uint) {
        continue; // typed, not cycled — a different write path
      }
      let (_dir, mut app) = make_app();
      app.enter_config_panel();
      app.config_panel.tab = tab;
      app.config_panel.selected = index;

      // Cycle through every choice the field offers, back to the start.
      // Asserting the *value moved* rather than just that the file still
      // loads: a write that fails leaves the config untouched, so a
      // load-only check passes while the setting silently never changes —
      // which is exactly the failure mode under test.
      for step in 0..field.choices().len() {
        let before = field.current(&app.config);
        app.activate_selected_setting();
        let file = _dir.path().join(gwm::config::CONFIG_FILE);
        let reloaded = gwm::config::Config::load_layered(_dir.path(), None);
        assert!(
          reloaded.is_ok(),
          "{}: the panel wrote a value the config cannot load back: {:?}\nfile:\n{}",
          field.key_path(),
          reloaded.err(),
          std::fs::read_to_string(&file).unwrap_or_default()
        );
        let after = field.current(&app.config);
        assert_ne!(
          before,
          after,
          "{} step {step}: activating must move the value, got {before:?} again — status: {:?}\nfile:\n{}",
          field.key_path(),
          app.status,
          std::fs::read_to_string(&file).unwrap_or_default()
        );
      }
    }
  }
}

/// A worktree with both an issue and a PR linked and fetched, the rich view
/// open on the PR (issue #551).
fn app_with_both_sides_linked() -> (tempfile::TempDir, git2::Repository, App) {
  let (dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_rich_view();
  (dir, repo, app)
}

#[test]
fn tab_reaches_the_issue_the_pr_was_standing_in_front_of() {
  // Issue #551. The PR wins whenever one is linked, which is the right
  // default and left the issue unreachable: a worktree in review had no way
  // back to the thing it is solving without unlinking the PR.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, _repo, mut app) = app_with_both_sides_linked();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichPr);

  app.rich_view_next_tab();

  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue);
  let vals: Vec<&str> = app.detail_overlay.rows.iter().map(|r| r.value.as_str()).collect();
  assert!(vals.iter().any(|v| v.contains("The issue body.")), "{vals:?}");
  assert!(
    app.detail_overlay.title.contains("Issue #42"),
    "the title follows the tab: {}",
    app.detail_overlay.title
  );

  app.rich_view_next_tab();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichPr, "and back again");
}

#[test]
fn the_overlay_link_follows_the_active_tab() {
  // The overlay is pinned to the link it renders so a disagreeing mutation
  // closes it (#529). With tabs that pin has to follow the TAB, or switching
  // to the issue would leave the overlay claiming to be the PR and a moved
  // PR link would close the issue tab out from under the reader.
  let (_dir, repo, mut app) = app_with_both_sides_linked();
  app.rich_view_next_tab();

  // The PR link moves. The issue tab has nothing to do with it.
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 62).unwrap();
  app.refresh_link();

  assert_eq!(
    app.view,
    View::DetailOverlay,
    "a PR link change must not close the issue tab"
  );
}

#[test]
fn a_landing_pr_does_not_yank_the_reader_off_a_chosen_issue_tab() {
  // The interaction the tabs create, and the one that would have shipped
  // silently: `sync_rich_overlay` promotes the issue to the PR the moment
  // the PR lands, which is right when the view opened on the issue only
  // because the PR was not there yet, and wrong when the reader ASKED for
  // the issue. Same class as the bug the promotion itself fixed (#529).
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, _repo, mut app) = app_with_both_sides_linked();
  app.rich_view_next_tab();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue);

  // A refresh lands the PR again while the reader is on the issue tab.
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));

  assert_eq!(
    app.detail_overlay.kind,
    DetailKind::RichIssue,
    "the reader chose this tab; a landing fetch does not get to overrule it"
  );
}

#[test]
fn an_unchosen_issue_tab_is_still_promoted_when_the_pr_lands() {
  // The other side of the pin, and the reason it is a pin rather than a
  // switch: with no PR fetched yet the view opens on the issue because that
  // is all there is, and the reader never asked for it. Promoting is exactly
  // right there, and removing the promotion to make the test above pass
  // would have re-broken #529.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.enter_rich_view();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue);

  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));

  assert_eq!(app.detail_overlay.kind, DetailKind::RichPr);
}

#[test]
fn closing_the_view_forgets_which_tab_was_chosen() {
  // The pin belongs to one open overlay. Left behind, it would silently
  // change what the NEXT `I` opens on, which is a setting nobody set.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, _repo, mut app) = app_with_both_sides_linked();
  app.rich_view_next_tab();
  app.close_detail_overlay();

  app.enter_rich_view();

  assert_eq!(
    app.detail_overlay.kind,
    DetailKind::RichPr,
    "a fresh open goes back to preferring the PR"
  );
}

#[test]
fn tab_is_inert_when_there_is_only_one_side() {
  // No second tab to reach, so the key must do nothing rather than close the
  // view or blank it.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.refresh_link();
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.enter_rich_view();

  app.rich_view_next_tab();

  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue);
  assert_eq!(app.view, View::DetailOverlay);
  assert!(
    app.rich_view_tabs().is_empty(),
    "and no tab bar is offered: {:?}",
    app.rich_view_tabs()
  );
}

#[test]
fn the_tab_bar_names_both_sides_and_marks_the_active_one() {
  let (_dir, _repo, mut app) = app_with_both_sides_linked();
  assert_eq!(
    app.rich_view_tabs(),
    vec![("Issue #42".to_string(), false), ("PR #61".to_string(), true)]
  );
  app.rich_view_next_tab();
  assert_eq!(
    app.rich_view_tabs(),
    vec![("Issue #42".to_string(), true), ("PR #61".to_string(), false)]
  );
}

#[test]
fn the_horizontal_offset_only_moves_as_far_as_there_is_something_to_see() {
  // Issue #551. A fenced line or a diff hunk is kept whole rather than
  // reflowed, so the offset is the only way to its tail. Unbounded, it would
  // scroll a view full of prose into blank space and leave the reader with
  // no clue how to get back.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut pr = rich_pr_fixture(61);
  let long = "x".repeat(400);
  pr.detail.body = format!("prose\n\n```\n{long}\n```");
  app.apply_pr_fetch_result(Ok(pr));
  app.set_term_width(200);
  app.enter_rich_view();

  assert_eq!(app.rich_h_offset(), 0, "it starts at the left edge");
  app.rich_view_scroll_left();
  assert_eq!(app.rich_h_offset(), 0, "and cannot go further left than that");

  for _ in 0..200 {
    app.rich_view_scroll_right();
  }
  let stopped = app.rich_h_offset();
  assert!(stopped > 0, "the offset moved");
  assert!(
    stopped < 400,
    "and stopped once the widest row's tail was on screen, at {stopped}"
  );

  app.rich_view_scroll_left();
  assert!(app.rich_h_offset() < stopped, "left walks it back");
}

#[test]
fn a_view_with_nothing_to_scroll_does_not_scroll() {
  // Every row is wrapped to fit, so there is no tail to reach and moving
  // would only hide the left edge of the text.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.set_term_width(200);
  app.enter_rich_view();

  app.rich_view_scroll_right();

  assert_eq!(app.rich_h_offset(), 0);
}

#[test]
fn switching_tab_or_closing_puts_the_offset_back_at_the_left_edge() {
  // The offset describes one side's widest row. Carried across, it would
  // open the other tab already scrolled, with its first columns hidden and
  // nothing on screen saying why.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut pr = rich_pr_fixture(61);
  pr.detail.body = format!("```\n{}\n```", "x".repeat(400));
  app.apply_pr_fetch_result(Ok(pr));
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.set_term_width(200);
  app.enter_rich_view();
  app.rich_view_scroll_right();
  assert!(app.rich_h_offset() > 0);

  app.rich_view_next_tab();
  assert_eq!(app.rich_h_offset(), 0, "a tab switch resets it");

  app.rich_view_next_tab();
  app.rich_view_scroll_right();
  app.close_detail_overlay();
  app.enter_rich_view();
  assert_eq!(app.rich_h_offset(), 0, "and so does closing the view");
}

#[test]
fn widening_the_terminal_does_not_leave_the_offset_past_the_end() {
  // Issue #551. The offset is bounded by what the widest preformatted row
  // has left to show, and that bound MOVES: a wider terminal is a wider
  // modal, so the same row runs out of tail sooner. Left where it was, the
  // renderer skips past the end of the line and paints a blank row with
  // nothing on screen to say why. Same class as the stale wrap
  // `set_term_width` already exists to prevent, and a refresh that returns
  // a shorter body gets there the same way.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut pr = rich_pr_fixture(61);
  pr.detail.body = format!("```\n{}\n```", "x".repeat(400));
  app.apply_pr_fetch_result(Ok(pr));
  app.set_term_width(60);
  app.enter_rich_view();

  for _ in 0..100 {
    app.rich_view_scroll_right();
  }
  let narrow = app.rich_h_offset();
  assert!(narrow > 0, "precondition: it scrolled on the narrow terminal");

  app.set_term_width(200);

  let widest = app
    .detail_overlay
    .rows
    .iter()
    .filter(|r| r.preformatted)
    .map(|r| r.value.chars().count())
    .max()
    .unwrap_or(0);
  assert!(
    app.rich_h_offset() < widest,
    "the offset must still land inside the widest row, got {} of {widest}",
    app.rich_h_offset()
  );
  assert!(
    app.rich_h_offset() < narrow,
    "and a wider modal leaves less to scroll, not the same"
  );
}

#[test]
fn the_tab_bar_survives_a_refresh_that_only_one_side_answers() {
  // Codex review, pass 2 (P2). `rich_view_tabs` demanded BOTH caches be
  // `Loaded`, but the overlay deliberately keeps its own source when a
  // refresh fails. So a refresh where the displayed side errors and the
  // other lands took the bar away and made `Tab` inert, stranding the
  // reader on stale data with no way across until they closed the view.
  //
  // The active side comes from the overlay's own source, which survives;
  // only the DESTINATION has to be loaded.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, _repo, mut app) = app_with_both_sides_linked();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichPr);
  assert_eq!(app.rich_view_tabs().len(), 2);

  // The PR side fails, the issue side is still there.
  app.apply_pr_fetch_result(Err("gh: HTTP 502".into()));

  assert_eq!(
    app.rich_view_tabs().len(),
    2,
    "the bar must still offer the side that IS loaded: {:?}",
    app.rich_view_tabs()
  );
  app.rich_view_next_tab();
  assert_eq!(
    app.detail_overlay.kind,
    DetailKind::RichIssue,
    "and Tab must still cross to it"
  );
}

#[test]
fn a_promotion_puts_the_horizontal_offset_back_at_the_left_edge() {
  // Codex review, pass 6 (P2). The offset resets on a tab switch and on
  // close, but a PR landing on an issue the view was standing in for
  // changes sides through `sync_rich_overlay`, which went past both. A PR
  // carrying a preformatted line of its own then opened already scrolled,
  // with its first columns hidden and nothing on screen saying why — the
  // exact failure the two existing resets were added to prevent.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut issue = rich_issue_fixture(42);
  issue.detail.body = format!("```\n{}\n```", "x".repeat(400));
  app.apply_issue_fetch_result(Ok(issue));
  app.set_term_width(200);
  app.enter_rich_view();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue);

  app.rich_view_scroll_right();
  assert!(app.rich_h_offset() > 0, "precondition: the issue scrolled");

  // The PR the issue was standing in for lands, carrying a long line too.
  let mut pr = rich_pr_fixture(61);
  pr.detail.body = format!("```\n{}\n```", "y".repeat(400));
  app.apply_pr_fetch_result(Ok(pr));

  assert_eq!(app.detail_overlay.kind, DetailKind::RichPr, "it promoted");
  assert_eq!(app.rich_h_offset(), 0, "and the new side opens at its left edge");
}

#[test]
fn the_rich_view_yanks_the_url_and_the_body_of_the_active_tab() {
  // Validation feedback: `y` copies the URL, `Y` copies the description.
  // Both read the OVERLAY's own source rather than the fetch cache, for the
  // reason `rebuild_rich_rows` gives — a manual refresh flushes that cache,
  // so a yank landing in that window would find nothing and copy an empty
  // string over whatever the user had.
  let (_dir, _repo, mut app) = app_with_both_sides_linked();

  assert_eq!(
    app.rich_yank_url().as_deref(),
    Some("https://example.test/pull/61"),
    "the PR tab yanks the PR"
  );
  assert_eq!(app.rich_yank_body().as_deref(), Some("A description worth reading."),);

  app.rich_view_next_tab();
  assert_eq!(
    app.rich_yank_url().as_deref(),
    Some("https://example.test/issues/42"),
    "and the issue tab yanks the issue"
  );
  assert_eq!(app.rich_yank_body().as_deref(), Some("The issue body."));
}

#[test]
fn yanking_a_body_that_is_empty_says_so_instead_of_copying_nothing() {
  // A PR with no description is ordinary. Copying an empty string over
  // whatever the user had on the clipboard is the worst of the options.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut pr = rich_pr_fixture(61);
  pr.detail.body = String::new();
  app.apply_pr_fetch_result(Ok(pr));
  app.enter_rich_view();

  assert_eq!(app.rich_yank_body(), None);
  assert!(
    app.rich_yank_url().is_some(),
    "the URL is still there; only the body is missing"
  );
}

#[test]
fn merging_needs_a_pr_that_is_linked_and_fetched() {
  // Validation feedback on #551. Three states, told apart rather than
  // collapsed into one refusal, the way `enter_rich_view` tells them apart:
  // the way out differs, so the message has to.
  use gwm::tui::ConfirmKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.refresh_link();

  app.enter_confirm_merge();
  assert_eq!(app.view, View::List, "nothing linked: no modal");
  assert!(
    app.status.contains("link"),
    "the status names the way out: {}",
    app.status
  );

  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.enter_confirm_merge();
  assert_eq!(app.view, View::List, "linked but not fetched: still no modal");
  assert!(app.status.contains("fetch"), "and a different way out: {}", app.status);

  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_confirm_merge();
  assert_eq!(app.view, View::Confirm);
  assert_eq!(app.confirm_kind(), ConfirmKind::MergePr);
  let pending = app.pending_merge().expect("the modal holds the merge");
  assert_eq!(pending.number, 61);
  assert_eq!(pending.base_ref, "dev");
}

#[test]
fn a_stale_workspace_selection_cannot_merge_the_wrong_repos_pr() {
  // The guard `enter_rich_view` and `rich_view_refresh` both carry, and the
  // one that cannot be skipped here: a failed `Repository::open` for the
  // selected row leaves the link pointing at the PREVIOUSLY active repo, so
  // a merge would land a PR in a repository the user is not looking at.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.workspace_active_stale = true;

  app.enter_confirm_merge();

  assert_eq!(app.view, View::List, "no modal opens on a stale selection");
  assert!(app.pending_merge().is_none());
  assert!(
    app.status.contains("unavailable"),
    "the status says why: {}",
    app.status
  );
}

#[test]
fn dismissing_a_merge_confirmation_leaves_the_delete_flow_as_it_was() {
  // `View::Confirm` was single-purpose before this. The delete path is the
  // one with a safety countdown and a batch snapshot, and it must come back
  // to its own default rather than inherit whatever the merge left behind.
  use gwm::tui::ConfirmKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_confirm_merge();
  assert_eq!(app.confirm_kind(), ConfirmKind::MergePr);

  app.confirm_dismiss();

  assert_eq!(app.view, View::List);
  assert!(app.pending_merge().is_none());
  assert_eq!(
    app.confirm_kind(),
    ConfirmKind::DeleteWorktree,
    "the next confirmation is a delete until something says otherwise"
  );
}

#[test]
fn the_merge_confirmation_carries_the_configured_method() {
  // The method is resolved when the modal opens and fired from that
  // snapshot, so what the summary showed is what runs.
  use gwm::forge::MergeMethod;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));

  assert_eq!(
    app.config.merge_method,
    MergeMethod::Merge,
    "the default is a merge commit, which is what this repo requires"
  );
  app.config.merge_method = MergeMethod::Squash;
  app.enter_confirm_merge();
  assert_eq!(app.pending_merge().unwrap().method, MergeMethod::Squash);
}

#[test]
fn the_rich_view_has_pager_motions() {
  // Validation feedback on #551: `D` / `U` move half a window, `g` / `G`
  // jump to the ends. A description now runs to hundreds of rows, so `j`
  // sixty times was the only way across it.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut pr = rich_pr_fixture(61);
  pr.detail.body = (0..200).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
  app.apply_pr_fetch_result(Ok(pr));
  app.set_term_width(120);
  app.set_term_height(40);
  app.enter_rich_view();
  let last = app.detail_overlay.rows.len() - 1;

  let half = app.rich_half_page();
  assert!(half > 1, "half a 40-row window is a real jump, got {half}");

  app.detail_overlay.select_page_down(half);
  assert_eq!(app.detail_overlay.selected, half);
  app.detail_overlay.select_page_up(half);
  assert_eq!(app.detail_overlay.selected, 0);

  // Clamped, not wrapped: a pager stops at the ends, and wrapping would
  // lose the reader's place in a body this long.
  app.detail_overlay.select_page_up(half);
  assert_eq!(app.detail_overlay.selected, 0, "up from the top stays");
  app.detail_overlay.select_last();
  assert_eq!(app.detail_overlay.selected, last);
  app.detail_overlay.select_page_down(half);
  assert_eq!(app.detail_overlay.selected, last, "down from the bottom stays");
  app.detail_overlay.select_first();
  assert_eq!(app.detail_overlay.selected, 0);
}

#[test]
fn the_half_page_jump_follows_the_window_the_reader_sees() {
  // The distance is half of what is ON SCREEN, so it has to come from the
  // renderer's own answer rather than a second guess at the same number.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.set_term_height(40);
  let tall = app.rich_half_page();
  app.set_term_height(20);
  let short = app.rich_half_page();

  assert!(short < tall, "a shorter terminal jumps less: {short} vs {tall}");
  assert_eq!(tall, gwm::tui::detail_visible_rows(40) / 2);
  assert!(app.rich_half_page() >= 1, "never zero, or the key would be inert");
}

#[test]
fn a_failed_merge_keeps_the_modal_and_the_forges_own_words() {
  // Validation feedback: the merge modal behaves like the delete one. It
  // stays up with a loader while the work runs, and a failure leaves its
  // banner where the decision was made instead of flashing on a status bar
  // the reader may miss. The forge refuses for reasons gwm does not model,
  // so its message is the only accurate one available.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_confirm_merge();
  assert_eq!(app.view, View::Confirm);

  app.apply_merge_result(Err(
    "Pull request is not mergeable: the base branch is protected".into(),
  ));

  assert_eq!(app.view, View::Confirm, "the modal does not vanish on failure");
  assert!(
    app.merge_failure().unwrap().contains("protected"),
    "and it carries the forge's own words: {:?}",
    app.merge_failure()
  );
  assert!(app.pending_merge().is_some(), "so a retry is one keypress");
}

#[test]
fn a_successful_merge_closes_the_modal_and_forgets_the_target() {
  use gwm::tui::ConfirmKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.enter_confirm_merge();

  app.apply_merge_result(Ok(()));

  assert_eq!(app.view, View::List);
  assert!(app.pending_merge().is_none());
  assert!(app.merge_failure().is_none());
  assert_eq!(app.confirm_kind(), ConfirmKind::DeleteWorktree);
  assert!(app.status.contains("merged"), "status: {}", app.status);
}

#[test]
fn closing_the_ci_list_comes_back_to_the_view_it_was_opened_from() {
  // Validation feedback on #551. `c` is reached from inside the rich view,
  // so `Esc` there returning to the worktree table threw away where the
  // reader was: re-select the row, press `I` again, find your place.
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut pr = rich_pr_fixture(61);
  pr.checks = vec![gwm::github::PrCheck {
    name: "test (ubuntu-latest)".into(),
    outcome: gwm::github::CheckOutcome::Passing,
    url: None,
    workflow_name: None,
    started_at: None,
    completed_at: None,
  }];
  app.apply_pr_fetch_result(Ok(pr));
  app.enter_rich_view();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichPr);

  app.enter_ci_checks();
  assert_eq!(app.detail_overlay.kind, DetailKind::CiChecks);

  app.close_detail_overlay();

  assert_eq!(app.view, View::DetailOverlay, "not all the way out to the table");
  assert_eq!(app.detail_overlay.kind, DetailKind::RichPr);
  assert!(
    app
      .detail_overlay
      .rows
      .iter()
      .any(|r| r.value.contains("worth reading")),
    "and it is the same view, rebuilt from its own source"
  );
}

#[test]
fn the_ci_list_opened_from_the_table_still_closes_to_the_table() {
  // The other half: nothing to come back to when `c` was pressed on the
  // worktree table, and inventing a rich view there would be worse than
  // the bug this fixes.
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  let mut pr = rich_pr_fixture(61);
  pr.checks = vec![gwm::github::PrCheck {
    name: "test".into(),
    outcome: gwm::github::CheckOutcome::Passing,
    url: None,
    workflow_name: None,
    started_at: None,
    completed_at: None,
  }];
  app.apply_pr_fetch_result(Ok(pr));

  app.enter_ci_checks();
  app.close_detail_overlay();

  assert_eq!(app.view, View::List);
}

#[test]
fn cancelling_a_merge_started_from_the_rich_view_comes_back_to_it() {
  use gwm::tui::state::detail_overlay::DetailKind;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  app.apply_issue_fetch_result(Ok(rich_issue_fixture(42)));
  app.enter_rich_view();
  // On the issue tab by choice, which the round trip must not undo.
  app.rich_view_next_tab();
  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue);

  app.enter_confirm_merge();
  assert_eq!(app.view, View::Confirm);
  app.confirm_dismiss();

  assert_eq!(app.view, View::DetailOverlay);
  assert_eq!(
    app.detail_overlay.kind,
    DetailKind::RichIssue,
    "the chosen tab survives the round trip"
  );

  // And the pin with it: a PR landing now must not promote it away.
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));
  assert_eq!(app.detail_overlay.kind, DetailKind::RichIssue);
}

#[test]
fn a_merge_started_from_the_table_still_ends_on_the_table() {
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));

  app.enter_confirm_merge();
  app.confirm_dismiss();

  assert_eq!(app.view, View::List);
}

#[test]
fn the_merge_modal_advertises_its_own_verbs_not_the_delete_flows() {
  // Validation feedback on #551: the merge modal was showing the delete
  // flow's hint bar, so it advertised `D  branch` — a key that means
  // nothing over a merge and does nothing when pressed. It has its own
  // thing to offer instead, and could not say so.
  use gwm::forge::MergeMethod;
  use gwm::tui::HintContext;
  let (_dir, repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  gwm::github::link_pr(&repo, "feat/#42-tui-search", 61).unwrap();
  app.refresh_link();
  app.apply_pr_fetch_result(Ok(rich_pr_fixture(61)));

  app.enter_confirm_merge();
  assert_eq!(app.hint_context(), HintContext::ConfirmMerge);

  // And the verb it advertises actually does something.
  assert_eq!(app.pending_merge().unwrap().method, MergeMethod::Merge);
  app.cycle_merge_method();
  assert_eq!(app.pending_merge().unwrap().method, MergeMethod::Squash);
  app.cycle_merge_method();
  assert_eq!(app.pending_merge().unwrap().method, MergeMethod::Rebase);
  app.cycle_merge_method();
  assert_eq!(app.pending_merge().unwrap().method, MergeMethod::Merge, "it cycles");
}

#[test]
fn a_delete_confirmation_keeps_the_delete_hint_bar() {
  // The other half: routing on the kind must not take the delete flow's
  // own verb away from it.
  use gwm::tui::HintContext;
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.enter_confirm_delete();
  if app.view == View::Confirm {
    assert_eq!(app.hint_context(), HintContext::Confirm);
  }
}

#[test]
fn cycling_the_method_cannot_touch_a_delete_confirmation() {
  // The verb lives in the shared `confirm` key context, so it is reachable
  // while a DELETE modal is up. It has to be inert there rather than
  // quietly mutating a merge that is not on screen.
  let (_dir, _repo, mut app) = make_app_on_branch("feat/#42-tui-search");
  app.enter_confirm_delete();

  app.cycle_merge_method();

  assert!(app.pending_merge().is_none());
}

// ---- lists in the note editor (#557) -------------------------------------

#[test]
fn the_checkbox_chord_spawns_a_box_then_ticks_it() {
  // Ctrl-modified on purpose: the note editor reserves every unmodified
  // printable for the buffer, so a bare letter here would be swallowed
  // mid-sentence.
  let (_dir, mut app) = app_with_note_open();
  for c in "ship it".chars() {
    app.handle_note_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
  }

  app.handle_note_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec!["- [ ] ship it"]);

  app.handle_note_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec!["- [x] ship it"]);
}

#[test]
fn the_bullet_chord_marks_the_line_as_an_item() {
  let (_dir, mut app) = app_with_note_open();
  for c in "one".chars() {
    app.handle_note_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
  }

  app.handle_note_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec!["- one"]);

  // And Enter continues what the chord started, without a second chord.
  app.handle_note_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
  for c in "two".chars() {
    app.handle_note_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
  }
  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec!["- one", "- two"]);
}

#[test]
fn a_ticked_box_survives_the_round_trip_to_disk() {
  // The end of the gesture: tick, leave, and the file reads as a checklist
  // in an editor that never saw gwm.
  let (_dir, mut app) = app_with_note_open();
  for c in "check the CI".chars() {
    app.handle_note_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
  }
  app.handle_note_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
  app.handle_note_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
  let path = app.note_editor.as_ref().unwrap().path.clone();

  app.handle_note_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

  assert_eq!(std::fs::read_to_string(&path).unwrap(), "- [x] check the CI\n");
}

// ---- the note editor's normal mode (#557) --------------------------------

use gwm::tui::state::note_editor::NoteMode;

/// Open the note editor with `[tui] note_vim = true`, which is the only way
/// normal mode is reachable at all.
fn app_with_vim_note_open() -> (tempfile::TempDir, App) {
  let (dir, mut app) = make_app();
  app.config.tui.note_vim = true;
  app.list_state.select(Some(0));
  app.open_note_editor();
  (dir, app)
}

fn note_key(app: &mut App, c: char) {
  app.handle_note_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

#[test]
fn the_knob_opens_the_note_in_normal_mode() {
  let (_dir, app) = app_with_vim_note_open();
  assert_eq!(app.note_editor.as_ref().unwrap().mode, NoteMode::Normal);
}

#[test]
fn with_the_knob_on_the_motion_keys_are_verbs_not_letters() {
  let (_dir, mut app) = app_with_vim_note_open();
  app.note_editor.as_mut().unwrap().lines = vec!["one".into(), "two".into()];
  app.note_editor.as_mut().unwrap().cursor_line = 0;
  app.note_editor.as_mut().unwrap().cursor_col = 0;

  note_key(&mut app, 'j');
  note_key(&mut app, 'l');

  let editor = app.note_editor.as_ref().unwrap();
  assert_eq!(editor.lines, vec!["one", "two"], "nothing was typed");
  assert_eq!((editor.cursor_line, editor.cursor_col), (1, 1));
}

#[test]
fn with_the_knob_off_the_same_keys_are_still_letters() {
  // The #515 editor, untouched: this is what the knob defends.
  let (_dir, mut app) = app_with_note_open();
  assert_eq!(app.note_editor.as_ref().unwrap().mode, NoteMode::Insert);
  for c in "jkl".chars() {
    note_key(&mut app, c);
  }
  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec!["jkl"]);
}

#[test]
fn i_opens_insert_mode_and_the_next_keys_are_text_again() {
  let (_dir, mut app) = app_with_vim_note_open();
  note_key(&mut app, 'i');
  assert_eq!(app.note_editor.as_ref().unwrap().mode, NoteMode::Insert);
  for c in "done".chars() {
    note_key(&mut app, c);
  }
  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec!["done"]);
  assert_eq!(app.view, View::Note, "and no global verb fired on the `d`");
}

#[test]
fn esc_leaves_insert_mode_before_it_leaves_the_note() {
  // The whole reason the knob exists: with a mode, the first `Esc` is the
  // one that leaves insert, so closing takes two.
  let (_dir, mut app) = app_with_vim_note_open();
  note_key(&mut app, 'i');
  for c in "kept".chars() {
    note_key(&mut app, c);
  }
  let path = app.note_editor.as_ref().unwrap().path.clone();

  app.handle_note_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
  assert_eq!(app.view, View::Note, "the first Esc only left insert mode");
  assert_eq!(app.note_editor.as_ref().unwrap().mode, NoteMode::Normal);

  app.handle_note_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
  assert_eq!(app.view, View::List, "the second one closed it");
  assert_eq!(std::fs::read_to_string(&path).unwrap(), "kept\n");
}

#[test]
fn enter_and_backspace_are_motions_in_normal_mode() {
  // They are text keys in insert mode, so in normal mode they must not
  // edit: a Backspace that eats a character there is prose lost to a key
  // the user pressed to move.
  let (_dir, mut app) = app_with_vim_note_open();
  app.note_editor.as_mut().unwrap().lines = vec!["one".into(), "two".into()];
  app.note_editor.as_mut().unwrap().cursor_line = 0;
  app.note_editor.as_mut().unwrap().cursor_col = 2;

  app.handle_note_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
  app.handle_note_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

  let editor = app.note_editor.as_ref().unwrap();
  assert_eq!(editor.lines, vec!["one", "two"], "the buffer is untouched");
  assert_eq!((editor.cursor_line, editor.cursor_col), (1, 1));
}

#[test]
fn the_list_chords_still_work_from_normal_mode() {
  let (_dir, mut app) = app_with_vim_note_open();
  app.note_editor.as_mut().unwrap().lines = vec!["ship it".into()];
  app.note_editor.as_mut().unwrap().cursor_col = 0;

  app.handle_note_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));

  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec!["- [ ] ship it"]);
}

#[test]
fn the_mode_survives_a_trip_through_the_real_editor() {
  // `Ctrl+e` re-reads the file into a fresh buffer; landing back in insert
  // mode would leave the user typing verbs into their note.
  let (_dir, mut app) = app_with_vim_note_open();
  let path = app.note_editor.as_ref().unwrap().path.clone();
  std::fs::write(&path, "written outside\n").unwrap();

  app.reload_note_after_editor();

  let editor = app.note_editor.as_ref().unwrap();
  assert_eq!(editor.lines, vec!["written outside", ""]);
  assert_eq!(editor.mode, NoteMode::Normal, "still in normal mode");
}

#[test]
fn a_shifted_letter_reaches_normal_mode_as_its_uppercase_verb() {
  // Terminals disagree on how they report a shifted letter: legacy sends
  // `Char('G')` bare, many modern ones `Char('G')` + SHIFT, the kitty
  // protocol the base key `Char('g')` + SHIFT. `KeyStroke::new` folds all
  // three to `Char('G')` (PR #192) — routing `key.code` instead would turn
  // `G` into `g` and take every uppercase verb (`G W B E I A O`) with it.
  let (_dir, mut app) = app_with_vim_note_open();
  app.note_editor.as_mut().unwrap().lines = vec!["one".into(), "two".into()];
  app.note_editor.as_mut().unwrap().cursor_line = 0;
  app.note_editor.as_mut().unwrap().cursor_col = 0;

  app.handle_note_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SHIFT));

  let editor = app.note_editor.as_ref().unwrap();
  assert_eq!(editor.cursor_line, 1, "`Shift+g` is `G`, the last-line verb");
  assert!(editor.pending.is_none(), "and not a half-typed `gg`");
}

#[test]
fn the_arrows_keep_the_caret_on_a_character_in_normal_mode() {
  // The arrows are insert-mode movement: `End` parks one past the last
  // char, which is where typing goes. In normal mode that position has no
  // character under it, so `x` would delete nothing and `i` would insert
  // past the end of the line.
  let (_dir, mut app) = app_with_vim_note_open();
  app.note_editor.as_mut().unwrap().lines = vec!["abc".into()];
  app.note_editor.as_mut().unwrap().cursor_line = 0;
  app.note_editor.as_mut().unwrap().cursor_col = 0;

  app.handle_note_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
  assert_eq!(app.note_editor.as_ref().unwrap().cursor_col, 2, "on `c`, not past it");

  note_key(&mut app, 'x');
  assert_eq!(
    app.note_editor.as_ref().unwrap().lines,
    vec!["ab"],
    "so `x` has something to delete"
  );
}

#[test]
fn a_list_chord_leaves_the_caret_on_a_character_in_normal_mode() {
  // `Ctrl+t` on an empty line writes `- [ ] ` and parks the caret where the
  // item text goes, which is past the end. Same invariant, same fix.
  let (_dir, mut app) = app_with_vim_note_open();
  app.note_editor.as_mut().unwrap().lines = vec![String::new()];
  app.note_editor.as_mut().unwrap().cursor_col = 0;

  app.handle_note_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));

  let editor = app.note_editor.as_ref().unwrap();
  assert_eq!(editor.lines, vec!["- [ ] "]);
  assert_eq!(editor.cursor_col, 5, "the caret sits on the last char, not after it");
}

#[test]
fn a_key_that_is_not_the_pair_abandons_a_half_typed_sequence() {
  // `d` then an arrow then `d`: the second `d` must open a fresh sequence,
  // not complete the first one on the line the arrow landed on. There is no
  // undo here, so a `dd` the user did not type is prose gone for good.
  let (_dir, mut app) = app_with_vim_note_open();
  app.note_editor.as_mut().unwrap().lines = vec!["one".into(), "two".into(), "three".into()];
  app.note_editor.as_mut().unwrap().cursor_line = 0;
  app.note_editor.as_mut().unwrap().cursor_col = 0;

  note_key(&mut app, 'd');
  app.handle_note_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
  note_key(&mut app, 'd');

  assert_eq!(
    app.note_editor.as_ref().unwrap().lines,
    vec!["one", "two", "three"],
    "the arrow dropped the pending `d`"
  );
}

#[test]
fn a_chord_also_abandons_a_half_typed_sequence() {
  // Same contract for the Ctrl-modified verbs, which route past
  // `normal_key` entirely.
  let (_dir, mut app) = app_with_vim_note_open();
  app.note_editor.as_mut().unwrap().lines = vec!["one".into(), "two".into()];
  app.note_editor.as_mut().unwrap().cursor_line = 0;
  app.note_editor.as_mut().unwrap().cursor_col = 0;

  note_key(&mut app, 'd');
  app.handle_note_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
  note_key(&mut app, 'd');

  let editor = app.note_editor.as_ref().unwrap();
  assert_eq!(editor.lines, vec!["- one", "two"], "the bullet landed, the line stayed");
}

// ---- the mode ships on, and the bullet chord moved (#557, install pass) ---

#[test]
fn the_note_editor_opens_in_normal_mode_out_of_the_box() {
  // The knob flipped after the first install pass: `note_vim = false` is
  // the opt-out now, not the default. An editor whose vim keys type
  // themselves into the prose is the surface a vim user actually meets,
  // and a knob nobody knows to set is a mode nobody gets.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.open_note_editor();

  assert_eq!(app.note_editor.as_ref().unwrap().mode, NoteMode::Normal);
}

#[test]
fn esc_leaves_insert_before_it_closes_out_of_the_box() {
  // The cost of the flip, pinned: `Esc` no longer writes and closes on the
  // first press. It leaves insert, and the second press is the one that
  // saves. `note_vim = false` buys the old gesture back.
  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.open_note_editor();
  note_key(&mut app, 'i');

  app.handle_note_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
  assert_eq!(app.view, View::Note, "the first Esc only leaves insert");
  assert_eq!(app.note_editor.as_ref().unwrap().mode, NoteMode::Normal);

  app.handle_note_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
  assert!(app.note_editor.is_none(), "the second one writes and closes");
}

#[test]
fn the_bullet_chord_is_ctrl_u_because_tmux_eats_ctrl_l() {
  // `Ctrl+h` / `j` / `k` / `l` are the tmux.nvim pane-navigation set: tmux
  // consumes them unless the pane runs vim, so gwm never sees the key.
  // Measured on a real config, same class as `Ctrl+b` being the prefix.
  let (_dir, mut app) = app_with_note_open();
  app.handle_note_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec!["- "]);

  app.handle_note_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
  assert_eq!(
    app.note_editor.as_ref().unwrap().lines,
    vec!["- "],
    "and the chord tmux steals no longer toggles anything"
  );
}

#[test]
fn appending_at_the_end_of_the_line_types_past_the_last_char() {
  // `A` is the one verb that legally leaves the caret one past the end of
  // the line: it enters insert before the normal-mode clamp runs, so the
  // clamp does not pull it back onto the last character.
  let (_dir, mut app) = app_with_vim_note_open();
  app.note_editor.as_mut().unwrap().lines = vec!["abc".into()];
  app.note_editor.as_mut().unwrap().cursor_line = 0;
  app.note_editor.as_mut().unwrap().cursor_col = 0;

  note_key(&mut app, 'A');
  app.handle_note_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));

  assert_eq!(app.note_editor.as_ref().unwrap().lines, vec!["abcX"]);
}

#[test]
fn the_note_hint_context_follows_the_mode() {
  // The bar is redrawn every frame from `hint_context()`, so the mode line
  // is only ever as truthful as this mapping.
  use gwm::tui::HintContext;

  let (_dir, mut app) = make_app();
  app.list_state.select(Some(0));
  app.open_note_editor();
  assert_eq!(app.hint_context(), HintContext::NoteNormal);

  note_key(&mut app, 'i');
  assert_eq!(app.hint_context(), HintContext::NoteInsert);

  let (_dir, app) = app_with_note_open();
  assert_eq!(
    app.hint_context(),
    HintContext::Note,
    "with the mode off the #515 bar is what stays"
  );
}

#[test]
fn emptying_the_buffer_with_dd_removes_the_note_too() {
  // The discard gesture in the mode that now ships by default: `dd` on the
  // last line leaves an empty buffer, and an empty buffer is a deleted
  // note rather than a one-byte file `gwm doctor` will report later.
  let (dir, mut app) = make_app();
  app.list_state.select(Some(0));
  let branch = app.selected().unwrap().branch.clone().unwrap();
  let path = gwm::notes::prepare(&git2::Repository::open(dir.path()).unwrap(), &branch)
    .unwrap()
    .unwrap();
  std::fs::write(&path, "old prose\n").unwrap();

  app.open_note_editor();
  // Twice: the file's trailing newline is a blank last line, and that is
  // the line the caret opens on. Both go before the buffer reads empty.
  for _ in 0..2 {
    note_key(&mut app, 'd');
    note_key(&mut app, 'd');
  }
  app.handle_note_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

  assert!(!path.exists(), "an emptied note is removed, not blanked");
}

/// Rebind `action` to `chords` on a live `App`, the way a `[tui.keys]`
/// override would. Used by the #613 toggle tests, which are exactly about
/// what a rebind does to the dispatch.
fn rebind(app: &mut App, action: Action, chords: &[&str]) {
  use gwm::tui::keymap::KeyStroke;
  let parsed: Vec<Vec<KeyStroke>> = chords.iter().map(|c| KeyStroke::parse_chord(c).unwrap()).collect();
  app.keymap.apply_override(action, parsed).unwrap();
}

/// Flatten a rendered sidebar/modal line into its plain text.
fn line_text(l: &ratatui::text::Line<'_>) -> String {
  l.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn working_tree_modal_snapshots_the_dirty_tree_on_open() {
  // Issue #592: `5` opens the Working Tree listing full size. The snapshot
  // is taken AT OPEN, not read from the sidebar cache — the sidebar is a
  // hidden pane here (`open = false`), the state in which that cache is
  // never rebuilt, and the overlay must still show the change set.
  let (dir, mut app) = make_app();
  std::fs::write(dir.path().join("scratch.rs"), "fn main() {}\n").unwrap();
  app.sidebar.open = false;

  app.enter_working_tree();

  assert_eq!(app.view, View::WorkingTree);
  let text: Vec<String> = app.working_tree.lines.iter().map(line_text).collect();
  assert!(
    text.iter().any(|l| l.contains("scratch.rs")),
    "the untracked file is listed — got {text:?}"
  );
  assert_eq!(app.working_tree.scroll, 0, "a fresh open starts at the top");
  assert_eq!(
    app.working_tree.counts.created, 1,
    "the footer counts come with the snapshot — got {:?}",
    app.working_tree.counts
  );
}

#[test]
fn the_working_tree_open_key_is_also_what_closes_it() {
  // `5` toggles. The dispatch resolves the toggle before the modal verbs
  // and against the action alone, so this is the pin for the default: one
  // stroke, `Fired`.
  let (_dir, mut app) = make_app();
  app.enter_working_tree();

  assert_eq!(
    app.modal_toggle_stroke(
      KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE),
      gwm::tui::keymap::Action::WorkingTree
    ),
    ToggleStroke::Fired
  );
}

#[test]
fn a_multi_stroke_toggle_closes_the_overlay_it_opened() {
  // Issue #613, first hole: the old guard asked `key_matches_action`, which
  // looks up ONE stroke, so `working_tree = ["g w"]` opened the overlay
  // through the chord-aware list dispatch and then had no way to shut it.
  // The prefix must be consumed (`Pending`), not handed to the modal verbs,
  // or `g` jumps the listing to the top on its way through.
  let (_dir, mut app) = make_app();
  rebind(&mut app, Action::WorkingTree, &["g w"]);
  app.enter_working_tree();

  assert_eq!(
    app.modal_toggle_stroke(
      KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
      Action::WorkingTree
    ),
    ToggleStroke::Pending
  );
  assert_eq!(
    app.modal_toggle_stroke(
      KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
      Action::WorkingTree
    ),
    ToggleStroke::Fired
  );
}

#[test]
fn a_stray_prefix_stroke_does_not_arm_a_phantom_toggle() {
  // The other half of the chord contract: a `g` that is not followed by the
  // toggle's own continuation must drop the buffer, or the next unrelated
  // stroke would complete a chord the user never typed.
  let (_dir, mut app) = make_app();
  rebind(&mut app, Action::WorkingTree, &["g w"]);
  app.enter_working_tree();

  assert_eq!(
    app.modal_toggle_stroke(
      KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
      Action::WorkingTree
    ),
    ToggleStroke::Pending
  );
  assert_eq!(
    app.modal_toggle_stroke(
      KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
      Action::WorkingTree
    ),
    ToggleStroke::Unclaimed,
    "`j` is not the continuation, so it falls through to the modal verbs"
  );
  assert!(app.pending_chord_is_empty(), "the half-typed prefix is dropped");
  assert_eq!(
    app.modal_toggle_stroke(
      KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
      Action::WorkingTree
    ),
    ToggleStroke::Unclaimed,
    "a bare `w` must not complete the chord the dropped `g` started"
  );
}

#[test]
fn a_toggle_rebound_onto_a_modal_verbs_key_still_closes() {
  // Issue #613, second hole: the guard used to run AFTER the modal
  // resolution, so `working_tree = ["j"]` opened the overlay and then
  // scrolled it. The toggle wins now, which is what the user asked for by
  // binding it there.
  let (_dir, mut app) = make_app();
  rebind(&mut app, Action::WorkingTree, &["j"]);
  app.enter_working_tree();

  assert_eq!(
    app.modal_toggle_stroke(
      KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
      Action::WorkingTree
    ),
    ToggleStroke::Fired
  );
  // `k` is untouched: only the rebound key is taken from the context.
  assert_eq!(
    app.modal_toggle_stroke(
      KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
      Action::WorkingTree
    ),
    ToggleStroke::Unclaimed
  );
}

#[test]
fn a_rebound_toggle_beats_the_scroll_verb_it_shadows() {
  // The precedence itself, not just the resolver: `handle_working_tree_key`
  // asks the toggle first. With `working_tree = ["j"]`, `j` closes and the
  // listing does NOT scroll. Put the modal resolution back in front and the
  // scroll assertion below goes red.
  let (_dir, mut app) = make_app();
  rebind(&mut app, Action::WorkingTree, &["j"]);
  app.enter_working_tree();
  app.working_tree.max_scroll = 10;

  let close = app.handle_working_tree_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));

  assert!(close, "the rebound toggle closes the overlay");
  assert_eq!(app.working_tree.scroll, 0, "and the shadowed scroll verb never ran");
}

#[test]
fn an_unclaimed_key_still_reaches_the_modal_verbs() {
  // The other side of that precedence: taking the toggle first must not
  // swallow the rest of the context.
  let (_dir, mut app) = make_app();
  app.enter_working_tree();
  app.working_tree.max_scroll = 10;

  assert!(!app.handle_working_tree_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)));
  assert_eq!(app.working_tree.scroll, 1);
  assert!(app.handle_working_tree_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
}

#[test]
fn a_chord_prefix_does_not_reach_the_scroll_verbs() {
  // `working_tree = ["g w"]`: the `g` is consumed as a prefix. Were it
  // handed to the modal verbs it would fire `scroll_top`, so the listing
  // would jump to the top on the way to closing.
  let (_dir, mut app) = make_app();
  rebind(&mut app, Action::WorkingTree, &["g w"]);
  app.enter_working_tree();
  app.working_tree.max_scroll = 10;
  app.working_tree.scroll = 7;

  assert!(!app.handle_working_tree_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)));
  assert_eq!(app.working_tree.scroll, 7, "the prefix did not fire `scroll_top`");
  assert!(app.handle_working_tree_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)));
}

#[test]
fn the_modal_toggle_never_fires_another_global_action() {
  // The reason this is not `dispatch_key`: inside an overlay the toggle is
  // the ONE global binding allowed through. `d` would otherwise open the
  // delete confirm from behind the modal.
  let (_dir, mut app) = make_app();
  app.enter_working_tree();

  for c in ['d', 'n', 'q', 'x', '3', '4'] {
    assert_eq!(
      app.modal_toggle_stroke(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), Action::WorkingTree),
      ToggleStroke::Unclaimed,
      "`{c}` is not the working_tree toggle and must not resolve here"
    );
  }
}

#[test]
fn every_overlay_that_advertises_a_toggle_resolves_one() {
  // The three overlays whose docs promise "the open key closes it too"
  // (issue #613 fixed all of them at once, not just #592's). Enumerated so
  // a fourth overlay copying the shape has a place to declare itself.
  let (_dir, mut app) = make_app();
  for (action, chord, ch) in [
    (Action::CommandLogs, "3", '3'),
    (Action::ConfigPanel, "4", '4'),
    (Action::WorkingTree, "5", '5'),
  ] {
    assert_eq!(
      app.keymap.primary_chord(action).as_deref(),
      Some(chord),
      "{action:?} default chord moved; update this table"
    );
    assert_eq!(
      app.modal_toggle_stroke(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), action),
      ToggleStroke::Fired
    );
  }
}

#[test]
fn working_tree_modal_reports_a_clean_tree() {
  // The empty-state: `init_repo` leaves no dirty file, so the overlay says
  // so rather than rendering a blank canvas.
  let (_dir, mut app) = make_app();

  app.enter_working_tree();

  let text: Vec<String> = app.working_tree.lines.iter().map(line_text).collect();
  assert!(
    text.iter().any(|l| l.contains("clean")),
    "a clean worktree gets the clean row — got {text:?}"
  );
}

#[test]
fn reopening_the_working_tree_modal_rewinds_the_scroll_and_resnapshots() {
  // Two invariants in one gesture: a stale scroll offset from the previous
  // visit does not survive the re-open (the `enter_help` / `enter_command_logs`
  // contract), and the listing is re-read, so a file created while the
  // overlay was closed shows up on the next open.
  let (dir, mut app) = make_app();
  app.enter_working_tree();
  app.working_tree.max_scroll = 40;
  app.working_tree.scroll = 12;

  std::fs::write(dir.path().join("late.rs"), "// added after the first open\n").unwrap();
  app.enter_working_tree();

  assert_eq!(app.working_tree.scroll, 0);
  let text: Vec<String> = app.working_tree.lines.iter().map(line_text).collect();
  assert!(
    text.iter().any(|l| l.contains("late.rs")),
    "the re-open re-reads the tree — got {text:?}"
  );
}

#[test]
fn the_working_tree_modal_scroll_clamps_to_the_published_bound() {
  // Same scroll contract as the help overlay: the renderer publishes
  // `max_scroll` against the live viewport and the cursor never passes it.
  let (_dir, mut app) = make_app();
  app.enter_working_tree();
  app.working_tree.max_scroll = 2;

  for _ in 0..5 {
    app.working_tree.scroll_down();
  }
  assert_eq!(app.working_tree.scroll, 2);

  app.working_tree.scroll_to_top();
  assert_eq!(app.working_tree.scroll, 0);
  app.working_tree.scroll_up();
  assert_eq!(app.working_tree.scroll, 0, "the top is a floor, not a wrap");
  app.working_tree.scroll_to_bottom();
  assert_eq!(app.working_tree.scroll, 2);
}
