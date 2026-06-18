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

use crate::config::CleanConfig;
use crate::error::{GwmError, Result};
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

/// Validate a profile `dirs` entry: it must be a single worktree-relative
/// directory **name** — exactly one `Normal` path component.
///
/// Profile-supplied dirs are user-controlled and later fed to
/// [`scan_worktree`], which does `worktree.join(entry)` and a recursive
/// `dir_size` *before* the git safety gate runs. Anything other than a single
/// plain name can escape the worktree:
/// - absolute (`"/"`, a drive prefix) — `Path::join` resolves to the FS root;
/// - a `..` traversal — climbs out of the worktree;
/// - empty or a bare `.` / `./.` — resolves to the worktree root itself;
/// - a **nested** path (`a/b`) — an intermediate component (`a`) could be a
///   symlink that `scan_worktree` / `remove_dir_all` follow outside the tree
///   (the symlink skip only covers the artifact root, not its ancestors).
///
/// Restricting to a single name eliminates every one of these by construction
/// and matches the built-in [`default_patterns`] (which are trusted and bypass
/// this check). Nesting is a deliberate, additive post-1.0 extension (it would
/// need an explicit symlinked-ancestor guard). Anything else is a config error
/// (exit 1) at resolution time.
///
/// Each accepted entry is returned **normalized** to its bare component name
/// (`"target/"` and `"./target"` both become `"target"`) so syntactic aliases
/// of the same directory collapse under the exact-match [`dedup_dirs`] — a
/// raw-string dedup would otherwise keep `["target", "target/"]` and
/// double-scan / double-delete it.
fn normalized_profile_dirs(profile: &str, dirs: &[String]) -> Result<Vec<String>> {
  use std::path::Component;
  let mut out = Vec::with_capacity(dirs.len());
  for d in dirs {
    if d.is_empty() {
      return Err(GwmError::Config(format!(
        "clean: profile `{profile}` has an empty `dirs` entry — list single worktree-relative directory names"
      )));
    }
    let mut comps = Path::new(d).components().filter(|c| !matches!(c, Component::CurDir));
    let name = match (comps.next(), comps.next()) {
      // Exactly one plain directory name — the only safe shape.
      (Some(Component::Normal(n)), None) => n.to_string_lossy().into_owned(),
      (Some(Component::ParentDir), _) => {
        return Err(GwmError::Config(format!(
          "clean: profile `{profile}` dir `{d}` must not escape the worktree with `..`"
        )));
      }
      (Some(Component::RootDir | Component::Prefix(_)), _) => {
        return Err(GwmError::Config(format!(
          "clean: profile `{profile}` dir `{d}` must be relative to the worktree, not absolute"
        )));
      }
      // `.` / `./.` collapse to nothing — they resolve to the worktree root.
      (None, _) => {
        return Err(GwmError::Config(format!(
          "clean: profile `{profile}` dir `{d}` resolves to the worktree root — name a real subdirectory"
        )));
      }
      // Two or more components — a nested path like `a/b`.
      _ => {
        return Err(GwmError::Config(format!(
          "clean: profile `{profile}` dir `{d}` must be a single directory name (no `/`); nested paths are not supported"
        )));
      }
    };
    // Reject git pathspec magic. The safety gate feeds this name to
    // `git ls-files -- <name>` / `git check-ignore -- <name>`, which treat it
    // as a PATHSPEC, not a literal path: a glob char (`* ? [ ]`) or a leading
    // `:` (magic prefix) would make git match something other than the literal
    // directory `std::fs` deletes — e.g. `ls-files -- "foo[bar]"` misses a
    // force-tracked file inside a literal `foo[bar]/`, so the tracked-file
    // guard wrongly passes and `--yes` deletes tracked data. `check-ignore`
    // can't be made literal (it rejects `:(literal)` magic), so reject these
    // names up-front rather than silently mishandling them. A leading `-` is
    // fine — the `--` delimiter in the git calls already neutralises it.
    if name.starts_with(':') || name.contains(['*', '?', '[', ']']) {
      return Err(GwmError::Config(format!(
        "clean: profile `{profile}` dir `{d}` contains git pathspec metacharacters (`* ? [ ]` or a leading `:`) — name a literal directory"
      )));
    }
    out.push(name);
  }
  Ok(out)
}

/// Validate a `[clean.profiles.<name>]` entry's `dirs` without resolving them
/// — same rules as [`normalized_profile_dirs`], surfaced for the config
/// validation path so `gwm config validate` / `gwm doctor` reject what
/// `gwm clean` would (issue #324 review).
pub fn validate_clean_profile_dirs(profile: &str, dirs: &[String]) -> Result<()> {
  normalized_profile_dirs(profile, dirs).map(|_| ())
}

/// Drop exact duplicate entries (declared order kept), so a directory listed
/// twice isn't scanned and reclaimed twice. Inputs come from
/// [`normalized_profile_dirs`], so syntactic aliases (`target` vs `target/`)
/// have already been folded to the same string — exact equality is the only
/// overlap left.
fn dedup_dirs(dirs: &[String]) -> Vec<String> {
  let mut kept: Vec<String> = Vec::new();
  for d in dirs {
    if !kept.contains(d) {
      kept.push(d.clone());
    }
  }
  kept
}

/// Resolve the directory set `gwm clean` should scan and reclaim (issue #324).
///
/// - `--profile <name>` selects `[clean.profiles.<name>].dirs`, a **complete**
///   set that replaces the built-ins. A name absent from `[clean.profiles]`
///   is an error (exit 1).
/// - **No** `--profile` uses `[clean.profiles.default].dirs` when that profile
///   exists, else falls back to the built-in [`default_patterns`].
///
/// Profile-supplied dirs are validated and normalized to single worktree-
/// relative names (absolute, `..`, `.`/root, nested, or empty → exit 1), then
/// exact-deduped. Whatever set is returned, the caller still runs every
/// directory through the safety gate (git-ignored + no tracked files + skip
/// symlinks) before delete.
pub fn resolve_clean_dirs(profile: Option<&str>, cfg: &CleanConfig) -> Result<Vec<String>> {
  match profile {
    Some(name) => {
      let p = cfg
        .profiles
        .get(name)
        .ok_or_else(|| GwmError::Config(format!("clean: no profile named `{name}` in [clean.profiles]")))?;
      Ok(dedup_dirs(&normalized_profile_dirs(name, &p.dirs)?))
    }
    None => match cfg.profiles.get("default") {
      Some(p) => Ok(dedup_dirs(&normalized_profile_dirs("default", &p.dirs)?)),
      None => Ok(default_patterns()),
    },
  }
}

/// Sum the logical length of every regular file under `dir`, recursively.
///
/// Symlinks are not followed: the entry type is read via `DirEntry::file_type`
/// (which, unlike `DirEntry::metadata`, does *not* traverse the link), so a
/// symlink is skipped outright rather than recursed into or counted. Its
/// target may live outside the worktree — or form a loop — and must not be
/// attributed to it (nor, on delete, reached through it).
fn dir_size(dir: &Path) -> u64 {
  let mut total = 0u64;
  let Ok(entries) = std::fs::read_dir(dir) else {
    return 0;
  };
  for entry in entries.flatten() {
    let Ok(ft) = entry.file_type() else {
      continue;
    };
    if ft.is_symlink() {
      continue;
    }
    if ft.is_dir() {
      total = total.saturating_add(dir_size(&entry.path()));
    } else if ft.is_file() {
      if let Ok(meta) = entry.metadata() {
        total = total.saturating_add(meta.len());
      }
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
    // Skip a symlinked artifact *root*: `Path::is_dir` follows the link, so
    // `dir_size` would walk (and `clean --yes` could reach) a tree outside
    // the worktree. `symlink_metadata` reports on the link itself.
    let Ok(meta) = std::fs::symlink_metadata(&candidate) else {
      continue;
    };
    if meta.file_type().is_symlink() {
      continue;
    }
    if meta.is_dir() {
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
