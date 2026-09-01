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

/// A directory holding nothing but a **working** `git`, for a test that hands
/// a subprocess a minimal `PATH`.
///
/// Two traps, one after the other. `/usr/bin:/bin` assumes a git in
/// `/usr/bin`, which is not a property of a POSIX system: where git comes from
/// nix or Homebrew, `/usr/bin/git` is the Xcode shim, and with no command line
/// tools installed it writes nothing, prints an install prompt on stderr and
/// **exits 0**. A caller that only checks the exit status reads that as an
/// empty answer. So the candidate is run before it is trusted, and one that
/// does not answer `git version …` is refused by name rather than symlinked
/// into the shim directory for every case to fail against.
///
/// A directory of its own rather than git's: the tools these suites stub are
/// meant to be a stub or absent, and git's neighbours on a Homebrew prefix or
/// a nix profile can include a real `cargo` or `gwm`, which would silently
/// defeat the case that wants one missing.
#[cfg(unix)]
#[allow(dead_code)] // used by the suites that drive a shell script.
pub fn git_only_bin() -> &'static Path {
  use std::sync::OnceLock;
  static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
  DIR
    .get_or_init(|| {
      let found = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v git")
        .output()
        .expect("locating git ran");
      let git = String::from_utf8_lossy(&found.stdout).trim().to_string();
      assert!(
        !git.is_empty(),
        "git must be on PATH: this suite hands a subprocess a minimal PATH and the script under \
         test opens on a git command"
      );
      let version = std::process::Command::new(&git)
        .arg("--version")
        .output()
        .expect("git --version ran");
      let reported = String::from_utf8_lossy(&version.stdout);
      assert!(
        reported.starts_with("git version"),
        "`{git}` is on PATH but does not answer `git version …` (it said {reported:?}). On macOS \
         that is the Xcode shim with no command line tools behind it: it exits 0 having done \
         nothing, so every case here would fail against an empty answer. Install the tools, or \
         run this suite with the real git first on PATH"
      );
      let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shell-suite-git-bin");
      std::fs::create_dir_all(&dir).expect("the git shim directory is creatable");
      let link = dir.join("git");
      // Recreated rather than reused: the toolchain moves between runs, and a
      // symlink to a garbage-collected nix store path resolves to nothing.
      let _ = std::fs::remove_file(&link);
      std::os::unix::fs::symlink(&git, &link).expect("git symlinks into the shim directory");
      dir
    })
    .as_path()
}
