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
  assert!(app.apply_agent_snapshot(generation, map, None, Default::default()));

  let text = draw_once(&mut app);
  assert!(text.contains("AGENT"), "a landed session must surface the column");
  assert!(text.contains("claude"), "the top session's agent must show");
}

// --- Bidi controls in a ref name (issue #506) ------------------------------

/// Every character carrying the Unicode `Bidi_Control` property. Not
/// `char::is_control`, which by construction matches none of them: they are
/// `Cf`, and that is the whole point.
const BIDI_CONTROLS: &[char] = &[
  '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}',
  '\u{2067}', '\u{2068}', '\u{2069}',
];

/// A repo whose checked-out branch is `name`.
fn repo_on_branch(name: &str) -> TempDir {
  let dir = repo();
  let repo = git2::Repository::open(dir.path()).unwrap();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch(name, &head, false).unwrap();
  repo.set_head(&format!("refs/heads/{name}")).unwrap();
  dir
}

#[test]
fn a_branch_name_cannot_carry_a_bidi_control_into_the_table() {
  // Git's ref rules refuse the ASCII controls, space and `~^:?*[`, but not
  // the Unicode format characters, so this name is a legal ref that can
  // arrive with a fetch rather than being typed locally. The table renders
  // through `Table`, which (measured on ratatui 0.30) drops zero-width
  // control bytes but keeps these, so the row can read in an order the ref
  // is not stored in.
  for c in BIDI_CONTROLS {
    let dir = repo_on_branch(&format!("feat/{c}danger"));
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    let text = draw_once(&mut app);
    let leaked: Vec<char> = text.chars().filter(|x| BIDI_CONTROLS.contains(x)).collect();
    assert!(
      leaked.is_empty(),
      "the table replayed U+{:04X} from the branch name",
      *c as u32
    );
  }
}

/// A repo living in a directory whose own name carries `c`.
///
/// The path column is the one cell that is not width-constrained, so it does
/// not pass through `trunc`'s funnel and needs its own sink. A hostile segment
/// therefore has to arrive through the path rather than through the ref name.
fn repo_under_segment(c: char) -> (TempDir, std::path::PathBuf) {
  let outer = TempDir::new().unwrap();
  let inner = outer.path().join(format!("wt{c}x"));
  std::fs::create_dir(&inner).unwrap();
  let repo = git2::Repository::init(&inner).unwrap();
  repo.set_head("refs/heads/main").ok();
  let sig = git2::Signature::now("gwm-test", "gwm@test").unwrap();
  std::fs::write(inner.join("file.txt"), "seed").unwrap();
  repo.index().unwrap().add_path(Path::new("file.txt")).unwrap();
  repo.index().unwrap().write().unwrap();
  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
  (outer, inner)
}

#[test]
fn a_worktree_path_cannot_carry_a_bidi_control_into_the_table() {
  // Wide enough that a `TempDir` path reaches the PATH column intact: at 120
  // cells the interesting segment is truncated away and the test would pass
  // without ever rendering it.
  for c in BIDI_CONTROLS {
    let (_outer, inner) = repo_under_segment(*c);
    let mut app = App::new_at_layered(Some(inner.as_path()), None).unwrap();
    let backend = TestBackend::new(300, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let text = buffer_text(&terminal);
    assert!(
      text.contains("wt?x"),
      "the fixture must actually reach the PATH column, got {:?}",
      text.lines().next().unwrap_or_default()
    );
    let leaked: Vec<char> = text.chars().filter(|x| BIDI_CONTROLS.contains(x)).collect();
    assert!(
      leaked.is_empty(),
      "the table replayed U+{:04X} from the worktree path",
      *c as u32
    );
  }
}
