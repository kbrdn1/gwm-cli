//! Pin the contracts of the `gwm::cli` formatters extracted in the
//! Copilot-review follow-up on PR #154 (issue #31). These are pure
//! string-building helpers — no repo, no worktree, no I/O — so they
//! exist mostly to lock the human-facing output against accidental
//! drift, and to prove two specific Copilot review nits stay fixed:
//!
//! - "(would be deleted)" must not be appended when there is no
//!   branch to delete (a detached HEAD worktree); the dry-run plan
//!   must not promise a side effect that `worktree::remove` would
//!   never perform.
//! - Column widths in the prune plan must be computed in characters,
//!   not bytes, so worktree names or paths containing non-ASCII code
//!   points still align in a fixed-width terminal.

use gwm::cli::{format_prune_plan, format_remove_plan};
use gwm::worktree::PrunableEntry;
use std::path::{Path, PathBuf};

// --- format_remove_plan -----------------------------------------------------

#[test]
fn remove_plan_with_branch_and_no_delete_omits_deletion_marker() {
  let out = format_remove_plan(
    "feat-31-preview",
    Path::new("/tmp/wt/feat-31-preview"),
    Some("feat/#31-preview"),
    false,
  );

  assert!(out.contains("would remove"), "header missing: {out}");
  assert!(out.contains("name:   feat-31-preview"), "name row missing: {out}");
  assert!(
    out.contains("path:   /tmp/wt/feat-31-preview"),
    "path row missing: {out}"
  );
  assert!(out.contains("branch: feat/#31-preview"), "branch row missing: {out}");
  assert!(
    !out.contains("would be deleted"),
    "must not advertise branch deletion when --delete-branch is off: {out}"
  );
  assert!(
    !out.contains("no branch"),
    "must not advertise 'no branch' when a branch is resolved: {out}"
  );
}

#[test]
fn remove_plan_with_branch_and_delete_flag_marks_branch_deletion() {
  let out = format_remove_plan(
    "feat-31-preview",
    Path::new("/tmp/wt/feat-31-preview"),
    Some("feat/#31-preview"),
    true,
  );

  assert!(
    out.contains("branch: feat/#31-preview (would be deleted)"),
    "delete marker missing: {out}"
  );
}

#[test]
fn remove_plan_without_branch_and_no_delete_renders_dash_only() {
  // Detached HEAD worktree with no resolvable branch; the legacy
  // format prints `branch: -` and that pre-existing contract must be
  // preserved so scripts that grep the line keep working.
  let out = format_remove_plan("detached", Path::new("/tmp/wt/detached"), None, false);

  assert!(out.contains("branch: -"), "dash row missing: {out}");
  assert!(
    !out.contains("would be deleted"),
    "no delete marker without --delete-branch: {out}"
  );
  assert!(
    !out.contains("no branch"),
    "no 'no branch' rider without --delete-branch (avoid noise): {out}"
  );
}

#[test]
fn remove_plan_without_branch_but_with_delete_flag_does_not_claim_deletion() {
  // The Copilot review nit on PR #154: combining `--dry-run` with
  // `--delete-branch` on a detached HEAD worktree must NOT print
  // "(would be deleted)" because `worktree::remove` only drops a
  // branch when one is resolvable. The dry-run plan must mirror the
  // real behaviour exactly — print a clarifying rider so the user
  // sees why the destructive path would not delete anything.
  let out = format_remove_plan("detached", Path::new("/tmp/wt/detached"), None, true);

  assert!(
    !out.contains("would be deleted"),
    "must not claim a deletion that worktree::remove will not perform: {out}"
  );
  assert!(
    out.contains("no branch to delete"),
    "must clarify why --delete-branch is a no-op here: {out}"
  );
}

// --- format_prune_plan ------------------------------------------------------

#[test]
fn prune_plan_empty_emits_zero_count_marker() {
  let out = format_prune_plan(&[]);
  assert!(out.contains("0 worktree"), "stable zero-count marker missing: {out}");
}

#[test]
fn prune_plan_lists_entries_with_header_and_rows() {
  let entries = vec![
    PrunableEntry {
      name: "feat-1-alpha".into(),
      path: PathBuf::from("/tmp/wt/feat-1-alpha"),
      reason: "working dir missing".into(),
    },
    PrunableEntry {
      name: "feat-2-bravo".into(),
      path: PathBuf::from("/tmp/wt/feat-2-bravo"),
      reason: "working dir missing".into(),
    },
  ];

  let out = format_prune_plan(&entries);
  assert!(out.contains("would prune 2 worktree(s):"), "header missing: {out}");
  assert!(out.contains("feat-1-alpha"), "first row missing: {out}");
  assert!(out.contains("feat-2-bravo"), "second row missing: {out}");
  assert!(out.contains("working dir missing"), "reason missing: {out}");
}

#[test]
fn prune_plan_aligns_columns_in_unicode_chars_not_bytes() {
  // Pin the Copilot nit on PR #154: column widths must be derived
  // from character counts, not byte counts, otherwise a multi-byte
  // codepoint in a name or path shoves later columns to the left.
  //
  // "féat-α-1" is 8 chars, 11 bytes. "feat-b-22" is 9 chars, 9
  // bytes. With `.len()` width, the unicode row would have its
  // `reason` column drift right by 3 byte-spaces relative to the
  // ASCII row. With `.chars().count()`, both rows hit `(working
  // dir missing)` at the same character column.
  let entries = vec![
    PrunableEntry {
      name: "féat-α-1".into(),
      path: PathBuf::from("/tmp/wt/féat-α-1"),
      reason: "working dir missing".into(),
    },
    PrunableEntry {
      name: "feat-b-22".into(),
      path: PathBuf::from("/tmp/wt/feat-b-22"),
      reason: "working dir missing".into(),
    },
  ];

  let out = format_prune_plan(&entries);

  // Find the character offset of "(working dir missing)" on each
  // data row. They must match.
  let mut reason_offsets: Vec<usize> = Vec::new();
  for line in out.lines() {
    if let Some(idx) = line.find("(working dir missing)") {
      // Convert byte index to char index — that's what the user's
      // terminal column reads.
      let char_idx = line[..idx].chars().count();
      reason_offsets.push(char_idx);
    }
  }

  assert_eq!(
    reason_offsets.len(),
    2,
    "expected two reason cells, got {reason_offsets:?} (output: {out})"
  );
  assert_eq!(
    reason_offsets[0], reason_offsets[1],
    "reason column drifted across rows ({reason_offsets:?}) — width must be in chars, not bytes:\n{out}"
  );
}
