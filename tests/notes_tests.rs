//! Storage layer for per-worktree notes (issue #515).
//!
//! Two invariants carry the whole feature and are pinned first:
//!
//! 1. **Round trip.** `branch_from_relative(relative_path(b)) == b`, and
//!    the strings [`gwm::notes::branches_with_notes`] hands back must
//!    compare `==` to `WorktreeInfo.branch` on macOS, Linux **and**
//!    Windows. That is what forces the explicit `/` re-join instead of a
//!    `to_string_lossy()` of the relative path, which yields
//!    `feat\#515-x` on a Windows runner (green locally, red on one CI leg).
//! 2. **Presence is "non-blank".** `vi` over an empty buffer writes one
//!    byte, so neither "the file exists" nor "len() > 0" answers *does this
//!    worktree carry a note*.

mod common;

use common::init_repo;
use gwm::notes;
use std::path::{Path, PathBuf};

/// Branch names a real gwm repo produces, plus the awkward ones git allows.
const ROUND_TRIP_BRANCHES: &[&str] = &[
  "main",
  "feat/#515-worktree-notes",
  "fix/#17-locked-worktree",
  "release/3.x/hotfix",
  "user@host",
  "accentué-branché",
  "dots.in.the.name",
  "note.md",
];

fn write_note(repo: &git2::Repository, branch: &str, body: &str) -> PathBuf {
  let path = notes::prepare(repo, branch).unwrap().expect("branch backs a note file");
  std::fs::write(&path, body).unwrap();
  path
}

// ---------------------------------------------------------------------------
// 1. Round trip
// ---------------------------------------------------------------------------

#[test]
fn relative_path_round_trips_through_branch_from_relative() {
  for branch in ROUND_TRIP_BRANCHES {
    let rel = notes::relative_path(branch).unwrap_or_else(|| panic!("{branch} should back a note file"));
    assert_eq!(
      notes::branch_from_relative(&rel).as_deref(),
      Some(*branch),
      "round trip lost {branch} (relative path {})",
      rel.display()
    );
  }
}

#[test]
fn relative_path_mirrors_refs_heads_layout() {
  // The nested layout is what makes the store greppable and hand-editable
  // with gwm shut down — the point of choosing a file over a git-config key.
  assert_eq!(
    notes::relative_path("feat/#515-worktree-notes"),
    Some(PathBuf::from("feat").join("#515-worktree-notes.md"))
  );
  assert_eq!(notes::relative_path("main"), Some(PathBuf::from("main.md")));
}

#[test]
fn branches_with_notes_returns_branch_names_not_paths() {
  // The strings must be comparable to `WorktreeInfo.branch` with `==`. A
  // `to_string_lossy()` of the relative path would hand back a
  // backslash-separated name on Windows and silently match nothing there.
  let (_dir, repo) = init_repo();
  write_note(&repo, "feat/#515-worktree-notes", "the flaky test is the ETXTBSY one\n");
  write_note(&repo, "release/3.x/hotfix", "cherry-pick only\n");

  // Exact set, not a `contains` pair: on a Windows runner a
  // `to_string_lossy()` of the relative path yields `feat\#515-worktree-notes`,
  // which `contains` would report as a plain miss with no clue why.
  let expected: std::collections::BTreeSet<String> = ["feat/#515-worktree-notes", "release/3.x/hotfix"]
    .iter()
    .map(|s| s.to_string())
    .collect();

  assert_eq!(
    notes::branches_with_notes(&repo),
    expected,
    "branch names must come back slash-joined and complete"
  );
}

#[test]
fn a_branch_name_that_cannot_back_a_portable_file_carries_no_note() {
  // git accepts all of these; Windows accepts none. Returning `None` beats
  // writing a file that means a different branch once the repo is cloned
  // there (`feat.` and `feat` would share one note).
  for branch in [
    "trailing.",
    "trailing ",
    "angle<bracket",
    "angle>bracket",
    "pipe|name",
    "quote\"name",
    "con",
    "CON",
    "nested/CON",
    "com1",
    "..",
    "feat/../escape",
    "",
    "feat//empty",
  ] {
    assert_eq!(
      notes::relative_path(branch),
      None,
      "{branch:?} must not back a note file"
    );
  }
}

#[test]
fn branch_from_relative_rejects_paths_gwm_never_wrote() {
  assert_eq!(notes::branch_from_relative(Path::new("feat/x.txt")), None);
  assert_eq!(notes::branch_from_relative(Path::new(".md")), None);
  assert_eq!(notes::branch_from_relative(Path::new("../escape.md")), None);
}

// ---------------------------------------------------------------------------
// 2. Presence is "non-blank"
// ---------------------------------------------------------------------------

#[test]
fn a_blank_note_reads_as_no_note() {
  let (_dir, repo) = init_repo();
  // Exactly what `vi` leaves behind when the user saves an empty buffer:
  // one byte. `metadata().len() > 0` would light the table marker up.
  write_note(&repo, "main", "\n");

  assert_eq!(notes::read(&repo, "main"), None, "a one-newline file is not a note");
  assert!(
    !notes::branches_with_notes(&repo).contains("main"),
    "a blank note must not reach the table marker"
  );
}

#[test]
fn a_note_reads_back_verbatim() {
  let (_dir, repo) = init_repo();
  let body = "- [ ] check the ETXTBSY retry\n\n`--base dev`, not free text\n";
  write_note(&repo, "main", body);

  assert_eq!(notes::read(&repo, "main").as_deref(), Some(body));
  assert!(notes::branches_with_notes(&repo).contains("main"));
}

#[test]
fn an_absent_note_reads_as_none_without_creating_anything() {
  let (_dir, repo) = init_repo();
  assert_eq!(notes::read(&repo, "main"), None);
  assert!(
    !notes::notes_dir(&repo).exists(),
    "reading must not create the notes tree"
  );
}

#[test]
fn prepare_creates_the_directory_but_not_the_file() {
  // An editor opened and quit without saving must leave no note behind.
  let (_dir, repo) = init_repo();
  let path = notes::prepare(&repo, "feat/#515-worktree-notes").unwrap().unwrap();

  assert!(path.parent().unwrap().is_dir(), "parent directory should exist");
  assert!(!path.exists(), "the note file itself is the editor's to create");
  assert_eq!(notes::read(&repo, "feat/#515-worktree-notes"), None);
}

// ---------------------------------------------------------------------------
// Storage location
// ---------------------------------------------------------------------------

#[test]
fn a_note_lands_inside_the_git_dir_so_it_is_never_committed() {
  // Living inside `.git` is what keeps the note private and disposable —
  // the property that separates it from the linked issue — and what makes
  // it survive `gwm remove` while staying readable from the main checkout.
  let (dir, repo) = init_repo();
  let path = write_note(&repo, "feat/#515-worktree-notes", "private\n");

  assert!(common::paths_equal(
    &path,
    &dir
      .path()
      .join(".git/gwm/notes/feat/#515-worktree-notes.md")
      .to_path_buf()
  ));
  assert!(path.is_file());
}

// ---------------------------------------------------------------------------
// Rename follows the branch
// ---------------------------------------------------------------------------

#[test]
fn rename_moves_the_note_to_the_new_branch() {
  let (_dir, repo) = init_repo();
  write_note(&repo, "feat/#515-old", "still relevant\n");

  assert!(notes::rename(&repo, "feat/#515-old", "feat/#515-new").unwrap());

  assert_eq!(notes::read(&repo, "feat/#515-old"), None);
  assert_eq!(notes::read(&repo, "feat/#515-new").as_deref(), Some("still relevant\n"));
}

#[test]
fn rename_across_nesting_depths_creates_the_target_directory() {
  let (_dir, repo) = init_repo();
  write_note(&repo, "flat", "body\n");

  assert!(notes::rename(&repo, "flat", "deep/nested/branch").unwrap());
  assert_eq!(notes::read(&repo, "deep/nested/branch").as_deref(), Some("body\n"));
}

#[test]
fn renaming_a_branch_without_a_note_is_a_no_op() {
  let (_dir, repo) = init_repo();
  assert!(!notes::rename(&repo, "feat/#515-old", "feat/#515-new").unwrap());
  assert_eq!(notes::read(&repo, "feat/#515-new"), None);
}
