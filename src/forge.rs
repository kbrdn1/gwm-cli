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
/// How much the `web_origin` of a [`RemoteRef`] can be trusted.
///
/// The distinction is load-bearing (Codex review #458): a guessed origin
/// is fine for building a link, but forcing it onto a forge CLI through
/// `$GITLAB_HOST` / `$GH_HOST` overrides a configuration that is very
/// likely more correct than the guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginTrust {
  /// Read from an `http(s)://` remote — scheme, host and port name the
  /// real web endpoint.
  FromUrl,
  /// Guessed from an SSH / scp-like remote, which carries no web scheme
  /// or port. The SSH hostname often differs from the web one, and the
  /// SSH port is not the web port.
  Guessed,
}

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
  /// Whether [`Self::web_origin`] was read from the remote or guessed.
  pub trust: OriginTrust,
}

impl RemoteRef {
  /// `host[:port]` — what `$GH_HOST` / a CLI `--hostname` expects.
  ///
  /// Derived from [`Self::web_origin`] so the web port survives: pinning
  /// the bare host for `https://ghe.example:8443/…` sent `gh` to port 443,
  /// which is guaranteed wrong and may reach a different instance
  /// listening there (Codex review #458). Whether every `gh` version
  /// honours a port here is not documented; passing it is still strictly
  /// better than dropping it.
  pub fn authority(&self) -> &str {
    self
      .web_origin
      .split_once("://")
      .map(|(_, rest)| rest)
      .unwrap_or(&self.web_origin)
      .trim_end_matches('/')
  }
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
  // A bracketed IPv6 literal keeps its brackets, and only a `:` past the
  // closing `]` can introduce a port — `[::1]` is a host, not host `[`
  // with port `:1]`.
  let port_sep = match authority.rfind(']') {
    Some(close) => authority[close..].find(':').map(|i| close + i),
    None => authority.rfind(':'),
  };
  let (host, port) = match port_sep.map(|i| (&authority[..i], &authority[i + 1..])) {
    Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
    _ => (authority, None),
  };
  if host.is_empty() {
    return Err(GwmError::Other(format!("origin '{}' has no host", url)));
  }
  // The port is web-relevant only over http(s). An `ssh://…:2222` port
  // addresses sshd, not the web UI, and an scp-like remote cannot carry a
  // port at all.
  // Both forges publish an alternate SSH endpoint for networks that block
  // port 22 (Codex review #458). It is an SSH host only — the API and web
  // UI stay on the canonical domain — so pinning it, or building links
  // from it, breaks every call. A short table of documented aliases, not
  // a heuristic: anything unrecognised is left verbatim.
  // DNS is case-insensitive, so normalise BEFORE the table and before
  // `web_origin` is built from the result (Codex review #458): matching
  // raw let `SSH.GITHUB.COM` fall through as an unknown host, and a
  // merely capitalised remote was pinned verbatim as the CLI's endpoint.
  // The path is deliberately left alone — repository paths ARE
  // case-sensitive on both forges.
  let lower_host = host.to_ascii_lowercase();
  let (host, known_alias) = match lower_host.as_str() {
    "ssh.github.com" => ("github.com", true),
    "altssh.gitlab.com" => ("gitlab.com", true),
    other => (other, false),
  };

  let (web_origin, trust) = match scheme.as_deref() {
    Some("http") => (
      format!("http://{}{}", host, port.map(|p| format!(":{p}")).unwrap_or_default()),
      OriginTrust::FromUrl,
    ),
    Some("https") => (
      format!("https://{}{}", host, port.map(|p| format!(":{p}")).unwrap_or_default()),
      OriginTrust::FromUrl,
    ),
    // A recognised alias resolves to a KNOWN instance, so this is
    // knowledge rather than inference — and it has to be, or nothing
    // downstream would pin the host and the CLI would re-read the raw
    // alternate endpoint from the remote (Codex review #458).
    _ if known_alias => (format!("https://{host}"), OriginTrust::FromUrl),
    _ => (format!("https://{host}"), OriginTrust::Guessed),
  };

  let path = trim_git_suffix(path_part.trim_start_matches('/'));
  if path.is_empty() {
    return Err(GwmError::Other(format!("origin '{}' has no repository path", url)));
  }

  Ok(RemoteRef {
    host: host.to_string(),
    path: path.to_string(),
    web_origin,
    trust,
  })
}

/// Split a remote URL into `(authority, path)`. Handles both the
/// scheme-ful form and git's scp-like `host:path` shorthand.
fn split_host_and_path(url: &str) -> Option<(&str, &str)> {
  if let Some((_scheme, rest)) = url.split_once("://") {
    return rest.split_once('/');
  }
  // `C:\repo`, `C:/repo` and the drive-relative `C:repo` all have the
  // shape of scp syntax, and parsing them as one produced host `c` with a
  // bogus path — so a local Windows checkout aimed network calls at an
  // invented host (Codex review #458).
  //
  // The first guard also demanded a separator after the colon, which let
  // `C:repo` through. The letter alone is the signal: a genuine
  // single-character hostname is vanishingly rare next to a drive
  // letter, and `git@localhost:team/proj` — a real one-label host, which
  // is what a LAN or tunnelled remote looks like — carries a `user@` and
  // more than one character either way.
  if !url.contains('@') {
    let mut chars = url.chars();
    if let (Some(first), Some(':')) = (chars.next(), chars.next()) {
      if first.is_ascii_alphabetic() {
        return None;
      }
    }
  }
  // scp-like `[user@]host:path`. The `:` separates host from path, so it
  // must not be confused with a `:port` (which scp syntax cannot express)
  // — nor with the colons inside a bracketed IPv6 literal, which a plain
  // `split_once(':')` chopped in half (Codex review #458): `git@[::1]:g/r`
  // became host `git@[` and path `:1]:g/r`.
  let user_len = url.find('@').map(|i| i + 1).unwrap_or(0);
  let hostpath = &url[user_len..];
  let sep = if hostpath.starts_with('[') {
    // Only a `:` past the closing bracket separates host from path.
    hostpath
      .find(']')
      .and_then(|close| hostpath[close..].find(':').map(|i| close + i))?
  } else {
    hostpath.find(':')?
  };
  let host_end = user_len + sep;
  Some((&url[..host_end], &url[host_end + 1..]))
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
/// Anything unrecognised falls back to GitHub.
///
/// That fallback is a *guess*, and [`resolve`] no longer acts on it
/// unprompted — see [`known_kind`], which is the same rule without the
/// guess. This function keeps returning one because callers that
/// already know the host is legitimate still want a default.
pub fn detect_kind(host: &str) -> ForgeKind {
  let lower = host.to_ascii_lowercase();
  if lower.starts_with("gitlab.") || lower.contains(".gitlab.") {
    return ForgeKind::GitLab;
  }
  known_kind(host).unwrap_or(ForgeKind::GitHub)
}

/// The forge a host **states**, or `None` when the URL does not say.
///
/// The known set is **only the vendors' own domains**: `github.com`,
/// the `ghe.com` tenancy domain, `gitlab.com`. Nothing else states
/// anything, and that includes the `gitlab.*` label — an attacker picks
/// their own hostname, so `gitlab.evil.example` would have walked
/// straight through this and collected a `$GITLAB_TOKEN` (Codex review
/// #458, the second hole in the same gate).
///
/// [`detect_kind`] keeps the label convention, because a *default* for
/// a host already known to be legitimate is a different question from
/// an *authorisation* for one that is not.
pub fn known_kind(host: &str) -> Option<ForgeKind> {
  let host = host.to_ascii_lowercase();
  match host.as_str() {
    "gitlab.com" | "www.gitlab.com" => Some(ForgeKind::GitLab),
    "github.com" | "www.github.com" | "ghe.com" => Some(ForgeKind::GitHub),
    h if h.ends_with(".ghe.com") => Some(ForgeKind::GitHub),
    _ => None,
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

  /// Whether the web origin was read from the remote rather than guessed
  /// (see [`OriginTrust`]). Callers that can afford a request use it to
  /// decide between the locally constructed URL and asking the server for
  /// its own `web_url`.
  fn origin_is_authoritative(&self) -> bool;

  /// What to pass as the CLI's repository selector — the slug, or `""`
  /// to let the CLI resolve the project from its working directory.
  ///
  /// Empty for a **guessed** origin with a known workdir (Codex review
  /// #458): no host can honestly be pinned there, and passing an explicit
  /// slug makes the CLI resolve it against its *default* host, which
  /// defeats the working directory entirely. Handing it nothing lets it
  /// read the repo's own remote — right host, right project, and the
  /// user's own CLI configuration honoured.
  fn repo_selector(&self) -> &str;

  /// Directory the forge CLI is spawned in, when known.
  ///
  /// This is the root fix for the wrong-tenant hazard (Codex review #458):
  /// `gh` / `glab` resolve the instance from their **working directory**
  /// whenever the flags do not pin it, and gwm's own cwd is not reliably
  /// the repo being queried — in workspace mode it is the workspace root
  /// while the row belongs to a child repo. Running the child inside the
  /// repo makes it read that repo's own remote, which is correct for SSH
  /// remotes too, where no host can be honestly pinned.
  fn workdir(&self) -> Option<&std::path::Path>;

  /// User-facing noun for a change proposal: `"PR"` or `"MR"`.
  fn pr_noun(&self) -> &'static str {
    match self.kind() {
      ForgeKind::GitHub => "PR",
      ForgeKind::GitLab => "MR",
    }
  }

  fn issue_url(&self, number: u64) -> String;
  fn pr_url(&self, number: u64) -> String;

  /// The issue's canonical URL, confirmed upstream when the local one
  /// would only be a guess (Codex review #458).
  ///
  /// An authoritative origin builds it offline. A guessed one asks the
  /// forge, which returns its own `web_url` — the only correct answer
  /// when the SSH hostname is not the web hostname, or the web UI runs on
  /// HTTP or a non-standard port. A failed request falls back to the
  /// guess rather than returning nothing.
  fn issue_url_confirmed(&self, number: u64) -> String {
    if self.origin_is_authoritative() {
      return self.issue_url(number);
    }
    self
      .fetch_issue(number)
      .ok()
      .map(|s| s.url)
      .filter(|u| !u.is_empty())
      .unwrap_or_else(|| self.issue_url(number))
  }

  /// PR/MR counterpart to [`Self::issue_url_confirmed`].
  fn pr_url_confirmed(&self, number: u64) -> String {
    if self.origin_is_authoritative() {
      return self.pr_url(number);
    }
    self
      .fetch_pr(number)
      .ok()
      .map(|s| s.url)
      .filter(|u| !u.is_empty())
      .unwrap_or_else(|| self.pr_url(number))
  }

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
  /// Reject a milestone this forge cannot accept, before anything is
  /// sent.
  ///
  /// GitLab's `due_date` is date-only while GitHub's `due_on` is
  /// RFC 3339, so a spec carrying a time is a hard error there. The
  /// check used to live inside `create_milestone` / `update_milestone`,
  /// which meant `--dry-run` printed a plan that could not run and a
  /// real push applied milestones until it reached the bad one, leaving
  /// the server half-updated (Codex review #458). Validating the whole
  /// set up front makes both paths fail before the first mutation.
  fn validate_milestone(&self, _spec: &MilestoneSpec) -> Result<()> {
    Ok(())
  }

  fn create_milestone(&self, spec: &MilestoneSpec) -> Result<()>;
  fn update_milestone(&self, number: u64, spec: &MilestoneSpec) -> Result<()>;
  fn delete_milestone(&self, number: u64) -> Result<()>;
}

/// Build a backend for an explicit kind. Exposed so tests (and the
/// doctor) can exercise the pure parts — URL building, terminology —
/// without a repository.
pub fn for_kind(kind: ForgeKind, origin: RemoteRef) -> Arc<dyn Forge> {
  for_kind_in(kind, origin, None)
}

/// [`for_kind`] with the directory the forge CLI should be spawned in —
/// the repo's workdir. See [`Forge::workdir`] for why it matters.
pub fn for_kind_in(kind: ForgeKind, origin: RemoteRef, workdir: Option<std::path::PathBuf>) -> Arc<dyn Forge> {
  match kind {
    ForgeKind::GitHub => Arc::new(crate::github::GitHubForge::new(origin, workdir)),
    ForgeKind::GitLab => Arc::new(crate::gitlab::GitLabForge::new(origin, workdir)),
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

/// [`resolve`], falling back to a project-less forge for the two creation
/// paths (`gwm new`, `gwm pr`) that predate #419 and deliberately
/// tolerate a repo with **no `origin`**: `gh` / `glab` can infer the
/// project from the local git context on their own, so the backend
/// simply omits `--repo` when the slug is empty. Every other call site
/// uses the strict [`resolve`], because an unresolvable slug there means
/// an unbuildable URL.
///
/// "No origin" is the whole of the fallback. It used to absorb *every*
/// resolve error, which quietly exempted these two commands from the
/// unrecognised-host refusal: they fell through to a guessed forge with
/// an empty slug and let the CLI infer the repo from the cwd — reading
/// the very remote that had just been refused, and sending the inherited
/// credentials there (Codex review #458). A gate with a fallback around
/// it is not a gate.
pub fn resolve_or_default(repo: &Repository, config: &Config) -> Result<Arc<dyn Forge>> {
  // The *remote* is the missing input, not the parse. `origin_ref`
  // reports "no origin", "no URL (non-utf8)" and "unparseable URL" as
  // one error type, and reading all three as absence sent a malformed
  // URL into the fallback — which has no project and no workdir, so an
  // ambient `$GH_REPO` / `$GITLAB_REPO` decided where the issue or PR
  // was created (Codex review #458). Ask git directly.
  if repo.find_remote("origin").is_ok() {
    return resolve(repo, config);
  }
  Ok({
    let kind = config.forge.unwrap_or(ForgeKind::GitHub);
    // An empty slug is the signal to let the CLI infer the project from
    // the local git context, and `Guessed` keeps that inference from being
    // overridden by a `$GITLAB_HOST` / `$GH_HOST` we have no basis for
    // (Codex review #458): forcing gitlab.com here would have created the
    // issue / MR on the wrong instance entirely.
    let (host, web_origin) = match kind {
      ForgeKind::GitHub => ("github.com", "https://github.com"),
      ForgeKind::GitLab => ("gitlab.com", "https://gitlab.com"),
    };
    for_kind(
      kind,
      RemoteRef {
        host: host.into(),
        path: String::new(),
        web_origin: web_origin.into(),
        trust: OriginTrust::Guessed,
      },
    )
  })
}

/// Bring the repo's persisted links in line with its configured backend,
/// for commands that write links without needing a forge.
///
/// `gwm link` is the one that matters: it wrote under the previous
/// backend's marker, and the next command that *did* resolve then read
/// the mismatch and deleted the line the user had just added (Codex
/// review #458). Reconciling first makes the write land under the marker
/// that will be checked against it.
///
/// Best-effort by design — `gwm link` works in a repo with no `origin`,
/// and a repo with no `origin` stamps nothing to reconcile.
pub fn reconcile_links(repo: &Repository, config: &Config) {
  if resolve(repo, config).is_ok() {
    return;
  }
  // `resolve` needs an origin; knowing which backend wrote a link does
  // not. With `forge = "gitlab"` and no origin yet, giving up here left
  // the marker unwritten — and an absent marker means "pre-#419 GitHub
  // links", so adding the origin later purged the line the user had just
  // made (Codex review #458).
  if let Some(kind) = config.forge {
    crate::github::reconcile_link_forge(repo, kind);
  }
}

/// Resolve the forge for `repo`: parse the `origin` remote, then pick the
/// backend from `.gwm.toml`'s `forge` key when set, else infer it from
/// the host.
///
/// This is also where the persisted links are reconciled against the
/// backend that will read them — see
/// [`crate::github::reconcile_link_forge`] for why it lives here and not
/// in the readers.
/// Which backend may drive an authenticated call against `origin`, or
/// the reason none may.
///
/// Three sources of authority, in order:
///
/// 1. **The host states it.** `github.com`, `ghe.com`, `gitlab.com` —
///    see [`known_kind`]. Nothing else is asked.
/// 2. **The user's own config states it.** `forge` in
///    `~/.config/gwm/config.toml`. It is not shipped with any repo, so
///    it is a statement by the person running gwm.
/// 3. **The repo's `.gwm.toml` states it** — and that file ships with
///    the repo, so it is only taken from a repo the user has approved
///    in the TOFU ledger. This is the case the review kept coming back
///    to (Codex review #458, three rounds): a hostile clone naming its
///    own server would otherwise get `gh` / `glab` pointed at it, with
///    whatever token the environment carries, from a plain `gwm
///    status` or a TUI selection.
///
/// The ledger is the same one `[[bootstrap.command]]` uses and the
/// answer is the same shape — approving a repo approves its whole
/// `.gwm.toml`, editing the file revokes it. It is read
/// non-interactively here because this runs on the TUI's selection
/// path; `gwm trust add` is how a user answers.
fn authorised_kind(repo: &Repository, config: &Config, parsed: &RemoteRef) -> Result<ForgeKind> {
  let Some(kind) = config.forge else {
    return known_kind(&parsed.host).ok_or_else(|| {
      GwmError::Other(format!(
        "origin host '{}' is not one gwm recognises, so it will not guess a forge and send \
         an authenticated call there. Name the backend with `forge = \"github\"` or \
         `forge = \"gitlab\"` in ~/.config/gwm/config.toml, or in the repo's .gwm.toml if \
         you trust it (`gwm trust add`).",
        parsed.host
      ))
    });
  };
  // On a host that states its own forge the key still wins outright, in
  // both directions — that is documented behaviour and costs nothing:
  // the call goes to a vendor domain either way, so the worst case is
  // gwm talking to `gitlab.com` with `gh`. The gate below is about
  // *where* the call goes, not which CLI makes it.
  if known_kind(&parsed.host).is_some() {
    return Ok(kind);
  }
  if Config::global_forge() == Some(kind) {
    return Ok(kind);
  }
  let workdir = repo.workdir().unwrap_or_else(|| repo.path());
  let origin_key = crate::trust::resolve_origin_key(Some(&parsed.web_origin), workdir);
  if crate::trust::config_is_trusted(workdir, &origin_key, crate::trust::resolve_mode(false, false))? {
    return Ok(kind);
  }
  Err(GwmError::Other(format!(
    "this repo's .gwm.toml points gwm at '{}', a host gwm does not recognise, and the repo \
     is not in the trust ledger. That file ships with the repo, so approving it is the same \
     decision as approving its bootstrap commands. Run `gwm trust add` here if you trust it, \
     or set `forge` in ~/.config/gwm/config.toml to cover the host yourself.",
    parsed.host
  )))
}

pub fn resolve(repo: &Repository, config: &Config) -> Result<Arc<dyn Forge>> {
  let parsed = origin_ref(repo)?;
  // An unrecognised host is not assumed to be anything. Before #419
  // `github::repo_slug` accepted `github.com` and rejected every other
  // origin outright, so gwm never made an authenticated call against an
  // arbitrary host; defaulting the guess to GitHub turned that hard
  // refusal into `GH_HOST=<whatever the remote says>`, and gh reads a
  // non-github.com host as Enterprise — so cloning a hostile repo and
  // running `gwm list --detect-pr` shipped `$GH_ENTERPRISE_TOKEN` to it
  // (Codex review #458, refused once on an unverified premise before
  // the diff against `dev` settled it).
  //
  // The residual hole, stated rather than justified: a `.gwm.toml`
  // committed *in* that repo can still name the forge, and none of the
  // read paths (`list`, `status`, `open`) pass through the bootstrap
  // trust gate, so the repo authorises itself. Closing that means
  // making the origin host a trust decision of its own — issue #460.
  // What this does close is the zero-config path, which needed no file
  // from the attacker at all.
  let kind = authorised_kind(repo, config, &parsed)?;
  crate::github::reconcile_link_forge(repo, kind);
  // A bare repo has no workdir, but its own directory is still a valid
  // git context for `gh` / `glab` to resolve remotes from — and losing it
  // means an unpinned origin falls back to the CLI's default tenant
  // (Codex review #458). Bare + worktrees is a normal gwm layout.
  let cwd = repo
    .workdir()
    .map(|p| p.to_path_buf())
    .or_else(|| Some(repo.path().to_path_buf()));
  Ok(for_kind_in(kind, parsed, cwd))
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
  run_cli_with(program, args, &CliSpawn::default())
}

/// Child-process settings a forge backend needs beyond the argv.
#[derive(Debug, Default, Clone, Copy)]
pub struct CliSpawn<'a> {
  /// Extra environment: `$GITLAB_HOST` / `$GH_HOST` pin the instance.
  pub env: &'a [(String, String)],
  /// Working directory. `gh` / `glab` fall back to resolving the instance
  /// from here, so it must be the repo being queried — not gwm's own cwd.
  pub cwd: Option<&'a std::path::Path>,
  /// Inherited variables to strip from the child (Codex review #458).
  ///
  /// Three separate P1s across this review were the same shape: gwm's
  /// environment is inherited, and something in it redirected the call.
  /// Rather than name one more variable per round, the backends audit the
  /// *class* — everything the target CLI documents as overriding which
  /// project or host it acts on — and clear what gwm knows better.
  pub env_remove: &'a [&'a str],
  /// Flags whose *value* must not reach the Command Logs transcript.
  pub redact_after: &'a [&'a str],
  /// Payload written to the child's stdin, for the creation paths that
  /// must keep a rendered body out of the argv (issue #459).
  pub stdin: Option<&'a [u8]>,
  /// Withhold the response from the Command Logs transcript. Set on the
  /// same creation paths: the GitLab endpoints echo `description` back,
  /// so a body kept off the argv would otherwise land in the modal via
  /// the reply instead (issue #459).
  pub redact_output: bool,
}

/// [`run_cli`] with extra environment for the child and a redaction list
/// for the transcript. Both exist for the GitLab backend: `$GITLAB_HOST`
/// pins the instance (otherwise `glab` resolves it from the *process* cwd
/// and falls back to gitlab.com), and `--description` carries a whole
/// rendered body that must not land verbatim in Command Logs.
pub fn run_cli_with<I, S>(program: &OsStr, args: I, spawn: &CliSpawn<'_>) -> Result<String>
where
  I: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  // Collect the args once so they can both drive the spawn and build the
  // human-readable command line stored on the transcript (issue #226).
  let collected: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_os_string()).collect();
  let name = program_name(program);
  let cmdline = cli_command_line_redacted(program, &collected, spawn.redact_after);
  let mut cmd = Command::new(program);
  cmd.args(&collected);
  for k in spawn.env_remove {
    cmd.env_remove(k);
  }
  for (k, v) in spawn.env {
    cmd.env(k, v);
  }
  if let Some(cwd) = spawn.cwd {
    cmd.current_dir(cwd);
  }
  let output = match spawn.stdin {
    Some(payload) => crate::command_log::run_logged_with_stdin(&mut cmd, cmdline, payload, spawn.redact_output),
    // `redact_output` used to be dropped here, which made it inert for
    // every read that asked for it (Codex review #458).
    None if spawn.redact_output => crate::command_log::run_logged_redacted(&mut cmd, cmdline),
    None => crate::command_log::run_logged(&mut cmd, cmdline),
  }
  .map_err(|e| {
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
