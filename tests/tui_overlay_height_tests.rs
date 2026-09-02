//! Overlays size their height to their content (issue #569).
//!
//! The assertions here measure the painted box, top border row to bottom
//! border row, rather than calling the policy and comparing it to itself. A
//! unit test on `modal_height` restates its formula; what it cannot catch is
//! the chrome arithmetic at the call site, which is where the off-by-one
//! actually lives (border, padding, header, footer), nor whether the call site
//! is wired up at all.

use gwm::config::{ConfigRow, ConfigSource, TuiLayout};
use gwm::tui::keymap::Keymap;
use gwm::tui::modal_keymap::ModalKeymap;
use gwm::tui::{build_key_rows, draw, App, SettingsTab, View};
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

/// Pin the boxed layout: since #594 the modal frame follows `[tui] layout`
/// and the shipped default is `compact`, which paints no corners at all.
/// Every measure below is border-row to border-row, so it is the boxed
/// contract they describe. The compact frame's own row cost is pinned in
/// `tests/tui_modal_render_tests.rs`.
fn pin_bordered(app: &mut App) {
  app.config.tui.layout = TuiLayout::Bordered;
}

/// The painted height of the topmost overlay box, in rows.
///
/// Located by its rounded corners rather than by a row count, so the measure
/// does not assume where the box sits. Returns `(top, bottom)` row indices.
///
/// Column 0 is skipped: in the boxed layout the sidebar's own sections are
/// rounded too, and they sit flush with the left edge where a centred modal
/// never does. Scanning it would stretch the measure from the sidebar's
/// first section to the modal's bottom rule (issue #594).
fn box_rows(terminal: &Terminal<TestBackend>) -> (usize, usize) {
  let buf = terminal.backend().buffer();
  let area = *buf.area();
  let rows: Vec<String> = (0..area.height)
    .map(|y| (1..area.width).map(|x| buf[(x, y)].symbol()).collect())
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

fn height(terminal: &Terminal<TestBackend>) -> usize {
  let (top, bottom) = box_rows(terminal);
  bottom - top + 1
}

/// The Settings panel on `tab`, drawn on a `frame_h`-row terminal, with both
/// the resolved-config rows and the keymap rows seeded so every tab has its
/// real content. The event loop is what fills these in production; here they
/// are injected so the *render* is deterministic.
fn settings(dir: &TempDir, tab: SettingsTab, frame_h: u16) -> Terminal<TestBackend> {
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  pin_bordered(&mut app);
  app.config_panel.rows = (0..12)
    .map(|i| ConfigRow {
      key: format!("key.number.{i}"),
      value: "\"value\"".into(),
      source: ConfigSource::Default,
    })
    .collect();
  app.config_panel.key_rows = build_key_rows(&Keymap::defaults(), &ModalKeymap::defaults(), |_| ConfigSource::Default);
  app.config_panel.tab = tab;
  app.view = View::Config;
  let mut terminal = Terminal::new(TestBackend::new(120, frame_h)).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  terminal
}

#[test]
fn the_settings_panel_is_shorter_on_a_short_tab_than_on_a_long_one() {
  // The discriminating assertion, and the only one that fails if the panel
  // goes back to a flat percentage of the frame: at one terminal size, two
  // tabs, two different boxes. Measured content (issue #569): the Worktree
  // tab has 3 rows, the Keys tab 173.
  let dir = repo();
  let short = height(&settings(&dir, SettingsTab::Worktree, 40));
  let long = height(&settings(&dir, SettingsTab::Keys, 40));
  assert!(
    short < long,
    "Worktree ({short} rows) must not get the same box as Keys ({long} rows)"
  );
}

#[test]
fn the_settings_panel_spends_its_rows_on_content_not_on_blanks() {
  // The chrome arithmetic, pinned exactly on the tab whose content is known
  // and small: 3 field rows, plus the header (layer subtitle, spacer, tab
  // strip, spacer), the blank row above the hints and the hints themselves,
  // the rounded border and the shared interior padding. Before #569 this box
  // was 24 rows on a 40-row terminal, roughly six of them blank. The gap row
  // is #594: content never sits flush against the footer, in either layout.
  //
  // The header's trailing spacer is #623: without it the first section rule
  // sits flush under the tab strip and reads as belonging to it. The Worktree
  // tab has no section of its own, and pays for it anyway, which is the trade
  // for one header shape across the five tabs.
  let dir = repo();
  let h = height(&settings(&dir, SettingsTab::Worktree, 40));
  let expected = 3 /* fields */ + 4 /* header */ + 2 /* gap + footer */ + 2 /* border */ + 2 /* padding */;
  assert_eq!(h, expected, "the Worktree tab's box must fit its three rows");
}

#[test]
fn the_settings_panel_stops_at_its_ceiling_rather_than_filling_the_frame() {
  // The Keys tab is 173 rows and scrolls; without a ceiling it would take the
  // whole terminal on any tall one. The scroll path is what makes a ceiling
  // safe here, and it predates this change (#279).
  let dir = repo();
  let tall = height(&settings(&dir, SettingsTab::Keys, 80));
  assert!(
    tall < 80 - 4,
    "a 173-row tab must not swallow an 80-row frame, got {tall}"
  );
}

#[test]
fn the_settings_panel_keeps_a_margin_on_a_short_terminal() {
  // The floor must never push the box past the frame: on a terminal shorter
  // than the floor, the margin wins.
  //
  // Which is only safe because the rows it costs are reachable. The same
  // margin was removed from the exact-height modals for taking a control off
  // the bottom of a delete dialogue (Codex review P2); here the footer hint is
  // a fixed-length region and the body a flexible one that already scrolls, so
  // the squeeze lands on the part that can be scrolled back. Asserted rather
  // than assumed, because that is precisely what went wrong next door.
  let dir = repo();
  for frame_h in [12u16, 14, 16, 20] {
    let terminal = settings(&dir, SettingsTab::Keys, frame_h);
    let painted = {
      let buf = terminal.backend().buffer();
      let area = *buf.area();
      (0..area.height)
        .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
    };
    assert!(
      painted.contains("Esc"),
      "frame_h={frame_h}: the footer hint must survive the squeeze"
    );
    let (top, bottom) = box_rows(&terminal);
    assert!(
      top >= 1,
      "frame_h={frame_h}: the box starts on row {top}, flush with the frame"
    );
    assert!(
      bottom + 1 < frame_h as usize,
      "frame_h={frame_h}: the box ends on row {bottom} of {frame_h}, flush with the frame"
    );
  }
}

#[test]
fn a_modal_that_cannot_scroll_keeps_its_rows_rather_than_its_margin() {
  // The margin is not free, and this is where it costs too much. The create
  // form, the rename form and both delete dialogues compute an exact content
  // height and have no scroll path, so reserving two rows per side does not
  // shrink a box, it deletes lines off the bottom of one. Codex review, P2:
  // a delete confirmation for a target carrying a branch asks for 13 rows,
  // and clamping it to 12 on a 16-row terminal drops the interactive
  // `Delete Branch` row entirely, with no way to reach it.
  //
  // So on these surfaces a modal that outgrows the frame takes the frame. The
  // border sitting flush with the edge is a cosmetic cost; a control the user
  // cannot see or reach is not.
  let dir = repo();
  for frame_h in [14u16, 16] {
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    pin_bordered(&mut app);
    app.view = View::Create;
    let mut terminal = Terminal::new(TestBackend::new(120, frame_h)).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let (top, bottom) = box_rows(&terminal);
    assert_eq!(
      bottom - top + 1,
      frame_h as usize,
      "frame_h={frame_h}: a non-scrolling modal must spend the whole frame, not lose rows to a margin"
    );
  }
}
