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
fn run_logged_keeps_stderr_for_a_mixed_output_failure() {
  // Codex review (#259): a command that writes to stdout *and* puts its
  // diagnostics on stderr before failing must not lose the error text — the
  // transcript is for troubleshooting. Both streams are kept.
  let out_sentinel = "gwm-cmdlog-mixed-out-a1";
  let err_sentinel = "gwm-cmdlog-mixed-err-b2";
  let mut cmd = Command::new("sh");
  cmd
    .arg("-c")
    .arg(format!("echo {out_sentinel}; echo {err_sentinel} >&2; exit 1"));
  let _ = command_log::run_logged(&mut cmd, format!("sh -c mixed # {err_sentinel}"));

  let recorded = command_log::snapshot();
  let mine = recorded
    .iter()
    .find(|e| e.command.contains(err_sentinel))
    .expect("the failing command was recorded");
  assert_eq!(mine.status, CommandStatus::Exited(Some(1)));
  assert!(mine.output.contains(out_sentinel), "stdout is kept");
  assert!(mine.output.contains(err_sentinel), "stderr diagnostics are not dropped");
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

#[cfg(unix)]
#[test]
fn a_redacted_run_keeps_the_response_out_of_the_transcript() {
  // Issue #459, second half. Taking the body off the argv is only half
  // the job: the GitLab create endpoints echo `description` back in the
  // response, and the transcript records stdout verbatim, so the text
  // reappeared there. The caller still needs the real stdout to parse
  // the new iid, so the redaction has to be on the log, not the return.
  //
  // `cat` is the honest fixture here — it echoes stdin to stdout, which
  // is exactly the shape of the problem.
  command_log::reset();
  let mut cmd = std::process::Command::new("cat");
  let out = command_log::run_logged_with_stdin(&mut cmd, "glab api -X POST …".into(), b"SECRET-BODY", true).unwrap();

  assert!(
    String::from_utf8_lossy(&out.stdout).contains("SECRET-BODY"),
    "the caller must still receive the real response"
  );
  assert!(
    !command_log::snapshot().iter().any(|e| e.output.contains("SECRET-BODY")),
    "the transcript must not: {:?}",
    command_log::snapshot()
  );
}

#[cfg(unix)]
#[test]
fn an_unredacted_run_still_records_its_output() {
  // The negative control: redaction is opt-in, and every other logged
  // command must keep the output that makes the modal useful.
  command_log::reset();
  let mut cmd = std::process::Command::new("cat");
  command_log::run_logged_with_stdin(&mut cmd, "cat".into(), b"ordinary output", false).unwrap();

  assert!(
    command_log::snapshot()
      .iter()
      .any(|e| e.output.contains("ordinary output")),
    "{:?}",
    command_log::snapshot()
  );
}
