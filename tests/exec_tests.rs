//! Unit tests for `gwm exec` (issue #313) — the pure command fan-out layer.
//!
//! The CLI wiring (worktree selection, printing) lives in `cli.rs`; the
//! testable surface is the spawn primitive (`exec_in_dir`), the exit-code
//! rollup, and the per-worktree line formatter — none of which need a git
//! repo. Spawning uses `sh -c`, present at `/bin/sh` even on a stripped
//! CI PATH, so these stay environment-independent (CLAUDE.md).

mod common;

use clap::Parser;
use gwm::cli::{Cli, Command};
use gwm::config::{ContainerConfig, ExecConfig, ExecProfile};
use gwm::exec::{
  build_container_argv, exec_capture_in_dir, exec_in_dir, format_outcome, resolve_container_runtime,
  resolve_exec_command, resolve_exec_container, resolve_jobs, resolve_program, rollup_exit_code, run_in_dirs_parallel,
  validate_exec_profile, ContainerPlan, ExecOutcome, ExecStatus, CONTAINER_RUNTIMES,
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
        container: None,
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
        container: None,
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
  let items: Vec<(String, PathBuf, Vec<String>)> = dirs
    .iter()
    .enumerate()
    .map(|(i, d)| {
      std::fs::write(d.path().join("id.txt"), format!("dir-{i}")).unwrap();
      (
        format!("wt{i}"),
        d.path().to_path_buf(),
        vec!["sh".to_string(), "-c".to_string(), "cat id.txt".to_string()],
      )
    })
    .collect();

  let results = run_in_dirs_parallel(2, &items);

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
  let items = vec![(
    "only".to_string(),
    dir.path().to_path_buf(),
    vec!["sh".to_string(), "-c".to_string(), "true".to_string()],
  )];
  // jobs far exceeds the single item — must not panic, returns the one result.
  let results = run_in_dirs_parallel(64, &items);
  assert_eq!(results.len(), 1);
  assert_eq!(results[0].0.status, ExecStatus::Ok);
}

#[test]
fn run_parallel_on_empty_items_returns_empty() {
  let results = run_in_dirs_parallel(4, &[]);
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

// --- container execution (issue #421) --------------------------------------
//
// `build_container_argv` is pure — no spawn, no `PATH` read, no filesystem —
// so these are hermetic on a CI runner that has neither docker nor podman.
// The one test that touches disk builds a REAL linked worktree, because the
// bug this feature exists to avoid is only visible there: a linked worktree's
// `.git` is a file holding an absolute host path.

/// A minimal `[container]` block: just the required image.
fn container_cfg(image: &str) -> ContainerConfig {
  ContainerConfig {
    image: image.to_string(),
    runtime: None,
    extra_args: Vec::new(),
    selinux_relabel: false,
  }
}

/// `&["a", "b"]` → `vec!["a".to_string(), "b".to_string()]`.
fn argv(tokens: &[&str]) -> Vec<String> {
  tokens.iter().map(|s| s.to_string()).collect()
}

/// The `-v` mount sources in `out`, in order (`-v <src>:<dst>` → `<src>`).
fn mount_sources(out: &[String]) -> Vec<String> {
  out
    .windows(2)
    .filter(|w| w[0] == "-v")
    .filter_map(|w| w[1].rsplit_once(':').map(|(src, _)| src.to_string()))
    .collect()
}

#[cfg(unix)]
#[test]
fn container_argv_mirrors_host_paths_and_runs_the_command_as_the_container_cmd() {
  let out = build_container_argv(
    "docker",
    &container_cfg("rust:1.90"),
    Path::new("/wt/feat-1"),
    Path::new("/main/.git"),
    &argv(&["cargo", "test"]),
    None,
  )
  .unwrap();
  assert_eq!(
    out,
    argv(&[
      "docker",
      "run",
      "--rm",
      "-v",
      "/wt/feat-1:/wt/feat-1",
      "-v",
      "/main/.git:/main/.git",
      "-w",
      "/wt/feat-1",
      "-e",
      "GIT_CONFIG_COUNT=2",
      "-e",
      "GIT_CONFIG_KEY_0=safe.directory",
      "-e",
      "GIT_CONFIG_VALUE_0=/wt/feat-1",
      "-e",
      "GIT_CONFIG_KEY_1=safe.directory",
      "-e",
      "GIT_CONFIG_VALUE_1=/main/.git",
      "rust:1.90",
      "cargo",
      "test",
    ]),
    "host paths are mirrored (no /workspace), the image comes last before the command"
  );
}

#[cfg(unix)]
#[test]
fn container_argv_mounts_the_main_checkout_gitdir_for_a_linked_worktree() {
  // THE test of this feature. A linked worktree's `.git` is a file holding
  // the absolute HOST path of `<main>/.git/worktrees/<id>`; mount the
  // worktree alone and that path does not exist in the container, so no git
  // command answers inside it. The assertion is therefore not "an extra -v is
  // present" but "the path git will follow is covered by a mount".
  let (main_dir, repo) = common::init_repo();
  let wt_parent = TempDir::new().unwrap();
  let wt_path = wt_parent.path().join("feat-1");
  repo.worktree("feat-1", &wt_path, None).expect("linked worktree");

  let dot_git = wt_path.join(".git");
  assert!(dot_git.is_file(), "a linked worktree's .git is a file, not a directory");
  let gitdir = std::fs::read_to_string(&dot_git).unwrap();
  let referenced = gitdir
    .trim()
    .strip_prefix("gitdir:")
    .expect("the .git file points at the admin dir")
    .trim()
    .to_string();

  let linked = git2::Repository::open(&wt_path).unwrap();
  let plan = ContainerPlan::resolve(container_cfg("rust:1.90"), linked.commondir(), |_| true).unwrap();
  let out = plan.wrap(&wt_path, &argv(&["git", "status"])).unwrap();

  // Canonicalise both sides: on macOS a TempDir is `/var/…` while git may
  // record `/private/var/…`, and this asserts coverage, not string equality.
  let referenced = Path::new(&referenced).canonicalize().unwrap();
  let covered = mount_sources(&out)
    .iter()
    .filter_map(|src| Path::new(src).canonicalize().ok())
    .any(|src| referenced.starts_with(&src));
  assert!(
    covered,
    "no mount covers {} — git would not answer inside the container. argv: {out:?}",
    referenced.display()
  );
  // And the mount is the main checkout's gitdir, not the worktree's own path.
  assert!(
    referenced.starts_with(main_dir.path().canonicalize().unwrap().join(".git")),
    "the referenced admin dir lives under the main checkout's .git"
  );
}

#[cfg(unix)]
#[test]
fn container_argv_skips_the_gitdir_mount_when_it_lives_inside_the_worktree() {
  // The main checkout (reachable via an explicit slug — `find_fuzzy` does not
  // filter it out) carries its own `.git`, so the first mount already covers
  // it. A second, nested mount would be redundant.
  let out = build_container_argv(
    "podman",
    &container_cfg("alpine"),
    Path::new("/repo"),
    Path::new("/repo/.git"),
    &argv(&["true"]),
    None,
  )
  .unwrap();
  assert_eq!(
    mount_sources(&out),
    vec!["/repo".to_string()],
    "one mount only, the worktree's own path: {out:?}"
  );
  // The main worktree's path comes out of git2 with a TRAILING SEPARATOR, and
  // the containment check runs on the raw path (`Path::starts_with` compares
  // components, so it holds) while the mount is normalised.
  let out = build_container_argv(
    "podman",
    &container_cfg("alpine"),
    Path::new("/repo/"),
    Path::new("/repo/.git"),
    &argv(&["true"]),
    None,
  )
  .unwrap();
  assert_eq!(
    mount_sources(&out),
    vec!["/repo".to_string()],
    "a trailing separator changes neither the dedupe nor the mount: {out:?}"
  );
}

#[cfg(unix)]
#[test]
fn container_argv_places_extra_args_after_gwms_flags_and_before_the_image() {
  let cfg = ContainerConfig {
    image: "node:22".to_string(),
    runtime: None,
    extra_args: argv(&["-e", "CI=1", "--network", "none"]),
    selinux_relabel: false,
  };
  let out = build_container_argv(
    "docker",
    &cfg,
    Path::new("/wt/x"),
    Path::new("/main/.git"),
    &argv(&["npm", "test"]),
    None,
  )
  .unwrap();
  let image_at = out.iter().position(|t| t == "node:22").expect("image present");
  let extra_at = out.iter().position(|t| t == "--network").expect("extra arg present");
  let w_at = out.iter().position(|t| t == "-w").expect("-w present");
  assert!(w_at < extra_at, "extra_args come after gwm's own flags, so they win");
  assert!(
    extra_at < image_at,
    "extra_args are `run` flags, so they precede the image"
  );
  assert_eq!(
    &out[image_at + 1..],
    &argv(&["npm", "test"])[..],
    "command follows the image"
  );
}

#[cfg(unix)]
#[test]
fn container_argv_never_quotes_or_joins_a_token() {
  // The invariant: gwm builds argv, never a shell string (the reference
  // implementation joins + shell-quotes because it feeds tmux). A token with
  // spaces and metachars must survive as ONE token, unquoted.
  let nasty = "hello world; rm -rf /".to_string();
  let out = build_container_argv(
    "docker",
    &container_cfg("alpine"),
    Path::new("/wt/x"),
    Path::new("/main/.git"),
    &["echo".to_string(), nasty.clone()],
    None,
  )
  .unwrap();
  assert_eq!(out.last().unwrap(), &nasty, "the token is passed through verbatim");
  assert!(
    !out.iter().any(|t| t.contains('\'') || t.contains('\\')),
    "no token is shell-quoted: {out:?}"
  );
}

#[cfg(unix)]
#[test]
fn container_plan_normalises_a_trailing_separator_on_the_common_dir() {
  // git2's `commondir()` returns `<main>/.git/`; the mount must read as a
  // path, not as a directory listing.
  let plan = ContainerPlan::resolve(container_cfg("alpine"), Path::new("/main/.git/"), |_| true).unwrap();
  assert_eq!(plan.common_dir, PathBuf::from("/main/.git"));
  let out = plan.wrap(Path::new("/wt/x"), &argv(&["true"])).unwrap();
  assert!(
    out.contains(&"/main/.git:/main/.git".to_string()),
    "mount carries no trailing separator: {out:?}"
  );
}

#[test]
fn runtime_detection_prefers_docker_then_podman() {
  assert_eq!(resolve_container_runtime(None, |_| true).unwrap(), "docker");
  assert_eq!(
    resolve_container_runtime(None, |bin| bin == "podman").unwrap(),
    "podman",
    "podman is the fallback, not the preference"
  );
  assert_eq!(CONTAINER_RUNTIMES, &["docker", "podman"]);
}

#[test]
fn an_explicit_runtime_wins_even_when_absent_from_path() {
  // A missing binary reports better as a spawn error naming it than as a
  // config error second-guessing the user (`nerdctl`, a wrapper script, …).
  assert_eq!(
    resolve_container_runtime(Some("nerdctl"), |_| false).unwrap(),
    "nerdctl"
  );
}

#[test]
fn runtime_detection_errors_when_no_runtime_is_available() {
  let err = resolve_container_runtime(None, |_| false).expect_err("no runtime must error");
  let msg = err.to_string();
  assert!(
    msg.contains("docker") && msg.contains("podman") && msg.contains("runtime"),
    "error should name what was looked for and the way out: {msg}"
  );
}

/// An [`ExecConfig`] with one profile carrying a `[container]` block.
fn exec_cfg_container(name: &str, command: &[&str], container: Option<ContainerConfig>) -> ExecConfig {
  let mut map = BTreeMap::new();
  map.insert(
    name.to_string(),
    ExecProfile {
      command: argv(command),
      jobs: None,
      container,
    },
  );
  ExecConfig {
    jobs: None,
    profiles: map,
  }
}

#[test]
fn the_inline_surface_is_never_containerised() {
  // The frozen 1.0 surface (#319): `gwm exec -- <cmd>` runs on the host, and
  // no config can change that. The block rides a profile the user has to name.
  let cfg = exec_cfg_container("test", &["cargo", "test"], Some(container_cfg("rust:1.90")));
  assert_eq!(
    resolve_exec_container(None, &cfg).unwrap(),
    None,
    "an inline command is not containerised even when a profile declares one"
  );
  assert_eq!(
    resolve_exec_command(None, &argv(&["cargo", "check"]), &cfg).unwrap(),
    argv(&["cargo", "check"]),
    "and the inline argv is forwarded verbatim, unchanged by #421"
  );
}

#[test]
fn a_named_profile_carries_its_container_block() {
  let cfg = exec_cfg_container("test", &["cargo", "test"], Some(container_cfg("rust:1.90")));
  let resolved = resolve_exec_container(Some("test"), &cfg).unwrap();
  assert_eq!(resolved.map(|c| c.image), Some("rust:1.90".to_string()));
}

#[test]
fn a_profile_without_a_container_block_runs_on_the_host() {
  let cfg = exec_cfg_container("test", &["cargo", "test"], None);
  assert_eq!(resolve_exec_container(Some("test"), &cfg).unwrap(), None);
}

#[test]
fn a_container_block_with_an_empty_image_is_rejected() {
  let cfg = exec_cfg_container("test", &["cargo", "test"], Some(container_cfg("  ")));
  let err = resolve_exec_container(Some("test"), &cfg).expect_err("empty image must error");
  assert!(
    err.to_string().contains("image") && err.to_string().contains("test"),
    "error should name the field and the profile: {err}"
  );
  // Same rejection on the config-validation path, so `gwm config validate` /
  // `gwm doctor` refuse exactly what `gwm exec --profile` would.
  let err = validate_exec_profile("test", cfg.profiles.get("test").unwrap()).expect_err("validator must agree");
  assert!(
    err.to_string().contains("image"),
    "validator flags the empty image: {err}"
  );
}

#[cfg(unix)]
#[test]
fn container_argv_declares_every_mounted_path_safe_for_git() {
  // With a rootful Docker on Linux the container runs as uid 0 while the
  // bind-mounted tree belongs to the host user, and git refuses a repository
  // it reads as `dubious ownership` — which would undo the gitdir mount this
  // feature exists for. The declaration rides `GIT_CONFIG_*` env (nothing
  // written to disk) and names ONLY the paths gwm mounts, never the blanket
  // `*`.
  let out = build_container_argv(
    "docker",
    &container_cfg("rust:1.90"),
    Path::new("/wt/feat-1"),
    Path::new("/main/.git"),
    &argv(&["git", "status"]),
    None,
  )
  .unwrap();
  let env: Vec<&String> = out.windows(2).filter(|w| w[0] == "-e").map(|w| &w[1]).collect();
  assert!(env.contains(&&"GIT_CONFIG_COUNT=2".to_string()), "{out:?}");
  assert!(env.contains(&&"GIT_CONFIG_KEY_0=safe.directory".to_string()), "{out:?}");
  assert!(env.contains(&&"GIT_CONFIG_VALUE_0=/wt/feat-1".to_string()), "{out:?}");
  assert!(env.contains(&&"GIT_CONFIG_KEY_1=safe.directory".to_string()), "{out:?}");
  assert!(env.contains(&&"GIT_CONFIG_VALUE_1=/main/.git".to_string()), "{out:?}");
  assert!(
    !out
      .iter()
      .any(|t| t.ends_with("safe.directory=*") || t == "GIT_CONFIG_VALUE_0=*"),
    "the ownership check stays on for every path gwm did not mount: {out:?}"
  );
  // The declared set is exactly the mounted set: one mount ⇒ one entry.
  let out = build_container_argv(
    "docker",
    &container_cfg("alpine"),
    Path::new("/repo"),
    Path::new("/repo/.git"),
    &argv(&["true"]),
    None,
  )
  .unwrap();
  assert!(
    out.contains(&"GIT_CONFIG_COUNT=1".to_string()),
    "the deduped mount declares one path, not two: {out:?}"
  );
}

#[cfg(unix)]
#[test]
fn only_the_interactive_wrap_allocates_a_tty() {
  // `gwm exec` is a fan-out over N worktrees, where a TTY per container means
  // nothing; the TUI overlay spawns into a real pty, where its absence would
  // cost a REPL its stdin and its terminal.
  let plan = ContainerPlan::resolve(container_cfg("rust:1.90"), Path::new("/main/.git"), |_| true).unwrap();
  let fanout = plan.wrap(Path::new("/wt/x"), &argv(&["cargo", "test"])).unwrap();
  let overlay = plan
    .wrap_interactive(Path::new("/wt/x"), &argv(&["cargo", "test"]), "gwm-x-1")
    .unwrap();

  assert!(
    !fanout.contains(&"-t".to_string()) && !fanout.contains(&"-i".to_string()),
    "the fan-out allocates no tty: {fanout:?}"
  );
  assert_eq!(
    &overlay[..7],
    &argv(&["docker", "run", "--rm", "-i", "-t", "--name", "gwm-x-1"])[..],
    "the overlay asks for stdin, a tty and a name, right after `run`: {overlay:?}"
  );
  // Nothing else differs: same mounts, same env, same command. The name is
  // what lets the overlay tear the container down on close; the fan-out never
  // kills its client mid-run, so it needs none.
  let mut skip_next = false;
  let strip: Vec<String> = overlay
    .iter()
    .filter(|t| {
      if skip_next {
        skip_next = false;
        return false;
      }
      if *t == "--name" {
        skip_next = true;
        return false;
      }
      *t != "-i" && *t != "-t"
    })
    .cloned()
    .collect();
  assert_eq!(strip, fanout, "the tty flags and the name are the only difference");
}

#[cfg(windows)]
#[test]
fn a_container_profile_is_refused_on_windows() {
  // The wrapper mirrors host paths, and `C:\…` is neither mountable nor
  // resolvable inside a Linux container — and the worktree's `.git` file
  // would still name a Windows path, so git could not answer even with a
  // translated mount. Refused with a message that says so, rather than
  // handed to `docker run` to fail obscurely.
  let err = ContainerPlan::resolve(container_cfg("rust:1.90"), Path::new("C:\\main\\.git"), |_| true)
    .expect_err("a container profile must be refused on Windows");
  let msg = err.to_string();
  assert!(
    msg.contains("Windows") && msg.contains("container"),
    "the error names the platform and the feature: {msg}"
  );
}

#[cfg(unix)]
#[test]
fn a_mount_path_holding_a_colon_is_refused_with_a_reason() {
  // `:` is legal in a Unix path but is the field separator of
  // `-v source:destination`, so the mount cannot be expressed. The runtime
  // would reject the spec with a message about neither the worktree nor gwm.
  let err = build_container_argv(
    "docker",
    &container_cfg("alpine"),
    Path::new("/wt/od:d"),
    Path::new("/main/.git"),
    &argv(&["true"]),
    None,
  )
  .expect_err("a colon in the worktree path must be refused");
  let msg = err.to_string();
  assert!(
    msg.contains("od:d") && msg.contains(':'),
    "the error names the path: {msg}"
  );

  // The gitdir side is checked too, not only the worktree.
  let err = build_container_argv(
    "docker",
    &container_cfg("alpine"),
    Path::new("/wt/x"),
    Path::new("/main:repo/.git"),
    &argv(&["true"]),
    None,
  )
  .expect_err("a colon in the gitdir path must be refused too");
  assert!(err.to_string().contains("main:repo"), "{err}");

  // And a path without one still builds.
  assert!(build_container_argv(
    "docker",
    &container_cfg("alpine"),
    Path::new("/wt/x"),
    Path::new("/main/.git"),
    &argv(&["true"]),
    None,
  )
  .is_ok());
}

#[cfg(unix)]
#[test]
fn selinux_relabel_suffixes_every_mount_gwm_builds() {
  // `extra_args` cannot reach the mounts gwm builds itself, so an
  // SELinux-enforcing host (Fedora, RHEL) has no way to relabel them without
  // this field — and without a relabel the container gets EACCES on both.
  let cfg = ContainerConfig {
    image: "fedora:41".to_string(),
    runtime: None,
    extra_args: Vec::new(),
    selinux_relabel: true,
  };
  let out = build_container_argv(
    "podman",
    &cfg,
    Path::new("/wt/feat-1"),
    Path::new("/main/.git"),
    &argv(&["true"]),
    None,
  )
  .unwrap();
  let mounts: Vec<&String> = out.windows(2).filter(|w| w[0] == "-v").map(|w| &w[1]).collect();
  assert_eq!(
    mounts,
    vec![
      &"/wt/feat-1:/wt/feat-1:z".to_string(),
      &"/main/.git:/main/.git:z".to_string()
    ],
    "both mounts carry `:z`: {out:?}"
  );
  // Off by default: relabelling writes to the host, recursively.
  let out = build_container_argv(
    "podman",
    &container_cfg("fedora:41"),
    Path::new("/wt/feat-1"),
    Path::new("/main/.git"),
    &argv(&["true"]),
    None,
  )
  .unwrap();
  assert!(
    !out.iter().any(|t| t.ends_with(":z")),
    "no relabel unless asked: {out:?}"
  );
}

#[cfg(unix)]
#[test]
fn the_interactive_wrap_names_the_container_so_it_can_be_torn_down() {
  // Killing the pty leader kills the `docker` client, not the container: the
  // daemon owns it and `--rm` only fires when it exits. Without a name there
  // is nothing to remove, and a long command keeps writing to the worktree
  // after the overlay closed.
  let plan = ContainerPlan::resolve(container_cfg("rust:1.90"), Path::new("/main/.git"), |_| true).unwrap();
  let name = gwm::exec::container_run_name(Path::new("/wt/feat-421-container-exec"), 4242, 3);
  assert_eq!(name, "gwm-feat-421-container-exec-4242-3");

  let out = plan
    .wrap_interactive(Path::new("/wt/feat-1"), &argv(&["cargo", "test"]), &name)
    .unwrap();
  assert!(
    out.windows(2).any(|w| w[0] == "--name" && w[1] == name),
    "the run is named: {out:?}"
  );
  assert_eq!(
    plan.container_teardown_argv(&name),
    argv(&["docker", "rm", "-f", "gwm-feat-421-container-exec-4242-3"]),
    "and the teardown removes exactly that name, with the same runtime"
  );
  // The fan-out form names nothing: its client is never killed mid-run.
  let fanout = plan.wrap(Path::new("/wt/feat-1"), &argv(&["cargo", "test"])).unwrap();
  assert!(!fanout.contains(&"--name".to_string()), "{fanout:?}");
}

#[cfg(unix)]
#[test]
fn a_container_name_is_reduced_to_the_accepted_character_class() {
  // A container name must match `[a-zA-Z0-9][a-zA-Z0-9_.-]*`. A worktree
  // directory is not held to that: `gwm link` adopts any path.
  let name = gwm::exec::container_run_name(Path::new("/wt/feat/#421 spike:x"), 9, 1);
  assert_eq!(name, "gwm--421-spike-x-9-1");
  assert!(
    name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()),
    "starts with an alphanumeric: {name}"
  );
  assert!(
    name
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'),
    "every character is accepted: {name}"
  );
  // An unnameable directory still yields a legal name.
  assert_eq!(gwm::exec::container_run_name(Path::new("/"), 5, 7), "gwm--5-7");
}

#[cfg(unix)]
#[test]
fn a_container_name_separates_two_gwm_processes() {
  // The seq restarts at 1 in every process, so without the pid two TUIs
  // opening their first overlay on the same worktree would agree on a name.
  // The loser of that race does not merely fail to start: its teardown then
  // removes the winner's container.
  let a = gwm::exec::container_run_name(Path::new("/wt/feat-1"), 111, 1);
  let b = gwm::exec::container_run_name(Path::new("/wt/feat-1"), 222, 1);
  assert_ne!(a, b, "two processes, same worktree, same seq: {a} vs {b}");
  assert_eq!(a, "gwm-feat-1-111-1");
  // And within one process the seq still separates two overlays.
  assert_ne!(
    gwm::exec::container_run_name(Path::new("/wt/feat-1"), 111, 2),
    a,
    "two overlays in one process"
  );
}

#[cfg(unix)]
#[test]
fn extra_args_may_not_take_over_the_container_name() {
  // A runtime honours the LAST `--name`, so an `extra_args` one would leave
  // the overlay's teardown removing a container that was never started, and
  // possibly one belonging to something else.
  let cfg = ContainerConfig {
    image: "rust:1.90".to_string(),
    runtime: None,
    extra_args: argv(&["--name", "custom"]),
    selinux_relabel: false,
  };
  let err = gwm::exec::validate_container("ci", &cfg).expect_err("`--name` in extra_args must be refused");
  let msg = err.to_string();
  assert!(
    msg.contains("--name") && msg.contains("ci"),
    "names the flag and the profile: {msg}"
  );

  // The `--name=value` spelling is refused too.
  let cfg = ContainerConfig {
    extra_args: argv(&["--name=custom"]),
    ..cfg.clone()
  };
  assert!(gwm::exec::validate_container("ci", &cfg).is_err(), "`--name=…` too");

  // A flag that merely starts with the same letters is fine.
  let cfg = ContainerConfig {
    extra_args: argv(&["--network", "none"]),
    ..cfg.clone()
  };
  assert!(
    gwm::exec::validate_container("ci", &cfg).is_ok(),
    "unrelated flags pass"
  );

  // And the same refusal reaches the command path, not only config validation.
  let mut profiles = BTreeMap::new();
  profiles.insert(
    "ci".to_string(),
    ExecProfile {
      command: argv(&["cargo", "test"]),
      jobs: None,
      container: Some(ContainerConfig {
        image: "rust:1.90".to_string(),
        runtime: None,
        extra_args: argv(&["--name", "custom"]),
        selinux_relabel: false,
      }),
    },
  );
  let cfg = ExecConfig { jobs: None, profiles };
  assert!(
    resolve_exec_container(Some("ci"), &cfg).is_err(),
    "`gwm exec --profile ci` refuses it too"
  );
}
