//! State-machine tests for the TUI workspace mode (issue #36). Ratatui-free:
//! they pin the merged-list construction, the per-row repo mapping, and the
//! swap-on-navigation that keeps `App`'s active repo (`repo`/`workdir`/
//! `repo_name`/`config`) aligned with the selected worktree's repo — without
//! ever drawing a frame.

use git2::{Repository, Signature};
use gwm::tui::{draw, App, SettingField, SettingsLayer};
use ratatui::{backend::TestBackend, Terminal};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Flatten a `TestBackend` buffer into one string of cell symbols.
fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
  terminal
    .backend()
    .buffer()
    .content()
    .iter()
    .map(|c| c.symbol())
    .collect()
}

/// Init a git repo at `path` (created if missing) on `main` with one commit.
fn init_repo_at(path: &Path) {
  fs::create_dir_all(path).unwrap();
  let repo = Repository::init(path).unwrap();
  repo.set_head("refs/heads/main").ok();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  let tree_id = {
    let mut index = repo.index().unwrap();
    index.write_tree().unwrap()
  };
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
}

/// A workspace root with two child repos (alpha, beta) plus a non-repo dir.
fn workspace_root() -> TempDir {
  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("alpha"));
  init_repo_at(&root.path().join("beta"));
  fs::create_dir_all(root.path().join("notes")).unwrap();
  root
}

#[test]
fn workspace_app_builds_a_merged_list_across_repos() {
  let root = workspace_root();
  let app = App::new_workspace_at_layered(root.path(), None).unwrap();

  assert!(app.is_workspace(), "the app is in workspace mode");
  // Both repos contribute their main worktree → at least two rows.
  assert!(
    app.worktrees.len() >= 2,
    "merged list spans both repos: {}",
    app.worktrees.len()
  );
  // Every row maps to a repo name; alpha sorts first, so row 0 is alpha.
  assert_eq!(app.row_repo_name(0), Some("alpha"), "row 0 belongs to alpha");
  let last = app.worktrees.len() - 1;
  assert_eq!(app.row_repo_name(last), Some("beta"), "the last row belongs to beta");
}

#[test]
fn workspace_app_starts_active_on_the_first_repo() {
  let root = workspace_root();
  let app = App::new_workspace_at_layered(root.path(), None).unwrap();
  assert_eq!(app.repo_name, "alpha", "active repo starts on the first row's repo");
  assert!(
    app.workdir.ends_with("alpha"),
    "active workdir points at alpha, got {:?}",
    app.workdir
  );
}

#[test]
fn sync_active_repo_follows_the_selection_across_repos() {
  let root = workspace_root();
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();

  // Select the last row (beta's main) and let the swap fire.
  let last = app.worktrees.len() - 1;
  app.list_state.select(Some(last));
  app.sync_active_repo();

  assert_eq!(app.repo_name, "beta", "active repo follows the selection to beta");
  assert!(
    app.workdir.ends_with("beta"),
    "active workdir swapped to beta, got {:?}",
    app.workdir
  );

  // Selecting back to row 0 swaps the active repo back to alpha.
  app.list_state.select(Some(0));
  app.sync_active_repo();
  assert_eq!(app.repo_name, "alpha", "active repo swaps back to alpha");
}

#[test]
fn sync_active_repo_reresolves_branch_types_from_the_selected_repo() {
  // beta declares its own `[[branch_types]]`; alpha uses the defaults. After
  // the active repo swaps to beta, the create form's branch types must follow
  // beta's config, not stay on alpha's defaults (Codex review #303 P2).
  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("alpha"));
  init_repo_at(&root.path().join("beta"));
  fs::write(
    root.path().join("beta").join(".gwm.toml"),
    "[[branch_types]]\nname = \"wibble\"\ndescription = \"custom\"\n",
  )
  .unwrap();

  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  // alpha (row 0) → default types include `feat`, and not `wibble`.
  assert!(
    app.branch_types.iter().any(|t| t.name == "feat"),
    "alpha uses default branch types"
  );

  let last = app.worktrees.len() - 1; // beta's main
  app.list_state.select(Some(last));
  app.sync_active_repo();
  assert_eq!(app.repo_name, "beta", "swapped to beta");
  let names: Vec<&str> = app.branch_types.iter().map(|t| t.name.as_str()).collect();
  assert_eq!(
    names,
    vec!["wibble"],
    "branch types now follow beta's config, got {names:?}"
  );
}

#[test]
fn sync_active_repo_is_a_noop_in_single_repo_mode() {
  // A non-workspace App must be unaffected by the swap hook.
  let (dir, _repo) = {
    let dir = TempDir::new().unwrap();
    init_repo_at(dir.path());
    (dir, ())
  };
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  assert!(!app.is_workspace(), "single-repo app is not in workspace mode");
  let before = app.workdir.clone();
  app.sync_active_repo();
  assert_eq!(app.workdir, before, "sync is inert without a workspace");
}

#[test]
fn workspace_list_renders_a_repo_column_with_repo_names() {
  let root = workspace_root();
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();

  let backend = TestBackend::new(140, 30);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  let text = buffer_text(&terminal);
  assert!(
    text.contains("REPO"),
    "the list header carries a REPO column, got:\n{text}"
  );
  assert!(
    text.contains("alpha"),
    "the alpha repo name renders in a row, got:\n{text}"
  );
  assert!(
    text.contains("beta"),
    "the beta repo name renders in a row, got:\n{text}"
  );
}

#[test]
fn workspace_settings_edit_survives_a_repo_swap_roundtrip() {
  // Editing a setting in workspace mode must update the active repo's *cached*
  // config too, not just `self.config` — otherwise navigating away and back
  // restores the stale cached config and silently reverts the edit (Codex
  // review #303 P3).
  let root = workspace_root();
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  assert_eq!(app.repo_name, "alpha");

  // Edit alpha's auto-refresh seconds via the Settings panel (Project layer).
  app.apply_setting(SettingField::AutoRefreshSecs, "99");
  assert_eq!(app.config.tui.auto_refresh_secs, 99, "edit applies live");

  // Navigate to beta and back to alpha.
  let last = app.worktrees.len() - 1;
  app.list_state.select(Some(last));
  app.sync_active_repo();
  assert_eq!(app.repo_name, "beta");
  app.list_state.select(Some(0));
  app.sync_active_repo();
  assert_eq!(app.repo_name, "alpha");

  assert_eq!(
    app.config.tui.auto_refresh_secs, 99,
    "the settings edit survived the repo-swap round-trip"
  );
}

#[test]
fn workspace_global_setting_edit_propagates_to_every_repo() {
  // A Global-layer edit changes the deep-merged config for *every* repo. After
  // it, navigating to another repo must see the new value, not the config that
  // repo was loaded with at startup (Codex review #303 P2).
  let root = workspace_root();
  let global = root.path().join("global-config.toml");
  fs::write(&global, "").unwrap();
  let mut app = App::new_workspace_at_layered(root.path(), Some(&global)).unwrap();

  app.config_panel.layer = SettingsLayer::Global;
  app.apply_setting(SettingField::AutoRefreshSecs, "77");
  assert_eq!(
    app.config.tui.auto_refresh_secs, 77,
    "global edit applies to the active repo"
  );

  // Navigate to beta: its cached config must reflect the global edit too.
  let last = app.worktrees.len() - 1;
  app.list_state.select(Some(last));
  app.sync_active_repo();
  assert_eq!(app.repo_name, "beta");
  assert_eq!(
    app.config.tui.auto_refresh_secs, 77,
    "the global edit reached the other repo's cached config"
  );
}

#[test]
fn workspace_refresh_rebuilds_the_full_merged_list() {
  let root = workspace_root();
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  let before = app.worktrees.len();

  // A refresh in workspace mode must re-list EVERY repo, not just the active
  // one — the merged list keeps its full span and the row→repo map stays in
  // sync (last row still beta).
  app.refresh().unwrap();
  assert_eq!(
    app.worktrees.len(),
    before,
    "merged list keeps spanning every repo after refresh"
  );
  let last = app.worktrees.len() - 1;
  assert_eq!(
    app.row_repo_name(last),
    Some("beta"),
    "row→repo map survives the refresh"
  );
}
