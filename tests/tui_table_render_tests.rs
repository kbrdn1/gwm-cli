//! Render-level tests for the worktrees table's AGENT column (issue #408,
//! Codex review round D): the column must be **conditional** — a user with
//! no agent tooling installed keeps the exact pre-feature table (no header,
//! no 8-cell constraint squeezing NAME/BRANCH/PATH on narrow terminals),
//! and the column appears once a detection snapshot carries any session.
//! Same TestBackend approach as `tui_sidebar_render_tests.rs`.

use gwm::tui::{draw, App, TaskKind};
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;
use tempfile::TempDir;

/// A minimal one-commit repo the App can open.
fn repo() -> TempDir {
  let dir = TempDir::new().unwrap();
  let repo = git2::Repository::init(dir.path()).unwrap();
  repo.set_head("refs/heads/main").ok();
  let sig = git2::Signature::now("gwm-test", "gwm@test").unwrap();
  std::fs::write(dir.path().join("file.txt"), "seed").unwrap();
  repo.index().unwrap().add_path(Path::new("file.txt")).unwrap();
  repo.index().unwrap().write().unwrap();
  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
  dir
}

/// Flatten a `TestBackend` buffer into one string so `contains` can look
/// for a substring regardless of styling.
fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
  terminal
    .backend()
    .buffer()
    .content()
    .iter()
    .map(|c| c.symbol())
    .collect()
}

fn draw_once(app: &mut App) -> String {
  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, app)).unwrap();
  buffer_text(&terminal)
}

#[test]
fn agent_column_is_hidden_while_no_session_is_detected() {
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let text = draw_once(&mut app);
  assert!(
    !text.contains("AGENT"),
    "no detected session -> the table must stay visually pre-#408"
  );
}

#[test]
fn agent_column_appears_once_a_session_lands() {
  use gwm::agent_sessions::{AgentKind, AgentSession, WorktreeAgents};
  use std::collections::BTreeMap;
  use std::time::{Duration, SystemTime};

  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let path = app.worktrees[0].path.to_string_lossy().to_string();
  let mut map = BTreeMap::new();
  map.insert(
    path.clone(),
    WorktreeAgents {
      sessions: vec![AgentSession {
        kind: AgentKind::ClaudeCode,
        cwd: std::path::PathBuf::from(&path),
        last_activity: SystemTime::now() - Duration::from_secs(5),
        ended: false,
        id: "render-test-session".into(),
        name: None,
      }],
    },
  );
  let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
  assert!(app.apply_agent_snapshot(generation, map, Vec::new()));

  let text = draw_once(&mut app);
  assert!(text.contains("AGENT"), "a landed session must surface the column");
  assert!(text.contains("claude"), "the top session's agent must show");
}
