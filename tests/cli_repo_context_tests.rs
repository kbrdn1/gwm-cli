//! Pin the contract of the shared CLI repo prelude `gwm::cli::repo_context`
//! (and its lenient sibling) extracted in issue #236. The triplet
//! `discover_repo → workdir → Config::load_for_repo` used to be copy-pasted
//! across ~half a dozen subcommands; these tests lock its two observable
//! branches so the dedupe stays behaviour-preserving:
//!
//! - Outside a git repository the prelude must surface
//!   `GwmError::NotInGitRepo`. A bare repo is a repository (#744), see
//!   the last test.
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

/// Issue #744: in a bare repo with linked worktrees, the context resolves
/// from the project directory and from inside a worktree alike, and the
/// config is the one committed to the default branch: the `.gwm.toml` of
/// the worktree that has it checked out, as in a normal clone's main
/// checkout.
#[test]
fn repo_context_in_bare_gitlink_layout_loads_the_default_branch_config() {
  let (_tmp, project, main) = common::init_bare_gitlink_layout();
  std::fs::write(
    main.join(".gwm.toml"),
    r#"
[[branch_types]]
name = "spike"
description = "Throwaway exploration"
"#,
  )
  .unwrap();

  for start in [&project, &main] {
    let RepoContext { workdir, config, .. } = match repo_context(Some(start)) {
      Ok(ctx) => ctx,
      Err(e) => panic!("from {start:?}: expected a repo context, got {e:?}"),
    };
    assert!(
      common::paths_equal(&workdir, &main),
      "from {start:?}: expected {main:?}, got {workdir:?}"
    );
    let names: Vec<String> = config
      .resolved_branch_types()
      .types
      .into_iter()
      .map(|t| t.name)
      .collect();
    assert_eq!(
      names,
      vec!["spike"],
      "from {start:?}: config must come from the default branch's .gwm.toml"
    );
  }
}
