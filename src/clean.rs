//! `gwm clean` (issue #313): report and reclaim heavy build artifacts across
//! worktrees.
//!
//! A worktree fleet accumulates gigabytes of regenerable build output
//! (`target/`, `node_modules/`, `dist/`, `build/`). This module scans each
//! worktree for those directories, sizes them, renders a report, and — only
//! when the caller opts in — deletes them. Everything here is pure path
//! plumbing (no git repo needed), so it is unit-tested directly.
//!
//! Deliberately **not** journaled into `gwm history` / `gwm undo` (#29): the
//! artifacts are regenerable, so a resurrection entry would be meaningless.

use crate::error::Result;
use std::path::{Path, PathBuf};

/// One reclaimable artifact directory inside a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
  /// Directory name relative to the worktree root (e.g. `target`).
  pub rel: String,
  /// Total logical size of the files underneath it.
  pub bytes: u64,
}

/// The reclaimable artifacts found in a single worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeReclaim {
  pub name: String,
  pub path: PathBuf,
  pub artifacts: Vec<Artifact>,
  pub total_bytes: u64,
}

/// The build-artifact directory names cleaned by default.
///
/// Hardcoded for the MVP; a `[clean]` config block to tune the set per repo
/// is a deliberate follow-up (issue #313).
pub fn default_patterns() -> Vec<String> {
  ["target", "node_modules", "dist", "build"]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Sum the logical length of every regular file under `dir`, recursively.
///
/// Symlinks are not followed: `DirEntry::metadata()` reports on the link
/// itself, so a symlink is neither recursed into nor counted — its target may
/// live outside the worktree and must not be attributed to it (nor deleted).
fn dir_size(dir: &Path) -> u64 {
  let mut total = 0u64;
  let Ok(entries) = std::fs::read_dir(dir) else {
    return 0;
  };
  for entry in entries.flatten() {
    let Ok(meta) = entry.metadata() else {
      continue;
    };
    if meta.is_dir() {
      total = total.saturating_add(dir_size(&entry.path()));
    } else if meta.is_file() {
      total = total.saturating_add(meta.len());
    }
  }
  total
}

/// Scan one worktree at `path` for each pattern directory, sizing the ones
/// that exist. Returns a [`WorktreeReclaim`] (possibly with no artifacts when
/// the worktree is already clean).
pub fn scan_worktree(name: &str, path: &Path, patterns: &[String]) -> WorktreeReclaim {
  let mut artifacts = Vec::new();
  let mut total = 0u64;
  for pat in patterns {
    let candidate = path.join(pat);
    if candidate.is_dir() {
      let bytes = dir_size(&candidate);
      total = total.saturating_add(bytes);
      artifacts.push(Artifact {
        rel: pat.clone(),
        bytes,
      });
    }
  }
  WorktreeReclaim {
    name: name.to_string(),
    path: path.to_path_buf(),
    artifacts,
    total_bytes: total,
  }
}

/// Delete every scanned artifact directory of `reclaim`, returning the number
/// of bytes freed (the sum of the scanned sizes). Only the directories named
/// in `reclaim.artifacts` are touched — anything else in the worktree is left
/// untouched.
pub fn delete_reclaim(reclaim: &WorktreeReclaim) -> Result<u64> {
  let mut freed = 0u64;
  for a in &reclaim.artifacts {
    let target = reclaim.path.join(&a.rel);
    std::fs::remove_dir_all(&target)?;
    freed = freed.saturating_add(a.bytes);
  }
  Ok(freed)
}

/// Format `bytes` as a human-readable size with a binary unit (`B`, `KiB`,
/// `MiB`, `GiB`), one decimal place above 1 KiB.
pub fn human_size(bytes: u64) -> String {
  const KIB: u64 = 1024;
  const MIB: u64 = 1024 * KIB;
  const GIB: u64 = 1024 * MIB;
  if bytes >= GIB {
    format!("{:.1} GiB", bytes as f64 / GIB as f64)
  } else if bytes >= MIB {
    format!("{:.1} MiB", bytes as f64 / MIB as f64)
  } else if bytes >= KIB {
    format!("{:.1} KiB", bytes as f64 / KIB as f64)
  } else {
    format!("{} B", bytes)
  }
}

/// Render the per-worktree report plus a grand total. Worktrees with no
/// reclaimable artifacts are omitted from the body (they would be noise), but
/// still count toward the total — which is zero when everything is clean.
pub fn format_report(reclaims: &[WorktreeReclaim]) -> String {
  let mut out = String::new();
  let grand: u64 = reclaims.iter().map(|r| r.total_bytes).sum();
  for r in reclaims {
    if r.artifacts.is_empty() {
      continue;
    }
    out.push_str(&format!("{} ({})\n", r.name, human_size(r.total_bytes)));
    for a in &r.artifacts {
      out.push_str(&format!("  {:<14} {}\n", a.rel, human_size(a.bytes)));
    }
  }
  out.push_str(&format!("\nTotal reclaimable: {}\n", human_size(grand)));
  out
}
