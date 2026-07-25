//! State-machine tests for the TUI workspace mode (issue #36). Ratatui-free:
//! they pin the merged-list construction, the per-row repo mapping, and the
//! swap-on-navigation that keeps `App`'s active repo (`repo`/`workdir`/
//! `repo_name`/`config`) aligned with the selected worktree's repo — without
//! ever drawing a frame.

use git2::{Repository, Signature};
use gwm::tui::keymap::Action;
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
fn sync_active_repo_reapplies_both_sidebar_knobs_from_the_selected_repo() {
  // Codex review #366 P2: `sync_active_repo` swapped `self.config` but left the
  // live sidebar on the previous repo's layout, so a per-repo `[tui]` sidebar
  // override was ignored until a reload or relaunch. The finding named
  // `sidebar_orientation` (#365, new); `sidebar_position` had the identical hole
  // since workspace mode landed (#36). Both are pinned here — the fix is one
  // shared apply, so one test guards both against drift.
  use gwm::config::{SidebarOrientation, SidebarPosition};

  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("alpha"));
  init_repo_at(&root.path().join("beta"));
  fs::write(
    root.path().join("beta").join(".gwm.toml"),
    "[tui]\nsidebar_orientation = \"side-by-side\"\nsidebar_position = \"left\"\n",
  )
  .unwrap();

  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  // alpha (row 0) has no override → the built-in defaults.
  assert_eq!(app.sidebar.orientation, SidebarOrientation::Stacked);
  assert_eq!(app.sidebar.position, SidebarPosition::Right);

  let last = app.worktrees.len() - 1; // beta's main
  app.list_state.select(Some(last));
  app.sync_active_repo();
  assert_eq!(app.repo_name, "beta", "swapped to beta");
  assert_eq!(
    app.sidebar.orientation,
    SidebarOrientation::SideBySide,
    "orientation follows the newly-active repo's config"
  );
  assert_eq!(
    app.sidebar.position,
    SidebarPosition::Left,
    "position follows the newly-active repo's config"
  );

  // ...and swapping back restores alpha's defaults rather than sticking on beta.
  app.list_state.select(Some(0));
  app.sync_active_repo();
  assert_eq!(app.repo_name, "alpha", "swapped back to alpha");
  assert_eq!(app.sidebar.orientation, SidebarOrientation::Stacked);
  assert_eq!(app.sidebar.position, SidebarPosition::Right);
}

#[test]
fn failed_repo_activation_marks_the_selection_stale_then_recovers() {
  // A repo that was listed but then vanished on disk must not silently leave
  // the active context pointing at the previous repo: the selection is flagged
  // stale (which blocks repo-mutating actions), and navigating back to a live
  // repo clears it (issue #304).
  let root = workspace_root(); // alpha, beta
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  assert!(!app.workspace_active_stale, "fresh workspace is not stale");

  // Delete beta's checkout on disk, then select its (now-dead) row.
  fs::remove_dir_all(root.path().join("beta")).unwrap();
  let last = app.worktrees.len() - 1;
  app.list_state.select(Some(last));
  app.sync_active_repo();

  assert!(app.workspace_active_stale, "an unreachable selected repo is stale");
  assert_eq!(
    app.repo_name, "alpha",
    "the active repo stays on the last live one, not the dead beta"
  );

  // Navigating back to a reachable repo clears the stale flag.
  app.list_state.select(Some(0));
  app.sync_active_repo();
  assert!(
    !app.workspace_active_stale,
    "selecting a live repo clears the stale flag"
  );
}

#[test]
fn going_stale_closes_the_open_ci_checks_overlay() {
  // Codex review #455 (P2): a relist that marks the selection stale while
  // the CI checks overlay is up used to leave it open — every verb, Enter
  // opening a check URL included, then acted on the previously active
  // repo. Every stale transition closes the overlay at the source.
  use gwm::tui::state::detail_overlay::{DetailKind, DetailRole, DetailRow};
  let root = workspace_root(); // alpha, beta
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  app.detail_overlay.open(
    DetailKind::CiChecks,
    "CI Checks".into(),
    vec![DetailRow {
      label: "✓".into(),
      value: "stale-check".into(),
      role: DetailRole::Success,
      meta: None,
      extra: None,
    }],
  );
  app.view = gwm::tui::View::DetailOverlay;

  fs::remove_dir_all(root.path().join("beta")).unwrap();
  let last = app.worktrees.len() - 1;
  app.list_state.select(Some(last));
  app.sync_active_repo();

  assert!(app.workspace_active_stale, "the dead selection is stale");
  assert_eq!(
    app.view,
    gwm::tui::View::List,
    "going stale closes the CI checks overlay"
  );
}

#[test]
fn no_visible_selection_marks_workspace_stale() {
  // When the filter hides every row (no selection), workspace mode has no
  // active repo the selection points at, so write actions must be blocked:
  // `sync_active_repo` flags the state stale (issue #304).
  let root = workspace_root();
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  assert!(!app.workspace_active_stale, "a fresh selection is not stale");

  app.list_state.select(None);
  app.sync_active_repo();
  assert!(
    app.workspace_active_stale,
    "no selected row → stale (blocks create/etc.)"
  );

  // Restoring a selection clears it again.
  app.list_state.select(Some(0));
  app.sync_active_repo();
  assert!(!app.workspace_active_stale, "a valid selection clears the stale flag");
}

#[test]
fn stale_selection_blocks_project_config_edits() {
  // With a stale selection (the selected repo vanished), a Project-layer config
  // edit would write to the previously active repo's `.gwm.toml` — refuse it
  // (the wrong-target write the guard prevents); #304.
  let root = workspace_root();
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  let before = app.config.tui.auto_refresh_secs;

  fs::remove_dir_all(root.path().join("beta")).unwrap();
  let last = app.worktrees.len() - 1;
  app.list_state.select(Some(last));
  app.sync_active_repo();
  assert!(app.workspace_active_stale, "precondition: selection is stale");

  // config_panel defaults to the Project layer.
  app.apply_setting(SettingField::AutoRefreshSecs, "42");
  assert_eq!(
    app.config.tui.auto_refresh_secs, before,
    "a Project-layer edit is refused while the selected repo is unavailable"
  );
  assert!(
    app.status.contains("unavailable"),
    "the refusal is surfaced on the status bar, got: {}",
    app.status
  );
}

#[test]
fn repo_mutating_actions_are_classified() {
  // The guard in `run_action` keys off this classification (#304).
  assert!(Action::Create.is_repo_mutating());
  assert!(Action::DeleteConfirm.is_repo_mutating());
  assert!(Action::Bootstrap.is_repo_mutating());
  assert!(Action::EditWorktree.is_repo_mutating());
  assert!(Action::LinkPrompt.is_repo_mutating());
  // Navigation / read-only launchers are not blocked.
  assert!(!Action::Down.is_repo_mutating());
  assert!(!Action::Refresh.is_repo_mutating());
  assert!(!Action::YankPath.is_repo_mutating());
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

// ---- async workspace re-list (issue #343) --------------------------------
// `maybe_auto_refresh` / `request_refresh` used to call `refresh_workspace`
// synchronously in workspace mode, freezing the event loop while every repo
// was opened + listed. They now ride `TaskKind::RefreshWorkspace`. These pin
// the async contract deterministically — claim the slot / inject a
// `TaskMsg::RefreshWorkspace` / drain, never a real OS worker thread.

#[test]
fn drain_applies_an_async_workspace_relist() {
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let root = workspace_root();
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  let last = app.worktrees.len() - 1;
  assert_eq!(app.row_repo_name(0), Some("alpha"));
  assert_eq!(app.row_repo_name(last), Some("beta"));

  // Claim the slot exactly as `request_refresh` would, then deliver a worker
  // payload that re-tags every row to repo 0 (alpha) — no OS thread.
  let generation = app.tasks.request(TaskKind::RefreshWorkspace).unwrap();
  let rows: Vec<_> = app.worktrees.iter().cloned().map(|w| (w, 0usize)).collect();
  app
    .task_result_sender()
    .send(TaskMsg::RefreshWorkspace(generation, rows))
    .unwrap();
  assert!(app.drain_task_results(), "drain applies the workspace re-list");

  assert_eq!(app.row_repo_name(0), Some("alpha"));
  assert_eq!(
    app.row_repo_name(last),
    Some("alpha"),
    "the drain rebuilt the row→repo map from the worker's payload"
  );
  assert!(
    !app.tasks.is_loading(TaskKind::RefreshWorkspace),
    "the slot is cleared once the result applies"
  );
}

#[test]
fn request_refresh_in_workspace_mode_coalesces_onto_an_inflight_relist() {
  // Discriminates the async path from the old synchronous one: the pre-#343
  // code called `refresh()`, which *invalidates* the slot (freeing it); the
  // async path calls `request(RefreshWorkspace)`, which coalesces (`None`) and
  // leaves the in-flight run's slot held.
  use gwm::tui::state::async_task::TaskKind;
  let root = workspace_root();
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  let _generation = app.tasks.request(TaskKind::RefreshWorkspace).unwrap();

  app.request_refresh(); // a second `r` — coalesces, no second worker

  assert!(
    app.tasks.is_loading(TaskKind::RefreshWorkspace),
    "the in-flight workspace re-list is still the one and only run"
  );
}

#[test]
fn refresh_invalidates_an_inflight_async_workspace_relist() {
  // A synchronous `refresh()` (post-mutation) produces authoritative state, so
  // an in-flight async workspace re-list is stale — its late payload must be
  // dropped, not applied on top of the fresh list.
  use gwm::tui::state::async_task::{TaskKind, TaskMsg};
  let root = workspace_root();
  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  let stale = app.tasks.request(TaskKind::RefreshWorkspace).unwrap();

  app.refresh().unwrap();

  // The stale worker reports late, re-tagging every row to repo 0.
  let rows: Vec<_> = app.worktrees.iter().cloned().map(|w| (w, 0usize)).collect();
  app
    .task_result_sender()
    .send(TaskMsg::RefreshWorkspace(stale, rows))
    .unwrap();
  app.drain_task_results();

  let last = app.worktrees.len() - 1;
  assert_eq!(
    app.row_repo_name(last),
    Some("beta"),
    "the superseded workspace payload was dropped — the fresh map stands"
  );
}

#[test]
fn workspace_pins_are_read_from_each_rows_own_repo() {
  // Codex review round I (P2): a session pinned in a child repo was
  // invisible in TUI workspace mode — `read_agent_pins` returned an empty
  // map instead of opening each row's owning repo, so the Agents pane
  // (pinned-only) never showed it.
  let root = workspace_root();
  let alpha = Repository::open(root.path().join("alpha")).unwrap();
  gwm::github::add_agent_pin(&alpha, "main", "sid-ws-alpha").unwrap();

  let app = App::new_workspace_at_layered(root.path(), None).unwrap();
  // Round P moved the periodic read into the detection worker; the same
  // sources + reader pair is exercised here without spawning the thread.
  let pins = gwm::tui::read_pins_from_sources(&app.agent_pin_sources());

  let alpha_row = app
    .worktrees
    .iter()
    .find(|w| w.path.ends_with("alpha"))
    .expect("alpha's main checkout is a row");
  let key = alpha_row.path.to_string_lossy().to_string();
  assert_eq!(
    pins.get(&key).map(Vec::as_slice),
    Some(&["sid-ws-alpha".to_string()][..]),
    "the pin set in alpha's branch config reaches the per-path map"
  );
}

#[test]
fn overlay_pin_markers_survive_an_active_repo_swap() {
  // Codex review round N (P2): the overlay's pinned markers were read
  // through `self.repo` — the ACTIVE repo. In workspace mode an
  // auto-refresh can move the selection (and swap the active repo) while
  // the overlay stays bound to its captured worktree; the markers then
  // came from the WRONG repo's branch config (both repos here share the
  // branch name `main`). They must come from the captured worktree's
  // owning repo.
  use gwm::tui::state::async_task::TaskKind;

  let root = workspace_root();
  let alpha = Repository::open(root.path().join("alpha")).unwrap();
  gwm::github::add_agent_pin(&alpha, "main", "sid-alpha-pin").unwrap();

  let mut app = App::new_workspace_at_layered(root.path(), None).unwrap();
  app.list_state.select(Some(0));
  app.sync_active_repo();
  app.open_agent_overlay(); // captures alpha's main-checkout row

  // Drift: the selection moves to beta's row -> the active repo swaps.
  let last = app.worktrees.len() - 1;
  app.list_state.select(Some(last));
  app.sync_active_repo();
  assert_eq!(app.repo_name, "beta", "precondition: the active repo drifted");

  // A detection snapshot lands and refreshes the overlay in place; the
  // pinned session resolves for alpha's path, so its row must carry the
  // pinned marker even though the active repo is beta now.
  let alpha_path = app.worktrees[0].path.clone();
  let mut map = std::collections::BTreeMap::new();
  map.insert(
    alpha_path.to_string_lossy().to_string(),
    gwm::agent_sessions::WorktreeAgents {
      sessions: vec![gwm::agent_sessions::AgentSession {
        kind: gwm::agent_sessions::AgentKind::ClaudeCode,
        cwd: alpha_path,
        last_activity: std::time::SystemTime::now(),
        ended: false,
        id: "sid-alpha-pin".into(),
        name: None,
      }],
    },
  );
  // The worker reads pins from each row's OWNING repo (round P): even
  // with the active repo drifted to beta, alpha's pin is in the sources.
  let pins = gwm::tui::read_pins_from_sources(&app.agent_pin_sources());
  let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
  assert!(app.apply_agent_snapshot(generation, map, None, pins));
  assert!(
    app.detail_overlay.rows.iter().any(|r| r.value.contains("pinned")),
    "the pin from alpha's own repo marks the row: {:?}",
    app
      .detail_overlay
      .rows
      .iter()
      .map(|r| r.value.clone())
      .collect::<Vec<_>>()
  );
}
