//! Overlays size their height to their content (issue #569).
//!
//! The assertions here measure the painted box, top border row to bottom
//! border row, rather than calling the policy and comparing it to itself. A
//! unit test on `modal_height` restates its formula; what it cannot catch is
//! the chrome arithmetic at the call site, which is where the off-by-one
//! actually lives (border, padding, header, footer), nor whether the call site
//! is wired up at all.

use gwm::tui::{draw, App, View};
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;
use tempfile::TempDir;

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

/// The painted height of the topmost overlay box, in rows.
///
/// Located by its rounded corners rather than by a row count, so the measure
/// does not assume where the box sits. Returns `(top, bottom)` row indices.
fn box_rows(terminal: &Terminal<TestBackend>) -> (usize, usize) {
  let buf = terminal.backend().buffer();
  let area = *buf.area();
  let rows: Vec<String> = (0..area.height)
    .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect())
    .collect();
  let top = rows
    .iter()
    .position(|r| r.contains('╭'))
    .expect("an overlay must be painted");
  let bottom = rows
    .iter()
    .rposition(|r| r.contains('╰'))
    .expect("the overlay must be closed");
  (top, bottom)
}

#[test]
fn a_content_sized_modal_keeps_a_margin_on_a_short_terminal() {
  // Not the Settings panel: the create form already sized to its content and
  // still painted its border on rows 0 and 13 of a 14-row terminal, because
  // `centered_abs` clamps to the frame without reserving anything. Measured
  // before the change: `top=0 bottom=13` at 14 rows, `top=0 bottom=15` at 16.
  let dir = repo();
  for frame_h in [14u16, 16] {
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    app.view = View::Create;
    let mut terminal = Terminal::new(TestBackend::new(120, frame_h)).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let (top, bottom) = box_rows(&terminal);
    assert!(top >= 1, "frame_h={frame_h}: the create form starts flush on row {top}");
    assert!(
      bottom + 1 < frame_h as usize,
      "frame_h={frame_h}: the create form ends flush on row {bottom}"
    );
  }
}
