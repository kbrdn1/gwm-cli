//! Operation journal backing `gwm undo` + `gwm history` (issue #29).
//!
//! Records destructive operations (today: `gwm remove`) to a single
//! per-user TOML file so the user can recover from a stray keystroke
//! without `git reflog` archaeology. Each entry carries enough state
//! to recreate the branch (its OID at deletion time) and re-add the
//! worktree (its path + name) — the inverse of [`worktree::remove`].
//!
//! ## File location
//!
//! Resolution order (matches the `trust::default_ledger_path` shape
//! so the two user-facing files live in symmetrical places):
//!
//!   1. `$GWM_HISTORY_FILE` if set and non-empty. The testability hook
//!      and power-user override.
//!   2. `$XDG_DATA_HOME/gwm/history.toml` if `$XDG_DATA_HOME` is set.
//!   3. `dirs::data_dir()/gwm/history.toml` (XDG on Linux,
//!      `Application Support` on macOS, `%LOCALAPPDATA%` on Windows).
//!
//! ## Cap + rotation
//!
//! Hard-cap at [`MAX_ENTRIES`] (currently 100). On [`Journal::append`]
//! we drop the OLDEST entry by timestamp when we'd overflow. Keeping
//! the cap global (not per-repo) means a power user juggling 30 repos
//! still has a bounded ledger; the per-repo separation is enforced at
//! READ time via [`Journal::entries_for_repo`], not at write time.
//!
//! ## Per-repo separation
//!
//! Every entry records its `repo_root` (the canonicalised main workdir
//! of the repo the op happened in). [`Journal::last_for_repo`] and
//! [`Journal::pop_last_for_repo`] filter on this so `gwm undo` in repo
//! A cannot resurrect a worktree from repo B.

use crate::error::{GwmError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::Builder;

/// Maximum number of entries kept across the whole journal. Append
/// past this cap drops the oldest entry — keeps the file bounded over
/// the lifetime of a heavy gwm user without per-repo bookkeeping.
pub const MAX_ENTRIES: usize = 100;

/// Discriminator for the kind of operation recorded. Today only
/// `Remove` is wired up — `Create` is reserved for a future extension
/// that hooks `gwm create` into the journal so `gwm undo` can also
/// roll back accidental worktree creations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpKind {
  /// `gwm remove` (with or without `--delete-branch`).
  Remove,
  /// Reserved for `gwm create` instrumentation — not wired today.
  Create,
}

impl OpKind {
  pub fn as_str(self) -> &'static str {
    match self {
      OpKind::Remove => "remove",
      OpKind::Create => "create",
    }
  }
}

/// One recorded destructive operation.
///
/// `branch_oid` is the SHA the branch ref pointed at *at deletion
/// time* — that is the resurrection anchor for `gwm undo`. `git`'s
/// object DB keeps the commit alive until `git gc` runs (defaults to
/// at least 30 days for unreachable objects), so the OID stays
/// resolvable for the whole window where undo makes sense.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpEntry {
  /// RFC3339 timestamp of when the op happened. Used to sort entries
  /// newest-first when surfacing to the user.
  pub ts: DateTime<Utc>,
  pub kind: OpKind,
  /// Name of the worktree (the `<dirname>` portion, not the full path).
  pub worktree: String,
  /// Branch name. `None` for detached-HEAD worktrees.
  pub branch: Option<String>,
  /// OID the branch ref pointed at when the op started. Used by
  /// `gwm undo` to recreate `refs/heads/<branch>` at the exact tip
  /// that was deleted.
  pub branch_oid: Option<String>,
  /// Worktree directory on disk (the `path` field of `WorktreeInfo`).
  pub path: PathBuf,
  /// Whether the destructive call also deleted the branch
  /// (`--delete-branch`). Drives whether `gwm undo` has to recreate
  /// the branch or just re-add the worktree.
  pub deleted_branch: bool,
  /// Canonicalised path of the main repo workdir this op happened in.
  /// Filters `last_for_repo` / `entries_for_repo` so a global journal
  /// doesn't cross-pollute repos.
  pub repo_root: PathBuf,
  /// Set to `true` after `gwm undo` consumed this entry — but `undo`
  /// removes the entry on success, so a `true` here means the user
  /// manually edited the journal. Surfaced in `gwm history` for
  /// completeness.
  #[serde(default)]
  pub undone: bool,
}

/// The on-disk journal. Plain TOML so users can `cat`/`less` it and
/// audit what their previous gwm sessions did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Journal {
  #[serde(default, rename = "op")]
  entries: Vec<OpEntry>,
}

impl Journal {
  /// Load the journal from `path`. A missing file is NOT an error —
  /// treated as an empty journal. Matches the trust-ledger contract
  /// in `src/trust.rs`.
  pub fn load(path: &Path) -> Result<Self> {
    match fs::read_to_string(path) {
      Ok(raw) => {
        let journal: Journal = toml::from_str(&raw)?;
        Ok(journal)
      }
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
      Err(e) => Err(e.into()),
    }
  }

  /// Append `entry` to the journal, enforcing the [`MAX_ENTRIES`] cap.
  /// When the cap is exceeded, the OLDEST entry (by timestamp) is
  /// dropped — the rotation policy.
  pub fn append(&mut self, entry: OpEntry) {
    self.entries.push(entry);
    if self.entries.len() > MAX_ENTRIES {
      // Drop the entry with the smallest timestamp. We scan rather
      // than maintain sorted order on push so the file remains in
      // insertion order on disk (easier to diff manually).
      let oldest_idx = self
        .entries
        .iter()
        .enumerate()
        .min_by_key(|(_, e)| e.ts)
        .map(|(i, _)| i);
      if let Some(i) = oldest_idx {
        self.entries.remove(i);
      }
    }
  }

  /// Persist the journal atomically (same primitive as the trust
  /// ledger): write a randomised tmp file in the same dir, then
  /// rename(2). Parent dirs are created on demand.
  pub fn save(&self, path: &Path) -> Result<()> {
    let parent = match path.parent() {
      Some(p) if !p.as_os_str().is_empty() => {
        fs::create_dir_all(p)?;
        p.to_path_buf()
      }
      _ => PathBuf::from("."),
    };
    let body = toml::to_string_pretty(self)?;
    let mut tmp = Builder::new()
      .prefix("gwm-history-")
      .suffix(".tmp")
      .tempfile_in(&parent)?;
    tmp.write_all(body.as_bytes())?;
    tmp.persist(path).map_err(|e| GwmError::Io(e.error))?;
    Ok(())
  }

  /// Read-only view of all entries (insertion order).
  pub fn entries(&self) -> &[OpEntry] {
    &self.entries
  }

  /// Iterator over entries whose `repo_root` matches `repo_root`
  /// verbatim. Used by `gwm history` (which filters by the current
  /// repo) and `gwm undo` (via [`Self::last_for_repo`]).
  pub fn entries_for_repo<'a>(&'a self, repo_root: &'a Path) -> impl Iterator<Item = &'a OpEntry> + 'a {
    self.entries.iter().filter(move |e| e.repo_root == repo_root)
  }

  /// Most-recent entry (by timestamp) for `repo_root`, or `None` if
  /// no op has been recorded for that repo.
  pub fn last_for_repo<'a>(&'a self, repo_root: &'a Path) -> Option<&'a OpEntry> {
    self.entries_for_repo(repo_root).max_by_key(|e| e.ts)
  }

  /// Remove and return the most-recent entry for `repo_root`. Used
  /// by `gwm undo` to consume the op it's about to replay. The
  /// caller is responsible for persisting the journal after a
  /// successful undo.
  pub fn pop_last_for_repo(&mut self, repo_root: &Path) -> Option<OpEntry> {
    let target = self
      .entries
      .iter()
      .enumerate()
      .filter(|(_, e)| e.repo_root == repo_root)
      .max_by_key(|(_, e)| e.ts)
      .map(|(i, _)| i)?;
    Some(self.entries.remove(target))
  }
}

/// Resolve the active journal path.
///
/// Resolution order:
///   1. `$GWM_HISTORY_FILE` if set and non-empty.
///   2. `$XDG_DATA_HOME/gwm/history.toml` if `$XDG_DATA_HOME` is set.
///   3. `dirs::data_dir()/gwm/history.toml`.
///
/// The fallback maps to platform-native data dirs (Linux:
/// `~/.local/share`, macOS: `~/Library/Application Support`, Windows:
/// `%LOCALAPPDATA%`).
pub fn default_journal_path() -> Result<PathBuf> {
  if let Ok(p) = std::env::var("GWM_HISTORY_FILE") {
    if !p.is_empty() {
      return Ok(PathBuf::from(p));
    }
  }
  if let Ok(p) = std::env::var("XDG_DATA_HOME") {
    if !p.is_empty() {
      return Ok(PathBuf::from(p).join("gwm").join("history.toml"));
    }
  }
  let base = dirs::data_dir().ok_or_else(|| {
    GwmError::Other("could not resolve user data directory — set GWM_HISTORY_FILE to override".into())
  })?;
  Ok(base.join("gwm").join("history.toml"))
}

/// Convenience: append `entry` to the journal at
/// [`default_journal_path`], creating parent dirs as needed. On IO
/// failure, returns the error WITHOUT modifying the on-disk file —
/// the caller (usually `worktree::remove` plumbing) decides whether
/// to fail the destructive op or log + continue. The recommended
/// pattern is "log + continue": never block a destruction the user
/// explicitly asked for because the journal couldn't be written.
pub fn record(entry: OpEntry) -> Result<()> {
  let path = default_journal_path()?;
  let mut journal = Journal::load(&path)?;
  journal.append(entry);
  journal.save(&path)?;
  Ok(())
}
