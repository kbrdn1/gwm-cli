//! `gwm create` and `gwm doctor` print paths the way `gwm list` does.
//!
//! On Windows the expanded `[worktree].base` mixes `\` (from `{home}`) and
//! `/` (from the template), and `Path::join` adds a `\` segment, so the two
//! surfaces showed the same directory in two spellings. The display helper
//! settles on forward slashes, and leaves Unix paths untouched.

use gwm::worktree::display_with_slashes;
use std::path::Path;

#[test]
fn display_uses_forward_slashes_on_windows() {
  if !cfg!(windows) {
    return;
  }
  let mixed = Path::new(r"C:\Users\me/cc-worktree/repo\feat-1");
  assert_eq!(display_with_slashes(mixed), "C:/Users/me/cc-worktree/repo/feat-1");
}

#[test]
fn display_leaves_a_unix_path_alone() {
  if cfg!(windows) {
    return;
  }
  let path = Path::new("/home/me/cc-worktree/repo/feat-1");
  assert_eq!(display_with_slashes(path), "/home/me/cc-worktree/repo/feat-1");
}
