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
use crate::labels::{LabelSpec, RemoteLabel};
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

/// Find the most recent PR opened from `branch` (head ref) on the given
/// repo, regardless of state. Returns `Ok(Some(N))` if at least one PR
/// exists (open, draft, closed, or merged — `gh pr list --state all`),
/// `Ok(None)` otherwise. Callers that need state-aware filtering should
/// pair this with `fetch_pr` to inspect `PrState` afterwards.
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
  let raw: Vec<RawLabel2> =
    serde_json::from_str(s).map_err(|e| GwmError::Other(format!("failed to parse labels json: {}", e)))?;
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
pub fn delete_label(slug: &str, name: &str) -> Result<()> {
  let argv = label_delete_argv(slug, name);
  let args: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
  run_gh(&args)?;
  Ok(())
}
