//! Tests for `src/agent_sessions.rs` — agent session detection (issue #408).
//!
//! Every test seeds its own artefact tree in a `tempfile::TempDir` and calls
//! the backends through their base-dir parameter; nothing here reads `$HOME`
//! or any ambient state.

use std::path::Path;
use std::time::{Duration, SystemTime};

use gwm::agent_sessions::{claude_slug, Freshness};

// -- Claude Code cwd-slug convention (research.md D2, pinned on real dirs) --

#[test]
fn claude_slug_replaces_separators_with_dashes() {
  assert_eq!(
    claude_slug(Path::new("/Users/x/Projects/gwm-cli")),
    "-Users-x-Projects-gwm-cli"
  );
}

#[test]
fn claude_slug_collapses_dots_to_dashes_yielding_double_dash() {
  // Real evidence: /Users/kbrdn1/.claude → -Users-kbrdn1--claude
  assert_eq!(claude_slug(Path::new("/Users/x/.claude")), "-Users-x--claude");
}

#[test]
fn claude_slug_preserves_case_and_existing_hyphens() {
  assert_eq!(
    claude_slug(Path::new("/Users/x/cc-worktree/LazyCurl")),
    "-Users-x-cc-worktree-LazyCurl"
  );
}

#[test]
fn claude_slug_maps_every_non_alphanumeric_to_dash() {
  // Underscores and spaces collapse too: [^A-Za-z0-9] → '-'.
  assert_eq!(claude_slug(Path::new("/tmp/a_b c.d")), "-tmp-a-b-c-d");
}

// -- Freshness classification (research.md D10) --

#[test]
fn freshness_recent_activity_is_active() {
  let now = SystemTime::now();
  let last = now - Duration::from_secs(100);
  assert_eq!(Freshness::classify(last, false, now), Freshness::Active);
}

#[test]
fn freshness_activity_older_than_window_is_idle() {
  let now = SystemTime::now();
  let last = now - Duration::from_secs(301);
  assert_eq!(Freshness::classify(last, false, now), Freshness::Idle);
}

#[test]
fn freshness_boundary_exactly_at_window_is_active() {
  let now = SystemTime::now();
  let last = now - Duration::from_secs(300);
  assert_eq!(Freshness::classify(last, false, now), Freshness::Active);
}

#[test]
fn freshness_future_mtime_clamps_to_active() {
  // Clock skew: an artefact stamped in the future is active, never an error.
  let now = SystemTime::now();
  let last = now + Duration::from_secs(3600);
  assert_eq!(Freshness::classify(last, false, now), Freshness::Active);
}

#[test]
fn freshness_ended_session_is_idle_regardless_of_recency() {
  // Vibe's non-null end_time forces idle even with a fresh mtime.
  let now = SystemTime::now();
  let last = now - Duration::from_secs(1);
  assert_eq!(Freshness::classify(last, true, now), Freshness::Idle);
}
