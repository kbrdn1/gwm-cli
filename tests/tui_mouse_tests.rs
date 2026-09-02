//! Hit-testing contract for the mouse map (issue #624).
//!
//! Pure geometry: no ratatui backend, no terminal, no `App`. What is pinned
//! here is the translation from a terminal cell to a target, which is the one
//! piece every click and wheel event in the TUI goes through.

use gwm::tui::mouse::{Hit, MouseMap, PaneId, RowList, Spot};
use gwm::tui::SettingsTab;
use ratatui::layout::Rect;

fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
  Rect {
    x,
    y,
    width: w,
    height: h,
  }
}

#[test]
fn a_row_strip_resolves_a_cell_to_the_row_drawn_on_it() {
  let mut map = MouseMap::new();
  map.push_rows(rect(0, 3, 40, 5), RowList::Worktrees, 0, 5);

  for n in 0..5u16 {
    assert_eq!(
      map.hit(10, 3 + n),
      Some(Hit::Row {
        list: RowList::Worktrees,
        index: n as usize,
      }),
      "row at y={} should resolve to index {}",
      3 + n,
      n
    );
  }
}

/// The guard that stops the table's click handler from being right only while
/// the list happens to fit on screen. `TableState::offset()` is non-zero the
/// moment the selection scrolls past the viewport, and every index the map
/// reports is shifted by it. A five-row list in a twenty-row viewport passes
/// this suite with the offset ignored entirely.
#[test]
fn a_scrolled_strip_reports_the_item_actually_drawn_not_the_screen_row() {
  let mut map = MouseMap::new();
  // 200 worktrees, scrolled so that item 42 is painted on the strip's first
  // row.
  map.push_rows(rect(0, 3, 40, 10), RowList::Worktrees, 42, 200);

  assert_eq!(
    map.hit(10, 3),
    Some(Hit::Row {
      list: RowList::Worktrees,
      index: 42
    })
  );
  assert_eq!(
    map.hit(10, 12),
    Some(Hit::Row {
      list: RowList::Worktrees,
      index: 51
    })
  );
}

#[test]
fn a_cell_past_the_last_item_resolves_to_nothing() {
  let mut map = MouseMap::new();
  // Three worktrees in a ten-row strip: the seven rows below them are blank
  // and clicking one must not select a worktree that is not there.
  map.push_rows(rect(0, 3, 40, 10), RowList::Worktrees, 0, 3);

  assert_eq!(
    map.hit(10, 5),
    Some(Hit::Row {
      list: RowList::Worktrees,
      index: 2
    })
  );
  assert_eq!(map.hit(10, 6), None);
  assert_eq!(map.hit(10, 12), None);
}

/// The rule that makes a modal work without `hit` knowing what a modal is:
/// the renderer draws back to front, so the map is walked front to back.
#[test]
fn the_last_zone_published_is_the_one_that_is_hit() {
  let mut map = MouseMap::new();
  // The list view underneath…
  map.push_pane(rect(0, 1, 80, 20), PaneId::Worktrees);
  map.push_rows(rect(0, 3, 80, 10), RowList::Worktrees, 0, 10);
  // …then a modal over the middle of it.
  map.push_rows(rect(10, 5, 60, 6), RowList::ExecPicker, 0, 6);

  assert_eq!(
    map.hit(40, 6),
    Some(Hit::Row {
      list: RowList::ExecPicker,
      index: 1
    }),
    "a cell the modal covers belongs to the modal"
  );
  assert_eq!(
    map.hit(2, 6),
    Some(Hit::Row {
      list: RowList::Worktrees,
      index: 3
    }),
    "a cell outside the modal still belongs to what is under it"
  );
}

/// Settings and the palette draw section rules and blank spacers between
/// their rows, so line N of the body is not item N. The map carries the
/// translation the renderer already had to compute.
#[test]
fn a_mapped_strip_ignores_the_lines_that_are_not_items() {
  let mut map = MouseMap::new();
  // body lines: [rule, item0, item1, blank, rule, item2]
  let lines = vec![None, Some(0), Some(1), None, None, Some(2)];
  map.push_mapped_rows(rect(0, 4, 60, 6), RowList::Config, 0, 3, lines);

  assert_eq!(map.hit(10, 4), None, "a section rule is not clickable");
  assert_eq!(
    map.hit(10, 5),
    Some(Hit::Row {
      list: RowList::Config,
      index: 0
    })
  );
  assert_eq!(
    map.hit(10, 6),
    Some(Hit::Row {
      list: RowList::Config,
      index: 1
    })
  );
  assert_eq!(map.hit(10, 7), None, "a blank spacer is not clickable");
  assert_eq!(
    map.hit(10, 9),
    Some(Hit::Row {
      list: RowList::Config,
      index: 2
    })
  );
}

#[test]
fn a_mapped_strip_offsets_into_the_map_by_the_scroll() {
  let mut map = MouseMap::new();
  let lines = vec![Some(0), None, Some(1), None, Some(2)];
  // Scrolled by two: body line 2 is painted on the strip's first row.
  map.push_mapped_rows(rect(0, 4, 60, 3), RowList::Config, 2, 3, lines);

  assert_eq!(
    map.hit(10, 4),
    Some(Hit::Row {
      list: RowList::Config,
      index: 1
    })
  );
  assert_eq!(map.hit(10, 5), None);
  assert_eq!(
    map.hit(10, 6),
    Some(Hit::Row {
      list: RowList::Config,
      index: 2
    })
  );
}

#[test]
fn panes_and_spots_resolve_to_themselves() {
  let mut map = MouseMap::new();
  map.push_pane(rect(0, 10, 80, 8), PaneId::Status);
  map.push_pane(rect(1, 12, 78, 3), PaneId::WorkingTree);
  map.push_spot(rect(70, 0, 3, 1), Spot::CommandLogs);
  map.push_spot(rect(73, 0, 3, 1), Spot::Settings);
  map.push_spot(rect(5, 3, 8, 1), Spot::ConfigTab(SettingsTab::Keys));
  map.push_spot(rect(20, 17, 12, 1), Spot::WtCounts);

  assert_eq!(map.hit(40, 11), Some(Hit::Pane(PaneId::Status)));
  assert_eq!(
    map.hit(40, 13),
    Some(Hit::Pane(PaneId::WorkingTree)),
    "the Working Tree sub-pane is published over the sidebar and wins"
  );
  assert_eq!(map.hit(71, 0), Some(Hit::Spot(Spot::CommandLogs)));
  assert_eq!(map.hit(74, 0), Some(Hit::Spot(Spot::Settings)));
  assert_eq!(map.hit(6, 3), Some(Hit::Spot(Spot::ConfigTab(SettingsTab::Keys))));
  assert_eq!(map.hit(21, 17), Some(Hit::Spot(Spot::WtCounts)));
}

#[test]
fn a_zero_area_zone_swallows_nothing() {
  let mut map = MouseMap::new();
  map.push_pane(rect(0, 1, 80, 20), PaneId::Worktrees);
  // A modal the height policy collapsed to nothing on a two-row terminal.
  map.push_rows(rect(10, 5, 0, 0), RowList::ExecPicker, 0, 4);

  assert_eq!(
    map.hit(10, 5),
    Some(Hit::Pane(PaneId::Worktrees)),
    "an empty rect must not eat the click of what it covers"
  );
}

#[test]
fn a_cell_outside_every_zone_hits_nothing() {
  let mut map = MouseMap::new();
  map.push_rows(rect(4, 3, 10, 5), RowList::Worktrees, 0, 5);

  assert_eq!(map.hit(3, 4), None, "one column left of the strip");
  assert_eq!(map.hit(14, 4), None, "one column right of the strip");
  assert_eq!(map.hit(8, 2), None, "one row above the strip");
  assert_eq!(map.hit(8, 8), None, "one row below the strip");
}

#[test]
fn clear_drops_every_zone_so_a_surface_that_left_the_screen_stops_being_clickable() {
  let mut map = MouseMap::new();
  map.push_rows(rect(0, 3, 40, 5), RowList::Worktrees, 0, 5);
  assert!(!map.is_empty());
  assert!(map.hit(10, 4).is_some());

  map.clear();

  assert!(map.is_empty());
  assert_eq!(map.hit(10, 4), None);
}

// ---- What gwm asks the terminal for ---------------------------------------

/// The lesson that cost a round of feedback: the three tracking modes are not
/// independent switches. A terminal keeps ONE, and `1003h` supersedes `1002h`
/// supersedes `1000h`, so setting all three and clearing the top two — the
/// obvious way to trim `EnableMouseCapture` down — leaves tracking off
/// entirely instead of falling back to `1000`. Measured on Ghostty 1.3.1: the
/// drag came back and the click stopped arriving.
///
/// So the sequence has to SET `1000` and never mention the other two, which
/// is a property of the bytes rather than of anything observable from a test
/// terminal — hence a byte assertion.
#[cfg(not(windows))]
#[test]
fn gwm_asks_for_press_tracking_and_nothing_that_reports_motion() {
  let mut out: Vec<u8> = Vec::new();
  gwm::tui::enable_mouse(&mut out).unwrap();
  let seq = String::from_utf8(out).unwrap();

  assert!(seq.contains("\u{1b}[?1000h"), "press tracking must be set: {seq:?}");
  assert!(
    seq.contains("\u{1b}[?1006h"),
    "SGR coordinates must be set, or a click past column 223 is unreadable: {seq:?}"
  );
  for mode in ["1002", "1003"] {
    assert!(
      !seq.contains(mode),
      "mode {mode} must not be mentioned at all — setting it and clearing it \
       leaves tracking OFF, and gwm reads neither drags nor motion: {seq:?}"
    );
  }
}
