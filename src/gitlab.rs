//! GitLab backend for the [`crate::forge::Forge`] trait (issue #419),
//! shelling out to the `glab` CLI.
//!
//! Two invocation styles, picked per operation rather than uniformly:
//!
//! - **issues / merge requests** go through the first-class subcommands
//!   (`glab issue view`, `glab mr view`, `glab mr list`) with
//!   `--output json`. `glab` passes the GitLab REST object through
//!   unchanged, so the parsers below deserialize the documented API shape.
//! - **labels / milestones** go through `glab api`. `glab label list`
//!   caps at 100 rows per page with no `--paginate`, and `glab label edit`
//!   keys on a numeric `--label-id` that [`crate::labels::RemoteLabel`]
//!   does not carry. The REST endpoints accept the label *title* as a
//!   key and `--paginate` covers the >100 case, so one code path handles
//!   list/create/update/delete for both resources.
//!
//! Every parser here is pure and `pub` so the contract is unit-testable
//! without a `glab` binary — CI runners have none, exactly as they have
//! no `gh`.
//!
//! ## Divergences from GitHub, and where they are absorbed
//!
//! | GitLab | GitHub | Absorbed by |
//! |---|---|---|
//! | `iid` (project-scoped) | `number` | parsers, at the boundary |
//! | `opened` / `locked` | `OPEN` | [`parse_mr_json`] |
//! | one `head_pipeline` | `statusCheckRollup[]` | one synthetic [`PrCheck`] |
//! | `"#RRGGBB"` | `"rrggbb"` | [`parse_labels_json`] / [`label_create_argv`] |
//! | `due_date` (`YYYY-MM-DD`) | `due_on` (RFC3339) | [`parse_milestones_json`] |
//! | `state_event=close` | `state=closed` | [`milestone_update_argv`] |

use crate::error::{GwmError, Result};
use crate::forge::{
  self, CheckOutcome, CreatedIssue, CreatedPr, Forge, ForgeKind, IssueCreateRequest, IssueState, IssueStatus, PrCheck,
  PrCreateRequest, PrHead, PrState, PrStatus,
};
use crate::labels::{LabelSpec, RemoteLabel};
use crate::milestones::{self, MilestoneSpec, MilestoneState, RemoteMilestone};
use serde::Deserialize;
use std::ffi::OsString;
use std::sync::LazyLock;

static ISSUE_URL_RE: LazyLock<regex::Regex> =
  LazyLock::new(|| regex::Regex::new(r"/-/issues/(\d+)").expect("static GitLab issue URL regex compiles"));
static MR_URL_RE: LazyLock<regex::Regex> =
  LazyLock::new(|| regex::Regex::new(r"/-/merge_requests/(\d+)").expect("static GitLab MR URL regex compiles"));

/// Resolve the `glab` program to invoke: `$GWM_GLAB` when set (test /
/// override hook), else `glab` on `PATH`. Mirrors
/// [`crate::github::gh_program`] so both backends have the same seam.
pub fn glab_program() -> OsString {
  std::env::var_os("GWM_GLAB").unwrap_or_else(|| "glab".into())
}

/// Environment pinned on every `glab` spawn.
///
/// Without `$GITLAB_HOST`, `glab` resolves the instance from the *process*
/// cwd's git remote and otherwise falls back to gitlab.com (Codex review
/// #458). gwm's cwd is not reliably the repo being queried — in workspace
/// mode it is the workspace root while the row belongs to a child repo —
/// so a same-named project on the wrong instance could be read and its
/// iid persisted into the local git config. The value is the resolved web
/// origin, which already carries scheme and port.
pub fn glab_env(web_origin: &str) -> Vec<(String, String)> {
  vec![("GITLAB_HOST".to_string(), web_origin.to_string())]
}

/// Flags whose *value* must never reach the Command Logs transcript.
/// `--description` carries a whole rendered issue / MR body because
/// `glab` has no `--body-file` counterpart to `gh`'s.
const REDACTED_FLAGS: &[&str] = &["--description"];

// ---- percent-encoding ----------------------------------------------------

/// Percent-encode one URL path segment, keeping only RFC 3986 unreserved
/// characters. Load-bearing in two places: the project path (`group/proj`
/// → `group%2Fproj`, which is how GitLab addresses a project by path) and
/// a label title used as a key (`good first issue`).
///
/// Hand-rolled rather than pulling a dependency for ~10 lines.
fn encode_segment(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for b in s.bytes() {
    match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
      _ => out.push_str(&format!("%{:02X}", b)),
    }
  }
  out
}

/// `--repo <slug>`, or nothing when the slug is empty. An unresolvable
/// `origin` is tolerated on the creation paths (see
/// [`crate::forge::resolve_or_default`]): `glab` then infers the project
/// from the local git context on its own.
fn repo_flag(slug: &str) -> Vec<String> {
  if slug.is_empty() {
    Vec::new()
  } else {
    vec!["--repo".into(), slug.into()]
  }
}

/// `projects/<url-encoded path>` — the REST prefix every `glab api` call
/// below is rooted at.
fn project_path(slug: &str) -> String {
  format!("projects/{}", encode_segment(slug))
}

// ---- issues --------------------------------------------------------------

#[derive(Deserialize)]
struct RawIssue {
  /// The project-scoped number the user sees and every URL uses. The
  /// sibling `id` is a global counter and is deliberately ignored:
  /// picking it would fail silently, producing wrong URLs and wrong
  /// follow-up fetches with no error anywhere.
  iid: u64,
  title: String,
  state: String,
  #[serde(default)]
  labels: Vec<String>,
  #[serde(default)]
  updated_at: String,
  #[serde(default)]
  web_url: String,
}

/// Parse `glab issue view <iid> --output json`.
pub fn parse_issue_json(s: &str) -> Result<IssueStatus> {
  let raw: RawIssue = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "gitlab issue",
    source: e,
  })?;
  let state = match raw.state.as_str() {
    "opened" => IssueState::Open,
    "closed" => IssueState::Closed,
    other => return Err(GwmError::Other(format!("unknown GitLab issue state '{}'", other))),
  };
  Ok(IssueStatus {
    number: raw.iid,
    title: raw.title,
    state,
    url: raw.web_url,
    labels: raw.labels,
    updated_at: raw.updated_at,
  })
}

pub fn issue_view_argv(slug: &str, number: u64) -> Vec<String> {
  vec![
    "issue".into(),
    "view".into(),
    number.to_string(),
    "--repo".into(),
    slug.into(),
    "--output".into(),
    "json".into(),
  ]
}

// ---- merge requests ------------------------------------------------------

#[derive(Deserialize)]
struct RawMr {
  iid: u64,
  #[serde(default)]
  title: String,
  state: String,
  /// `draft` superseded `work_in_progress`; older self-hosted instances
  /// still only send the legacy key, so both are read.
  #[serde(default)]
  draft: bool,
  #[serde(default)]
  work_in_progress: bool,
  #[serde(default)]
  web_url: String,
  #[serde(default)]
  updated_at: String,
  #[serde(default)]
  source_branch: String,
  #[serde(default)]
  target_branch: String,
  // `Option` (not just `#[serde(default)]`) so an explicit
  // `"author": null` — a deleted account — deserialises to `None`
  // instead of erroring; `default` alone only covers a *missing* key.
  #[serde(default)]
  author: Option<RawAuthor>,
  #[serde(default)]
  head_pipeline: Option<RawPipeline>,
}

#[derive(Deserialize, Default)]
struct RawAuthor {
  #[serde(default)]
  username: String,
}

#[derive(Deserialize)]
struct RawPipeline {
  #[serde(default)]
  status: String,
  #[serde(default)]
  web_url: Option<String>,
  #[serde(default)]
  started_at: Option<String>,
  #[serde(default)]
  finished_at: Option<String>,
}

/// Classify a GitLab pipeline status into the shared [`CheckOutcome`].
///
/// `skipped` is treated as accepted (green), mirroring the GitHub side
/// counting `NEUTRAL` / `SKIPPED`. Anything the list does not
/// cover lands on [`CheckOutcome::Unknown`] **by design** (issue #419): a
/// `_ => Passing` catch-all would let a future GitLab status report a
/// green CI that is not green, and that failure is silent.
pub fn classify_pipeline_status(status: &str) -> CheckOutcome {
  match status.trim().to_ascii_lowercase().as_str() {
    "success" | "skipped" => CheckOutcome::Passing,
    "failed" | "canceled" | "cancelled" | "canceling" | "cancelling" => CheckOutcome::Failing,
    // `manual` sits here, NOT with `skipped` (Codex review #458): a
    // pipeline reports `manual` while it waits on a *blocking* manual
    // job — it is suspended, it can bar the merge, and it is not a
    // pass. Reading it as GitHub's `SKIPPED` painted a blocked MR green.
    "created" | "waiting_for_resource" | "preparing" | "pending" | "running" | "scheduled" | "manual" => {
      CheckOutcome::Running
    }
    _ => CheckOutcome::Unknown,
  }
}

/// Parse `glab mr view <iid> --output json`.
///
/// GitLab exposes one `head_pipeline` object where GitHub exposes a
/// `statusCheckRollup` array, so the pipeline becomes a **single**
/// synthetic [`PrCheck`]. Per-job granularity would need a second
/// request against `/pipelines/:id/jobs` and is intentionally out of
/// scope: `PrStatus` stays byte-identical in shape across forges, and
/// the CI overlay renders the pipeline row with a link to it.
pub fn parse_mr_json(s: &str) -> Result<PrStatus> {
  let raw: RawMr = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "gitlab merge request",
    source: e,
  })?;
  let draft = raw.draft || raw.work_in_progress;
  let state = match (raw.state.as_str(), draft) {
    ("merged", _) => PrState::Merged,
    ("closed", _) => PrState::Closed,
    // `locked` is a transient state while a merge is in flight; from the
    // user's point of view the MR is still open.
    ("opened" | "locked", true) => PrState::Draft,
    ("opened" | "locked", false) => PrState::Open,
    (other, _) => return Err(GwmError::Other(format!("unknown GitLab MR state '{}'", other))),
  };

  let checks: Vec<PrCheck> = raw
    .head_pipeline
    .map(|p| {
      vec![PrCheck {
        name: "pipeline".into(),
        outcome: classify_pipeline_status(&p.status),
        url: p.web_url,
        // GitLab has no per-pipeline "workflow" grouping to surface.
        workflow_name: None,
        started_at: p.started_at,
        completed_at: p.finished_at,
      }]
    })
    .unwrap_or_default();

  let checks_total = checks.len() as u32;
  let checks_passed = checks.iter().filter(|c| c.outcome == CheckOutcome::Passing).count() as u32;
  let ci = forge::aggregate_ci_state(checks.iter().map(|c| c.outcome));

  Ok(PrStatus {
    number: raw.iid,
    title: raw.title,
    state,
    url: raw.web_url,
    updated_at: raw.updated_at,
    checks_passed,
    checks_total,
    ci,
    checks,
  })
}

/// Parse the same payload as [`parse_mr_json`] down to the head metadata
/// `gwm review` needs (author / source branch / target branch).
pub fn parse_mr_head_json(s: &str) -> Result<PrHead> {
  let raw: RawMr = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "gitlab mr head",
    source: e,
  })?;
  Ok(PrHead {
    number: raw.iid,
    author: raw.author.unwrap_or_default().username,
    head_ref_name: raw.source_branch,
    base_ref_name: raw.target_branch,
  })
}

pub fn mr_view_argv(slug: &str, number: u64) -> Vec<String> {
  vec![
    "mr".into(),
    "view".into(),
    number.to_string(),
    "--repo".into(),
    slug.into(),
    "--output".into(),
    "json".into(),
  ]
}

/// Argv for `glab mr list --repo <slug> --source-branch <branch> --all
/// --output json --per-page 1`. `--all` is the load-bearing bit — the
/// GitHub counterpart's `--state all`: a closed or merged MR for the
/// branch is still detected, and its state is resolved later via
/// [`parse_mr_json`].
pub fn mr_list_argv(slug: &str, branch: &str) -> Vec<String> {
  vec![
    "mr".into(),
    "list".into(),
    "--repo".into(),
    slug.into(),
    "--source-branch".into(),
    branch.into(),
    "--all".into(),
    "--output".into(),
    "json".into(),
    "--per-page".into(),
    "1".into(),
  ]
}

/// Parse the JSON array printed by `glab mr list --output json`,
/// returning the first MR's `iid` if any.
pub fn parse_mr_list_number(s: &str) -> Result<Option<u64>> {
  #[derive(Deserialize)]
  struct MrRef {
    iid: u64,
  }
  let arr: Vec<MrRef> = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "gitlab mr list",
    source: e,
  })?;
  Ok(arr.into_iter().next().map(|m| m.iid))
}

// ---- create --------------------------------------------------------------

/// Argv for `glab issue create`.
///
/// `glab` has no `--body-file`, so the rendered body is passed inline via
/// `--description`. `--no-editor` and `--yes` are both required: without
/// them `glab` opens `$EDITOR` and/or blocks on a TTY confirm, which
/// hangs a non-interactive `gwm issue create`.
pub fn issue_create_argv(slug: &str, title: &str, body: &str, labels: &[String]) -> Vec<String> {
  let mut argv = vec!["issue".into(), "create".into()];
  argv.extend(repo_flag(slug));
  argv.extend(["--title".into(), title.into(), "--description".into(), body.into()]);
  for label in labels {
    argv.push("--label".into());
    argv.push(label.clone());
  }
  argv.push("--no-editor".into());
  argv.push("--yes".into());
  argv
}

/// Argv for `glab mr create`. Same `--no-editor` / `--yes` reasoning as
/// [`issue_create_argv`].
pub fn mr_create_argv(slug: &str, title: &str, body: &str, head: &str, base: Option<&str>, draft: bool) -> Vec<String> {
  let mut argv = vec!["mr".into(), "create".into()];
  argv.extend(repo_flag(slug));
  argv.extend([
    "--title".into(),
    title.into(),
    "--description".into(),
    body.into(),
    "--source-branch".into(),
    head.into(),
  ]);
  if let Some(base) = base {
    argv.push("--target-branch".into());
    argv.push(base.into());
  }
  if draft {
    argv.push("--draft".into());
  }
  argv.push("--no-editor".into());
  argv.push("--yes".into());
  argv
}

fn parse_created_number(out: &str, re: &regex::Regex, kind: &str) -> Result<u64> {
  re.captures(out)
    .and_then(|c| c.get(1))
    .and_then(|m| m.as_str().parse::<u64>().ok())
    .ok_or_else(|| {
      GwmError::CommandFailed(format!(
        "glab {} create did not print a URL containing a number: {}",
        kind,
        out.trim()
      ))
    })
}

/// Recover the issue number from `glab issue create` output. GitLab's URL
/// shape is `/-/issues/N`, not GitHub's `/issues/N` — the `-/` infix is
/// what keeps the two regexes from cross-matching.
pub fn parse_created_issue_number(out: &str) -> Result<u64> {
  parse_created_number(out, &ISSUE_URL_RE, "issue")
}

/// Recover the MR number from `glab mr create` output (`/-/merge_requests/N`).
pub fn parse_created_mr_number(out: &str) -> Result<u64> {
  parse_created_number(out, &MR_URL_RE, "mr")
}

/// The first whitespace-delimited token in `out` that looks like the
/// created object's URL. `glab` prints a banner line before it.
fn extract_url(out: &str, marker: &str) -> Option<String> {
  out
    .split_whitespace()
    .find(|t| t.contains(marker))
    .map(|t| t.to_string())
}

// ---- labels --------------------------------------------------------------

#[derive(Deserialize)]
struct RawLabel {
  name: String,
  /// GitLab serialises `"#D9534F"`. Not `#[serde(default)]` on purpose:
  /// a contract change that dropped the field should be a hard parse
  /// error, not a silent empty string flagging every label as a colour
  /// mismatch (same reasoning as the GitHub side).
  color: String,
  #[serde(default)]
  description: Option<String>,
}

/// Parse `glab api projects/<id>/labels`.
///
/// Colour is normalised to the bare lowercase 6-hex the shared diff
/// engine compares against — GitLab's leading `#` would otherwise make
/// every label read as changed on every run.
pub fn parse_labels_json(s: &str) -> Result<Vec<RemoteLabel>> {
  let raw: Vec<RawLabel> = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "gitlab labels",
    source: e,
  })?;
  Ok(
    raw
      .into_iter()
      .map(|r| RemoteLabel {
        name: r.name,
        description: r.description,
        color: r.color.trim_start_matches('#').to_ascii_lowercase(),
      })
      .collect(),
  )
}

/// Argv for `GET /projects/:id/labels`.
///
/// **Unverified assumption**: this deserializes into a single `Vec<_>`, so
/// `glab api --paginate` must *merge* pages into one JSON array the way
/// `gh api --paginate` does. If glab instead concatenates arrays
/// (`[…][…]`), parsing breaks — and only for projects past the 100-row
/// first page, so it would not show up in light use. Confirm against a
/// real instance before relying on it at that scale.
pub fn label_list_argv(slug: &str) -> Vec<String> {
  vec![
    "api".into(),
    "--paginate".into(),
    format!("{}/labels?per_page=100", project_path(slug)),
  ]
}

/// Argv for `POST /projects/:id/labels`. The `#` GitLab expects on the
/// colour is re-added here, mirroring the strip in [`parse_labels_json`].
pub fn label_create_argv(slug: &str, spec: &LabelSpec) -> Vec<String> {
  let mut argv = vec![
    "api".into(),
    "-X".into(),
    "POST".into(),
    format!("{}/labels", project_path(slug)),
    "--raw-field".into(),
    format!("name={}", spec.name),
    "--raw-field".into(),
    format!("color=#{}", spec.color),
  ];
  if let Some(desc) = spec.description.as_ref().filter(|s| !s.is_empty()) {
    argv.push("--raw-field".into());
    argv.push(format!("description={}", desc));
  }
  argv
}

/// Argv for `PUT /projects/:id/labels/:label_id`. GitLab accepts the
/// label **title** in place of the numeric id, which is what lets
/// [`RemoteLabel`] stay id-free and shared with the GitHub backend
/// (`glab label edit` would have required `--label-id`).
///
/// An absent description is omitted rather than sent empty — GitLab would
/// otherwise wipe a description the user never meant to touch.
pub fn label_update_argv(slug: &str, spec: &LabelSpec) -> Vec<String> {
  let mut argv = vec![
    "api".into(),
    "-X".into(),
    "PUT".into(),
    format!("{}/labels/{}", project_path(slug), encode_segment(&spec.name)),
    "--raw-field".into(),
    format!("color=#{}", spec.color),
  ];
  if let Some(desc) = spec.description.as_ref().filter(|s| !s.is_empty()) {
    argv.push("--raw-field".into());
    argv.push(format!("description={}", desc));
  }
  argv
}

pub fn label_delete_argv(slug: &str, name: &str) -> Vec<String> {
  vec![
    "api".into(),
    "-X".into(),
    "DELETE".into(),
    format!("{}/labels/{}", project_path(slug), encode_segment(name)),
  ]
}

// ---- milestones ----------------------------------------------------------

#[derive(Deserialize)]
struct RawMilestone {
  /// The **global** id, not `iid`: `PUT`/`DELETE
  /// /projects/:id/milestones/:milestone_id` keys on it. This is the one
  /// place the `iid` rule that governs issues and MRs is deliberately
  /// inverted, because the endpoint demands it.
  id: u64,
  title: String,
  state: String,
  #[serde(default)]
  description: Option<String>,
  /// `YYYY-MM-DD`, where GitHub sends an RFC3339 `due_on`.
  #[serde(default)]
  due_date: Option<String>,
}

/// Parse `glab api projects/<id>/milestones`.
///
/// Two normalisations so the shared diff engine never sees a spurious
/// change: `active` → open, and the bare `due_date` is widened to the
/// RFC3339 end-of-day form [`MilestoneSpec::due_on`] carries.
pub fn parse_milestones_json(s: &str) -> Result<Vec<RemoteMilestone>> {
  let raw: Vec<RawMilestone> = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "gitlab milestones",
    source: e,
  })?;
  raw
    .into_iter()
    .map(|r| {
      let state = match r.state.as_str() {
        "active" => MilestoneState::Open,
        "closed" => MilestoneState::Closed,
        other => {
          return Err(GwmError::Other(format!(
            "milestone '{}' has unknown GitLab state '{}': expected 'active' or 'closed'",
            r.title, other
          )))
        }
      };
      let due_on = match r.due_date.as_deref().filter(|s| !s.is_empty()) {
        Some(d) => Some(milestones::normalize_due_on(d)?),
        None => None,
      };
      Ok(RemoteMilestone {
        number: r.id,
        title: r.title,
        description: r.description,
        due_on,
        state,
      })
    })
    .collect()
}

/// GitLab wants `due_date=YYYY-MM-DD` where the spec carries RFC3339.
fn due_date_field(due_on: &str) -> &str {
  due_on.split('T').next().unwrap_or(due_on)
}

/// Refuse a declared `due_on` that carries a time other than end-of-day
/// (Codex review #458).
///
/// GitLab's `due_date` is **date-only**. A spec like `2026-07-15T17:00:00Z`
/// is written as `2026-07-15`, read back as `2026-07-15T23:59:59Z` — the
/// form [`crate::milestones::normalize_due_on`] gives a bare date — and so
/// never compares equal to what was declared. The milestone would show as
/// changed on every `gwm milestones list` and be PUT again on every push,
/// without ever reaching the declared state.
///
/// The shared diff engine compares timestamps, not dates; making it
/// date-granular per forge is a larger change than this belongs in. Until
/// then, failing with the cause named beats looping silently.
pub fn check_due_on_is_date_only(spec: &MilestoneSpec) -> Result<()> {
  let Some(due) = spec.due_on.as_deref().filter(|s| !s.is_empty()) else {
    return Ok(());
  };
  // `normalize_due_on` maps a bare `YYYY-MM-DD` to end-of-day UTC, which is
  // exactly what the GitLab read path reconstructs — so end-of-day is the
  // one time-of-day that round-trips.
  let normalized = milestones::normalize_due_on(due)?;
  if normalized.ends_with("T23:59:59Z") {
    return Ok(());
  }
  Err(GwmError::Config(format!(
    "milestone '{}': due_on '{}' carries a time of day, but GitLab stores milestone due dates as a date only \
     ('{}'). The value would be rewritten on every push without ever matching. Declare a bare date \
     (due_on = \"{}\") instead.",
    spec.title,
    due,
    due_date_field(&normalized),
    due_date_field(&normalized),
  )))
}

/// `state` is not writable on GitLab milestones — closing and reopening
/// are `state_event` transitions.
fn state_event(state: MilestoneState) -> &'static str {
  match state {
    MilestoneState::Open => "activate",
    MilestoneState::Closed => "close",
  }
}

/// Argv for `GET /projects/:id/milestones`.
///
/// No `state` filter: GitLab returns both active and closed milestones
/// when the parameter is omitted. `--paginate` matters for the same
/// reason as on GitHub — `per_page` caps at 100, and diffing against a
/// truncated set would make `--prune` propose deleting whatever fell off
/// the page.
///
/// **Unverified assumption**: this deserializes into a single `Vec<_>`, so
/// `glab api --paginate` must *merge* pages into one JSON array the way
/// `gh api --paginate` does. If glab instead concatenates arrays
/// (`[…][…]`), parsing breaks — and only for projects past the 100-row
/// first page, so it would not show up in light use. Confirm against a
/// real instance before relying on it at that scale.
pub fn milestone_list_argv(slug: &str) -> Vec<String> {
  vec![
    "api".into(),
    "--paginate".into(),
    format!("{}/milestones?per_page=100", project_path(slug)),
  ]
}

/// Argv for `POST /projects/:id/milestones`.
///
/// `state_event` is not accepted on create — a milestone is always born
/// active — so a declared `state = "closed"` needs the follow-up PUT that
/// [`GitLabForge::create_milestone`] issues.
pub fn milestone_create_argv(slug: &str, spec: &MilestoneSpec) -> Vec<String> {
  let mut argv = vec![
    "api".into(),
    "-X".into(),
    "POST".into(),
    format!("{}/milestones", project_path(slug)),
    "--raw-field".into(),
    format!("title={}", spec.title),
  ];
  if let Some(desc) = spec.description.as_ref().filter(|s| !s.is_empty()) {
    argv.push("--raw-field".into());
    argv.push(format!("description={}", desc));
  }
  if let Some(due) = spec.due_on.as_ref().filter(|s| !s.is_empty()) {
    argv.push("--raw-field".into());
    argv.push(format!("due_date={}", due_date_field(due)));
  }
  argv
}

/// Argv for `PUT /projects/:id/milestones/:milestone_id`.
pub fn milestone_update_argv(slug: &str, number: u64, spec: &MilestoneSpec) -> Vec<String> {
  let mut argv = vec![
    "api".into(),
    "-X".into(),
    "PUT".into(),
    format!("{}/milestones/{}", project_path(slug), number),
    "--raw-field".into(),
    format!("title={}", spec.title),
  ];
  if let Some(desc) = spec.description.as_ref().filter(|s| !s.is_empty()) {
    argv.push("--raw-field".into());
    argv.push(format!("description={}", desc));
  }
  if let Some(due) = spec.due_on.as_ref().filter(|s| !s.is_empty()) {
    argv.push("--raw-field".into());
    argv.push(format!("due_date={}", due_date_field(due)));
  }
  argv.push("--raw-field".into());
  argv.push(format!("state_event={}", state_event(spec.state)));
  argv
}

pub fn milestone_delete_argv(slug: &str, number: u64) -> Vec<String> {
  vec![
    "api".into(),
    "-X".into(),
    "DELETE".into(),
    format!("{}/milestones/{}", project_path(slug), number),
  ]
}

/// Pull the `id` out of the object `POST /milestones` echoes back, so a
/// declared-closed milestone can be transitioned immediately after.
fn parse_created_milestone_id(s: &str) -> Result<u64> {
  #[derive(Deserialize)]
  struct Created {
    id: u64,
  }
  let created: Created = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "gitlab milestone create",
    source: e,
  })?;
  Ok(created.id)
}

// ---- the backend ---------------------------------------------------------

/// GitLab implementation of [`Forge`], shelling out to `glab`.
#[derive(Debug, Clone)]
pub struct GitLabForge {
  web_origin: String,
  slug: String,
  program: OsString,
}

impl GitLabForge {
  /// Resolves `$GWM_GLAB` **now**, on the calling thread, so a forge
  /// handed to the TUI's fetch worker never re-reads the process
  /// environment concurrently with env-mutating code (issue #217).
  pub fn new(web_origin: String, slug: String) -> Self {
    Self {
      web_origin,
      slug,
      program: glab_program(),
    }
  }

  fn run_argv(&self, argv: Vec<String>) -> Result<String> {
    forge::run_cli_with(&self.program, argv, &glab_env(&self.web_origin), REDACTED_FLAGS)
  }
}

impl Forge for GitLabForge {
  fn kind(&self) -> ForgeKind {
    ForgeKind::GitLab
  }

  fn slug(&self) -> &str {
    &self.slug
  }

  fn web_origin(&self) -> &str {
    &self.web_origin
  }

  fn issue_url(&self, number: u64) -> String {
    format!("{}/{}/-/issues/{}", self.web_origin, self.slug, number)
  }

  fn pr_url(&self, number: u64) -> String {
    format!("{}/{}/-/merge_requests/{}", self.web_origin, self.slug, number)
  }

  fn pr_head_refspec(&self, number: u64) -> String {
    format!("merge-requests/{number}/head")
  }

  fn fetch_issue(&self, number: u64) -> Result<IssueStatus> {
    parse_issue_json(&self.run_argv(issue_view_argv(&self.slug, number))?)
  }

  fn fetch_pr(&self, number: u64) -> Result<PrStatus> {
    parse_mr_json(&self.run_argv(mr_view_argv(&self.slug, number))?)
  }

  fn fetch_pr_head(&self, number: u64) -> Result<PrHead> {
    parse_mr_head_json(&self.run_argv(mr_view_argv(&self.slug, number))?)
  }

  fn find_pr_for_branch(&self, branch: &str) -> Result<Option<u64>> {
    parse_mr_list_number(&self.run_argv(mr_list_argv(&self.slug, branch))?)
  }

  fn create_issue(&self, req: &IssueCreateRequest<'_>) -> Result<CreatedIssue> {
    let body = forge::read_body_file(req.body_file)?;
    let out = self.run_argv(issue_create_argv(&self.slug, req.title, &body, req.labels))?;
    let number = parse_created_issue_number(&out)?;
    Ok(CreatedIssue {
      number,
      url: extract_url(&out, "/-/issues/").unwrap_or_else(|| self.issue_url(number)),
    })
  }

  fn create_pr(&self, req: &PrCreateRequest<'_>) -> Result<CreatedPr> {
    let body = forge::read_body_file(req.body_file)?;
    let out = self.run_argv(mr_create_argv(
      &self.slug, req.title, &body, req.head, req.base, req.draft,
    ))?;
    let number = parse_created_mr_number(&out)?;
    Ok(CreatedPr {
      number,
      url: extract_url(&out, "/-/merge_requests/").unwrap_or_else(|| self.pr_url(number)),
    })
  }

  fn fetch_remote_labels(&self) -> Result<Vec<RemoteLabel>> {
    parse_labels_json(&self.run_argv(label_list_argv(&self.slug))?)
  }

  fn create_label(&self, spec: &LabelSpec) -> Result<()> {
    self.run_argv(label_create_argv(&self.slug, spec))?;
    Ok(())
  }

  fn update_label(&self, spec: &LabelSpec) -> Result<()> {
    self.run_argv(label_update_argv(&self.slug, spec))?;
    Ok(())
  }

  fn delete_label(&self, name: &str) -> Result<()> {
    // Same guard as the GitHub backend (issue #100): the name lands in a
    // URL path here rather than an argv slot, but a remote label planted
    // with a `-`-prefixed or otherwise hostile name should be refused
    // uniformly across forges rather than depending on which one is in
    // use.
    crate::labels::validate_label_name(name).map_err(|e| {
      let inner = match e {
        GwmError::Config(msg) => msg,
        other => other.to_string(),
      };
      GwmError::Config(format!("labels (remote): {} — refusing to delete via `glab`", inner))
    })?;
    self.run_argv(label_delete_argv(&self.slug, name))?;
    Ok(())
  }

  fn fetch_remote_milestones(&self) -> Result<Vec<RemoteMilestone>> {
    parse_milestones_json(&self.run_argv(milestone_list_argv(&self.slug))?)
  }

  fn create_milestone(&self, spec: &MilestoneSpec) -> Result<()> {
    check_due_on_is_date_only(spec)?;
    let out = self.run_argv(milestone_create_argv(&self.slug, spec))?;
    // GitLab has no `state` on create, so a declared-closed milestone
    // needs a second call to transition it. Keyed on the id echoed back
    // by the POST.
    if spec.state == MilestoneState::Closed {
      let id = parse_created_milestone_id(&out)?;
      self.run_argv(milestone_update_argv(&self.slug, id, spec))?;
    }
    Ok(())
  }

  fn update_milestone(&self, number: u64, spec: &MilestoneSpec) -> Result<()> {
    check_due_on_is_date_only(spec)?;
    self.run_argv(milestone_update_argv(&self.slug, number, spec))?;
    Ok(())
  }

  fn delete_milestone(&self, number: u64) -> Result<()> {
    self.run_argv(milestone_delete_argv(&self.slug, number))?;
    Ok(())
  }
}
