//! GitHub milestone declarative management (issue #82).
//!
//! `[[milestones]]` in `.gwm.toml` declares the desired milestone set;
//! this module resolves declared entries into concrete `MilestoneSpec`
//! values (normalising dates, defaulting state to open), computes the
//! diff against the upstream remote, and exposes the structs that the
//! CLI (`gwm milestones list / push`) renders.
//!
//! The `gh`-backed I/O lives in `github.rs`; this module is
//! intentionally I/O-free so unit tests don't need a network or a `gh`
//! binary. Mirrors the shape of `labels.rs` so the two subcommands
//! read symmetrically.

use crate::config::MilestoneConfig;
use crate::error::{GwmError, Result};
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::{HashMap, HashSet};

/// A fully-resolved milestone declared by the user: same shape as
/// `MilestoneConfig` but with `state` materialised to the enum and
/// `due_on` normalised to RFC3339 (`YYYY-MM-DD` → end-of-day UTC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MilestoneSpec {
  pub title: String,
  pub description: Option<String>,
  /// RFC3339 normalised form (e.g. `2026-07-15T23:59:59Z`). `None`
  /// when the user omitted `due_on` — push code skips the field so the
  /// remote value is left untouched.
  pub due_on: Option<String>,
  pub state: MilestoneState,
}

/// One milestone as returned by `gh api repos/:owner/:repo/milestones`.
/// `number` is the GitHub-issued identifier required by the PATCH
/// endpoint; the diff carries it through so the push step doesn't need
/// a second fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMilestone {
  pub number: u64,
  pub title: String,
  pub description: Option<String>,
  pub due_on: Option<String>,
  pub state: MilestoneState,
}

/// `open` (default) or `closed`. GitHub's REST contract only knows
/// these two values; we model them as an enum so a typo can't sneak
/// through the diff as a "fourth state".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilestoneState {
  Open,
  Closed,
}

impl MilestoneState {
  /// Canonical lowercase form sent on the wire and rendered in the
  /// `gwm milestones list` output.
  pub fn as_str(&self) -> &'static str {
    match self {
      Self::Open => "open",
      Self::Closed => "closed",
    }
  }
}

/// Kind of mutation a `MilestoneUpdate` carries. The variant exists
/// to leave room for future `delete` entries when `--prune` evolves
/// without reshuffling the `MilestoneDiff` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilestoneAction {
  Create,
  Update,
}

/// One row in `MilestoneDiff::to_update`. Carries the previous remote
/// values (when they differed) so `gwm milestones list` can render
/// `~ v0.7.0 (due 2026-07-01 → 2026-07-15)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MilestoneUpdate {
  pub action: MilestoneAction,
  pub spec: MilestoneSpec,
  /// Remote milestone number — required by the gh PATCH endpoint
  /// (`gh api -X PATCH repos/:owner/:repo/milestones/{number}`).
  pub number: u64,
  pub previous_due_on: Option<String>,
  pub previous_description: Option<String>,
  pub previous_state: Option<MilestoneState>,
}

/// Result of diffing the declared milestone set against the remote.
/// Each bucket is rendered separately by `gwm milestones list` and
/// consumed by `gwm milestones push`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MilestoneDiff {
  pub to_create: Vec<MilestoneSpec>,
  pub to_update: Vec<MilestoneUpdate>,
  pub matching: Vec<MilestoneSpec>,
  pub extra_on_remote: Vec<RemoteMilestone>,
}

impl MilestoneDiff {
  /// `(create, update, match, extra_on_remote)` — the four numbers
  /// surfaced in the one-line push summary.
  pub fn counts(&self) -> (usize, usize, usize, usize) {
    (
      self.to_create.len(),
      self.to_update.len(),
      self.matching.len(),
      self.extra_on_remote.len(),
    )
  }
}

// --- Date / state helpers -----------------------------------------------

/// Accept either `YYYY-MM-DD` (materialised as 23:59:59 UTC of that
/// day) or a full RFC3339 timestamp (canonicalised to UTC `…Z`). The
/// short form is the issue spec's "common-sense" semantic for a due
/// date — "due Friday" should not close at midnight UTC and surprise
/// the user. Long forms with an offset are converted to the
/// equivalent UTC instant before serialising, so the diff doesn't
/// flip-flop against GitHub's canonical `…Z` representation (Copilot
/// review on PR #92).
pub fn normalize_due_on(s: &str) -> Result<String> {
  let trimmed = s.trim();
  if trimmed.is_empty() {
    return Err(GwmError::Config("invalid due_on: empty string".into()));
  }
  // Short form: exactly `YYYY-MM-DD` (10 chars). chrono's strict
  // parser catches `2026-02-30` and friends.
  if trimmed.len() == 10 {
    let date = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
      .map_err(|e| GwmError::Config(format!("invalid due_on '{}': {}", trimmed, e)))?;
    return Ok(format!("{}T23:59:59Z", date.format("%Y-%m-%d")));
  }
  // Long form: full RFC3339. Convert any offset to UTC and emit `…Z`.
  // GitHub serialises `due_on` as `…Z`; without canonicalisation a
  // user-supplied `+00:00` (or `+02:00`) would surface as a perpetual
  // mismatch in `diff_milestones` and `gwm milestones push` would
  // issue no-op updates on every run.
  let dt = DateTime::parse_from_rfc3339(trimmed)
    .map_err(|e| GwmError::Config(format!("invalid due_on '{}': {}", trimmed, e)))?;
  Ok(dt.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// Strict lowercase parse — `open` or `closed`. GitHub stores the
/// value lowercase, so accepting `Open` / `OPEN` would mask a typo on
/// the user side without any upside.
pub fn parse_state(s: &str) -> Result<MilestoneState> {
  match s {
    "open" => Ok(MilestoneState::Open),
    "closed" => Ok(MilestoneState::Closed),
    other => Err(GwmError::Config(format!(
      "invalid milestone state '{}': expected 'open' or 'closed'",
      other
    ))),
  }
}

// --- Resolve MilestoneConfig → MilestoneSpec ----------------------------

/// Materialise a list of declared `[[milestones]]` entries into
/// concrete `MilestoneSpec` values. Defaults `state` to `Open`,
/// normalises `due_on` to RFC3339, and collapses an empty
/// `description` to `None` (same contract as labels).
pub fn resolve_milestones(declared: &[MilestoneConfig]) -> Result<Vec<MilestoneSpec>> {
  declared
    .iter()
    .map(|m| {
      let state = match &m.state {
        Some(s) => {
          parse_state(s).map_err(|e| GwmError::Config(format!("milestone '{}' has invalid state: {}", m.title, e)))?
        }
        None => MilestoneState::Open,
      };
      let due_on = match &m.due_on {
        Some(s) => Some(
          normalize_due_on(s)
            .map_err(|e| GwmError::Config(format!("milestone '{}' has invalid due_on: {}", m.title, e)))?,
        ),
        None => None,
      };
      Ok(MilestoneSpec {
        title: m.title.clone(),
        description: m.description.clone().filter(|s| !s.is_empty()),
        due_on,
        state,
      })
    })
    .collect()
}

// --- Diff declared vs remote --------------------------------------------

/// Compute the diff between the user's declared milestone set and the
/// milestones currently on the upstream remote. Pure function — no
/// I/O, no observable side effects.
///
/// Comparison rules:
/// - **Title** is the unique key. Matching is byte-exact, since
///   GitHub treats `v0.7.0` and `V0.7.0` as different milestones.
/// - **`due_on`** is compared after normalising both sides through
///   `normalize_due_on` — defence in depth on top of
///   [`crate::github::parse_milestones_json`], which already returns
///   RFC3339, so a manually-constructed `RemoteMilestone` (e.g. from a
///   fixture) with the short form doesn't slip through as a spurious
///   update.
/// - **`description`**: `None` and `Some("")` are equivalent (GitHub
///   stores them interchangeably).
/// - **`state`**: byte-exact `Open` / `Closed` comparison.
pub fn diff_milestones(declared: &[MilestoneSpec], remote: &[RemoteMilestone]) -> MilestoneDiff {
  let remote_by_title: HashMap<&str, &RemoteMilestone> = remote.iter().map(|r| (r.title.as_str(), r)).collect();
  let declared_titles: HashSet<&str> = declared.iter().map(|s| s.title.as_str()).collect();

  let mut to_create = Vec::new();
  let mut to_update = Vec::new();
  let mut matching = Vec::new();

  for spec in declared {
    match remote_by_title.get(spec.title.as_str()) {
      None => to_create.push(spec.clone()),
      Some(r) => {
        let due_match = norm_due(&spec.due_on) == norm_due(&r.due_on);
        let desc_match = norm_desc(&spec.description) == norm_desc(&r.description);
        let state_match = spec.state == r.state;
        if due_match && desc_match && state_match {
          matching.push(spec.clone());
        } else {
          to_update.push(MilestoneUpdate {
            action: MilestoneAction::Update,
            spec: spec.clone(),
            number: r.number,
            previous_due_on: if due_match { None } else { r.due_on.clone() },
            previous_description: if desc_match { None } else { r.description.clone() },
            previous_state: if state_match { None } else { Some(r.state) },
          });
        }
      }
    }
  }

  let extra_on_remote: Vec<RemoteMilestone> = remote
    .iter()
    .filter(|r| !declared_titles.contains(r.title.as_str()))
    .cloned()
    .collect();

  MilestoneDiff {
    to_create,
    to_update,
    matching,
    extra_on_remote,
  }
}

fn norm_desc(d: &Option<String>) -> Option<String> {
  d.as_ref().filter(|s| !s.is_empty()).cloned()
}

/// Best-effort normalisation for comparison purposes. If `s` is the
/// short form (10 chars), expand it; if it parses as RFC3339, accept
/// it verbatim; if neither parses, fall back to the raw string so a
/// fixture with a non-canonical value still compares deterministically
/// against itself.
fn norm_due(d: &Option<String>) -> Option<String> {
  d.as_ref().map(|s| normalize_due_on(s).unwrap_or_else(|_| s.clone()))
}
