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
  let github = forge::for_kind(ForgeKind::GitHub, "github.com".into(), "o/r".into());
  let gitlab = forge::for_kind(ForgeKind::GitLab, "gitlab.com".into(), "g/p".into());

  assert_eq!(github.pr_noun(), "PR");
  assert_eq!(gitlab.pr_noun(), "MR");
}

// --- URL builders ---------------------------------------------------------

#[test]
fn github_urls_use_the_issues_and_pull_paths() {
  let f = forge::for_kind(ForgeKind::GitHub, "github.com".into(), "kbrdn1/gwm-cli".into());

  assert_eq!(f.issue_url(42), "https://github.com/kbrdn1/gwm-cli/issues/42");
  assert_eq!(f.pr_url(61), "https://github.com/kbrdn1/gwm-cli/pull/61");
}

#[test]
fn gitlab_urls_use_the_dash_infix_and_merge_requests_path() {
  let f = forge::for_kind(ForgeKind::GitLab, "gitlab.com".into(), "group/sub/proj".into());

  assert_eq!(f.issue_url(42), "https://gitlab.com/group/sub/proj/-/issues/42");
  assert_eq!(f.pr_url(61), "https://gitlab.com/group/sub/proj/-/merge_requests/61");
}

#[test]
fn urls_honour_a_self_hosted_host() {
  // The pre-#419 free functions hardcoded `https://github.com/…`, so a
  // self-hosted instance would have produced links pointing at the wrong
  // server entirely.
  let f = forge::for_kind(ForgeKind::GitLab, "gitlab.acme.internal".into(), "team/proj".into());

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
  let f = forge::for_kind(ForgeKind::GitHub, "github.com".into(), "o/r".into());

  assert_eq!(f.pr_head_refspec(61), "pull/61/head");
}

#[test]
fn gitlab_mr_head_refspec_uses_refs_merge_requests() {
  // GitLab does not publish `refs/pull/*` at all. Reusing the GitHub
  // refspec made `gwm review` fail *after* `glab mr view` had already
  // succeeded and printed "resolving MR #61 …", which reads as a gwm bug
  // rather than an unsupported path.
  let f = forge::for_kind(ForgeKind::GitLab, "gitlab.com".into(), "g/p".into());

  assert_eq!(f.pr_head_refspec(61), "merge-requests/61/head");
}
