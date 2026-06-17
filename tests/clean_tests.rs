//! Unit tests for `gwm clean` (issue #313) — the pure disk-reclaim layer.
//!
//! The scan / size / delete / report functions take plain paths and need no
//! git repo, so they are fully testable against a `tempfile::TempDir`. Sizes
//! are summed from the logical length of regular files written by the test,
//! which is deterministic across filesystems (CLAUDE.md env-independence).

use gwm::clean::{default_patterns, delete_reclaim, format_report, human_size, scan_worktree, WorktreeReclaim};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Create `dir/<rel>` and drop one `blob` of `bytes` length inside it.
fn make_artifact(root: &Path, rel: &str, bytes: usize) {
  let d = root.join(rel);
  fs::create_dir_all(&d).unwrap();
  fs::write(d.join("blob.bin"), vec![0u8; bytes]).unwrap();
}

#[test]
fn human_size_formats_units() {
  assert_eq!(human_size(0), "0 B");
  assert_eq!(human_size(512), "512 B");
  assert_eq!(human_size(1024), "1.0 KiB");
  assert_eq!(human_size(1536), "1.5 KiB");
  assert_eq!(human_size(1024 * 1024), "1.0 MiB");
  assert_eq!(human_size(1024 * 1024 * 1024), "1.0 GiB");
}

#[test]
fn default_patterns_cover_the_common_build_dirs() {
  let p = default_patterns();
  for expected in ["target", "node_modules", "dist", "build"] {
    assert!(
      p.iter().any(|d| d == expected),
      "default patterns should include {expected}: {p:?}"
    );
  }
}

#[test]
fn scan_finds_default_artifact_dirs_and_sums_sizes() {
  let dir = TempDir::new().unwrap();
  let wt = dir.path();
  make_artifact(wt, "target", 2048);
  make_artifact(wt, "node_modules", 1024);
  // Not an artifact: must be ignored and excluded from the total.
  make_artifact(wt, "src", 4096);

  let r = scan_worktree("feat-1", wt, &default_patterns());

  assert_eq!(r.name, "feat-1");
  let mut found: Vec<&str> = r.artifacts.iter().map(|a| a.rel.as_str()).collect();
  found.sort_unstable();
  assert_eq!(found, vec!["node_modules", "target"]);
  assert_eq!(r.total_bytes, 2048 + 1024);
}

#[test]
fn scan_returns_empty_when_no_artifacts_present() {
  let dir = TempDir::new().unwrap();
  make_artifact(dir.path(), "src", 4096);

  let r = scan_worktree("clean-wt", dir.path(), &default_patterns());

  assert!(r.artifacts.is_empty());
  assert_eq!(r.total_bytes, 0);
}

#[cfg(unix)]
#[test]
fn scan_does_not_follow_symlinks_inside_artifacts() {
  use std::os::unix::fs::symlink;
  let dir = TempDir::new().unwrap();
  let wt = dir.path();
  // Heavy content that a followed symlink would wrongly pull into the total.
  let outside = TempDir::new().unwrap();
  make_artifact(outside.path(), "heavy", 100_000);

  make_artifact(wt, "target", 100);
  symlink(outside.path(), wt.join("target").join("link_out")).unwrap();

  let r = scan_worktree("wt", wt, &default_patterns());

  assert_eq!(
    r.total_bytes, 100,
    "a symlink inside target/ must not be followed or counted (only the real 100-byte file)"
  );
}

#[cfg(unix)]
#[test]
fn scan_skips_a_symlinked_artifact_root() {
  use std::os::unix::fs::symlink;
  let dir = TempDir::new().unwrap();
  let wt = dir.path();
  // An external dir the symlinked root would wrongly pull in if followed.
  let outside = TempDir::new().unwrap();
  make_artifact(outside.path(), "heavy", 100_000);
  // `target` itself is a symlink to the external dir.
  symlink(outside.path(), wt.join("target")).unwrap();

  let r = scan_worktree("wt", wt, &default_patterns());

  assert!(
    r.artifacts.is_empty(),
    "a symlinked artifact root must not be scanned: {:?}",
    r.artifacts
  );
  assert_eq!(r.total_bytes, 0);
}

#[test]
fn scan_counts_nested_files_recursively() {
  let dir = TempDir::new().unwrap();
  let wt = dir.path();
  make_artifact(wt, "target", 100);
  make_artifact(wt, "target/debug/deps", 250);

  let r = scan_worktree("nested", wt, &default_patterns());

  assert_eq!(r.total_bytes, 350, "size should sum files at every depth under target/");
}

#[test]
fn delete_reclaim_removes_artifact_dirs_only() {
  let dir = TempDir::new().unwrap();
  let wt = dir.path();
  make_artifact(wt, "target", 2048);
  make_artifact(wt, "src", 4096);

  let r = scan_worktree("feat-1", wt, &default_patterns());
  let freed = delete_reclaim(&r).unwrap();

  assert_eq!(freed, 2048);
  assert!(!wt.join("target").exists(), "target/ should be deleted");
  assert!(wt.join("src").exists(), "src/ must be preserved");
}

#[test]
fn format_report_lists_each_worktree_and_a_grand_total() {
  let reclaims = vec![
    WorktreeReclaim {
      name: "feat-1".into(),
      path: "/x/feat-1".into(),
      artifacts: vec![gwm::clean::Artifact {
        rel: "target".into(),
        bytes: 1024 * 1024,
      }],
      total_bytes: 1024 * 1024,
    },
    WorktreeReclaim {
      name: "fix-2".into(),
      path: "/x/fix-2".into(),
      artifacts: vec![],
      total_bytes: 0,
    },
  ];

  let report = format_report(&reclaims);

  assert!(report.contains("feat-1"), "report should name worktrees with artifacts");
  assert!(report.contains("target"), "report should name the artifact dir");
  assert!(report.contains("1.0 MiB"), "report should render human sizes");
  // Grand total of reclaimable space across all worktrees.
  assert!(report.contains("1.0 MiB"), "report should carry a grand total");
}
