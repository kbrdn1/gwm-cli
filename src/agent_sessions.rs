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

/// Extra slack behind [`SCAN_WINDOW`] for the codex day-dir prune: `codex
/// resume` appends to the ORIGINAL rollout file, so a session created up to
/// this long before the window and resumed today (fresh mtime, old dir
/// date) must still be walked. Sessions older than window + slack are
/// pruned by dir name without statting their files — the documented
/// ceiling that keeps the walk bounded on multi-year stores (Codex review
/// round I).
const RESUME_SLACK: Duration = Duration::from_secs(30 * 24 * 60 * 60);

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
  /// user prompt (Claude Code / Codex, bounded read), the recorded title
  /// (Vibe, opencode's `opencode.db`), or the tool's own registry (Claude
  /// live sessions, Codex `session_index.jsonl` — renames show). `None`
  /// when the store has nothing usable (legacy opencode JSON).
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
      if let Some(clean) = collapse_and_cap(cmd) {
        return Some(clean);
      }
    }
  }
  collapse_and_cap(raw)
}

/// Session-id hygiene at ingestion (Codex review round G): ids print raw
/// on the human surfaces (CLI listing, TUI rows), so control characters
/// are stripped — sanitising at the source keeps display, pins and JSON
/// consistent (a pin written for a sanitised id round-trips). `None` when
/// nothing printable remains.
fn clean_id(raw: &str) -> Option<String> {
  let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
  if cleaned.is_empty() {
    None
  } else {
    Some(cleaned)
  }
}

/// Bounded whole-file read: `None` when the file is missing, unreadable,
/// or larger than `cap` — an oversized artefact degrades instead of
/// exhausting the process (Codex review round G).
fn read_capped(path: &Path, cap: u64) -> Option<String> {
  use std::io::Read;
  let file = std::fs::File::open(path).ok()?;
  let mut buf = String::new();
  file.take(cap.saturating_add(1)).read_to_string(&mut buf).ok()?;
  if buf.len() as u64 > cap {
    return None;
  }
  Some(buf)
}

/// Shared name hygiene for every extraction path (first prompt, live
/// registry, `<command-name>`, Vibe title): non-whitespace control
/// characters are blanked BEFORE whitespace collapsing — artefacts are
/// untrusted input, and a raw ESC would smuggle ANSI/OSC sequences into
/// `gwm agents` output and the TUI (Codex review round D) — then the
/// result is length-capped.
fn collapse_and_cap(raw: &str) -> Option<String> {
  let stripped: String = raw
    .chars()
    .map(|c| if c.is_control() && !c.is_whitespace() { ' ' } else { c })
    .collect();
  let collapsed = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
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
    let Some(raw) = read_capped(&entry.path(), NAME_SCAN_BYTES) else {
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
  /// Full scan: slug-matched worktree dirs PLUS the unclaimed-dir sweep —
  /// the pool semantics. The sweep reads up to [`NAME_SCAN_BYTES`] of every
  /// recent foreign artefact for its name, so summary-only surfaces use
  /// [`Self::scan_matched`] instead (Codex review round F: the sweep took
  /// `gwm list` from ~0.15 s to ~1.1 s on a busy store).
  pub fn scan(&self, base: &Path, worktrees: &[PathBuf], now: SystemTime) -> Vec<AgentSession> {
    self.scan_impl(base, worktrees, now, true)
  }

  /// Matched-only scan: just the managed worktrees' slug dirs. Feeds the
  /// per-worktree summary (`gwm list`, daemon polls, TUI table), where
  /// foreign sessions can never appear anyway.
  pub fn scan_matched(&self, base: &Path, worktrees: &[PathBuf], now: SystemTime) -> Vec<AgentSession> {
    self.scan_impl(base, worktrees, now, false)
  }

  fn scan_impl(&self, base: &Path, worktrees: &[PathBuf], now: SystemTime, sweep: bool) -> Vec<AgentSession> {
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
    if !sweep {
      return out;
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
    let Some(id) = clean_id(stem) else {
      continue;
    };
    out.push(AgentSession {
      kind: AgentKind::ClaudeCode,
      cwd: cwd.to_path_buf(),
      last_activity: mtime,
      ended: false,
      id,
      name: live_names.get(stem).cloned().or_else(|| first_user_text(&path)),
    });
  }
}

/// Codex backend: `<base>/YYYY/MM/DD/rollout-*.jsonl`; the first line is a
/// `session_meta` JSON event carrying `payload.cwd`.
pub struct CodexSource;

/// Codex thread names from `~/.codex/session_index.jsonl` (user feedback
/// 2026-07-22): one `{id, thread_name, updated_at}` JSON per line,
/// append-only — a rename appends a new line, so LATER entries win.
/// Missing file or corrupt lines degrade to an empty/partial map (FR-009);
/// names are sanitised like every other extraction path.
fn codex_thread_names(sessions_base: &Path) -> std::collections::HashMap<String, String> {
  let mut map = std::collections::HashMap::new();
  let Some(index) = sessions_base.parent().map(|p| p.join("session_index.jsonl")) else {
    return map;
  };
  // 8 MiB cap: ~120 B per line, so even 10k+ sessions fit; anything
  // bigger is treated as corrupt and degrades to no names.
  let Some(contents) = read_capped(&index, 8 * 1024 * 1024) else {
    return map;
  };
  for line in contents.lines() {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
      continue;
    };
    if let (Some(id), Some(name)) = (
      v.get("id").and_then(|x| x.as_str()),
      v.get("thread_name").and_then(|x| x.as_str()),
    ) {
      if let Some(clean) = clean_session_name(name) {
        map.insert(id.to_string(), clean);
      }
    }
  }
  map
}

impl CodexSource {
  /// Full scan with names for every session — the pool semantics.
  pub fn scan(&self, base: &Path, now: SystemTime) -> Vec<AgentSession> {
    self.scan_naming(base, now, &|_, _| true)
  }

  /// `want_name(cwd, id)` gates the expensive per-rollout name extraction
  /// (`first_user_text` reads up to [`NAME_SCAN_BYTES`] each — Codex
  /// review round G): the summary surfaces only name the sessions a
  /// worktree will claim, or a pin references; foreign rollouts are
  /// dropped by `summarize` anyway. The cheap meta/index lookups still
  /// run for every rollout (cwd + id are needed for the filtering).
  fn scan_naming(&self, base: &Path, now: SystemTime, want_name: &dyn Fn(&Path, &str) -> bool) -> Vec<AgentSession> {
    let thread_names = codex_thread_names(base);
    // Day dirs older than SCAN_WINDOW + RESUME_SLACK are pruned by NAME
    // (see `codex_day_dirs`) so the walk stays bounded on multi-year
    // stores; the per-file mtime filter below still decides membership,
    // which keeps `codex resume` working — an append refreshes the file
    // mtime inside its original (possibly out-of-window, in-slack) day dir.
    let days = codex_day_dirs(base, now);
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
        let name = thread_names.get(&id).cloned().or_else(|| {
          if want_name(&cwd, &id) {
            first_user_text(&path)
          } else {
            None
          }
        });
        out.push(AgentSession {
          kind: AgentKind::Codex,
          cwd,
          last_activity: mtime,
          ended: false,
          id,
          name,
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
    // opencode ≥ 1.x migrated `storage/project/*.json` into a SQLite
    // `opencode.db` two levels up (`storage/migration` marker on disk) —
    // the stale JSON made every new session invisible (user feedback
    // 2026-07-22). The db is authoritative when present and readable; the
    // legacy JSON scan below stays as the fallback for old installs and
    // for hosts without a `sqlite3` CLI (FR-009 degradation).
    if let Some(db) = base
      .parent()
      .and_then(|p| p.parent())
      .map(|p| p.join("opencode.db"))
      .filter(|p| p.exists())
    {
      if let Some(sessions) = opencode_scan_db(&db, now) {
        return sessions;
      }
    }
    let Ok(entries) = std::fs::read_dir(base) else {
      return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().and_then(|e| e.to_str()) != Some("json") {
        continue;
      }
      let Some(raw) = read_capped(&path, NAME_SCAN_BYTES) else {
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
      let Some(id) = clean_id(id) else {
        continue;
      };
      out.push(AgentSession {
        kind: AgentKind::Opencode,
        cwd: PathBuf::from(worktree),
        last_activity,
        ended: false,
        id,
        // The opencode project index records no prompt or title.
        name: None,
      });
    }
    out
  }
}

/// Query `opencode.db` in-process via `rusqlite` (read-only). `None` = the
/// db could not be opened or queried (locked, corrupt, unexpected schema) →
/// the caller falls back to the legacy JSON layout. `Some(vec)` — even
/// empty — is authoritative. In-process because Windows rarely ships a
/// `sqlite3` CLI: shelling out silently hid every opencode ≥ 1.x session
/// there (Codex review round I).
fn opencode_scan_db(db: &Path, now: SystemTime) -> Option<Vec<AgentSession>> {
  let cutoff_ms = now
    .duration_since(SystemTime::UNIX_EPOCH)
    .ok()?
    .saturating_sub(SCAN_WINDOW)
    .as_millis() as i64;
  let conn = rusqlite::Connection::open_with_flags(
    db,
    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
  )
  .ok()?;
  // Top-level sessions only: children (`parent_id`) are subagent runs.
  let mut stmt = conn
    .prepare(
      "SELECT id, directory, title, time_updated, time_archived FROM session \
       WHERE parent_id IS NULL AND time_updated >= ?1",
    )
    .ok()?;
  let rows = stmt
    .query_map([cutoff_ms], |r| {
      Ok((
        r.get::<_, String>(0)?,
        r.get::<_, String>(1)?,
        r.get::<_, Option<String>>(2)?,
        r.get::<_, i64>(3)?,
        !matches!(r.get_ref(4)?, rusqlite::types::ValueRef::Null),
      ))
    })
    .ok()?;
  Some(
    rows
      .filter_map(|row| {
        let (id, dir, title, ms, ended) = row.ok()?;
        let id = clean_id(&id)?;
        let last_activity = SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(u64::try_from(ms).ok()?))?;
        Some(AgentSession {
          kind: AgentKind::Opencode,
          cwd: PathBuf::from(dir),
          last_activity,
          ended,
          id,
          name: title.as_deref().and_then(clean_session_name),
        })
      })
      .collect(),
  )
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
      let Some(raw) = read_capped(&meta_path, NAME_SCAN_BYTES) else {
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
      // Same id hygiene as every other backend (Codex review round H):
      // both the recorded id and the dir-name fallback go through clean_id.
      let Some(id) = v
        .get("session_id")
        .and_then(|s| s.as_str())
        .and_then(clean_id)
        .or_else(|| clean_id(&entry.file_name().to_string_lossy()))
      else {
        continue;
      };
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
  use std::io::{BufRead, Read};
  // Bounded read (Codex review round G): `read_line` allocates the WHOLE
  // line, so a corrupt artefact with a giant first line could exhaust the
  // process. A meta line is <1 KiB in practice; anything without a newline
  // inside the cap is skipped (FR-009 degradation).
  let file = std::fs::File::open(path).ok()?;
  let mut line = String::new();
  std::io::BufReader::new(file.take(NAME_SCAN_BYTES))
    .read_line(&mut line)
    .ok()?;
  if !line.ends_with('\n') && line.len() as u64 >= NAME_SCAN_BYTES {
    return None;
  }
  let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
  let payload = v.get("payload")?;
  let cwd = payload.get("cwd")?.as_str()?;
  if cwd.is_empty() {
    return None;
  }
  // `session_id` is the historical field; Codex 0.138+ writes `id`
  // (review round F). The file stem is the last-resort fallback only.
  let id = payload
    .get("session_id")
    .or_else(|| payload.get("id"))
    .and_then(|s| s.as_str())
    .and_then(clean_id)
    .or_else(|| path.file_stem().and_then(|s| clean_id(&s.to_string_lossy())))?;
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

/// Civil (proleptic Gregorian) date for a count of days since 1970-01-01.
/// Howard Hinnant's `civil_from_days` — exact over the whole i64 range we
/// can meet here.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
  let z = z + 719_468;
  let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
  let doe = z - era * 146_097; // [0, 146096]
  let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
  let mp = (5 * doy + 2) / 153; // [0, 11]
  let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
  let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
  let y = yoe + era * 400 + i64::from(m <= 2);
  (y, m, d)
}

/// Civil date of `t` (UTC), or `None` before the epoch.
fn civil_date(t: SystemTime) -> Option<(i64, u32, u32)> {
  let days = t.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs() / 86_400;
  Some(civil_from_days(days as i64))
}

/// The `YYYY/MM/DD` day-dir fragment codex files a rollout under for the
/// day of `t` (UTC — codex may use the local date, a ±1 day drift the
/// 30-day [`RESUME_SLACK`] absorbs). Public so test fixtures build their
/// store with the exact layout the pruned walk expects.
pub fn codex_day_dir(t: SystemTime) -> PathBuf {
  let (y, m, d) = civil_date(t).unwrap_or((1970, 1, 1));
  PathBuf::from(format!("{y:04}/{m:02}/{d:02}"))
}

/// Numeric value of a path's file name (`"07"` → 7), `None` when the dir is
/// not date-shaped — such dirs are conservatively kept and walked.
fn dir_num(p: &Path) -> Option<i64> {
  p.file_name()?.to_str()?.parse().ok()
}

/// The codex `YYYY/MM/DD` day dirs under `base` worth walking: dirs whose
/// name-date is older than `SCAN_WINDOW + RESUME_SLACK` are pruned without
/// reading them, so the walk stays bounded on multi-year stores. Comparison
/// is per-level and only when the components parse — anything non-numeric
/// is kept (defensive against layout drift).
fn codex_day_dirs(base: &Path, now: SystemTime) -> Vec<PathBuf> {
  let cutoff = civil_date(
    now
      .checked_sub(SCAN_WINDOW + RESUME_SLACK)
      .unwrap_or(SystemTime::UNIX_EPOCH),
  );
  let Some((cy, cm, cd)) = cutoff else {
    return subdirs_flat(&subdirs_flat(&subdirs_flat(&[base.to_path_buf()])));
  };
  let mut days = Vec::new();
  for ydir in subdirs_flat(&[base.to_path_buf()]) {
    let y = dir_num(&ydir);
    if y.is_some_and(|y| y < cy) {
      continue;
    }
    for mdir in subdirs_flat(&[ydir]) {
      let m = dir_num(&mdir);
      if y.zip(m).is_some_and(|(y, m)| (y, m) < (cy, i64::from(cm))) {
        continue;
      }
      for ddir in subdirs_flat(&[mdir]) {
        let d = dir_num(&ddir);
        if y
          .zip(m)
          .zip(d)
          .is_some_and(|((y, m), d)| (y, m, d) < (cy, i64::from(cm), i64::from(cd)))
        {
          continue;
        }
        days.push(ddir);
      }
    }
  }
  days
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
/// iteration; the comparison stays case-EXACT on every platform (Codex
/// review round F). Case handling belongs to the canonicalizer: on a
/// case-insensitive volume `canonicalize` converges both sides to the
/// on-disk casing, so exact comparison still matches — while a platform-wide
/// fold would MERGE two genuinely distinct worktrees on a case-sensitive
/// APFS/NTFS volume, fabricating a session match.
fn comparison_key(path: &Path) -> String {
  path
    .components()
    .map(|c| c.as_os_str().to_string_lossy())
    .collect::<Vec<_>>()
    .join("\u{1f}")
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

/// Production entry point for the SUMMARY surfaces (`gwm list`, daemon
/// polls, the TUI table): resolve the four artefact roots under `home`, run
/// every backend **matched-only** (no foreign-dir sweep — Codex review
/// round F: reading every recent foreign artefact's name took `gwm list`
/// from ~0.15 s to ~1.1 s on a busy store), summarize per worktree, then
/// overlay the manual `pins` — a foreign pinned Claude session resolves
/// through the targeted by-id sweep, so pins lose nothing. Pure given its
/// inputs — production passes [`agents_home`], tests a seeded `TempDir`.
pub fn detect_all(
  home: &Path,
  worktrees: &[(String, PathBuf)],
  pins: &[(String, String)],
  now: SystemTime,
) -> std::collections::BTreeMap<String, WorktreeAgents> {
  let paths: Vec<PathBuf> = worktrees.iter().map(|(_, p)| p.clone()).collect();
  let pinned_ids: std::collections::BTreeSet<&str> = pins.iter().map(|(_, sid)| sid.as_str()).collect();
  let mut sessions = collect_with(home, &paths, now, false, &pinned_ids);
  sessions.sort_by_key(|s| (s.ended, std::cmp::Reverse(s.last_activity)));
  let mut map = summarize(&sessions, worktrees);
  overlay_pins(&mut map, &sessions, pins, home, now);
  map
}

/// Every session the four backends can see right now — the raw pool behind
/// [`detect_with_sessions`], consumed by the TUI's attach-by-id prompt and
/// `gwm agents`' unmatched section (a session matched to no worktree is
/// exactly the one worth pinning manually). Includes the Claude
/// foreign-dir sweep — the bounded name reads are the price of the pool.
pub fn collect_sessions(home: &Path, worktree_paths: &[PathBuf], now: SystemTime) -> Vec<AgentSession> {
  collect_with(home, worktree_paths, now, true, &std::collections::BTreeSet::new())
}

fn collect_with(
  home: &Path,
  worktree_paths: &[PathBuf],
  now: SystemTime,
  sweep: bool,
  pinned_ids: &std::collections::BTreeSet<&str>,
) -> Vec<AgentSession> {
  let claude = ClaudeCodeSource;
  let base = home.join(".claude/projects");
  let mut sessions = if sweep {
    claude.scan(&base, worktree_paths, now)
  } else {
    claude.scan_matched(&base, worktree_paths, now)
  };
  let codex_base = home.join(".codex/sessions");
  if sweep {
    sessions.extend(CodexSource.scan(&codex_base, now));
  } else {
    // Summary surfaces: only name the rollouts a worktree will claim or a
    // pin references (Codex review round G) — same canonical comparison as
    // `summarize`, so an eager skip can't drop a name summarize would show.
    let keys: std::collections::BTreeSet<String> = worktree_paths
      .iter()
      .map(|p| comparison_key(&p.canonicalize().unwrap_or_else(|_| p.to_path_buf())))
      .collect();
    let want = |cwd: &Path, id: &str| {
      pinned_ids.contains(id)
        || keys.contains(&comparison_key(
          &cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf()),
        ))
    };
    sessions.extend(CodexSource.scan_naming(&codex_base, now, &want));
  }
  // opencode's own cross-platform convention is home-relative .local/share
  // (research.md D4), so no per-OS data-dir split here.
  sessions.extend(OpencodeSource.scan(&home.join(".local/share/opencode/storage/project"), now));
  sessions.extend(VibeSource.scan(&home.join(".vibe/logs/session"), now));
  sessions
}

/// [`detect_all`] variant returning the per-worktree summary AND the raw
/// session pool from one single scan pass — the pool surfaces (TUI worker,
/// `gwm agents`). This is the path that pays for the foreign-dir sweep.
pub fn detect_with_sessions(
  home: &Path,
  worktrees: &[(String, PathBuf)],
  pins: &[(String, String)],
  now: SystemTime,
) -> (std::collections::BTreeMap<String, WorktreeAgents>, Vec<AgentSession>) {
  let paths: Vec<PathBuf> = worktrees.iter().map(|(_, p)| p.clone()).collect();
  let mut sessions = collect_sessions(home, &paths, now);
  // Live-first pool (user feedback 2026-07-22): every consumer of the raw
  // pool — the TUI attach-by-id prompt, `gwm agents`' unmatched section —
  // must offer active sessions first, not backend concatenation order.
  sessions.sort_by_key(|s| (s.ended, std::cmp::Reverse(s.last_activity)));
  let mut map = summarize(&sessions, worktrees);
  overlay_pins(&mut map, &sessions, pins, home, now);
  (map, sessions)
}

/// Overlay manual pins onto a summary (shared by both detection entries).
fn overlay_pins(
  map: &mut std::collections::BTreeMap<String, WorktreeAgents>,
  sessions: &[AgentSession],
  pins: &[(String, String)],
  home: &Path,
  now: SystemTime,
) {
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
      id: clean_id(sid)?,
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
