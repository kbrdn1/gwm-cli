//! The map against the frame it describes (issue #624).
//!
//! `tui_mouse_app_tests.rs` pushes zones by hand, so on its own it proves
//! nothing about the renderer: every one of its assertions would still pass
//! with `ui::draw` publishing nothing at all. This file closes that hole by
//! rendering a real frame, finding a thing **in the painted buffer**, and
//! asking the map what is at that cell. The two answers come from different
//! places, which is the only reason the agreement means anything.

use gwm::tui::mouse::{Hit, RowList, Spot};
use gwm::tui::{draw, App, SettingsTab, View, CLOSE_ICON, COMMAND_LOGS_ICON, SETTINGS_ICON};
use gwm::worktree::{BranchStatus, WorktreeInfo};
use ratatui::{backend::TestBackend, Terminal};
use std::path::{Path, PathBuf};
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

fn app_with(rows: usize, compact: bool) -> (TempDir, App) {
  let dir = repo();
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  app.config.tui.layout = if compact {
    gwm::config::TuiLayout::Compact
  } else {
    gwm::config::TuiLayout::Bordered
  };
  app.worktrees = (0..rows).map(|i| worktree_fixture(&format!("zzwt{i:03}"))).collect();
  app.filter.invalidate();
  app.list_state.select(Some(0));
  (dir, app)
}

/// Rows of the painted buffer, one string per terminal line.
fn render(app: &mut App, w: u16, h: u16) -> Vec<String> {
  let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
  terminal.draw(|f| draw(f, app)).unwrap();
  let buf = terminal.backend().buffer().clone();
  (0..buf.area.height)
    .map(|y| {
      (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect::<String>()
    })
    .collect()
}

/// `(row, column)` of the leftmost `\u{203a}` in the buffer — the Settings
/// panel's selection marker.
///
/// Not a `find("\u{203a} ")`: a choice field renders its value as
/// `\u{2039} bordered \u{203a}`, so the plain search lands on the first
/// chevron of a value column rather than on the marker. The marker leads its
/// row and the value column trails it, so leftmost is the discriminant.
fn find_selection_marker(lines: &[String]) -> Option<(u16, u16)> {
  lines
    .iter()
    .enumerate()
    .filter_map(|(y, l)| {
      let byte = l.find('\u{203a}')?;
      Some((l[..byte].chars().count() as u16, y as u16))
    })
    .min()
    .map(|(x, y)| (y, x))
}

/// `(row, column)` of the first cell `needle` starts on.
fn find_cell(lines: &[String], needle: &str) -> Option<(u16, u16)> {
  lines.iter().enumerate().find_map(|(y, line)| {
    // Column, not byte offset: the buffer is one `String` per cell, so
    // counting cells means counting the symbols before the match.
    let byte = line.find(needle)?;
    let col = line[..byte].chars().count();
    Some((y as u16, col as u16))
  })
}

/// The load-bearing one. A worktree name is found where the renderer painted
/// it, and the map is asked what is at that cell — if `draw_list` published
/// nothing, or published a strip one row off, this is what says so.
///
/// Both layouts, because they reach the first data row by different routes
/// (compact carves its header line off the area, bordered lets the block eat
/// it) and only agree today by coincidence.
#[test]
fn clicking_where_a_worktree_name_is_painted_selects_that_worktree() {
  for compact in [false, true] {
    let (_d, mut app) = app_with(6, compact);
    let lines = render(&mut app, 120, 40);

    for target in [0usize, 3, 5] {
      let name = format!("zzwt{target:03}");
      let (y, x) = find_cell(&lines, &name)
        .unwrap_or_else(|| panic!("compact={compact}: {name} was never painted:\n{}", lines.join("\n")));
      assert_eq!(
        app.mouse.hit(x, y),
        Some(Hit::Row {
          list: RowList::Worktrees,
          index: target,
        }),
        "compact={compact}: the map disagrees with the row {name} is painted on (y={y})"
      );
    }
  }
}

/// The same agreement with a non-zero `TableState` offset, which is the case
/// a list that fits on screen can never produce: the render is what updates
/// the offset to bring the selection into view, so a map published before it
/// reports the previous frame's scroll.
#[test]
fn a_scrolled_table_still_agrees_with_what_it_painted() {
  let (_d, mut app) = app_with(120, false);
  app.list_state.select(Some(90));
  // Two frames: the first scrolls the selection into view, the second is
  // rendered against the settled offset.
  let _ = render(&mut app, 120, 30);
  let lines = render(&mut app, 120, 30);
  assert!(
    app.list_state.offset() > 0,
    "the fixture has to actually scroll or this proves nothing"
  );

  let name = "zzwt090";
  let (y, x) = find_cell(&lines, name).expect("the selected row is on screen");
  assert_eq!(
    app.mouse.hit(x, y),
    Some(Hit::Row {
      list: RowList::Worktrees,
      index: 90
    })
  );
}

#[test]
fn the_header_affordances_are_clickable_where_they_are_painted() {
  let (_d, mut app) = app_with(3, false);
  let lines = render(&mut app, 120, 40);

  for (icon, expected) in [(COMMAND_LOGS_ICON, Spot::CommandLogs), (SETTINGS_ICON, Spot::Settings)] {
    let (y, x) = find_cell(&lines, icon).unwrap_or_else(|| panic!("{icon} was never painted"));
    assert_eq!(y, 0, "the affordances live on the header row");
    assert_eq!(app.mouse.hit(x, y), Some(Hit::Spot(expected)));
  }
}

/// The close button, in both layouts, over a modal that is genuinely on
/// screen. `ModalFrame::render` is the one place it is painted, so this
/// covers every modal there is.
#[test]
fn the_modal_close_button_is_clickable_where_it_is_painted() {
  for compact in [false, true] {
    let (_d, mut app) = app_with(3, compact);
    app.enter_command_logs();
    let lines = render(&mut app, 120, 40);

    let (y, x) = find_cell(&lines, CLOSE_ICON)
      .unwrap_or_else(|| panic!("compact={compact}: no close button:\n{}", lines.join("\n")));
    assert_eq!(
      app.mouse.hit(x, y),
      Some(Hit::Spot(Spot::CloseModal)),
      "compact={compact}: the glyph at ({x},{y}) is not a close target"
    );
  }
}

/// A modal covers what is under it. The row the list would have published at
/// that cell must not be reachable through it.
#[test]
fn an_open_modal_takes_the_clicks_of_the_rows_it_covers() {
  let (_d, mut app) = app_with(6, false);
  let before = render(&mut app, 120, 40);
  let (y, x) = find_cell(&before, "zzwt002").expect("painted");
  assert!(matches!(app.mouse.hit(x, y), Some(Hit::Row { .. })));

  app.enter_command_logs();
  let _ = render(&mut app, 120, 40);

  assert!(
    !matches!(
      app.mouse.hit(x, y),
      Some(Hit::Row {
        list: RowList::Worktrees,
        ..
      })
    ),
    "the worktree row under the modal is still clickable"
  );
}

/// Every Settings tab, and for each the row the panel says is selected has to
/// be the row the map reports at the line the panel drew it on. Derived from
/// the renderer's own answer rather than from a handful of chosen lines: the
/// tabs section their rows differently (Keys groups by scope, the field tabs
/// by section, `All` not at all), so a fixed line number would only ever pin
/// one of them.
#[test]
fn the_settings_row_map_agrees_with_the_panel_selection_on_every_tab() {
  let (_d, mut app) = app_with(3, false);
  app.enter_config_panel();

  for tab in SettingsTab::ALL {
    app.config_panel.set_tab(tab);
    let selectable = app.config_panel.selectable_count();
    if selectable == 0 {
      continue;
    }
    for selected in [0usize, selectable / 2, selectable - 1] {
      app.config_panel.select_index(selected);
      // Two frames: the first scrolls the selection into view.
      let _ = render(&mut app, 120, 44);
      let lines = render(&mut app, 120, 44);
      assert_eq!(app.view, View::Config);

      // The row marker the panel paints on the selected row.
      let (y, x) = find_selection_marker(&lines)
        .unwrap_or_else(|| panic!("{tab:?}/{selected}: no selection marker:\n{}", lines.join("\n")));
      assert_eq!(
        app.mouse.hit(x, y),
        Some(Hit::Row {
          list: RowList::Config,
          index: selected,
        }),
        "{tab:?}: the map disagrees with the line the marker was painted on"
      );
    }
  }
}

#[test]
fn the_settings_tab_strip_is_clickable_where_it_is_painted() {
  let (_d, mut app) = app_with(3, false);
  app.enter_config_panel();
  let lines = render(&mut app, 120, 44);

  // The strip is the one row carrying every label; a lone `find` would land
  // on whichever body row happens to say "TUI" or "All" first.
  let (strip_y, strip) = lines
    .iter()
    .enumerate()
    .find(|(_, l)| SettingsTab::ALL.iter().all(|t| l.contains(t.label())))
    .map(|(y, l)| (y as u16, l.clone()))
    .expect("no tab strip on screen");

  for tab in SettingsTab::ALL {
    let byte = strip.find(tab.label()).expect("checked above");
    let (y, x) = (strip_y, strip[..byte].chars().count() as u16);
    assert_eq!(
      app.mouse.hit(x, y),
      Some(Hit::Spot(Spot::ConfigTab(tab))),
      "{tab:?}: the strip cell at ({x},{y}) is not that tab"
    );
  }
}
