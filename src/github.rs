//! Issue ↔ PR ↔ branch link storage + GitHub API fetch (via `gh` CLI).
//!
//! Storage lives in git branch config: `branch.<name>.gwm-issue` and
//! `branch.<name>.gwm-pr`. Issue numbers are auto-detected from the
//! `<type>/#<N>-<slug>` branch convention when no explicit override is set.
//!
//! Fetch shells out to `gh` and parses its JSON output. The parsing functions
//! (`parse_issue_json`, `parse_pr_json`) are exposed publicly so tests can
//! cover the JSON contract without depending on a real `gh` binary.

use crate::error::{GwmError, Result};
use crate::naming::parse_branch;
use git2::Repository;
use serde::Deserialize;
use std::process::Command;

const ISSUE_CONFIG_KEY: &str = "gwm-issue";
const PR_CONFIG_KEY: &str = "gwm-pr";

/// Where the issue or PR number came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSource {
  /// No link known (no branch-name match and no explicit override).
  None,
  /// Inferred from a branch following `<type>/#<N>-<slug>`.
  BranchName,
  /// Explicit override set via `gwm link …` (lives in git branch config).
  Explicit,
}

/// Resolved link for one branch: which issue (if any), which PR (if any),
/// and where each number came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchLink {
  pub issue: Option<u64>,
  pub pr: Option<u64>,
  pub issue_source: LinkSource,
  pub pr_source: LinkSource,
}

impl BranchLink {
  pub fn empty() -> Self {
    Self {
      issue: None,
      pr: None,
      issue_source: LinkSource::None,
      pr_source: LinkSource::None,
    }
  }

  /// One-line human-readable rendering for the CLI / TUI status bar.
  pub fn summary(&self) -> String {
    match (self.issue, self.pr) {
      (None, None) => "no link".into(),
      (Some(i), None) => format!("issue #{i}"),
      (None, Some(p)) => format!("PR #{p}"),
      (Some(i), Some(p)) => format!("issue #{i} · PR #{p}"),
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

  let (pr, pr_source) = match explicit_pr {
    Some(n) => (Some(n), LinkSource::Explicit),
    None => (None, LinkSource::None),
  };

  Ok(BranchLink {
    issue,
    pr,
    issue_source,
    pr_source,
  })
}

pub fn link_issue(repo: &Repository, branch: &str, number: u64) -> Result<()> {
  write_branch_u64(repo, branch, ISSUE_CONFIG_KEY, number)
}

pub fn link_pr(repo: &Repository, branch: &str, number: u64) -> Result<()> {
  write_branch_u64(repo, branch, PR_CONFIG_KEY, number)
}

pub fn unlink_issue(repo: &Repository, branch: &str) -> Result<()> {
  remove_branch_key(repo, branch, ISSUE_CONFIG_KEY)
}

pub fn unlink_pr(repo: &Repository, branch: &str) -> Result<()> {
  remove_branch_key(repo, branch, PR_CONFIG_KEY)
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

fn write_branch_u64(repo: &Repository, branch: &str, leaf: &str, value: u64) -> Result<()> {
  let mut cfg = repo.config()?;
  cfg.set_str(&config_key(branch, leaf), &value.to_string())?;
  Ok(())
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

// ---- Repo slug from origin remote --------------------------------------

/// Extract the `owner/repo` slug from the `origin` remote URL.
/// Supports the two GitHub URL flavours: `git@github.com:owner/repo(.git)?`
/// and `https://github.com/owner/repo(.git)?`.
pub fn repo_slug(repo: &Repository) -> Result<String> {
  let remote = repo
    .find_remote("origin")
    .map_err(|_| GwmError::Other("no 'origin' remote configured".into()))?;
  let url = remote
    .url()
    .ok_or_else(|| GwmError::Other("origin remote has no URL (non-utf8?)".into()))?
    .to_string();
  parse_github_slug(&url)
}

fn parse_github_slug(url: &str) -> Result<String> {
  // SSH: git@github.com:owner/repo(.git)?
  if let Some(rest) = url.strip_prefix("git@github.com:") {
    return Ok(trim_git_suffix(rest).to_string());
  }
  // HTTPS: https://github.com/owner/repo(.git)?
  for prefix in ["https://github.com/", "http://github.com/"] {
    if let Some(rest) = url.strip_prefix(prefix) {
      return Ok(trim_git_suffix(rest).to_string());
    }
  }
  Err(GwmError::Other(format!(
    "origin '{}' is not a github URL (expected git@github.com:… or https://github.com/…)",
    url
  )))
}

fn trim_git_suffix(s: &str) -> &str {
  // Normalise trailing slashes first so `owner/repo.git/` becomes
  // `owner/repo.git` before the `.git` strip kicks in. Pre-fix this
  // returned `owner/repo.git` because `.git` was sought with a trailing
  // `/` still attached (Copilot PR #68 review).
  let trimmed = s.trim_end_matches('/');
  trimmed.strip_suffix(".git").unwrap_or(trimmed)
}

// ---- Issue / PR status ---------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
  Open,
  Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueStatus {
  pub number: u64,
  pub title: String,
  pub state: IssueState,
  pub url: String,
  pub labels: Vec<String>,
  pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
  Open,
  Draft,
  Closed,
  Merged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrStatus {
  pub number: u64,
  pub title: String,
  pub state: PrState,
  pub url: String,
  pub updated_at: String,
  pub checks_passed: u32,
  pub checks_total: u32,
}

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

#[derive(Deserialize)]
struct RawCheck {
  #[serde(default)]
  status: String,
  #[serde(default)]
  conclusion: Option<String>,
}

pub fn parse_issue_json(s: &str) -> Result<IssueStatus> {
  let raw: RawIssue =
    serde_json::from_str(s).map_err(|e| GwmError::Other(format!("failed to parse issue json: {}", e)))?;
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
  let raw: RawPr = serde_json::from_str(s).map_err(|e| GwmError::Other(format!("failed to parse PR json: {}", e)))?;
  let state = match (raw.state.as_str(), raw.is_draft) {
    ("MERGED" | "merged", _) => PrState::Merged,
    ("CLOSED" | "closed", _) => PrState::Closed,
    ("OPEN" | "open", true) => PrState::Draft,
    ("OPEN" | "open", false) => PrState::Open,
    (other, _) => return Err(GwmError::Other(format!("unknown PR state '{}'", other))),
  };
  let checks_total = raw.status_check_rollup.len() as u32;
  let checks_passed = raw
    .status_check_rollup
    .iter()
    .filter(|c| {
      c.status.eq_ignore_ascii_case("COMPLETED")
        && c
          .conclusion
          .as_deref()
          .is_some_and(|s| s.eq_ignore_ascii_case("SUCCESS"))
    })
    .count() as u32;
  Ok(PrStatus {
    number: raw.number,
    title: raw.title,
    state,
    url: raw.url,
    updated_at: raw.updated_at,
    checks_passed,
    checks_total,
  })
}

// ---- gh CLI invocation ---------------------------------------------------

const ISSUE_JSON_FIELDS: &str = "number,title,state,url,labels,updatedAt";
const PR_JSON_FIELDS: &str = "number,title,state,isDraft,url,updatedAt,statusCheckRollup";

/// Run `gh issue view <n> --repo <slug> --json …` and parse the result.
pub fn fetch_issue(slug: &str, number: u64) -> Result<IssueStatus> {
  let stdout = run_gh(&[
    "issue",
    "view",
    &number.to_string(),
    "--repo",
    slug,
    "--json",
    ISSUE_JSON_FIELDS,
  ])?;
  parse_issue_json(&stdout)
}

/// Run `gh pr view <n> --repo <slug> --json …` and parse the result.
pub fn fetch_pr(slug: &str, number: u64) -> Result<PrStatus> {
  let stdout = run_gh(&[
    "pr",
    "view",
    &number.to_string(),
    "--repo",
    slug,
    "--json",
    PR_JSON_FIELDS,
  ])?;
  parse_pr_json(&stdout)
}

/// Find the PR opened from `branch` (head ref) on the given repo. Returns
/// `Ok(Some(N))` if exactly one open PR is found, `Ok(None)` if none.
pub fn find_pr_for_branch(slug: &str, branch: &str) -> Result<Option<u64>> {
  let stdout = run_gh(&[
    "pr", "list", "--repo", slug, "--head", branch, "--state", "all", "--json", "number", "--limit", "1",
  ])?;
  #[derive(Deserialize)]
  struct PrRef {
    number: u64,
  }
  let arr: Vec<PrRef> =
    serde_json::from_str(&stdout).map_err(|e| GwmError::Other(format!("failed to parse pr list json: {}", e)))?;
  Ok(arr.into_iter().next().map(|p| p.number))
}

fn run_gh(args: &[&str]) -> Result<String> {
  let output = Command::new("gh")
    .args(args)
    .output()
    .map_err(|e| GwmError::CommandFailed(format!("gh: failed to spawn ({}). Is `gh` installed and on PATH?", e)))?;
  if !output.status.success() {
    return Err(GwmError::CommandFailed(format!(
      "gh exited {}: {}",
      output.status,
      String::from_utf8_lossy(&output.stderr).trim()
    )));
  }
  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Build the canonical GitHub URL for an issue, given the repo slug.
pub fn issue_url(slug: &str, number: u64) -> String {
  format!("https://github.com/{}/issues/{}", slug, number)
}

/// Build the canonical GitHub URL for a PR, given the repo slug.
pub fn pr_url(slug: &str, number: u64) -> String {
  format!("https://github.com/{}/pull/{}", slug, number)
}
