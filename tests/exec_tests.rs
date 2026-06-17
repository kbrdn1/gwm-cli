//! Unit tests for `gwm exec` (issue #313) — the pure command fan-out layer.
//!
//! The CLI wiring (worktree selection, printing) lives in `cli.rs`; the
//! testable surface is the spawn primitive (`exec_in_dir`), the exit-code
//! rollup, and the per-worktree line formatter — none of which need a git
//! repo. Spawning uses `sh -c`, present at `/bin/sh` even on a stripped
//! CI PATH, so these stay environment-independent (CLAUDE.md).

use clap::Parser;
use gwm::cli::{Cli, Command};
use gwm::exec::{exec_in_dir, format_outcome, resolve_program, rollup_exit_code, ExecOutcome, ExecStatus};
use std::path::{Path, PathBuf};
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
fn resolve_program_leaves_bare_names_as_path_lookups() {
  // No separator ⇒ PATH lookup, untouched (must not become `dir/git`).
  let dir = Path::new("/some/worktree");
  assert_eq!(resolve_program(dir, "git"), PathBuf::from("git"));
}

#[test]
fn resolve_program_joins_relative_paths_onto_the_worktree() {
  let dir = Path::new("/some/worktree");
  assert_eq!(resolve_program(dir, "./build.sh"), dir.join("./build.sh"));
  assert_eq!(resolve_program(dir, "scripts/run"), dir.join("scripts/run"));
}

#[test]
fn resolve_program_leaves_absolute_paths_unchanged() {
  let dir = Path::new("/some/worktree");
  assert_eq!(resolve_program(dir, "/usr/bin/env"), PathBuf::from("/usr/bin/env"));
}

#[cfg(unix)]
#[test]
fn exec_in_dir_runs_a_relative_script_from_the_worktree() {
  use std::os::unix::fs::PermissionsExt;
  let dir = TempDir::new().unwrap();
  let script = dir.path().join("run.sh");
  std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
  std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
  // `./run.sh` only resolves if it is anchored to the worktree dir.
  let status = exec_in_dir(dir.path(), "./run.sh", &[]);
  assert_eq!(
    status,
    ExecStatus::Ok,
    "a worktree-relative script must resolve against dir"
  );
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
