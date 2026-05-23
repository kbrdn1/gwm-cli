//! Integration tests for the `gwm hooks install commit-msg` machinery
//! (issue #85). The hook script is the bridge between `gwm
//! commit-prefix` and `git commit` — it auto-prepends the resolved
//! prefix when the user's commit message doesn't already start with
//! one, and stays out of the way otherwise.

use gwm::hooks::{commit_msg_script, install_commit_msg};
use std::os::unix::fs::PermissionsExt;

/// Initialise a fresh git repo with a worktree on `feat/#42-demo`. The
/// hook installer needs a `.git` directory; we don't need any commits
/// to verify the install path (the hook is a static script + permission
/// bits).
fn init_repo_on_feat_branch() -> tempfile::TempDir {
  let dir = tempfile::TempDir::new().expect("tempdir");
  let repo = git2::Repository::init(dir.path()).expect("init repo");
  // Set HEAD to a feat branch so the hook's `gwm commit-prefix`
  // resolution has something realistic to chew on at runtime — we
  // don't actually execute the hook in this test, but a fixture that
  // mirrors a real scenario is cheap.
  repo
    .reference_symbolic("HEAD", "refs/heads/feat/#42-demo", true, "setup")
    .expect("set HEAD");
  dir
}

#[test]
fn install_commit_msg_creates_the_hook_file() {
  // The contract: `install_commit_msg(repo)` writes
  // `<repo>/.git/hooks/commit-msg` and returns its path. The file must
  // exist after the call — the hook installer is the only surface that
  // touches the `.git/hooks/` directory.
  let dir = init_repo_on_feat_branch();
  let path = install_commit_msg(dir.path(), false).expect("install hook");
  assert!(path.exists(), "commit-msg hook file should exist after install");
  assert!(
    path.ends_with(".git/hooks/commit-msg"),
    "expected .git/hooks/commit-msg, got {}",
    path.display()
  );
}

#[test]
#[cfg(unix)]
fn install_commit_msg_marks_the_hook_executable() {
  // Git refuses to invoke a hook that isn't executable. The installer
  // sets mode 0o755 explicitly; we assert on the owner-exec bit
  // because the group / world bits depend on the user's umask and
  // are not part of the contract.
  let dir = init_repo_on_feat_branch();
  let path = install_commit_msg(dir.path(), false).expect("install hook");
  let mode = std::fs::metadata(&path).expect("stat hook").permissions().mode();
  assert!(
    mode & 0o100 != 0,
    "hook must be executable by owner (mode = {:o})",
    mode
  );
}

#[test]
fn install_commit_msg_refuses_to_overwrite_existing_hook_without_force() {
  // A pre-existing `commit-msg` may belong to husky, pre-commit,
  // commitlint, … — silently clobbering it would be destructive. The
  // contract is "refuse without `--force`", with an error that names
  // the offending file so the user can decide.
  let dir = init_repo_on_feat_branch();
  let hooks_dir = dir.path().join(".git").join("hooks");
  std::fs::create_dir_all(&hooks_dir).expect("hooks dir");
  let hook_path = hooks_dir.join("commit-msg");
  std::fs::write(&hook_path, "#!/bin/sh\necho 'pre-existing hook'\n").expect("seed hook");

  let result = install_commit_msg(dir.path(), false);
  assert!(result.is_err(), "install should refuse to overwrite without --force");

  // The pre-existing file MUST be intact — the failure path is
  // non-destructive by contract.
  let body = std::fs::read_to_string(&hook_path).expect("read seeded hook");
  assert!(
    body.contains("pre-existing hook"),
    "seeded hook must not be overwritten on the refusal path; got {:?}",
    body
  );
}

#[test]
fn install_commit_msg_overwrites_with_force() {
  // The escape hatch: `--force` lets the user knowingly replace an
  // existing hook. After the call, the file must be our generated
  // script (we detect this via a stable marker comment).
  let dir = init_repo_on_feat_branch();
  let hooks_dir = dir.path().join(".git").join("hooks");
  std::fs::create_dir_all(&hooks_dir).expect("hooks dir");
  let hook_path = hooks_dir.join("commit-msg");
  std::fs::write(&hook_path, "#!/bin/sh\necho 'old hook'\n").expect("seed hook");

  let path = install_commit_msg(dir.path(), true).expect("install with --force");
  let body = std::fs::read_to_string(&path).expect("read installed hook");
  assert!(
    body.contains("gwm commit-msg hook"),
    "installed hook must carry the gwm marker; got {:?}",
    body
  );
  assert!(
    !body.contains("old hook"),
    "force-install must replace the pre-existing body; got {:?}",
    body
  );
}

#[test]
fn install_commit_msg_outside_git_repo_fails() {
  // Defence-in-depth: a plain temp dir has no `.git` directory, so
  // there's no place to put the hook. The installer must refuse
  // cleanly rather than fabricating a `.git/hooks/` tree.
  let dir = tempfile::TempDir::new().expect("tempdir");
  let result = install_commit_msg(dir.path(), false);
  assert!(result.is_err(), "install outside a git repo must fail");
}

#[test]
fn commit_msg_script_is_a_posix_sh_file_referencing_gwm() {
  // The generated script's shape: a shebang, our marker, and a
  // shell-out to `gwm commit-prefix --unicode`. We don't assert on
  // the exact body so future cosmetic tweaks don't break the test —
  // just the three load-bearing tokens.
  let script = commit_msg_script();
  assert!(
    script.starts_with("#!/bin/sh\n") || script.starts_with("#!/usr/bin/env sh\n"),
    "script must start with a POSIX shebang; got {:?}",
    &script[..script.len().min(40)]
  );
  assert!(
    script.contains("gwm commit-msg hook"),
    "script must contain the gwm marker"
  );
  assert!(
    script.contains("gwm commit-prefix"),
    "script must shell out to gwm commit-prefix"
  );
  assert!(
    script.contains("--unicode"),
    "script must request --unicode so the commit message gets the real emoji"
  );
}

#[test]
fn commit_msg_script_skips_when_prefix_already_present() {
  // The hook's behavioural contract: if the commit message already
  // starts with an emoji, leave it alone. We can't run the hook
  // headless here (no `git` shell invocation), but we can assert the
  // script's pattern-detection clause exists — a regression that
  // drops it would auto-double-prefix every amend.
  let script = commit_msg_script();
  // The script uses `grep` on the first line to detect existing
  // gitmoji / `:shortcode:` prefixes. Both forms must be covered.
  assert!(
    script.contains("grep") || script.contains("case "),
    "script must include a guard for already-prefixed messages"
  );
}
