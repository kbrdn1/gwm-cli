//! End-to-end tests that invoke the compiled `gwm` binary via assert_cmd.
//! These exercise the user-visible CLI surface (subcommands, help, errors).

mod common;

use assert_cmd::Command;
use common::init_repo;
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
    .stdout(predicate::str::contains("prune"))
    .stdout(predicate::str::contains("completions"))
    .stdout(predicate::str::contains("cd"))
    .stdout(predicate::str::contains("shell-init"));
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

#[test]
fn completions_zsh_emits_compdef_header() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "zsh"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("#compdef gwm"))
    .stdout(predicate::str::contains("_gwm"));
}

#[test]
fn completions_bash_emits_complete_directive() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "bash"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("_gwm"))
    .stdout(predicate::str::contains("complete "));
}

#[test]
fn completions_fish_emits_complete_directive() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "fish"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("complete -c gwm"));
}

#[test]
fn completions_powershell_emits_register_block() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "powershell"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("Register-ArgumentCompleter"))
    .stdout(predicate::str::contains("gwm"));
}

#[test]
fn completions_rejects_unknown_shell() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "tcsh"]);
  cmd.assert().failure();
}

#[test]
fn list_format_names_emits_one_name_per_line() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["list", "--format=names"])
    .assert()
    .success()
    // No table header, no leading "*" marker, no STATUS column.
    .stdout(predicate::str::contains("NAME").not())
    .stdout(predicate::str::contains("STATUS").not())
    // Main worktree is excluded so the output mirrors what `find_fuzzy`
    // accepts (path/remove/bootstrap skip the main workdir). A fresh repo
    // therefore prints nothing.
    .stdout(predicate::str::is_empty());
}

#[test]
fn cd_unknown_pattern_fails_with_not_found() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["cd", "nope"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not found"));
}

#[test]
fn cd_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["cd", "anything"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn shell_init_bash_emits_gcd_function() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "bash"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("gcd()"))
    .stdout(predicate::str::contains("gwm cd"));
}

#[test]
fn shell_init_zsh_emits_gcd_function() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "zsh"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("gcd()"))
    .stdout(predicate::str::contains("gwm cd"));
}

#[test]
fn shell_init_fish_emits_function_block() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "fish"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("function gcd"))
    .stdout(predicate::str::contains("gwm cd"))
    .stdout(predicate::str::contains("end"));
}

#[test]
fn shell_init_powershell_emits_function_block() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "powershell"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("function gcd"))
    .stdout(predicate::str::contains("gwm cd"))
    .stdout(predicate::str::contains("Set-Location"));
}

#[test]
fn shell_init_rejects_unknown_shell() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "tcsh"]);
  cmd.assert().failure();
}

#[test]
fn list_format_table_is_default() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .arg("list")
    .assert()
    .success()
    .stdout(predicate::str::contains("NAME"))
    .stdout(predicate::str::contains("STATUS"));
}
