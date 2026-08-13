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

// --- wide glyphs in a table cell (issue #560) ------------------------------

/// The rendered rows, one string per terminal line — `buffer_text` flattens
/// the whole buffer, so a needle found in it may come from any row.
fn rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
  let buf = terminal.backend().buffer();
  let area = *buf.area();
  (0..area.height)
    .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect())
    .collect()
}

#[test]
fn a_branch_of_wide_glyphs_is_truncated_by_gwm_not_clipped_by_ratatui() {
  // 20 ideographs: 20 characters, 40 columns. The BRANCH column is sized off
  // the same character count, so it is 20 cells wide — `trunc` measuring
  // characters called the name short and handed it over whole, and the
  // `Table` then hard-clipped it at the column edge. A clip drops the tail
  // with no marker, which is the whole difference: nothing on the row says
  // the branch shown is not the branch it is on.
  let branch = "作".repeat(20);
  let dir = repo_on_branch(&branch);
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  let rows = rows(&terminal);

  // The cursor row, not the sidebar's Status block, which shows the same
  // branch through a funnel of its own.
  let row = rows
    .iter()
    .find(|r| r.starts_with('▶'))
    .unwrap_or_else(|| panic!("no cursor row in the table:\n{}", rows.join("\n")));
  assert!(
    row.matches('作').count() >= 5,
    "the fixture never reached the BRANCH column: {row:?}"
  );
  // Whatever follows the last ideograph is what the cell ended on. A wide
  // glyph owns a second buffer cell the renderer leaves blank, so the
  // ellipsis is reached past that blank rather than glued to the glyph.
  let last = row.rfind('作').unwrap() + '作'.len_utf8();
  assert!(
    row[last..].trim_start().starts_with('…'),
    "the branch cell must end on gwm's ellipsis, not on a ratatui clip: {row:?}"
  );
}

// --- mark column (issue #484) ---------------------------------------------
//
// Same conditional-column contract as AGENT above: a user who never presses
// `Space` keeps the exact pre-#484 table, and the column appears with the
// first mark.

/// A synthetic non-main row, the only kind that can be marked.
fn markable_row(name: &str) -> gwm::worktree::WorktreeInfo {
  gwm::worktree::WorktreeInfo {
    name: name.into(),
    id: name.into(),
    path: std::path::PathBuf::from(format!("/tmp/gwm-test/{}", name)),
    branch: Some(format!("feat/#484-{}", name)),
    head: None,
    is_main: false,
    is_locked: false,
    is_prunable: false,
    status: gwm::worktree::BranchStatus::default(),
    link: gwm::github::BranchLink::empty(),
    issue_state: None,
    pr_state: None,
    age: None,
    has_note: false,
  }
}

#[test]
fn the_mark_column_is_absent_until_a_row_is_marked() {
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  // The sidebar paints `State ✓ clean`, so leaving it open would satisfy the
  // glyph search below from outside the table and pass both tests vacuously.
  app.sidebar.open = false;
  app.worktrees.push(markable_row("feat-484-plain"));
  app.list_state.select(Some(app.worktrees.len() - 1));

  let text = draw_once(&mut app);
  assert!(
    !text.contains('\u{2713}'),
    "nothing marked -> the table must stay visually pre-#484: {text}"
  );
  assert!(
    !text.contains("marked"),
    "and the footer must not carry a mark count: {text}"
  );
}

#[test]
fn a_marked_row_shows_the_glyph_and_the_footer_count() {
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  // Sidebar closed for the same reason as above: its `State ✓ clean` row
  // would make the glyph assertion pass without a mark column at all.
  app.sidebar.open = false;
  app.worktrees.push(markable_row("feat-484-batch"));
  app.list_state.select(Some(app.worktrees.len() - 1));
  app.toggle_select();
  assert_eq!(app.marked_count(), 1, "status was: {}", app.status);

  let text = draw_once(&mut app);
  assert!(text.contains('\u{2713}'), "the marked row must carry the glyph");
  assert!(
    text.contains("1 marked"),
    "and the pane footer must carry the count: {text}"
  );
}

// --- Note column (issue #515) ----------------------------------------------
//
// Same conditional rule as AGENT and the mark column: a user who has never
// written a note keeps the exact pre-#515 table, and the marker appears with
// the first note. Binary by design — this row carries one or it does not.

#[test]
fn the_note_column_is_absent_while_no_row_carries_a_note() {
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  app.sidebar.open = false;
  app.worktrees.push(markable_row("feat-515-plain"));
  app.list_state.select(Some(app.worktrees.len() - 1));

  let text = draw_once(&mut app);
  assert!(
    !text.contains('\u{2261}'),
    "no note -> the table must stay visually pre-#515: {text}"
  );
}

#[test]
fn a_row_with_a_note_shows_the_marker() {
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  app.sidebar.open = false;
  let mut row = markable_row("feat-515-noted");
  row.has_note = true;
  app.worktrees.push(row);
  app.list_state.select(Some(app.worktrees.len() - 1));

  let text = draw_once(&mut app);
  assert!(text.contains('\u{2261}'), "the noted row must carry the marker: {text}");
}

#[test]
fn the_note_marker_is_only_on_the_rows_that_carry_one() {
  // The marker column is shown as soon as ANY visible row has a note, so the
  // rows without one must render an empty cell rather than inherit the glyph.
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  app.sidebar.open = false;
  let mut noted = markable_row("feat-515-noted");
  noted.has_note = true;
  app.worktrees.push(noted);
  app.worktrees.push(markable_row("feat-515-bare"));
  app.list_state.select(Some(0));

  let text = draw_once(&mut app);
  assert_eq!(
    text.matches('\u{2261}').count(),
    1,
    "exactly one row carries a note: {text}"
  );
}
