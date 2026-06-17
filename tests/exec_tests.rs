//! Unit tests for `gwm exec` (issue #313) — the pure command fan-out layer.
//!
//! The CLI wiring (worktree selection, printing) lives in `cli.rs`; the
//! testable surface is the spawn primitive (`exec_in_dir`), the exit-code
//! rollup, and the per-worktree line formatter — none of which need a git
//! repo. Spawning uses `sh -c`, present at `/bin/sh` even on a stripped
//! CI PATH, so these stay environment-independent (CLAUDE.md).

use clap::Parser;
use gwm::cli::{Cli, Command};
use gwm::exec::{exec_in_dir, format_outcome, rollup_exit_code, ExecOutcome, ExecStatus};
use tempfile::TempDir;

#[test]
fn rollup_is_zero_when_all_ok() {
  let outcomes = vec![
    ExecOutcome {
      name: "a".into(),
      status: ExecStatus::Ok,
    },
    ExecOutcome {
      name: "b".into(),
      status: ExecStatus::Ok,
    },
  ];
  assert_eq!(rollup_exit_code(&outcomes), 0);
}

#[test]
fn rollup_is_nonzero_when_any_failed() {
  let outcomes = vec![
    ExecOutcome {
      name: "a".into(),
      status: ExecStatus::Ok,
    },
    ExecOutcome {
      name: "b".into(),
      status: ExecStatus::Failed(3),
    },
  ];
  assert_ne!(rollup_exit_code(&outcomes), 0);
}

#[test]
fn rollup_is_nonzero_on_spawn_error() {
  let outcomes = vec![ExecOutcome {
    name: "a".into(),
    status: ExecStatus::SpawnError("not found".into()),
  }];
  assert_ne!(rollup_exit_code(&outcomes), 0);
}

#[test]
fn rollup_is_nonzero_on_signal() {
  let outcomes = vec![ExecOutcome {
    name: "a".into(),
    status: ExecStatus::Signal,
  }];
  assert_ne!(rollup_exit_code(&outcomes), 0);
}

#[test]
fn exec_in_dir_runs_in_the_given_directory() {
  let dir = TempDir::new().unwrap();
  std::fs::write(dir.path().join("marker.txt"), b"hi").unwrap();
  // `test -f marker.txt` is true only if the child's CWD is `dir`.
  let status = exec_in_dir(dir.path(), "sh", &["-c".into(), "test -f marker.txt".into()]);
  assert_eq!(
    status,
    ExecStatus::Ok,
    "expected the command to run inside the worktree dir"
  );
}

#[test]
fn exec_in_dir_captures_nonzero_exit_code() {
  let dir = TempDir::new().unwrap();
  let status = exec_in_dir(dir.path(), "sh", &["-c".into(), "exit 7".into()]);
  assert_eq!(status, ExecStatus::Failed(7));
}

#[test]
fn exec_in_dir_reports_spawn_error_for_missing_program() {
  let dir = TempDir::new().unwrap();
  let status = exec_in_dir(dir.path(), "gwm-no-such-program-xyz", &[]);
  assert!(
    matches!(status, ExecStatus::SpawnError(_)),
    "missing binary should surface as SpawnError, got {status:?}"
  );
}

#[test]
fn format_outcome_marks_success_with_name() {
  let ok = ExecOutcome {
    name: "feat-1".into(),
    status: ExecStatus::Ok,
  };
  let line = format_outcome(&ok);
  assert!(line.contains('✓'), "success line should carry the ✓ sigil: {line}");
  assert!(line.contains("feat-1"), "success line should name the worktree: {line}");
}

#[test]
fn format_outcome_marks_failure_with_exit_code() {
  let bad = ExecOutcome {
    name: "fix-2".into(),
    status: ExecStatus::Failed(2),
  };
  let line = format_outcome(&bad);
  assert!(line.contains('✗'), "failure line should carry the ✗ sigil: {line}");
  assert!(line.contains("fix-2"), "failure line should name the worktree: {line}");
  assert!(line.contains('2'), "failure line should surface the exit code: {line}");
}

// --- clap surface: pin the `gwm exec [<slug>...] -- <cmd>` grammar ---

#[test]
fn parses_command_after_double_dash_with_no_slugs() {
  let cli = Cli::try_parse_from(["gwm", "exec", "--", "git", "fetch"]).expect("should parse");
  match cli.command {
    Some(Command::Exec { slugs, command }) => {
      assert!(slugs.is_empty(), "no slugs before `--`");
      assert_eq!(command, vec!["git".to_string(), "fetch".to_string()]);
    }
    other => panic!("expected Exec, got {other:?}"),
  }
}

#[test]
fn parses_slugs_before_double_dash_and_command_after() {
  let cli = Cli::try_parse_from(["gwm", "exec", "feat-1", "fix-2", "--", "cargo", "check"]).expect("should parse");
  match cli.command {
    Some(Command::Exec { slugs, command }) => {
      assert_eq!(slugs, vec!["feat-1".to_string(), "fix-2".to_string()]);
      assert_eq!(command, vec!["cargo".to_string(), "check".to_string()]);
    }
    other => panic!("expected Exec, got {other:?}"),
  }
}

#[test]
fn keeps_hyphenated_flags_in_the_command_intact() {
  let cli = Cli::try_parse_from(["gwm", "exec", "--", "git", "log", "--oneline"]).expect("should parse");
  match cli.command {
    Some(Command::Exec { command, .. }) => {
      assert_eq!(
        command,
        vec!["git".to_string(), "log".to_string(), "--oneline".to_string()]
      );
    }
    other => panic!("expected Exec, got {other:?}"),
  }
}

#[test]
fn rejects_exec_with_no_command() {
  // `--` is required: `gwm exec` alone (or with only slugs) must error.
  assert!(Cli::try_parse_from(["gwm", "exec"]).is_err());
}
