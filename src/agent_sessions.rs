//! Agent session detection (issue #408).
//!
//! Detects AI-agent coding sessions (Claude Code, Codex, opencode, Mistral
//! Vibe) by reading each tool's persisted session artefacts under the user's
//! home area — `std::fs` only, no process scanning, no OS-specific API, so the
//! same code path runs on Linux, macOS and Windows and every backend is
//! testable against a seeded `tempfile::TempDir`.
//!
//! Every backend takes its artefact root as a parameter (the injection seam);
//! the single production call site resolves the real locations from
//! `dirs::home_dir()`. Detection is deliberately *total*: missing directories,
//! malformed records or unreadable files degrade to "no sessions", never to an
//! error (FR-009 in `.specify/specs/408-agent-session-pane/spec.md`).

use std::path::Path;
use std::time::{Duration, SystemTime};

/// How long after its last artefact activity a session still counts as active.
pub const ACTIVE_WINDOW: Duration = Duration::from_secs(300);

/// Activity classification of a detected session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
  Active,
  Idle,
}

impl Freshness {
  /// Classify from the last artefact activity. `ended` (only Vibe can set it)
  /// forces `Idle`; a timestamp in the future clamps to `Active`.
  pub fn classify(last_activity: SystemTime, ended: bool, now: SystemTime) -> Self {
    if ended {
      return Freshness::Idle;
    }
    // duration_since errs when last_activity > now (clock skew) — that is
    // "just happened", so it clamps to zero elapsed and reads Active.
    let elapsed = now
      .duration_since(last_activity)
      .unwrap_or(Duration::ZERO);
    if elapsed <= ACTIVE_WINDOW {
      Freshness::Active
    } else {
      Freshness::Idle
    }
  }
}

/// Claude Code's project-directory slug for a working directory.
///
/// Convention pinned on real `~/.claude/projects/` entries (research.md D2):
/// every character outside `[A-Za-z0-9]` becomes `-`, case is preserved. The
/// mapping is lossy, so matching is forward-only: slugify the worktree path we
/// manage and look the directory up — never try to reverse a slug.
pub fn claude_slug(path: &Path) -> String {
  path
    .to_string_lossy()
    .chars()
    .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
    .collect()
}
