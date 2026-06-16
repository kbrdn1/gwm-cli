//! The statusline consumer (issue #309): the first real client of the
//! `gwm daemon` JSON-RPC surface (issue #38).
//!
//! This module is the **pure render core** — it turns a slice of
//! [`JsonWorktree`] (exactly what the daemon's `list` / `subscribe` stream
//! hands back) into a compact, single-line summary for tmux / starship /
//! zsh prompts. No socket, no git I/O, no clock: deterministic and unit
//! tested in `tests/statusline_tests.rs`. The socket transport lives in
//! [`crate::daemon`] (client side) behind the `daemon` feature.
//!
//! Scope note (MVP): the summary is built **only** from fields present in
//! [`JsonWorktree`] — active branch, worktree count, dirty / ahead /
//! behind, and the linked issue / PR numbers. A CI rollup (the
//! `CI passing 9/9` half of issue #309's example) is deliberately omitted:
//! it is not part of the stable daemon schema, and fetching it per update
//! would mean a `gh` call on every prompt redraw. That is left as a
//! follow-up rather than smuggled into the daemon. `age_seconds` is also
//! skipped — [`crate::daemon::worktrees_differ`] ignores it, so it never
//! moves a `--watch` stream.

use crate::json_api::JsonWorktree;
use std::path::{Path, PathBuf};

/// Resolve which worktree in `worktrees` owns `cwd`, passing every path
/// (the `cwd` and each worktree `path`) through `canonicalize` first.
///
/// A worktree owns `cwd` when its canonicalised `path` is an ancestor of
/// (or equal to) the canonicalised `cwd`. When several match — a worktree
/// nested inside another — the one with the longest canonical path wins
/// (the most specific enclosing worktree). Returns `None` when `cwd` is
/// outside every worktree.
///
/// The canonicaliser is injected so the core stays free of filesystem
/// access (and unit-testable): callers that need symlink equivalence
/// (e.g. macOS `/var` ↔ `/private/var`, or a worktree added under a
/// symlink) pass `std::fs::canonicalize`; tests pass a deterministic stub.
/// Canonicalising **both** sides is the point — resolving only the `cwd`
/// while the daemon's paths stay raw would defeat the match.
pub fn active_index_with<F>(worktrees: &[JsonWorktree], cwd: &Path, canonicalize: F) -> Option<usize>
where
  F: Fn(&Path) -> PathBuf,
{
  let cwd = canonicalize(cwd);
  worktrees
    .iter()
    .enumerate()
    .filter_map(|(i, w)| {
      let wp = canonicalize(Path::new(&w.path));
      cwd.starts_with(&wp).then_some((i, wp))
    })
    .max_by_key(|(_, wp)| wp.as_os_str().len())
    .map(|(i, _)| i)
}

/// Resolve the enclosing worktree by raw path comparison — [`active_index_with`]
/// under the identity canonicaliser. No filesystem access, so symlink
/// equivalence is the caller's responsibility (see [`active_index_with`]).
pub fn active_index(worktrees: &[JsonWorktree], cwd: &Path) -> Option<usize> {
  active_index_with(worktrees, cwd, |p| p.to_path_buf())
}

/// Render the compact statusline for `worktrees`, highlighting the
/// `active` worktree's branch and local state. Returns an empty string for
/// an empty set so a prompt substitution collapses to nothing.
pub fn render(worktrees: &[JsonWorktree], active: Option<usize>) -> String {
  if worktrees.is_empty() {
    return String::new();
  }

  let mut parts: Vec<String> = Vec::new();

  // Active branch (or worktree name when detached) leads the line.
  if let Some(w) = active.and_then(|i| worktrees.get(i)) {
    parts.push(w.branch.clone().unwrap_or_else(|| w.name.clone()));
  }

  parts.push(format!("{} wt", worktrees.len()));

  if let Some(w) = active.and_then(|i| worktrees.get(i)) {
    // Local-state flags for the active worktree: `*` dirty, `↑n` ahead,
    // `↓n` behind. An `unknown` status (detached / unborn HEAD) has no
    // meaningful ahead/behind/dirty, so the whole group is suppressed.
    let s = &w.status;
    if !s.unknown {
      let mut flags: Vec<String> = Vec::new();
      if s.is_dirty {
        flags.push("*".to_string());
      }
      if s.has_upstream {
        if s.ahead > 0 {
          flags.push(format!("↑{}", s.ahead));
        }
        if s.behind > 0 {
          flags.push(format!("↓{}", s.behind));
        }
      }
      if !flags.is_empty() {
        parts.push(flags.join(" "));
      }
    }

    if let Some(issue) = w.issue {
      parts.push(format!("#{issue}"));
    }
    if let Some(pr) = w.pr {
      parts.push(format!("PR #{pr}"));
    }
  }

  parts.join(" · ")
}

/// Convenience: resolve the active worktree from `cwd`, then [`render`].
pub fn render_for_cwd(worktrees: &[JsonWorktree], cwd: &Path) -> String {
  render(worktrees, active_index(worktrees, cwd))
}
