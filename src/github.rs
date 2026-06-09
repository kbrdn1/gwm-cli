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
use crate::milestones::{MilestoneSpec, MilestoneState, RemoteMilestone};
use crate::naming::parse_branch;
use git2::Repository;
use serde::Deserialize;
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

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

  Ok(BranchLink {
    issue,
    pr,
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
pub fn read_link_with_pr_detection(repo: &Repository, branch: &str, slug: &str) -> Result<BranchLink> {
  let mut link = read_link(repo, branch)?;
  if link.pr_source != LinkSource::Explicit {
    // Re-resolve live. On success, the fresh result replaces any persisted
    // detection (a vanished PR clears it); on a `gh` failure, keep whatever
    // `read_link` already resolved (possibly a persisted detection).
    if let Ok(detected) = find_pr_for_branch(slug, branch) {
      link.pr = detected;
      link.pr_source = match detected {
        Some(_) => LinkSource::Detected,
        None => LinkSource::None,
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
  write_branch_u64(repo, branch, ISSUE_CONFIG_KEY, number)
}

pub fn link_pr(repo: &Repository, branch: &str, number: u64) -> Result<()> {
  write_branch_u64(repo, branch, PR_CONFIG_KEY, number)
}

pub fn unlink_issue(repo: &Repository, branch: &str) -> Result<()> {
  remove_branch_key(repo, branch, ISSUE_CONFIG_KEY)
}

pub fn unlink_pr(repo: &Repository, branch: &str) -> Result<()> {
  // Drop both the explicit link and any persisted auto-detection (#283),
  // otherwise unlinking would leave a stale `gwm-pr-detected` number that
  // `read_link` would resurface as a `Detected` PR on the next read.
  remove_branch_key(repo, branch, PR_CONFIG_KEY)?;
  remove_branch_key(repo, branch, DETECTED_PR_CONFIG_KEY)
}

/// Persist an auto-detected PR number to its own branch-config key
/// (`gwm-pr-detected`, issue #283), distinct from the explicit `gwm-pr`.
/// This lets the no-fetch table read path surface the detected PR on every
/// row without a per-row `gh` shell-out, while keeping the
/// detected/explicit distinction the pane badge needs. An explicit
/// `gwm link --pr` still wins in [`read_link`]. Re-detection overwrites the
/// stored value.
pub fn persist_detected_pr(repo: &Repository, branch: &str, number: u64) -> Result<()> {
  write_branch_u64(repo, branch, DETECTED_PR_CONFIG_KEY, number)
}

/// Drop a persisted auto-detection (issue #283). A no-op when no detected
/// PR was stored. Used when a detection no longer holds (the branch's PR
/// went away) so a stale number doesn't linger in the config.
pub fn clear_persisted_detected_pr(repo: &Repository, branch: &str) -> Result<()> {
  remove_branch_key(repo, branch, DETECTED_PR_CONFIG_KEY)
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
    .ok()
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

#[derive(Debug, Clone)]
pub struct IssueCreateRequest<'a> {
  pub title: &'a str,
  pub body_file: &'a std::path::Path,
  pub labels: &'a [String],
  pub repo: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct CreatedIssue {
  pub number: u64,
  pub url: String,
}

#[derive(Debug, Clone)]
pub struct PrCreateRequest<'a> {
  pub title: &'a str,
  pub body_file: &'a std::path::Path,
  pub head: &'a str,
  pub base: Option<&'a str>,
  pub draft: bool,
  pub repo: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct CreatedPr {
  pub number: u64,
  pub url: String,
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
  fetch_issue_with(&gh_program(), slug, number)
}

/// [`fetch_issue`] with an explicitly resolved `gh` program path. Used by
/// the TUI's off-thread fetch (issue #217): the program is resolved on the
/// main thread via [`gh_program`] and handed to the worker thread, so the
/// thread never touches `GWM_GH` / the process environment concurrently
/// with env-mutating callers.
pub fn fetch_issue_with(program: &OsStr, slug: &str, number: u64) -> Result<IssueStatus> {
  let stdout = run_gh_with(
    program,
    [
      "issue",
      "view",
      &number.to_string(),
      "--repo",
      slug,
      "--json",
      ISSUE_JSON_FIELDS,
    ],
  )?;
  parse_issue_json(&stdout)
}

/// Resolve the `gh` program to invoke: `$GWM_GH` when set (test / override
/// hook), else `gh` on `PATH`. Read once on the calling thread so off-thread
/// fetches can capture it without re-reading the environment.
pub fn gh_program() -> OsString {
  std::env::var_os("GWM_GH").unwrap_or_else(|| "gh".into())
}

pub fn create_issue(req: &IssueCreateRequest<'_>) -> Result<CreatedIssue> {
  let mut args: Vec<OsString> = Vec::with_capacity(6 + 2 * req.labels.len() + if req.repo.is_some() { 2 } else { 0 });
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
  if let Some(repo) = req.repo {
    args.push("--repo".into());
    args.push(repo.into());
  }
  let stdout = run_gh(&args)?;
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
pub fn create_pr(req: &PrCreateRequest<'_>) -> Result<CreatedPr> {
  let mut args: Vec<OsString> = Vec::with_capacity(
    8 + if req.draft { 1 } else { 0 } + if req.base.is_some() { 2 } else { 0 } + if req.repo.is_some() { 2 } else { 0 },
  );
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
  if let Some(repo) = req.repo {
    args.push("--repo".into());
    args.push(repo.into());
  }
  let stdout = run_gh(&args)?;
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
  let stdout = run_gh_with(
    program,
    [
      "pr",
      "view",
      &number.to_string(),
      "--repo",
      slug,
      "--json",
      PR_JSON_FIELDS,
    ],
  )?;
  parse_pr_json(&stdout)
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

/// Build the human-readable command line stored on the Command Logs
/// transcript (issue #226) for a `gh` invocation: the program's *file name*
/// (so a `GWM_GH=/usr/bin/gh` override still reads as `gh issue view …`
/// rather than leaking the full path) followed by the resolved args. Kept
/// pure and `pub` so its argv format is unit-testable without spawning `gh`
/// (which CI runners do not have).
pub fn gh_command_line(program: &OsStr, args: &[OsString]) -> String {
  let name = Path::new(program)
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_else(|| program.to_string_lossy().into_owned());
  let mut line = name;
  for arg in args {
    line.push(' ');
    line.push_str(&arg.to_string_lossy());
  }
  line
}

/// [`run_gh`] against an explicitly resolved `gh` program. Lets callers on
/// a worker thread (issue #217) avoid re-reading `GWM_GH` / the process
/// environment concurrently with env-mutating code on other threads.
fn run_gh_with<I, S>(program: &OsStr, args: I) -> Result<String>
where
  I: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  // Collect the args once so they can both drive the spawn and build the
  // human-readable command line stored on the Command Logs transcript
  // (issue #226): the resolved `gh <args…>`, not an opaque handle.
  let collected: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_os_string()).collect();
  let cmdline = gh_command_line(program, &collected);
  let mut cmd = Command::new(program);
  cmd.args(&collected);
  let output = crate::command_log::run_logged(&mut cmd, cmdline)
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
  crate::labels::validate_label_name(name).map_err(|e| {
    let inner = match e {
      GwmError::Config(msg) => msg,
      other => other.to_string(),
    };
    GwmError::Config(format!(
      "labels (remote): {} — refusing to delete via `gh label delete`",
      inner
    ))
  })?;
  let argv = label_delete_argv(slug, name);
  let args: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
  run_gh(&args)?;
  Ok(())
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
