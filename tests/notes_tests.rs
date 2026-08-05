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
    // Win32 reads the ISO 8859-1 superscripts as digits in a device name, so
    // `COM¹` is `COM1`. The full list already lives in `naming.rs`; a second,
    // shorter copy in `notes.rs` is what let these through (Codex review, PR
    // #530).
    "COM¹",
    "nested/LPT³",
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
fn two_branches_differing_only_in_case_are_refused_a_note() {
  // Measured, not assumed: `git branch feat/Foo` is refused while `feat/foo`
  // is a *loose* ref on a case-insensitive volume, and accepted once the
  // refs are packed. Both then show up in `git branch --list`, so two live
  // branches map to one note file and editing either one silently rewrites
  // the other's prose (Codex review, PR #530).
  let (dir, repo) = init_repo();
  repo
    .branch("feat/foo", &repo.head().unwrap().peel_to_commit().unwrap(), false)
    .unwrap();
  let packed = std::process::Command::new("git")
    .args(["pack-refs", "--all"])
    .current_dir(dir.path())
    .status()
    .expect("git is on PATH");
  assert!(packed.success(), "git pack-refs --all failed");
  repo
    .branch("feat/Foo", &repo.head().unwrap().peel_to_commit().unwrap(), false)
    .expect("a packed ref no longer blocks the case variant");

  let err =
    notes::prepare(&repo, "feat/Foo").expect_err("a branch that shares a note file with another must be refused");
  let message = err.to_string();
  assert!(
    message.contains("feat/foo") && message.contains("feat/Foo"),
    "the refusal has to name both branches, it is the user who picks which one to rename: {message}"
  );
  assert!(
    !notes::notes_dir(&repo).join("feat").join("Foo.md").exists(),
    "nothing is written for a refused branch"
  );
}

#[test]
fn a_branch_with_no_case_variant_still_prepares() {
  // The counterpart: the guard above must not turn every note into a branch
  // walk that refuses on its own name.
  let (_dir, repo) = init_repo();
  repo
    .branch("feat/foo", &repo.head().unwrap().peel_to_commit().unwrap(), false)
    .unwrap();

  assert!(notes::prepare(&repo, "feat/foo").unwrap().is_some());
  // And a branch with no ref at all (a note prepared before the branch is
  // created) is not a collision either.
  assert!(notes::prepare(&repo, "feat/never-created").unwrap().is_some());
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

// ---------------------------------------------------------------------------
// Never destroy prose (Codex review, PR #530)
// ---------------------------------------------------------------------------

#[test]
fn rename_refuses_to_overwrite_a_note_already_at_the_destination() {
  // `git branch -m old new` fails when `new` already exists, so a note found
  // under `new` at this point is necessarily an orphan left by a previous
  // branch of that name. `fs::rename` replaces it silently on Unix, which
  // destroys prose that nothing else can recover.
  let (_dir, repo) = init_repo();
  write_note(&repo, "feat/#515-old", "the note I am carrying over\n");
  write_note(&repo, "feat/#515-new", "prose from a previous life of this name\n");

  assert!(
    !notes::rename(&repo, "feat/#515-old", "feat/#515-new").unwrap(),
    "an occupied destination must not be moved onto"
  );
  assert_eq!(
    notes::read(&repo, "feat/#515-old").as_deref(),
    Some("the note I am carrying over\n"),
    "the source note stays put rather than vanishing"
  );
  assert_eq!(
    notes::read(&repo, "feat/#515-new").as_deref(),
    Some("prose from a previous life of this name\n"),
    "and the destination is untouched"
  );
}

#[test]
fn a_blank_note_at_the_destination_does_not_block_the_move() {
  // Presence is non-blank everywhere, this rule included: overwriting a file
  // an editor left empty loses nothing.
  let (_dir, repo) = init_repo();
  write_note(&repo, "feat/#515-old", "still relevant\n");
  write_note(&repo, "feat/#515-new", "\n");

  assert!(notes::rename(&repo, "feat/#515-old", "feat/#515-new").unwrap());
  assert_eq!(notes::read(&repo, "feat/#515-new").as_deref(), Some("still relevant\n"));
}

#[test]
fn occupied_by_answers_what_a_move_would_destroy() {
  let (_dir, repo) = init_repo();
  assert_eq!(notes::occupied_by(&repo, "feat/#515-new"), None);

  write_note(&repo, "feat/#515-new", "prose\n");
  assert_eq!(
    notes::occupied_by(&repo, "feat/#515-new"),
    notes::path_for(&repo, "feat/#515-new")
  );
}

#[test]
fn a_parent_component_never_ends_in_the_note_extension() {
  // `foo` maps to the FILE `foo.md`, so a branch `foo.md/bar` would want the
  // DIRECTORY `foo.md`. Both branches are legal in git and can coexist, and
  // whichever note is written first makes the other impossible. Files always
  // end in `.md` and directories now never do, so the two sets are disjoint
  // by construction.
  assert_eq!(notes::relative_path("foo.md/bar"), None);
  assert_eq!(notes::relative_path("a/b.md/c"), None);
  // The final component is free: `foo.md` is the file `foo.md.md`, which
  // collides with nothing.
  assert_eq!(
    notes::relative_path("foo.md"),
    Some(PathBuf::from("foo.md.md")),
    "a branch may still be named after a Markdown file"
  );
}

#[test]
fn neither_order_of_the_colliding_pair_can_break_the_other() {
  let (_dir, repo) = init_repo();

  // `foo` first, then the branch that would want `foo.md` as a directory.
  write_note(&repo, "foo", "the flat one\n");
  assert_eq!(notes::prepare(&repo, "foo.md/bar").unwrap(), None);
  assert_eq!(notes::read(&repo, "foo").as_deref(), Some("the flat one\n"));

  // And the reverse order, on a name nothing has touched yet.
  assert_eq!(notes::prepare(&repo, "baz.md/qux").unwrap(), None);
  write_note(&repo, "baz", "still writable\n");
  assert_eq!(notes::read(&repo, "baz").as_deref(), Some("still writable\n"));
}

#[test]
fn an_unreadable_note_at_the_destination_blocks_the_move_too() {
  // "Absent" and "I could not read it" are different answers, and only the
  // first makes a move safe. `read` collapses both to `None` because it
  // answers *does this worktree carry a note* for display; overwriting asks
  // a different question and must not reuse that answer (Codex review, PR
  // #530, pass 2).
  let (_dir, repo) = init_repo();
  write_note(&repo, "feat/#515-old", "the note I am carrying over\n");
  let occupied = notes::prepare(&repo, "feat/#515-new").unwrap().unwrap();
  // Invalid UTF-8: prose an editor could have written from a latin-1 paste.
  // `read_to_string` fails on it, so the destination reads as free unless
  // the failure itself is treated as occupied.
  std::fs::write(&occupied, [0xff, 0xfe, b'h', b'i']).unwrap();

  assert!(
    notes::occupied_by(&repo, "feat/#515-new").is_some(),
    "a file that exists but cannot be read is not a free destination"
  );
  assert!(!notes::rename(&repo, "feat/#515-old", "feat/#515-new").unwrap());
  assert_eq!(
    std::fs::read(&occupied).unwrap(),
    vec![0xff, 0xfe, b'h', b'i'],
    "the unreadable file must survive byte for byte"
  );
  assert_eq!(
    notes::read(&repo, "feat/#515-old").as_deref(),
    Some("the note I am carrying over\n")
  );
}

#[test]
fn a_destination_that_is_provably_blank_is_still_free() {
  // The counterpart: only a file read successfully AND found blank may be
  // replaced. Without this the previous test's rule would block every move
  // onto an editor's leftover empty file.
  let (_dir, repo) = init_repo();
  write_note(&repo, "feat/#515-blank", "   \n\n");

  assert_eq!(notes::occupied_by(&repo, "feat/#515-blank"), None);
  assert_eq!(notes::occupied_by(&repo, "feat/#515-absent"), None);
}

#[test]
fn the_note_extension_is_matched_without_regard_to_case() {
  // macOS's default filesystem and Windows both fold case, so `foo.MD/bar`
  // wants the directory `foo.MD` and `foo` wants the file `foo.md`, which are
  // the same path there. The rule that keeps the suffix off directory names
  // has to fold case too, or it holds on Linux and not where most of the
  // users are (Codex review, PR #530, pass 4).
  for branch in ["foo.MD/bar", "foo.Md/bar", "a/b.mD/c"] {
    assert_eq!(notes::relative_path(branch), None, "{branch:?} must carry no note");
  }
}

#[test]
fn a_case_only_rename_is_not_its_own_occupied_destination() {
  // `git branch -m feat/foo feat/Foo` is a valid rename. On a case-folding
  // volume (macOS's default, Windows) the destination path opens the SOURCE
  // file, so the "never overwrite" guard read it as occupied prose and
  // refused a rename that worked before the guard existed. A guard that
  // breaks the case it was not written for is a regression, not hardening
  // (Codex review, PR #530, pass 5).
  //
  // What this exercises depends on the runner, and that is the point: on a
  // case-sensitive volume the two paths are two files and the move is
  // ordinary; on a folding one they are one file and only the same-file
  // check lets it through. The assertion holds either way.
  let (_dir, repo) = init_repo();
  write_note(&repo, "feat/foo", "same work, better capitalisation\n");

  assert!(
    notes::rename(&repo, "feat/foo", "feat/Foo").unwrap(),
    "a case-only rename must move the note rather than read as occupied"
  );
  assert_eq!(
    notes::read(&repo, "feat/Foo").as_deref(),
    Some("same work, better capitalisation\n")
  );
}
