//! Unit tests for `gwm clean` (issue #313) — the pure disk-reclaim layer.
//!
//! The scan / size / delete / report functions take plain paths and need no
//! git repo, so they are fully testable against a `tempfile::TempDir`. Sizes
//! are summed from the logical length of regular files written by the test,
//! which is deterministic across filesystems (CLAUDE.md env-independence).

use gwm::clean::{
  default_patterns, delete_reclaim, format_report, human_size, resolve_clean_dirs, scan_worktree, WorktreeReclaim,
};
use gwm::config::{CleanConfig, CleanProfile};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Create `dir/<rel>` and drop one `blob` of `bytes` length inside it.
fn make_artifact(root: &Path, rel: &str, bytes: usize) {
  let d = root.join(rel);
  fs::create_dir_all(&d).unwrap();
  fs::write(d.join("blob.bin"), vec![0u8; bytes]).unwrap();
}

/// Build a [`CleanConfig`] with the given `[clean.profiles.*]` entries.
fn clean_cfg(profiles: &[(&str, &[&str])]) -> CleanConfig {
  let mut map = BTreeMap::new();
  for (name, dirs) in profiles {
    map.insert(
      (*name).to_string(),
      CleanProfile {
        dirs: dirs.iter().map(|s| s.to_string()).collect(),
      },
    );
  }
  CleanConfig { profiles: map }
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

// --- profile resolution (issue #324) ----------------------------------------

#[test]
fn resolve_dirs_falls_back_to_builtins_without_profile_or_default() {
  // No `--profile` and no `[clean.profiles.default]` ⇒ the built-in set.
  let cfg = clean_cfg(&[]);
  let dirs = resolve_clean_dirs(None, &cfg).expect("builtin fallback");
  assert_eq!(dirs, default_patterns());
}

#[test]
fn resolve_dirs_uses_the_default_profile_when_present() {
  // `gwm clean` without `--profile` prefers `[clean.profiles.default]`.
  let cfg = clean_cfg(&[("default", &["target", "node_modules", "coverage", ".turbo"])]);
  let dirs = resolve_clean_dirs(None, &cfg).expect("default profile");
  assert_eq!(dirs, vec!["target", "node_modules", "coverage", ".turbo"]);
  assert_ne!(dirs, default_patterns(), "the default profile overrides the built-ins");
}

#[test]
fn resolve_dirs_uses_a_named_profile() {
  let cfg = clean_cfg(&[("deep", &["target", ".cache", ".venv"])]);
  let dirs = resolve_clean_dirs(Some("deep"), &cfg).expect("named profile");
  assert_eq!(dirs, vec!["target", ".cache", ".venv"]);
}

#[test]
fn resolve_dirs_named_profile_is_a_complete_set_not_additive() {
  // A profile's `dirs` REPLACES the built-ins — `build`/`dist` are absent
  // from the resolved set unless the profile lists them.
  let cfg = clean_cfg(&[("rust", &["target"])]);
  let dirs = resolve_clean_dirs(Some("rust"), &cfg).expect("named profile");
  assert_eq!(dirs, vec!["target"]);
  assert!(!dirs.contains(&"node_modules".to_string()), "built-ins are not added");
  assert!(!dirs.contains(&"build".to_string()), "built-ins are not added");
}

#[test]
fn resolve_dirs_rejects_an_unknown_profile() {
  let cfg = clean_cfg(&[("deep", &["target"])]);
  let err = resolve_clean_dirs(Some("nope"), &cfg).expect_err("unknown profile must error");
  assert!(
    err.to_string().contains("nope") && err.to_string().contains("profile"),
    "error should name the missing profile: {err}"
  );
}

#[test]
fn resolve_dirs_rejects_an_absolute_profile_dir() {
  // `worktree.join("/")` resolves to the FS root — a dry-run scan would walk
  // the whole filesystem. Reject absolute entries at resolution (exit 1).
  let cfg = clean_cfg(&[("evil", &["/"])]);
  let err = resolve_clean_dirs(Some("evil"), &cfg).expect_err("absolute dir must error");
  assert!(
    err.to_string().contains("absolute"),
    "error should flag the absolute path: {err}"
  );
}

#[test]
fn resolve_dirs_rejects_a_parent_traversal_profile_dir() {
  let cfg = clean_cfg(&[("evil", &["../sibling"])]);
  let err = resolve_clean_dirs(Some("evil"), &cfg).expect_err("`..` dir must error");
  assert!(err.to_string().contains(".."), "error should flag the traversal: {err}");
}

#[test]
fn resolve_dirs_rejects_an_empty_profile_dir() {
  let cfg = clean_cfg(&[("evil", &[""])]);
  let err = resolve_clean_dirs(Some("evil"), &cfg).expect_err("empty dir must error");
  assert!(
    err.to_string().contains("empty"),
    "error should flag the empty entry: {err}"
  );
}

#[test]
fn resolve_dirs_validates_the_default_profile_too() {
  // The escape guard also covers the implicit `default` profile (no --profile).
  let cfg = clean_cfg(&[("default", &[".."])]);
  let err = resolve_clean_dirs(None, &cfg).expect_err("default profile is validated");
  assert!(err.to_string().contains(".."), "error should flag the traversal: {err}");
}

#[test]
fn resolve_dirs_allows_a_nested_relative_subpath() {
  // A relative subpath that stays inside the worktree (no `..`) is fine.
  let cfg = clean_cfg(&[("nested", &["packages/app/node_modules"])]);
  let dirs = resolve_clean_dirs(Some("nested"), &cfg).expect("nested relative path is allowed");
  assert_eq!(dirs, vec!["packages/app/node_modules"]);
}
