//! Issue ↔ PR ↔ branch link storage, and the **GitHub backend** for the
//! [`crate::forge::Forge`] trait (via the `gh` CLI).
//!
//! Storage lives in git branch config: `branch.<name>.gwm-issue` and
//! `branch.<name>.gwm-pr`. Issue numbers are auto-detected from the
//! `<type>/#<N>-<slug>` branch convention when no explicit override is set.
//! Those `branch.<x>.gwm-*` keys are **forge-neutral** and deliberately stay
//! shared rather than moving behind the trait (issue #419): a GitLab worktree
//! reads and writes exactly the same keys.
//!
//! Fetch shells out to `gh` and parses its JSON output. The parsing functions
//! (`parse_issue_json`, `parse_pr_json`) are exposed publicly so tests can
//! cover the JSON contract without depending on a real `gh` binary.

use crate::error::{GwmError, Result};
use crate::forge::{self, Forge, ForgeKind};
use crate::labels::{LabelSpec, RemoteLabel};
use crate::milestones::{MilestoneSpec, MilestoneState, RemoteMilestone};
use crate::naming::parse_branch;
use git2::Repository;
use serde::Deserialize;
use std::ffi::{OsStr, OsString};
use std::sync::LazyLock;

// The parsed shapes are forge-agnostic and now live in `forge`; re-exported
// here so the many `github::PrStatus` / `github::CiState` imports across the
// TUI and CLI keep resolving unchanged.
pub use crate::forge::{cli_command_line as gh_command_line, repo_slug};
pub use crate::forge::{
  CheckOutcome, CiState, CreatedIssue, CreatedPr, IssueCreateRequest, IssueState, IssueStatus, PrCheck,
  PrCreateRequest, PrHead, PrState, PrStatus,
};

static ISSUE_URL_RE: LazyLock<regex::Regex> =
  LazyLock::new(|| regex::Regex::new(r"/issues/(\d+)(?:\b|$)").expect("static issue URL regex compiles"));
static PR_URL_RE: LazyLock<regex::Regex> =
  LazyLock::new(|| regex::Regex::new(r"/pull/(\d+)(?:\b|$)").expect("static PR URL regex compiles"));

const ISSUE_CONFIG_KEY: &str = "gwm-issue";
const PR_CONFIG_KEY: &str = "gwm-pr";
/// Persisted home of an auto-detected PR (issue #283). Kept distinct from
/// the explicit [`PR_CONFIG_KEY`] so [`read_link`] can resolve it as
/// [`LinkSource::Detected`] (not `Explicit`) — the pane needs that
/// distinction for its `detected` badge, and the explicit override must
/// still win.
const DETECTED_PR_CONFIG_KEY: &str = "gwm-pr-detected";
const ISSUE_TITLE_CONFIG_KEY: &str = "gwm-issue-title";
const PR_TITLE_CONFIG_KEY: &str = "gwm-pr-title";
const DETECTED_PR_TITLE_CONFIG_KEY: &str = "gwm-pr-detected-title";
const ISSUE_STATE_CONFIG_KEY: &str = "gwm-issue-state";
const PR_STATE_CONFIG_KEY: &str = "gwm-pr-state";
const DETECTED_PR_STATE_CONFIG_KEY: &str = "gwm-pr-detected-state";
/// Manual agent-session pin (issue #408 US4): the session id the user
/// attached to this branch's worktree with `gwm agents attach`. One pin per
/// worktree; auto-detection stays the default and the pin only adds.
const AGENT_PIN_CONFIG_KEY: &str = "gwm-agent-pin";

/// Where the issue or PR number came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSource {
  /// No link known (no branch-name match and no explicit override).
  None,
  /// Inferred from a branch following `<type>/#<N>-<slug>`.
  BranchName,
  /// Explicit override set via `gwm link …` (lives in git branch config).
  Explicit,
  /// Auto-detected from GitHub: a PR whose head ref is this branch was
  /// found via `gh pr list --head <branch>` (issue #181). May be persisted
  /// to the `gwm-pr-detected` branch-config key (issue #283) so the
  /// no-fetch table read path surfaces it on every row; an explicit
  /// `gwm link --pr` still always wins on the next read.
  Detected,
}

/// Resolved link for one branch: which issue (if any), which PR (if any),
/// and where each number came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchLink {
  pub issue: Option<u64>,
  pub pr: Option<u64>,
  pub issue_title: Option<String>,
  pub pr_title: Option<String>,
  pub issue_state: Option<IssueState>,
  pub pr_state: Option<PrState>,
  pub issue_source: LinkSource,
  pub pr_source: LinkSource,
}

impl BranchLink {
  pub fn empty() -> Self {
    Self {
      issue: None,
      pr: None,
      issue_title: None,
      pr_title: None,
      issue_state: None,
      pr_state: None,
      issue_source: LinkSource::None,
      pr_source: LinkSource::None,
    }
  }

  /// One-line human-readable rendering for the CLI / TUI status bar.
  ///
  /// `pr_noun` comes from [`crate::forge::Forge::pr_noun`] — "PR" on
  /// GitHub, "MR" on GitLab (issue #419). Passed in rather than read from
  /// a global so `BranchLink` stays a plain data struct.
  pub fn summary(&self, pr_noun: &str) -> String {
    match (self.issue, self.pr) {
      (None, None) => "no link".into(),
      (Some(i), None) => format!("issue #{i}"),
      (None, Some(p)) => format!("{pr_noun} #{p}"),
      (Some(i), Some(p)) => format!("issue #{i} · {pr_noun} #{p}"),
    }
  }
}

/// Read the link for `branch`. Explicit overrides win over branch-name auto-detect.
pub fn read_link(repo: &Repository, branch: &str) -> Result<BranchLink> {
  let explicit_issue = read_branch_u64(repo, branch, ISSUE_CONFIG_KEY)?;
  let explicit_pr = read_branch_u64(repo, branch, PR_CONFIG_KEY)?;

  let (issue, issue_source) = match explicit_issue {
    Some(n) => (Some(n), LinkSource::Explicit),
    None => match parse_branch(branch).and_then(|s| s.issue.parse::<u64>().ok()) {
      Some(n) => (Some(n), LinkSource::BranchName),
      None => (None, LinkSource::None),
    },
  };

  // PR resolution order (issue #283): an explicit `gwm link --pr` wins,
  // then a persisted auto-detection (`gwm-pr-detected`), then nothing. The
  // persisted-detected branch is what lets the no-fetch table read path
  // colour the PR pastille on every row without a per-row `gh` shell-out.
  let (pr, pr_source) = match explicit_pr {
    Some(n) => (Some(n), LinkSource::Explicit),
    None => match read_branch_u64(repo, branch, DETECTED_PR_CONFIG_KEY)? {
      Some(n) => (Some(n), LinkSource::Detected),
      None => (None, LinkSource::None),
    },
  };
  let issue_title = match issue {
    Some(_) => read_branch_string(repo, branch, ISSUE_TITLE_CONFIG_KEY)?,
    None => None,
  };
  let issue_state = match issue {
    Some(_) => read_branch_issue_state(repo, branch)?,
    None => None,
  };
  let pr_title = match pr_source {
    LinkSource::Explicit => read_branch_string(repo, branch, PR_TITLE_CONFIG_KEY)?,
    LinkSource::Detected => read_branch_string(repo, branch, DETECTED_PR_TITLE_CONFIG_KEY)?,
    LinkSource::BranchName | LinkSource::None => None,
  };
  let pr_state = match pr_source {
    LinkSource::Explicit => read_branch_pr_state(repo, branch, PR_STATE_CONFIG_KEY)?,
    LinkSource::Detected => read_branch_pr_state(repo, branch, DETECTED_PR_STATE_CONFIG_KEY)?,
    LinkSource::BranchName | LinkSource::None => None,
  };

  Ok(BranchLink {
    issue,
    pr,
    issue_title,
    pr_title,
    issue_state,
    pr_state,
    issue_source,
    pr_source,
  })
}

/// Stamp an auto-detected PR number onto `link` when no PR is already
/// linked. Pure helper (issue #181): the caller supplies the detection
/// result — typically `find_pr_for_branch(slug, branch).ok().flatten()` —
/// and this decides whether to apply it.
///
/// An explicit (or previously-detected) PR always wins: when `link.pr`
/// is already `Some`, this is a no-op so a `gwm link --pr` override is
/// never clobbered. The applied number is marked [`LinkSource::Detected`].
/// This function only mutates the in-memory [`BranchLink`]; call
/// [`persist_detected_pr`] separately to write it to the git config so the
/// table read path (issue #283) picks it up.
pub fn apply_detected_pr(link: &mut BranchLink, detected: Option<u64>) {
  if link.pr.is_none() {
    if let Some(n) = detected {
      link.pr = Some(n);
      link.pr_source = LinkSource::Detected;
      link.pr_title = None;
      link.pr_state = None;
    }
  }
}

/// Resolve the link for `branch` and, unless a PR is *explicitly* linked,
/// auto-detect the branch's PR from GitHub via `gh` (issue #181). The
/// detected PR is marked [`LinkSource::Detected`].
///
/// A persisted auto-detection (`gwm-pr-detected`, issue #283) does NOT pin
/// the result here: this is the live-detection path (`gwm status` /
/// `gwm list --detect-pr`), so it re-runs `gh pr list` to reflect a PR that
/// was opened / closed / replaced since the last detection, rather than
/// echoing a stale stored number (Codex review #284). Only an explicit
/// `gwm link --pr` short-circuits the probe.
///
/// On a successful probe this also **reconciles the persisted cache**
/// (`gwm-pr-detected`): it rewrites the stored number to the fresh result,
/// or clears it when the PR vanished, so the no-fetch consumers (`read_link`,
/// the TUI table at startup, `gwm open pr`) don't resurrect a stale number
/// after this path saw it change (Codex review #284). The cache write is
/// best-effort — a read-only repo must not turn `gwm status` into an error.
///
/// Detection is best-effort: a `gh` failure (not installed, no network)
/// leaves the link untouched — a persisted detection survives the failed
/// probe rather than being wiped — and the local link is still returned.
/// This shells out, so callers on hot paths (per-worktree listing) must opt
/// in deliberately rather than route every read through here.
pub fn read_link_with_pr_detection(repo: &Repository, branch: &str, forge: &dyn Forge) -> Result<BranchLink> {
  let mut link = read_link(repo, branch)?;
  if link.pr_source != LinkSource::Explicit {
    // Re-resolve live. On success, the fresh result replaces any persisted
    // detection (a vanished PR clears it); on a CLI failure, keep whatever
    // `read_link` already resolved (possibly a persisted detection).
    if let Ok(detected) = forge.find_pr_for_branch(branch) {
      let previous_pr = link.pr;
      let previous_pr_source = link.pr_source;
      let previous_pr_title = link.pr_title.clone();
      let previous_pr_state = link.pr_state;
      link.pr = detected;
      link.pr_source = match detected {
        Some(_) => LinkSource::Detected,
        None => LinkSource::None,
      };
      link.pr_title = if previous_pr_source == LinkSource::Detected && detected == previous_pr {
        previous_pr_title
      } else {
        None
      };
      link.pr_state = if previous_pr_source == LinkSource::Detected && detected == previous_pr {
        previous_pr_state
      } else {
        None
      };
      // Reconcile the persisted cache (issue #283 / Codex review #284) so the
      // no-fetch consumers (`read_link`, the TUI table at startup,
      // `gwm open pr`) don't resurrect a stale number after this live path
      // saw it change or vanish. Best-effort: a read-only repo must not turn
      // `gwm status` into an error, so a write failure is discarded.
      let _ = match detected {
        Some(n) => persist_detected_pr(repo, branch, n),
        None => clear_persisted_detected_pr(repo, branch),
      };
    }
  }
  Ok(link)
}

pub fn link_issue(repo: &Repository, branch: &str, number: u64) -> Result<()> {
  write_branch_u64(repo, branch, ISSUE_CONFIG_KEY, number)?;
  remove_branch_key(repo, branch, ISSUE_TITLE_CONFIG_KEY)?;
  remove_branch_key(repo, branch, ISSUE_STATE_CONFIG_KEY)
}

pub fn link_pr(repo: &Repository, branch: &str, number: u64) -> Result<()> {
  write_branch_u64(repo, branch, PR_CONFIG_KEY, number)?;
  remove_branch_key(repo, branch, PR_TITLE_CONFIG_KEY)?;
  remove_branch_key(repo, branch, PR_STATE_CONFIG_KEY)
}

pub fn unlink_issue(repo: &Repository, branch: &str) -> Result<()> {
  remove_branch_key(repo, branch, ISSUE_CONFIG_KEY)?;
  remove_branch_key(repo, branch, ISSUE_TITLE_CONFIG_KEY)?;
  remove_branch_key(repo, branch, ISSUE_STATE_CONFIG_KEY)
}

pub fn unlink_pr(repo: &Repository, branch: &str) -> Result<()> {
  // Drop both the explicit link and any persisted auto-detection (#283),
  // otherwise unlinking would leave a stale `gwm-pr-detected` number that
  // `read_link` would resurface as a `Detected` PR on the next read.
  remove_branch_key(repo, branch, PR_CONFIG_KEY)?;
  remove_branch_key(repo, branch, PR_TITLE_CONFIG_KEY)?;
  remove_branch_key(repo, branch, PR_STATE_CONFIG_KEY)?;
  remove_branch_key(repo, branch, DETECTED_PR_CONFIG_KEY)?;
  remove_branch_key(repo, branch, DETECTED_PR_TITLE_CONFIG_KEY)?;
  remove_branch_key(repo, branch, DETECTED_PR_STATE_CONFIG_KEY)
}

/// Persist an auto-detected PR number to its own branch-config key
/// (`gwm-pr-detected`, issue #283), distinct from the explicit `gwm-pr`.
/// This lets the no-fetch table read path surface the detected PR on every
/// row without a per-row `gh` shell-out, while keeping the
/// detected/explicit distinction the pane badge needs. An explicit
/// `gwm link --pr` still wins in [`read_link`]. Re-detection overwrites the
/// stored value and clears a cached title only when the detected number
/// actually changed.
pub fn persist_detected_pr(repo: &Repository, branch: &str, number: u64) -> Result<()> {
  let previous = read_branch_u64(repo, branch, DETECTED_PR_CONFIG_KEY)?;
  write_branch_u64(repo, branch, DETECTED_PR_CONFIG_KEY, number)?;
  if previous == Some(number) {
    Ok(())
  } else {
    remove_branch_key(repo, branch, DETECTED_PR_TITLE_CONFIG_KEY)?;
    remove_branch_key(repo, branch, DETECTED_PR_STATE_CONFIG_KEY)
  }
}

/// Drop a persisted auto-detection (issue #283). A no-op when no detected
/// PR was stored. Used when a detection no longer holds (the branch's PR
/// went away) so a stale number doesn't linger in the config.
pub fn clear_persisted_detected_pr(repo: &Repository, branch: &str) -> Result<()> {
  remove_branch_key(repo, branch, DETECTED_PR_CONFIG_KEY)?;
  remove_branch_key(repo, branch, DETECTED_PR_TITLE_CONFIG_KEY)?;
  remove_branch_key(repo, branch, DETECTED_PR_STATE_CONFIG_KEY)
}

pub fn persist_issue_title(repo: &Repository, branch: &str, title: &str) -> Result<()> {
  write_branch_string(repo, branch, ISSUE_TITLE_CONFIG_KEY, title)
}

pub fn persist_pr_title(repo: &Repository, branch: &str, title: &str) -> Result<()> {
  write_branch_string(repo, branch, PR_TITLE_CONFIG_KEY, title)
}

pub fn persist_detected_pr_title(repo: &Repository, branch: &str, title: &str) -> Result<()> {
  write_branch_string(repo, branch, DETECTED_PR_TITLE_CONFIG_KEY, title)
}

pub fn persist_issue_state(repo: &Repository, branch: &str, state: IssueState) -> Result<()> {
  write_branch_string(repo, branch, ISSUE_STATE_CONFIG_KEY, issue_state_config_value(state))
}

pub fn persist_pr_state(repo: &Repository, branch: &str, state: PrState) -> Result<()> {
  write_branch_string(repo, branch, PR_STATE_CONFIG_KEY, pr_state_config_value(state))
}

pub fn persist_detected_pr_state(repo: &Repository, branch: &str, state: PrState) -> Result<()> {
  write_branch_string(repo, branch, DETECTED_PR_STATE_CONFIG_KEY, pr_state_config_value(state))
}

fn config_key(branch: &str, leaf: &str) -> String {
  format!("branch.{}.{}", branch, leaf)
}

fn read_branch_u64(repo: &Repository, branch: &str, leaf: &str) -> Result<Option<u64>> {
  let cfg = repo.config()?;
  let key = config_key(branch, leaf);
  match cfg.get_string(&key) {
    Ok(s) => s
      .trim()
      .parse::<u64>()
      .map(Some)
      .map_err(|_| GwmError::Other(format!("config '{}' is not a valid number: {}", key, s))),
    Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
    Err(e) => Err(GwmError::Git(e)),
  }
}

fn read_branch_string(repo: &Repository, branch: &str, leaf: &str) -> Result<Option<String>> {
  let cfg = repo.config()?;
  let key = config_key(branch, leaf);
  match cfg.get_string(&key) {
    Ok(s) => Ok(Some(s)),
    Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
    Err(e) => Err(GwmError::Git(e)),
  }
}

fn read_branch_issue_state(repo: &Repository, branch: &str) -> Result<Option<IssueState>> {
  Ok(
    read_branch_string(repo, branch, ISSUE_STATE_CONFIG_KEY)?
      .as_deref()
      .and_then(parse_issue_state_config_value),
  )
}

fn read_branch_pr_state(repo: &Repository, branch: &str, leaf: &str) -> Result<Option<PrState>> {
  Ok(
    read_branch_string(repo, branch, leaf)?
      .as_deref()
      .and_then(parse_pr_state_config_value),
  )
}

fn parse_issue_state_config_value(value: &str) -> Option<IssueState> {
  match value.trim().to_ascii_lowercase().as_str() {
    "open" => Some(IssueState::Open),
    "closed" => Some(IssueState::Closed),
    _ => None,
  }
}

fn parse_pr_state_config_value(value: &str) -> Option<PrState> {
  match value.trim().to_ascii_lowercase().as_str() {
    "open" => Some(PrState::Open),
    "draft" => Some(PrState::Draft),
    "closed" => Some(PrState::Closed),
    "merged" => Some(PrState::Merged),
    _ => None,
  }
}

fn issue_state_config_value(state: IssueState) -> &'static str {
  match state {
    IssueState::Open => "open",
    IssueState::Closed => "closed",
  }
}

fn pr_state_config_value(state: PrState) -> &'static str {
  match state {
    PrState::Open => "open",
    PrState::Draft => "draft",
    PrState::Closed => "closed",
    PrState::Merged => "merged",
  }
}

fn write_branch_u64(repo: &Repository, branch: &str, leaf: &str, value: u64) -> Result<()> {
  let mut cfg = repo.config()?;
  cfg.set_str(&config_key(branch, leaf), &value.to_string())?;
  Ok(())
}

fn write_branch_string(repo: &Repository, branch: &str, leaf: &str, value: &str) -> Result<()> {
  let mut cfg = repo.config()?;
  cfg.set_str(&config_key(branch, leaf), value)?;
  Ok(())
}

/// Normalise a worktree's branch for pin storage (issue #408): libgit2
/// surfaces a detached HEAD either as `None` or as the literal `"HEAD"`
/// (the same trap the statusline handles), and a `branch.HEAD.*` config key
/// would silently share one pin across every detached worktree. Every pin
/// read/write goes through this guard.
pub fn pinnable_branch(branch: Option<&str>) -> Option<&str> {
  match branch {
    None | Some("HEAD") => None,
    other => other,
  }
}

/// Every manual agent-session pin on `branch` (issue #408 US4). The key is
/// **multi-valued** (user feedback 2026-07-22): several agents can work one
/// worktree at once, so attach accumulates instead of replacing.
pub fn agent_pins(repo: &Repository, branch: &str) -> Result<Vec<String>> {
  let cfg = repo.config()?;
  let key = config_key(branch, AGENT_PIN_CONFIG_KEY);
  let mut out = Vec::new();
  let result = match cfg.multivar(&key, None) {
    Ok(entries) => {
      entries
        .for_each(|e| {
          if let Ok(v) = e.value() {
            out.push(v.to_string());
          }
        })
        .map_err(GwmError::Git)?;
      Ok(out)
    }
    Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(out),
    Err(e) => Err(GwmError::Git(e)),
  };
  result
}

/// Pin `session_id` to `branch`'s worktree (`gwm agents attach`). Appends
/// to the multi-valued key; re-attaching an already-pinned id is a no-op.
pub fn add_agent_pin(repo: &Repository, branch: &str, session_id: &str) -> Result<()> {
  if agent_pins(repo, branch)?.iter().any(|p| p == session_id) {
    return Ok(());
  }
  let mut cfg = repo.config()?;
  // The never-matching regex makes libgit2 append a new value instead of
  // replacing an existing one (the documented multivar-append idiom).
  cfg.set_multivar(&config_key(branch, AGENT_PIN_CONFIG_KEY), "^$", session_id)?;
  Ok(())
}

/// Remove exactly the `session_id` pin (`gwm agents detach <wt> <id>` / `d`
/// on a pinned row). Returns whether it was present; absent is not an error.
pub fn remove_agent_pin(repo: &Repository, branch: &str, session_id: &str) -> Result<bool> {
  if !agent_pins(repo, branch)?.iter().any(|p| p == session_id) {
    return Ok(false);
  }
  let mut cfg = repo.config()?;
  // Escape regex metacharacters so an id is matched literally, anchored.
  let escaped: String = session_id
    .chars()
    .flat_map(|c| {
      if c.is_ascii_alphanumeric() {
        vec![c]
      } else {
        vec!['\\', c]
      }
    })
    .collect();
  cfg.remove_multivar(&config_key(branch, AGENT_PIN_CONFIG_KEY), &format!("^{escaped}$"))?;
  Ok(true)
}

/// Remove every pin on `branch` (bare `gwm agents detach <wt>`). A no-op
/// when none is set.
pub fn clear_agent_pins(repo: &Repository, branch: &str) -> Result<()> {
  let mut cfg = repo.config()?;
  match cfg.remove_multivar(&config_key(branch, AGENT_PIN_CONFIG_KEY), ".*") {
    Ok(()) => Ok(()),
    Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(()),
    Err(e) => Err(GwmError::Git(e)),
  }
}

fn remove_branch_key(repo: &Repository, branch: &str, leaf: &str) -> Result<()> {
  let mut cfg = repo.config()?;
  let key = config_key(branch, leaf);
  match cfg.remove(&key) {
    Ok(_) => Ok(()),
    Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(()),
    Err(e) => Err(GwmError::Git(e)),
  }
}

// ---- Issue / PR status ---------------------------------------------------

#[derive(Deserialize)]
struct RawIssue {
  number: u64,
  title: String,
  state: String,
  url: String,
  #[serde(default)]
  labels: Vec<RawLabel>,
  #[serde(rename = "updatedAt", default)]
  updated_at: String,
}

#[derive(Deserialize)]
struct RawLabel {
  name: String,
}

#[derive(Deserialize)]
struct RawPr {
  number: u64,
  title: String,
  state: String,
  #[serde(rename = "isDraft", default)]
  is_draft: bool,
  url: String,
  #[serde(rename = "updatedAt", default)]
  updated_at: String,
  #[serde(rename = "statusCheckRollup", default)]
  status_check_rollup: Vec<RawCheck>,
}

/// One `statusCheckRollup` entry. GitHub returns two shapes here: a
/// `CheckRun` (the Checks API — carries `status` + `conclusion`) and a
/// legacy `StatusContext` (the commit-status API — carries `state`). We
/// deserialize all three so both shapes classify correctly.
#[derive(Deserialize)]
struct RawCheck {
  #[serde(default)]
  status: String,
  #[serde(default)]
  conclusion: Option<String>,
  #[serde(default)]
  state: String,
  // Per-check identity + link, kept for the CI checks overlay (issue #436).
  // `name` + `detailsUrl` on the `CheckRun` shape; `context` + `targetUrl`
  // on the legacy `StatusContext` shape.
  #[serde(default)]
  name: String,
  #[serde(rename = "detailsUrl", default)]
  details_url: Option<String>,
  #[serde(default)]
  context: String,
  #[serde(rename = "targetUrl", default)]
  target_url: Option<String>,
  // Run metadata (CheckRun shape), kept for the overlay's detail column.
  #[serde(rename = "workflowName", default)]
  workflow_name: Option<String>,
  #[serde(rename = "startedAt", default)]
  started_at: Option<String>,
  #[serde(rename = "completedAt", default)]
  completed_at: Option<String>,
}

pub fn parse_issue_json(s: &str) -> Result<IssueStatus> {
  let raw: RawIssue = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "issue",
    source: e,
  })?;
  let state = match raw.state.as_str() {
    "OPEN" | "open" => IssueState::Open,
    "CLOSED" | "closed" => IssueState::Closed,
    other => return Err(GwmError::Other(format!("unknown issue state '{}'", other))),
  };
  Ok(IssueStatus {
    number: raw.number,
    title: raw.title,
    state,
    url: raw.url,
    labels: raw.labels.into_iter().map(|l| l.name).collect(),
    updated_at: raw.updated_at,
  })
}

pub fn parse_pr_json(s: &str) -> Result<PrStatus> {
  let raw: RawPr = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse { kind: "pr", source: e })?;
  let state = match (raw.state.as_str(), raw.is_draft) {
    ("MERGED" | "merged", _) => PrState::Merged,
    ("CLOSED" | "closed", _) => PrState::Closed,
    ("OPEN" | "open", true) => PrState::Draft,
    ("OPEN" | "open", false) => PrState::Open,
    (other, _) => return Err(GwmError::Other(format!("unknown PR state '{}'", other))),
  };
  let checks_total = raw.status_check_rollup.len() as u32;
  // Count the same "accepted" terminals the CI state treats as green, so the
  // `N/M` shown next to the indicator stays consistent with its label — a
  // rollup of SUCCESS + NEUTRAL + SKIPPED reads "passing 3/3", not "1/3"
  // (Codex review #302).
  let checks_passed = raw
    .status_check_rollup
    .iter()
    .filter(|c| matches!(classify_check(c), CheckOutcome::Passing))
    .count() as u32;
  let ci = derive_ci_state(&raw.status_check_rollup);
  let checks = raw
    .status_check_rollup
    .iter()
    .map(|c| PrCheck {
      name: if c.name.is_empty() {
        c.context.clone()
      } else {
        c.name.clone()
      },
      outcome: classify_check(c),
      url: c.details_url.clone().or_else(|| c.target_url.clone()),
      workflow_name: c.workflow_name.clone(),
      started_at: c.started_at.clone(),
      completed_at: c.completed_at.clone(),
    })
    .collect();
  Ok(PrStatus {
    number: raw.number,
    title: raw.title,
    state,
    url: raw.url,
    updated_at: raw.updated_at,
    checks_passed,
    checks_total,
    ci,
    checks,
  })
}

/// Classify one rollup entry, handling both the `CheckRun` shape
/// (`status` + `conclusion`) and the legacy `StatusContext` shape
/// (`state`). A `CheckRun` is only green for an *accepted* terminal
/// conclusion (SUCCESS / NEUTRAL / SKIPPED, or a completed check with no
/// conclusion); every other terminal conclusion — FAILURE, CANCELLED,
/// TIMED_OUT, ACTION_REQUIRED, STARTUP_FAILURE, STALE, … — reads as failing
/// rather than silently falling through to green (Codex review #302).
fn classify_check(c: &RawCheck) -> CheckOutcome {
  // `CheckRun`: `status` is populated (QUEUED / IN_PROGRESS / COMPLETED).
  if !c.status.is_empty() {
    if !c.status.eq_ignore_ascii_case("COMPLETED") {
      return CheckOutcome::Running;
    }
    return match c.conclusion.as_deref() {
      Some(s) if is_accepted_conclusion(s) => CheckOutcome::Passing,
      // A completed check with no conclusion is treated leniently (green) so
      // missing data never paints a false red.
      None => CheckOutcome::Passing,
      Some(_) => CheckOutcome::Failing,
    };
  }
  // Legacy `StatusContext`: classify by `state`.
  match c.state.to_ascii_uppercase().as_str() {
    "SUCCESS" => CheckOutcome::Passing,
    "FAILURE" | "ERROR" => CheckOutcome::Failing,
    // PENDING / EXPECTED / unknown — not yet conclusive.
    _ => CheckOutcome::Running,
  }
}

/// Terminal `CheckRun` conclusions that count as green.
fn is_accepted_conclusion(conclusion: &str) -> bool {
  matches!(
    conclusion.to_ascii_uppercase().as_str(),
    "SUCCESS" | "NEUTRAL" | "SKIPPED"
  )
}

/// Collapse a `statusCheckRollup` into a single [`CiState`]. The
/// aggregation rule itself is shared with the GitLab backend since #419 —
/// see [`forge::aggregate_ci_state`].
fn derive_ci_state(checks: &[RawCheck]) -> CiState {
  forge::aggregate_ci_state(checks.iter().map(classify_check))
}

// ---- gh CLI invocation ---------------------------------------------------

const ISSUE_JSON_FIELDS: &str = "number,title,state,url,labels,updatedAt";
const PR_JSON_FIELDS: &str = "number,title,state,isDraft,url,updatedAt,statusCheckRollup";

/// Run `gh issue view <n> --repo <slug> --json …` and parse the result.
pub fn fetch_issue(slug: &str, number: u64) -> Result<IssueStatus> {
  fetch_issue_with(&gh_program(), slug, number)
}

/// [`fetch_issue`] with an explicitly resolved `gh` program path. Used by
/// the TUI's off-thread fetch (issue #217): the program is resolved on the
/// main thread via [`gh_program`] and handed to the worker thread, so the
/// thread never touches `GWM_GH` / the process environment concurrently
/// with env-mutating callers.
pub fn fetch_issue_with(program: &OsStr, slug: &str, number: u64) -> Result<IssueStatus> {
  parse_issue_json(&run_gh_with(program, issue_view_argv(slug, number))?)
}

/// Argv for `gh issue view <n> --repo <slug> --json …`.
pub fn issue_view_argv(slug: &str, number: u64) -> Vec<String> {
  vec![
    "issue".into(),
    "view".into(),
    number.to_string(),
    "--repo".into(),
    slug.into(),
    "--json".into(),
    ISSUE_JSON_FIELDS.into(),
  ]
}

/// Resolve the `gh` program to invoke: `$GWM_GH` when set (test / override
/// hook), else `gh` on `PATH`. Read once on the calling thread so off-thread
/// fetches can capture it without re-reading the environment.
pub fn gh_program() -> OsString {
  std::env::var_os("GWM_GH").unwrap_or_else(|| "gh".into())
}

pub fn create_issue(slug: &str, req: &IssueCreateRequest<'_>) -> Result<CreatedIssue> {
  parse_created_issue(&run_gh(issue_create_argv(slug, req))?)
}

/// Argv for `gh issue create …`.
pub fn issue_create_argv(slug: &str, req: &IssueCreateRequest<'_>) -> Vec<OsString> {
  let mut args: Vec<OsString> = Vec::with_capacity(8 + 2 * req.labels.len());
  args.push("issue".into());
  args.push("create".into());
  args.push("--title".into());
  args.push(req.title.into());
  args.push("--body-file".into());
  args.push(req.body_file.as_os_str().to_owned());
  for label in req.labels {
    args.push("--label".into());
    args.push(label.into());
  }
  // An empty slug means `origin` was unresolvable; `gh` then infers the
  // repo from the local git context, which is the pre-#419 behaviour this
  // path has always relied on.
  if !slug.is_empty() {
    args.push("--repo".into());
    args.push(slug.into());
  }
  args
}

/// Recover the created issue from the URL `gh issue create` prints.
pub fn parse_created_issue(stdout: &str) -> Result<CreatedIssue> {
  let stdout = stdout.trim().to_string();
  let Some(caps) = ISSUE_URL_RE.captures(&stdout) else {
    return Err(GwmError::CommandFailed(format!(
      "gh issue create did not print an issue URL containing a number: {}",
      stdout
    )));
  };
  let number = caps
    .get(1)
    .and_then(|m| m.as_str().parse::<u64>().ok())
    .ok_or_else(|| GwmError::CommandFailed(format!("failed to parse issue number from gh output: {}", stdout)))?;
  Ok(CreatedIssue { number, url: stdout })
}

/// Shell out to `gh pr create` with a body file already rendered by
/// [`crate::pr_templates::render_pr_body`]. Parses the URL printed by
/// gh on success to extract the PR number.
pub fn create_pr(slug: &str, req: &PrCreateRequest<'_>) -> Result<CreatedPr> {
  parse_created_pr(&run_gh(pr_create_argv(slug, req))?)
}

/// Argv for `gh pr create …`.
pub fn pr_create_argv(slug: &str, req: &PrCreateRequest<'_>) -> Vec<OsString> {
  let mut args: Vec<OsString> =
    Vec::with_capacity(10 + if req.draft { 1 } else { 0 } + if req.base.is_some() { 2 } else { 0 });
  args.push("pr".into());
  args.push("create".into());
  args.push("--title".into());
  args.push(req.title.into());
  args.push("--body-file".into());
  args.push(req.body_file.as_os_str().to_owned());
  args.push("--head".into());
  args.push(req.head.into());
  if let Some(base) = req.base {
    args.push("--base".into());
    args.push(base.into());
  }
  if req.draft {
    args.push("--draft".into());
  }
  // An empty slug means `origin` was unresolvable; `gh` then infers the
  // repo from the local git context, which is the pre-#419 behaviour this
  // path has always relied on.
  if !slug.is_empty() {
    args.push("--repo".into());
    args.push(slug.into());
  }
  args
}

/// Recover the created PR from the URL `gh pr create` prints.
pub fn parse_created_pr(stdout: &str) -> Result<CreatedPr> {
  let stdout = stdout.trim().to_string();
  let Some(caps) = PR_URL_RE.captures(&stdout) else {
    return Err(GwmError::CommandFailed(format!(
      "gh pr create did not print a PR URL containing a number: {}",
      stdout
    )));
  };
  let number = caps
    .get(1)
    .and_then(|m| m.as_str().parse::<u64>().ok())
    .ok_or_else(|| GwmError::CommandFailed(format!("failed to parse PR number from gh output: {}", stdout)))?;
  Ok(CreatedPr { number, url: stdout })
}

/// Run `gh pr view <n> --repo <slug> --json …` and parse the result.
pub fn fetch_pr(slug: &str, number: u64) -> Result<PrStatus> {
  fetch_pr_with(&gh_program(), slug, number)
}

/// [`fetch_pr`] with an explicitly resolved `gh` program path — PR-side
/// counterpart to [`fetch_issue_with`], used by the TUI off-thread fetch
/// (issue #217).
pub fn fetch_pr_with(program: &OsStr, slug: &str, number: u64) -> Result<PrStatus> {
  parse_pr_json(&run_gh_with(program, pr_view_argv(slug, number))?)
}

/// Argv for `gh pr view <n> --repo <slug> --json …`.
pub fn pr_view_argv(slug: &str, number: u64) -> Vec<String> {
  vec![
    "pr".into(),
    "view".into(),
    number.to_string(),
    "--repo".into(),
    slug.into(),
    "--json".into(),
    PR_JSON_FIELDS.into(),
  ]
}

#[derive(Deserialize)]
struct RawPrHead {
  number: u64,
  // `Option` (not just `#[serde(default)]`) so an explicit `"author": null`
  // — a deleted GitHub account — deserialises to `None` instead of erroring;
  // `default` alone only covers a *missing* key.
  #[serde(default)]
  author: Option<RawAuthor>,
  #[serde(rename = "headRefName", default)]
  head_ref_name: String,
  #[serde(rename = "baseRefName", default)]
  base_ref_name: String,
}

#[derive(Deserialize, Default)]
struct RawAuthor {
  #[serde(default)]
  login: String,
}

const PR_HEAD_JSON_FIELDS: &str = "number,author,headRefName,baseRefName";

/// Parse the JSON from `gh pr view <n> --json number,author,headRefName,baseRefName`.
/// Kept pure + `pub` so its shape is unit-testable without spawning `gh`.
pub fn parse_pr_head_json(s: &str) -> Result<PrHead> {
  let raw: RawPrHead = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "pr head",
    source: e,
  })?;
  Ok(PrHead {
    number: raw.number,
    author: raw.author.unwrap_or_default().login,
    head_ref_name: raw.head_ref_name,
    base_ref_name: raw.base_ref_name,
  })
}

/// Run `gh pr view <n> --repo <slug> --json …` and parse the head metadata
/// `gwm review` needs (author / head ref / base ref). Works for PRs in any
/// state — open, draft, closed, or merged.
pub fn fetch_pr_head(slug: &str, number: u64) -> Result<PrHead> {
  parse_pr_head_json(&run_gh(pr_head_argv(slug, number))?)
}

/// Argv for `gh pr view <n> --repo <slug> --json number,author,headRefName,baseRefName`.
pub fn pr_head_argv(slug: &str, number: u64) -> Vec<String> {
  vec![
    "pr".into(),
    "view".into(),
    number.to_string(),
    "--repo".into(),
    slug.into(),
    "--json".into(),
    PR_HEAD_JSON_FIELDS.into(),
  ]
}

/// Find the most recent PR opened from `branch` (head ref) on the given
/// repo, regardless of state. Returns `Ok(Some(N))` if at least one PR
/// exists (open, draft, closed, or merged — `gh pr list --state all`),
/// `Ok(None)` otherwise. Callers that need state-aware filtering should
/// pair this with `fetch_pr` to inspect `PrState` afterwards.
pub fn find_pr_for_branch(slug: &str, branch: &str) -> Result<Option<u64>> {
  let stdout = run_gh(find_pr_argv(slug, branch))?;
  parse_pr_list_number(&stdout)
}

/// Argv for `gh pr list --repo <slug> --head <branch> --state all --json
/// number --limit 1`. Extracted so the test suite can pin the `gh`
/// contract without shelling out; [`find_pr_for_branch`] is the caller
/// that actually invokes it. `--state all` is the load-bearing bit: a
/// closed or merged PR for the branch is still detected (its `PrState`
/// is resolved later via [`fetch_pr`]).
pub fn find_pr_argv(slug: &str, branch: &str) -> Vec<String> {
  vec![
    "pr".into(),
    "list".into(),
    "--repo".into(),
    slug.into(),
    "--head".into(),
    branch.into(),
    "--state".into(),
    "all".into(),
    "--json".into(),
    "number".into(),
    "--limit".into(),
    "1".into(),
  ]
}

/// Parse the JSON array printed by `gh pr list --json number --limit 1`,
/// returning the first PR number if any. Exposed for unit tests so the
/// parse contract is covered without a `gh` shell-out.
pub fn parse_pr_list_number(s: &str) -> Result<Option<u64>> {
  #[derive(Deserialize)]
  struct PrRef {
    number: u64,
  }
  let arr: Vec<PrRef> = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "pr list",
    source: e,
  })?;
  Ok(arr.into_iter().next().map(|p| p.number))
}

fn run_gh<I, S>(args: I) -> Result<String>
where
  I: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  run_gh_with(&gh_program(), args)
}

/// [`run_gh`] against an explicitly resolved `gh` program. Lets callers on
/// a worker thread (issue #217) avoid re-reading `GWM_GH` / the process
/// environment concurrently with env-mutating code on other threads. The
/// spawn + logging + error shape is shared with the GitLab backend since
/// #419 — see [`forge::run_cli`].
fn run_gh_with<I, S>(program: &OsStr, args: I) -> Result<String>
where
  I: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  forge::run_cli(program, args)
}

// ---- Labels (issue #81) -------------------------------------------------

const LABEL_JSON_FIELDS: &str = "name,color,description";
const LABEL_LIST_LIMIT: &str = "1000";

#[derive(Deserialize)]
struct RawLabel2 {
  name: String,
  /// `color` is a documented gh-CLI invariant — every label always
  /// carries one. We deliberately do NOT mark this `#[serde(default)]`:
  /// if a future gh contract change drops the field, we want a hard
  /// parse error rather than a silent empty-string that would flag
  /// every remote label as a colour mismatch in the diff. (Copilot
  /// review on PR #90.)
  color: String,
  #[serde(default)]
  description: Option<String>,
}

/// Parse the JSON returned by `gh label list --json name,color,description`.
/// Exposed publicly so unit tests can cover the contract without
/// shelling out. Two normalisations happen here so callers get a
/// uniformly-shaped `RemoteLabel`:
///
/// - **`color`** is lowercased. GitHub serialises hex colours in
///   either case; the diff engine expects the lowercase form, and
///   normalising at the parse boundary means downstream code never
///   has to think about it.
/// - **`description`** is left as-is. An empty `""` from GitHub
///   round-trips as `Some("")`; the labels-diff module collapses
///   empty strings to `None` on its own.
pub fn parse_labels_json(s: &str) -> Result<Vec<RemoteLabel>> {
  let raw: Vec<RawLabel2> = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "labels",
    source: e,
  })?;
  Ok(
    raw
      .into_iter()
      .map(|r| RemoteLabel {
        name: r.name,
        description: r.description,
        color: r.color.to_ascii_lowercase(),
      })
      .collect(),
  )
}

/// Argv for `gh label list --repo <slug> --json name,color,description --limit 1000`.
/// Extracted so the test suite can pin the contract; callers should
/// prefer `fetch_remote_labels` which actually shells out.
pub fn label_list_argv(slug: &str) -> Vec<String> {
  vec![
    "label".into(),
    "list".into(),
    "--repo".into(),
    slug.into(),
    "--json".into(),
    LABEL_JSON_FIELDS.into(),
    "--limit".into(),
    LABEL_LIST_LIMIT.into(),
  ]
}

/// Argv for `gh label create <name> --color <hex> [--description <desc>] --force --repo <slug>`.
/// The `--force` flag is the key contract bit: GitHub's CLI uses it
/// to mean "create OR update", which is exactly what `gwm labels
/// push` needs (no separate "edit" call). When `description` is
/// `None` we omit the flag entirely rather than pass `""` — gh would
/// otherwise wipe an existing description that the user didn't intend
/// to touch.
pub fn label_create_argv(slug: &str, spec: &LabelSpec) -> Vec<String> {
  let mut argv = vec![
    "label".into(),
    "create".into(),
    spec.name.clone(),
    "--repo".into(),
    slug.into(),
    "--color".into(),
    spec.color.clone(),
    "--force".into(),
  ];
  if let Some(desc) = spec.description.as_ref().filter(|s| !s.is_empty()) {
    argv.push("--description".into());
    argv.push(desc.clone());
  }
  argv
}

/// Argv for `gh label delete <name> --repo <slug> --yes`. The `--yes`
/// flag bypasses the interactive confirm prompt; without it gh blocks
/// on a TTY read and `gwm labels push --prune` hangs.
pub fn label_delete_argv(slug: &str, name: &str) -> Vec<String> {
  vec![
    "label".into(),
    "delete".into(),
    name.into(),
    "--repo".into(),
    slug.into(),
    "--yes".into(),
  ]
}

/// Run `gh label list --repo <slug> --json …` and parse the result.
/// Returns an empty vec when the remote has no labels (which is
/// distinct from "gh not installed" — that surfaces as
/// `CommandFailed`).
pub fn fetch_remote_labels(slug: &str) -> Result<Vec<RemoteLabel>> {
  let argv = label_list_argv(slug);
  let args: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
  let stdout = run_gh(&args)?;
  parse_labels_json(&stdout)
}

/// Push one label upstream via `gh label create --force`. Returns
/// `Ok(())` on success; the caller is responsible for tracking which
/// label was created vs. updated (the diff already knows).
pub fn push_label(slug: &str, spec: &LabelSpec) -> Result<()> {
  let argv = label_create_argv(slug, spec);
  let args: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
  run_gh(&args)?;
  Ok(())
}

/// Delete one label on the remote via `gh label delete --yes`. Used
/// by `gwm labels push --prune` for labels declared on the remote but
/// not in `.gwm.toml`.
///
/// Validates `name` through [`crate::labels::validate_label_name`]
/// BEFORE shelling out (issue #100). The argv-injection vector that
/// motivates `validate_label_name` for declared labels (config side)
/// applies equally to the prune path: `gh label delete <name>` takes
/// the name positionally, so a remote label whose name starts with
/// `-` (planted by an attacker who can edit the upstream label set,
/// or by an unrelated tool predating the validator) would be parsed
/// as a flag — `-h` no-ops the delete with a help banner, `--repo
/// other/repo` retargets the operation. We refuse the prune with a
/// scoped error instead of running the risky argv.
pub fn delete_label(slug: &str, name: &str) -> Result<()> {
  validate_remote_label_name(name)?;
  let argv = label_delete_argv(slug, name);
  let args: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
  run_gh(&args)?;
  Ok(())
}

/// Refuse a hostile remote label name before it reaches an argv slot
/// (issue #100). `gh label delete <name>` takes the name positionally, so
/// a remote label starting with `-` would be parsed as a flag: `-h` no-ops
/// the delete with a help banner, `--repo other/repo` retargets it.
fn validate_remote_label_name(name: &str) -> Result<()> {
  crate::labels::validate_label_name(name).map_err(|e| {
    let inner = match e {
      GwmError::Config(msg) => msg,
      other => other.to_string(),
    };
    GwmError::Config(format!(
      "labels (remote): {} — refusing to delete via `gh label delete`",
      inner
    ))
  })
}

// ---- Milestones (issue #82) ---------------------------------------------

const MILESTONE_PER_PAGE: &str = "100";

#[derive(Deserialize)]
struct RawMilestone {
  number: u64,
  title: String,
  /// Always present in the documented schema. Like `RawLabel2.color`
  /// for labels, we deliberately do NOT mark this `#[serde(default)]`:
  /// a contract change would surface as a hard parse error rather than
  /// silently flagging every remote milestone as a state mismatch.
  state: String,
  #[serde(default)]
  description: Option<String>,
  #[serde(default)]
  due_on: Option<String>,
}

/// Parse the JSON returned by `gh api repos/:owner/:repo/milestones?state=all`.
/// Exposed publicly so unit tests can cover the contract without
/// shelling out. The `state` field is mapped to the strict
/// `MilestoneState` enum — an unknown value is a hard error rather
/// than a silent third state on the diff side.
pub fn parse_milestones_json(s: &str) -> Result<Vec<RemoteMilestone>> {
  let raw: Vec<RawMilestone> = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "milestones",
    source: e,
  })?;
  raw
    .into_iter()
    .map(|r| {
      let state = match r.state.as_str() {
        "open" => MilestoneState::Open,
        "closed" => MilestoneState::Closed,
        other => {
          return Err(GwmError::Other(format!(
            "milestone '{}' has unknown state '{}': expected 'open' or 'closed'",
            r.title, other
          )))
        }
      };
      Ok(RemoteMilestone {
        number: r.number,
        title: r.title,
        description: r.description,
        due_on: r.due_on,
        state,
      })
    })
    .collect()
}

/// Argv for `gh api --paginate repos/<slug>/milestones?state=all&per_page=100`.
///
/// Two contract bits worth pinning:
/// - `state=all` — without it, the default endpoint only lists `open`
///   milestones and `gwm milestones push --prune` would silently
///   leave closed ones in place.
/// - `--paginate` — GitHub caps `per_page` at 100. Without paginating
///   we'd diff against a truncated remote set for repos with more
///   than 100 milestones, leading to bogus `create` rows and a
///   dangerously confusing `--prune` (Copilot review on PR #92).
pub fn milestone_list_argv(slug: &str) -> Vec<String> {
  vec![
    "api".into(),
    "--paginate".into(),
    format!("repos/{}/milestones?state=all&per_page={}", slug, MILESTONE_PER_PAGE),
  ]
}

/// Argv for `gh api -X POST repos/<slug>/milestones -f title=… [-f
/// description=…] [-f due_on=…] -f state=…`. Each optional field is
/// omitted entirely when absent — `gh` would otherwise wipe the
/// existing remote value.
pub fn milestone_create_argv(slug: &str, spec: &MilestoneSpec) -> Vec<String> {
  let mut argv = vec![
    "api".into(),
    "-X".into(),
    "POST".into(),
    format!("repos/{}/milestones", slug),
    "-f".into(),
    format!("title={}", spec.title),
    "-f".into(),
    format!("state={}", spec.state.as_str()),
  ];
  if let Some(desc) = spec.description.as_ref().filter(|s| !s.is_empty()) {
    argv.push("-f".into());
    argv.push(format!("description={}", desc));
  }
  if let Some(due) = spec.due_on.as_ref().filter(|s| !s.is_empty()) {
    argv.push("-f".into());
    argv.push(format!("due_on={}", due));
  }
  argv
}

/// Argv for `gh api -X PATCH repos/<slug>/milestones/<number> -f …`.
/// Same omission rules as `milestone_create_argv`: absent optionals
/// are skipped so the remote value isn't wiped.
pub fn milestone_update_argv(slug: &str, number: u64, spec: &MilestoneSpec) -> Vec<String> {
  let mut argv = vec![
    "api".into(),
    "-X".into(),
    "PATCH".into(),
    format!("repos/{}/milestones/{}", slug, number),
    "-f".into(),
    format!("title={}", spec.title),
    "-f".into(),
    format!("state={}", spec.state.as_str()),
  ];
  if let Some(desc) = spec.description.as_ref().filter(|s| !s.is_empty()) {
    argv.push("-f".into());
    argv.push(format!("description={}", desc));
  }
  if let Some(due) = spec.due_on.as_ref().filter(|s| !s.is_empty()) {
    argv.push("-f".into());
    argv.push(format!("due_on={}", due));
  }
  argv
}

/// Argv for `gh api -X DELETE repos/<slug>/milestones/<number>`.
/// `gh api -X DELETE` is non-interactive by construction (no TTY
/// confirm), so there's no `--yes` equivalent to add.
pub fn milestone_delete_argv(slug: &str, number: u64) -> Vec<String> {
  vec![
    "api".into(),
    "-X".into(),
    "DELETE".into(),
    format!("repos/{}/milestones/{}", slug, number),
  ]
}

/// Run `gh api repos/<slug>/milestones?state=all` and parse the
/// result. Returns an empty vec when the remote has no milestones.
pub fn fetch_remote_milestones(slug: &str) -> Result<Vec<RemoteMilestone>> {
  let argv = milestone_list_argv(slug);
  let args: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
  let stdout = run_gh(&args)?;
  parse_milestones_json(&stdout)
}

/// Create one milestone upstream via `gh api -X POST`. Returns
/// `Ok(())` — the caller already has the spec; we don't bother
/// parsing the response back into a `RemoteMilestone`.
pub fn create_milestone(slug: &str, spec: &MilestoneSpec) -> Result<()> {
  let argv = milestone_create_argv(slug, spec);
  let args: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
  run_gh(&args)?;
  Ok(())
}

/// Update one milestone upstream via `gh api -X PATCH`. `number` is
/// the GitHub-issued identifier carried through `MilestoneUpdate`.
pub fn update_milestone(slug: &str, number: u64, spec: &MilestoneSpec) -> Result<()> {
  let argv = milestone_update_argv(slug, number, spec);
  let args: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
  run_gh(&args)?;
  Ok(())
}

/// Delete one milestone on the remote via `gh api -X DELETE`. Used
/// by `gwm milestones push --prune` for milestones declared on the
/// remote but not in `.gwm.toml`.
pub fn delete_milestone(slug: &str, number: u64) -> Result<()> {
  let argv = milestone_delete_argv(slug, number);
  let args: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
  run_gh(&args)?;
  Ok(())
}

// ---- The Forge backend (issue #419) -------------------------------------

/// GitHub implementation of [`Forge`], shelling out to `gh`.
///
/// A thin binding over the free functions above rather than a rewrite:
/// they were already the GitHub backend in all but name, and keeping them
/// `pub` means the extraction reads as a no-op for the existing tests
/// that pin the `gh` argv contract.
#[derive(Debug, Clone)]
pub struct GitHubForge {
  origin: forge::RemoteRef,
  program: OsString,
  env: Vec<(String, String)>,
  workdir: Option<std::path::PathBuf>,
}

impl GitHubForge {
  /// Resolves `$GWM_GH` **now**, on the calling thread, so a forge handed
  /// to the TUI's fetch worker never re-reads the process environment
  /// concurrently with env-mutating code (issue #217).
  pub fn new(origin: forge::RemoteRef, workdir: Option<std::path::PathBuf>) -> Self {
    Self {
      env: gh_env(&origin),
      origin,
      program: gh_program(),
      workdir,
    }
  }

  fn run<I, S>(&self, args: I) -> Result<String>
  where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
  {
    forge::run_cli_with(
      &self.program,
      args,
      &forge::CliSpawn {
        env: &self.env,
        cwd: self.workdir.as_deref(),
        redact_after: &[],
      },
    )
  }
}

/// Environment pinned on every `gh` spawn (Codex review #458).
///
/// `$GH_HOST` selects the GitHub instance. Before #419 the slug parser
/// rejected anything that was not github.com, so a GitHub Enterprise host
/// could not reach this code at all; host-agnostic parsing opened that
/// door, and without the pin `gh` would silently target github.com and
/// could read a same-named repo on the wrong tenant.
///
/// github.com is pinned like any other host, deliberately: the child
/// inherits gwm's environment, so a user's ambient `GH_HOST` — routine for
/// enterprise users — would otherwise retarget a github.com repo, since
/// the argv only ever carries `--repo owner/repo` and never a hostname
/// (Codex review #458, round 3).
///
/// The host is pinned whenever a slug is known — including github.com,
/// and including a **guessed** (SSH) origin. Both were exempted at some
/// point and both exemptions were wrong (Codex review #458):
///
/// - The child inherits gwm's environment, so an ambient `GH_HOST` —
///   routine for enterprise users — retargets every call, since the argv
///   only ever carries `--repo owner/repo` and never a hostname. Knowing
///   the repo is on github.com, gwm says so rather than letting the
///   environment decide.
/// - `gh` cannot be steered any other way: `gh api repos/<slug>/…` bakes
///   the slug into the request path, so unlike `glab` it has no working
///   directory to fall back to. This is the one place the two backends
///   diverge — see [`crate::gitlab::glab_env`], where a guessed origin is
///   deliberately left alone because a distinct SSH hostname *is* a
///   documented GitLab pattern.
///
/// Nothing is pinned only when the slug is empty: that is the caller
/// asking `gh` to infer the project locally.
pub fn gh_env(origin: &forge::RemoteRef) -> Vec<(String, String)> {
  if origin.path.is_empty() {
    return Vec::new();
  }
  vec![("GH_HOST".to_string(), origin.authority().to_string())]
}

impl Forge for GitHubForge {
  fn kind(&self) -> ForgeKind {
    ForgeKind::GitHub
  }

  fn slug(&self) -> &str {
    &self.origin.path
  }

  fn web_origin(&self) -> &str {
    &self.origin.web_origin
  }

  fn workdir(&self) -> Option<&std::path::Path> {
    self.workdir.as_deref()
  }

  fn origin_is_authoritative(&self) -> bool {
    self.origin.trust == forge::OriginTrust::FromUrl
  }

  /// Always the slug: `gh` is pinned by `$GH_HOST` even for a guessed
  /// origin (see [`gh_env`]), so there is no ambiguity to defer to the
  /// working directory — and `gh api repos/<slug>/…` could not defer
  /// anyway, the slug being part of the request path.
  fn repo_selector(&self) -> &str {
    &self.origin.path
  }

  fn issue_url(&self, number: u64) -> String {
    format!("{}/{}/issues/{}", self.origin.web_origin, self.origin.path, number)
  }

  fn pr_url(&self, number: u64) -> String {
    format!("{}/{}/pull/{}", self.origin.web_origin, self.origin.path, number)
  }

  fn pr_head_refspec(&self, number: u64) -> String {
    format!("pull/{number}/head")
  }

  // Every method below goes through `self.run`, never the free functions,
  // so `$GH_HOST` reaches the child (Codex review #458). The free functions
  // stay for the argv/parse contract the test suite pins.

  fn fetch_issue(&self, number: u64) -> Result<IssueStatus> {
    parse_issue_json(&self.run(issue_view_argv(&self.origin.path, number))?)
  }

  fn fetch_pr(&self, number: u64) -> Result<PrStatus> {
    parse_pr_json(&self.run(pr_view_argv(&self.origin.path, number))?)
  }

  fn fetch_pr_head(&self, number: u64) -> Result<PrHead> {
    parse_pr_head_json(&self.run(pr_head_argv(&self.origin.path, number))?)
  }

  fn find_pr_for_branch(&self, branch: &str) -> Result<Option<u64>> {
    parse_pr_list_number(&self.run(find_pr_argv(&self.origin.path, branch))?)
  }

  fn create_issue(&self, req: &IssueCreateRequest<'_>) -> Result<CreatedIssue> {
    parse_created_issue(&self.run(issue_create_argv(&self.origin.path, req))?)
  }

  fn create_pr(&self, req: &PrCreateRequest<'_>) -> Result<CreatedPr> {
    parse_created_pr(&self.run(pr_create_argv(&self.origin.path, req))?)
  }

  fn fetch_remote_labels(&self) -> Result<Vec<RemoteLabel>> {
    parse_labels_json(&self.run(label_list_argv(&self.origin.path))?)
  }

  fn create_label(&self, spec: &LabelSpec) -> Result<()> {
    // `gh label create --force` means "create OR update", so both halves
    // of the trait's create/update split land on the same call here. The
    // split exists for GitLab, which has no such flag.
    self.run(label_create_argv(&self.origin.path, spec))?;
    Ok(())
  }

  fn update_label(&self, spec: &LabelSpec) -> Result<()> {
    self.create_label(spec)
  }

  fn delete_label(&self, name: &str) -> Result<()> {
    validate_remote_label_name(name)?;
    self.run(label_delete_argv(&self.origin.path, name))?;
    Ok(())
  }

  fn fetch_remote_milestones(&self) -> Result<Vec<RemoteMilestone>> {
    parse_milestones_json(&self.run(milestone_list_argv(&self.origin.path))?)
  }

  fn create_milestone(&self, spec: &MilestoneSpec) -> Result<()> {
    self.run(milestone_create_argv(&self.origin.path, spec))?;
    Ok(())
  }

  fn update_milestone(&self, number: u64, spec: &MilestoneSpec) -> Result<()> {
    self.run(milestone_update_argv(&self.origin.path, number, spec))?;
    Ok(())
  }

  fn delete_milestone(&self, number: u64) -> Result<()> {
    self.run(milestone_delete_argv(&self.origin.path, number))?;
    Ok(())
  }
}
