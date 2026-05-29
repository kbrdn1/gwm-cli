//! Integration tests for `gwm sync` (issue #24) — fetch + rebase / merge
//! a worktree's branch onto its upstream.
//!
//! These exercise the public `gwm::sync::sync` entry point against
//! fully offline fixtures: a bare `origin` repo on the local
//! filesystem plays the remote, a `local` clone tracks `origin/main`,
//! and a throwaway `seed` clone is used to push "upstream" commits.
//! No network, deterministic — `git fetch <path>` against a `file://`
//! style path needs only the `git` binary, which CI always has.

use gwm::sync::{self, SyncAction, SyncStrategy};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Run `git -C <dir> <args>` with a deterministic identity and no GPG
/// signing, asserting success. Test-only helper — panics are the right
/// failure mode here.
fn git(dir: &Path, args: &[&str]) {
  let out = Command::new("git")
    .arg("-C")
    .arg(dir)
    // Global `-c` options must precede the subcommand.
    .args(["-c", "commit.gpgsign=false"])
    .args(args)
    .env("GIT_AUTHOR_NAME", "gwm-test")
    .env("GIT_AUTHOR_EMAIL", "gwm@test")
    .env("GIT_COMMITTER_NAME", "gwm-test")
    .env("GIT_COMMITTER_EMAIL", "gwm@test")
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_SYSTEM", "/dev/null")
    .output()
    .unwrap_or_else(|e| panic!("git {:?} failed to spawn: {e}", args));
  assert!(
    out.status.success(),
    "git {:?} exited {}: {}",
    args,
    out.status,
    String::from_utf8_lossy(&out.stderr)
  );
}

/// Capture stdout of `git -C <dir> <args>`, trimmed.
fn git_out(dir: &Path, args: &[&str]) -> String {
  let out = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_SYSTEM", "/dev/null")
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "git {:?} failed: {}",
    args,
    String::from_utf8_lossy(&out.stderr)
  );
  String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(path: &Path, contents: &str) {
  std::fs::write(path, contents).unwrap();
}

/// Build `origin` (bare) + `local` (clone tracking `origin/main`, one
/// commit). Returns the tempdir guard and the `local` working dir.
fn local_tracking_origin() -> (TempDir, PathBuf, PathBuf) {
  let td = TempDir::new().unwrap();
  let root = td.path();
  let origin = root.join("origin");
  let local = root.join("local");
  std::fs::create_dir_all(&origin).unwrap();
  std::fs::create_dir_all(&local).unwrap();

  // Bare origin on `main`.
  let out = Command::new("git")
    .args(["init", "--bare", "-b", "main"])
    .arg(&origin)
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "init bare: {}",
    String::from_utf8_lossy(&out.stderr)
  );

  // Local repo with one commit, pushed with upstream tracking.
  let out = Command::new("git")
    .args(["init", "-b", "main"])
    .arg(&local)
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "init local: {}",
    String::from_utf8_lossy(&out.stderr)
  );
  write(&local.join("file.txt"), "base\n");
  git(&local, &["add", "-A"]);
  git(&local, &["commit", "-m", "init"]);
  git(&local, &["remote", "add", "origin", origin.to_str().unwrap()]);
  git(&local, &["push", "-u", "origin", "main"]);

  (td, origin, local)
}

/// Push one extra commit to `origin/main` from a throwaway clone so the
/// `local` repo becomes one commit behind upstream after a fetch.
fn push_upstream_commit(root: &Path, origin: &Path, file: &str, contents: &str, msg: &str) {
  let seed = root.join(format!("seed-{msg}"));
  let out = Command::new("git")
    .arg("clone")
    .arg(origin)
    .arg(&seed)
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "clone seed: {}",
    String::from_utf8_lossy(&out.stderr)
  );
  write(&seed.join(file), contents);
  git(&seed, &["add", "-A"]);
  git(&seed, &["commit", "-m", msg]);
  git(&seed, &["push", "origin", "main"]);
}

#[test]
fn sync_refuses_dirty_worktree() {
  let (_td, _origin, local) = local_tracking_origin();
  // Uncommitted change → sync must refuse before touching the remote.
  write(&local.join("file.txt"), "dirty edit\n");

  let err = sync::sync(&local, SyncStrategy::Rebase).unwrap_err();
  let msg = err.to_string().to_lowercase();
  assert!(
    msg.contains("uncommitted") || msg.contains("stash"),
    "dirty refusal should mention uncommitted/stash, got: {msg}"
  );
}

#[test]
fn sync_errors_without_upstream() {
  // A repo with a committed branch but no upstream configured.
  let td = TempDir::new().unwrap();
  let local = td.path().join("solo");
  let out = Command::new("git")
    .args(["init", "-b", "main"])
    .arg(&local)
    .output()
    .unwrap();
  assert!(out.status.success());
  write(&local.join("file.txt"), "base\n");
  git(&local, &["add", "-A"]);
  git(&local, &["commit", "-m", "init"]);

  let err = sync::sync(&local, SyncStrategy::Rebase).unwrap_err();
  let msg = err.to_string().to_lowercase();
  assert!(
    msg.contains("upstream"),
    "missing-upstream error should mention upstream, got: {msg}"
  );
}

#[test]
fn sync_reports_up_to_date_when_level_with_upstream() {
  let (_td, _origin, local) = local_tracking_origin();

  let report = sync::sync(&local, SyncStrategy::Rebase).unwrap();
  assert_eq!(report.action, SyncAction::UpToDate, "no upstream commits ⇒ up to date");
  assert_eq!(report.behind_before, 0);
  assert_eq!(report.branch, "main");
  assert!(
    report.upstream.contains("main"),
    "upstream label should name the tracked ref"
  );
}

#[test]
fn sync_rebases_branch_that_is_behind_upstream() {
  let (td, origin, local) = local_tracking_origin();
  push_upstream_commit(td.path(), &origin, "file.txt", "base\nupstream\n", "upstream-change");

  let report = sync::sync(&local, SyncStrategy::Rebase).unwrap();
  assert_eq!(
    report.action,
    SyncAction::Integrated,
    "behind branch should integrate upstream"
  );
  assert_eq!(report.behind_before, 1, "exactly one upstream commit was pending");
  assert_eq!(report.strategy, SyncStrategy::Rebase);

  // After rebase the local tip must equal origin/main's tip.
  let local_head = git_out(&local, &["rev-parse", "HEAD"]);
  let origin_head = git_out(&origin, &["rev-parse", "main"]);
  assert_eq!(
    local_head, origin_head,
    "local HEAD should match upstream after a clean rebase"
  );
}

#[test]
fn sync_merge_strategy_integrates_upstream() {
  let (td, origin, local) = local_tracking_origin();
  push_upstream_commit(td.path(), &origin, "other.txt", "added upstream\n", "upstream-feature");

  let report = sync::sync(&local, SyncStrategy::Merge).unwrap();
  assert_eq!(report.action, SyncAction::Integrated);
  assert_eq!(report.strategy, SyncStrategy::Merge);
  assert_eq!(report.behind_before, 1);

  // The upstream file must now be present in the local worktree.
  assert!(
    local.join("other.txt").exists(),
    "merge should bring the upstream-added file into the worktree"
  );
}

#[test]
fn sync_aborts_and_errors_on_conflict() {
  let (td, origin, local) = local_tracking_origin();
  // Upstream edits file.txt one way…
  push_upstream_commit(td.path(), &origin, "file.txt", "base\nUPSTREAM\n", "upstream-edit");
  // …local edits the same line differently and commits, so a rebase conflicts.
  write(&local.join("file.txt"), "base\nLOCAL\n");
  git(&local, &["add", "-A"]);
  git(&local, &["commit", "-m", "local-edit"]);

  let err = sync::sync(&local, SyncStrategy::Rebase).unwrap_err();
  let msg = err.to_string().to_lowercase();
  assert!(msg.contains("conflict"), "conflict error should say so, got: {msg}");

  // The failed rebase must be aborted: no rebase-in-progress state left behind.
  assert!(
    !local.join(".git/rebase-merge").exists() && !local.join(".git/rebase-apply").exists(),
    "sync must abort the rebase so the worktree is left usable"
  );
}
