//! End-to-end tests that invoke the compiled `gwm` binary via assert_cmd.
//! These exercise the user-visible CLI surface (subcommands, help, errors).

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_prints_subcommands() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.arg("--help");
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("init"))
    .stdout(predicate::str::contains("list"))
    .stdout(predicate::str::contains("create"))
    .stdout(predicate::str::contains("bootstrap"))
    .stdout(predicate::str::contains("prune"));
}

#[test]
fn version_flag() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.arg("--version");
  cmd.assert().success().stdout(predicate::str::contains("gwm"));
}

#[test]
fn types_lists_branch_types() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.arg("types");
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("feat"))
    .stdout(predicate::str::contains("fix"))
    .stdout(predicate::str::contains("hotfix"))
    .stdout(predicate::str::contains("chore"));
}

#[test]
fn create_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.current_dir(dir.path()).arg("list");
  cmd
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}
