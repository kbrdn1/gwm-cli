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

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long after its last artefact activity a session still counts as active.
pub const ACTIVE_WINDOW: Duration = Duration::from_secs(300);

/// Recency window bounding every artefact scan: sessions whose last activity
/// is older than this are not harvested at all, so detection cost and result
/// size stay independent of years of accumulated history (spec edge case
/// "artefact store is huge").
pub const SCAN_WINDOW: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// A supported coding agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentKind {
  ClaudeCode,
  Codex,
  Opencode,
  Vibe,
}

impl AgentKind {
  /// Stable lowercase name — the string frozen into the JSON contract.
  pub fn display(&self) -> &'static str {
    match self {
      AgentKind::ClaudeCode => "claude",
      AgentKind::Codex => "codex",
      AgentKind::Opencode => "opencode",
      AgentKind::Vibe => "vibe",
    }
  }
}

/// One detected session, before worktree matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
  pub kind: AgentKind,
  /// Recorded working directory (for Claude Code: the matched worktree path,
  /// since the slug mapping is lossy and only matches forward).
  pub cwd: PathBuf,
  pub last_activity: SystemTime,
  /// Only Vibe can observe a terminated session (non-null `end_time`).
  pub ended: bool,
  pub id: String,
  /// Human-readable session name when the artefacts carry one: the first
  /// user prompt (Claude Code / Codex, bounded read) or the recorded title
  /// (Vibe). `None` when the store has nothing usable (opencode).
  pub name: Option<String>,
}

/// Longest session name surfaces display before truncation.
const NAME_MAX_CHARS: usize = 60;
/// Bound on how much of a session artefact is read to find its name.
const NAME_SCAN_BYTES: u64 = 64 * 1024;

/// Collapse whitespace, resolve Claude's `<command-message>` envelopes to the
/// command name, and truncate — the shape every surface displays.
fn clean_session_name(raw: &str) -> Option<String> {
  // A slash-command prompt is XML-ish noise; its `<command-name>` is the name.
  if let Some(rest) = raw.split("<command-name>").nth(1) {
    if let Some(cmd) = rest.split("</command-name>").next() {
      let cmd = cmd.trim();
      if !cmd.is_empty() {
        return Some(cmd.to_string());
      }
    }
  }
  let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
  if collapsed.is_empty() {
    return None;
  }
  Some(collapsed.chars().take(NAME_MAX_CHARS).collect())
}

/// First user-prompt text found in the opening lines of a session artefact
/// (Claude project jsonl or Codex rollout), reading at most
/// [`NAME_SCAN_BYTES`]. Total: any miss or parse failure yields `None`.
fn first_user_text(path: &Path) -> Option<String> {
  use std::io::{BufRead, Read};
  let file = std::fs::File::open(path).ok()?;
  let mut reader = std::io::BufReader::new(file.take(NAME_SCAN_BYTES));
  let mut line = String::new();
  loop {
    line.clear();
    let n = reader.read_line(&mut line).ok()?;
    if n == 0 {
      return None;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
      continue;
    };
    // Claude Code: {"type":"user","message":{"content": <str | [{text}]>}}
    if v.get("type").and_then(|t| t.as_str()) == Some("user") {
      let content = v.get("message").and_then(|m| m.get("content"));
      let text = match content {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Array(parts)) => parts.iter().find_map(|p| {
          (p.get("type").and_then(|t| t.as_str()) == Some("text"))
            .then(|| p.get("text").and_then(|t| t.as_str()).map(str::to_string))
            .flatten()
        }),
        _ => None,
      };
      if let Some(t) = text.as_deref().and_then(clean_session_name) {
        return Some(t);
      }
    }
    // Codex: {"type":"event_msg","payload":{"type":"user_message","message":"…"}}
    if let Some(p) = v.get("payload") {
      if p.get("type").and_then(|t| t.as_str()) == Some("user_message") {
        if let Some(t) = p.get("message").and_then(|m| m.as_str()).and_then(clean_session_name) {
          return Some(t);
        }
      }
    }
  }
}

/// Claude Code backend: `<base>/<slug(worktree)>/**.jsonl`, one session per
/// `.jsonl` file. The slug is lossy, so the scan takes the managed worktree
/// paths and looks their slugs up — O(#worktrees), no directory sweep.
pub struct ClaudeCodeSource;

/// Live-session names from `<.claude>/sessions/*.json` — Claude Code's own
/// registry of running sessions (`{sessionId, name, status}`). The name it
/// carries is what the app displays, so it beats the first-prompt heuristic
/// (user feedback 2026-07-22). Missing dir → empty map (dead sessions fall
/// back to their first prompt).
fn claude_live_names(projects_base: &Path) -> std::collections::HashMap<String, String> {
  let mut map = std::collections::HashMap::new();
  let Some(sessions_dir) = projects_base.parent().map(|p| p.join("sessions")) else {
    return map;
  };
  let Ok(entries) = std::fs::read_dir(sessions_dir) else {
    return map;
  };
  for entry in entries.flatten() {
    let Ok(raw) = std::fs::read_to_string(entry.path()) else {
      continue;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
      continue;
    };
    if let (Some(sid), Some(name)) = (
      v.get("sessionId").and_then(|x| x.as_str()),
      v.get("name").and_then(|x| x.as_str()),
    ) {
      if let Some(clean) = clean_session_name(name) {
        map.insert(sid.to_string(), clean);
      }
    }
  }
  map
}

impl ClaudeCodeSource {
  pub fn scan(&self, base: &Path, worktrees: &[PathBuf], now: SystemTime) -> Vec<AgentSession> {
    let live_names = claude_live_names(base);
    // Normalise before slugging: libgit2 reports the main checkout with a
    // trailing '/', which would grow a trailing '-' the recorded cwd never
    // has. `components()` drops redundant separators lexically.
    let slugs: Vec<String> = worktrees
      .iter()
      .map(|wt| claude_slug(&wt.components().collect::<PathBuf>()))
      .collect();
    let mut out = Vec::new();
    let mut claimed: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for (wt, slug) in worktrees.iter().zip(&slugs) {
      // The slug is lossy: /a/b-c and /a/b/c collide. Assigning one dir's
      // sessions to both worktrees would fabricate a phantom session, so an
      // ambiguous slug is skipped outright (FR-009 degradation) — its dir
      // is swept below instead, where sessions claim no worktree.
      if slugs.iter().filter(|s| *s == slug).count() > 1 {
        continue;
      }
      claimed.insert(slug.as_str());
      scan_claude_dir(&base.join(slug), wt, &live_names, now, &mut out);
    }
    // Sweep the project dirs no managed worktree claimed (Codex review
    // round C): a session launched in another repo, a subdirectory, or an
    // old path must still reach the raw pool — the attach-by-id prompt
    // exists precisely for these. The lossy slug cannot be reversed to a
    // cwd, so the dir path rides `cwd` purely as provenance; it can never
    // forward-match a worktree in `summarize`.
    if let Ok(entries) = std::fs::read_dir(base) {
      for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
          continue; // sessions/registry files at the base level
        }
        let name = entry.file_name();
        if claimed.contains(name.to_string_lossy().as_ref()) {
          continue;
        }
        scan_claude_dir(&dir, &dir, &live_names, now, &mut out);
      }
    }
    out
  }
}

/// One Claude project dir → sessions. `cwd` is the worktree for a
/// slug-matched dir, or the dir itself (provenance only) for a swept one.
fn scan_claude_dir(
  dir: &Path,
  cwd: &Path,
  live_names: &std::collections::HashMap<String, String>,
  now: SystemTime,
  out: &mut Vec<AgentSession>,
) {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return; // unmatched worktree or missing base: no sessions (FR-009)
  };
  for entry in entries.flatten() {
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
      continue; // real dirs hold a `memory/` subdir and other non-session files
    }
    let Some(mtime) = file_mtime(&path) else {
      continue;
    };
    if !within_scan_window(mtime, now) {
      continue;
    }
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
      continue;
    };
    out.push(AgentSession {
      kind: AgentKind::ClaudeCode,
      cwd: cwd.to_path_buf(),
      last_activity: mtime,
      ended: false,
      id: stem.to_string(),
      name: live_names.get(stem).cloned().or_else(|| first_user_text(&path)),
    });
  }
}

/// Codex backend: `<base>/YYYY/MM/DD/rollout-*.jsonl`; the first line is a
/// `session_meta` JSON event carrying `payload.cwd`.
pub struct CodexSource;

impl CodexSource {
  pub fn scan(&self, base: &Path, now: SystemTime) -> Vec<AgentSession> {
    // ponytail: full YYYY/MM/DD walk + per-file mtime filter instead of
    // date-name pruning — a rollout appended today in a 40-day-old day dir
    // must still be found (appends touch the file mtime, not the dir's).
    // Switch to name-based pruning only if real stores make this walk slow.
    let years = subdirs_flat(&[base.to_path_buf()]);
    let months = subdirs_flat(&years);
    let days = subdirs_flat(&months);
    let mut out = Vec::new();
    for day_dir in days {
      let Ok(entries) = std::fs::read_dir(&day_dir) else {
        continue;
      };
      for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("rollout-") || path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
          continue; // legacy .json (real 2025 data) and strangers are skipped
        }
        let Some(mtime) = file_mtime(&path) else {
          continue;
        };
        if !within_scan_window(mtime, now) {
          continue;
        }
        let Some((cwd, id)) = codex_first_line_meta(&path) else {
          continue; // malformed first line: skip, never hide the others
        };
        out.push(AgentSession {
          kind: AgentKind::Codex,
          cwd,
          last_activity: mtime,
          ended: false,
          id,
          name: first_user_text(&path),
        });
      }
    }
    out
  }
}

/// opencode backend: `<base>/*.json`, one per project ever opened; `worktree`
/// field carries the path, `time.updated` (epoch ms) the last activity.
pub struct OpencodeSource;

impl OpencodeSource {
  pub fn scan(&self, base: &Path, now: SystemTime) -> Vec<AgentSession> {
    let Ok(entries) = std::fs::read_dir(base) else {
      return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().and_then(|e| e.to_str()) != Some("json") {
        continue;
      }
      let Ok(raw) = std::fs::read_to_string(&path) else {
        continue;
      };
      let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        continue;
      };
      let id = v.get("id").and_then(|s| s.as_str()).unwrap_or_default();
      let worktree = v.get("worktree").and_then(|s| s.as_str()).unwrap_or_default();
      if id.is_empty() || id == "global" || worktree.is_empty() {
        continue; // global.json (worktree "/") is the store's own pseudo-entry
      }
      // Prefer the agent's own record over fs metadata: updated → created → mtime.
      let recorded_ms = v
        .get("time")
        .and_then(|t| t.get("updated").or_else(|| t.get("created")))
        .and_then(|n| n.as_u64());
      // checked_add: a corrupt epoch-ms large enough to overflow the
      // platform time representation must skip the record, not panic
      // (Codex review round B — FILETIME on Windows overflows first).
      let last_activity = match recorded_ms {
        Some(ms) => match SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(ms)) {
          Some(t) => t,
          None => continue,
        },
        None => match file_mtime(&path) {
          Some(t) => t,
          None => continue,
        },
      };
      if !within_scan_window(last_activity, now) {
        continue;
      }
      out.push(AgentSession {
        kind: AgentKind::Opencode,
        cwd: PathBuf::from(worktree),
        last_activity,
        ended: false,
        id: id.to_string(),
        // The opencode project index records no prompt or title.
        name: None,
      });
    }
    out
  }
}

/// Mistral Vibe backend: `<base>/session_<ts>_<id>/meta.json` carries
/// `environment.working_directory`; a non-null `end_time` marks the session
/// terminated (the one backend with a real liveness signal).
pub struct VibeSource;

impl VibeSource {
  pub fn scan(&self, base: &Path, now: SystemTime) -> Vec<AgentSession> {
    let Ok(entries) = std::fs::read_dir(base) else {
      return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
      let dir = entry.path();
      if !dir.is_dir() || !entry.file_name().to_string_lossy().starts_with("session_") {
        continue;
      }
      let meta_path = dir.join("meta.json");
      // Recency gate BEFORE reading/parsing any content (Codex review
      // round B): with years of sessions, opening every meta.json made the
      // scan linear in the whole history despite the 30-day bound.
      let Some(last_activity) = file_mtime(&dir.join("messages.jsonl")).or_else(|| file_mtime(&meta_path)) else {
        continue;
      };
      if !within_scan_window(last_activity, now) {
        continue;
      }
      let Ok(raw) = std::fs::read_to_string(&meta_path) else {
        continue;
      };
      let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        continue; // malformed meta.json: skip, never hide the others
      };
      let cwd = v
        .get("environment")
        .and_then(|e| e.get("working_directory"))
        .and_then(|s| s.as_str())
        .unwrap_or_default();
      if cwd.is_empty() {
        continue;
      }
      // Timestamps in meta.json appear with AND without a tz suffix across
      // Vibe versions (research.md D5) — never string-parse them. Activity
      // comes from messages.jsonl mtime (meta.json mtime as fallback);
      // end_time is only ever inspected for null-ness.
      let ended = v.get("end_time").is_some_and(|t| !t.is_null());
      let id = v
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
      out.push(AgentSession {
        kind: AgentKind::Vibe,
        cwd: PathBuf::from(cwd),
        last_activity,
        ended,
        id,
        name: v.get("title").and_then(|t| t.as_str()).and_then(clean_session_name),
      });
    }
    out
  }
}

/// Common face of the four backends — one artefact scheme each, all total
/// (missing/corrupt input degrades to an empty result, never an error).
/// `worktrees` is only consumed by Claude Code, whose slug mapping is lossy
/// and can only be matched forward from the managed worktree paths.
pub trait SessionSource {
  fn kind(&self) -> AgentKind;
  fn sessions(&self, base: &Path, worktrees: &[PathBuf], now: SystemTime) -> Vec<AgentSession>;
}

impl SessionSource for ClaudeCodeSource {
  fn kind(&self) -> AgentKind {
    AgentKind::ClaudeCode
  }
  fn sessions(&self, base: &Path, worktrees: &[PathBuf], now: SystemTime) -> Vec<AgentSession> {
    self.scan(base, worktrees, now)
  }
}

impl SessionSource for CodexSource {
  fn kind(&self) -> AgentKind {
    AgentKind::Codex
  }
  fn sessions(&self, base: &Path, _worktrees: &[PathBuf], now: SystemTime) -> Vec<AgentSession> {
    self.scan(base, now)
  }
}

impl SessionSource for OpencodeSource {
  fn kind(&self) -> AgentKind {
    AgentKind::Opencode
  }
  fn sessions(&self, base: &Path, _worktrees: &[PathBuf], now: SystemTime) -> Vec<AgentSession> {
    self.scan(base, now)
  }
}

impl SessionSource for VibeSource {
  fn kind(&self) -> AgentKind {
    AgentKind::Vibe
  }
  fn sessions(&self, base: &Path, _worktrees: &[PathBuf], now: SystemTime) -> Vec<AgentSession> {
    self.scan(base, now)
  }
}

/// Parse the first line of a rollout file: `payload.cwd` (+ session id).
/// Reads one line only — rollout files grow large.
fn codex_first_line_meta(path: &Path) -> Option<(PathBuf, String)> {
  use std::io::BufRead;
  let file = std::fs::File::open(path).ok()?;
  let mut line = String::new();
  std::io::BufReader::new(file).read_line(&mut line).ok()?;
  let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
  let payload = v.get("payload")?;
  let cwd = payload.get("cwd")?.as_str()?;
  if cwd.is_empty() {
    return None;
  }
  let id = payload
    .get("session_id")
    .and_then(|s| s.as_str())
    .map(str::to_string)
    .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))?;
  Some((PathBuf::from(cwd), id))
}

/// Child directories of every path in `dirs` (unreadable parents skipped).
fn subdirs_flat(dirs: &[PathBuf]) -> Vec<PathBuf> {
  let mut out = Vec::new();
  for dir in dirs {
    let Ok(entries) = std::fs::read_dir(dir) else {
      continue;
    };
    for entry in entries.flatten() {
      let path = entry.path();
      if path.is_dir() {
        out.push(path);
      }
    }
  }
  out
}

/// All sessions matched to one worktree, most recent first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorktreeAgents {
  pub sessions: Vec<AgentSession>,
}

impl WorktreeAgents {
  /// The most recently active session — what compact surfaces display.
  pub fn top(&self) -> Option<&AgentSession> {
    self.sessions.first()
  }
}

/// Match sessions to worktrees through an injected canonicalizer (same seam
/// as `statusline::active_index_with`: both sides are canonicalised, tests
/// pass a deterministic stub, production passes fs canonicalisation with a
/// lexical fallback for paths that no longer exist).
pub fn summarize_with<F>(
  sessions: &[AgentSession],
  worktrees: &[(String, std::path::PathBuf)],
  canonicalize: F,
) -> std::collections::BTreeMap<String, WorktreeAgents>
where
  F: Fn(&Path) -> PathBuf,
{
  let keyed: Vec<(&String, String)> = worktrees
    .iter()
    .map(|(id, path)| (id, comparison_key(&canonicalize(path))))
    .collect();
  let mut map = std::collections::BTreeMap::<String, WorktreeAgents>::new();
  for s in sessions {
    let skey = comparison_key(&canonicalize(&s.cwd));
    for (id, wkey) in &keyed {
      if &skey == wkey {
        map.entry((*id).clone()).or_default().sessions.push(s.clone());
      }
    }
  }
  for agents in map.values_mut() {
    agents
      .sessions
      .sort_by_key(|s| (s.ended, std::cmp::Reverse(s.last_activity)));
  }
  map
}

/// Path comparison key: trailing separators are normalised away by component
/// iteration; case folds on the platforms whose filesystems are
/// case-insensitive by default (Windows, macOS), stays exact on Linux.
fn comparison_key(path: &Path) -> String {
  let joined = path
    .components()
    .map(|c| c.as_os_str().to_string_lossy())
    .collect::<Vec<_>>()
    .join("\u{1f}");
  #[cfg(any(windows, target_os = "macos"))]
  {
    joined.to_lowercase()
  }
  #[cfg(not(any(windows, target_os = "macos")))]
  {
    joined
  }
}

/// Production entry: fs canonicalisation with lexical fallback (a session may
/// reference a worktree that was since removed — it must simply not match).
pub fn summarize(
  sessions: &[AgentSession],
  worktrees: &[(String, std::path::PathBuf)],
) -> std::collections::BTreeMap<String, WorktreeAgents> {
  summarize_with(sessions, worktrees, |p| {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
  })
}

/// The artefact-root base for production surfaces: `GWM_AGENTS_HOME` when set
/// (a deterministic seam for tests and CI), else the real home directory.
pub fn agents_home() -> Option<PathBuf> {
  std::env::var_os("GWM_AGENTS_HOME")
    .map(PathBuf::from)
    .or_else(dirs::home_dir)
}

/// Production entry point: resolve the four artefact roots under `home`, run
/// every backend, summarize per worktree, then overlay the manual `pins`
/// (`(worktree id, session id)` pairs — auto-detection stays the default,
/// a pin only *adds* the named session to the named worktree). Pure given
/// its inputs — production passes [`agents_home`], tests a seeded `TempDir`.
pub fn detect_all(
  home: &Path,
  worktrees: &[(String, PathBuf)],
  pins: &[(String, String)],
  now: SystemTime,
) -> std::collections::BTreeMap<String, WorktreeAgents> {
  detect_with_sessions(home, worktrees, pins, now).0
}

/// Every session the four backends can see right now — the raw pool behind
/// [`detect_all`], also consumed by the TUI's attach-by-id prompt (a session
/// matched to no worktree is exactly the one worth pinning manually).
pub fn collect_sessions(home: &Path, worktree_paths: &[PathBuf], now: SystemTime) -> Vec<AgentSession> {
  let mut sessions = ClaudeCodeSource.scan(&home.join(".claude/projects"), worktree_paths, now);
  sessions.extend(CodexSource.scan(&home.join(".codex/sessions"), now));
  // opencode's own cross-platform convention is home-relative .local/share
  // (research.md D4), so no per-OS data-dir split here.
  sessions.extend(OpencodeSource.scan(&home.join(".local/share/opencode/storage/project"), now));
  sessions.extend(VibeSource.scan(&home.join(".vibe/logs/session"), now));
  sessions
}

/// [`detect_all`] variant returning the per-worktree summary AND the raw
/// session pool from one single scan pass (the TUI worker needs both).
pub fn detect_with_sessions(
  home: &Path,
  worktrees: &[(String, PathBuf)],
  pins: &[(String, String)],
  now: SystemTime,
) -> (std::collections::BTreeMap<String, WorktreeAgents>, Vec<AgentSession>) {
  let paths: Vec<PathBuf> = worktrees.iter().map(|(_, p)| p.clone()).collect();
  let sessions = collect_sessions(home, &paths, now);
  let mut map = summarize(&sessions, worktrees);

  for (wt_id, sid) in pins {
    // Resolve the pinned session: usually already collected; a Claude
    // session whose project dir matches no managed worktree is invisible to
    // forward matching, so fall back to an id sweep of the project dirs.
    let found = sessions
      .iter()
      .find(|s| &s.id == sid)
      .cloned()
      .or_else(|| claude_session_by_id(&home.join(".claude/projects"), sid, now));
    let Some(session) = found else {
      continue; // unknown id: a stale pin degrades silently (FR-009 spirit)
    };
    let agents = map.entry(wt_id.clone()).or_default();
    if !agents.sessions.iter().any(|s| &s.id == sid) {
      agents.sessions.push(session);
      agents
        .sessions
        .sort_by_key(|s| (s.ended, std::cmp::Reverse(s.last_activity)));
    }
  }
  (map, sessions)
}

/// Id sweep over every Claude project dir — only reached for a pin whose id
/// no cwd-matched scan produced. The recorded cwd is unrecoverable from the
/// lossy slug, which is fine: a pinned session's assignment comes from the
/// pin, so `cwd` carries the slug dir path purely as provenance.
fn claude_session_by_id(base: &Path, sid: &str, now: SystemTime) -> Option<AgentSession> {
  let live_names = claude_live_names(base);
  let entries = std::fs::read_dir(base).ok()?;
  for dir in entries.flatten() {
    let path = dir.path().join(format!("{sid}.jsonl"));
    let Some(mtime) = file_mtime(&path) else {
      continue;
    };
    if !within_scan_window(mtime, now) {
      continue;
    }
    return Some(AgentSession {
      kind: AgentKind::ClaudeCode,
      cwd: dir.path(),
      last_activity: mtime,
      ended: false,
      id: sid.to_string(),
      name: live_names.get(sid).cloned().or_else(|| first_user_text(&path)),
    });
  }
  None
}

/// mtime of a file, or `None` when unreadable (degrade, don't error).
fn file_mtime(path: &Path) -> Option<SystemTime> {
  std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Inside the scan recency window? Future timestamps count as "now" (skew).
fn within_scan_window(t: SystemTime, now: SystemTime) -> bool {
  now.duration_since(t).unwrap_or(Duration::ZERO) <= SCAN_WINDOW
}

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
    let elapsed = now.duration_since(last_activity).unwrap_or(Duration::ZERO);
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
