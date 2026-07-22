//! Stable, machine-readable JSON surface shared by the `--format=json`
//! CLI flags (issue #38, phase 1) and the daemon's JSON-RPC methods
//! (phase 2).
//!
//! The DTOs here are deliberately decoupled from the internal
//! [`crate::worktree::WorktreeInfo`] / [`crate::doctor::DoctorReport`]
//! types. Those structs carry TUI-runtime baggage (loaded GitHub issue /
//! PR state, cached branch age as a `Duration`, the `BranchLink` graph)
//! whose shape churns as the TUI evolves. Pinning the documented schema
//! (see `docs/schema/`) to a dedicated set of `Serialize` DTOs means a
//! refactor of `WorktreeInfo` can't silently break a downstream editor
//! plugin. Conversions are one-directional (`From<&Internal>`); the JSON
//! surface is output-only.
//!
//! Key convention: `snake_case`, matching the hand-built
//! `print_status_json` in [`crate::cli`].

use crate::doctor::{CheckStatus, DoctorReport};
use crate::error::Result;
use crate::worktree::{self, BranchStatus, WorktreeInfo};
use serde::{Deserialize, Serialize};

/// Working-tree + upstream status, the stable projection of
/// [`BranchStatus`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonStatus {
  pub is_dirty: bool,
  pub has_upstream: bool,
  pub ahead: usize,
  pub behind: usize,
  /// Status couldn't be computed (detached HEAD, unborn branch).
  pub unknown: bool,
}

impl From<&BranchStatus> for JsonStatus {
  fn from(s: &BranchStatus) -> Self {
    Self {
      is_dirty: s.is_dirty,
      has_upstream: s.has_upstream,
      ahead: s.ahead,
      behind: s.behind,
      unknown: s.unknown,
    }
  }
}

/// One worktree as exposed to scripting / editor integrations. Mirrors
/// the columns of `gwm list` plus the machine-only fields a consumer
/// needs (absolute `path`, raw `age_seconds`, linked issue/PR numbers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonWorktree {
  /// Display name — the basename of the worktree directory.
  pub name: String,
  /// Internal git worktree id (`.git/worktrees/<id>`); diverges from
  /// `name` after a `git worktree move`.
  pub id: String,
  /// Absolute path to the worktree working directory.
  pub path: String,
  pub branch: Option<String>,
  /// Full HEAD commit oid (40-char hex), when resolvable. A machine
  /// consumer gets the exact oid for comparison; truncate client-side if a
  /// short form is wanted.
  pub head: Option<String>,
  pub is_main: bool,
  pub is_locked: bool,
  pub is_prunable: bool,
  pub status: JsonStatus,
  /// Branch age relative to the trunk baseline, in whole seconds.
  /// `null` for trunk branches and unresolvable repos.
  pub age_seconds: Option<u64>,
  /// Linked issue number (branch-name inferred or explicit), if any.
  pub issue: Option<u64>,
  /// Linked PR number (inferred, explicit, or auto-detected), if any.
  pub pr: Option<u64>,
  /// Agent sessions matched to this worktree (issue #408). **Experimental
  /// tier** — additive, omitted entirely (never `null`) when no session
  /// matched, so pre-#408 payloads are byte-identical. See
  /// `docs/schema/README.md` for the tier rules.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agents: Option<JsonWorktreeAgents>,
}

/// The agent-session summary of one worktree row (issue #408).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonWorktreeAgents {
  /// The most recently active session — what compact surfaces display.
  pub top: JsonAgentSession,
  /// Every matched session, most recent first.
  pub sessions: Vec<JsonAgentSession>,
}

/// One detected agent session on the wire (issue #408).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonAgentSession {
  /// Stable lowercase agent name: `claude` | `codex` | `opencode` | `vibe`.
  pub kind: String,
  /// `active` | `idle`.
  pub freshness: String,
  /// Last artefact activity, epoch seconds UTC.
  pub last_activity: u64,
  /// Backend-stable session identifier.
  pub id: String,
  /// Human-readable session name when the artefacts carry one (first user
  /// prompt or recorded title). Omitted when unavailable.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
}

impl JsonWorktreeAgents {
  /// Wire shape of a detection summary, `None` when no session matched —
  /// feeding `skip_serializing_if` so empty rows stay byte-identical.
  pub fn from_summary(agents: &crate::agent_sessions::WorktreeAgents, now: std::time::SystemTime) -> Option<Self> {
    let to_wire = |s: &crate::agent_sessions::AgentSession| JsonAgentSession {
      kind: s.kind.display().to_string(),
      freshness: match crate::agent_sessions::Freshness::classify(s.last_activity, s.ended, now) {
        crate::agent_sessions::Freshness::Active => "active".to_string(),
        crate::agent_sessions::Freshness::Idle => "idle".to_string(),
      },
      last_activity: s
        .last_activity
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0),
      id: s.id.clone(),
      name: s.name.clone(),
    };
    let top = agents.top()?;
    Some(Self {
      top: to_wire(top),
      sessions: agents.sessions.iter().map(to_wire).collect(),
    })
  }
}

impl From<&WorktreeInfo> for JsonWorktree {
  fn from(w: &WorktreeInfo) -> Self {
    Self {
      name: w.name.clone(),
      id: w.id.clone(),
      path: w.path.to_string_lossy().into_owned(),
      branch: w.branch.clone(),
      head: w.head.clone(),
      is_main: w.is_main,
      is_locked: w.is_locked,
      is_prunable: w.is_prunable,
      status: JsonStatus::from(&w.status),
      age_seconds: w.age.map(|d| d.as_secs()),
      issue: w.link.issue,
      pr: w.link.pr,
      // Filled by the list assembly when detection ran (issue #408); a bare
      // conversion carries no session info.
      agents: None,
    }
  }
}

/// The `{ name, path, branch }` triple returned by `gwm path --format=json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonPath {
  pub name: String,
  pub path: String,
  pub branch: Option<String>,
}

impl From<&WorktreeInfo> for JsonPath {
  fn from(w: &WorktreeInfo) -> Self {
    Self {
      name: w.name.clone(),
      path: w.path.to_string_lossy().into_owned(),
      branch: w.branch.clone(),
    }
  }
}

/// Stable lowercase string for a [`CheckStatus`], used as the `status`
/// field of [`JsonCheck`] and the `severity` of [`JsonDoctorReport`].
pub fn check_status_str(status: &CheckStatus) -> &'static str {
  match status {
    CheckStatus::Ok => "ok",
    CheckStatus::Warning => "warning",
    CheckStatus::Failed => "failed",
  }
}

/// One diagnostic check, the stable projection of [`crate::doctor::Check`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonCheck {
  pub name: String,
  /// `"ok"`, `"warning"`, or `"failed"`.
  pub status: String,
  pub detail: String,
  pub fix_hint: Option<String>,
}

/// A full doctor run, carrying the per-check list plus the aggregate
/// `severity` and the process `exit_code` (`0`/`1`/`2`) so a consumer
/// doesn't have to re-derive them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonDoctorReport {
  pub checks: Vec<JsonCheck>,
  /// Highest severity present: `"ok"`, `"warning"`, or `"failed"`.
  pub severity: String,
  pub exit_code: i32,
}

impl From<&DoctorReport> for JsonDoctorReport {
  fn from(r: &DoctorReport) -> Self {
    Self {
      checks: r
        .checks
        .iter()
        .map(|c| JsonCheck {
          name: c.name.clone(),
          status: check_status_str(&c.status).to_string(),
          detail: c.detail.clone(),
          fix_hint: c.fix_hint.clone(),
        })
        .collect(),
      severity: check_status_str(&r.severity()).to_string(),
      exit_code: r.exit_code(),
    }
  }
}

/// Build the stable JSON worktree list for `repo`. Shared by
/// `gwm list --format=json` and the daemon's `list` RPC method so both
/// surfaces stay byte-identical.
pub fn worktrees(repo: &git2::Repository) -> Result<Vec<JsonWorktree>> {
  let mut rows: Vec<JsonWorktree> = worktree::list(repo)?.iter().map(JsonWorktree::from).collect();
  let pins = agent_pins_for_rows(repo, &rows);
  attach_agents(&mut rows, &pins);
  Ok(rows)
}

/// Manual agent pins for already-built rows: `(path key, session id)` pairs
/// read from each row's branch config (issue #408 US4). Rows without a
/// branch (detached) cannot carry a pin.
pub fn agent_pins_for_rows(repo: &git2::Repository, rows: &[JsonWorktree]) -> Vec<(String, String)> {
  rows
    .iter()
    .filter_map(|r| {
      let branch = r.branch.as_deref()?;
      let sid = crate::github::agent_pin(repo, branch).ok().flatten()?;
      Some((r.path.clone(), sid))
    })
    .collect()
}

/// Populate the experimental `agents` field on already-built rows (issue
/// #408): one detection pass over the whole set, keyed back by `path`, with
/// manual `pins` overlaid. The single shared implementation for every
/// surface (CLI list, daemon, workspace rows) so they cannot drift. No home
/// directory → no-op (FR-009).
///
/// ponytail: workspace mode passes empty pins (each row belongs to a
/// different repo whose config we don't open here); wire per-repo pins if
/// workspace users ask.
pub fn attach_agents(rows: &mut [JsonWorktree], pins: &[(String, String)]) {
  let Some(home) = crate::agent_sessions::agents_home() else {
    return;
  };
  let now = std::time::SystemTime::now();
  let keyed: Vec<(String, std::path::PathBuf)> = rows
    .iter()
    .map(|r| (r.path.clone(), std::path::PathBuf::from(&r.path)))
    .collect();
  let summary = detect_cached(&home, &keyed, pins, now);
  for row in rows.iter_mut() {
    row.agents = summary
      .get(&row.path)
      .and_then(|a| JsonWorktreeAgents::from_summary(a, now));
  }
}

/// Detection result cache for the daemon's poll loop (Codex review round A):
/// `subscribe` consumers make the daemon re-list every poll tick (1 s by
/// default, once per subscriber), and re-walking the Codex/opencode/Vibe
/// stores each time is real disk churn. Same inputs within the TTL reuse the
/// last summary — the TUI's own 30 s re-detection cadence, applied here.
/// ponytail: one process-global slot guarded by a Mutex; per-input LRU only
/// if a real multi-repo daemon setup ever needs it.
fn detect_cached(
  home: &std::path::Path,
  keyed: &[(String, std::path::PathBuf)],
  pins: &[(String, String)],
  now: std::time::SystemTime,
) -> std::collections::BTreeMap<String, crate::agent_sessions::WorktreeAgents> {
  const TTL: std::time::Duration = std::time::Duration::from_secs(30);
  type CacheKey = (std::path::PathBuf, Vec<(String, String)>, Vec<String>);
  type CacheSlot = Option<(
    std::time::Instant,
    CacheKey,
    std::collections::BTreeMap<String, crate::agent_sessions::WorktreeAgents>,
  )>;
  static CACHE: std::sync::Mutex<CacheSlot> = std::sync::Mutex::new(None);

  let key: CacheKey = (
    home.to_path_buf(),
    pins.to_vec(),
    keyed.iter().map(|(k, _)| k.clone()).collect(),
  );
  // A poisoned mutex here would mean a panic mid-detection; recover by
  // recomputing rather than propagating the poison.
  let mut slot = CACHE.lock().unwrap_or_else(|e| e.into_inner());
  if let Some((at, cached_key, summary)) = slot.as_ref() {
    if *cached_key == key && at.elapsed() < TTL {
      return summary.clone();
    }
  }
  let summary = crate::agent_sessions::detect_all(home, keyed, pins, now);
  *slot = Some((std::time::Instant::now(), key, summary.clone()));
  summary
}
