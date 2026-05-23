//! Integration tests for the `gwm hooks install commit-msg` machinery
//! (issue #85). The hook script is the bridge between `gwm
//! commit-prefix` and `git commit` — it auto-prepends the resolved
//! prefix when the user's commit message doesn't already start with
//! one, and stays out of the way otherwise.

use gwm::hooks::{commit_msg_script, install_commit_msg};
#[cfg(unix)]
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

#[test]
fn commit_msg_script_does_not_use_unguarded_set_e() {
  // Stated goal of the hook (see module docs): "never block a commit
  // because the hook itself broke". `set -e` aborts the script on any
  // non-zero exit — including transient `mktemp` / `mv` failures from
  // a full /tmp, a noexec mount, etc. — which would in turn abort
  // `git commit`. We accept `set -u` (unset variables are a real bug
  // we want to surface), but `set -eu` together violates the contract.
  let script = commit_msg_script();
  assert!(
    !script.contains("set -eu") && !script.contains("set -e\n") && !script.contains("set -e "),
    "script must not enable `set -e`: a failing fs op would abort `git commit`. Found: {:?}",
    script
  );
}

#[test]
fn commit_msg_script_skips_leading_comment_lines() {
  // Doc says the script inspects the "first non-empty line" — git's
  // own commit-message template puts `# Please enter the commit
  // message…` comments at the top, and `git commit -v` adds a diff
  // dump prefixed with `#`. The script must therefore find the first
  // line that is neither empty nor a `#`-comment when deciding
  // whether the message is already prefixed (and when prepending).
  let script = commit_msg_script();
  assert!(
    script.contains("first non-empty non-comment")
      || script.contains("skip leading empty / comment lines")
      || script.contains("grep -nvE '^([[:space:]]*#|[[:space:]]*$)'"),
    "script must explicitly skip leading empty / `#`-prefixed lines when locating the user's first real line; got: {}",
    script
  );
}

#[test]
fn install_commit_msg_resolves_linked_worktree_gitdir() {
  // `gwm`'s primary use case IS linked worktrees: `gwm create feat 42
  // demo` materialises one at `<root>/feat-42-demo` whose `.git` is a
  // *file* (`gitdir: <main>/.git/worktrees/feat-42-demo`), not a
  // directory. Installing into `<worktree>/.git/hooks/commit-msg`
  // would either fail (write into a file) or — worse — install at the
  // wrong location. The installer must follow the `.git` pointer and
  // land the hook under the worktree's actual gitdir (which, for a
  // linked worktree, is `<main>/.git/worktrees/<name>/hooks/` — that's
  // where git itself looks for `commit-msg` when this worktree is
  // active).
  let main = tempfile::TempDir::new().expect("main tempdir");
  let repo = git2::Repository::init(main.path()).expect("init main repo");
  // Need one commit so a linked worktree can attach.
  {
    let sig = git2::Signature::now("Test", "test@example.com").expect("sig");
    let tree_id = {
      let mut idx = repo.index().expect("index");
      idx.write_tree().expect("write tree")
    };
    let tree = repo.find_tree(tree_id).expect("find tree");
    repo
      .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
      .expect("seed commit");
  }
  let wt_path = main.path().join("wt-demo");
  let wt = repo
    .worktree("wt-demo", &wt_path, None)
    .expect("create linked worktree");

  let hook_path = install_commit_msg(&wt_path, false).expect("install in linked worktree");
  assert!(
    hook_path.exists(),
    "hook must be installed at the resolved gitdir, got {}",
    hook_path.display()
  );
  // The hook MUST live under the linked worktree's *admin* directory
  // — `<main>/.git/worktrees/wt-demo/hooks/commit-msg` — not under
  // `<worktree>/.git/hooks/` (the latter would treat `.git` as a
  // directory, which fails on a linked worktree where `.git` is a
  // file pointer). Assert by structural shape rather than equality
  // because macOS canonicalizes `/var/folders/…` to
  // `/private/var/folders/…` and the two halves of the comparison
  // wouldn't share a common prefix without canonicalize on both
  // sides.
  let canon_got = std::fs::canonicalize(&hook_path).expect("canonicalize got");
  assert!(
    canon_got.ends_with(".git/worktrees/wt-demo/hooks/commit-msg"),
    "hook must land in the linked worktree's admin dir; got {}",
    canon_got.display()
  );
  // Also ensure the installer did NOT fabricate `<worktree>/.git` as
  // a directory — for a linked worktree it is and must remain a file
  // pointing at the admin dir.
  let dotgit = wt_path.join(".git");
  assert!(
    dotgit.is_file(),
    "linked worktree `.git` must remain a pointer file, got {:?}",
    std::fs::metadata(&dotgit).map(|m| m.file_type())
  );
  // Keep `wt` alive until here so its drop doesn't run while we still
  // read the admin directory above. Touching its path is a cheap
  // way to silence the unused-variable warning without `_` (we want
  // it to drop *after* assertions, not be optimised out).
  let _ = wt.path();
}

#[test]
fn install_commit_msg_honours_core_hookspath() {
  // Git supports `core.hooksPath` to relocate the entire hooks dir
  // (this repo recommends `git config core.hooksPath .githooks`).
  // When set, writing into `.git/hooks/commit-msg` is dead code: git
  // never runs it. The installer must resolve the effective hooks
  // directory via the repo config and install there.
  let dir = init_repo_on_feat_branch();
  let custom_hooks = dir.path().join(".githooks");
  std::fs::create_dir_all(&custom_hooks).expect("custom hooks dir");
  // Persist `core.hooksPath` into the repo config so a fresh
  // `Repository::open` sees it.
  let repo = git2::Repository::open(dir.path()).expect("reopen repo");
  let mut cfg = repo.config().expect("config");
  cfg.set_str("core.hooksPath", ".githooks").expect("set core.hooksPath");
  drop(cfg);
  drop(repo);

  let hook_path = install_commit_msg(dir.path(), false).expect("install honouring core.hooksPath");
  // Canonicalize because macOS resolves `/var/folders/…` to
  // `/private/var/folders/…`, which would make a literal `starts_with`
  // false-negative even though both paths refer to the same dir.
  let canon_hook = std::fs::canonicalize(&hook_path).expect("canonicalize hook");
  let canon_custom = std::fs::canonicalize(&custom_hooks).expect("canonicalize custom hooks dir");
  assert!(
    canon_hook.starts_with(&canon_custom),
    "expected hook under {} (core.hooksPath target), got {}",
    canon_custom.display(),
    canon_hook.display()
  );
  // The legacy `.git/hooks/commit-msg` MUST NOT be created — leaving
  // a stale file there is misleading (the user would think the hook
  // is installed when git is actually reading from .githooks/).
  let legacy = dir.path().join(".git").join("hooks").join("commit-msg");
  assert!(
    !legacy.exists(),
    "installer must not write into .git/hooks/ when core.hooksPath is set; found stale {}",
    legacy.display()
  );
}
