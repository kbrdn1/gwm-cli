//! Pin the contract of the shared CLI repo prelude `gwm::cli::repo_context`
//! (and its lenient sibling) extracted in issue #236. The triplet
//! `discover_repo → workdir → Config::load_for_repo` used to be copy-pasted
//! across ~half a dozen subcommands; these tests lock its two observable
//! branches so the dedupe stays behaviour-preserving:
//!
//! - Outside a git repository (or in a bare repo with no workdir) the
//!   prelude must surface `GwmError::NotInGitRepo`.
//! - Inside a real working tree the repo, workdir, and config must all
//!   resolve, with `workdir` pointing at the discovered tree.
//!
//! Both helpers take an explicit `start` path (mirroring
//! `worktree::discover_repo`), so the tests target a `tempfile::TempDir`
//! directly without mutating the process-wide current directory — no
//! cross-test cwd races.

mod common;

use gwm::cli::{repo_context, repo_context_lenient, RepoContext};
use gwm::error::GwmError;
use tempfile::TempDir;

/// `repo_context` against a directory that is not a git repository must
/// fail with the `NotInGitRepo` gate, not panic or load a config.
#[test]
fn repo_context_outside_repo_is_not_in_git_repo() {
  let not_a_repo = TempDir::new().unwrap();

  match repo_context(Some(not_a_repo.path())) {
    Err(GwmError::NotInGitRepo) => {}
    Err(other) => panic!("expected NotInGitRepo, got {other:?}"),
    Ok(_) => panic!("a bare tempdir is not a git repo, expected an error"),
  }
}

/// The lenient variant keeps the repo/workdir gate strict: outside a repo
/// it must still surface `NotInGitRepo` (it is lenient only on the config
/// *load*, not on the repo discovery).
#[test]
fn repo_context_lenient_outside_repo_is_not_in_git_repo() {
  let not_a_repo = TempDir::new().unwrap();

  match repo_context_lenient(Some(not_a_repo.path())) {
    Err(GwmError::NotInGitRepo) => {}
    Err(other) => panic!("expected NotInGitRepo, got {other:?}"),
    Ok(_) => panic!("a bare tempdir is not a git repo, expected an error"),
  }
}

/// Happy path: inside a real working tree the repo opens, the workdir
/// resolves to that tree, and the config is actually loaded *from that
/// tree's* `.gwm.toml` (not silently defaulted). We prove the latter by
/// writing a non-default `[[branch_types]]` block and asserting it
/// replaces the built-in list — distinguishing "loaded from disk" from
/// "hard-coded to default".
#[test]
fn repo_context_inside_repo_resolves_repo_workdir_and_config() {
  let (dir, _repo) = common::init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[[branch_types]]
name = "spike"
description = "Throwaway exploration"
"#,
  )
  .unwrap();

  let Ok(RepoContext { repo, workdir, config }) = repo_context(Some(dir.path())) else {
    panic!("temp repo should resolve a repo context");
  };

  // The discovered repo points at the same working tree we created.
  assert!(
    common::paths_equal(repo.workdir().unwrap(), dir.path()),
    "repo.workdir() {:?} should match the temp repo {:?}",
    repo.workdir(),
    dir.path()
  );
  assert!(
    common::paths_equal(&workdir, dir.path()),
    "context workdir {:?} should match the temp repo {:?}",
    workdir,
    dir.path()
  );

  // The config was read from this tree's `.gwm.toml`: the custom
  // `[[branch_types]]` replaces the built-in list. A defaulted config
  // would carry `feat`/`fix`/… and never `spike`.
  let resolved = config.resolved_branch_types();
  let names: Vec<&str> = resolved.types.iter().map(|t| t.name.as_str()).collect();
  assert_eq!(
    names,
    vec!["spike"],
    "config must come from the repo's .gwm.toml, not a silent default"
  );
}
