//! Tests for `src/agent_sessions.rs` — agent session detection (issue #408).
//!
//! Every test seeds its own artefact tree in a `tempfile::TempDir` and calls
//! the backends through their base-dir parameter; nothing here reads `$HOME`
//! or any ambient state.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gwm::agent_sessions::{
  claude_slug, AgentKind, ClaudeCodeSource, CodexSource, Freshness, OpencodeSource, VibeSource,
};

/// Write a file, creating parents.
fn write(path: &Path, contents: &str) {
  fs::create_dir_all(path.parent().unwrap()).unwrap();
  fs::write(path, contents).unwrap();
}

/// Write a file and backdate its mtime.
fn write_aged(path: &Path, contents: &str, mtime: SystemTime) {
  write(path, contents);
  let f = fs::File::options().append(true).open(path).unwrap();
  f.set_modified(mtime).unwrap();
}

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

// -- Claude Code backend (research.md D2) --

#[test]
fn claude_scan_yields_one_session_per_jsonl_in_matched_slug_dir() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("projects");
  let wt = PathBuf::from("/Users/x/Projects/gwm-cli");
  let dir = base.join(claude_slug(&wt));
  write(&dir.join("aaaa-1111.jsonl"), "{}");
  write(&dir.join("bbbb-2222.jsonl"), "{}");

  let now = SystemTime::now();
  let sessions = ClaudeCodeSource.scan(&base, &[wt.clone()], now);
  assert_eq!(sessions.len(), 2);
  for s in &sessions {
    assert_eq!(s.kind, AgentKind::ClaudeCode);
    assert_eq!(s.cwd, wt);
    assert!(!s.ended);
  }
  let mut ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
  ids.sort_unstable();
  assert_eq!(ids, ["aaaa-1111", "bbbb-2222"]);
}

#[test]
fn claude_scan_ignores_non_jsonl_entries() {
  // Real project dirs contain a `memory/` subdir and other non-session files.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("projects");
  let wt = PathBuf::from("/Users/x/proj");
  let dir = base.join(claude_slug(&wt));
  write(&dir.join("cccc-3333.jsonl"), "{}");
  write(&dir.join("notes.txt"), "not a session");
  fs::create_dir_all(dir.join("memory")).unwrap();

  let sessions = ClaudeCodeSource.scan(&base, &[wt], SystemTime::now());
  assert_eq!(sessions.len(), 1);
  assert_eq!(sessions[0].id, "cccc-3333");
}

#[test]
fn claude_scan_missing_base_or_unmatched_worktree_is_empty() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("does-not-exist");
  let wt = PathBuf::from("/Users/x/proj");
  assert!(ClaudeCodeSource.scan(&base, &[wt], SystemTime::now()).is_empty());

  let base2 = tmp.path().join("projects");
  fs::create_dir_all(&base2).unwrap();
  let unmatched = PathBuf::from("/Users/x/never-opened");
  assert!(
    ClaudeCodeSource
      .scan(&base2, &[unmatched], SystemTime::now())
      .is_empty()
  );
}

// -- Codex backend (research.md D3) --

const CODEX_META: &str = r#"{"timestamp":"2026-07-21T10:00:00.000Z","type":"session_meta","payload":{"session_id":"019f6b95-b01a-7d30-a28a-68d9813e2248","cwd":"/work/one","originator":"codex_exec"}}"#;

#[test]
fn codex_scan_recovers_cwd_from_first_line_session_meta() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  let file = base.join("2026/07/21/rollout-2026-07-21T10-00-00-019f6b95.jsonl");
  // Second line is a huge unrelated event — only the first line may be read.
  write(&file, &format!("{CODEX_META}\n{{\"type\":\"other\"}}\n"));

  let sessions = CodexSource.scan(&base, SystemTime::now());
  assert_eq!(sessions.len(), 1);
  let s = &sessions[0];
  assert_eq!(s.kind, AgentKind::Codex);
  assert_eq!(s.cwd, PathBuf::from("/work/one"));
  assert_eq!(s.id, "019f6b95-b01a-7d30-a28a-68d9813e2248");
  assert!(!s.ended);
}

#[test]
fn codex_scan_skips_legacy_json_and_malformed_first_lines() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  // Legacy pre-jsonl format seen in real data: .json extension → skipped.
  write(&base.join("2025/04/19/rollout-2025-04-19-old.json"), CODEX_META);
  // Malformed first line → skipped silently, must not hide the valid one.
  write(&base.join("2026/07/21/rollout-broken.jsonl"), "not json at all\n");
  write(
    &base.join("2026/07/21/rollout-good.jsonl"),
    &format!("{CODEX_META}\n"),
  );

  let sessions = CodexSource.scan(&base, SystemTime::now());
  assert_eq!(sessions.len(), 1);
  assert_eq!(sessions[0].cwd, PathBuf::from("/work/one"));
}

#[test]
fn codex_scan_bounds_by_recency_window() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  let now = SystemTime::now();
  write(&base.join("2026/07/21/rollout-now.jsonl"), &format!("{CODEX_META}\n"));
  write_aged(
    &base.join("2020/01/01/rollout-ancient.jsonl"),
    &format!("{CODEX_META}\n"),
    now - Duration::from_secs(2000 * 24 * 60 * 60),
  );

  let sessions = CodexSource.scan(&base, now);
  assert_eq!(sessions.len(), 1);
  assert_eq!(sessions[0].id, "019f6b95-b01a-7d30-a28a-68d9813e2248");
}

// -- opencode backend (research.md D4) --

fn opencode_project(id: &str, worktree: &str, created_ms: u64, updated_ms: Option<u64>) -> String {
  let time = match updated_ms {
    Some(u) => format!(r#"{{"created":{created_ms},"updated":{u}}}"#),
    None => format!(r#"{{"created":{created_ms}}}"#),
  };
  format!(r#"{{"id":"{id}","worktree":"{worktree}","vcs":"git","time":{time},"sandboxes":[]}}"#)
}

fn epoch_ms(t: SystemTime) -> u64 {
  t.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64
}

#[test]
fn opencode_scan_recovers_worktree_and_updated_time() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("project");
  let now = SystemTime::now();
  let updated = epoch_ms(now - Duration::from_secs(60));
  write(
    &base.join("d4d5e31c.json"),
    &opencode_project("d4d5e31c", "/work/front", 1, Some(updated)),
  );

  let sessions = OpencodeSource.scan(&base, now);
  assert_eq!(sessions.len(), 1);
  let s = &sessions[0];
  assert_eq!(s.kind, AgentKind::Opencode);
  assert_eq!(s.cwd, PathBuf::from("/work/front"));
  assert_eq!(s.id, "d4d5e31c");
  assert_eq!(
    s.last_activity,
    SystemTime::UNIX_EPOCH + Duration::from_millis(updated)
  );
  assert!(!s.ended);
}

#[test]
fn opencode_scan_skips_the_global_pseudo_project() {
  // Real data: global.json has id "global" and worktree "/".
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("project");
  let now = SystemTime::now();
  write(
    &base.join("global.json"),
    &opencode_project("global", "/", epoch_ms(now), None),
  );
  assert!(OpencodeSource.scan(&base, now).is_empty());
}

#[test]
fn opencode_scan_falls_back_to_created_then_mtime() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("project");
  let now = SystemTime::now();
  let created = epoch_ms(now - Duration::from_secs(120));
  // No "updated" → falls back to "created".
  write(
    &base.join("aa.json"),
    &opencode_project("aa", "/work/a", created, None),
  );
  // No "time" at all → falls back to file mtime (fresh, so within window).
  write(&base.join("bb.json"), r#"{"id":"bb","worktree":"/work/b"}"#);

  let sessions = OpencodeSource.scan(&base, now);
  assert_eq!(sessions.len(), 2);
  let aa = sessions.iter().find(|s| s.id == "aa").unwrap();
  assert_eq!(
    aa.last_activity,
    SystemTime::UNIX_EPOCH + Duration::from_millis(created)
  );
  assert!(sessions.iter().any(|s| s.id == "bb"));
}

#[test]
fn opencode_scan_bounds_by_recency_window() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("project");
  let now = SystemTime::now();
  let stale = epoch_ms(now - Duration::from_secs(40 * 24 * 60 * 60));
  write(
    &base.join("old.json"),
    &opencode_project("old", "/work/old", 1, Some(stale)),
  );
  assert!(OpencodeSource.scan(&base, now).is_empty());
}

// -- Mistral Vibe backend (research.md D5) --

fn vibe_meta(session_id: &str, cwd: &str, end_time: Option<&str>) -> String {
  let end = match end_time {
    Some(t) => format!(r#""{t}""#),
    None => "null".to_string(),
  };
  format!(
    r#"{{"session_id":"{session_id}","start_time":"2026-07-21T10:00:00.000000","end_time":{end},"environment":{{"working_directory":"{cwd}"}},"username":"u"}}"#
  )
}

#[test]
fn vibe_scan_recovers_working_directory_and_liveness() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("session");
  // Running session: end_time is null.
  let live = base.join("session_20260721_100000_aaaa1111");
  write(&live.join("meta.json"), &vibe_meta("aaaa-1111", "/work/live", None));
  write(&live.join("messages.jsonl"), "{}\n");
  // Terminated session: end_time set → ended, regardless of fresh mtimes.
  let done = base.join("session_20260721_090000_bbbb2222");
  write(
    &done.join("meta.json"),
    &vibe_meta("bbbb-2222", "/work/done", Some("2026-07-21T09:30:00.000000")),
  );
  write(&done.join("messages.jsonl"), "{}\n");

  let mut sessions = VibeSource.scan(&base, SystemTime::now());
  sessions.sort_by(|a, b| a.id.cmp(&b.id));
  assert_eq!(sessions.len(), 2);
  assert_eq!(sessions[0].id, "aaaa-1111");
  assert_eq!(sessions[0].cwd, PathBuf::from("/work/live"));
  assert!(!sessions[0].ended);
  assert_eq!(sessions[1].id, "bbbb-2222");
  assert!(sessions[1].ended);
  assert_eq!(sessions[0].kind, AgentKind::Vibe);
}

#[test]
fn vibe_scan_skips_malformed_meta_and_bounds_by_recency() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("session");
  let now = SystemTime::now();
  // Malformed meta.json → skipped silently.
  let broken = base.join("session_20260721_080000_cccc3333");
  write(&broken.join("meta.json"), "not json");
  // Ancient session (messages.jsonl mtime beyond window) → skipped.
  let old = base.join("session_20250101_000000_dddd4444");
  write(&old.join("meta.json"), &vibe_meta("dddd-4444", "/work/old", None));
  write_aged(
    &old.join("messages.jsonl"),
    "{}\n",
    now - Duration::from_secs(200 * 24 * 60 * 60),
  );
  // And its meta.json must be aged too (it is the fallback activity source).
  write_aged(
    &old.join("meta.json"),
    &vibe_meta("dddd-4444", "/work/old", None),
    now - Duration::from_secs(200 * 24 * 60 * 60),
  );
  // One good one.
  let good = base.join("session_20260721_110000_eeee5555");
  write(&good.join("meta.json"), &vibe_meta("eeee-5555", "/work/good", None));
  write(&good.join("messages.jsonl"), "{}\n");

  let sessions = VibeSource.scan(&base, now);
  assert_eq!(sessions.len(), 1);
  assert_eq!(sessions[0].id, "eeee-5555");
}

#[test]
fn codex_scan_missing_base_is_empty() {
  let tmp = tempfile::TempDir::new().unwrap();
  assert!(
    CodexSource
      .scan(&tmp.path().join("nope"), SystemTime::now())
      .is_empty()
  );
}

#[test]
fn claude_scan_skips_sessions_older_than_the_scan_window() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("projects");
  let wt = PathBuf::from("/Users/x/proj");
  let dir = base.join(claude_slug(&wt));
  let now = SystemTime::now();
  write(&dir.join("recent.jsonl"), "{}");
  write_aged(
    &dir.join("ancient.jsonl"),
    "{}",
    now - Duration::from_secs(40 * 24 * 60 * 60),
  );

  let sessions = ClaudeCodeSource.scan(&base, &[wt], now);
  assert_eq!(sessions.len(), 1);
  assert_eq!(sessions[0].id, "recent");
}
