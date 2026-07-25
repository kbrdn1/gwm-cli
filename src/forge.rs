//! Forge abstraction (issue #419): the network surface gwm needs from a
//! code-hosting platform, plus the two backends that implement it —
//! GitHub via `gh` ([`crate::github`]) and GitLab via `glab`
//! ([`crate::gitlab`]).
//!
//! ## What lives here and what does not
//!
//! This module owns the **forge-agnostic types** ([`IssueStatus`],
//! [`PrStatus`], [`CiState`], …) and the [`Forge`] trait covering
//! **network operations only**. It deliberately does NOT own the
//! `persist_*` / `read_branch_*` family in [`crate::github`]: those write
//! to `branch.<x>.gwm-*` git-config keys whose names are already
//! forge-neutral, so both backends share them as-is. Keeping persistence
//! common roughly halves the surface that needed abstracting.
//!
//! ## Terminology
//!
//! GitHub says "pull request", GitLab says "merge request". The parsed
//! types stay `Pr*` internally (renaming them would touch every consumer
//! for no behavioural gain); user-facing strings go through
//! [`Forge::pr_noun`]. The TUI-wide rename of render labels and key hints
//! is tracked separately — this module only provides the seam.

use crate::config::Config;
use crate::error::{GwmError, Result};
use crate::labels::{LabelSpec, RemoteLabel};
use crate::milestones::{MilestoneSpec, RemoteMilestone};
use git2::Repository;
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

// ---- Forge-agnostic parsed types ----------------------------------------

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

/// Overall CI outcome derived from a PR's checks (issue #299). A single
/// ordered signal so the sidebar can render pass/fail/running at a glance
/// instead of a bare `N/M` count. Priority is **failing > running >
/// passing**: the most actionable state always wins, so a red check is
/// never hidden behind an in-flight one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiState {
  /// The PR has no checks at all — render nothing.
  None,
  /// Every check completed successfully (counting `NEUTRAL` / `SKIPPED`).
  Passing,
  /// At least one check is still in flight (or reported an outcome we do
  /// not recognise) and none has failed.
  Running,
  /// At least one check completed with a failing conclusion.
  Failing,
}

/// The outcome of a single check, before the per-PR aggregation.
/// Public since #436: the CI checks overlay renders one row per check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckOutcome {
  Passing,
  Running,
  Failing,
  /// The forge reported a state this build does not know (issue #419).
  /// An explicit variant rather than a `_ => Passing` catch-all: a new
  /// GitLab pipeline status must never be able to paint a green CI that
  /// is not green. Aggregates as non-green (see [`aggregate_ci_state`]).
  Unknown,
}

/// One classified check, kept per-check for the CI checks overlay
/// (issue #436). On GitHub this is one `statusCheckRollup` entry; on
/// GitLab, the MR's single `head_pipeline` synthesised into one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCheck {
  pub name: String,
  pub outcome: CheckOutcome,
  pub url: Option<String>,
  /// Owning workflow (`workflowName`, GitHub CheckRun shape only) —
  /// surfaced in the overlay's detail column (#436 validation feedback).
  pub workflow_name: Option<String>,
  /// RFC 3339 run timestamps: the overlay derives the run duration (or
  /// the elapsed time of an in-flight run) from them.
  pub started_at: Option<String>,
  pub completed_at: Option<String>,
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
  /// Overall CI state derived from the same checks that feed
  /// `checks_passed` / `checks_total` — no extra request.
  pub ci: CiState,
  /// The classified per-check list, same order as the forge returned it.
  pub checks: Vec<PrCheck>,
}

/// The slice of PR metadata `gwm review` needs to materialise a worktree:
/// the head ref name (slug source), the author login (path component),
/// and the base ref (diff base). Distinct from [`PrStatus`] so the TUI's
/// status/CI path stays untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrHead {
  pub number: u64,
  /// Author login, e.g. `alice` (`dependabot[bot]` for GitHub bot PRs).
  pub author: String,
  /// The PR's head branch name, e.g. `feat/spike-x`.
  pub head_ref_name: String,
  /// The PR's base branch name, e.g. `main`.
  pub base_ref_name: String,
}

#[derive(Debug, Clone)]
pub struct IssueCreateRequest<'a> {
  pub title: &'a str,
  pub body_file: &'a Path,
  pub labels: &'a [String],
}

#[derive(Debug, Clone)]
pub struct CreatedIssue {
  pub number: u64,
  pub url: String,
}

#[derive(Debug, Clone)]
pub struct PrCreateRequest<'a> {
  pub title: &'a str,
  pub body_file: &'a Path,
  pub head: &'a str,
  pub base: Option<&'a str>,
  pub draft: bool,
}

#[derive(Debug, Clone)]
pub struct CreatedPr {
  pub number: u64,
  pub url: String,
}

/// Collapse a per-check list into a single [`CiState`] with the priority
/// **failing > (running | unknown) > passing** (issue #299 / #419). A
/// failing check wins immediately; any pending *or unrecognised* check
/// downgrades an otherwise-green set to `Running`; an empty set is `None`.
pub fn aggregate_ci_state(outcomes: impl IntoIterator<Item = CheckOutcome>) -> CiState {
  let mut any_inconclusive = false;
  let mut any = false;
  for outcome in outcomes {
    any = true;
    match outcome {
      // Failing outranks everything — short-circuit so a red check is
      // never masked by a later in-flight one.
      CheckOutcome::Failing => return CiState::Failing,
      // `Unknown` rides with `Running` rather than getting its own
      // `CiState`: it must not read as green, and "not conclusive yet"
      // is the safe direction for a state we cannot classify.
      CheckOutcome::Running | CheckOutcome::Unknown => any_inconclusive = true,
      CheckOutcome::Passing => {}
    }
  }
  if !any {
    CiState::None
  } else if any_inconclusive {
    CiState::Running
  } else {
    CiState::Passing
  }
}

// ---- Forge kind ----------------------------------------------------------

/// Which forge backend drives the network calls. Set explicitly with
/// `forge = "github" | "gitlab"` in `.gwm.toml`, else inferred from the
/// `origin` host by [`detect_kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
  GitHub,
  GitLab,
}

impl ForgeKind {
  pub fn as_str(&self) -> &'static str {
    match self {
      Self::GitHub => "github",
      Self::GitLab => "gitlab",
    }
  }

  /// The CLI this backend shells out to. Surfaced by `gwm doctor` so a
  /// missing binary is reported against the forge actually in use.
  pub fn cli_name(&self) -> &'static str {
    match self {
      Self::GitHub => "gh",
      Self::GitLab => "glab",
    }
  }
}

// ---- origin remote parsing ----------------------------------------------

/// An `origin` URL split into the parts a forge needs: the host (which
/// selects the backend and roots every generated URL) and the repository
/// path.
///
/// `path` is NOT limited to `owner/repo`: a GitLab project can sit any
/// number of subgroups deep (`group/sub/deeper/proj`), so the whole path
/// is kept verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRef {
  pub host: String,
  pub path: String,
  /// Scheme + host + web port, e.g. `https://github.com` or
  /// `http://gitlab.acme:8080`. Every generated URL is rooted here rather
  /// than rebuilt as `https://{host}`, which silently broke self-hosted
  /// instances on plain HTTP or a non-default port (Codex review #458).
  ///
  /// Only an `http(s)://` remote contributes a port: on `ssh://host:2222`
  /// the port is the SSH port, and carrying it into a web URL would be
  /// just as wrong as dropping a real one.
  pub web_origin: String,
}

/// Parse any of the git remote URL flavours into [`RemoteRef`]:
///
/// - scp-like SSH: `git@host:group/proj.git`
/// - explicit scheme: `ssh://git@host:2222/group/proj.git`,
///   `https://host/group/proj`, `git://host/group/proj`
///
/// Pre-#419 this only accepted `github.com`; it is now host-agnostic so a
/// self-hosted instance parses, and the forge is chosen separately.
pub fn parse_remote_url(url: &str) -> Result<RemoteRef> {
  let url = url.trim();
  let scheme = url.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
  let (host_part, path_part) = split_host_and_path(url)
    .ok_or_else(|| GwmError::Other(format!("origin '{}' is not a recognised git remote URL", url)))?;

  // Drop any `user@` prefix, then split off a `:port` suffix.
  let authority = host_part.rsplit('@').next().unwrap_or(host_part);
  let (host, port) = match authority.rsplit_once(':') {
    // Only split a trailing `:port`, never an IPv6 segment.
    Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
    _ => (authority, None),
  };
  if host.is_empty() {
    return Err(GwmError::Other(format!("origin '{}' has no host", url)));
  }
  // The port is web-relevant only over http(s). An `ssh://…:2222` port
  // addresses sshd, not the web UI, and an scp-like remote cannot carry a
  // port at all.
  let web_origin = match scheme.as_deref() {
    Some("http") => format!("http://{}{}", host, port.map(|p| format!(":{p}")).unwrap_or_default()),
    Some("https") => format!("https://{}{}", host, port.map(|p| format!(":{p}")).unwrap_or_default()),
    _ => format!("https://{host}"),
  };

  let path = trim_git_suffix(path_part.trim_start_matches('/'));
  if path.is_empty() {
    return Err(GwmError::Other(format!("origin '{}' has no repository path", url)));
  }

  Ok(RemoteRef {
    host: host.to_ascii_lowercase(),
    path: path.to_string(),
    web_origin,
  })
}

/// Split a remote URL into `(authority, path)`. Handles both the
/// scheme-ful form and git's scp-like `host:path` shorthand.
fn split_host_and_path(url: &str) -> Option<(&str, &str)> {
  if let Some((_scheme, rest)) = url.split_once("://") {
    return rest.split_once('/');
  }
  // scp-like `[user@]host:path`. The `:` here separates host from path,
  // so it must not be confused with a `:port` (which scp syntax cannot
  // express anyway).
  url.split_once(':')
}

fn trim_git_suffix(s: &str) -> &str {
  // Normalise trailing slashes first so `owner/repo.git/` becomes
  // `owner/repo.git` before the `.git` strip kicks in.
  let trimmed = s.trim_end_matches('/');
  trimmed.strip_suffix(".git").unwrap_or(trimmed).trim_end_matches('/')
}

/// Infer the forge from the `origin` host.
///
/// Self-hosted GitLab lives on an arbitrary domain and **cannot** be
/// detected from the URL alone — the `gitlab.*` label convention is a
/// best-effort nicety, not a contract. `forge = "gitlab"` in `.gwm.toml`
/// is the supported way in, and it always wins (see [`resolve`]).
/// Anything unrecognised defaults to GitHub, preserving pre-#419
/// behaviour for GitHub Enterprise hosts.
pub fn detect_kind(host: &str) -> ForgeKind {
  let host = host.to_ascii_lowercase();
  if host == "gitlab.com" || host.starts_with("gitlab.") || host.contains(".gitlab.") {
    ForgeKind::GitLab
  } else {
    ForgeKind::GitHub
  }
}

// ---- the trait -----------------------------------------------------------

/// The network surface gwm needs from a forge.
///
/// Implementors carry their own resolved slug, host and CLI program, so
/// call sites pass numbers rather than threading a slug through every
/// call. `Send + Sync` is load-bearing: the TUI resolves the forge on the
/// main thread and moves an `Arc<dyn Forge>` into the fetch worker, which
/// is what keeps the worker from re-reading `GWM_GH` / `GWM_GLAB`
/// concurrently with env-mutating code (the race issue #217 fixed).
pub trait Forge: Send + Sync + std::fmt::Debug {
  fn kind(&self) -> ForgeKind;
  /// The repository path on the forge (`owner/repo`, or a nested
  /// `group/sub/proj` on GitLab).
  fn slug(&self) -> &str;
  /// Scheme + host + web port (`https://github.com`,
  /// `http://gitlab.acme:8080`). The root of every generated URL.
  fn web_origin(&self) -> &str;

  /// User-facing noun for a change proposal: `"PR"` or `"MR"`.
  fn pr_noun(&self) -> &'static str {
    match self.kind() {
      ForgeKind::GitHub => "PR",
      ForgeKind::GitLab => "MR",
    }
  }

  fn issue_url(&self, number: u64) -> String;
  fn pr_url(&self, number: u64) -> String;

  /// The `git fetch origin <spec>` left-hand side that resolves the
  /// change's head commit, for `gwm review`. Forge-specific and NOT
  /// interchangeable: GitHub publishes `refs/pull/<n>/head`, GitLab
  /// publishes `refs/merge-requests/<iid>/head` and no `refs/pull/*` at
  /// all, so a shared literal would fail the fetch *after* the metadata
  /// lookup already succeeded.
  fn pr_head_refspec(&self, number: u64) -> String;

  fn fetch_issue(&self, number: u64) -> Result<IssueStatus>;
  fn fetch_pr(&self, number: u64) -> Result<PrStatus>;
  fn fetch_pr_head(&self, number: u64) -> Result<PrHead>;
  /// The most recent PR/MR whose head (source) branch is `branch`,
  /// regardless of state. `Ok(None)` when there is none.
  fn find_pr_for_branch(&self, branch: &str) -> Result<Option<u64>>;

  fn create_issue(&self, req: &IssueCreateRequest<'_>) -> Result<CreatedIssue>;
  fn create_pr(&self, req: &PrCreateRequest<'_>) -> Result<CreatedPr>;

  fn fetch_remote_labels(&self) -> Result<Vec<RemoteLabel>>;
  /// Create a label that does not exist upstream. Split from
  /// [`Forge::update_label`] because GitLab has no create-or-update flag
  /// (GitHub's `gh label create --force`); the caller's diff already
  /// knows which side it is on.
  fn create_label(&self, spec: &LabelSpec) -> Result<()>;
  fn update_label(&self, spec: &LabelSpec) -> Result<()>;
  fn delete_label(&self, name: &str) -> Result<()>;

  fn fetch_remote_milestones(&self) -> Result<Vec<RemoteMilestone>>;
  fn create_milestone(&self, spec: &MilestoneSpec) -> Result<()>;
  fn update_milestone(&self, number: u64, spec: &MilestoneSpec) -> Result<()>;
  fn delete_milestone(&self, number: u64) -> Result<()>;
}

/// Build a backend for an explicit kind. Exposed so tests (and the
/// doctor) can exercise the pure parts — URL building, terminology —
/// without a repository.
pub fn for_kind(kind: ForgeKind, web_origin: String, slug: String) -> Arc<dyn Forge> {
  match kind {
    ForgeKind::GitHub => Arc::new(crate::github::GitHubForge::new(web_origin, slug)),
    ForgeKind::GitLab => Arc::new(crate::gitlab::GitLabForge::new(web_origin, slug)),
  }
}

/// Parse the `origin` remote of `repo`. The single place the remote URL
/// is read, shared by [`resolve`] and [`repo_slug`].
pub fn origin_ref(repo: &Repository) -> Result<RemoteRef> {
  let remote = repo
    .find_remote("origin")
    .map_err(|_| GwmError::Other("no 'origin' remote configured".into()))?;
  let url = remote
    .url()
    .ok()
    .ok_or_else(|| GwmError::Other("origin remote has no URL (non-utf8?)".into()))?
    .to_string();
  parse_remote_url(&url)
}

/// The repository path on the forge, from the `origin` remote. Forge-free
/// and config-free, for the display-only call sites that just need to
/// render `owner/repo` without deciding on a backend.
pub fn repo_slug(repo: &Repository) -> Result<String> {
  Ok(origin_ref(repo)?.path)
}

/// [`resolve`] that never fails, for the two creation paths (`gwm new`,
/// `gwm pr`) that predate #419 and deliberately tolerate a repo with no
/// `origin`: `gh` / `glab` can infer the project from the local git
/// context on their own, so the backend simply omits `--repo` when the
/// slug is empty. Every other call site uses the strict [`resolve`],
/// because an unresolvable slug there means an unbuildable URL.
pub fn resolve_or_default(repo: &Repository, config: &Config) -> Arc<dyn Forge> {
  resolve(repo, config).unwrap_or_else(|_| {
    let kind = config.forge.unwrap_or(ForgeKind::GitHub);
    let origin = match kind {
      ForgeKind::GitHub => "https://github.com",
      ForgeKind::GitLab => "https://gitlab.com",
    };
    for_kind(kind, origin.into(), String::new())
  })
}

/// Resolve the forge for `repo`: parse the `origin` remote, then pick the
/// backend from `.gwm.toml`'s `forge` key when set, else infer it from
/// the host.
pub fn resolve(repo: &Repository, config: &Config) -> Result<Arc<dyn Forge>> {
  let parsed = origin_ref(repo)?;
  let kind = config.forge.unwrap_or_else(|| detect_kind(&parsed.host));
  Ok(for_kind(kind, parsed.web_origin, parsed.path))
}

// ---- shared CLI invocation ----------------------------------------------

/// Build the human-readable command line stored on the Command Logs
/// transcript (issue #226): the program's *file name* (so a
/// `GWM_GH=/usr/bin/gh` override still reads as `gh issue view …` rather
/// than leaking the full path) followed by the resolved args. Kept pure
/// and `pub` so its argv format is unit-testable without spawning the CLI
/// (which CI runners do not have).
pub fn cli_command_line(program: &OsStr, args: &[OsString]) -> String {
  cli_command_line_redacted(program, args, &[])
}

/// [`cli_command_line`] that masks the value following any flag named in
/// `redact_after`.
///
/// `glab` has no `--body-file`, so a whole rendered issue / MR body rides
/// inline in `--description` (Codex review #458). The transcript is ours
/// to build, so the value is replaced by its length rather than pasted
/// into a log line the user can scroll and copy. The argv itself still
/// carries the body and is visible to `ps` — that one is `glab`'s CLI
/// surface, not something gwm can fix from here.
pub fn cli_command_line_redacted(program: &OsStr, args: &[OsString], redact_after: &[&str]) -> String {
  let mut line = program_name(program);
  let mut redact_next = false;
  for arg in args {
    let text = arg.to_string_lossy();
    line.push(' ');
    if redact_next {
      line.push_str(&format!("<redacted:{} chars>", text.chars().count()));
    } else {
      line.push_str(&text);
    }
    redact_next = redact_after.contains(&text.as_ref());
  }
  line
}

fn program_name(program: &OsStr) -> String {
  Path::new(program)
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_else(|| program.to_string_lossy().into_owned())
}

/// Spawn `program` with `args`, log the invocation, and return stdout.
/// Shared by both backends so the Command Logs transcript and the error
/// shape stay identical across forges.
pub fn run_cli<I, S>(program: &OsStr, args: I) -> Result<String>
where
  I: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  run_cli_with(program, args, &[], &[])
}

/// [`run_cli`] with extra environment for the child and a redaction list
/// for the transcript. Both exist for the GitLab backend: `$GITLAB_HOST`
/// pins the instance (otherwise `glab` resolves it from the *process* cwd
/// and falls back to gitlab.com), and `--description` carries a whole
/// rendered body that must not land verbatim in Command Logs.
pub fn run_cli_with<I, S>(program: &OsStr, args: I, env: &[(String, String)], redact_after: &[&str]) -> Result<String>
where
  I: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  // Collect the args once so they can both drive the spawn and build the
  // human-readable command line stored on the transcript (issue #226).
  let collected: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_os_string()).collect();
  let name = program_name(program);
  let cmdline = cli_command_line_redacted(program, &collected, redact_after);
  let mut cmd = Command::new(program);
  cmd.args(&collected);
  for (k, v) in env {
    cmd.env(k, v);
  }
  let output = crate::command_log::run_logged(&mut cmd, cmdline).map_err(|e| {
    GwmError::CommandFailed(format!(
      "{name}: failed to spawn ({e}). Is `{name}` installed and on PATH?"
    ))
  })?;
  if !output.status.success() {
    return Err(GwmError::CommandFailed(format!(
      "{name} exited {}: {}",
      output.status,
      String::from_utf8_lossy(&output.stderr).trim()
    )));
  }
  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Read the rendered body file a `*CreateRequest` points at. GitHub's
/// `gh` takes `--body-file`; GitLab's `glab` only takes an inline
/// `--description`, so the GitLab backend needs the contents.
pub(crate) fn read_body_file(path: &Path) -> Result<String> {
  std::fs::read_to_string(path)
    .map_err(|e| GwmError::Other(format!("could not read rendered body file {}: {}", path.display(), e)))
}
