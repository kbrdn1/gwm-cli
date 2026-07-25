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

/// Validation feedback on PR #455: the CI checks overlay rendered the
/// AGENTS footer hints (attach / detach / by id) because the modal hint
/// context was hard-coded to `Detail` — and its rows carried no detail
/// beyond the name. The footer must advertise the ci_checks verbs, and
/// each row must surface the workflow + duration in a right-aligned
/// column.
#[test]
fn ci_checks_overlay_shows_its_own_hints_and_row_details() {
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::{ci_check_rows, DetailKind};
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let checks = vec![PrCheck {
    name: "test (ubuntu-latest)".into(),
    outcome: CheckOutcome::Passing,
    url: None,
    workflow_name: Some("ci".into()),
    started_at: Some("2026-07-24T14:51:06Z".into()),
    completed_at: Some("2026-07-24T14:52:24Z".into()),
  }];
  let rows = ci_check_rows(&checks, SystemTime::now());
  app.detail_overlay.open(DetailKind::CiChecks, "CI Checks".into(), rows);
  app.view = gwm::tui::View::DetailOverlay;

  let joined = draw_rows(&mut app, 100, 30).join("\n");
  assert!(joined.contains("open"), "ci_checks verbs must be advertised: {joined}");
  assert!(
    joined.contains("filter"),
    "ci_checks verbs must be advertised: {joined}"
  );
  assert!(
    !joined.contains("attach"),
    "the agents verbs must not leak into the CI overlay: {joined}"
  );
  assert!(
    joined.contains("ci · 1m18s"),
    "each row must carry workflow + duration: {joined}"
  );
}

/// Codex review on PR #455: a long check name used to push the detail
/// column past the modal edge (ratatui clips right), erasing exactly the
/// workflow + duration the column exists for. The value truncates with an
/// ellipsis instead, reserving the detail column's width.
#[test]
fn ci_checks_long_name_truncates_and_keeps_the_detail_column() {
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::{ci_check_rows, DetailKind};
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let checks = vec![PrCheck {
    name: format!("test (windows-latest, {})", "x".repeat(90)),
    outcome: CheckOutcome::Passing,
    url: None,
    workflow_name: Some("ci".into()),
    started_at: Some("2026-07-24T14:51:06Z".into()),
    completed_at: Some("2026-07-24T14:52:24Z".into()),
  }];
  let rows = ci_check_rows(&checks, SystemTime::now());
  app.detail_overlay.open(DetailKind::CiChecks, "CI Checks".into(), rows);
  app.view = gwm::tui::View::DetailOverlay;

  let joined = draw_rows(&mut app, 100, 30).join("\n");
  assert!(
    joined.contains("ci · 1m18s"),
    "the detail column survives a long check name: {joined}"
  );
  assert!(
    joined.contains('…'),
    "the long name truncates with an ellipsis: {joined}"
  );
}

/// Codex review #455 (P2): on a narrow terminal a long WORKFLOW name made
/// the detail column itself wider than the modal — the value truncated to
/// nothing while the untouched column ran past the clipping edge, and
/// ratatui cut its right end: the duration, the very info the column
/// carries. The column is bounded too, truncating from the workflow side
/// (leading ellipsis) so the duration tail survives.
#[test]
fn ci_checks_narrow_terminal_bounds_the_detail_column() {
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::{ci_check_rows, DetailKind};
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let checks = vec![PrCheck {
    name: "build".into(),
    outcome: CheckOutcome::Passing,
    url: None,
    workflow_name: Some(format!("release-{}", "w".repeat(80))),
    started_at: Some("2026-07-24T14:51:06Z".into()),
    completed_at: Some("2026-07-24T14:52:24Z".into()),
  }];
  let rows = ci_check_rows(&checks, SystemTime::now());
  app.detail_overlay.open(DetailKind::CiChecks, "CI Checks".into(), rows);
  app.view = gwm::tui::View::DetailOverlay;

  let joined = draw_rows(&mut app, 60, 20).join("\n");
  assert!(
    joined.contains("build"),
    "the check name keeps its reserved width: {joined}"
  );
  assert!(
    joined.contains("1m18s"),
    "the duration tail survives the bounded detail column: {joined}"
  );
  assert!(
    joined.contains('…'),
    "the oversized detail column truncates with an ellipsis: {joined}"
  );
}

/// Codex review on PR #455: the filter's scrollbar rect started at the
/// query line (`area.y + 4`) instead of the filtered listing (`+ 6`),
/// overlapping the input and stopping short of the last rows.
#[test]
fn ci_checks_filter_scrollbar_tracks_the_listing_not_the_query() {
  use gwm::github::{CheckOutcome, PrCheck};
  use gwm::tui::state::detail_overlay::{ci_check_rows, DetailKind};
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let checks: Vec<PrCheck> = (0..25)
    .map(|i| PrCheck {
      name: format!("check-{i:02}"),
      outcome: CheckOutcome::Passing,
      url: None,
      workflow_name: None,
      started_at: None,
      completed_at: None,
    })
    .collect();
  let rows = ci_check_rows(&checks, SystemTime::now());
  app.detail_overlay.open(DetailKind::CiChecks, "CI Checks".into(), rows);
  app.view = gwm::tui::View::DetailOverlay;
  app.ci_input_open();

  let rows = draw_rows(&mut app, 100, 26);
  assert!(
    rows.iter().any(|r| r.contains('█')),
    "an overflowing filtered list must show the scrollbar thumb"
  );
  let query_row = rows
    .iter()
    .find(|r| r.contains("filter:"))
    .expect("the query line is on screen");
  assert!(
    !query_row.contains('█'),
    "the scrollbar must not overlap the query line: {query_row}"
  );
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

#[test]
fn attach_prompt_survives_a_tiny_terminal() {
  // Codex review #445: on a terminal of 8 rows or fewer, centered_abs
  // clamps the modal to the terminal height but the scrollbar rect kept
  // its full geometry and rendered past the ratatui buffer — a panic.
  let dir = repo();
  let mut app = app_with_open_prompt(&dir, 10);
  let _ = draw_rows(&mut app, 100, 8);
  let _ = draw_rows(&mut app, 100, 5);
}

#[test]
fn detail_overlay_survives_a_tiny_terminal() {
  // Same class of bug on the sibling detail mode's rows rect.
  use gwm::agent_sessions::WorktreeAgents;
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let generation = app.tasks.request(TaskKind::AgentSessions).unwrap();
  let path = app.worktrees[0].path.to_string_lossy().to_string();
  let mut map = BTreeMap::new();
  map.insert(
    path,
    WorktreeAgents {
      sessions: (0..6).map(session).collect(),
    },
  );
  assert!(app.apply_agent_snapshot(generation, map, None, BTreeMap::new()));
  app.open_agent_overlay();
  let _ = draw_rows(&mut app, 100, 6);
  let _ = draw_rows(&mut app, 100, 5);
}

#[test]
fn attach_prompt_window_is_capped_on_tall_terminals() {
  // User feedback 2026-07-23: the fixed window must not swallow the whole
  // terminal — at most 10 candidate rows, the scrollbar covers the rest.
  let dir = repo();
  let mut app = app_with_open_prompt(&dir, 40);
  let rows = draw_rows(&mut app, 100, 40);
  let shown = rows.iter().filter(|r| r.contains("aaa-")).count();
  assert!(shown <= 10, "the candidate window is capped at 10 rows, got {shown}");
}
