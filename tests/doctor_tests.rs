//! `gwm doctor` checks. Each test exercises one diagnostic in isolation.

mod common;

use common::init_repo;
use gwm::config::Config;
use gwm::doctor::{self, CheckStatus, DoctorCtx, Severity};

fn ctx_for<'a>(repo: &'a git2::Repository, workdir: &'a std::path::Path, config: &'a Config) -> DoctorCtx<'a> {
  DoctorCtx { repo_workdir: workdir, repo, config }
}

#[test]
fn fresh_repo_without_config_reports_defaults_assumed() {
  let (dir, repo) = init_repo();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let cfg = report
    .checks
    .iter()
    .find(|c| c.name.contains(".gwm.toml"))
    .expect("expected a `.gwm.toml` check in the report");

  // Missing config is not an error — defaults are perfectly usable.
  assert_eq!(cfg.status, CheckStatus::Ok);
  assert!(
    cfg.detail.to_lowercase().contains("default"),
    "missing config should mention 'defaults assumed', got: {}",
    cfg.detail
  );
}

#[test]
fn invalid_toml_marks_config_check_failed_with_severity_failed() {
  let (dir, repo) = init_repo();
  std::fs::write(dir.path().join(".gwm.toml"), "this is = not valid [toml").unwrap();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let cfg = report
    .checks
    .iter()
    .find(|c| c.name.contains(".gwm.toml"))
    .expect("expected a `.gwm.toml` check");

  assert_eq!(cfg.status, CheckStatus::Failed);
  assert_eq!(report.severity(), Severity::Failed);
  assert_eq!(report.exit_code(), 2);
}

#[test]
fn valid_toml_marks_config_check_ok() {
  let (dir, repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"[worktree]
base = "{home}/wt/{repo}"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"
"#,
  )
  .unwrap();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let cfg = report
    .checks
    .iter()
    .find(|c| c.name.contains(".gwm.toml"))
    .expect("expected a `.gwm.toml` check");
  assert_eq!(cfg.status, CheckStatus::Ok);
}

#[test]
fn severity_ok_when_no_checks_fail() {
  let (dir, repo) = init_repo();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  // A fresh repo with no `.gwm.toml` and no orphan branches must come back
  // green — anything else means the doctor is over-flagging vanilla setups.
  assert_eq!(report.severity(), Severity::Ok);
  assert_eq!(report.exit_code(), 0);
}
