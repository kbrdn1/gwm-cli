//! Unit tests for the `forge` module (issue #419): origin-remote parsing
//! into host + path, forge kind detection / override, and the parts of the
//! [`gwm::forge::Forge`] contract that are pure (nouns, URL builders).
//!
//! Everything here stays free of a `gh` / `glab` shell-out: CI runners have
//! neither binary, so the network methods are covered by the per-backend
//! parser tests (`github_tests.rs`, `gitlab_tests.rs`) instead.

mod common;

use common::init_repo;
use gwm::config::Config;
use gwm::forge::{self, ForgeKind};

// --- origin URL parsing ---------------------------------------------------

#[test]
fn parse_remote_url_handles_github_ssh_scp_syntax() {
  let r = forge::parse_remote_url("git@github.com:kbrdn1/gwm-cli.git").unwrap();

  assert_eq!(r.host, "github.com");
  assert_eq!(r.path, "kbrdn1/gwm-cli");
}

#[test]
fn parse_remote_url_handles_https() {
  let r = forge::parse_remote_url("https://github.com/kbrdn1/gwm-cli.git").unwrap();

  assert_eq!(r.host, "github.com");
  assert_eq!(r.path, "kbrdn1/gwm-cli");
}

#[test]
fn parse_remote_url_keeps_nested_gitlab_subgroups() {
  // GitLab paths are not limited to `owner/repo`: a project can live any
  // number of subgroups deep. Truncating to two segments would target the
  // wrong project silently.
  let r = forge::parse_remote_url("https://gitlab.com/group/sub/deeper/proj.git").unwrap();

  assert_eq!(r.host, "gitlab.com");
  assert_eq!(r.path, "group/sub/deeper/proj");
}

#[test]
fn parse_remote_url_handles_ssh_scheme_with_port() {
  let r = forge::parse_remote_url("ssh://git@gitlab.example.com:2222/group/proj.git").unwrap();

  assert_eq!(r.host, "gitlab.example.com");
  assert_eq!(r.path, "group/proj");
}

#[test]
fn parse_remote_url_handles_scp_syntax_on_self_hosted_host() {
  let r = forge::parse_remote_url("git@gitlab.example.com:group/sub/proj.git").unwrap();

  assert_eq!(r.host, "gitlab.example.com");
  assert_eq!(r.path, "group/sub/proj");
}

#[test]
fn parse_remote_url_strips_trailing_slash_before_dot_git() {
  let r = forge::parse_remote_url("https://github.com/kbrdn1/gwm-cli.git/").unwrap();

  assert_eq!(r.path, "kbrdn1/gwm-cli");
}

#[test]
fn parse_remote_url_rejects_a_url_without_a_path() {
  let err = forge::parse_remote_url("https://github.com/").unwrap_err();

  assert!(
    err.to_string().contains("no repository path"),
    "error should name the missing path: {}",
    err
  );
}

// --- kind detection -------------------------------------------------------

#[test]
fn detect_kind_recognises_github_dot_com() {
  assert_eq!(forge::detect_kind("github.com"), ForgeKind::GitHub);
}

#[test]
fn detect_kind_recognises_gitlab_dot_com() {
  assert_eq!(forge::detect_kind("gitlab.com"), ForgeKind::GitLab);
}

#[test]
fn detect_kind_recognises_a_self_hosted_gitlab_by_host_label() {
  // The common self-hosted convention. Anything else is undecidable from
  // the URL alone, which is exactly why `forge = "gitlab"` exists.
  assert_eq!(forge::detect_kind("gitlab.example.com"), ForgeKind::GitLab);
}

#[test]
fn detect_kind_defaults_to_github_for_an_unknown_host() {
  assert_eq!(forge::detect_kind("git.example.com"), ForgeKind::GitHub);
}

// --- resolution from the repo + config -----------------------------------

#[test]
fn resolve_reads_the_slug_and_kind_from_the_origin_remote() {
  let (_dir, repo) = init_repo();
  repo.remote("origin", "git@github.com:kbrdn1/gwm-cli.git").unwrap();

  let f = forge::resolve(&repo, &Config::default()).unwrap();

  assert_eq!(f.kind(), ForgeKind::GitHub);
  assert_eq!(f.slug(), "kbrdn1/gwm-cli");
}

#[test]
fn resolve_infers_gitlab_from_a_gitlab_dot_com_origin() {
  let (_dir, repo) = init_repo();
  repo.remote("origin", "https://gitlab.com/group/proj.git").unwrap();

  let f = forge::resolve(&repo, &Config::default()).unwrap();

  assert_eq!(f.kind(), ForgeKind::GitLab);
  assert_eq!(f.slug(), "group/proj");
}

#[test]
fn config_forge_override_wins_over_host_inference() {
  // The load-bearing case: a self-hosted GitLab on an arbitrary domain
  // cannot be detected from the remote URL, so the explicit override is
  // the only way in (issue #419, "Forge detection").
  let (_dir, repo) = init_repo();
  repo.remote("origin", "git@code.acme.internal:team/proj.git").unwrap();
  let cfg = Config {
    forge: Some(ForgeKind::GitLab),
    ..Default::default()
  };

  let f = forge::resolve(&repo, &cfg).unwrap();

  assert_eq!(f.kind(), ForgeKind::GitLab);
  assert_eq!(f.slug(), "team/proj");
}

#[test]
fn config_forge_override_can_force_github_on_a_gitlab_host() {
  let (_dir, repo) = init_repo();
  repo.remote("origin", "https://gitlab.com/group/proj.git").unwrap();
  let cfg = Config {
    forge: Some(ForgeKind::GitHub),
    ..Default::default()
  };

  let f = forge::resolve(&repo, &cfg).unwrap();

  assert_eq!(f.kind(), ForgeKind::GitHub);
}

#[test]
fn resolve_errors_without_an_origin_remote() {
  let (_dir, repo) = init_repo();

  let err = forge::resolve(&repo, &Config::default()).unwrap_err();

  assert!(
    err.to_string().contains("origin"),
    "error should name the missing remote: {}",
    err
  );
}

// --- terminology ----------------------------------------------------------

#[test]
fn pr_noun_follows_the_forge() {
  let github = forge::for_kind(
    ForgeKind::GitHub,
    forge::parse_remote_url("https://github.com/o/r").unwrap(),
  );
  let gitlab = forge::for_kind(
    ForgeKind::GitLab,
    forge::parse_remote_url("https://gitlab.com/g/p").unwrap(),
  );

  assert_eq!(github.pr_noun(), "PR");
  assert_eq!(gitlab.pr_noun(), "MR");
}

// --- URL builders ---------------------------------------------------------

#[test]
fn github_urls_use_the_issues_and_pull_paths() {
  let f = forge::for_kind(
    ForgeKind::GitHub,
    forge::parse_remote_url("https://github.com/kbrdn1/gwm-cli").unwrap(),
  );

  assert_eq!(f.issue_url(42), "https://github.com/kbrdn1/gwm-cli/issues/42");
  assert_eq!(f.pr_url(61), "https://github.com/kbrdn1/gwm-cli/pull/61");
}

#[test]
fn gitlab_urls_use_the_dash_infix_and_merge_requests_path() {
  let f = forge::for_kind(
    ForgeKind::GitLab,
    forge::parse_remote_url("https://gitlab.com/group/sub/proj").unwrap(),
  );

  assert_eq!(f.issue_url(42), "https://gitlab.com/group/sub/proj/-/issues/42");
  assert_eq!(f.pr_url(61), "https://gitlab.com/group/sub/proj/-/merge_requests/61");
}

#[test]
fn urls_honour_a_self_hosted_host() {
  // The pre-#419 free functions hardcoded `https://github.com/…`, so a
  // self-hosted instance would have produced links pointing at the wrong
  // server entirely.
  let f = forge::for_kind(
    ForgeKind::GitLab,
    forge::parse_remote_url("https://gitlab.acme.internal/team/proj").unwrap(),
  );

  assert_eq!(f.issue_url(7), "https://gitlab.acme.internal/team/proj/-/issues/7");
}

// --- config plumbing ------------------------------------------------------

#[test]
fn forge_key_parses_from_gwm_toml() {
  let cfg: Config = toml::from_str("forge = \"gitlab\"\n").unwrap();

  assert_eq!(cfg.forge, Some(ForgeKind::GitLab));
}

#[test]
fn forge_key_is_optional() {
  let cfg: Config = toml::from_str("").unwrap();

  assert_eq!(cfg.forge, None);
}

#[test]
fn forge_key_rejects_an_unknown_value() {
  let err = toml::from_str::<Config>("forge = \"bitbucket\"\n").unwrap_err();

  assert!(
    err.to_string().contains("bitbucket") || err.to_string().contains("unknown variant"),
    "error should name the bad value: {}",
    err
  );
}

// --- `gwm review` head refspec --------------------------------------------

#[test]
fn github_pr_head_refspec_uses_refs_pull() {
  let f = forge::for_kind(
    ForgeKind::GitHub,
    forge::parse_remote_url("https://github.com/o/r").unwrap(),
  );

  assert_eq!(f.pr_head_refspec(61), "pull/61/head");
}

#[test]
fn gitlab_mr_head_refspec_uses_refs_merge_requests() {
  // GitLab does not publish `refs/pull/*` at all. Reusing the GitHub
  // refspec made `gwm review` fail *after* `glab mr view` had already
  // succeeded and printed "resolving MR #61 …", which reads as a gwm bug
  // rather than an unsupported path.
  let f = forge::for_kind(
    ForgeKind::GitLab,
    forge::parse_remote_url("https://gitlab.com/g/p").unwrap(),
  );

  assert_eq!(f.pr_head_refspec(61), "merge-requests/61/head");
}

// --- web origin: scheme + port survive (Codex review #458, P2) ------------

#[test]
fn parse_remote_url_keeps_the_scheme_and_port_of_an_http_remote() {
  // A self-hosted instance on a non-default port or plain HTTP: the port
  // here IS the web port, so dropping it (and forcing https) produced a
  // dead link from `gwm open`.
  let r = forge::parse_remote_url("http://gitlab.acme:8080/g/p.git").unwrap();

  assert_eq!(r.host, "gitlab.acme");
  assert_eq!(r.web_origin, "http://gitlab.acme:8080");
  assert_eq!(r.path, "g/p");
}

#[test]
fn parse_remote_url_drops_the_ssh_port_from_the_web_origin() {
  // The opposite case, and why the port cannot be kept blindly: 2222 is
  // the SSH port, not the web port. Carrying it into an https:// URL
  // would be just as broken as dropping a real web port.
  let r = forge::parse_remote_url("ssh://git@gitlab.example.com:2222/group/proj.git").unwrap();

  assert_eq!(r.host, "gitlab.example.com");
  assert_eq!(r.web_origin, "https://gitlab.example.com");
}

#[test]
fn parse_remote_url_scp_syntax_defaults_to_https() {
  let r = forge::parse_remote_url("git@gitlab.example.com:group/proj.git").unwrap();

  assert_eq!(r.web_origin, "https://gitlab.example.com");
}

#[test]
fn urls_are_built_from_the_web_origin_not_a_rebuilt_https_host() {
  let f = forge::for_kind(
    ForgeKind::GitLab,
    forge::parse_remote_url("http://gitlab.acme:8080/g/p").unwrap(),
  );

  assert_eq!(f.issue_url(7), "http://gitlab.acme:8080/g/p/-/issues/7");
  assert_eq!(f.pr_url(9), "http://gitlab.acme:8080/g/p/-/merge_requests/9");
}

// --- Codex review #458, round 2: origin trust -----------------------------

#[test]
fn an_http_remote_yields_an_authoritative_origin() {
  let r = forge::parse_remote_url("https://gitlab.acme:8443/g/p.git").unwrap();

  assert_eq!(r.trust, forge::OriginTrust::FromUrl);
}

#[test]
fn an_ssh_remote_yields_a_guessed_origin() {
  // `https://<ssh-host>` is the best a link builder can do, but it must be
  // labelled as a guess so nothing forces it onto a forge CLI that already
  // knows better.
  for url in [
    "ssh://git@gitlab.example.com:2222/group/proj.git",
    "git@gitlab.example.com:group/proj.git",
  ] {
    let r = forge::parse_remote_url(url).unwrap();
    assert_eq!(r.trust, forge::OriginTrust::Guessed, "url {url}");
  }
}

#[test]
fn a_github_enterprise_origin_pins_gh_host() {
  // #419 made non-github.com hosts reachable for the first time (the old
  // parser rejected them outright), so `gh` must be told which instance to
  // hit — otherwise it silently targets github.com and could read a
  // same-named repo on the wrong tenant. github.com is pinned too, see
  // `gh_host_is_pinned_even_for_github_dot_com`.
  let ghe = forge::parse_remote_url("https://github.acme.internal/team/proj.git").unwrap();

  assert_eq!(
    gwm::github::gh_env(&ghe),
    vec![("GH_HOST".to_string(), "github.acme.internal".to_string())]
  );
}

#[test]
fn an_ssh_enterprise_remote_still_pins_gh_host() {
  // Supersedes the round-2 rule for GitHub only. `gh` cannot be steered
  // any other way here: `--repo owner/repo` carries no hostname,
  // `gh api repos/<slug>/…` bakes the slug into the request path, and
  // neither falls back to the working directory the way `glab` does. So
  // pinning nothing means silently querying github.com, where a
  // same-named repo owned by someone else may answer. A distinct SSH
  // hostname is a documented GitLab pattern, not a GitHub one.
  let r = forge::parse_remote_url("git@github.acme.internal:team/proj.git").unwrap();

  assert_eq!(
    gwm::github::gh_env(&r),
    vec![("GH_HOST".to_string(), "github.acme.internal".to_string())]
  );
}

#[test]
fn an_ssh_github_dot_com_remote_pins_nothing() {
  // github.com is the default anyway; pinning it from a guess adds risk
  // with no benefit.
  let r = forge::parse_remote_url("git@github.com:o/r.git").unwrap();

  assert!(gwm::github::gh_env(&r).is_empty());
}

#[test]
fn gh_host_is_pinned_even_for_github_dot_com() {
  // The child inherits gwm's environment, so a user's ambient
  // `GH_HOST=github.acme.internal` (routine for enterprise users) would
  // silently retarget a github.com repo: the argv only carries
  // `--repo owner/repo`, never a hostname. Exempting github.com left that
  // hijack open, so an authoritative origin now always states its host.
  let r = forge::parse_remote_url("https://github.com/o/r.git").unwrap();

  assert_eq!(
    gwm::github::gh_env(&r),
    vec![("GH_HOST".to_string(), "github.com".to_string())]
  );
}

#[test]
fn the_forge_child_runs_in_the_repo_not_the_process_cwd() {
  // The root cause behind the workspace tenant hazard: `gh` / `glab`
  // resolve the instance from their *working directory* when the flags do
  // not pin it. gwm's cwd is the workspace root, not the row's repo, so
  // the child is spawned in the repo instead. This covers SSH remotes,
  // where the web origin is only a guess and no host can be pinned.
  let dir = tempfile::tempdir().unwrap();
  let f = forge::for_kind_in(
    ForgeKind::GitLab,
    forge::parse_remote_url("git@gitlab.acme:team/proj.git").unwrap(),
    Some(dir.path().to_path_buf()),
  );

  assert_eq!(f.workdir(), Some(dir.path()));
}

#[test]
fn gh_host_carries_the_port_of_an_enterprise_origin() {
  // Dropping it targeted 443 — guaranteed wrong when the remote states a
  // port, and possibly a different instance listening there.
  let r = forge::parse_remote_url("https://ghe.example:8443/org/repo.git").unwrap();

  assert_eq!(
    gwm::github::gh_env(&r),
    vec![("GH_HOST".to_string(), "ghe.example:8443".to_string())]
  );
}

#[test]
fn gh_host_omits_a_default_port() {
  let r = forge::parse_remote_url("https://github.com/o/r.git").unwrap();

  assert_eq!(
    gwm::github::gh_env(&r),
    vec![("GH_HOST".to_string(), "github.com".to_string())]
  );
}

#[test]
fn an_ipv6_scp_remote_splits_on_the_bracketed_host() {
  // `split_once(':')` cut `git@[::1]:group/repo.git` into host `git@[` and
  // path `:1]:group/repo.git`, so both the URL and every CLI call pointed
  // at nonsense.
  let r = forge::parse_remote_url("git@[::1]:group/repo.git").unwrap();

  assert_eq!(r.host, "[::1]");
  assert_eq!(r.path, "group/repo");
}

#[test]
fn an_ipv6_ssh_url_keeps_its_brackets_and_drops_the_ssh_port() {
  let r = forge::parse_remote_url("ssh://git@[2001:db8::1]:2222/group/repo.git").unwrap();

  assert_eq!(r.host, "[2001:db8::1]");
  assert_eq!(r.path, "group/repo");
  assert_eq!(r.web_origin, "https://[2001:db8::1]");
}

#[test]
fn an_ipv6_https_url_keeps_its_web_port() {
  let r = forge::parse_remote_url("https://[2001:db8::1]:8443/group/repo.git").unwrap();

  assert_eq!(r.host, "[2001:db8::1]");
  assert_eq!(r.web_origin, "https://[2001:db8::1]:8443");
}

// --- canonical URLs for a guessed origin ----------------------------------

#[test]
fn an_authoritative_origin_builds_its_urls_locally() {
  // No network: the remote stated the web endpoint, so the constructed
  // URL is already the right one.
  let f = forge::for_kind(
    ForgeKind::GitLab,
    forge::parse_remote_url("https://gitlab.acme:8443/team/proj.git").unwrap(),
  );

  assert!(f.origin_is_authoritative());
  assert_eq!(f.issue_url(7), "https://gitlab.acme:8443/team/proj/-/issues/7");
}

#[test]
fn a_guessed_origin_is_flagged_so_urls_can_be_confirmed_upstream() {
  // `https://<ssh-host>` is a guess: wrong whenever the SSH hostname is
  // not the web one, or the web UI runs on HTTP / a non-standard port.
  // Callers that can afford a fetch ask the server for its own `web_url`
  // instead; the constructed value stays the offline fallback.
  let f = forge::for_kind(
    ForgeKind::GitLab,
    forge::parse_remote_url("git@gitlab-ssh.acme:team/proj.git").unwrap(),
  );

  assert!(!f.origin_is_authoritative());
  assert_eq!(f.issue_url(7), "https://gitlab-ssh.acme/team/proj/-/issues/7");
}
