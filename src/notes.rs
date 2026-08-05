//! Per-worktree notes (issue #515) — plain Markdown, one file per branch,
//! stored under `.git/gwm/notes/` **in the main checkout**.
//!
//! ## Why a file, and why there
//!
//! gwm already stores three per-branch keys in git config (`gwm-issue`,
//! `gwm-base`, `gwm-agent-pin`), and a fourth would have been free. It was
//! rejected: the round trip is lossless, but the on-disk form is a single
//! escaped line, which is not something to `grep`, pipe, or open in an
//! editor with gwm shut down — the whole reason the note is a file.
//!
//! The main checkout's git dir ([`git2::Repository::commondir`], shared by
//! every linked worktree) is the only location that takes all four columns
//! at once: plain Markdown on disk, readable from the main checkout,
//! surviving `gwm remove`, and never committed. The lifecycle rule is
//! stated once here: **the note lives as long as the branch**, and the
//! orphan question ("does this branch still exist?") is one git call, asked
//! by `gwm doctor`. Deleting notes is not `gwm clean`'s job: `clean`
//! reclaims regenerable build artefacts and its safety property is that
//! `--yes` only removes directories git already ignores.
//!
//! ## Consequences of keying on the branch
//!
//! - **A detached row carries no note.** No branch, no filename. The same
//!   rule the agent pin settled ([`crate::github::pinnable_branch`]).
//! - **A rename has to move the file**, which
//!   [`crate::worktree::rename_worktree`] does.
//! - **Recreating a branch of the same name adopts its note, on purpose.**
//!   Surviving `gwm remove` is the property that decided this storage over
//!   the in-worktree one, and the note is usually worth most between the
//!   removal and the merge. Recreating `feat/#515-notes` is resuming that
//!   work, restoring it with `gwm undo`, or reviewing the same PR again;
//!   in all three the old note is the thing you wanted back. Guarding
//!   `worktree::add` against it would trade the storage's stated advantage
//!   for a case the user can settle with one `rm` that `gwm doctor` has
//!   been naming since the branch went away.
//! - **A branch name is not always a legal filename.** git accepts `<`,
//!   `>`, `"`, `|` and a component ending in `.`; Windows accepts none of
//!   them (and silently normalises the trailing dot away, which would make
//!   two branches share one note). [`relative_path`] returns `None` for
//!   those rather than writing a file whose name means something else on
//!   another platform.
//!
//! ## Presence is "non-blank", not "exists"
//!
//! `vi` over an empty buffer writes one byte. So neither "the file exists"
//! nor "its length is non-zero" answers *does this worktree carry a note*;
//! the content has to be looked at. That single predicate lives in
//! [`read`], and every surface (the table marker, `gwm note show`, the JSON
//! field, the doctor check) reads presence through it.

use crate::error::Result;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

/// Filename extension every note carries. It is also what keeps a branch
/// `feat` and a branch `feat/x` from colliding on disk: `feat.md` is a
/// file, `feat/` a directory, and the two coexist.
const NOTE_EXT: &str = ".md";

/// Device names Windows reserves at every directory level, with or without
/// an extension — `CON.md` is `CON`. A branch named after one is legal in
/// git and unwritable there, so it carries no note anywhere (the check is
/// platform-independent on purpose: a note must not exist on macOS and
/// vanish on the same repo cloned to Windows).
const WINDOWS_RESERVED: &[&str] = &[
  "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
  "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// The notes directory for `repo` — `<main>/.git/gwm/notes`. `commondir`
/// rather than `path`, so a handle opened on a *linked* worktree resolves
/// to the same directory as one opened on the main checkout.
pub fn notes_dir(repo: &git2::Repository) -> PathBuf {
  repo.commondir().join("gwm").join("notes")
}

/// Whether one path component of a branch name can back a file of the same
/// name on every platform gwm ships to.
fn component_is_portable(c: &str) -> bool {
  if c.is_empty() || c == "." || c == ".." {
    return false;
  }
  // Windows strips a trailing dot or space, so `foo.` and `foo` would share
  // one note there and not here. git accepts `foo./bar` (it only rejects a
  // trailing dot on the ref as a whole), so this is reachable.
  if c.ends_with('.') || c.ends_with(' ') {
    return false;
  }
  if c
    .chars()
    .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*'))
  {
    return false;
  }
  let stem = c.split('.').next().unwrap_or_default().to_ascii_lowercase();
  !WINDOWS_RESERVED.contains(&stem.as_str())
}

/// Path of `branch`'s note **relative to [`notes_dir`]**, mirroring
/// `refs/heads/` — branch `feat/#515-notes` maps to `feat/#515-notes.md`.
///
/// `None` when the branch name cannot back a portable filename; the caller
/// says so in the status bar rather than writing a file that means a
/// different branch on another platform.
pub fn relative_path(branch: &str) -> Option<PathBuf> {
  let components: Vec<&str> = branch.split('/').collect();
  let (last, parents) = components.split_last()?;
  if !components.iter().all(|c| component_is_portable(c)) {
    return None;
  }
  // A parent component becomes a DIRECTORY, and every note is a FILE ending
  // in `.md`. Let a directory end in `.md` too and the two sets meet: branch
  // `foo` wants the file `foo.md`, branch `foo.md/bar` wants the directory
  // `foo.md`, both branches are legal in git and can coexist, and whichever
  // note is written first makes the other impossible (Codex review, PR
  // #530). Keeping the suffix off directory names makes the sets disjoint by
  // construction rather than by ordering luck. The LAST component is free:
  // branch `foo.md` is the file `foo.md.md`, which collides with nothing.
  // Case-folded, because the comparison has to hold where the filesystem
  // folds too: on macOS's default volume and on Windows, `foo.MD` and
  // `foo.md` are one path, so a case-sensitive check would keep the sets
  // disjoint on Linux only.
  if parents.iter().any(|c| c.to_ascii_lowercase().ends_with(NOTE_EXT)) {
    return None;
  }
  let mut out = PathBuf::new();
  for parent in parents {
    out.push(parent);
  }
  out.push(format!("{last}{NOTE_EXT}"));
  Some(out)
}

/// Inverse of [`relative_path`]: the branch a note file belongs to.
///
/// The components are re-joined with `/` **explicitly**, never through
/// `Path::display` / `to_string_lossy`, which would hand back
/// `feat\#515-notes` on Windows and never compare equal to
/// [`crate::worktree::WorktreeInfo::branch`].
pub fn branch_from_relative(rel: &Path) -> Option<String> {
  let mut parts: Vec<String> = Vec::new();
  for component in rel.components() {
    match component {
      Component::Normal(part) => parts.push(part.to_str()?.to_string()),
      // Anything else (`..`, a root, a prefix) is not a note we wrote.
      _ => return None,
    }
  }
  let last = parts.last_mut()?;
  let stem = last.strip_suffix(NOTE_EXT)?;
  if stem.is_empty() {
    return None;
  }
  *last = stem.to_string();
  Some(parts.join("/"))
}

/// Absolute path of `branch`'s note file. `None` for the same reason
/// [`relative_path`] is.
pub fn path_for(repo: &git2::Repository, branch: &str) -> Option<PathBuf> {
  Some(notes_dir(repo).join(relative_path(branch)?))
}

/// The note attached to `branch`, or `None` when there is none.
///
/// The single presence predicate: a file that is absent, unreadable, or
/// blank all read as "no note". Unreadable is deliberately not an error —
/// a permission problem on one row must not fail `gwm list --format=json`
/// or a daemon poll.
pub fn read(repo: &git2::Repository, branch: &str) -> Option<String> {
  let path = path_for(repo, branch)?;
  read_at(&path)
}

fn read_at(path: &Path) -> Option<String> {
  let text = std::fs::read_to_string(path).ok()?;
  (!text.trim().is_empty()).then_some(text)
}

/// Create the directory `branch`'s note lives in and return the file path,
/// ready to hand to `$EDITOR`. The file itself is not created — an editor
/// exited without saving leaves no note behind.
pub fn prepare(repo: &git2::Repository, branch: &str) -> Result<Option<PathBuf>> {
  let Some(path) = path_for(repo, branch) else {
    return Ok(None);
  };
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  Ok(Some(path))
}

/// Every branch carrying a non-blank note, in one walk of the notes tree.
///
/// This is what the table marker is built from: `worktree::list` calls it
/// once per refresh, so the render path never touches the filesystem
/// (issue #343 moved the per-row git reads off it; a file read per frame
/// would put one straight back).
pub fn branches_with_notes(repo: &git2::Repository) -> BTreeSet<String> {
  let base = notes_dir(repo);
  let mut files = Vec::new();
  collect_files(&base, &base, &mut files);
  files
    .into_iter()
    .filter(|(_, path)| read_at(path).is_some())
    .filter_map(|(rel, _)| branch_from_relative(&rel))
    .collect()
}

/// Depth-first walk yielding `(path relative to base, absolute path)`.
/// `DirEntry::file_type` does not follow symlinks, so a symlinked
/// directory is neither file nor dir here and is skipped — no loop.
fn collect_files(dir: &Path, base: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return;
  };
  for entry in entries.flatten() {
    let path = entry.path();
    match entry.file_type() {
      Ok(t) if t.is_dir() => collect_files(&path, base, out),
      Ok(t) if t.is_file() => {
        if let Ok(rel) = path.strip_prefix(base) {
          out.push((rel.to_path_buf(), path.clone()));
        }
      }
      _ => {}
    }
  }
}

/// The prose sitting under `branch`, if any: what a move onto it would
/// destroy, and equally what a move away from it would carry.
///
/// Deliberately **not** [`read`]. That one answers *does this worktree carry
/// a note* and collapses absent, unreadable and blank into `None`, which is
/// right for a marker and wrong here: "there is nothing there" and "I could
/// not read what is there" are different answers, and only the first makes a
/// move safe (Codex review, PR #530). A file that exists but fails to read
/// (invalid UTF-8 from a stray paste, a permission problem, a directory in
/// the way) is treated as occupied, so the only destination ever replaced is
/// one read successfully and found blank.
///
/// Preflighted by [`crate::worktree::rename_worktree`], which refuses the
/// whole rename before touching a ref rather than reporting the loss
/// afterwards.
pub fn occupied_by(repo: &git2::Repository, branch: &str) -> Option<PathBuf> {
  let path = path_for(repo, branch)?;
  match std::fs::read_to_string(&path) {
    // Read, and provably empty: replacing it loses nothing.
    Ok(text) if text.trim().is_empty() => None,
    Ok(_) => Some(path),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
    Err(_) => Some(path),
  }
}

/// The prose a move from `old_branch` to `new_branch` would destroy.
///
/// [`occupied_by`] answers "is there prose here"; this answers "would this
/// particular move lose any", which is not the same question when the two
/// names resolve to one file. `git branch -m feat/foo feat/Foo` is a valid
/// rename, and on a case-folding volume (macOS's default, Windows) the
/// destination path opens the SOURCE note: reading that as occupied refused
/// a rename that worked before the guard existed (Codex review, PR #530,
/// pass 5). The single place that distinction is made, so the preflight and
/// the move itself cannot disagree about it.
pub fn move_conflict(repo: &git2::Repository, old_branch: &str, new_branch: &str) -> Option<PathBuf> {
  let destination = occupied_by(repo, new_branch)?;
  let source = path_for(repo, old_branch)?;
  (!is_same_file(&source, &destination)).then_some(destination)
}

/// Whether two paths name one file. Compared through `canonicalize` rather
/// than as strings, since a case-folding volume is exactly the case where
/// two different strings are one file; falls back to string equality when
/// either side cannot be resolved.
fn is_same_file(a: &Path, b: &Path) -> bool {
  a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Follow a branch rename (#479). Returns `true` when a note was actually
/// moved, `false` when there was nothing to move, either name cannot back a
/// file, or the move would destroy prose already at the destination.
///
/// Never overwrites (Codex review, PR #530). `git branch -m old new` fails
/// when `new` already exists, so a note found there is necessarily an orphan
/// left by a previous branch of that name, and `fs::rename` would replace it
/// silently on Unix. Prose nothing can regenerate is not something to lose
/// to a name reuse.
pub fn rename(repo: &git2::Repository, old_branch: &str, new_branch: &str) -> Result<bool> {
  let (Some(from), Some(to)) = (path_for(repo, old_branch), path_for(repo, new_branch)) else {
    return Ok(false);
  };
  if from == to || !from.is_file() || move_conflict(repo, old_branch, new_branch).is_some() {
    return Ok(false);
  }
  if let Some(parent) = to.parent() {
    std::fs::create_dir_all(parent)?;
  }
  std::fs::rename(&from, &to)?;
  Ok(true)
}
