use gwm::bootstrap::{self, BootstrapCtx, StepStatus};
use gwm::config::{CommandStep, Config, CopyStep, FallbackContent, Guard, NoSymlink};
use std::collections::HashMap;
use tempfile::TempDir;

fn dirs() -> (TempDir, TempDir, Config) {
  (TempDir::new().unwrap(), TempDir::new().unwrap(), Config::default())
}

#[test]
fn copy_step_happy_path() {
  let (main, wt, mut cfg) = dirs();
  std::fs::write(main.path().join(".env"), "FOO=bar").unwrap();
  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: true,
    guards: vec![],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert_eq!(report.steps.iter().filter(|s| s.status == StepStatus::Ok).count(), 1);
  assert!(wt.path().join(".env").exists());
}

#[test]
fn copy_step_skipped_when_dest_exists() {
  let (main, wt, mut cfg) = dirs();
  std::fs::write(main.path().join(".env"), "NEW").unwrap();
  std::fs::write(wt.path().join(".env"), "EXISTING").unwrap();
  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: true,
    guards: vec![],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Skipped));
  assert_eq!(std::fs::read_to_string(wt.path().join(".env")).unwrap(), "EXISTING");
}

#[test]
fn copy_step_required_source_missing_fails() {
  let (main, wt, mut cfg) = dirs();
  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: true,
    guards: vec![],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Failed));
}

#[test]
fn copy_step_optional_source_missing_skipped() {
  let (main, wt, mut cfg) = dirs();
  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: false,
    guards: vec![],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(report.steps.iter().all(|s| s.status != StepStatus::Failed));
}

#[test]
fn copy_step_inline_fallback_writes_content() {
  let (main, wt, mut cfg) = dirs();
  cfg.bootstrap.copy.push(CopyStep {
    from: ".env.testing".into(),
    to: ".env.testing".into(),
    required: true,
    guards: vec![],
    fallback: Some("inline".into()),
  });
  cfg.bootstrap.fallback.insert(
    "env_testing".into(),
    FallbackContent {
      target: ".env.testing".into(),
      content: "DB_CONNECTION=sqlite\n".into(),
    },
  );
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(wt.path().join(".env.testing").exists());
  let content = std::fs::read_to_string(wt.path().join(".env.testing")).unwrap();
  assert!(content.contains("DB_CONNECTION=sqlite"));
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Warning));
}

#[test]
fn guard_abort_blocks_copy() {
  let (main, wt, mut cfg) = dirs();
  std::fs::write(main.path().join(".env"), "DB_HOST=db.rds.amazonaws.com").unwrap();
  cfg.bootstrap.guard.push(Guard {
    name: "no-aws".into(),
    deny_patterns: vec!["amazonaws\\.com".into()],
    on_match: "abort".into(),
    example_file: None,
  });
  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: false,
    guards: vec!["no-aws".into()],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(!wt.path().join(".env").exists());
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Failed));
}

#[test]
fn guard_seed_from_example_substitutes() {
  let (main, wt, mut cfg) = dirs();
  std::fs::write(main.path().join(".env"), "DB_HOST=prod.rds.amazonaws.com").unwrap();
  std::fs::write(main.path().join(".env.example"), "DB_HOST=localhost").unwrap();
  cfg.bootstrap.guard.push(Guard {
    name: "no-aws".into(),
    deny_patterns: vec!["amazonaws\\.com".into()],
    on_match: "seed-from-example".into(),
    example_file: Some(".env.example".into()),
  });
  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: false,
    guards: vec!["no-aws".into()],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(wt.path().join(".env").exists());
  let content = std::fs::read_to_string(wt.path().join(".env")).unwrap();
  assert_eq!(content, "DB_HOST=localhost");
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Warning));
}

#[test]
fn guard_does_not_trip_on_safe_content() {
  let (main, wt, mut cfg) = dirs();
  std::fs::write(main.path().join(".env"), "DB_HOST=localhost").unwrap();
  cfg.bootstrap.guard.push(Guard {
    name: "no-aws".into(),
    deny_patterns: vec!["amazonaws\\.com".into()],
    on_match: "abort".into(),
    example_file: None,
  });
  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: false,
    guards: vec!["no-aws".into()],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(wt.path().join(".env").exists());
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Ok));
}

#[test]
#[cfg(unix)]
fn no_symlink_removes_existing_symlink() {
  let (main, wt, mut cfg) = dirs();
  let real_target = main.path().join("vendor_real");
  std::fs::create_dir(&real_target).unwrap();
  std::os::unix::fs::symlink(&real_target, wt.path().join("vendor")).unwrap();
  cfg.bootstrap.no_symlink.push(NoSymlink { path: "vendor".into() });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(!wt.path().join("vendor").is_symlink());
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Warning));
}

#[test]
fn command_when_file_exists_skips_if_missing() {
  let (main, wt, mut cfg) = dirs();
  cfg.bootstrap.command.push(CommandStep {
    name: "composer install".into(),
    run: "echo composer".into(),
    when: Some("file_exists:composer.json".into()),
    env: HashMap::new(),
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Skipped));
}

#[test]
fn command_runs_when_condition_satisfied() {
  let (main, wt, mut cfg) = dirs();
  std::fs::write(wt.path().join("composer.json"), "{}").unwrap();
  cfg.bootstrap.command.push(CommandStep {
    name: "echo".into(),
    run: "echo ok > out.txt".into(),
    when: Some("file_exists:composer.json".into()),
    env: HashMap::new(),
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Ok));
  assert!(wt.path().join("out.txt").exists());
}

#[test]
fn command_failure_recorded() {
  let (main, wt, mut cfg) = dirs();
  cfg.bootstrap.command.push(CommandStep {
    name: "boom".into(),
    run: "exit 1".into(),
    when: None,
    env: HashMap::new(),
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();
  assert!(report.steps.iter().any(|s| s.status == StepStatus::Failed));
}

#[test]
fn command_env_is_propagated() {
  let (main, wt, mut cfg) = dirs();
  let mut env = HashMap::new();
  env.insert("HELLO".into(), "world".into());
  cfg.bootstrap.command.push(CommandStep {
    name: "echo-env".into(),
    run: "echo $HELLO > env.txt".into(),
    when: None,
    env,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  bootstrap::run(&ctx).unwrap();
  let content = std::fs::read_to_string(wt.path().join("env.txt")).unwrap();
  assert_eq!(content.trim(), "world");
}

// --------------------------------------------------------------------------
// Issue #93 — bootstrap must refuse to write through symlinks at copy dst.
// --------------------------------------------------------------------------
//
// Three failure modes the pre-#93 code could exhibit, each pinned below:
//
// 1. **Dangling symlink at dst** — `Path::exists()` follows symlinks, so a
//    broken symlink reports as "doesn't exist" and the existing skip-if-
//    populated branch is bypassed. `fs::copy` then opens the broken symlink
//    in `O_WRONLY|O_CREAT|O_TRUNC` mode, which materialises the file at the
//    symlink's resolved path — anywhere on disk the user (or attacker) chose.
// 2. **Symlink + declared `[[bootstrap.no_symlink]]`, run_copies still
//    first** — the no-symlink pass was sequenced AFTER copies; when both
//    target the same path, the copy ran (or skipped through the symlink)
//    before the symlink got cleaned up, defeating the declared invariant.
// 3. **Symlink to an existing file, no `[[no_symlink]]` declaration** —
//    silently skipped because the existing entry is treated as "already
//    populated". Defence in depth: a symlink at a copy destination is
//    suspicious enough that we surface it as Failed instead of swallowing.

#[test]
#[cfg(unix)]
fn copy_refuses_to_write_through_dangling_symlink_at_destination() {
  // Scenario from the bug report: an attacker (or stale tooling) plants a
  // symlink at the bootstrap copy destination pointing at a sentinel path
  // OUTSIDE the worktree that doesn't exist yet. The pre-fix code falls
  // through to `fs::copy`, which materialises the sentinel — a write-
  // anywhere primitive. Post-fix, `symlink_metadata` detects the symlink
  // and aborts the step. The sentinel file must NOT appear on disk.
  let (main, wt, mut cfg) = dirs();
  std::fs::write(main.path().join(".env"), "SECRET=value").unwrap();
  let sentinel_dir = TempDir::new().unwrap();
  let sentinel = sentinel_dir.path().join("would-be-created-by-bug");
  assert!(!sentinel.exists(), "precondition: sentinel must not exist");
  std::os::unix::fs::symlink(&sentinel, wt.path().join(".env")).unwrap();

  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: true,
    guards: vec![],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();

  assert!(
    !sentinel.exists(),
    "the bug: fs::copy followed the dangling symlink and wrote outside the worktree"
  );
  let copy_step = report
    .steps
    .iter()
    .find(|s| s.label.starts_with("copy "))
    .expect("a copy step must be reported");
  assert_eq!(
    copy_step.status,
    StepStatus::Failed,
    "copy through symlink must be reported as Failed, got: {:?}",
    copy_step
  );
  assert!(
    copy_step.detail.contains("symlink"),
    "Failed detail must name the offending symlink, got: {:?}",
    copy_step.detail
  );
}

#[test]
#[cfg(unix)]
fn copy_refuses_to_skip_through_symlink_to_existing_file() {
  // Defence in depth: even when the symlink resolves to an existing file
  // (so `Path::exists()` returns true and the pre-fix code would silently
  // Skip), we surface the situation as Failed. A symlink at a declared
  // bootstrap destination is suspicious — silent skipping is silent
  // acknowledgement, which the user wouldn't see scroll past.
  let (main, wt, mut cfg) = dirs();
  std::fs::write(main.path().join(".env"), "FROM_MAIN").unwrap();
  let sentinel_dir = TempDir::new().unwrap();
  let sentinel = sentinel_dir.path().join("sentinel");
  std::fs::write(&sentinel, "SENTINEL_UNTOUCHED").unwrap();
  std::os::unix::fs::symlink(&sentinel, wt.path().join(".env")).unwrap();

  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: true,
    guards: vec![],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();

  // The sentinel content must survive — bootstrap MUST NOT write through.
  assert_eq!(
    std::fs::read_to_string(&sentinel).unwrap(),
    "SENTINEL_UNTOUCHED",
    "fs::copy must not have followed the symlink and overwritten the sentinel"
  );
  let copy_step = report
    .steps
    .iter()
    .find(|s| s.label.starts_with("copy "))
    .expect("a copy step must be reported");
  assert_eq!(
    copy_step.status,
    StepStatus::Failed,
    "symlink at dst must be surfaced as Failed (not silently Skipped)"
  );
  assert!(
    copy_step.detail.contains("symlink"),
    "Failed detail must mention the symlink, got: {:?}",
    copy_step.detail
  );
}

#[test]
#[cfg(unix)]
fn no_symlinks_runs_before_copies_when_both_target_the_same_path() {
  // Reorder regression test: when the same path appears in both
  // [[bootstrap.no_symlink]] AND [[bootstrap.copy]] and the worktree
  // carries a symlink at that path, no_symlinks must run FIRST. The
  // symlink is removed, then the copy creates a regular file from the
  // main repo's source. End state: regular file in the worktree, no
  // write-through to the symlink's target.
  let (main, wt, mut cfg) = dirs();
  std::fs::write(main.path().join(".env"), "FROM_MAIN").unwrap();
  let sentinel_dir = TempDir::new().unwrap();
  let sentinel = sentinel_dir.path().join("sentinel");
  std::fs::write(&sentinel, "SENTINEL_UNTOUCHED").unwrap();
  std::os::unix::fs::symlink(&sentinel, wt.path().join(".env")).unwrap();

  // Both passes declared on the same target.
  cfg.bootstrap.no_symlink.push(NoSymlink { path: ".env".into() });
  cfg.bootstrap.copy.push(CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: true,
    guards: vec![],
    fallback: None,
  });
  let ctx = BootstrapCtx {
    main_repo: main.path(),
    worktree: wt.path(),
    config: &cfg,
  };
  let report = bootstrap::run(&ctx).unwrap();

  // Sentinel still untouched.
  assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "SENTINEL_UNTOUCHED");
  // The worktree now carries a regular file (no longer a symlink) with
  // the source's contents.
  let dst = wt.path().join(".env");
  assert!(!dst.is_symlink(), "no_symlinks should have stripped the symlink");
  assert_eq!(
    std::fs::read_to_string(&dst).unwrap(),
    "FROM_MAIN",
    "copy must run AFTER no_symlinks and seed the worktree from main"
  );
  // Both passes must have left a step on the report — proves the
  // ordering: no_symlinks (Warning: removed symlink) then copy (Ok).
  let labels: Vec<&str> = report.steps.iter().map(|s| s.label.as_str()).collect();
  let ns_idx = labels
    .iter()
    .position(|l| l.starts_with("no-symlink "))
    .expect("no-symlink step must be reported");
  let cp_idx = labels
    .iter()
    .position(|l| l.starts_with("copy "))
    .expect("copy step must be reported");
  assert!(
    ns_idx < cp_idx,
    "no_symlinks must be sequenced before copies, got order {:?}",
    labels
  );
}

// --------------------------------------------------------------------------
// Issue #93 follow-up — direct unit tests for the O_NOFOLLOW primitives.
// --------------------------------------------------------------------------
//
// The match guard at the top of `run_copies` is end-to-end tested above,
// but the TOCTOU window between the stat and the subsequent write is
// only closed by `copy_no_follow` / `write_no_follow`. Since the
// pre-existing-symlink case never reaches the helper (the guard
// intercepts first), these tests exercise the helpers directly with a
// symlink pre-placed at `dst` — simulating the race outcome where the
// guard's stat returned NotFound but a symlink materialised before
// the open syscall.

#[test]
#[cfg(unix)]
fn write_no_follow_refuses_symlink_at_destination() {
  // Direct primitive test: pre-place a symlink at `dst` (live, pointing
  // at an existing sentinel), call `write_no_follow`, and assert (a)
  // the call returns Err, (b) the sentinel is NOT modified — proving
  // `O_NOFOLLOW` short-circuited the open before any write happened.
  let dir = TempDir::new().unwrap();
  let sentinel = dir.path().join("sentinel");
  std::fs::write(&sentinel, "SENTINEL_UNTOUCHED").unwrap();
  let dst = dir.path().join("victim");
  std::os::unix::fs::symlink(&sentinel, &dst).unwrap();

  let err = bootstrap::write_no_follow(&dst, b"PAYLOAD").expect_err("must refuse symlink dst");
  assert_eq!(
    std::fs::read_to_string(&sentinel).unwrap(),
    "SENTINEL_UNTOUCHED",
    "write_no_follow must NOT follow the symlink; got io error {:?} but the sentinel was modified",
    err
  );
}

#[test]
#[cfg(unix)]
fn write_no_follow_refuses_dangling_symlink_at_destination() {
  // Dangling symlink variant: `dst` is a broken symlink. Pre-fix
  // `fs::write(O_CREAT|O_TRUNC)` followed it and materialised the
  // sentinel; `O_NOFOLLOW` rejects the open. The sentinel path must
  // remain absent on disk after the failed call.
  let dir = TempDir::new().unwrap();
  let sentinel_holder = TempDir::new().unwrap();
  let sentinel = sentinel_holder.path().join("would-be-created");
  assert!(!sentinel.exists(), "precondition: sentinel must not exist");
  let dst = dir.path().join("victim");
  std::os::unix::fs::symlink(&sentinel, &dst).unwrap();

  bootstrap::write_no_follow(&dst, b"PAYLOAD").expect_err("must refuse dangling symlink dst");
  assert!(
    !sentinel.exists(),
    "write_no_follow must NOT create the symlink's resolved target"
  );
}

#[test]
fn write_no_follow_refuses_pre_existing_regular_file() {
  // `create_new(true)` half of the contract — independent of the
  // unix-only O_NOFOLLOW. A regular file pre-existing at `dst` must
  // produce an `AlreadyExists` error rather than truncating. This
  // holds on all platforms (Windows uses CREATE_NEW).
  let dir = TempDir::new().unwrap();
  let dst = dir.path().join("victim");
  std::fs::write(&dst, "ORIGINAL").unwrap();

  let err = bootstrap::write_no_follow(&dst, b"NEW").expect_err("must refuse pre-existing dst");
  assert_eq!(
    err.kind(),
    std::io::ErrorKind::AlreadyExists,
    "expected AlreadyExists, got {:?}",
    err.kind()
  );
  assert_eq!(
    std::fs::read_to_string(&dst).unwrap(),
    "ORIGINAL",
    "the pre-existing file must be untouched"
  );
}

#[test]
fn write_no_follow_creates_fresh_dst() {
  // Positive control: with no pre-existing entry at `dst`, the helper
  // creates the file and writes the payload. Same observable behaviour
  // as `std::fs::write` on the happy path — proves the helper is a
  // drop-in replacement for the bootstrap call sites.
  let dir = TempDir::new().unwrap();
  let dst = dir.path().join("fresh");
  bootstrap::write_no_follow(&dst, b"HELLO").expect("happy path must succeed");
  assert_eq!(std::fs::read_to_string(&dst).unwrap(), "HELLO");
}

#[test]
#[cfg(unix)]
fn copy_no_follow_refuses_symlink_at_destination_and_preserves_mode() {
  // Companion test for the `copy_no_follow` primitive. Two assertions:
  // (1) a pre-existing symlink at `dst` aborts the copy (mirror of
  //     `write_no_follow_refuses_symlink_at_destination`), and
  // (2) on the happy path the helper preserves `src`'s unix mode —
  //     this is the bit `std::fs::copy` provides and our hand-rolled
  //     primitive must not silently drop.
  use std::os::unix::fs::PermissionsExt;
  let dir = TempDir::new().unwrap();
  let src = dir.path().join("src");
  std::fs::write(&src, "PAYLOAD").unwrap();
  std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o640)).unwrap();

  // (1) Symlink at dst is refused.
  let sentinel = TempDir::new().unwrap();
  let sentinel_path = sentinel.path().join("would-be-created");
  let dst_sym = dir.path().join("dst-via-symlink");
  std::os::unix::fs::symlink(&sentinel_path, &dst_sym).unwrap();
  bootstrap::copy_no_follow(&src, &dst_sym).expect_err("must refuse symlink dst");
  assert!(!sentinel_path.exists(), "must NOT have followed the symlink");

  // (2) Happy path preserves the source mode.
  let dst = dir.path().join("dst-fresh");
  bootstrap::copy_no_follow(&src, &dst).expect("happy path must succeed");
  let mode = std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777;
  assert_eq!(mode, 0o640, "copy_no_follow must preserve src's unix mode");
  assert_eq!(std::fs::read_to_string(&dst).unwrap(), "PAYLOAD");
}
