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
  /// The worktree's note, verbatim (issue #515). **Experimental tier** —
  /// additive, omitted entirely (never `null`) when the branch carries no
  /// note, so pre-#515 payloads are byte-identical. A note that exists only
  /// inside the TUI would be off-contract for a project shipping a frozen
  /// JSON schema, a daemon and a statusline; this is that machine surface,
  /// alongside `gwm note show`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub note: Option<String>,
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
      // The plain lossy absolute path: it is the PUBLIC schema value
      // consumers open and compare, so it must never grow disambiguation
      // suffixes (Codex review round U undoing round T's key reuse) —
      // agent association uses a separate lossless INTERNAL key derived
      // from the caller-kept real `PathBuf`s instead.
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
      // Filled by [`attach_notes`] (issue #515). `WorktreeInfo` carries only
      // the presence flag the table marker needs, so the note text never
      // travels through the TUI's per-row snapshot.
      note: None,
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
  let trees = worktree::list(repo)?;
  let mut rows: Vec<JsonWorktree> = trees.iter().map(JsonWorktree::from).collect();
  let reals: Vec<std::path::PathBuf> = trees.iter().map(|w| w.path.clone()).collect();
  let pins = agent_pins_for_rows(repo, &trees);
  attach_agents(&mut rows, &reals, &pins);
  attach_notes(repo, &mut rows);
  Ok(rows)
}

/// Populate the experimental `note` field on already-built rows (issue
/// #515) — one small file read per branched row, the same order of cost as
/// the `git config` reads `list` already pays per row.
///
/// Every failure mode (detached row, unportable branch name, absent,
/// unreadable, or blank file) collapses to `None` through
/// [`crate::notes::read`]: a permission problem on one note must not fail
/// `gwm list --format=json` or a daemon poll.
pub fn attach_notes(repo: &git2::Repository, rows: &mut [JsonWorktree]) {
  for row in rows.iter_mut() {
    let Some(branch) = crate::github::pinnable_branch(row.branch.as_deref()) else {
      continue;
    };
    row.note = crate::notes::read(repo, branch);
  }
}

/// Manual agent pins for already-built rows: `(path key, session id)` pairs
/// read from each row's branch config (issue #408 US4). Rows without a
/// branch (detached) cannot carry a pin.
pub fn agent_pins_for_rows(repo: &git2::Repository, trees: &[crate::worktree::WorktreeInfo]) -> Vec<(String, String)> {
  trees
    .iter()
    .flat_map(|w| {
      let pins = crate::github::pinnable_branch(w.branch.as_deref())
        .map(|branch| crate::github::agent_pins(repo, branch).unwrap_or_default())
        .unwrap_or_default();
      // Keyed by the lossless display key — the INTERNAL association key
      // shared with `attach_agents` (round U), never the public path.
      let key = crate::agent_sessions::path_display_key(&w.path);
      pins.into_iter().map(move |sid| (key.clone(), sid))
    })
    .collect()
}

/// Populate the experimental `agents` field on already-built rows (issue
/// #408): one detection pass over the whole set, keyed back by `path`, with
/// manual `pins` overlaid. The single shared implementation for every
/// surface (CLI list, daemon, workspace rows) so they cannot drift. No home
/// directory → no-op (FR-009).
///
/// Workspace callers open each row's owning repo to build `pins` (Codex
/// review round I) — this pass itself stays repo-agnostic.
pub fn attach_agents(rows: &mut [JsonWorktree], reals: &[std::path::PathBuf], pins: &[(String, String)]) {
  attach_agents_inner(rows, reals, pins, false);
}

/// [`attach_agents`] variant that also returns the raw session pool, so
/// `gwm agents` can list the sessions no worktree matched — precisely the
/// ones worth attaching manually (Codex review round C). Split from the
/// plain call because the pool costs the Claude foreign-dir sweep (round
/// F): `gwm list` and daemon polls must not pay it.
pub fn attach_agents_with_pool(
  rows: &mut [JsonWorktree],
  reals: &[std::path::PathBuf],
  pins: &[(String, String)],
) -> Vec<crate::agent_sessions::AgentSession> {
  attach_agents_inner(rows, reals, pins, true)
}

fn attach_agents_inner(
  rows: &mut [JsonWorktree],
  reals: &[std::path::PathBuf],
  pins: &[(String, String)],
  want_pool: bool,
) -> Vec<crate::agent_sessions::AgentSession> {
  let Some(home) = crate::agent_sessions::agents_home() else {
    return Vec::new();
  };
  let now = std::time::SystemTime::now();
  // The association keys are lossless display keys derived from the
  // ORIGINAL PathBufs the caller kept (rows and reals are parallel) —
  // never the public `row.path`, which stays the plain lossy absolute
  // path of the schema and could collide for non-UTF-8 worktrees
  // (Codex review rounds T + U).
  debug_assert_eq!(rows.len(), reals.len());
  let keyed: Vec<(String, std::path::PathBuf)> = reals
    .iter()
    .map(|p| (crate::agent_sessions::path_display_key(p), p.clone()))
    .collect();
  let (summary, pool) = detect_cached(&home, &keyed, pins, now, want_pool);
  for (row, real) in rows.iter_mut().zip(reals) {
    row.agents = summary
      .get(&crate::agent_sessions::path_display_key(real))
      .and_then(|a| JsonWorktreeAgents::from_summary(a, now));
  }
  pool
}

/// Detection result cache for the daemon's poll loop (Codex review round A):
/// `subscribe` consumers make the daemon re-list every poll tick (1 s by
/// default, once per subscriber), and re-walking the Codex/opencode/Vibe
/// stores each time is real disk churn. Same inputs within the TTL reuse the
/// last summary — the TUI's own 30 s re-detection cadence, applied here.
/// ponytail: one process-global slot guarded by a Mutex; per-input LRU only
/// if a real multi-repo daemon setup ever needs it.
/// `want_pool` selects the detection depth (round F): `false` = summary
/// only, matched-only Claude scan, empty pool returned; `true` = full
/// sweep + raw pool. A cached full detection serves BOTH shapes (the
/// summary is identical — swept sessions never summarize); a cached
/// summary-only entry cannot serve a pool request and is recomputed.
fn detect_cached(
  home: &std::path::Path,
  keyed: &[(String, std::path::PathBuf)],
  pins: &[(String, String)],
  now: std::time::SystemTime,
  want_pool: bool,
) -> (
  std::collections::BTreeMap<String, crate::agent_sessions::WorktreeAgents>,
  Vec<crate::agent_sessions::AgentSession>,
) {
  const TTL: std::time::Duration = std::time::Duration::from_secs(30);
  type CacheKey = (std::path::PathBuf, Vec<(String, String)>, Vec<String>);
  type Detection = (
    std::collections::BTreeMap<String, crate::agent_sessions::WorktreeAgents>,
    Vec<crate::agent_sessions::AgentSession>,
  );
  type CacheSlot = Option<(std::time::Instant, CacheKey, bool, Detection)>;
  static CACHE: std::sync::Mutex<CacheSlot> = std::sync::Mutex::new(None);

  let key: CacheKey = (
    home.to_path_buf(),
    pins.to_vec(),
    keyed.iter().map(|(k, _)| k.clone()).collect(),
  );
  // A poisoned mutex here would mean a panic mid-detection; recover by
  // recomputing rather than propagating the poison.
  let mut slot = CACHE.lock().unwrap_or_else(|e| e.into_inner());
  if let Some((at, cached_key, has_pool, detection)) = slot.as_ref() {
    if *cached_key == key && at.elapsed() < TTL && (*has_pool || !want_pool) {
      return detection.clone();
    }
  }
  let detection = if want_pool {
    crate::agent_sessions::detect_with_sessions(home, keyed, pins, now)
  } else {
    (crate::agent_sessions::detect_all(home, keyed, pins, now), Vec::new())
  };
  *slot = Some((std::time::Instant::now(), key, want_pool, detection.clone()));
  detection
}
