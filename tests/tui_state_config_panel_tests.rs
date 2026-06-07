//! Configuration panel modal state slice (issue #232).
//!
//! Pure-state contract for `tui::state::config_panel::ConfigPanel`: the
//! scroll cursor (vertical + horizontal) clamped against bounds the
//! renderer republishes each frame, plus the owned resolved-row snapshot
//! the modal paints. Mirrors the Command Logs slice's clamp tests —
//! ratatui-free, no terminal backend.

use gwm::config::{ConfigRow, ConfigSource};
use gwm::tui::ConfigPanel;

fn sample_row() -> ConfigRow {
  ConfigRow {
    key: "worktree.base".into(),
    value: "\"/tmp/wt\"".into(),
    source: ConfigSource::Repo,
  }
}

#[test]
fn new_starts_empty_at_the_origin() {
  let panel = ConfigPanel::new();
  assert!(panel.rows.is_empty());
  assert_eq!(panel.scroll, 0);
  assert_eq!(panel.x_scroll, 0);
  assert_eq!(panel.max_scroll, 0);
  assert_eq!(panel.max_x_scroll, 0);
}

#[test]
fn scroll_down_clamps_to_max_scroll() {
  let mut panel = ConfigPanel::new();
  panel.max_scroll = 3;
  for _ in 0..10 {
    panel.scroll_down();
  }
  assert_eq!(panel.scroll, 3, "never scrolls past the last line");
}

#[test]
fn scroll_up_saturates_at_zero() {
  let mut panel = ConfigPanel::new();
  panel.max_scroll = 5;
  panel.scroll = 2;
  panel.scroll_up();
  panel.scroll_up();
  panel.scroll_up();
  assert_eq!(panel.scroll, 0);
}

#[test]
fn horizontal_scroll_clamps_both_ways() {
  let mut panel = ConfigPanel::new();
  panel.max_x_scroll = 2;
  for _ in 0..5 {
    panel.scroll_right();
  }
  assert_eq!(panel.x_scroll, 2);
  for _ in 0..5 {
    panel.scroll_left();
  }
  assert_eq!(panel.x_scroll, 0);
}

#[test]
fn scroll_to_top_and_bottom_jump_to_the_bounds() {
  let mut panel = ConfigPanel::new();
  panel.max_scroll = 7;
  panel.scroll_to_bottom();
  assert_eq!(panel.scroll, 7);
  panel.scroll_to_top();
  assert_eq!(panel.scroll, 0);
}

#[test]
fn reset_zeroes_the_cursor_but_keeps_rows() {
  let mut panel = ConfigPanel::new();
  panel.rows.push(sample_row());
  panel.scroll = 4;
  panel.x_scroll = 2;
  panel.reset();
  assert_eq!(panel.scroll, 0);
  assert_eq!(panel.x_scroll, 0);
  assert_eq!(panel.rows.len(), 1, "reset clears the cursor, not the data");
}
