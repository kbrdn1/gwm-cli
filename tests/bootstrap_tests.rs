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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
    FallbackContent { target: ".env.testing".into(), content: "DB_CONNECTION=sqlite\n".into() },
  );
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
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
  let ctx = BootstrapCtx { main_repo: main.path(), worktree: wt.path(), config: &cfg };
  bootstrap::run(&ctx).unwrap();
  let content = std::fs::read_to_string(wt.path().join("env.txt")).unwrap();
  assert_eq!(content.trim(), "world");
}
