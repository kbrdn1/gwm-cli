//! GitLab backend for the [`crate::forge::Forge`] trait (issue #419),
//! shelling out to the `glab` CLI.
//!
//! Two invocation styles, picked per operation rather than uniformly:
//!
//! - **reading issues / merge requests** goes through the first-class
//!   subcommands (`glab issue view`, `glab mr view`, `glab mr list`) with
//!   `--output json`. `glab` passes the GitLab REST object through
//!   unchanged, so the parsers below deserialize the documented API shape.
//! - **creating issues / merge requests** goes through `glab api` too.
//!   `glab issue|mr create` only takes the body as `--description
//!   <text>`, which publishes the whole rendered document on the command
//!   line for any local process to read via `ps`; `glab api --input -`
//!   sends it on stdin instead (issue #459).
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
  PrCreateRequest, PrHead, PrState, PrStatus, ReviewThreads,
};
use crate::labels::{LabelSpec, RemoteLabel};
use crate::milestones::{self, MilestoneSpec, MilestoneState, RemoteMilestone};
use serde::Deserialize;
use std::ffi::OsString;

/// Resolve the `glab` program to invoke: `$GWM_GLAB` when set (test /
/// override hook), else `glab` on `PATH`. Mirrors
/// [`crate::github::gh_program`] so both backends have the same seam.
pub fn glab_program() -> OsString {
  std::env::var_os("GWM_GLAB").unwrap_or_else(|| "glab".into())
}

/// Inherited variables that would redirect `glab` at another project.
///
/// `$GITLAB_REPO` is the flag's environment binding, `$GITLAB_GROUP` is
/// the default group for issue / MR listings, and five further names —
/// `$REMOTE_ALIAS`, `$GIT_REMOTE_ALIAS`, `$REMOTE_NICKNAME`,
/// `$GIT_REMOTE_NICKNAME`, `$GIT_REMOTE_URL_VAR` — all bind the single
/// `remote_alias` setting that picks which git remote glab reads the
/// project from. Every one overrides the working directory gwm
/// deliberately sets.
/// Cleared whenever gwm supplies a project of its own — a slug, or the
/// repo it spawns the child inside. Not cleared when it supplies
/// neither; see [`crate::github::gh_env_remove`] for why that case is
/// real and what it costs to get wrong.
///
/// The host is the asymmetric case, and it is asymmetric on purpose: gwm
/// does not always know it, and on an SSH origin the user's exported
/// value may be the only correct signal there is. So the host variables
/// are cleared only when [`glab_env`] has an authoritative value to put
/// in their place.
///
/// That last clause is the whole test, and rounds 10 and 11 of the #458
/// review disagreed about which side `$GITLAB_API_HOST` falls on. Round
/// 10 cleared it, round 11 called it orthogonal — "gwm has nothing to
/// put in its place" — and restored it. Round 11 was wrong on the fact,
/// which is why the finding came back five more times: glab's client
/// builder ends with
///
/// ```go
/// apiHost, _ := cfg.Get(repoHost, "api_host")
/// if apiHost == "" { apiHost = repoHost }
/// ```
///
/// (`internal/api/client.go`). The replacement is the pin itself. And
/// `cfg.Get` consults the environment **before** it looks at
/// `repoHost` (`internal/config/config.go::GetWithSource`), while
/// `resolveHostAndSubfolder` lets the value replace the base host
/// outright — so an inherited `$GITLAB_API_HOST` silently outranks
/// `$GITLAB_HOST` and carries the token to another tenant. `api_host`
/// is `ScopePerHost` in glab's schema; the env var is the global escape
/// hatch, so a split-host install belongs in `glab config set --host
/// <h> api_host <v>` rather than in the ambient environment.
///
/// `$GITLAB_URI` and `$GL_HOST` are the second and third spellings of
/// the `host` key itself. `$GITLAB_HOST` is first in glab's list and
/// the lookup returns the first non-empty, so neither can outrank the
/// pin — clearing them is tidiness, not hardening, and they are listed
/// together because the asymmetry is what invites the next audit.
///
/// `$GITLAB_API_HOST` and `$API_PROTOCOL` are **not** tied to the pin,
/// and that distinction is the whole of round 28's correction. Round 25
/// cleared them only behind it, which left the delegated case — an SSH
/// origin, where gwm pins nothing and glab reads the instance from the
/// repo's own remote — still inheriting them, and `api_host` wins
/// host-blind. Their replacement never depended on the pin: `repoHost`
/// is whatever glab resolved, from the pin *or* from that remote. So
/// they go in both cases.
///
/// The cost, named rather than discovered later: a split-host or
/// plain-http instance configured through the environment breaks
/// loudly. The fix is `glab config set --host <h> api_host <v>`, which
/// is where glab scopes the setting per host anyway.
///
/// The rule that would have saved six rounds: **clear only what you can
/// replace** — and check what the replacement actually is before
/// deciding you have none.
///
/// Audited against glab's schema (`internal/config/schema.go`), not its
/// README: `EnvKeyEquivalence` falls back to the upper-cased key name,
/// so every setting without an explicit `EnvVars` list still has an env
/// var. Under the three-tier rule stated on
/// [`crate::github::gh_env_remove`]. Tier 1 (cleared when gwm supplies a
/// project): `$GITLAB_REPO`, `$GITLAB_GROUP`, the five `remote_alias`
/// spellings.
/// Tier 2 (cleared or replaced only behind a pin): `$GITLAB_URI`,
/// `$GL_HOST`, `$GITLAB_API_HOST`, and `$API_PROTOCOL` — which
/// [`glab_env`] *sets* rather than clears, see there. Tier 3 (never
/// touched): `$GITLAB_TOKEN`, `$GITLAB_CLIENT_ID`, `$JOB_TOKEN` /
/// `$CI_JOB_TOKEN`, `$GLAB_ENABLE_CI_AUTOLOGIN`, `$GLAB_CONFIG_DIR` —
/// clearing the last three would break gwm inside a GitLab pipeline,
/// which is precisely where that token is the only credential there is.
///
/// `$PROXY`, `$SKIP_TLS_VERIFY`, `$CA_CERT`, `$CLIENT_CERT`,
/// `$CLIENT_KEY` and `$CUSTOM_HEADERS` are the synthesized names the
/// schema fallback produces, and they are tier 3 for the same reason:
/// they decide how the connection is made, not which project it
/// targets, and gwm has no corporate proxy or private CA to put in
/// their place.
///
/// One consequence is worth stating rather than hiding, because it is a
/// real hole and not an oversight: with `$GLAB_ENABLE_CI_AUTOLOGIN=true`
/// glab authenticates from `$CI_SERVER_FQDN` / `$CI_JOB_TOKEN` and
/// documents that it then "ignores host variables like `GITLAB_HOST`",
/// so inside a pipeline the pin yields to the CI instance. Clearing the
/// flag would close that, and would also strip gwm of the only
/// credential a pipeline has. The pin loses on purpose: a job runs on
/// the instance it runs on, and that is better ground truth than an
/// origin URL.
///
/// The remainder (`$BROWSER`, `$EDITOR`/`$VISUAL`,
/// `$GLAB_GLAMOUR_STYLE`, `$GLAB_FORCE_HYPERLINKS`, `$NO_COLOR`,
/// `$GLAB_NO_PROMPT`, `$GLAB_DEBUG*`, `$GLAB_CHECK_UPDATE`,
/// `$GLAB_SEND_TELEMETRY`,
/// `$GITLAB_RELEASE_ASSETS_USE_PACKAGE_REGISTRY`) is presentation,
/// diagnostics or telemetry and cannot retarget a call.
pub fn glab_env_remove(origin: &forge::RemoteRef, has_workdir: bool) -> Vec<&'static str> {
  // Same bound as [`crate::github::gh_env_remove`]: with no slug and no
  // repo to infer from, `$GITLAB_REPO` / `$GITLAB_GROUP` are the only
  // way the user has left to name a project, and gwm has nothing to put
  // in their place. Both backends move together here — round 22 was one
  // surface swept in three passes.
  if origin.path.is_empty() && !has_workdir {
    return Vec::new();
  }
  let mut vars = vec![
    "GITLAB_REPO",
    "GITLAB_GROUP",
    // Five spellings of one setting (`remote_alias`), per glab's config
    // schema. The README documents a subset, which is how the first
    // audit came away with two of them (Codex review #458).
    "REMOTE_ALIAS",
    "GIT_REMOTE_ALIAS",
    "REMOTE_NICKNAME",
    "GIT_REMOTE_NICKNAME",
    "GIT_REMOTE_URL_VAR",
    // Not tied to the pin, because their replacement does not depend on
    // it: glab falls back to `apiHost = repoHost`, and `repoHost` is
    // whatever glab resolved — gwm's pin when there is one, the repo's
    // own remote when gwm delegates. `api_protocol` falls back to
    // https. So clearing never leaves glab without an endpoint, in
    // either case, which is the fact round 10 (cleared, no rationale)
    // and round 11 (restored, false rationale) both missed.
    "GITLAB_API_HOST",
    "API_PROTOCOL",
  ];
  if !glab_env(origin).is_empty() {
    // These two are spellings of the `host` key itself, so they only
    // lose to a value gwm actually sets.
    vars.push("GITLAB_URI");
    vars.push("GL_HOST");
  }
  vars
}

/// Environment pinned on every `glab` spawn.
///
/// Without `$GITLAB_HOST`, `glab` resolves the instance from the *process*
/// cwd's git remote and otherwise falls back to gitlab.com (Codex review
/// #458). gwm's cwd is not reliably the repo being queried — in workspace
/// mode it is the workspace root while the row belongs to a child repo —
/// so a same-named project on the wrong instance could be read and its
/// iid persisted into the local git config.
///
/// Nothing is pinned unless the origin is **authoritative**: an SSH remote
/// carries no web scheme or port, so `https://<ssh-host>` is a guess, and
/// forcing a guess over a working `glab` configuration (different web
/// hostname, plain HTTP, non-standard port) breaks setups that were fine.
/// An empty slug is likewise left alone — that is the caller asking `glab`
/// to infer the project locally, and pinning gitlab.com there would create
/// the issue / MR on the wrong instance entirely.
///
/// The cases left unpinned are not left unprotected: the child is spawned
/// **inside the repo** (see [`forge::Forge::workdir`]), so `glab` resolves
/// the instance from that repo's own remote rather than from gwm's cwd.
///
/// `$API_PROTOCOL` rides along because the host pin alone does not carry
/// the scheme: glab strips it off the hostname and takes the protocol
/// from the separate `api_protocol` setting, defaulting to https
/// (`internal/api/client.go`, `internal/glinstance/host.go`). That
/// setting has no explicit `EnvVars` in glab's schema, so
/// `EnvKeyEquivalence` synthesizes the bare name `API_PROTOCOL` — an
/// inherited one downgrades a pinned https instance to cleartext, and
/// merely clearing it would force https onto a plain-http instance. gwm
/// parses the scheme out of the origin, so it sets the value instead
/// (Codex review #458).
pub fn glab_env(origin: &forge::RemoteRef) -> Vec<(String, String)> {
  if origin.trust != forge::OriginTrust::FromUrl || origin.path.is_empty() {
    return Vec::new();
  }
  let mut env = vec![("GITLAB_HOST".to_string(), origin.web_origin.clone())];
  if let (Some(scheme), _) = origin_scheme_and_port(origin) {
    env.push(("API_PROTOCOL".to_string(), scheme));
  }
  env
}

/// `true` when `glab` would sign in from the pipeline's own CI variables.
///
/// Both halves of glab's condition, compared the way glab compares them:
/// `os.Getenv("GLAB_ENABLE_CI_AUTOLOGIN") == "true" &&
/// os.Getenv("GITLAB_CI") == "true"`
/// (`internal/config/config_mapping.go::EnvKeyEquivalence`) — a literal
/// match, no trimming and no case folding. gwm read only the first
/// variable and accepted `1` / `yes` / `TRUE`, so a developer who
/// exports the flag outside a pipeline had every glab call refused for a
/// mode glab would never have entered. Mirroring glab exactly is also
/// the safe direction: where glab does not auto-login, the pin holds and
/// there is no divergence left to protect against.
///
/// One predicate, two callers ([`ci_autologin_conflict`] and
/// [`resolve_selector`]) — the gate diverging between them is the
/// failure this shape prevents.
fn ci_autologin_active() -> bool {
  std::env::var("GLAB_ENABLE_CI_AUTOLOGIN").as_deref() == Ok("true")
    && std::env::var("GITLAB_CI").as_deref() == Ok("true")
}

/// Refuse to run when glab's CI auto-login would authenticate against a
/// different instance than the one gwm resolved.
///
/// With `$GLAB_ENABLE_CI_AUTOLOGIN=true` glab signs in from
/// `$CI_SERVER_FQDN` / `$CI_JOB_TOKEN` and documents that it then
/// "ignores host variables like `GITLAB_HOST`" — so the pin yields and a
/// same-named project on the runner's instance can be read, or worse
/// written: `labels push --prune` and `milestones push --prune` delete.
///
/// Clearing the flag was the obvious move and the wrong one — it also
/// strips a pipeline of its only credential, breaking the normal case
/// where the job runs on the instance that hosts the repo and the two
/// agree anyway. Comparing them costs nothing there and fails closed
/// only on a genuine divergence (Codex review #458, raised four times
/// before this shape was agreed; see issue #460 for the general problem).
///
/// Read once, at construction, so the TUI's fetch worker never re-reads
/// the environment off-thread (issue #217).
pub fn ci_autologin_conflict(origin: &forge::RemoteRef) -> Option<String> {
  if !ci_autologin_active() {
    return None;
  }

  // A guessed origin gives only the SSH hostname, so comparing it to
  // the runner's *web* host manufactures a conflict and blocks every
  // call on a valid split-host install. But GitLab publishes
  // `CI_SERVER_SHELL_SSH_HOST` next to `CI_SERVER_HOST`, and that is the
  // right-hand side for this case rather than a reason to give up on it
  // (Codex review #458): a job handling an SSH checkout of another
  // instance can still read or prune the runner tenant's same-named
  // project. Without the variable there is nothing to compare, and
  // abstaining beats guessing.
  if origin.trust != forge::OriginTrust::FromUrl {
    let ssh_host = non_empty(std::env::var("CI_SERVER_SHELL_SSH_HOST").ok())?;
    if ssh_host.eq_ignore_ascii_case(&origin.host) {
      return None;
    }
    return Some(format!(
      "refusing to run glab: CI auto-login would authenticate against the instance at \
       '{ssh_host}' but this repo's origin is '{}'. glab ignores GITLAB_HOST in that \
       mode, so the call would target the wrong instance. Unset \
       GLAB_ENABLE_CI_AUTOLOGIN to proceed.",
      origin.host
    ));
  }

  // `CI_SERVER_FQDN` is `gitlab.example.com:8080` — host AND port —
  // while `origin.host` never carries one, so comparing them raw refused
  // legitimate pipelines. GitLab publishes the pieces separately;
  // prefer those, fall back to splitting the FQDN or the URL.
  let (url_host, url_port, url_scheme) = match std::env::var("CI_SERVER_URL").ok() {
    Some(u) => split_origin_url(u.trim()),
    None => (None, None, None),
  };
  let (fqdn_host, fqdn_port) = match std::env::var("CI_SERVER_FQDN").ok() {
    Some(f) => split_host_port(f.trim()),
    None => (None, None),
  };
  let ci_host = non_empty(std::env::var("CI_SERVER_HOST").ok())
    .or(url_host)
    .or(fqdn_host)?;
  let ci_scheme = non_empty(std::env::var("CI_SERVER_PROTOCOL").ok()).or(url_scheme);
  let ci_port = non_empty(std::env::var("CI_SERVER_PORT").ok())
    .or(url_port)
    .or(fqdn_port);

  // Resolve the implicit port from the scheme on both sides: an https
  // origin on 443 and a runner on `:8443` are two instances, and
  // comparing only the *explicit* ports called them equal.
  let ci = (
    ci_host.to_ascii_lowercase(),
    effective_port(ci_port.as_deref(), ci_scheme.as_deref()),
  );
  let (origin_scheme, origin_port) = origin_scheme_and_port(origin);
  let ours = (
    origin.host.to_ascii_lowercase(),
    effective_port(origin_port.as_deref(), origin_scheme.as_deref()),
  );

  // Ports decide only when both sides resolve one. An unknown port is
  // not evidence of anything, and treating it as one turned the guard
  // into a blanket refusal the moment the runner published no scheme.
  let host_differs = ci.0 != ours.0;
  let port_differs = matches!((&ci.1, &ours.1), (Some(a), Some(b)) if a != b);
  if !host_differs && !port_differs {
    return None;
  }

  Some(format!(
    "refusing to run glab: CI auto-login would authenticate against '{}' but this \
     repo's origin is '{}'. glab ignores GITLAB_HOST in that mode, so the call would \
     target the wrong instance. Unset GLAB_ENABLE_CI_AUTOLOGIN to proceed.",
    render(&ci),
    render(&ours),
  ))
}

fn non_empty(v: Option<String>) -> Option<String> {
  v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn render((host, port): &(String, Option<String>)) -> String {
  match port {
    Some(p) => format!("{host}:{p}"),
    None => host.clone(),
  }
}

/// The port actually addressed: the explicit one, else the scheme's
/// default. `None` only when neither is known.
fn effective_port(port: Option<&str>, scheme: Option<&str>) -> Option<String> {
  if let Some(p) = port {
    return Some(p.to_string());
  }
  match scheme.map(|s| s.to_ascii_lowercase()).as_deref() {
    Some("https") => Some("443".into()),
    Some("http") => Some("80".into()),
    _ => None,
  }
}

fn origin_scheme_and_port(origin: &forge::RemoteRef) -> (Option<String>, Option<String>) {
  let (_, port, scheme) = split_origin_url(&origin.web_origin);
  (scheme, port)
}

/// Split `scheme://host[:port]` into its three parts.
fn split_origin_url(url: &str) -> (Option<String>, Option<String>, Option<String>) {
  let (scheme, rest) = match url.split_once("://") {
    Some((s, r)) => (Some(s.to_string()), r),
    None => (None, url),
  };
  let rest = rest.trim_end_matches('/');
  let rest = rest.split('/').next().unwrap_or(rest);
  let (host, port) = split_host_port(rest);
  (host, port, scheme)
}

/// Split `host[:port]`, keeping the port only when it is all digits.
fn split_host_port(s: &str) -> (Option<String>, Option<String>) {
  if s.is_empty() {
    return (None, None);
  }
  match s.rsplit_once(':') {
    Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
      (Some(h.to_string()), Some(p.to_string()))
    }
    _ => (Some(s.to_string()), None),
  }
}

/// The project selector for `glab`, decided once at construction.
///
/// Empty for a guessed origin with a repo to stand in: hand glab nothing
/// and let it read that repo's own remote.
///
/// Otherwise the origin path, minus any **instance subfolder**. glab
/// supports GitLab hosted under a URL prefix (`https://host/gitlab/`)
/// and reads it from `$GITLAB_SUBFOLDER` as well as its config file, so
/// its API base already carries the prefix — leaving it on the slug
/// targets `gitlab/group/proj`, a project that does not exist (Codex
/// review #458). Only the API selector is stripped: the web URLs keep
/// the full path, because that really is where the pages live.
///
/// Inside a pipeline the variable is not the only source. glab's
/// auto-login overrides replace the schema's env list for `subfolder`
/// with `{GITLAB_SUBFOLDER, CI_SERVER_URL}` and derive the second from
/// the URL's path (`extractSubfolderFromURL`), so an instance served
/// under `/gitlab` needs no explicit variable for glab's API base to
/// carry the prefix. gwm mirrors that list, in that order — the
/// explicit variable still wins, and a `$CI_SERVER_URL` with no path
/// contributes nothing, exactly as glab's `continue` arm does.
///
/// Two limits worth naming rather than leaving silent.
///
/// The config-file half of the setting stays out of reach; gwm does not
/// parse glab's config. An instance configured that way, accessed over
/// https, still needs the slug it would have to guess.
///
/// And glab scopes `subfolder` **per host** in its config, while the
/// environment variable is global. gwm resolves a forge per repo, so a
/// workspace spanning two instances applies one exported value to both.
/// The strip is still conditional on the origin path actually carrying
/// the prefix, so the damage needs a second instance whose top-level
/// group is named like the first's subfolder.
///
/// Raised three times in the #458 review, so this is the decision and
/// not a gap: tying the value to a host needs a trusted host source,
/// which is issue #460. Adding a fourth heuristic here is what produced
/// the round 9-to-16 oscillation, and the `api_host` half of the same
/// problem is already closed by the pin in [`glab_env_remove`].
///
/// Read here rather than at call time so the TUI's fetch worker never
/// re-reads the environment off-thread (issue #217).
fn resolve_selector(origin: &forge::RemoteRef, has_workdir: bool) -> String {
  if origin.trust != forge::OriginTrust::FromUrl && has_workdir {
    return String::new();
  }
  let Some(sub) = subfolder_from_env() else {
    return origin.path.clone();
  };
  origin
    .path
    .strip_prefix(&format!("{sub}/"))
    .unwrap_or(&origin.path)
    .to_string()
}

/// The instance subfolder glab would resolve, from the same variables in
/// the same order. See [`resolve_selector`] for why the CI arm exists.
fn subfolder_from_env() -> Option<String> {
  let trimmed = |v: String| {
    let s = v.trim().trim_matches('/').to_string();
    (!s.is_empty()).then_some(s)
  };
  if let Some(sub) = std::env::var("GITLAB_SUBFOLDER").ok().and_then(trimmed) {
    return Some(sub);
  }
  if !ci_autologin_active() {
    return None;
  }
  // `extractSubfolderFromURL`: the URL's path, slash-trimmed. Empty
  // means "no subfolder", not "fall back to the default".
  let url = std::env::var("CI_SERVER_URL").ok()?;
  let rest = url.trim().split_once("://").map(|(_, r)| r).unwrap_or(url.trim());
  let (_, path) = rest.split_once('/')?;
  let path = path.trim_matches('/').to_string();
  (!path.is_empty()).then_some(path)
}

/// Percent-encode one URL path segment. A GitLab project path contains
/// slashes (`group/sub/proj`) and must arrive as a single encoded
/// segment for `projects/:id` to resolve it.
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

/// `--repo <slug>`, or nothing when the slug is empty.
///
/// Two callers rely on the empty case, both wanting `glab` to infer the
/// project from its working directory: an unresolvable `origin` on the
/// creation paths (see [`crate::forge::resolve_or_default`]), and a
/// guessed SSH origin, where passing a slug would make glab resolve it
/// against its *default* host (see
/// [`crate::forge::Forge::repo_selector`]).
fn repo_flag(slug: &str) -> Vec<String> {
  if slug.is_empty() {
    Vec::new()
  } else {
    vec!["--repo".into(), slug.into()]
  }
}

/// `projects/<url-encoded path>` — the REST prefix every `glab api` call
/// below is rooted at.
///
/// An empty slug yields `projects/:fullpath`, the placeholder `glab api`
/// substitutes from the repo in its working directory (Codex review
/// #458). That keeps the REST paths on the same rule as the subcommands:
/// rather than baking a slug that would be resolved against the wrong
/// host, let glab resolve the project itself.
fn project_path(slug: &str) -> String {
  if slug.is_empty() {
    return "projects/:fullpath".to_string();
  }
  format!("projects/{}", encode_segment(slug))
}

// ---- issues --------------------------------------------------------------

/// GitLab counterpart of [`crate::github::null_to_default`] (Codex review
/// #529): read `null` as the type's default rather than aborting the
/// parse. More likely to fire here than on the GitHub side, since
/// `"description": null` is what GitLab sends for an issue or MR that has
/// none, which is the common case.
fn null_to_default<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
  D: serde::Deserializer<'de>,
  T: Deserialize<'de> + Default,
{
  Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize)]
struct RawIssue {
  /// The project-scoped number the user sees and every URL uses. The
  /// sibling `id` is a global counter and is deliberately ignored:
  /// picking it would fail silently, producing wrong URLs and wrong
  /// follow-up fetches with no error anywhere.
  iid: u64,
  title: String,
  state: String,
  #[serde(default, deserialize_with = "null_to_default")]
  labels: Vec<String>,
  #[serde(default, deserialize_with = "null_to_default")]
  updated_at: String,
  #[serde(default, deserialize_with = "null_to_default")]
  web_url: String,
  /// GitLab's name for the issue body (issue #420).
  #[serde(default, deserialize_with = "null_to_default")]
  description: String,
  #[serde(default, deserialize_with = "null_to_default")]
  author: Option<RawAuthor>,
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
    // Partial rich tier (issue #420): the body and the author ride along
    // in the payload gwm already asks for, so they cost nothing. Comments
    // are GitLab *notes* behind a second `/notes` request and stay out —
    // the rich view renders "no comments" from an honest empty vector
    // rather than pretending the thread is empty.
    detail: crate::forge::IssueDetail {
      body: raw.description,
      author: raw.author.unwrap_or_default().username,
      comments: Vec::new(),
    },
  })
}

pub fn issue_view_argv(slug: &str, number: u64) -> Vec<String> {
  let mut argv = vec!["issue".into(), "view".into(), number.to_string()];
  argv.extend(repo_flag(slug));
  argv.extend(["--output".into(), "json".into()]);
  argv
}

// ---- merge requests ------------------------------------------------------

#[derive(Deserialize)]
struct RawMr {
  iid: u64,
  #[serde(default, deserialize_with = "null_to_default")]
  title: String,
  state: String,
  /// `draft` superseded `work_in_progress`; older self-hosted instances
  /// still only send the legacy key, so both are read.
  #[serde(default, deserialize_with = "null_to_default")]
  draft: bool,
  #[serde(default, deserialize_with = "null_to_default")]
  work_in_progress: bool,
  #[serde(default, deserialize_with = "null_to_default")]
  web_url: String,
  #[serde(default, deserialize_with = "null_to_default")]
  updated_at: String,
  #[serde(default, deserialize_with = "null_to_default")]
  source_branch: String,
  #[serde(default, deserialize_with = "null_to_default")]
  target_branch: String,
  // `Option` (not just `#[serde(default, deserialize_with = "null_to_default")]`) so an explicit
  // `"author": null` — a deleted account — deserialises to `None`
  // instead of erroring; `default` alone only covers a *missing* key.
  #[serde(default, deserialize_with = "null_to_default")]
  author: Option<RawAuthor>,
  #[serde(default, deserialize_with = "null_to_default")]
  head_pipeline: Option<RawPipeline>,
  /// GitLab's name for the MR body (issue #420).
  #[serde(default, deserialize_with = "null_to_default")]
  description: String,
}

#[derive(Deserialize, Default)]
struct RawAuthor {
  #[serde(default, deserialize_with = "null_to_default")]
  username: String,
}

#[derive(Deserialize)]
struct RawPipeline {
  #[serde(default, deserialize_with = "null_to_default")]
  status: String,
  #[serde(default, deserialize_with = "null_to_default")]
  web_url: Option<String>,
  #[serde(default, deserialize_with = "null_to_default")]
  started_at: Option<String>,
  #[serde(default, deserialize_with = "null_to_default")]
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
    // Partial rich tier (issue #420), same rule as the issue side: the
    // body, the author and the branch pair are already in this payload.
    // `additions` / `deletions` are not — GitLab only reports a diff size
    // through a separate `/changes` request — so they stay 0 and the view
    // omits the diff line rather than printing a fabricated `+0 −0`.
    // Reviews (approvals) and comments (notes) are separate requests too.
    detail: crate::forge::PrDetail {
      body: raw.description,
      author: raw.author.unwrap_or_default().username,
      base_ref: raw.target_branch,
      head_ref: raw.source_branch,
      ..Default::default()
    },
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
  let mut argv = vec!["mr".into(), "view".into(), number.to_string()];
  argv.extend(repo_flag(slug));
  argv.extend(["--output".into(), "json".into()]);
  argv
}

/// Argv for `glab mr list --repo <slug> --source-branch <branch> --all
/// --output json --per-page 1`. `--all` is the load-bearing bit — the
/// GitHub counterpart's `--state all`: a closed or merged MR for the
/// branch is still detected, and its state is resolved later via
/// [`parse_mr_json`].
pub fn mr_list_argv(slug: &str, branch: &str) -> Vec<String> {
  let mut argv = vec!["mr".into(), "list".into()];
  argv.extend(repo_flag(slug));
  argv.extend([
    "--source-branch".into(),
    branch.into(),
    "--all".into(),
    "--output".into(),
    "json".into(),
    // More than one row on purpose: `--source-branch` matches the branch
    // NAME only, so a fork carrying the same name can appear. The
    // same-project MR is picked in `parse_mr_list_number`, which needs
    // candidates to pick from (Codex review #458).
    //
    // A page, not a sample: 20 rows meant the project's own MR could sit
    // behind twenty fork MRs sharing the branch name and never be looked
    // at, so detection returned nothing at all. 100 is GitLab's page cap,
    // and there is no server-side filter for "source project = this one"
    // to narrow it further.
    "--per-page".into(),
    "100".into(),
  ]);
  argv
}

/// Parse the JSON array printed by `glab mr list --output json`,
/// returning the first MR opened from **this** project.
///
/// `--source-branch` constrains the branch name but not the source
/// project, so a fork whose branch shares the name shows up here too
/// (Codex review #458) — and its iid would be persisted as this branch's
/// `gwm-pr-detected`, silently linking the worktree to a stranger's MR.
/// A same-project MR is the one whose `source_project_id` matches the
/// target `project_id`; a payload that reports neither is kept, since
/// dropping it would break detection on older instances outright.
pub fn parse_mr_list_number(s: &str) -> Result<Option<u64>> {
  #[derive(Deserialize)]
  struct MrRef {
    iid: u64,
    #[serde(default)]
    project_id: Option<u64>,
    #[serde(default)]
    source_project_id: Option<u64>,
  }
  let arr: Vec<MrRef> = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: "gitlab mr list",
    source: e,
  })?;
  // Rank the evidence rather than taking the first row that is not a
  // proven fork. `_ => true` let an MR with missing ids win against a
  // later one that positively identified itself as ours — and a fork MR
  // whose source project was deleted reports exactly that shape (Codex
  // review #458).
  //
  // 1. `project_id == source_project_id`: proven ours.
  // 2. ids absent: older instances omit them, and dropping those rows
  //    would break detection outright — so they are second, not first.
  // 3. a proven fork: better than reporting nothing, worse than anything
  //    that might be ours. Same ranking as the GitHub side, and for the
  //    same reason — `source_project_id != project_id` is exactly the
  //    shape of the standard contributor workflow (fork the project,
  //    push the branch, open the MR upstream), so treating it as a
  //    disqualification stops detecting the MR the user actually
  //    opened. It is positive evidence of a fork, not of someone
  //    else's fork; separating those needs the source project's owner
  //    matched against the repo's remotes (issue #461).
  let ours = |m: &MrRef| matches!((m.project_id, m.source_project_id), (Some(t), Some(s)) if t == s);
  let unidentified = |m: &MrRef| m.project_id.is_none() || m.source_project_id.is_none();
  Ok(
    arr
      .iter()
      .find(|m| ours(m))
      .or_else(|| arr.iter().find(|m| unidentified(m)))
      .or_else(|| arr.first())
      .map(|m| m.iid),
  )
}

// ---- create --------------------------------------------------------------

// ---- creation via `glab api` (issue #459) --------------------------------
//
// `glab issue|mr create` only accepts the body as `--description
// <text>`, which puts the whole rendered document on the command line
// where `ps` shows it to every local process. `gh` has `--body-file`,
// so the GitHub path never had this problem; going through `glab api
// --input -` gives the GitLab path the same property by sending the
// request body on stdin.

/// Argv for creating an issue through the REST API. Body-free by
/// construction: everything sensitive travels on stdin.
pub fn issue_create_api_argv(slug: &str) -> Vec<String> {
  api_post_argv(slug, "issues")
}

/// Argv for creating a merge request through the REST API.
pub fn mr_create_api_argv(slug: &str) -> Vec<String> {
  api_post_argv(slug, "merge_requests")
}

fn api_post_argv(slug: &str, collection: &str) -> Vec<String> {
  vec![
    "api".into(),
    "-X".into(),
    "POST".into(),
    format!("{}/{}", project_path(slug), collection),
    "--input".into(),
    "-".into(),
  ]
}

/// JSON request body for `POST /projects/:id/issues`.
pub fn issue_create_payload(title: &str, body: &str, labels: &[String]) -> String {
  // `labels` as a comma-separated string rather than an array: both are
  // accepted today, the string form also works on older instances.
  serde_json::json!({
    "title": title,
    "description": body,
    "labels": labels.join(","),
  })
  .to_string()
}

/// JSON request body for `POST /projects/:id/merge_requests`.
///
/// Two divergences from `glab mr create` that the CLI hid:
/// `target_branch` is mandatory on the endpoint (the CLI inferred the
/// default branch), and there is no `draft` field — draft state is
/// carried by a `Draft:` title prefix, which is exactly what the CLI
/// did client-side.
pub fn mr_create_payload(title: &str, body: &str, head: &str, base: Option<&str>, draft: bool) -> Result<String> {
  let base = base.ok_or_else(|| {
    GwmError::Other(
      "creating a GitLab merge request needs an explicit target branch: the REST endpoint has no default".into(),
    )
  })?;
  let title = if draft {
    format!("Draft: {title}")
  } else {
    title.to_string()
  };
  Ok(
    serde_json::json!({
      "title": title,
      "description": body,
      "source_branch": head,
      "target_branch": base,
    })
    .to_string(),
  )
}

/// Read the `iid` and server-reported `web_url` back off a created
/// object. Both come from the API response, so the URL is the
/// instance's own rather than one gwm reconstructed.
pub fn parse_created_api(s: &str, kind: &'static str) -> Result<(u64, String)> {
  #[derive(Deserialize)]
  struct Created {
    iid: u64,
    #[serde(default)]
    web_url: String,
  }
  let c: Created = serde_json::from_str(s).map_err(|e| GwmError::GhJsonParse {
    kind: match kind {
      "issue" => "gitlab created issue",
      _ => "gitlab created mr",
    },
    source: e,
  })?;
  Ok((c.iid, c.web_url))
}

// ---- labels --------------------------------------------------------------

#[derive(Deserialize)]
struct RawLabel {
  name: String,
  /// `false` for a label inherited from an ancestor group. Absent on
  /// older self-managed payloads, where the label is kept.
  #[serde(default)]
  is_project_label: Option<bool>,
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
/// Read `glab api --paginate` output: **one JSON array per page**, not
/// one merged array.
///
/// `gh api --paginate` merges, and the GitLab side was written assuming
/// glab did the same. It does not: the request loop calls
/// `processResponse` once per page and that function ends in
/// `io.Copy(opts.io.StdOut, responseBody)`
/// (`internal/commands/api/api.go` @ v1.68.0), so stdout carries
/// `[…][…]` — concatenated values. `serde_json::from_str` rejects the
/// second one as trailing characters, and only past the 100-row first
/// page, which is exactly the project whose `--prune` matters most
/// (Codex review #458).
///
/// Empty input stays an **error**. Reading it as an empty remote would
/// hand `labels push --prune` / `milestones push --prune` a baseline
/// that says the remote owns nothing.
fn parse_paginated_array<T: serde::de::DeserializeOwned>(s: &str, kind: &'static str) -> Result<Vec<T>> {
  let mut out = Vec::new();
  let mut pages = 0usize;
  for page in serde_json::Deserializer::from_str(s).into_iter::<Vec<T>>() {
    out.extend(page.map_err(|e| GwmError::GhJsonParse { kind, source: e })?);
    pages += 1;
  }
  if pages == 0 {
    return Err(GwmError::Other(format!(
      "{kind}: glab returned no JSON at all — treating that as an empty \
       remote would let --prune run against a baseline it never read"
    )));
  }
  Ok(out)
}

pub fn parse_labels_json(s: &str) -> Result<Vec<RemoteLabel>> {
  let raw: Vec<RawLabel> = parse_paginated_array(s, "gitlab labels")?;
  Ok(
    raw
      .into_iter()
      // Belt and braces behind `include_ancestor_groups=false`: an older
      // self-managed instance that ignores the parameter must still not
      // feed group labels into a project-scoped prune.
      .filter(|r| r.is_project_label.unwrap_or(true))
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
/// The pages arrive as separate JSON arrays — see
/// [`parse_paginated_array`], which is what reads them.
///
/// `include_ancestor_groups=false` is load-bearing (Codex review #458):
/// GitLab defaults it to **true**, so the plain query also returns the
/// parent groups' labels. The shared diff engine reads those as extras —
/// `gwm labels push --prune` then proposes deleting labels the project
/// does not own, and issues a project-scoped DELETE that fails.
pub fn label_list_argv(slug: &str) -> Vec<String> {
  vec![
    "api".into(),
    "--paginate".into(),
    format!(
      "{}/labels?per_page=100&include_ancestor_groups=false",
      project_path(slug)
    ),
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
/// An absent description is sent **empty**, not omitted (Codex review
/// #458). `.gwm.toml` declares the desired state, so dropping a label's
/// `description` means "this label has none"; omitting the field left the
/// remote value in place and the diff replayed the same update forever.
pub fn label_update_argv(slug: &str, spec: &LabelSpec) -> Vec<String> {
  vec![
    "api".into(),
    "-X".into(),
    "PUT".into(),
    format!("{}/labels/{}", project_path(slug), encode_segment(&spec.name)),
    "--raw-field".into(),
    format!("color=#{}", spec.color),
    "--raw-field".into(),
    format!(
      "description={}",
      spec.description.as_deref().filter(|s| !s.is_empty()).unwrap_or("")
    ),
  ]
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
  let raw: Vec<RawMilestone> = parse_paginated_array(s, "gitlab milestones")?;
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
/// the page. The pages arrive as separate JSON arrays — see
/// [`parse_paginated_array`].
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
///
/// Absent optionals are sent **empty**, not omitted (Codex review #458):
/// the declared set is the desired state, so removing `description` or
/// `due_on` from `.gwm.toml` must clear them upstream. Omitting the fields
/// left stale remote data in place and made every push replay the same
/// update without ever converging.
pub fn milestone_update_argv(slug: &str, number: u64, spec: &MilestoneSpec) -> Vec<String> {
  vec![
    "api".into(),
    "-X".into(),
    "PUT".into(),
    format!("{}/milestones/{}", project_path(slug), number),
    "--raw-field".into(),
    format!("title={}", spec.title),
    "--raw-field".into(),
    format!(
      "description={}",
      spec.description.as_deref().filter(|s| !s.is_empty()).unwrap_or("")
    ),
    "--raw-field".into(),
    format!(
      "due_date={}",
      spec
        .due_on
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(due_date_field)
        .unwrap_or("")
    ),
    "--raw-field".into(),
    format!("state_event={}", state_event(spec.state)),
  ]
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
  origin: forge::RemoteRef,
  program: OsString,
  env: Vec<(String, String)>,
  env_remove: Vec<&'static str>,
  workdir: Option<std::path::PathBuf>,
  /// Why this forge must refuse to run, decided once at construction.
  /// `None` is the normal case.
  refuse: Option<String>,
  /// The project selector handed to `glab`, resolved once at
  /// construction: the origin path minus any instance subfolder, or
  /// empty when the CLI should resolve the project itself.
  selector: String,
}

impl GitLabForge {
  /// Resolves `$GWM_GLAB` **now**, on the calling thread, so a forge
  /// handed to the TUI's fetch worker never re-reads the process
  /// environment concurrently with env-mutating code (issue #217).
  pub fn new(origin: forge::RemoteRef, workdir: Option<std::path::PathBuf>) -> Self {
    Self {
      env: glab_env(&origin),
      env_remove: glab_env_remove(&origin, workdir.is_some()),
      refuse: ci_autologin_conflict(&origin),
      selector: resolve_selector(&origin, workdir.is_some()),
      origin,
      program: glab_program(),
      workdir,
    }
  }

  fn run_argv(&self, argv: Vec<String>) -> Result<String> {
    self.run_argv_with_stdin(argv, None)
  }

  /// A read whose response is a whole REST object, so the transcript
  /// gets the outcome and not the payload.
  ///
  /// `glab issue|mr view --output json` returns `description` — the same
  /// text #459 went to the trouble of keeping off the argv on the way
  /// out. Withholding it on create and printing it back on the next read
  /// is not a rule, it is a gap (Codex review #458).
  fn run_argv_object(&self, argv: Vec<String>) -> Result<String> {
    self.run_spawn(argv, None, true)
  }

  /// `stdin` carries the request body for the `glab api` creation paths,
  /// which is the whole reason it exists: it keeps the rendered text out
  /// of the argv (issue #459).
  fn run_argv_with_stdin(&self, argv: Vec<String>, stdin: Option<&[u8]>) -> Result<String> {
    let redact = stdin.is_some();
    self.run_spawn(argv, stdin, redact)
  }

  fn run_spawn(&self, argv: Vec<String>, stdin: Option<&[u8]>, redact_output: bool) -> Result<String> {
    if let Some(why) = &self.refuse {
      return Err(GwmError::Other(why.clone()));
    }
    // A stdin payload is, by construction, the one thing that must not
    // reach the transcript — and the create endpoints echo it back.
    // Redacting stdout is not enough on its own: `$GLAB_DEBUG_HTTP`
    // dumps whole requests and responses, bodies included, to *stderr*,
    // which the transcript keeps and the error path quotes verbatim
    // (Codex review #458). The env audit that produced the three-tier
    // rule filed both debug variables under "cannot retarget a call",
    // which was true and beside the point — it only ever asked what
    // could redirect a call, never what could disclose one.
    //
    // Cleared only for the calls that actually carry a body, so
    // debugging every other operation still works.
    let mut env_remove: Vec<&'static str> = self.env_remove.clone();
    if redact_output {
      env_remove.extend_from_slice(&["GLAB_DEBUG_HTTP", "GLAB_DEBUG"]);
    }
    forge::run_cli_with(
      &self.program,
      argv,
      &forge::CliSpawn {
        env: &self.env,
        cwd: self.workdir.as_deref(),
        env_remove: &env_remove,
        redact_after: &[],
        stdin,
        redact_output,
      },
    )
  }
}

impl Forge for GitLabForge {
  fn kind(&self) -> ForgeKind {
    ForgeKind::GitLab
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

  fn repo_selector(&self) -> &str {
    &self.selector
  }

  fn issue_url(&self, number: u64) -> String {
    format!("{}/{}/-/issues/{}", self.origin.web_origin, self.origin.path, number)
  }

  fn pr_url(&self, number: u64) -> String {
    format!(
      "{}/{}/-/merge_requests/{}",
      self.origin.web_origin, self.origin.path, number
    )
  }

  fn pr_head_refspec(&self, number: u64) -> String {
    format!("merge-requests/{number}/head")
  }

  fn fetch_issue(&self, number: u64) -> Result<IssueStatus> {
    parse_issue_json(&self.run_argv_object(issue_view_argv(self.repo_selector(), number))?)
  }

  fn fetch_pr(&self, number: u64) -> Result<PrStatus> {
    parse_mr_json(&self.run_argv_object(mr_view_argv(self.repo_selector(), number))?)
  }

  fn fetch_pr_head(&self, number: u64) -> Result<PrHead> {
    parse_mr_head_json(&self.run_argv_object(mr_view_argv(self.repo_selector(), number))?)
  }

  /// GitLab carries inline comments as *discussion notes* with `position`
  /// data, under `/merge_requests/:iid/discussions` — a different shape
  /// behind a different request, which issue #528 deliberately leaves out
  /// of scope. Answered explicitly rather than as an empty list, so the
  /// view can say "gwm cannot show these here" instead of the false
  /// "this MR has none".
  fn fetch_pr_threads(&self, _number: u64) -> Result<ReviewThreads> {
    Ok(ReviewThreads::Unsupported)
  }

  fn find_pr_for_branch(&self, branch: &str) -> Result<Option<u64>> {
    parse_mr_list_number(&self.run_argv_object(mr_list_argv(self.repo_selector(), branch))?)
  }

  fn create_issue(&self, req: &IssueCreateRequest<'_>) -> Result<CreatedIssue> {
    let body = forge::read_body_file(req.body_file)?;
    let payload = issue_create_payload(req.title, &body, req.labels);
    let out = self.run_argv_with_stdin(issue_create_api_argv(self.repo_selector()), Some(payload.as_bytes()))?;
    let (number, url) = parse_created_api(&out, "issue")?;
    Ok(CreatedIssue {
      number,
      url: if url.is_empty() { self.issue_url(number) } else { url },
    })
  }

  fn create_pr(&self, req: &PrCreateRequest<'_>) -> Result<CreatedPr> {
    let body = forge::read_body_file(req.body_file)?;
    let payload = mr_create_payload(req.title, &body, req.head, req.base, req.draft)?;
    let out = self.run_argv_with_stdin(mr_create_api_argv(self.repo_selector()), Some(payload.as_bytes()))?;
    let (number, url) = parse_created_api(&out, "mr")?;
    Ok(CreatedPr {
      number,
      url: if url.is_empty() { self.pr_url(number) } else { url },
    })
  }

  fn fetch_remote_labels(&self) -> Result<Vec<RemoteLabel>> {
    parse_labels_json(&self.run_argv(label_list_argv(self.repo_selector()))?)
  }

  fn create_label(&self, spec: &LabelSpec) -> Result<()> {
    self.run_argv(label_create_argv(self.repo_selector(), spec))?;
    Ok(())
  }

  fn update_label(&self, spec: &LabelSpec) -> Result<()> {
    self.run_argv(label_update_argv(self.repo_selector(), spec))?;
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
    self.run_argv(label_delete_argv(self.repo_selector(), name))?;
    Ok(())
  }

  fn fetch_remote_milestones(&self) -> Result<Vec<RemoteMilestone>> {
    parse_milestones_json(&self.run_argv(milestone_list_argv(self.repo_selector()))?)
  }

  fn validate_milestone(&self, spec: &MilestoneSpec) -> Result<()> {
    check_due_on_is_date_only(spec)
  }

  fn create_milestone(&self, spec: &MilestoneSpec) -> Result<()> {
    check_due_on_is_date_only(spec)?;
    let out = self.run_argv(milestone_create_argv(self.repo_selector(), spec))?;
    // GitLab has no `state` on create, so a declared-closed milestone
    // needs a second call to transition it. Keyed on the id echoed back
    // by the POST.
    if spec.state == MilestoneState::Closed {
      let id = parse_created_milestone_id(&out)?;
      self.run_argv(milestone_update_argv(self.repo_selector(), id, spec))?;
    }
    Ok(())
  }

  fn update_milestone(&self, number: u64, spec: &MilestoneSpec) -> Result<()> {
    check_due_on_is_date_only(spec)?;
    self.run_argv(milestone_update_argv(self.repo_selector(), number, spec))?;
    Ok(())
  }

  fn delete_milestone(&self, number: u64) -> Result<()> {
    self.run_argv(milestone_delete_argv(self.repo_selector(), number))?;
    Ok(())
  }
}
