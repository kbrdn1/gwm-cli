//! Command-log ring buffer + logged-exec helper (issue #226).
//!
//! The command log is the data behind the lazygit-style Command Logs modal:
//! a bounded, in-memory transcript of the external commands gwm shells out
//! to (the `gh` GitHub calls, bootstrap shell steps, lifecycle hooks). This
//! file pins two layers:
//!
//!   1. [`CommandLog`] — the pure ring buffer (push / bound / snapshot /
//!      clear), tested in isolation with no global state so the assertions
//!      are deterministic under `cargo test`'s parallel runner.
//!   2. [`command_log::run_logged`] — the exec wrapper that times a child
//!      process, captures its output + exit, and records an entry on the
//!      process-global log. The global-touching tests use a unique sentinel
//!      command string and assert *presence* (never an exact count), so a
//!      sibling test recording concurrently cannot make them flake.

use gwm::command_log::{self, CommandLog, CommandLogEntry, CommandStatus, MAX_ENTRIES};
use std::process::Command;
use std::time::Duration;

fn entry(command: &str, code: i32) -> CommandLogEntry {
  CommandLogEntry {
    command: command.into(),
    duration: Duration::from_millis(1),
    status: CommandStatus::Exited(Some(code)),
    output: String::new(),
  }
}

// ---------------------------------------------------------------------------
// CommandLog — pure ring buffer (no global, no I/O)
// ---------------------------------------------------------------------------

#[test]
fn push_appends_in_order_and_snapshot_clones() {
  let mut log = CommandLog::new();
  assert!(log.is_empty());
  log.push(entry("gh issue view 1", 0));
  log.push(entry("gh pr list", 0));
  assert_eq!(log.len(), 2);
  let snap = log.snapshot();
  assert_eq!(snap.len(), 2);
  assert_eq!(snap[0].command, "gh issue view 1");
  assert_eq!(snap[1].command, "gh pr list");
}

#[test]
fn ring_drops_the_oldest_entry_past_the_cap() {
  let mut log = CommandLog::new();
  // One past the cap: the very first push must have been evicted.
  for i in 0..=MAX_ENTRIES {
    log.push(entry(&format!("cmd {i}"), 0));
  }
  assert_eq!(log.len(), MAX_ENTRIES, "ring is bounded to MAX_ENTRIES");
  let snap = log.snapshot();
  assert_eq!(snap.first().unwrap().command, "cmd 1", "oldest (cmd 0) evicted");
  assert_eq!(snap.last().unwrap().command, format!("cmd {MAX_ENTRIES}"));
}

#[test]
fn clear_empties_the_ring() {
  let mut log = CommandLog::new();
  log.push(entry("x", 0));
  log.clear();
  assert!(log.is_empty());
  assert_eq!(log.len(), 0);
}

#[test]
fn is_success_tracks_the_exit_code() {
  assert!(entry("ok", 0).is_success());
  assert!(!entry("bad", 1).is_success());
  let spawn = CommandLogEntry {
    command: "missing-bin".into(),
    duration: Duration::from_millis(0),
    status: CommandStatus::Spawn,
    output: String::new(),
  };
  assert!(!spawn.is_success(), "a spawn failure is never a success");
}

// ---------------------------------------------------------------------------
// run_logged — exec wrapper feeding the process-global log
// ---------------------------------------------------------------------------

#[test]
fn run_logged_records_a_successful_command_with_its_output() {
  // Unique sentinel so a concurrent test recording to the same global log
  // cannot collide with this assertion (presence, not count).
  let sentinel = "gwm-cmdlog-success-7f3a";
  let mut cmd = Command::new("sh");
  cmd.arg("-c").arg(format!("echo {sentinel}"));
  let out = command_log::run_logged(&mut cmd, format!("sh -c echo {sentinel}")).expect("spawns");
  assert!(out.status.success());

  let recorded = command_log::snapshot();
  let mine = recorded
    .iter()
    .find(|e| e.command.contains(sentinel))
    .expect("the executed command was recorded on the global log");
  assert_eq!(mine.status, CommandStatus::Exited(Some(0)));
  assert!(mine.output.contains(sentinel), "captured stdout is stored on the entry");
}

#[test]
fn run_logged_records_a_nonzero_exit() {
  let sentinel = "gwm-cmdlog-fail-b219";
  let mut cmd = Command::new("sh");
  // `: marker` keeps the sentinel in the argv string we log without
  // emitting it to stdout; the command itself exits 3.
  cmd.arg("-c").arg("exit 3");
  let _ = command_log::run_logged(&mut cmd, format!("sh -c exit 3 # {sentinel}"));

  let recorded = command_log::snapshot();
  let mine = recorded
    .iter()
    .find(|e| e.command.contains(sentinel))
    .expect("a failing command is still recorded");
  assert_eq!(mine.status, CommandStatus::Exited(Some(3)));
  assert!(!mine.is_success());
}
