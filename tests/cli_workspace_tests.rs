//! End-to-end tests for workspace mode on the CLI (issue #36): the
//! `--workspace <dir>` global flag and the merged `gwm list` table.

use assert_cmd::Command;
use git2::{Repository, Signature};
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Init a git repo at `path` (created if missing) on `main` with one commit.
fn init_repo_at(path: &Path) {
  fs::create_dir_all(path).unwrap();
  let repo = Repository::init(path).unwrap();
  repo.set_head("refs/heads/main").ok();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  let tree_id = {
    let mut index = repo.index().unwrap();
    index.write_tree().unwrap()
  };
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
}

/// A workspace root with two child repos plus a non-repo dir to ignore.
fn workspace_root() -> TempDir {
  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("alpha"));
  init_repo_at(&root.path().join("beta"));
  fs::create_dir_all(root.path().join("notes")).unwrap();
  root
}

#[test]
fn list_workspace_prints_repo_column_and_every_repo() {
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .args(["list", "--workspace"])
    .arg(root.path())
    .assert()
    .success()
    // The merged table leads with a REPO column ahead of NAME/BRANCH/STATUS.
    .stdout(predicate::str::contains("REPO"))
    .stdout(predicate::str::contains("NAME"))
    .stdout(predicate::str::contains("BRANCH"))
    .stdout(predicate::str::contains("STATUS"))
    // Both child repos appear; the non-repo `notes/` dir does not.
    .stdout(predicate::str::contains("alpha"))
    .stdout(predicate::str::contains("beta"))
    .stdout(predicate::str::contains("notes").not());
}

#[test]
fn workspace_flag_is_global_before_the_subcommand() {
  // `gwm --workspace <root> list` parses identically to `gwm list
  // --workspace <root>` because the flag is `global = true`.
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .arg("--workspace")
    .arg(root.path())
    .arg("list")
    .assert()
    .success()
    .stdout(predicate::str::contains("alpha"))
    .stdout(predicate::str::contains("beta"));
}

#[test]
fn list_workspace_missing_root_fails() {
  let root = TempDir::new().unwrap();
  let missing = root.path().join("nope");
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["list", "--workspace"]).arg(&missing).assert().failure();
}

#[test]
fn list_workspace_empty_root_reports_no_repos() {
  let root = TempDir::new().unwrap();
  fs::create_dir_all(root.path().join("plain")).unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .args(["list", "--workspace"])
    .arg(root.path())
    .assert()
    .failure()
    .stderr(predicate::str::contains("no git repos"));
}

/// Minimal `.gwm.toml` pinning the worktree base into `base` with no
/// bootstrap commands, so `create --no-bootstrap` never hits the trust prompt.
fn write_min_config(repo_root: &Path, base: &Path) {
  let body = format!(
    "[worktree]\nbase = \"{}\"\npath_pattern = \"{{type}}-{{issue}}-{{desc}}\"\nbranch_pattern = \"{{type}}/#{{issue}}-{{desc}}\"\n",
    base.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
  );
  fs::write(repo_root.join(".gwm.toml"), body).unwrap();
}

#[test]
fn create_in_workspace_without_repo_flag_fails() {
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .arg("--workspace")
    .arg(root.path())
    .args(["create", "feat", "42", "demo", "--no-bootstrap"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("--repo"));
}

#[test]
fn create_in_workspace_targets_the_named_repo() {
  let root = workspace_root();
  let base = TempDir::new().unwrap();
  write_min_config(&root.path().join("alpha"), base.path());

  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .arg("--workspace")
    .arg(root.path())
    .args(["create", "feat", "42", "demo", "--repo", "alpha", "--no-bootstrap"])
    .assert()
    .success()
    .stdout(predicate::str::contains("feat/#42-demo"));

  // The branch lands in alpha, not beta.
  let alpha = Repository::open(root.path().join("alpha")).unwrap();
  assert!(
    alpha.find_branch("feat/#42-demo", git2::BranchType::Local).is_ok(),
    "branch must be created in the alpha repo"
  );
  let beta = Repository::open(root.path().join("beta")).unwrap();
  assert!(
    beta.find_branch("feat/#42-demo", git2::BranchType::Local).is_err(),
    "branch must NOT leak into the beta repo"
  );
}

#[test]
fn create_in_workspace_with_unknown_repo_lists_available() {
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .arg("--workspace")
    .arg(root.path())
    .args(["create", "feat", "42", "demo", "--repo", "ghost", "--no-bootstrap"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("ghost"))
    .stderr(predicate::str::contains("alpha"));
}

#[test]
fn list_workspace_names_format_lists_worktrees_per_repo() {
  // `--format names` in workspace mode qualifies each worktree with its repo
  // (`<repo>/<name>`) so a shell-completion candidate is unambiguous.
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .args(["list", "--workspace"])
    .arg(root.path())
    .args(["--format", "names"])
    .assert()
    .success()
    .stdout(predicate::str::contains("alpha/alpha"))
    .stdout(predicate::str::contains("beta/beta"));
}
