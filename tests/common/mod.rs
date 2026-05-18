//! Shared test helpers. `mod.rs` to opt out of being picked up as its own
//! integration target (cargo treats top-level `tests/*.rs` files as targets).

use git2::{Repository, Signature};
use std::path::Path;
use tempfile::TempDir;

/// Initialize a tempdir with a fresh git repo on `main` carrying one empty
/// commit. Returns the tempdir (kept alive by the caller) and the repo handle.
pub fn init_repo() -> (TempDir, Repository) {
  let dir = TempDir::new().unwrap();
  let repo = Repository::init(dir.path()).unwrap();
  repo.set_head("refs/heads/main").ok();

  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  let tree_id = {
    let mut index = repo.index().unwrap();
    index.write_tree().unwrap()
  };
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();

  let reopened = Repository::open(dir.path()).unwrap();
  (dir, reopened)
}

/// Canonicalize two paths and compare them. On macOS `/var/...` and
/// `/private/var/...` denote the same inode but compare unequal as strings.
#[allow(dead_code)] // used only by worktree_integration; cargo compiles common per-test crate.
pub fn paths_equal(a: &Path, b: &Path) -> bool {
  let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
  let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
  a == b
}
