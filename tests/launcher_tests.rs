//! Unit tests for the shared launcher machinery — placeholder expansion,
//! base-resolution chain, `{diff}` lazy materialisation, `which::which`
//! probing. Issue #75.

mod common;

use common::init_repo;
use gwm::config::ResolvedLauncher;
use gwm::launcher::{
  count_commits_ahead, expand_command, git_diff_argv, git_rev_list_count_argv, locate_binary, missing_binary_for,
  resolve_review_base, write_gwm_base, LauncherContext,
};
use std::path::Path;

fn ctx<'a>(path: &'a Path, base: Option<&'a str>, head: Option<&'a str>) -> LauncherContext<'a> {
  LauncherContext {
    worktree_path: path,
    base,
    head,
    repo_workdir: Some(path),
  }
}

#[test]
fn expand_substitutes_path_placeholder() {
  // `[git_tui]` only needs `{path}`. The launcher must work without a
  // base/head context — that's the contract for the `l` keybinding.
  let c = ctx(Path::new("/tmp/wt/x"), None, None);
  let cmd = expand_command("lazygit -p {path}", &c).unwrap();
  assert_eq!(cmd.argv, vec!["lazygit", "-p", "/tmp/wt/x"]);
  assert!(
    cmd.diff_file.is_none(),
    "no {{diff}} in template ⇒ no tempfile materialised"
  );
}

#[test]
fn expand_substitutes_base_head_path() {
  // The review-style template wires base + head into a `git diff` style
  // expression. shell-words splits on whitespace — `{base}..{head}`
  // resolves to one token, not two.
  let c = ctx(Path::new("/tmp/wt/x"), Some("dev"), Some("feat/foo"));
  let cmd = expand_command("lumen diff {base}..{head}", &c).unwrap();
  assert_eq!(cmd.argv, vec!["lumen", "diff", "dev..feat/foo"]);
}

#[test]
fn expand_respects_shell_words_quoting() {
  // The primary contract from the issue: the user can pass any shell
  // line. shell-words must keep quoted arguments together.
  let c = ctx(Path::new("/tmp/wt/x"), Some("dev"), Some("feat/foo"));
  let cmd = expand_command("claude --print 'review {base}..{head}'", &c).unwrap();
  assert_eq!(
    cmd.argv,
    vec!["claude", "--print", "review dev..feat/foo"],
    "quoted argument must stay one token even after placeholder expansion"
  );
}

#[test]
fn expand_errors_on_unsubstitutable_base() {
  // A template that names `{base}` without a context-provided base is a
  // configuration bug — refuse loudly instead of running the command
  // with a literal `{base}` token.
  let c = ctx(Path::new("/tmp/wt/x"), None, None);
  let err = expand_command("lumen diff {base}..HEAD", &c).unwrap_err();
  let msg = format!("{}", err);
  assert!(
    msg.contains("{base}"),
    "error must mention the missing placeholder: {}",
    msg
  );
}

#[test]
fn expand_errors_on_diff_without_repo() {
  // `{diff}` requires a repo workdir to shell out `git diff`. Refuse if
  // the caller didn't supply one (defensive — the TUI always does).
  let c = LauncherContext {
    worktree_path: Path::new("/tmp/wt/x"),
    base: Some("dev"),
    head: Some("feat/foo"),
    repo_workdir: None,
  };
  let err = expand_command("reviewer --diff {diff}", &c).unwrap_err();
  let msg = format!("{}", err);
  assert!(
    msg.contains("workdir") || msg.contains("repo"),
    "diff error should mention the missing repo workdir: {}",
    msg
  );
}

#[test]
fn expand_materialises_diff_lazily() {
  // The {diff} placeholder is the only one that triggers a `git diff`
  // shell-out. A template that doesn't reference it must not pay that
  // cost — which we assert above. Here we verify that the tempfile is
  // produced when {diff} *is* referenced, and that the path makes its
  // way into the argv.
  let (dir, repo) = init_repo();
  // Add a second commit so `git diff` has something to render.
  std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
  let mut index = repo.index().unwrap();
  index.add_path(Path::new("a.txt")).unwrap();
  let tree_id = index.write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  let parent = repo.head().unwrap().peel_to_commit().unwrap();
  let sig = git2::Signature::now("t", "t@test").unwrap();
  let commit_oid = repo.commit(None, &sig, &sig, "add a", &tree, &[&parent]).unwrap();
  // Create a `feat` branch on the new commit, leave `main` at the
  // original commit. `git diff main..feat` will then emit the new file.
  let new_commit = repo.find_commit(commit_oid).unwrap();
  repo.branch("feat", &new_commit, false).unwrap();

  let c = LauncherContext {
    worktree_path: dir.path(),
    base: Some("main"),
    head: Some("feat"),
    repo_workdir: Some(dir.path()),
  };
  let cmd = expand_command("reviewer --diff {diff}", &c).unwrap();
  assert_eq!(cmd.argv[0], "reviewer");
  assert_eq!(cmd.argv[1], "--diff");
  let diff_path = &cmd.argv[2];
  assert!(
    Path::new(diff_path).exists(),
    "{{diff}} placeholder must produce a real tempfile at: {}",
    diff_path
  );
  let body = std::fs::read_to_string(diff_path).unwrap();
  assert!(
    body.contains("a.txt"),
    "git diff output should mention the file: {}",
    body
  );
  assert!(
    cmd.diff_file.is_some(),
    "ExpandedCommand must keep the tempfile alive until drop"
  );
}

// ---- Base resolution chain ----------------------------------------------

#[test]
fn resolve_base_falls_back_to_main_when_no_dev_branch_exists() {
  // PR #76 Copilot review: the docstring promises a `"dev" → "main"`
  // fallback. `init_repo` creates only `main`, so resolution must
  // land on `main` instead of returning a non-existent `dev` that
  // would later make `git diff` / `git rev-list` fail.
  let (_dir, repo) = init_repo();
  let base = resolve_review_base(&repo, "feat/x", None);
  assert_eq!(base, "main", "no dev branch ⇒ fall through to main");
}

#[test]
fn resolve_base_falls_back_to_dev_when_dev_branch_exists() {
  // The "dev" fallback is gwm's project convention. We only prefer
  // it when it actually exists locally — otherwise we'd hand a bogus
  // ref to `git diff`. Mirror of the `main` fallback test.
  let (_dir, repo) = init_repo();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("dev", &head, false).unwrap();
  let base = resolve_review_base(&repo, "feat/x", None);
  assert_eq!(base, "dev", "dev branch present ⇒ prefer it over main");
}

#[test]
fn resolve_base_uses_config_default_when_set() {
  // `[review].default_base` overrides the static "dev" fallback.
  let (_dir, repo) = init_repo();
  let base = resolve_review_base(&repo, "feat/x", Some("trunk"));
  assert_eq!(base, "trunk");
}

#[test]
fn resolve_base_prefers_gwm_base_over_default() {
  // `branch.<n>.gwm-base` is set by `gwm create` so the parent ref is
  // recoverable even on branches without an upstream.
  let (_dir, repo) = init_repo();
  write_gwm_base(&repo, "feat/x", "release-1.x").unwrap();
  let base = resolve_review_base(&repo, "feat/x", Some("trunk"));
  assert_eq!(base, "release-1.x");
}

#[test]
fn resolve_base_prefers_upstream_over_gwm_base() {
  // The upstream ref is the strongest signal — the user is actively
  // tracking it via `git push -u`. It must outrank `gwm-base`.
  let (_dir, repo) = init_repo();
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.feat/x.merge", "refs/heads/staging").unwrap();
  }
  write_gwm_base(&repo, "feat/x", "release-1.x").unwrap();
  let base = resolve_review_base(&repo, "feat/x", Some("trunk"));
  assert_eq!(base, "staging", "branch.<n>.merge must outrank gwm-base");
}

#[test]
fn resolve_base_strips_refs_heads_prefix_from_merge() {
  // `branch.<n>.merge` is stored as a refspec (`refs/heads/dev`); the
  // launcher hands the value straight to `git diff`, so the short name
  // must come out.
  let (_dir, repo) = init_repo();
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.feat/x.merge", "refs/heads/dev").unwrap();
  }
  let base = resolve_review_base(&repo, "feat/x", None);
  assert_eq!(base, "dev");
}

#[test]
fn resolve_base_empty_strings_in_config_are_ignored() {
  // A leftover empty `branch.<n>.merge = ""` must not poison the chain.
  let (_dir, repo) = init_repo();
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.feat/x.merge", "").unwrap();
  }
  let base = resolve_review_base(&repo, "feat/x", Some("trunk"));
  assert_eq!(base, "trunk", "empty merge must fall through to the next chain step");
}

#[test]
fn resolve_base_skips_self_tracking_upstream() {
  // Regression for #117. After the canonical `gwm create … && git push -u
  // origin <branch>` workflow, git records `branch.<n>.merge =
  // refs/heads/<same-branch>` because the local branch tracks its own
  // remote-side copy. Returning the branch's own short name from priority-1
  // makes `git rev-list --count <branch>..<branch> = 0`, so the launcher's
  // `skip_when_no_changes` knob silently swallows the `R: review`
  // keystroke — the user gets nothing, no error, no diff.
  //
  // The fix is to treat a merge value equal to the branch itself as
  // "not a usable upstream" and fall through to the next chain step
  // (gwm-base, then [review].default_base, then dev / main).
  let (_dir, repo) = init_repo();
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.feat/x.merge", "refs/heads/feat/x").unwrap();
  }
  write_gwm_base(&repo, "feat/x", "release-1.x").unwrap();
  let base = resolve_review_base(&repo, "feat/x", Some("trunk"));
  assert_eq!(
    base, "release-1.x",
    "self-tracking upstream must be ignored — fall through to gwm-base"
  );
}

#[test]
fn resolve_base_skips_self_tracking_upstream_short_form() {
  // Defensive: some git versions / manual edits drop the `refs/heads/`
  // prefix, leaving the value as a bare branch name. The self-equality
  // check must run on the stripped short form too, otherwise a bare
  // `branch.feat/x.merge = feat/x` slips past the guard.
  let (_dir, repo) = init_repo();
  {
    let mut cfg = repo.config().unwrap();
    cfg.set_str("branch.feat/x.merge", "feat/x").unwrap();
  }
  let base = resolve_review_base(&repo, "feat/x", Some("trunk"));
  assert_eq!(
    base, "trunk",
    "bare self-tracking upstream must be ignored too — fall through to default_base"
  );
}

// ---- count_commits_ahead ------------------------------------------------

#[test]
fn count_commits_ahead_is_zero_on_identical_refs() {
  let (dir, _repo) = init_repo();
  let n = count_commits_ahead(dir.path(), "HEAD", "HEAD");
  assert_eq!(n, 0, "HEAD..HEAD must always be 0");
}

#[test]
fn count_commits_ahead_returns_zero_on_git_failure() {
  // Pointing at a non-existent base should not panic — the launcher's
  // `skip_when_no_changes` path treats 0 as "nothing to review", which
  // is the safer default when we can't tell.
  let (dir, _repo) = init_repo();
  let n = count_commits_ahead(dir.path(), "does-not-exist", "HEAD");
  assert_eq!(n, 0);
}

// ---- missing binary probe -----------------------------------------------

#[test]
fn missing_binary_for_returns_some_when_absent() {
  // A garbage binary name must come back so the doctor / status bar can
  // surface it verbatim.
  let l = ResolvedLauncher {
    command: "definitely-not-a-real-binary-3afe2c {path}".into(),
    fullscreen: false,
  };
  assert_eq!(
    missing_binary_for(&l).as_deref(),
    Some("definitely-not-a-real-binary-3afe2c")
  );
}

#[test]
fn missing_binary_for_returns_none_when_present() {
  // `sh` is universally on POSIX PATH (the project's CI matrix). Pick a
  // command that's nearly certain to resolve — windows isn't in the
  // test matrix anyway.
  let l = ResolvedLauncher {
    command: "sh -c 'echo {base}'".into(),
    fullscreen: false,
  };
  assert!(
    missing_binary_for(&l).is_none(),
    "/bin/sh must resolve on every supported platform"
  );
}

#[test]
fn locate_binary_finds_resolved_argv0() {
  // Sanity: `sh -c 'echo hi'` expands to argv `[sh, -c, echo hi]`,
  // which the locator must find.
  let c = ctx(Path::new("/tmp"), None, None);
  let cmd = expand_command("sh -c 'echo hi'", &c).unwrap();
  assert!(locate_binary(&cmd).is_some());
}

// --- Issue #100: --end-of-options guards before user-derived refs -------

#[test]
fn git_diff_argv_inserts_end_of_options_before_refspec() {
  // Issue #100 / CVE-2017-1000117 shape. `base` and `head` come from
  // user-controlled surfaces (`branch.<n>.merge`, `[review].default_base`,
  // branch names). Without `--end-of-options` a value like
  // `--upload-pack=/tmp/x` would be re-parsed as a git option. The
  // separator MUST appear before the refspec token so every following
  // arg is treated as positional.
  let argv = git_diff_argv("dev", "feat/#1-x");
  assert_eq!(argv, vec!["diff", "--end-of-options", "dev..feat/#1-x"]);

  let sep_idx = argv.iter().position(|a| a == "--end-of-options").unwrap();
  let refspec_idx = argv.iter().position(|a| a.contains("..")).unwrap();
  assert!(
    sep_idx < refspec_idx,
    "--end-of-options must precede the refspec; argv: {:?}",
    argv
  );
}

#[test]
fn git_diff_argv_keeps_separator_first_for_dash_prefixed_base() {
  // The whole point of the guard: a malicious base value like
  // `--upload-pack=/tmp/x` ends up after `--end-of-options` so git
  // treats it as a ref name (which will fail to resolve, surfacing as
  // a clean error) instead of executing the option.
  let argv = git_diff_argv("--upload-pack=/tmp/x", "HEAD");
  assert_eq!(argv[0], "diff");
  assert_eq!(argv[1], "--end-of-options");
  assert!(
    argv[2].starts_with("--upload-pack=/tmp/x.."),
    "malicious base must land as a positional ref after the separator; argv: {:?}",
    argv
  );
}

#[test]
fn git_rev_list_count_argv_inserts_end_of_options_before_refspec() {
  let argv = git_rev_list_count_argv("dev", "HEAD");
  assert_eq!(argv, vec!["rev-list", "--count", "--end-of-options", "dev..HEAD"]);
}

#[test]
fn count_commits_ahead_treats_dash_prefixed_base_as_ref_not_flag() {
  // End-to-end smoke: pass a base that looks like a flag and prove the
  // shell-out doesn't drift to "git --version → 0 (parse error)" by
  // mistake. With the guard git tries to resolve the value as a ref,
  // fails to find it, exits non-zero, and `count_commits_ahead`
  // coalesces to 0. The argv-level tests above pin the actual
  // contract; this guards against future refactors that forget to
  // re-thread the helper.
  let (dir, _) = init_repo();
  let n = count_commits_ahead(dir.path(), "--version", "HEAD");
  assert_eq!(n, 0, "dash-prefixed base must never let git happily print a banner");
}
