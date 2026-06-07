//! Command Logs modal state slice (issue #226).
//!
//! Pure-state contract for `tui::state::command_logs::CommandLogs`: the
//! scroll cursor (vertical + horizontal) clamped against bounds the
//! renderer republishes each frame, and the `sync` that copies the global
//! command-log into owned `App` state so the modal never renders off the
//! live global. Mirrors the help-overlay scroll model (`help_scroll` /
//! `help_max_scroll`) and the sidebar slice's clamp tests — ratatui-free,
//! no terminal backend.

use gwm::command_log::{self, CommandLogEntry, CommandStatus};
use gwm::tui::CommandLogs;
use std::time::Duration;

#[test]
fn new_starts_empty_at_the_origin() {
  let logs = CommandLogs::new();
  assert!(logs.entries.is_empty());
  assert_eq!(logs.scroll, 0);
  assert_eq!(logs.x_scroll, 0);
  assert_eq!(logs.max_scroll, 0);
  assert_eq!(logs.max_x_scroll, 0);
}

#[test]
fn scroll_down_clamps_to_max_scroll() {
  let mut logs = CommandLogs::new();
  logs.max_scroll = 3;
  for _ in 0..10 {
    logs.scroll_down();
  }
  assert_eq!(logs.scroll, 3, "never scrolls past the last line");
}

#[test]
fn scroll_up_saturates_at_zero() {
  let mut logs = CommandLogs::new();
  logs.max_scroll = 5;
  logs.scroll = 2;
  logs.scroll_up();
  logs.scroll_up();
  logs.scroll_up();
  assert_eq!(logs.scroll, 0);
}

#[test]
fn horizontal_scroll_clamps_both_ways() {
  let mut logs = CommandLogs::new();
  logs.max_x_scroll = 2;
  for _ in 0..5 {
    logs.scroll_right();
  }
  assert_eq!(logs.x_scroll, 2);
  for _ in 0..5 {
    logs.scroll_left();
  }
  assert_eq!(logs.x_scroll, 0);
}

#[test]
fn scroll_to_top_and_bottom_jump_to_the_bounds() {
  let mut logs = CommandLogs::new();
  logs.max_scroll = 7;
  logs.scroll_to_bottom();
  assert_eq!(logs.scroll, 7);
  logs.scroll_to_top();
  assert_eq!(logs.scroll, 0);
}

#[test]
fn reset_zeroes_the_cursor_but_keeps_entries() {
  let mut logs = CommandLogs::new();
  logs.entries.push(CommandLogEntry {
    command: "gh pr list".into(),
    duration: Duration::from_millis(1),
    status: CommandStatus::Exited(Some(0)),
    output: String::new(),
  });
  logs.scroll = 4;
  logs.x_scroll = 2;
  logs.reset();
  assert_eq!(logs.scroll, 0);
  assert_eq!(logs.x_scroll, 0);
  assert_eq!(logs.entries.len(), 1, "reset clears the cursor, not the data");
}

#[test]
fn sync_copies_the_global_command_log_into_the_slice() {
  // Unique sentinel keeps this assertion robust against entries a sibling
  // test recorded concurrently (presence, not exact contents).
  let sentinel = "gwm-slice-sync-c4d1";
  command_log::record(CommandLogEntry {
    command: format!("gh issue view 226 # {sentinel}"),
    duration: Duration::from_millis(2),
    status: CommandStatus::Exited(Some(0)),
    output: String::new(),
  });
  let mut logs = CommandLogs::new();
  logs.sync();
  assert!(
    logs.entries.iter().any(|e| e.command.contains(sentinel)),
    "sync pulls the global log into the slice"
  );
}
