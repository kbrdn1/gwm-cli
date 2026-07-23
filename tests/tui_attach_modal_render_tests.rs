//! Render-level tests for the attach-by-id prompt of the agent detail
//! overlay (issue #445): the modal's frame must keep a FIXED height while
//! the filter narrows or widens the candidate list (no resize on each
//! keystroke), and an overflowing candidate list must show a scrollbar —
//! the two conventions the sibling detail mode already follows. Same
//! `TestBackend` approach as `tui_table_render_tests.rs`.

use gwm::agent_sessions::{AgentKind, AgentSession};
use gwm::tui::{draw, App, TaskKind};
use ratatui::{backend::TestBackend, Terminal};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime};
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

/// One pool session with a distinctive id.
fn session(i: usize) -> AgentSession {
  AgentSession {
    kind: AgentKind::ClaudeCode,
    cwd: std::path::PathBuf::from("/x/elsewhere"),
    last_activity: SystemTime::now() - Duration::from_secs(30),
    ended: false,
    id: format!("aaa-{i:02}"),
    name: None,
  }
}

/// An App with the attach prompt open over a `count`-session pool.
fn app_with_open_prompt(dir: &TempDir, count: usize) -> App {
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
  let pool: Vec<AgentSession> = (0..count).map(session).collect();
  assert!(app.apply_agent_snapshot(generation, BTreeMap::new(), Some(pool), BTreeMap::new()));
  app.open_agent_overlay();
  app.open_agent_input();
  assert_eq!(app.agent_input_candidates().len(), count, "the pool feeds the prompt");
  app
}

/// Render into a fixed terminal and return the buffer as one row per line.
fn draw_rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
  let backend = TestBackend::new(width, height);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, app)).unwrap();
  let cells: Vec<String> = terminal
    .backend()
    .buffer()
    .content()
    .iter()
    .map(|c| c.symbol().to_string())
    .collect();
  cells.chunks(width as usize).map(|row| row.concat()).collect()
}

/// Line indices holding a bottom-left rounded corner — the modal's bottom
/// border moves iff the frame resizes (the background is identical across
/// draws of the same App state family).
fn corner_rows(rows: &[String]) -> Vec<usize> {
  rows
    .iter()
    .enumerate()
    .filter(|(_, r)| r.contains('╰') || r.contains('└'))
    .map(|(i, _)| i)
    .collect()
}

#[test]
fn attach_prompt_frame_does_not_resize_while_typing() {
  let dir = repo();
  let mut app = app_with_open_prompt(&dir, 8);
  let before = draw_rows(&mut app, 100, 32);
  assert!(
    before.iter().any(|r| r.contains("Attach a session")),
    "the prompt is on screen"
  );

  // 'z' matches no id: the candidate list collapses to the empty state.
  app.agent_input_push('z');
  assert_eq!(app.agent_input_candidates().len(), 0);
  let after = draw_rows(&mut app, 100, 32);

  assert_eq!(
    corner_rows(&before),
    corner_rows(&after),
    "the modal frame must keep its height when the filter empties the list"
  );
}

#[test]
fn attach_prompt_shows_a_scrollbar_when_candidates_overflow() {
  let dir = repo();
  // 40 candidates against a 24-row terminal: the visible window is far
  // smaller than the list, so the scrollbar affordance must appear.
  let mut app = app_with_open_prompt(&dir, 40);
  let rows = draw_rows(&mut app, 100, 24);
  assert!(
    rows.iter().any(|r| r.contains('█')),
    "an overflowing candidate list must render a scrollbar thumb"
  );
}
