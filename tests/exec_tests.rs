//! Unit tests for `gwm exec` (issue #313) — the pure command fan-out layer.
//!
//! The CLI wiring (worktree selection, printing) lives in `cli.rs`; the
//! testable surface is the spawn primitive (`exec_in_dir`), the exit-code
//! rollup, and the per-worktree line formatter — none of which need a git
//! repo. Spawning uses `sh -c`, present at `/bin/sh` even on a stripped
//! CI PATH, so these stay environment-independent (CLAUDE.md).

use clap::Parser;
use gwm::cli::{Cli, Command};
use gwm::config::{ExecConfig, ExecProfile};
use gwm::exec::{
  exec_capture_in_dir, exec_in_dir, format_outcome, resolve_exec_command, resolve_jobs, resolve_program,
  rollup_exit_code, run_in_dirs_parallel, ExecOutcome, ExecStatus,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Build an [`ExecConfig`] with the given `[exec.profiles.*]` entries (no
/// `jobs` set anywhere).
fn exec_cfg(profiles: &[(&str, &[&str])]) -> ExecConfig {
  let mut map = BTreeMap::new();
  for (name, argv) in profiles {
    map.insert(
      (*name).to_string(),
      ExecProfile {
        command: argv.iter().map(|s| s.to_string()).collect(),
        jobs: None,
      },
    );
  }
  ExecConfig {
    jobs: None,
    profiles: map,
  }
}

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
  //
  // Retried on `ETXTBSY`, and that is not flake-hiding (issue #500). `execve`
  // refuses a file that is open for writing by ANY process, and the window is
  // between another thread's `fork` and its own `execve`: a child forked by
  // one of the other spawning tests in this binary carries a copy of every fd
  // the harness had open at fork time, including the write handle this test
  // just used for `run.sh`. The errno therefore says nothing about
  // `exec_in_dir` — it is inherent to writing an executable and running it
  // from a multi-threaded harness. Matching `(os error N)` rather than the
  // message: `strerror` prose is localised, the suffix is not.
  //
  // Every other spawn error still fails on the first attempt, because the
  // assertion below is unchanged.
  let mut status = exec_in_dir(dir.path(), "./run.sh", &[]);
  for _ in 0..10 {
    match &status {
      ExecStatus::SpawnError(e) if e.contains("(os error 26)") => {
        std::thread::sleep(std::time::Duration::from_millis(20));
        status = exec_in_dir(dir.path(), "./run.sh", &[]);
      }
      _ => break,
    }
  }

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
    Some(Command::Exec {
      slugs,
      profile,
      jobs,
      command,
    }) => {
      assert!(slugs.is_empty(), "no slugs before `--`");
      assert!(profile.is_none(), "no --profile given");
      assert!(jobs.is_none(), "no --jobs given");
      assert_eq!(command, vec!["git".to_string(), "fetch".to_string()]);
    }
    other => panic!("expected Exec, got {other:?}"),
  }
}

#[test]
fn parses_jobs_flag() {
  let cli = Cli::try_parse_from(["gwm", "exec", "--jobs", "4", "--", "echo", "hi"]).expect("should parse");
  match cli.command {
    Some(Command::Exec { jobs, command, .. }) => {
      assert_eq!(jobs, Some(4));
      assert_eq!(command, vec!["echo".to_string(), "hi".to_string()]);
    }
    other => panic!("expected Exec, got {other:?}"),
  }
}

// --- jobs precedence + parallel runner (issue #324) ------------------------

/// ExecConfig with an explicit global `jobs` and per-profile `(name, argv, jobs)`.
fn exec_cfg_jobs(global: Option<u32>, profiles: &[(&str, &[&str], Option<u32>)]) -> ExecConfig {
  let mut map = BTreeMap::new();
  for (name, argv, jobs) in profiles {
    map.insert(
      (*name).to_string(),
      ExecProfile {
        command: argv.iter().map(|s| s.to_string()).collect(),
        jobs: *jobs,
      },
    );
  }
  ExecConfig {
    jobs: global,
    profiles: map,
  }
}

#[test]
fn resolve_jobs_defaults_to_one() {
  assert_eq!(resolve_jobs(None, None, &exec_cfg(&[])), 1);
}

#[test]
fn resolve_jobs_flag_wins_over_profile_and_global() {
  let cfg = exec_cfg_jobs(Some(2), &[("p", &["true"], Some(3))]);
  assert_eq!(resolve_jobs(Some(8), Some("p"), &cfg), 8);
}

#[test]
fn resolve_jobs_profile_overrides_global() {
  let cfg = exec_cfg_jobs(Some(2), &[("p", &["true"], Some(4))]);
  assert_eq!(resolve_jobs(None, Some("p"), &cfg), 4);
}

#[test]
fn resolve_jobs_falls_back_global_then_one() {
  let cfg = exec_cfg_jobs(Some(5), &[("p", &["true"], None)]);
  assert_eq!(resolve_jobs(None, Some("p"), &cfg), 5, "profile without jobs → global");
  assert_eq!(resolve_jobs(None, None, &cfg), 5, "inline → global");
  assert_eq!(resolve_jobs(None, None, &exec_cfg_jobs(None, &[])), 1, "nothing → 1");
}

#[test]
fn resolve_jobs_clamps_zero_to_one() {
  let cfg = exec_cfg_jobs(Some(0), &[]);
  assert_eq!(resolve_jobs(Some(0), None, &cfg), 1);
  assert_eq!(resolve_jobs(None, None, &cfg), 1);
}

#[test]
fn exec_capture_captures_stdout() {
  let dir = TempDir::new().unwrap();
  let (status, out) = exec_capture_in_dir(dir.path(), "sh", &["-c".into(), "echo captured".into()]);
  assert_eq!(status, ExecStatus::Ok);
  assert!(
    String::from_utf8_lossy(&out).contains("captured"),
    "stdout should be captured: {out:?}"
  );
}

#[test]
fn exec_capture_reports_exit_code_and_stderr() {
  let dir = TempDir::new().unwrap();
  let (status, out) = exec_capture_in_dir(dir.path(), "sh", &["-c".into(), "echo oops 1>&2; exit 3".into()]);
  assert_eq!(status, ExecStatus::Failed(3));
  assert!(
    String::from_utf8_lossy(&out).contains("oops"),
    "stderr should be captured too: {out:?}"
  );
}

#[test]
fn run_parallel_captures_each_dirs_output_in_input_order() {
  // Each dir holds an `id.txt`; running `cat id.txt` in each (cwd = the dir)
  // proves per-worktree capture, and the results must come back in INPUT
  // order regardless of completion order (bounded at 2 workers).
  let dirs: Vec<TempDir> = (0..3).map(|_| TempDir::new().unwrap()).collect();
  let items: Vec<(String, PathBuf)> = dirs
    .iter()
    .enumerate()
    .map(|(i, d)| {
      std::fs::write(d.path().join("id.txt"), format!("dir-{i}")).unwrap();
      (format!("wt{i}"), d.path().to_path_buf())
    })
    .collect();

  let results = run_in_dirs_parallel(2, &items, "sh", &["-c".into(), "cat id.txt".into()]);

  assert_eq!(results.len(), 3);
  for (i, (outcome, output)) in results.iter().enumerate() {
    assert_eq!(outcome.name, format!("wt{i}"), "results stay in input order");
    assert_eq!(outcome.status, ExecStatus::Ok);
    assert!(
      String::from_utf8_lossy(output).contains(&format!("dir-{i}")),
      "block {i} must carry its own dir's output"
    );
  }
}

#[test]
fn run_parallel_clamps_jobs_above_item_count() {
  let dir = TempDir::new().unwrap();
  let items = vec![("only".to_string(), dir.path().to_path_buf())];
  // jobs far exceeds the single item — must not panic, returns the one result.
  let results = run_in_dirs_parallel(64, &items, "sh", &["-c".into(), "true".into()]);
  assert_eq!(results.len(), 1);
  assert_eq!(results[0].0.status, ExecStatus::Ok);
}

#[test]
fn run_parallel_on_empty_items_returns_empty() {
  let results = run_in_dirs_parallel(4, &[], "true", &[]);
  assert!(results.is_empty());
}

#[test]
fn parses_slugs_before_double_dash_and_command_after() {
  let cli = Cli::try_parse_from(["gwm", "exec", "feat-1", "fix-2", "--", "cargo", "check"]).expect("should parse");
  match cli.command {
    Some(Command::Exec { slugs, command, .. }) => {
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
fn parses_profile_flag_with_no_inline_command() {
  // `--profile` no longer needs an inline `-- <cmd>`; the profile carries it.
  let cli = Cli::try_parse_from(["gwm", "exec", "--profile", "test"]).expect("should parse");
  match cli.command {
    Some(Command::Exec { profile, command, .. }) => {
      assert_eq!(profile.as_deref(), Some("test"));
      assert!(command.is_empty(), "no inline command alongside --profile");
    }
    other => panic!("expected Exec, got {other:?}"),
  }
}

#[test]
fn exec_with_no_command_and_no_profile_parses_but_resolves_to_an_error() {
  // `command` is no longer `required` at the clap layer (a `--profile` can
  // stand in), so `gwm exec` alone now *parses*. The "nothing to run"
  // rejection moved to runtime resolution (exit 1) — pinned below.
  let cli = Cli::try_parse_from(["gwm", "exec"]).expect("parses with neither source");
  match cli.command {
    Some(Command::Exec { profile, command, .. }) => {
      assert!(profile.is_none());
      assert!(command.is_empty());
    }
    other => panic!("expected Exec, got {other:?}"),
  }
}

// --- profile/inline resolution (issue #324) ---------------------------------

#[test]
fn resolve_uses_the_inline_command_when_no_profile() {
  let cfg = exec_cfg(&[]);
  let argv = resolve_exec_command(None, &["git".into(), "fetch".into()], &cfg).expect("inline resolves");
  assert_eq!(argv, vec!["git".to_string(), "fetch".to_string()]);
}

#[test]
fn resolve_uses_the_profile_command_array() {
  let cfg = exec_cfg(&[("test", &["cargo", "test"])]);
  let argv = resolve_exec_command(Some("test"), &[], &cfg).expect("profile resolves");
  assert_eq!(argv, vec!["cargo".to_string(), "test".to_string()]);
}

#[test]
fn resolve_rejects_profile_and_inline_together() {
  let cfg = exec_cfg(&[("test", &["cargo", "test"])]);
  let err =
    resolve_exec_command(Some("test"), &["echo".into(), "hi".into()], &cfg).expect_err("both sources must be rejected");
  assert!(
    err.to_string().contains("mutually exclusive"),
    "error should explain the exclusivity: {err}"
  );
}

#[test]
fn resolve_rejects_an_unknown_profile() {
  let cfg = exec_cfg(&[("test", &["cargo", "test"])]);
  let err = resolve_exec_command(Some("nope"), &[], &cfg).expect_err("unknown profile must error");
  assert!(
    err.to_string().contains("nope") && err.to_string().contains("profile"),
    "error should name the missing profile: {err}"
  );
}

#[test]
fn resolve_rejects_neither_profile_nor_inline() {
  let cfg = exec_cfg(&[]);
  let err = resolve_exec_command(None, &[], &cfg).expect_err("nothing to run must error");
  assert!(
    err.to_string().contains("--profile") || err.to_string().contains("command"),
    "error should hint at the two sources: {err}"
  );
}

#[test]
fn resolve_rejects_a_profile_with_an_empty_command() {
  let cfg = exec_cfg(&[("empty", &[])]);
  let err = resolve_exec_command(Some("empty"), &[], &cfg).expect_err("empty command must error");
  assert!(
    err.to_string().contains("empty"),
    "error should flag the empty command: {err}"
  );
}
