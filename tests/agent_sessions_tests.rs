//! Tests for `src/agent_sessions.rs` — agent session detection (issue #408).
//!
//! Every test seeds its own artefact tree in a `tempfile::TempDir` and calls
//! the backends through their base-dir parameter; nothing here reads `$HOME`
//! or any ambient state.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gwm::agent_sessions::{
  claude_slug, codex_day_dir, summarize_with, AgentKind, AgentSession, ClaudeCodeSource, CodexSource, Freshness,
  OpencodeSource, VibeSource,
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
  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), now);
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
  assert!(ClaudeCodeSource
    .scan(&base2, &[unmatched], SystemTime::now())
    .is_empty());
}

#[test]
fn claude_scan_sweeps_unmatched_project_dirs_into_the_pool() {
  // A Claude session launched in a project that is no managed worktree
  // (another repo, a subdirectory, an old path) must still reach the raw
  // pool, otherwise the attach-by-id prompt can never pin it (Codex review
  // round C — the prompt exists precisely for unmatched sessions).
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("projects");
  let wt = PathBuf::from("/Users/x/proj");
  write(&base.join(claude_slug(&wt)).join("matched-1.jsonl"), "{}");
  let foreign = base.join("-Users-x-other-project");
  write(
    &foreign.join("foreign-1.jsonl"),
    r#"{"type":"user","message":{"role":"user","content":"other work"}}"#,
  );

  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
  let mut ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
  ids.sort_unstable();
  assert_eq!(ids, ["foreign-1", "matched-1"]);
  let f = sessions.iter().find(|s| s.id == "foreign-1").unwrap();
  // The lossy slug cannot be reversed to a cwd, so the swept session
  // carries the project dir purely as provenance.
  assert_eq!(f.cwd, foreign);
  assert_eq!(f.name.as_deref(), Some("other work"));
}

#[test]
fn swept_foreign_claude_sessions_never_join_a_worktree_summary() {
  // The sweep feeds the pool only: its provenance cwd (a dir under the
  // Claude base) can never forward-match a real worktree path, so the
  // per-worktree summary stays exactly what the slug matching produced.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("projects");
  let wt = PathBuf::from("/Users/x/proj");
  write(&base.join("-Users-x-other-project").join("foreign-1.jsonl"), "{}");

  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
  assert_eq!(sessions.len(), 1, "the swept session is in the pool");
  let keyed = [("wt".to_string(), wt)];
  let map = gwm::agent_sessions::summarize(&sessions, &keyed);
  assert!(map.is_empty(), "but no worktree claims it: {map:?}");
}

#[test]
fn scan_matched_never_visits_foreign_project_dirs() {
  // Codex review round F: the global sweep costs a bounded read of every
  // recent foreign artefact (names), so the summary-only surfaces
  // (`gwm list`, daemon polls) use the matched-only scan; only the pool
  // surfaces (`gwm agents`, the attach-by-id prompt) pay for the sweep.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("projects");
  let wt = PathBuf::from("/Users/x/proj");
  write(&base.join(claude_slug(&wt)).join("matched-1.jsonl"), "{}");
  write(&base.join("-Users-x-other").join("foreign-1.jsonl"), "{}");

  let sessions = ClaudeCodeSource.scan_matched(&base, std::slice::from_ref(&wt), SystemTime::now());
  let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
  assert_eq!(ids, ["matched-1"], "no foreign session in the matched scan");
}

#[test]
fn detect_all_still_resolves_an_unmatched_claude_pin_without_the_sweep() {
  // Guard for the round-F split: the summary path drops the global sweep,
  // so a pin on a foreign Claude session must resolve through the by-id
  // sweep fallback — pins keep working on every surface.
  let tmp = tempfile::TempDir::new().unwrap();
  write(
    &tmp
      .path()
      .join(".claude/projects/-Users-x-other")
      .join("pinned-far.jsonl"),
    r#"{"type":"user","message":{"role":"user","content":"far away"}}"#,
  );
  let keyed = [("wt".to_string(), PathBuf::from("/Users/x/proj"))];
  let pins = [("wt".to_string(), "pinned-far".to_string())];
  let map = gwm::agent_sessions::detect_all(tmp.path(), &keyed, &pins, SystemTime::now());
  let agents = map.get("wt").expect("the pin materialises the session");
  assert_eq!(agents.sessions.len(), 1);
  assert_eq!(agents.sessions[0].id, "pinned-far");
}

#[test]
fn claude_by_id_pin_never_escapes_the_store() {
  // Codex review round J (P2): the pinned id becomes a file name in the
  // by-id sweep (`<project dir>/<sid>.jsonl`) — an id carrying a path
  // separator would make `Path::join` escape the store, so
  // `gwm agents attach <wt> /tmp/foo` could pin and read any fresh
  // `.jsonl` on disk. Separator-carrying ids must resolve to nothing.
  let tmp = tempfile::TempDir::new().unwrap();
  write(
    &tmp.path().join("outside.jsonl"),
    r#"{"type":"user","message":{"role":"user","content":"pwned"}}"#,
  );
  fs::create_dir_all(tmp.path().join(".claude/projects/-Users-x-proj")).unwrap();
  let keyed = [("wt".to_string(), PathBuf::from("/Users/x/proj"))];
  let evil = tmp.path().join("outside").display().to_string();
  let pins = [("wt".to_string(), evil)];
  let map = gwm::agent_sessions::detect_all(tmp.path(), &keyed, &pins, SystemTime::now());
  assert!(
    map.get("wt").is_none_or(|a| a.sessions.is_empty()),
    "a path-shaped pin id must not resolve: {map:?}"
  );
}

#[test]
fn session_pool_is_sorted_live_first() {
  // User feedback 2026-07-22: the attach-by-id prompt and every listing fed
  // by the raw pool must offer ACTIVE sessions first — the pool used to be
  // raw backend concatenation order.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join(".claude/projects");
  let wt = PathBuf::from("/Users/x/proj");
  let dir = base.join(claude_slug(&wt));
  let now = SystemTime::now();
  write_aged(&dir.join("old-idle.jsonl"), "{}", now - Duration::from_secs(4000));
  write(&dir.join("fresh-active.jsonl"), "{}");

  let keyed = [("wt".to_string(), wt)];
  let (_map, pool) = gwm::agent_sessions::detect_with_sessions(tmp.path(), &keyed, &[], now);
  let ids: Vec<&str> = pool.iter().map(|s| s.id.as_str()).collect();
  assert_eq!(
    ids,
    ["fresh-active", "old-idle"],
    "most recent (active) first in the pool"
  );
}

#[test]
fn session_names_neutralise_control_characters() {
  // Codex review round D: a hostile or corrupt artefact must not smuggle
  // ANSI escapes into the terminal through `gwm agents` / the TUI. ESC is
  // not whitespace, so `split_whitespace` alone keeps it.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("projects");
  let wt = PathBuf::from("/Users/x/proj");
  write(
    &base.join(claude_slug(&wt)).join("evil-1.jsonl"),
    "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"\\u001b[31mevil\\u001b[0m name\"}}",
  );

  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
  let name = sessions[0].name.as_deref().expect("a name is extracted");
  assert!(
    !name.chars().any(|c| c.is_control()),
    "control characters must be stripped: {name:?}"
  );
  assert!(name.contains("evil"), "the visible text survives: {name:?}");
}

#[test]
fn live_session_names_are_sanitised_too() {
  // The live registry (`~/.claude/sessions/<pid>.json`) is another
  // untrusted input: same stripping, same length cap.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join(".claude/projects");
  let wt = PathBuf::from("/Users/x/proj");
  write(&base.join(claude_slug(&wt)).join("aaaa-1111.jsonl"), "{}");
  let long = "x".repeat(80);
  write(
    &tmp.path().join(".claude/sessions/99.json"),
    &format!("{{\"sessionId\":\"aaaa-1111\",\"name\":\"\\u001b]0;t\\u0007{long}\"}}"),
  );

  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
  let name = sessions[0].name.as_deref().expect("a name is extracted");
  assert!(!name.chars().any(|c| c.is_control()), "sanitised: {name:?}");
  assert!(name.chars().count() <= 60, "capped: {} chars", name.chars().count());
}

#[test]
fn command_name_titles_are_capped_like_any_other_name() {
  // The `<command-name>` fast path must apply the same cap + stripping as
  // the plain-prompt branch (Codex review round D).
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("projects");
  let wt = PathBuf::from("/Users/x/proj");
  let long_cmd = "c".repeat(100);
  write(
    &base.join(claude_slug(&wt)).join("bbbb-2222.jsonl"),
    &format!(
      r#"{{"type":"user","message":{{"role":"user","content":"<command-name>/{long_cmd}</command-name> args"}}}}"#
    ),
  );

  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
  let name = sessions[0].name.as_deref().expect("a name is extracted");
  assert!(name.chars().count() <= 60, "capped: {} chars", name.chars().count());
  assert!(name.starts_with("/c"), "still the command name: {name:?}");
}

// -- opencode SQLite backend (user feedback 2026-07-22) --

/// Build the opencode home layout with an `opencode.db` (schema subset of
/// the real one — only the queried columns). Skips silently when the
/// `sqlite3` CLI is unavailable (Windows CI) — the JSON fallback tests
/// cover that path.
fn seed_opencode_db(home: &Path, rows: &str) {
  let dir = home.join(".local/share/opencode");
  fs::create_dir_all(dir.join("storage/project")).unwrap();
  let conn = rusqlite::Connection::open(dir.join("opencode.db")).unwrap();
  let sql = format!(
    "CREATE TABLE session (id text, parent_id text, directory text, title text, time_updated integer, time_archived integer); {rows}"
  );
  conn.execute_batch(&sql).unwrap();
}

#[test]
fn opencode_scan_reads_the_sqlite_db_when_present() {
  // opencode ≥ 1.x migrated `storage/project/*.json` into `opencode.db`
  // (`storage/migration` marker); the stale JSON made every new session
  // invisible (user feedback 2026-07-22). The db is authoritative when
  // present: `directory` is the cwd, `title` the name (renames show),
  // `time_updated` (epoch ms) the activity, `time_archived` the end.
  let tmp = tempfile::TempDir::new().unwrap();
  let now_ms = SystemTime::now()
    .duration_since(SystemTime::UNIX_EPOCH)
    .unwrap()
    .as_millis();
  let old_ms = now_ms - 40 * 24 * 3600 * 1000; // outside the 30-day window
  let rows = format!(
    "INSERT INTO session VALUES ('ses_live', NULL, '/work/one', 'Rename test-opencode', {now_ms}, NULL); \
     INSERT INTO session VALUES ('ses_done', NULL, '/work/one', 'archived one', {now_ms}, {now_ms}); \
     INSERT INTO session VALUES ('ses_old', NULL, '/work/one', 'too old', {old_ms}, NULL); \
     INSERT INTO session VALUES ('ses_child', 'ses_live', '/work/one', 'subagent', {now_ms}, NULL);"
  );
  seed_opencode_db(tmp.path(), &rows);

  let base = tmp.path().join(".local/share/opencode/storage/project");
  let sessions = OpencodeSource.scan(&base, SystemTime::now());
  let mut ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
  ids.sort_unstable();
  assert_eq!(ids, ["ses_done", "ses_live"], "recent top-level only: {ids:?}");
  let live = sessions.iter().find(|s| s.id == "ses_live").unwrap();
  assert_eq!(live.cwd, PathBuf::from("/work/one"));
  assert_eq!(live.name.as_deref(), Some("Rename test-opencode"));
  assert!(!live.ended);
  let done = sessions.iter().find(|s| s.id == "ses_done").unwrap();
  assert!(done.ended, "time_archived marks the session ended");
}

// -- Codex backend (research.md D3) --

const CODEX_META: &str = r#"{"timestamp":"2026-07-21T10:00:00.000Z","type":"session_meta","payload":{"session_id":"019f6b95-b01a-7d30-a28a-68d9813e2248","cwd":"/work/one","originator":"codex_exec"}}"#;

#[test]
fn codex_scan_recovers_cwd_from_first_line_session_meta() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  let file = base
    .join(codex_day_dir(SystemTime::now()))
    .join("rollout-2026-07-21T10-00-00-019f6b95.jsonl");
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
fn codex_thread_name_beats_the_first_prompt() {
  // User feedback 2026-07-22: `codex` renames land in
  // `~/.codex/session_index.jsonl` ({id, thread_name, updated_at}, one
  // JSON per line, append-only — later lines win). Same precedence as the
  // Claude live registry: the recorded name beats the first-prompt
  // heuristic, and a session absent from the index keeps the fallback.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  let named = r#"{"type":"session_meta","payload":{"session_id":"019f89bb-0dcd-7e60-9717-b3745ed8343d","cwd":"/work/one"}}
{"type":"event_msg","payload":{"type":"user_message","message":"first prompt text"}}"#;
  write(
    &base.join(codex_day_dir(SystemTime::now())).join("rollout-a.jsonl"),
    named,
  );
  let unnamed = r#"{"type":"session_meta","payload":{"session_id":"0199dddd-0000-0000-0000-000000000000","cwd":"/work/two"}}
{"type":"event_msg","payload":{"type":"user_message","message":"plain prompt"}}"#;
  write(
    &base.join(codex_day_dir(SystemTime::now())).join("rollout-b.jsonl"),
    unnamed,
  );
  write(
    &tmp.path().join("session_index.jsonl"),
    concat!(
      "{\"id\":\"019f89bb-0dcd-7e60-9717-b3745ed8343d\",\"thread_name\":\"old-name\",\"updated_at\":\"2026-07-22T10:00:00Z\"}\n",
      "not json — a corrupt line must not poison the rest\n",
      "{\"id\":\"019f89bb-0dcd-7e60-9717-b3745ed8343d\",\"thread_name\":\"test-rename\",\"updated_at\":\"2026-07-22T12:09:47Z\"}\n",
    ),
  );

  let sessions = CodexSource.scan(&base, SystemTime::now());
  let by_id = |id: &str| sessions.iter().find(|s| s.id == id).unwrap();
  assert_eq!(
    by_id("019f89bb-0dcd-7e60-9717-b3745ed8343d").name.as_deref(),
    Some("test-rename"),
    "the LAST index entry wins"
  );
  assert_eq!(
    by_id("0199dddd-0000-0000-0000-000000000000").name.as_deref(),
    Some("plain prompt"),
    "absent from the index -> first-prompt fallback"
  );
}

#[test]
fn session_ids_neutralise_control_characters() {
  // Codex review round G: ids print raw on the human surfaces (CLI listing,
  // TUI rows) — a malformed artefact id carrying ESC/newline could forge
  // lines or drive the terminal. Ids are sanitised at ingestion so display,
  // pins and JSON stay consistent.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  let meta = r#"{"type":"session_meta","payload":{"session_id":"evil[31m-id","cwd":"/work/one"}}"#;
  write(
    &base.join(codex_day_dir(SystemTime::now())).join("rollout-evil.jsonl"),
    &format!("{meta}\n"),
  );

  let sessions = CodexSource.scan(&base, SystemTime::now());
  assert_eq!(sessions.len(), 1);
  assert!(
    !sessions[0].id.chars().any(|c| c.is_control()),
    "control characters stripped from the id: {:?}",
    sessions[0].id
  );
  assert!(sessions[0].id.contains("evil"), "visible text survives");
}

#[test]
fn codex_first_line_longer_than_the_cap_is_rejected() {
  // Codex review round G: `read_line` allocates the WHOLE first line, so a
  // corrupt multi-hundred-MB artefact could exhaust the process. The read
  // is capped; a first line that big is skipped (FR-009 degradation) even
  // when it would parse as valid meta.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  let padding = "x".repeat(100 * 1024);
  let meta =
    format!(r#"{{"type":"session_meta","payload":{{"session_id":"huge-1","cwd":"/work/one","pad":"{padding}"}}}}"#);
  write(
    &base.join(codex_day_dir(SystemTime::now())).join("rollout-huge.jsonl"),
    &format!("{meta}\n"),
  );

  let sessions = CodexSource.scan(&base, SystemTime::now());
  assert!(sessions.is_empty(), "oversized first line must be skipped");
}

#[test]
fn detect_all_defers_codex_names_but_keeps_matched_and_pinned_ones() {
  // Codex review round G: on the summary surfaces, foreign Codex rollouts
  // must not pay the 64 KiB name read (`summarize` drops them anyway) —
  // but a rollout matched to a worktree, or pinned to one, keeps its name.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp
    .path()
    .join(".codex/sessions")
    .join(codex_day_dir(SystemTime::now()));
  // Windows worktree paths carry backslashes — escape for JSON embedding
  // (raw interpolation made the meta line unparseable on windows-latest).
  let mk = |sid: &str, cwd: &str, prompt: &str| {
    let cwd = cwd.replace('\\', "\\\\");
    format!(
      "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"{sid}\",\"cwd\":\"{cwd}\"}}}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"{prompt}\"}}}}\n"
    )
  };
  // A REAL worktree dir (canonical matching needs it on disk).
  let wt = tmp.path().join("wt");
  fs::create_dir_all(&wt).unwrap();
  write(
    &base.join("rollout-m.jsonl"),
    &mk("matched-cx", &wt.display().to_string(), "matched work"),
  );
  write(
    &base.join("rollout-f.jsonl"),
    &mk("foreign-cx", "/somewhere/else", "foreign work"),
  );
  write(
    &base.join("rollout-p.jsonl"),
    &mk("pinned-cx", "/another/place", "pinned work"),
  );

  let keyed = [("wt".to_string(), wt)];
  let pins = [("wt".to_string(), "pinned-cx".to_string())];
  let map = gwm::agent_sessions::detect_all(tmp.path(), &keyed, &pins, SystemTime::now());
  let agents = map.get("wt").expect("matched + pinned sessions");
  let name_of = |id: &str| {
    agents
      .sessions
      .iter()
      .find(|s| s.id == id)
      .and_then(|s| s.name.as_deref().map(str::to_string))
  };
  assert_eq!(name_of("matched-cx").as_deref(), Some("matched work"));
  assert_eq!(
    name_of("pinned-cx").as_deref(),
    Some("pinned work"),
    "a pin keeps its name"
  );
  assert!(!agents.sessions.iter().any(|s| s.id == "foreign-cx"));
}

#[test]
fn codex_scan_reads_the_newer_payload_id_field() {
  // Codex review round F: rollouts from newer Codex versions (0.138+)
  // carry `payload.id` instead of `payload.session_id`. Falling back to
  // the file stem there leaked `rollout-<date>-<uuid>` ids, breaking the
  // documented session-UUID contract for pins and JSON consumers.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  let meta = r#"{"type":"session_meta","payload":{"id":"0199aaaa-bbbb-cccc-dddd-eeeeffff0000","cwd":"/work/two"}}"#;
  write(
    &base
      .join(codex_day_dir(SystemTime::now()))
      .join("rollout-2026-07-22T09-00-00-0199aaaa.jsonl"),
    &format!("{meta}\n"),
  );

  let sessions = CodexSource.scan(&base, SystemTime::now());
  assert_eq!(sessions.len(), 1);
  assert_eq!(sessions[0].id, "0199aaaa-bbbb-cccc-dddd-eeeeffff0000");
}

#[test]
fn codex_scan_skips_legacy_json_and_malformed_first_lines() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  // Legacy pre-jsonl format seen in real data: .json extension → skipped.
  write(&base.join("2025/04/19/rollout-2025-04-19-old.json"), CODEX_META);
  // Malformed first line → skipped silently, must not hide the valid one.
  write(
    &base.join(codex_day_dir(SystemTime::now())).join("rollout-broken.jsonl"),
    "not json at all\n",
  );
  write(
    &base.join(codex_day_dir(SystemTime::now())).join("rollout-good.jsonl"),
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
  write(
    &base.join(codex_day_dir(SystemTime::now())).join("rollout-now.jsonl"),
    &format!("{CODEX_META}\n"),
  );
  write_aged(
    &base.join("2020/01/01/rollout-ancient.jsonl"),
    &format!("{CODEX_META}\n"),
    now - Duration::from_secs(2000 * 24 * 60 * 60),
  );

  let sessions = CodexSource.scan(&base, now);
  assert_eq!(sessions.len(), 1);
  assert_eq!(sessions[0].id, "019f6b95-b01a-7d30-a28a-68d9813e2248");
}

#[test]
fn codex_walk_prunes_day_dirs_beyond_the_resume_slack() {
  // Codex review round I (P2): a multi-year store must not be enumerated
  // wholesale on every detection — day dirs dated beyond SCAN_WINDOW +
  // RESUME_SLACK are pruned by NAME, so even a fresh mtime inside one
  // (impossible outside a resume, and resumes that old are out of the
  // documented slack) cannot resurrect the walk.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  write(
    &base.join("2000/01/01/rollout-ancient-fresh.jsonl"),
    &format!("{CODEX_META}\n"),
  );

  let sessions = CodexSource.scan(&base, SystemTime::now());
  assert!(sessions.is_empty(), "ancient day dir pruned by name, got {sessions:?}");
}

#[test]
fn codex_resume_within_the_slack_is_still_detected() {
  // The prune must NOT break `codex resume`: appending to a rollout in a
  // day dir older than SCAN_WINDOW but within RESUME_SLACK refreshes the
  // file mtime, and that session must still surface.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("sessions");
  let now = SystemTime::now();
  let resumed_day = codex_day_dir(now - Duration::from_secs(40 * 24 * 60 * 60));
  write(
    &base.join(resumed_day).join("rollout-resumed.jsonl"),
    &format!("{CODEX_META}\n"),
  );

  let sessions = CodexSource.scan(&base, now);
  assert_eq!(sessions.len(), 1, "40-day-old dir with a fresh append stays visible");
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
  assert_eq!(s.last_activity, SystemTime::UNIX_EPOCH + Duration::from_millis(updated));
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
  write(&base.join("aa.json"), &opencode_project("aa", "/work/a", created, None));
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

/// A Vibe session dir name whose embedded start date lies `days_ago` back
/// from now — fixtures must never hardcode a date the name-prune expires.
fn vibe_dir(days_ago: u64, hms_and_id: &str) -> String {
  let t = SystemTime::now() - Duration::from_secs(days_ago * 24 * 60 * 60);
  let day = codex_day_dir(t).display().to_string().replace('/', "");
  format!("session_{day}_{hms_and_id}")
}

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
fn vibe_walk_prunes_session_dirs_beyond_the_resume_slack() {
  // Codex review round J (P2): with years of Vibe sessions the walk
  // statted every historical dir before the recency gate. Dir names embed
  // the start date (`session_YYYYMMDD_...`) — dirs dated beyond
  // SCAN_WINDOW + the resume slack are pruned by NAME, fresh mtimes or
  // not, before any metadata call.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("session");
  let ancient = base.join("session_20000101_000000_zzzz9999");
  write(&ancient.join("meta.json"), &vibe_meta("zzzz9999", "/work/one", None));
  write(&ancient.join("messages.jsonl"), "{}\n"); // fresh mtime

  let sessions = VibeSource.scan(&base, SystemTime::now());
  assert!(sessions.is_empty(), "ancient-dated dir pruned by name: {sessions:?}");
}

#[test]
fn vibe_session_started_before_the_window_but_in_slack_stays_visible() {
  // The prune must not hide a long-lived session: started 40 days ago
  // (outside SCAN_WINDOW, inside the slack) with fresh activity, it stays.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("session");
  let dir = base.join(vibe_dir(40, "120000_aaaa1111"));
  write(&dir.join("meta.json"), &vibe_meta("aaaa1111", "/work/one", None));
  write(&dir.join("messages.jsonl"), "{}\n");

  let sessions = VibeSource.scan(&base, SystemTime::now());
  assert_eq!(sessions.len(), 1, "40-day-old start with fresh activity stays visible");
}

#[test]
fn vibe_ids_neutralise_control_characters_too() {
  // Codex review round H: every other backend routes its ids through
  // `clean_id`; a malformed Vibe `session_id` carrying ESC/newline must
  // not reach the terminal either — same for the dir-name fallback.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("session");
  let dir = base.join(vibe_dir(0, "100000_evil"));
  write(
    &dir.join("meta.json"),
    r#"{"session_id":"evil\u001b[31m-vibe","start_time":"2026-07-22T10:00:00.000000","end_time":null,"environment":{"working_directory":"/work/one"}}"#,
  );
  write(&dir.join("messages.jsonl"), "{}\n");

  let sessions = VibeSource.scan(&base, SystemTime::now());
  assert_eq!(sessions.len(), 1);
  assert!(
    !sessions[0].id.chars().any(|c| c.is_control()),
    "control characters stripped: {:?}",
    sessions[0].id
  );
  assert!(sessions[0].id.contains("evil"));
}

#[test]
fn vibe_scan_recovers_working_directory_and_liveness() {
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("session");
  // Running session: end_time is null.
  let live = base.join(vibe_dir(1, "100000_aaaa1111"));
  write(&live.join("meta.json"), &vibe_meta("aaaa-1111", "/work/live", None));
  write(&live.join("messages.jsonl"), "{}\n");
  // Terminated session: end_time set → ended, regardless of fresh mtimes.
  let done = base.join(vibe_dir(1, "090000_bbbb2222"));
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
  let broken = base.join(vibe_dir(1, "080000_cccc3333"));
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
  let good = base.join(vibe_dir(1, "110000_eeee5555"));
  write(&good.join("meta.json"), &vibe_meta("eeee-5555", "/work/good", None));
  write(&good.join("messages.jsonl"), "{}\n");

  let sessions = VibeSource.scan(&base, now);
  assert_eq!(sessions.len(), 1);
  assert_eq!(sessions[0].id, "eeee-5555");
}

// -- summarize: session → worktree matching (research.md D7) --

fn session(kind: AgentKind, cwd: &str, age_secs: u64, id: &str) -> AgentSession {
  AgentSession {
    kind,
    cwd: PathBuf::from(cwd),
    last_activity: SystemTime::now() - Duration::from_secs(age_secs),
    ended: false,
    id: id.to_string(),
    name: None,
  }
}

fn wt(id: &str, path: &str) -> (String, PathBuf) {
  (id.to_string(), PathBuf::from(path))
}

/// Identity canonicalizer — matching is then purely lexical.
fn ident(p: &Path) -> PathBuf {
  p.to_path_buf()
}

#[test]
fn summarize_matches_ignoring_trailing_separator() {
  let sessions = [session(AgentKind::Codex, "/work/one/", 10, "s1")];
  let map = summarize_with(&sessions, &[wt("one", "/work/one")], ident);
  assert_eq!(map.get("one").unwrap().sessions.len(), 1);
}

#[test]
fn summarize_matches_through_the_injected_canonicalizer() {
  // Symlink equivalence is the canonicalizer's job (statusline pattern):
  // both sides go through it before comparison.
  let canon = |p: &Path| {
    let s = p.to_string_lossy().replace("/link/", "/real/");
    PathBuf::from(s)
  };
  let sessions = [session(AgentKind::Opencode, "/link/proj", 10, "s1")];
  let map = summarize_with(&sessions, &[wt("proj", "/real/proj")], canon);
  assert_eq!(map.get("proj").unwrap().sessions.len(), 1);
}

#[test]
fn summarize_compares_exactly_after_canonicalisation() {
  // Codex review round F: case handling belongs to the CANONICALIZER, not
  // a lexical fold. On a case-insensitive volume `canonicalize` converges
  // both sides to the on-disk casing, so exact comparison still matches;
  // a platform-wide lowercase fold would MERGE two genuinely distinct
  // worktrees on a case-sensitive volume (fabricated match — worse than a
  // missed one).
  let sessions = [session(AgentKind::Codex, "/Work/One", 10, "s1")];
  let map = summarize_with(&sessions, &[wt("one", "/work/one")], ident);
  assert!(map.is_empty(), "identity canon + different case = no match");

  // A case-folding canonicalizer (what a case-insensitive volume gives)
  // still matches through the injected fn.
  let fold = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
  let map = summarize_with(&sessions, &[wt("one", "/work/one")], fold);
  assert_eq!(map.get("one").unwrap().sessions.len(), 1);
}

#[test]
fn summarize_converges_case_variants_on_case_insensitive_volumes() {
  // Codex review round L (P1, discarded with proof): the finding claimed
  // `canonicalize()` keeps the caller's casing on macOS, so an agent
  // recording `/x/PROJ` and libgit2 handing `/x/proj` would never match.
  // Rust's canonicalize does NOT use libc realpath semantics: it resolves
  // to the ON-DISK casing (`F_GETPATH` on macOS, final-path-by-handle on
  // Windows), so both sides converge and the round-F exact comparison
  // matches. Pinned end-to-end through the real `summarize`; on a
  // case-sensitive volume the variant path does not exist and the test
  // degrades to a skip (round-F semantics own that world).
  let tmp = tempfile::TempDir::new().unwrap();
  let real = tmp.path().join("proj");
  fs::create_dir_all(&real).unwrap();
  let variant = tmp.path().join("PROJ");
  if fs::metadata(&variant).is_err() {
    return; // case-sensitive volume
  }
  let sessions = [AgentSession {
    kind: AgentKind::Codex,
    cwd: variant,
    last_activity: SystemTime::now(),
    ended: false,
    id: "case-var".into(),
    name: None,
  }];
  let keyed = [("wt".to_string(), real)];
  let map = gwm::agent_sessions::summarize(&sessions, &keyed);
  assert_eq!(
    map.get("wt").map(|a| a.sessions.len()),
    Some(1),
    "canonicalize converges casing on this volume: {map:?}"
  );
}

#[test]
fn summarize_never_merges_case_distinct_worktrees() {
  // /repo/Foo and /repo/foo can be two REAL dirs on case-sensitive APFS /
  // any Linux fs: a session recorded in Foo must never surface on foo.
  let sessions = [session(AgentKind::Codex, "/repo/Foo", 10, "s1")];
  let map = summarize_with(&sessions, &[wt("upper", "/repo/Foo"), wt("lower", "/repo/foo")], ident);
  assert_eq!(map.get("upper").unwrap().sessions.len(), 1);
  assert!(!map.contains_key("lower"), "no phantom match: {map:?}");
}

#[test]
fn equal_timestamps_sort_deterministically_by_kind_then_id() {
  // Codex review round Q (P2): with `ended` and mtime equal (common on
  // low-resolution filesystems), the order used to be whatever read_dir
  // produced — `top` and the JSON order could flip between two scans and
  // fake a `worktrees.changed` daemon push. kind + id break the tie.
  let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_784_000_000);
  let mk = |kind: AgentKind, id: &str| AgentSession {
    kind,
    cwd: PathBuf::from("/work/one"),
    last_activity: t,
    ended: false,
    id: id.into(),
    name: None,
  };
  let keyed = [("one".to_string(), PathBuf::from("/work/one"))];
  let shuffled = [
    mk(AgentKind::Codex, "bbb"),
    mk(AgentKind::Codex, "aaa"),
    mk(AgentKind::ClaudeCode, "zzz"),
  ];
  let reversed = [
    mk(AgentKind::ClaudeCode, "zzz"),
    mk(AgentKind::Codex, "aaa"),
    mk(AgentKind::Codex, "bbb"),
  ];
  let ids = |sessions: &[AgentSession]| -> Vec<String> {
    summarize_with(sessions, &keyed, ident)
      .get("one")
      .unwrap()
      .sessions
      .iter()
      .map(|s| s.id.clone())
      .collect()
  };
  let a = ids(&shuffled);
  assert_eq!(a, ids(&reversed), "input order never leaks into the output");
  assert_eq!(a, ["zzz", "aaa", "bbb"], "tie broken by kind (claude < codex) then id");
}

#[cfg(unix)]
#[test]
fn distinct_non_utf8_paths_never_collide() {
  // Codex review round R (P2): `to_string_lossy` in the comparison key
  // mapped DIFFERENT invalid byte sequences to the same replacement
  // character — two legitimate worktrees could share a key and a session
  // could attach to the wrong one. The key is byte-lossless now.
  use std::ffi::OsStr;
  use std::os::unix::ffi::OsStrExt;
  let wt_a = PathBuf::from("/repo").join(OsStr::from_bytes(b"proj-\xff"));
  let wt_b = PathBuf::from("/repo").join(OsStr::from_bytes(b"proj-\xfe"));
  let sessions = [AgentSession {
    kind: AgentKind::Codex,
    cwd: wt_a.clone(),
    last_activity: SystemTime::now() - Duration::from_secs(10),
    ended: false,
    id: "s1".to_string(),
    name: None,
  }];
  let keyed = [("a".to_string(), wt_a), ("b".to_string(), wt_b)];
  let map = summarize_with(&sessions, &keyed, ident);
  assert_eq!(map.get("a").map(|x| x.sessions.len()), Some(1));
  assert!(!map.contains_key("b"), "no phantom cross-attachment: {map:?}");
}

#[test]
fn path_display_key_is_exact_for_utf8_paths() {
  // The overwhelmingly common case must stay byte-identical to the old
  // lossy form: the key doubles as the JSON/TUI display string.
  assert_eq!(
    gwm::agent_sessions::path_display_key(Path::new("/work/one")),
    "/work/one"
  );
}

#[cfg(unix)]
#[test]
fn path_display_key_disambiguates_non_utf8_paths() {
  // Codex review round S (P2): the TUI snapshot/pins maps were keyed by
  // `to_string_lossy`, so two paths differing only in invalid UTF-8 bytes
  // shared one entry and a session leaked across worktrees. The key
  // appends a hash of the raw OsStr for those paths.
  use std::ffi::OsStr;
  use std::os::unix::ffi::OsStrExt;
  let a = PathBuf::from("/repo").join(OsStr::from_bytes(b"wt-\xff"));
  let b = PathBuf::from("/repo").join(OsStr::from_bytes(b"wt-\xfe"));
  let (ka, kb) = (
    gwm::agent_sessions::path_display_key(&a),
    gwm::agent_sessions::path_display_key(&b),
  );
  assert_ne!(ka, kb, "distinct invalid-byte paths never share a key");
  assert!(
    ka.starts_with(&*a.to_string_lossy()),
    "the key stays human-readable: {ka}"
  );
}

#[test]
fn summarize_drops_unmatched_sessions_without_error() {
  let sessions = [session(AgentKind::Vibe, "/somewhere/else", 10, "s1")];
  let map = summarize_with(&sessions, &[wt("one", "/work/one")], ident);
  assert!(map.is_empty());
}

#[test]
fn summarize_orders_most_recent_first_and_top_is_most_recent() {
  let sessions = [
    session(AgentKind::Codex, "/work/one", 500, "older"),
    session(AgentKind::ClaudeCode, "/work/one", 10, "newest"),
    session(AgentKind::Vibe, "/work/one", 100, "middle"),
  ];
  let map = summarize_with(&sessions, &[wt("one", "/work/one")], ident);
  let agents = map.get("one").unwrap();
  let ids: Vec<&str> = agents.sessions.iter().map(|s| s.id.as_str()).collect();
  assert_eq!(ids, ["newest", "middle", "older"]);
  assert_eq!(agents.top().unwrap().id, "newest");
  assert_eq!(agents.top().unwrap().kind, AgentKind::ClaudeCode);
}

#[test]
fn codex_scan_missing_base_is_empty() {
  let tmp = tempfile::TempDir::new().unwrap();
  assert!(CodexSource.scan(&tmp.path().join("nope"), SystemTime::now()).is_empty());
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

#[test]
fn claude_scan_matches_a_worktree_path_with_trailing_separator() {
  // libgit2 reports the main checkout with a trailing '/' (seen live on
  // `gwm list`); the slug lookup must normalise it away or the dir misses.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("projects");
  let clean = PathBuf::from("/Users/x/proj");
  let dir = base.join(claude_slug(&clean));
  write(&dir.join("aaaa-1111.jsonl"), "{}");

  let trailing = PathBuf::from("/Users/x/proj/");
  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&trailing), SystemTime::now());
  assert_eq!(sessions.len(), 1);
  // The session's cwd carries the worktree path as given (summarize
  // normalises separators itself).
  assert_eq!(sessions[0].cwd, trailing);
}

// -- Manual pins overlaying auto-detection (US4, convergence) --------------

mod pins {
  use super::*;
  use gwm::agent_sessions::detect_all;

  /// Seed a codex session whose cwd matches nothing we manage.
  fn seed_unmatched_codex(home: &Path) {
    write(
      &home
        .join(".codex/sessions")
        .join(codex_day_dir(SystemTime::now()))
        .join("rollout-x.jsonl"),
      &format!("{CODEX_META}\n"), // cwd = /work/one
    );
  }

  #[test]
  fn pin_assigns_an_unmatched_session_to_the_pinned_worktree() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path();
    seed_unmatched_codex(home);
    let wts = [wt("mine", "/repo/mine")];
    let pins = [("mine".to_string(), "019f6b95-b01a-7d30-a28a-68d9813e2248".to_string())];

    let map = detect_all(home, &wts, &pins, SystemTime::now());
    let agents = map.get("mine").expect("pin must attach the session");
    assert_eq!(agents.sessions.len(), 1);
    assert_eq!(agents.sessions[0].kind, AgentKind::Codex);
  }

  #[test]
  fn pin_with_unknown_id_is_ignored_silently() {
    let tmp = tempfile::TempDir::new().unwrap();
    let wts = [wt("mine", "/repo/mine")];
    let pins = [("mine".to_string(), "does-not-exist".to_string())];
    let map = detect_all(tmp.path(), &wts, &pins, SystemTime::now());
    assert!(map.is_empty());
  }

  #[test]
  fn pin_on_an_already_matched_session_does_not_duplicate_it() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path();
    // cwd matches the worktree — auto-detection already assigns it.
    let wts = [wt("one", "/work/one")];
    seed_unmatched_codex(home); // cwd IS /work/one here
    let pins = [("one".to_string(), "019f6b95-b01a-7d30-a28a-68d9813e2248".to_string())];
    let map = detect_all(home, &wts, &pins, SystemTime::now());
    assert_eq!(map.get("one").unwrap().sessions.len(), 1);
  }

  #[test]
  fn pinned_claude_session_outside_any_worktree_is_found_by_id_sweep() {
    // A claude project dir that matches NO managed worktree slug is normally
    // invisible (forward matching); a pin by id must still find it.
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path();
    let foreign = home.join(".claude/projects/-Users-x-somewhere-else");
    write(&foreign.join("deadbeef-cafe.jsonl"), "{}");
    let wts = [wt("mine", "/repo/mine")];
    let pins = [("mine".to_string(), "deadbeef-cafe".to_string())];

    let map = detect_all(home, &wts, &pins, SystemTime::now());
    let agents = map.get("mine").expect("id sweep must find the pinned claude session");
    assert_eq!(agents.sessions[0].kind, AgentKind::ClaudeCode);
    assert_eq!(agents.sessions[0].id, "deadbeef-cafe");
  }
}

// -- Session names (user feedback 2026-07-22) ------------------------------

mod session_names {
  use super::*;

  #[test]
  fn claude_name_comes_from_the_first_user_message() {
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path().join("projects");
    let wt = PathBuf::from("/Users/x/proj");
    let dir = base.join(claude_slug(&wt));
    let lines = concat!(
      r#"{"type":"last-prompt","leafUuid":"x"}"#,
      "\n",
      r#"{"type":"mode","mode":"normal"}"#,
      "\n",
      r#"{"type":"user","message":{"role":"user","content":"fix the login timeout bug"}}"#,
      "\n",
    );
    write(&dir.join("aaaa-1111.jsonl"), lines);
    let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
    assert_eq!(sessions[0].name.as_deref(), Some("fix the login timeout bug"));
  }

  #[test]
  fn claude_command_message_collapses_to_the_command_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path().join("projects");
    let wt = PathBuf::from("/Users/x/proj");
    let dir = base.join(claude_slug(&wt));
    let content = r#"{"type":"user","message":{"role":"user","content":"<command-message>speckit.specify</command-message>\n<command-name>/speckit.specify</command-name>\n<command-args>https://x</command-args>"}}"#;
    write(&dir.join("bbbb-2222.jsonl"), &format!("{content}\n"));
    let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
    assert_eq!(sessions[0].name.as_deref(), Some("/speckit.specify"));
  }

  #[test]
  fn codex_name_comes_from_the_first_user_message_event() {
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path().join("sessions");
    let lines = concat!(
      r#"{"timestamp":"t","type":"session_meta","payload":{"session_id":"sid-1","cwd":"/work/one"}}"#,
      "\n",
      r#"{"type":"event_msg","payload":{"type":"task_started"}}"#,
      "\n",
      r#"{"type":"event_msg","payload":{"type":"user_message","message":"review the feature flags branch"}}"#,
      "\n",
    );
    write(
      &base.join(codex_day_dir(SystemTime::now())).join("rollout-a.jsonl"),
      lines,
    );
    let sessions = CodexSource.scan(&base, SystemTime::now());
    assert_eq!(sessions[0].name.as_deref(), Some("review the feature flags branch"));
  }

  #[test]
  fn vibe_name_comes_from_the_title_field() {
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path().join("session");
    let dir = base.join(super::vibe_dir(0, "100000_aaaa1111"));
    write(
      &dir.join("meta.json"),
      r#"{"session_id":"v1","end_time":null,"title":"le plan PRO mistral","environment":{"working_directory":"/work/v"}}"#,
    );
    write(&dir.join("messages.jsonl"), "{}\n");
    let sessions = VibeSource.scan(&base, SystemTime::now());
    assert_eq!(sessions[0].name.as_deref(), Some("le plan PRO mistral"));
  }

  #[test]
  fn names_are_truncated_and_whitespace_collapsed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path().join("projects");
    let wt = PathBuf::from("/Users/x/proj");
    let dir = base.join(claude_slug(&wt));
    let long = "a ".repeat(100);
    let content = format!(r#"{{"type":"user","message":{{"role":"user","content":"{long}"}}}}"#);
    write(&dir.join("cccc-3333.jsonl"), &format!("{content}\n"));
    let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
    let name = sessions[0].name.as_deref().unwrap();
    assert!(name.chars().count() <= 60, "got {} chars", name.chars().count());
    assert!(!name.contains('\n'));
  }
}

// -- Codex review round A fixes --------------------------------------------

mod review_round_a {
  use super::*;

  #[test]
  fn top_prefers_a_live_session_over_a_more_recent_ended_one() {
    // A Vibe session that just ENDED has the freshest mtime; the still-live
    // codex session must win the compact surfaces (review finding).
    let now = SystemTime::now();
    let mk = |kind, age, ended, id: &str| AgentSession {
      kind,
      cwd: PathBuf::from("/work/one"),
      last_activity: now - Duration::from_secs(age),
      ended,
      id: id.into(),
      name: None,
    };
    let sessions = [
      mk(AgentKind::Vibe, 5, true, "ended-fresh"),
      mk(AgentKind::Codex, 60, false, "live-older"),
    ];
    let map = summarize_with(&sessions, &[wt("one", "/work/one")], |p| p.to_path_buf());
    assert_eq!(map.get("one").unwrap().top().unwrap().id, "live-older");
  }

  #[test]
  fn ambiguous_claude_slugs_are_skipped_not_duplicated() {
    // /a/b-c and /a/b/c collide on the lossy slug: assigning the same dir's
    // sessions to both worktrees would fabricate a phantom session.
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path().join("projects");
    let wt1 = PathBuf::from("/a/b-c");
    let wt2 = PathBuf::from("/a/b/c");
    assert_eq!(claude_slug(&wt1), claude_slug(&wt2));
    write(&base.join(claude_slug(&wt1)).join("dddd-4444.jsonl"), "{}");

    let sessions = ClaudeCodeSource.scan(&base, &[wt1.clone(), wt2.clone()], SystemTime::now());
    // The dir still reaches the raw pool ONCE (swept, attachable by id) —
    // but with provenance cwd only, never attributed to either worktree.
    assert_eq!(sessions.len(), 1, "swept once, not duplicated");
    assert_eq!(sessions[0].cwd, base.join(claude_slug(&wt1)));
    let keyed = [("a".to_string(), wt1), ("b".to_string(), wt2)];
    let map = gwm::agent_sessions::summarize(&sessions, &keyed);
    assert!(map.is_empty(), "no worktree claims the ambiguous dir: {map:?}");
  }
}

#[test]
fn opencode_scan_never_panics_on_an_extreme_timestamp() {
  // Codex review round B: a corrupt `time.updated` large enough to overflow
  // the platform time representation (Windows FILETIME) must skip the
  // record via checked_add, never panic in `UNIX_EPOCH + Duration`. On
  // 64-bit-seconds platforms the same value clamps to "the future", which
  // the freshness rules already treat as active — both outcomes are fine;
  // panicking is not.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join("project");
  write(
    &base.join("corrupt.json"),
    &format!(
      r#"{{"id":"corrupt","worktree":"/work/c","time":{{"created":1,"updated":{}}}}}"#,
      u64::MAX
    ),
  );
  let sessions = OpencodeSource.scan(&base, SystemTime::now());
  assert!(sessions.len() <= 1, "at most the one record, got {}", sessions.len());
}

#[test]
fn claude_live_session_name_beats_the_first_prompt() {
  // User feedback: Claude Code names live sessions in
  // ~/.claude/sessions/<pid>.json ({sessionId, name}); that name is what
  // the app shows, so it wins over the first-prompt heuristic.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join(".claude/projects");
  let wt = PathBuf::from("/Users/x/proj");
  let dir = base.join(claude_slug(&wt));
  write(
    &dir.join("aaaa-1111.jsonl"),
    "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"/speckit.install\"}}\n",
  );
  write(
    &tmp.path().join(".claude/sessions/3831.json"),
    r#"{"pid":3831,"sessionId":"aaaa-1111","name":"feat-408-agent-session-pane","status":"busy"}"#,
  );

  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
  assert_eq!(sessions[0].name.as_deref(), Some("feat-408-agent-session-pane"));
}

// --- process-level liveness (issue #441) ------------------------------------

/// Spawn-and-reap a child so its PID names a real but dead process. The
/// reuse window between `wait()` and the scan is microseconds on
/// sequential-PID kernels — accepted; the alternative is no dead-PID
/// coverage at all.
#[cfg(unix)]
fn reaped_pid() -> u32 {
  let mut child = std::process::Command::new("true").spawn().expect("spawn true");
  let pid = child.id();
  child.wait().expect("reap the child");
  pid
}

#[cfg(unix)]
#[test]
fn a_dead_registry_pid_ends_the_claude_session() {
  // An agent killed outright leaves its live-registry file behind; the
  // recorded PID is the process-level signal that it is not coming back,
  // so the session must not read as active for the rest of ACTIVE_WINDOW.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join(".claude/projects");
  let wt = PathBuf::from("/Users/x/proj");
  write(&base.join(claude_slug(&wt)).join("aaaa-1111.jsonl"), "{}");
  write(
    &tmp.path().join(".claude/sessions/40001.json"),
    &format!(
      r#"{{"pid":{},"sessionId":"aaaa-1111","name":"killed","status":"busy"}}"#,
      reaped_pid()
    ),
  );

  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
  assert!(
    sessions[0].ended,
    "a registry entry whose process is gone must end the session"
  );
}

#[test]
fn a_live_registry_pid_keeps_the_claude_session_running() {
  // Our own PID is alive by definition — the registry entry must not
  // demote the session it names.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join(".claude/projects");
  let wt = PathBuf::from("/Users/x/proj");
  write(&base.join(claude_slug(&wt)).join("aaaa-1111.jsonl"), "{}");
  write(
    &tmp.path().join(".claude/sessions/40002.json"),
    &format!(
      r#"{{"pid":{},"sessionId":"aaaa-1111","name":"alive","status":"busy"}}"#,
      std::process::id()
    ),
  );

  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
  assert!(!sessions[0].ended, "a live PID must keep the session running");
}

#[test]
fn a_registry_entry_without_a_pid_stays_artefact_only() {
  // No PID in the registry file → no process signal → the artefact-only
  // behaviour is the graceful degradation, not a demotion.
  let tmp = tempfile::TempDir::new().unwrap();
  let base = tmp.path().join(".claude/projects");
  let wt = PathBuf::from("/Users/x/proj");
  write(&base.join(claude_slug(&wt)).join("aaaa-1111.jsonl"), "{}");
  write(
    &tmp.path().join(".claude/sessions/40003.json"),
    r#"{"sessionId":"aaaa-1111","name":"no-pid","status":"busy"}"#,
  );

  let sessions = ClaudeCodeSource.scan(&base, std::slice::from_ref(&wt), SystemTime::now());
  assert!(
    !sessions[0].ended,
    "no process signal → keep the artefact-only classification"
  );
}
