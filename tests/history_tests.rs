//! Tests for the operation journal (`src/history.rs`). Issue #29.
//!
//! The journal records destructive operations (`gwm remove`) so they
//! can be undone via `gwm undo`. The contract is intentionally narrow:
//!
//! - Append a new entry to `$XDG_DATA_HOME/gwm/history.toml` (or
//!   `~/.local/share/gwm/history.toml` fallback). Override via
//!   `GWM_HISTORY_FILE` env var (the testability hook).
//! - Read the most-recent-N entries.
//! - Rotate at a cap of 100 entries — drop the oldest on append.
//! - Per-repo filtering: each entry records the `repo_root` of the
//!   worktree it operated on, so `gwm undo` only resurfaces the
//!   current repo's history.
//!
//! These tests pin the IO contract before any production code lands —
//! TDD-strict per CLAUDE.md.

use chrono::{Duration, Utc};
use gwm::history::{self, OpEntry, OpKind};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Build a synthetic [`OpEntry`] for the given suffix. Timestamps
/// default to `now()`; callers that need ordering override after the
/// fact.
fn entry(suffix: &str, repo_root: &Path) -> OpEntry {
  OpEntry {
    ts: Utc::now(),
    kind: OpKind::Remove,
    worktree: format!("feat-{}-foo", suffix),
    branch: Some(format!("feat/#{}-foo", suffix)),
    branch_oid: Some("a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".into()),
    path: PathBuf::from(format!("/tmp/cc-worktree/feat-{}-foo", suffix)),
    deleted_branch: false,
    repo_root: repo_root.to_path_buf(),
    undone: false,
  }
}

#[test]
fn load_returns_empty_on_missing_file() {
  // Fresh install — no `~/.local/share/gwm/history.toml` exists.
  // `load` must return an empty journal, NOT propagate a NotFound
  // error: the first ever invocation of `gwm history` must succeed
  // cleanly with "0 ops recorded".
  let tmp = TempDir::new().unwrap();
  let path = tmp.path().join("history.toml");
  let journal = history::Journal::load(&path).unwrap();
  assert!(journal.entries().is_empty());
}

#[test]
fn append_then_load_roundtrips_entry() {
  // Append an entry, persist to disk, reload. The reloaded journal
  // must contain the exact same entry — TOML round-trip is the IO
  // backbone of the whole feature.
  let tmp = TempDir::new().unwrap();
  let path = tmp.path().join("history.toml");
  let repo = tmp.path().join("repo");

  let mut journal = history::Journal::load(&path).unwrap();
  let original = entry("99", &repo);
  journal.append(original.clone());
  journal.save(&path).unwrap();

  let reloaded = history::Journal::load(&path).unwrap();
  let entries = reloaded.entries();
  assert_eq!(entries.len(), 1);
  assert_eq!(entries[0].worktree, original.worktree);
  assert_eq!(entries[0].branch, original.branch);
  assert_eq!(entries[0].branch_oid, original.branch_oid);
  assert_eq!(entries[0].path, original.path);
  assert_eq!(entries[0].deleted_branch, original.deleted_branch);
  assert_eq!(entries[0].repo_root, original.repo_root);
}

#[test]
fn rotation_caps_at_one_hundred_entries() {
  // Cap is 100 entries TOTAL across all repos. On append #101, drop
  // the oldest. This pins the rotation contract — the journal must
  // not grow unbounded.
  let tmp = TempDir::new().unwrap();
  let path = tmp.path().join("history.toml");
  let repo = tmp.path().join("repo");

  let mut journal = history::Journal::default();
  // Seed 101 entries with monotonic timestamps (oldest first).
  let base = Utc::now() - Duration::days(200);
  for i in 0..101 {
    let mut e = entry(&format!("{}", i), &repo);
    e.ts = base + Duration::minutes(i as i64);
    journal.append(e);
  }
  journal.save(&path).unwrap();

  let reloaded = history::Journal::load(&path).unwrap();
  let entries = reloaded.entries();
  assert_eq!(entries.len(), 100, "journal must cap at 100 entries");
  // The dropped entry is the oldest one (worktree feat-0-foo).
  assert!(
    !entries.iter().any(|e| e.worktree == "feat-0-foo"),
    "oldest entry must be dropped on rotation"
  );
  // The newest entry survives.
  assert!(entries.iter().any(|e| e.worktree == "feat-100-foo"));
}

#[test]
fn entries_for_repo_filters_by_repo_root() {
  // The journal is global (one file across all repos). `entries_for_repo`
  // must return only the ops whose `repo_root` matches verbatim — that
  // is the per-repo separation `gwm undo` and `gwm history` rely on so
  // an `undo` in repo A cannot resurrect a worktree from repo B.
  let tmp = TempDir::new().unwrap();
  let path = tmp.path().join("history.toml");
  let repo_a = tmp.path().join("repo-a");
  let repo_b = tmp.path().join("repo-b");

  let mut journal = history::Journal::default();
  journal.append(entry("1", &repo_a));
  journal.append(entry("2", &repo_b));
  journal.append(entry("3", &repo_a));
  journal.save(&path).unwrap();

  let reloaded = history::Journal::load(&path).unwrap();
  let only_a: Vec<&OpEntry> = reloaded.entries_for_repo(&repo_a).collect();
  assert_eq!(only_a.len(), 2);
  assert!(only_a.iter().all(|e| e.repo_root == repo_a));

  let only_b: Vec<&OpEntry> = reloaded.entries_for_repo(&repo_b).collect();
  assert_eq!(only_b.len(), 1);
  assert_eq!(only_b[0].repo_root, repo_b);
}

#[test]
fn last_for_repo_returns_most_recent_entry() {
  // `gwm undo` pulls the *most recent* op for the current repo. Pin
  // the ordering contract: entries are listed newest-first via
  // `last_for_repo` regardless of insertion order.
  let tmp = TempDir::new().unwrap();
  let path = tmp.path().join("history.toml");
  let repo = tmp.path().join("repo");

  let mut journal = history::Journal::default();
  let base = Utc::now();
  let mut e1 = entry("old", &repo);
  e1.ts = base - Duration::hours(2);
  let mut e2 = entry("new", &repo);
  e2.ts = base - Duration::minutes(1);
  // Insert in non-monotonic order to make sure the lookup is
  // timestamp-driven, not insertion-driven.
  journal.append(e2.clone());
  journal.append(e1);
  journal.save(&path).unwrap();

  let reloaded = history::Journal::load(&path).unwrap();
  let last = reloaded.last_for_repo(&repo).expect("must find latest op");
  assert_eq!(last.worktree, "feat-new-foo");
}

#[test]
fn last_for_repo_returns_none_for_unknown_repo() {
  // If no ops have been recorded for the current repo, `last_for_repo`
  // returns `None` — `gwm undo` then prints "nothing to undo" rather
  // than crashing.
  let tmp = TempDir::new().unwrap();
  let path = tmp.path().join("history.toml");
  let repo_a = tmp.path().join("repo-a");
  let repo_b = tmp.path().join("repo-b");

  let mut journal = history::Journal::default();
  journal.append(entry("1", &repo_a));
  journal.save(&path).unwrap();

  let reloaded = history::Journal::load(&path).unwrap();
  assert!(reloaded.last_for_repo(&repo_b).is_none());
}

#[test]
fn pop_last_for_repo_removes_and_returns_entry() {
  // After a successful undo, `gwm undo` needs to drop the entry so a
  // second `undo` resurfaces the previous op instead of replaying the
  // same one. `pop_last_for_repo` must atomically return-and-remove
  // the newest entry for the repo.
  let tmp = TempDir::new().unwrap();
  let path = tmp.path().join("history.toml");
  let repo = tmp.path().join("repo");

  let mut journal = history::Journal::default();
  journal.append(entry("1", &repo));
  journal.append(entry("2", &repo));
  journal.save(&path).unwrap();

  let mut reloaded = history::Journal::load(&path).unwrap();
  let popped = reloaded.pop_last_for_repo(&repo).expect("must pop");
  assert_eq!(popped.worktree, "feat-2-foo");
  assert_eq!(reloaded.entries_for_repo(&repo).count(), 1);
}

#[test]
fn default_path_resolution_order() {
  // Combined into one test because `std::env::set_var` is process-global
  // and parallel tests would race. Cargo runs each `#[test]` on a
  // separate thread but in the same process, so two env-mutating tests
  // would flake without serialisation. Folding the two cases into one
  // sequential test makes the resolution-order contract testable without
  // pulling in `serial_test` for one assertion.
  //
  // Resolution order under test:
  //   1. `GWM_HISTORY_FILE` (testability hook + power-user override).
  //   2. `XDG_DATA_HOME/gwm/history.toml`.
  //
  // The third fallback (`dirs::data_dir()`) depends on the platform
  // home dir lookup and isn't safe to override mid-test — we trust the
  // `dirs` crate for that one.

  // Snapshot whatever the runner's env looked like so we restore it
  // verbatim afterwards (CI runners can have either, neither, or both
  // set; PR #43 hit a CI flake from a similar oversight in trust tests).
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_xdg = std::env::var("XDG_DATA_HOME").ok();

  // (1) `GWM_HISTORY_FILE` wins when set and non-empty.
  std::env::set_var("GWM_HISTORY_FILE", "/tmp/explicit-gwm-history.toml");
  let path = history::default_journal_path().unwrap();
  assert_eq!(path, PathBuf::from("/tmp/explicit-gwm-history.toml"));

  // (2) Falls back to `$XDG_DATA_HOME/gwm/history.toml` when the override
  // is unset.
  let tmp = TempDir::new().unwrap();
  std::env::remove_var("GWM_HISTORY_FILE");
  std::env::set_var("XDG_DATA_HOME", tmp.path());
  let path = history::default_journal_path().unwrap();
  assert_eq!(path, tmp.path().join("gwm").join("history.toml"));

  // Restore the original env so unrelated tests in the same process
  // aren't affected.
  match prev_history {
    Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
    None => std::env::remove_var("GWM_HISTORY_FILE"),
  }
  match prev_xdg {
    Some(v) => std::env::set_var("XDG_DATA_HOME", v),
    None => std::env::remove_var("XDG_DATA_HOME"),
  }
}
