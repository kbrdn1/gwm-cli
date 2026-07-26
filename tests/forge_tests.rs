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
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Take a process-wide lock and neutralise every variable a GitLab forge
/// reads when it is constructed.
///
/// `gitlab::resolve_selector` reads `$GITLAB_SUBFOLDER`, and
/// `$CI_SERVER_URL` under auto-login, at construction time — so any test
/// asserting a GitLab selector is only deterministic once those are
/// gone. `GITLAB_SUBFOLDER=group cargo test --test forge_tests` failed
/// here reproducibly; the same class was reported against
/// `gitlab_tests.rs` and swept there in the same pass rather than one
/// test per review round (Codex review #458).
fn clean_env() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  let guard = LOCK
    .get_or_init(|| Mutex::new(()))
    .lock()
    .unwrap_or_else(|p| p.into_inner());
  // SAFETY: env mutation guarded by the lock just taken.
  unsafe {
    for v in [
      "GITLAB_SUBFOLDER",
      "CI_SERVER_URL",
      "GLAB_ENABLE_CI_AUTOLOGIN",
      "GITLAB_CI",
    ] {
      std::env::remove_var(v);
    }
  }
  guard
}

/// `forge = "github"` as a `Config`.
///
/// A host like `git.acme.internal` states no forge, and `resolve`
/// refuses to guess one rather than send an authenticated call to it
/// (Codex review #458). These tests are about link scoping, not about
/// that gate, so they name the backend the way a real user would.
fn github_cfg() -> Config {
  Config {
    forge: Some(ForgeKind::GitHub),
    ..Default::default()
  }
}

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
fn gh_host_is_not_pinned_when_the_port_cannot_be_expressed() {
  // This test used to assert the opposite, on the reasoning that
  // dropping the port targets 443 and is guaranteed wrong. The premise
  // was right and the conclusion was not: gh's `HostnameValidator`
  // rejects any hostname containing `:`
  // (`internal/ghinstance/host.go`), so `ghe.example:8443` is not a
  // value gh accepts either. Both options are wrong, which means the
  // pin is what has to go — gwm delegates and gh reads the remote from
  // the repo the child runs in (Codex review #458).
  let r = forge::parse_remote_url("https://ghe.example:8443/org/repo.git").unwrap();

  assert!(gwm::github::gh_env(&r).is_empty());
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

// --- Codex review #458, round 7 -------------------------------------------

#[test]
fn gh_host_is_still_unpinned_without_a_slug() {
  // The `gwm new` / `gwm pr` fallback for an unparseable origin: the
  // caller wants `gh` to infer everything locally.
  let (_dir, repo) = init_repo();
  let f = forge::resolve_or_default(&repo, &Config::default()).unwrap();

  assert_eq!(f.slug(), "");
}

// --- Codex review #458, round 8 -------------------------------------------

#[test]
fn githubs_alternate_ssh_endpoint_maps_back_to_the_api_host() {
  // `ssh://git@ssh.github.com:443/o/r.git` is GitHub's documented SSH
  // endpoint for networks that block port 22. The API still lives on
  // github.com, so pinning `ssh.github.com` — which round 7's
  // "pin whenever the slug is known" did — broke every call, mutations
  // included. A named alias, not a heuristic.
  let r = forge::parse_remote_url("ssh://git@ssh.github.com:443/owner/repo.git").unwrap();

  assert_eq!(r.host, "github.com");
  assert_eq!(r.web_origin, "https://github.com");
  assert_eq!(
    gwm::github::gh_env(&r),
    vec![("GH_HOST".to_string(), "github.com".to_string())]
  );
}

#[test]
fn gitlabs_alternate_ssh_endpoint_maps_back_too() {
  let r = forge::parse_remote_url("ssh://git@altssh.gitlab.com:443/group/proj.git").unwrap();

  assert_eq!(r.host, "gitlab.com");
  assert_eq!(r.web_origin, "https://gitlab.com");
}

#[test]
fn an_unknown_ssh_host_is_left_alone() {
  // Only the two documented aliases are rewritten; anything else stays
  // verbatim rather than being guessed at.
  let r = forge::parse_remote_url("git@ssh.gitlab.acme:team/proj.git").unwrap();

  assert_eq!(r.host, "ssh.gitlab.acme");
}

// --- Codex review #458, round 9: inherited targeting environment ----------

#[test]
fn the_repo_selector_env_vars_are_always_cleared() {
  // The recurring shape behind three separate P1s: the child inherits
  // gwm's environment, and every one of these overrides WHICH project the
  // CLI acts on. gwm always knows the project — either the slug, or "the
  // repo I am spawning you in" — so an inherited selector is never right.
  // Audited as a class rather than one variable per review round.
  let gh = forge::parse_remote_url("https://github.com/o/r.git").unwrap();
  assert_eq!(gwm::github::gh_env_remove(&gh, false), vec!["GH_REPO"]);

  // `GITLAB_GROUP` belongs to the same class: glab documents it as the
  // default group for listing merge requests and issues.
  let gl = forge::parse_remote_url("https://gitlab.com/g/p.git").unwrap();
  for v in ["GITLAB_REPO", "GITLAB_GROUP", "REMOTE_ALIAS", "GIT_REMOTE_URL_VAR"] {
    assert!(
      gwm::gitlab::glab_env_remove(&gl, false).contains(&v),
      "{v} must be cleared"
    );
  }
}

#[test]
fn pinning_the_host_also_closes_the_ways_around_the_pin() {
  // "Clear only what you can replace", applied to every way an
  // inherited variable can get past `GITLAB_HOST`.
  //
  // Round 9 pinned only the host. Round 10 cleared `GITLAB_API_HOST`
  // too. Round 11 reverted that, calling the variable orthogonal —
  // "gwm has nothing to put in its place" — and the finding then came
  // back five more times, because that premise is false. glab's client
  // builder reads `api_host` env-first and host-blind, lets it replace
  // the base host, and ends with `if apiHost == "" { apiHost = repoHost
  // }` (`internal/api/client.go`). The replacement is the pin.
  //
  // `GL_HOST` is the third spelling of the `host` key
  // (`internal/config/schema.go`: `GITLAB_HOST, GITLAB_URI, GL_HOST`,
  // first non-empty wins), so it rides with `GITLAB_URI`.
  let gl = forge::parse_remote_url("https://gitlab.example.com/g/p.git").unwrap();
  let removed = gwm::gitlab::glab_env_remove(&gl, false);

  assert!(
    !gwm::gitlab::glab_env(&gl).is_empty(),
    "precondition: this origin IS pinned"
  );
  for v in ["GITLAB_URI", "GL_HOST", "GITLAB_API_HOST"] {
    assert!(removed.contains(&v), "{v} must not outrank the pin: {removed:?}");
  }

  // And the bound that keeps this from becoming round 10 again: the
  // *host* spellings only lose to a value gwm actually sets, so with no
  // pin they survive. `GITLAB_API_HOST` is not one of them — its
  // replacement is glab's own `apiHost = repoHost` fallback, which does
  // not depend on the pin, so it goes either way (round 28).
  let ssh = forge::parse_remote_url("git@gitlab.example.com:g/p.git").unwrap();
  let kept = gwm::gitlab::glab_env_remove(&ssh, false);
  assert!(gwm::gitlab::glab_env(&ssh).is_empty(), "precondition: not pinned");
  for v in ["GITLAB_URI", "GL_HOST"] {
    assert!(!kept.contains(&v), "{v} is the user's only signal here: {kept:?}");
  }
  assert!(kept.contains(&"GITLAB_API_HOST"), "{kept:?}");
}

#[test]
fn authentication_and_config_location_are_never_stripped() {
  // Tier 3 of the rule, and the one that BOUNDS the other two — without
  // it "strip anything that could redirect us" eats the user's setup.
  // gwm knows the project, and sometimes the host. It never knows better
  // than the user which identity they meant to use or where they keep
  // their credentials: clearing `$GH_CONFIG_DIR` breaks a deliberately
  // relocated config, and clearing `$CI_JOB_TOKEN` breaks gwm inside a
  // GitLab pipeline, where that token is the only credential there is.
  // A pinning test, not red→green: it locks a decision in place.
  let gh = forge::parse_remote_url("https://github.com/o/r.git").unwrap();
  for v in ["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GH_CONFIG_DIR"] {
    assert!(
      !gwm::github::gh_env_remove(&gh, false).contains(&v),
      "{v} is the user's to set"
    );
  }

  // Checked on a PINNED origin, the aggressive case: this is the branch
  // that clears host variables, so it is the one that could overreach.
  let gl = forge::parse_remote_url("https://gitlab.example.com/g/p.git").unwrap();
  assert!(
    !gwm::gitlab::glab_env(&gl).is_empty(),
    "precondition: this origin IS pinned"
  );
  for v in [
    "GITLAB_TOKEN",
    "GITLAB_CLIENT_ID",
    "CI_JOB_TOKEN",
    "GLAB_ENABLE_CI_AUTOLOGIN",
    "GLAB_CONFIG_DIR",
  ] {
    assert!(
      !gwm::gitlab::glab_env_remove(&gl, false).contains(&v),
      "{v} is the user's to set"
    );
  }
}

#[test]
fn an_ssh_alias_matches_regardless_of_case() {
  // DNS is case-insensitive, so `SSH.GITHUB.COM` is a valid spelling of
  // the same host — but the alias table matched before the lowercase, so
  // it fell through and the SSH endpoint was used as the API host.
  let r = forge::parse_remote_url("git@SSH.GITHUB.COM:owner/repo.git").unwrap();

  assert_eq!(r.host, "github.com");
  assert_eq!(r.web_origin, "https://github.com");
  assert_eq!(r.trust, forge::OriginTrust::FromUrl);
}

#[test]
fn the_host_is_normalised_but_the_path_is_not() {
  // `web_origin` was built from the raw host, so a capitalised remote
  // pinned `GITLAB_HOST=https://GitLab.Example.COM` and produced links
  // to match. Repository paths stay verbatim: those ARE case-sensitive.
  let r = forge::parse_remote_url("https://GitLab.Example.COM/Group/Proj.git").unwrap();

  assert_eq!(r.host, "gitlab.example.com");
  assert_eq!(r.web_origin, "https://gitlab.example.com");
  assert_eq!(r.path, "Group/Proj");
}

#[test]
fn the_host_env_vars_are_left_alone_when_we_cannot_know_the_host() {
  // Deliberately NOT symmetrical with the selectors. gwm always knows the
  // project; it does not always know the host. On an SSH origin the
  // user's exported `GITLAB_HOST` may be the only correct signal there
  // is, so it is not cleared out from under them.
  //
  // `GITLAB_API_HOST` is deliberately absent from that list: it names
  // the API endpoint, not the host, and glab replaces it with whatever
  // host it resolved — including the one it reads off this very remote.
  let ssh = forge::parse_remote_url("git@gitlab-ssh.acme:team/proj.git").unwrap();

  assert!(gwm::gitlab::glab_env(&ssh).is_empty());
  let removed = gwm::gitlab::glab_env_remove(&ssh, false);
  for v in ["GITLAB_HOST", "GITLAB_URI"] {
    assert!(
      !removed.contains(&v),
      "{v} must survive: we have no host to put in its place"
    );
  }
}

#[test]
fn a_known_ssh_alias_is_authoritative_not_a_guess() {
  // `altssh.gitlab.com` maps to a *known* instance, so normalising it is
  // knowledge, not inference. Leaving it `Guessed` (round 8) fixed the
  // URLs but sent neither `GITLAB_HOST` nor `--repo`, so glab re-read the
  // raw remote and failed on the alternate endpoint.
  let gl = forge::parse_remote_url("ssh://git@altssh.gitlab.com:443/group/proj.git").unwrap();
  assert_eq!(gl.trust, forge::OriginTrust::FromUrl);
  assert_eq!(
    gwm::gitlab::glab_env(&gl),
    vec![
      ("GITLAB_HOST".to_string(), "https://gitlab.com".to_string()),
      ("API_PROTOCOL".to_string(), "https".to_string()),
    ]
  );

  let gh = forge::parse_remote_url("ssh://git@ssh.github.com:443/owner/repo.git").unwrap();
  assert_eq!(gh.trust, forge::OriginTrust::FromUrl);
}

#[test]
fn a_known_alias_keeps_its_explicit_repo_selector() {
  let _env = clean_env();
  let dir = tempfile::tempdir().unwrap();
  let f = forge::for_kind_in(
    ForgeKind::GitLab,
    forge::parse_remote_url("ssh://git@altssh.gitlab.com:443/group/proj.git").unwrap(),
    Some(dir.path().to_path_buf()),
  );

  assert_eq!(f.repo_selector(), "group/proj");
}

#[test]
fn a_windows_drive_path_is_not_an_scp_remote() {
  // `C:\repo` has the shape of scp syntax, so it parsed as host `c` with
  // a bogus path and network calls were aimed at an invented host
  // (Codex review #458). A local path must be refused, not guessed at.
  for url in ["C:\\repo", "C:/repo", "d:/work/thing"] {
    assert!(
      forge::parse_remote_url(url).is_err(),
      "{url} is a local path, not a remote"
    );
  }
}

#[test]
fn a_single_label_host_is_still_a_valid_scp_remote() {
  // The negative control: the drive-letter rule must not eat a real
  // one-label host, which is what a LAN or tunnelled remote looks like.
  let r = forge::parse_remote_url("git@localhost:team/proj.git").unwrap();

  assert_eq!(r.host, "localhost");
  assert_eq!(r.path, "team/proj");
}

// --- the two backends pin differently, on purpose (Codex review #458) -----

#[test]
fn a_drive_relative_windows_path_is_not_an_scp_remote() {
  // `C:repo` is a valid drive-relative Windows path and slipped past the
  // first guard, which required a `\` or `/` right after the colon
  // (Codex review #458). A one-letter *hostname* is vanishingly rare
  // next to a drive letter, so the letter alone is the signal.
  for url in ["C:repo", "d:work/thing"] {
    assert!(
      forge::parse_remote_url(url).is_err(),
      "{url} is a local path, not a remote"
    );
  }
}

// --- one pinning rule, shared by both backends (Codex review #458) --------

#[test]
fn a_guessed_origin_is_delegated_to_the_cli_on_both_backends() {
  // Supersedes the round-4/5/7 rule of pinning `$GH_HOST` from an SSH
  // hostname. The hazard those rounds closed is real — the child
  // inherits gwm's environment, so an ambient `$GH_HOST` could retarget
  // reads, label creates and milestone deletes at a same-named repo on
  // another tenant — but pinning was the wrong lever: on a GHE whose SSH
  // endpoint is not its API host, the guess is simply wrong.
  //
  // Round 16 refused to change this on the claim that `gh api` had no
  // counterpart to glab's `projects/:fullpath`. That came from a stale
  // code comment, not the docs, and it is false: gh documents `{owner}`
  // and `{repo}` as endpoint placeholders "replaced with values from the
  // repository of the current directory", and documents `$GH_HOST` as
  // applying only "where a hostname ... cannot be inferred from the
  // context of a local Git repository". Delegating therefore closes the
  // retargeting hazard instead of reopening it.
  let dir = tempfile::tempdir().unwrap();
  let ssh = forge::parse_remote_url("git@ghe-ssh.acme.com:team/proj.git").unwrap();
  assert_eq!(ssh.trust, forge::OriginTrust::Guessed);

  assert!(
    gwm::github::gh_env(&ssh).is_empty(),
    "a guessed host must not be pinned"
  );
  assert!(gwm::gitlab::glab_env(&ssh).is_empty(), "and the backends must agree");

  for kind in [ForgeKind::GitHub, ForgeKind::GitLab] {
    let f = forge::for_kind_in(kind, ssh.clone(), Some(dir.path().to_path_buf()));
    assert_eq!(
      f.repo_selector(),
      "",
      "{kind:?} still passed a slug for a guessed origin"
    );
  }
}

#[test]
fn an_authoritative_origin_is_pinned_on_both_backends() {
  // The negative control. Not pinning is for the case gwm cannot know
  // the host; an https origin names it, so it is pinned and no ambient
  // value gets a say.
  let web = forge::parse_remote_url("https://ghe.acme.com/team/proj.git").unwrap();

  assert_eq!(
    gwm::github::gh_env(&web),
    vec![("GH_HOST".to_string(), "ghe.acme.com".to_string())]
  );
  assert_eq!(
    gwm::gitlab::glab_env(&web),
    vec![
      ("GITLAB_HOST".to_string(), "https://ghe.acme.com".to_string()),
      ("API_PROTOCOL".to_string(), "https".to_string()),
    ]
  );
}

#[test]
fn an_empty_slug_uses_the_gh_api_placeholders_not_an_empty_path() {
  // The mechanism that makes delegation possible on the GitHub side, and
  // the exact thing round 16 claimed did not exist. Without it an empty
  // selector would build `repos//milestones`.
  let dir = tempfile::tempdir().unwrap();
  let ssh = forge::parse_remote_url("git@ghe-ssh.acme.com:team/proj.git").unwrap();
  let f = forge::for_kind_in(ForgeKind::GitHub, ssh, Some(dir.path().to_path_buf()));

  let argv = gwm::github::milestone_list_argv(f.repo_selector());

  assert!(
    argv.iter().any(|a| a.contains("repos/{owner}/{repo}/milestones")),
    "{argv:?}"
  );
  assert!(!argv.iter().any(|a| a.contains("repos//")), "{argv:?}");
}

#[test]
fn no_github_argv_builder_smuggles_the_slug_past_the_selector() {
  // Round 18 added `repo_selector()` returning "" for a guessed origin,
  // round 19 routed the methods through it, and round 22 found four
  // builders still pushing `--repo` unconditionally — so they emitted
  // `--repo ""`, which is worse than a wrong target: it fails outright,
  // silently killing PR detection and label sync (Codex review #458).
  //
  // Two partial sweeps in a row, so this list is EVERY builder that
  // takes a slug. A new one belongs here the day it is written.
  let dir = tempfile::tempdir().unwrap();
  let ssh = forge::parse_remote_url("git@ghe-ssh.acme.com:team/proj.git").unwrap();
  let f = forge::for_kind_in(ForgeKind::GitHub, ssh, Some(dir.path().to_path_buf()));
  let sel = f.repo_selector();
  assert_eq!(sel, "");

  let spec = gwm::labels::LabelSpec {
    name: "bug".into(),
    color: "ff0000".into(),
    description: None,
  };
  let ms = gwm::milestones::MilestoneSpec {
    title: "v1".into(),
    description: None,
    due_on: None,
    state: gwm::milestones::MilestoneState::Open,
  };
  let builders: Vec<(&str, Vec<String>)> = vec![
    ("issue_view", gwm::github::issue_view_argv(sel, 1)),
    ("pr_view", gwm::github::pr_view_argv(sel, 1)),
    ("pr_head", gwm::github::pr_head_argv(sel, 1)),
    ("find_pr", gwm::github::find_pr_argv(sel, "feat/x")),
    ("label_list", gwm::github::label_list_argv(sel)),
    ("label_create", gwm::github::label_create_argv(sel, &spec)),
    ("label_delete", gwm::github::label_delete_argv(sel, "bug")),
    ("milestone_list", gwm::github::milestone_list_argv(sel)),
    ("milestone_create", gwm::github::milestone_create_argv(sel, &ms)),
    ("milestone_update", gwm::github::milestone_update_argv(sel, 1, &ms)),
    ("milestone_delete", gwm::github::milestone_delete_argv(sel, 1)),
  ];

  for (name, argv) in builders {
    assert!(
      !argv.iter().any(|a| a == "--repo"),
      "{name} still passes --repo with an empty selector: {argv:?}"
    );
    assert!(
      !argv.iter().any(|a| a.contains("repos//") || a.contains("team/proj")),
      "{name} built a broken or slug-bearing path: {argv:?}"
    );
  }
}

#[test]
fn a_milestone_the_forge_cannot_accept_is_rejected_before_any_mutation() {
  // GitLab's `due_date` is date-only; GitHub's `due_on` is RFC 3339. The
  // check used to live inside `create_milestone` / `update_milestone`,
  // so `--dry-run` printed a plan that could not run and a real push
  // applied entries until it reached the bad one, leaving the server
  // half-updated (Codex review #458). `validate_milestone` is the seam
  // the push path now runs over the whole set first.
  let spec = |due: &str| gwm::milestones::MilestoneSpec {
    title: "v1".into(),
    description: None,
    due_on: Some(due.to_string()),
    state: gwm::milestones::MilestoneState::Open,
  };
  let bad = spec("2026-07-26T12:00:00Z");
  let good = spec("2026-07-26");

  let gl = forge::for_kind(
    ForgeKind::GitLab,
    forge::parse_remote_url("https://gitlab.com/g/p").unwrap(),
  );
  assert!(
    gl.validate_milestone(&bad).is_err(),
    "a timestamp is not a GitLab due date"
  );
  assert!(gl.validate_milestone(&good).is_ok());

  // GitHub takes RFC 3339, so the default no-op must stay a no-op.
  let gh = forge::for_kind(
    ForgeKind::GitHub,
    forge::parse_remote_url("https://github.com/o/r").unwrap(),
  );
  assert!(gh.validate_milestone(&bad).is_ok(), "GitHub accepts a timestamp");
}

#[test]
fn every_remote_alias_spelling_is_cleared() {
  // glab's config schema binds one setting, `remote_alias`, to five
  // environment names. The README documents a subset, which is how the
  // first audit came away with two of them and left three inherited
  // variables able to redirect the call — `push --prune` included
  // (Codex review #458).
  let gl = forge::parse_remote_url("https://gitlab.com/g/p.git").unwrap();
  let removed = gwm::gitlab::glab_env_remove(&gl, false);

  for v in [
    "REMOTE_ALIAS",
    "GIT_REMOTE_ALIAS",
    "REMOTE_NICKNAME",
    "GIT_REMOTE_NICKNAME",
    "GIT_REMOTE_URL_VAR",
  ] {
    assert!(removed.contains(&v), "{v} still reaches the child: {removed:?}");
  }
}

#[test]
fn a_bare_repo_still_gives_the_cli_a_git_context() {
  // `workdir()` is `None` for a bare repo, and passing that through left
  // `gh` / `glab` with no directory to resolve remotes from — so an
  // unpinned origin fell back to the CLI's default tenant. Bare plus
  // worktrees is a normal gwm layout (Codex review #458).
  let dir = tempfile::tempdir().unwrap();
  let repo = git2::Repository::init_bare(dir.path()).unwrap();
  repo.remote("origin", "git@ghe-ssh.acme.com:team/proj.git").unwrap();

  let f = forge::resolve(&repo, &github_cfg()).unwrap();

  assert!(f.workdir().is_some(), "a bare repo is still a git context");
}

#[test]
fn flipping_the_backend_drops_the_numbers_the_other_one_wrote() {
  // The origin stamp catches a change of *instance*; it cannot catch a
  // change of *backend*, because flipping `forge = "gitlab"` in
  // `.gwm.toml` leaves the remote — and therefore `<web origin>/<path>`
  // — untouched. Issue #42 then comes back as merge request !42 on the
  // other forge: a real page, the wrong one (Codex review #458).
  //
  // Reconciling at resolve time rather than at read time is what keeps
  // this cheap. `worktree::list` reads links with no `Config` in hand
  // and is the busiest reader; `forge::resolve` is the one place that
  // decides which backend a repo uses, and it already has both.
  let (_dir, repo) = init_repo();
  repo
    .remote("origin", "https://git.acme.internal/team/proj.git")
    .unwrap();
  let branch = repo.head().unwrap().shorthand().unwrap().to_string();
  gwm::github::link_issue(&repo, &branch, 42).unwrap();

  // An absent record adopts rather than purges: links written before
  // this key existed must survive the upgrade that introduces it.
  let github = forge::resolve(&repo, &github_cfg()).unwrap();
  assert_eq!(github.kind(), ForgeKind::GitHub, "precondition: host infers GitHub");
  assert_eq!(
    gwm::github::read_link(&repo, &branch).unwrap().issue,
    Some(42),
    "a first resolve must adopt the existing links, not wipe them"
  );

  // Same backend, again: idempotent.
  forge::resolve(&repo, &github_cfg()).unwrap();
  assert_eq!(gwm::github::read_link(&repo, &branch).unwrap().issue, Some(42));

  let flipped = Config {
    forge: Some(ForgeKind::GitLab),
    ..Default::default()
  };
  forge::resolve(&repo, &flipped).unwrap();
  assert_eq!(
    gwm::github::read_link(&repo, &branch).unwrap().issue,
    None,
    "a GitHub issue number is not a GitLab issue number"
  );

  // And the new backend now owns the record, so it does not re-purge
  // what it writes itself.
  gwm::github::link_issue(&repo, &branch, 7).unwrap();
  forge::resolve(&repo, &flipped).unwrap();
  assert_eq!(gwm::github::read_link(&repo, &branch).unwrap().issue, Some(7));
}

#[test]
fn the_tui_reads_the_links_after_the_reconcile_not_before() {
  // `reread_link` loaded the numbers and *then* resolved the forge, so
  // the flip's purge landed one call too late: the TUI kept serving the
  // old backend's number until the next refresh, and the open menu would
  // have sent the user to the other forge's real page for it (Codex
  // review #458). Same ordering trap as `gwm open`, one layer up.
  let (_dir, repo) = init_repo();
  repo
    .remote("origin", "https://git.acme.internal/team/proj.git")
    .unwrap();
  let branch = repo.head().unwrap().shorthand().unwrap().to_string();
  gwm::github::link_issue(&repo, &branch, 42).unwrap();

  let mut fetch = gwm::tui::state::github_fetch::GitHubFetch::new();
  // A fresh cache has no identity yet, so the first read is itself a
  // change — the caller must pair it with the spine bump.
  assert!(fetch.reread_link(&repo, Some(&branch), &github_cfg()));
  assert_eq!(fetch.link.issue, Some(42), "precondition: adopted, not purged");

  let flipped = Config {
    forge: Some(ForgeKind::GitLab),
    ..Default::default()
  };
  assert!(
    fetch.reread_link(&repo, Some(&branch), &flipped),
    "the backend moved, so the caches went and the spine must follow"
  );
  assert_eq!(
    fetch.link.issue, None,
    "the very first read after a flip must already see the purge"
  );
}

#[cfg(unix)]
#[test]
fn a_purge_that_could_not_finish_does_not_advance_the_marker() {
  // The marker is only trustworthy if it means "everything written under
  // the previous backend is gone". Advancing it after a failed removal
  // re-blesses the old numbers permanently: the mismatch never fires
  // again, and the other backend reads them as its own (Codex review
  // #458). Same invariant `stamp_link_origin` already states for the
  // origin stamp — an eager purge, or no new stamp.
  //
  // Honest about what this pins: the removals and the marker write go
  // through the same config lock, so they fail together and the marker
  // would have stayed put even without the explicit guard. This locks
  // the *observable* contract — a blocked reconcile stays pending — so
  // it holds if that coincidence ever stops holding (a partial failure
  // part-way through the branch sweep is the reachable version).
  use std::os::unix::fs::PermissionsExt;

  let (dir, repo) = init_repo();
  repo
    .remote("origin", "https://git.acme.internal/team/proj.git")
    .unwrap();
  let branch = repo.head().unwrap().shorthand().unwrap().to_string();
  gwm::github::link_issue(&repo, &branch, 42).unwrap();
  forge::resolve(&repo, &github_cfg()).unwrap();

  // A read-only `.git` blocks the config lock file, so reads still work
  // and writes do not — exactly the shape of a repo on a read-only mount.
  let gitdir = dir.path().join(".git");
  let original = std::fs::metadata(&gitdir).unwrap().permissions();
  std::fs::set_permissions(&gitdir, std::fs::Permissions::from_mode(0o555)).unwrap();

  let flipped = Config {
    forge: Some(ForgeKind::GitLab),
    ..Default::default()
  };
  forge::resolve(&repo, &flipped).unwrap();

  std::fs::set_permissions(&gitdir, original).unwrap();

  // Writable again: the flip must still be pending, not silently blessed.
  assert_eq!(
    gwm::github::read_link(&repo, &branch).unwrap().issue,
    Some(42),
    "precondition: the purge really did fail"
  );
  forge::resolve(&repo, &flipped).unwrap();
  assert_eq!(
    gwm::github::read_link(&repo, &branch).unwrap().issue,
    None,
    "the retry must still see a mismatch to act on"
  );
}

#[test]
fn gh_host_is_pinned_only_with_a_value_gh_would_accept() {
  // gh's own `HostnameValidator` rejects any hostname containing `:`
  // (`internal/ghinstance/host.go`), and `RESTPrefix` /
  // `GraphQLEndpoint` always emit `https://` for anything but the
  // hardcoded `github.localhost`. So a port cannot be expressed through
  // `GH_HOST` at all, and neither can plain http.
  //
  // Worse than a 404: `IsEnterprise` is "not github.com", so
  // `GH_HOST=github.com:443` reads as Enterprise — gh then picks
  // `GH_ENTERPRISE_TOKEN` and sends it to github.com, and hits
  // `/api/v3/` there. This was an open question in the code from round
  // 2 ("undocumented; we pass it anyway"); it is answered, and the
  // answer is no.
  //
  // Where gwm cannot express the origin it pins nothing and delegates,
  // exactly as it already does for a guessed origin: the child runs
  // inside the repo, so gh reads the remote itself.
  let pin = |url: &str| gwm::github::gh_env(&forge::parse_remote_url(url).unwrap());

  assert_eq!(
    pin("https://ghe.acme.com/team/proj.git"),
    vec![("GH_HOST".to_string(), "ghe.acme.com".to_string())]
  );
  // A default port is noise, not information — drop it and keep pinning.
  assert_eq!(
    pin("https://github.com:443/o/r.git"),
    vec![("GH_HOST".to_string(), "github.com".to_string())],
    "443 on https is the default; pinning `github.com:443` reads as Enterprise"
  );
  assert!(
    pin("https://ghe.acme.com:8443/team/proj.git").is_empty(),
    "a non-default port cannot be expressed, so delegate rather than mis-pin"
  );
  assert!(
    pin("http://ghe.acme.com/team/proj.git").is_empty(),
    "gh always builds https; pinning would silently upgrade the scheme"
  );
}

#[test]
fn the_project_selector_survives_when_gwm_supplies_no_project() {
  // Tier 1 says project selectors are always cleared "because gwm always
  // knows the project". That premise is false in exactly one place: the
  // `resolve_or_default` fallback for a repo with no `origin`, which
  // deliberately passes no slug and has no workdir either. `GH_REPO` /
  // `GITLAB_REPO` are then the user's only way to name the project, and
  // clearing them regressed `gwm new` / `gwm pr` in the very scenario
  // that fallback exists to support (Codex review #458).
  let with_slug = forge::parse_remote_url("https://github.com/o/r.git").unwrap();
  let nothing = forge::RemoteRef {
    host: "github.com".into(),
    path: String::new(),
    web_origin: "https://github.com".into(),
    trust: forge::OriginTrust::Guessed,
  };

  assert!(gwm::github::gh_env_remove(&with_slug, false).contains(&"GH_REPO"));
  assert!(
    gwm::github::gh_env_remove(&nothing, true).contains(&"GH_REPO"),
    "a workdir IS a project signal: gh infers from the repo we spawn in"
  );
  assert!(
    !gwm::github::gh_env_remove(&nothing, false).contains(&"GH_REPO"),
    "no slug and no repo to infer from: the variable is all the user has"
  );

  // Same rule on the GitLab side — round 22 was one surface swept in
  // three passes, so both backends move together here.
  let gl = forge::parse_remote_url("https://gitlab.com/g/p.git").unwrap();
  for v in ["GITLAB_REPO", "GITLAB_GROUP"] {
    assert!(gwm::gitlab::glab_env_remove(&gl, false).contains(&v), "{v}");
    assert!(!gwm::gitlab::glab_env_remove(&nothing, false).contains(&v), "{v}");
  }
}

#[test]
fn an_unrecognised_host_is_not_assumed_to_be_github() {
  // The security regression this PR introduced, and the one refused in
  // round 27 on a premise that was never checked against `dev`.
  // Pre-PR `github::repo_slug` accepted `git@github.com:` and
  // `https://github.com/` only — every other origin was rejected with
  // "is not a github URL", so gwm never made an authenticated call
  // against an arbitrary host. `detect_kind` then started defaulting
  // any unknown host to GitHub, `gh_env` pinned it as `$GH_HOST`, and
  // `gh` reads a non-github.com host as Enterprise — so cloning a
  // hostile repo and running `gwm list --detect-pr` shipped
  // `$GH_ENTERPRISE_TOKEN` to whatever the remote named.
  let (_dir, repo) = init_repo();
  repo.remote("origin", "https://evil.example/team/proj.git").unwrap();

  let err = forge::resolve(&repo, &Config::default()).unwrap_err().to_string();
  assert!(err.contains("evil.example"), "must name the host: {err}");
  assert!(err.contains("forge"), "must name the way out: {err}");

  // Naming the backend is the way in — that is what the config key is
  // for (a self-hosted instance cannot be detected from a URL).
  let named = Config {
    forge: Some(ForgeKind::GitLab),
    ..Default::default()
  };
  assert_eq!(forge::resolve(&repo, &named).unwrap().kind(), ForgeKind::GitLab);
}

#[test]
fn the_known_hosts_still_need_no_configuration() {
  // The bound on the gate: it must not turn `gwm` into a
  // configure-before-use tool for the hosts that genuinely state which
  // forge they run. That set is the vendors' own domains and nothing
  // else — a `gitlab.*` label is chosen by whoever owns the domain, so
  // it says nothing (see `a_hostname_label_is_not_a_statement_about_the_forge`).
  let (_dir, repo) = init_repo();
  for (url, want) in [
    ("https://github.com/o/r.git", ForgeKind::GitHub),
    ("git@github.com:o/r.git", ForgeKind::GitHub),
    ("https://acme.ghe.com/o/r.git", ForgeKind::GitHub),
    ("https://gitlab.com/g/p.git", ForgeKind::GitLab),
    ("git@gitlab.com:g/p.git", ForgeKind::GitLab),
  ] {
    repo.remote_delete("origin").ok();
    repo.remote("origin", url).unwrap();
    let f =
      forge::resolve(&repo, &Config::default()).unwrap_or_else(|e| panic!("{url} must resolve without config: {e}"));
    assert_eq!(f.kind(), want, "{url}");
  }
}

#[test]
fn a_flip_on_one_branch_leaves_the_other_branches_alone() {
  // The marker started repo-level and the purge swept every local
  // branch. `.gwm.toml` is a versioned file, so two worktrees of the
  // same repo legitimately carry different `forge` values — running gwm
  // in each in turn then wiped every branch's links, both ways, forever
  // (Codex review #458). Repo-wide data loss from a per-worktree
  // setting.
  //
  // Per-branch marker, and only the branch at HEAD is reconciled: that
  // is the branch whose links are about to be read or written.
  let (_dir, repo) = init_repo();
  repo.remote("origin", "https://gitlab.com/g/p.git").unwrap();
  let head = repo.head().unwrap().shorthand().unwrap().to_string();
  let commit = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("other", &commit, false).unwrap();

  gwm::github::link_issue(&repo, &head, 42).unwrap();
  gwm::github::link_issue(&repo, "other", 7).unwrap();
  forge::resolve(&repo, &Config::default()).unwrap();

  let flipped = Config {
    forge: Some(ForgeKind::GitHub),
    ..Default::default()
  };
  forge::resolve(&repo, &flipped).unwrap();

  assert_eq!(
    gwm::github::read_link(&repo, &head).unwrap().issue,
    None,
    "the branch at HEAD is the one being flipped"
  );
  assert_eq!(
    gwm::github::read_link(&repo, "other").unwrap().issue,
    Some(7),
    "another worktree's branch is not this branch's business"
  );
}

#[test]
fn links_written_before_the_marker_existed_are_github_links() {
  // An absent marker was read as "adopt whatever backend is resolving
  // now". On the first GitLab resolve after an upgrade that silently
  // re-blessed every number written by pre-#419 gwm — which only ever
  // spoke to github.com — as a GitLab iid (Codex review #458).
  //
  // Absent is not unknown: it means GitHub, because nothing else could
  // have written it.
  let (_dir, repo) = init_repo();
  repo.remote("origin", "https://gitlab.com/g/p.git").unwrap();
  let branch = repo.head().unwrap().shorthand().unwrap().to_string();
  gwm::github::link_issue(&repo, &branch, 42).unwrap();
  // Clear the marker the writer stamps, leaving the pre-#419 shape:
  // links present, no record of which backend wrote them.
  repo
    .config()
    .unwrap()
    .remove(&format!("branch.{branch}.gwm-link-forge"))
    .ok();

  forge::resolve(&repo, &Config::default()).unwrap();

  assert_eq!(
    gwm::github::read_link(&repo, &branch).unwrap().issue,
    None,
    "a GitHub issue number is not a GitLab iid"
  );
}

#[test]
fn a_hostname_label_is_not_a_statement_about_the_forge() {
  // The gate added last round accepted the `gitlab.*` label as proof.
  // An attacker picks their own hostname, so `gitlab.evil.example`
  // walked straight through it and got `$GITLAB_TOKEN` (Codex review
  // #458). Only the vendors' own domains state anything.
  let (_dir, repo) = init_repo();
  repo.remote("origin", "https://gitlab.evil.example/g/p.git").unwrap();

  assert!(forge::resolve(&repo, &Config::default()).is_err());
  assert_eq!(forge::known_kind("gitlab.acme.internal"), None);
  assert_eq!(forge::known_kind("gitlab.com"), Some(ForgeKind::GitLab));
  assert_eq!(forge::known_kind("github.com"), Some(ForgeKind::GitHub));
  assert_eq!(forge::known_kind("acme.ghe.com"), Some(ForgeKind::GitHub));

  // The convention still drives the *default* for callers that already
  // know the host is legitimate — it just cannot authorise a call.
  assert_eq!(forge::detect_kind("gitlab.acme.internal"), ForgeKind::GitLab);
}

#[test]
fn an_unpinnable_github_origin_stops_passing_a_slug_too() {
  // The pin and the selector have to move together, which is the round
  // 19/22 lesson. Round 27 made `gh_env` bail for an origin gh cannot
  // express — a non-default port, or plain http — and left
  // `repo_selector` handing over `owner/repo` anyway. `gh` then
  // resolved that slug against github.com, or against an ambient
  // `$GH_HOST`: a same-named repo on another tenant, read and pruned
  // (Codex review #458).
  let dir = tempfile::tempdir().unwrap();
  let named = Config {
    forge: Some(ForgeKind::GitHub),
    ..Default::default()
  };
  let _ = &named;
  for url in [
    "https://ghe.acme.com:8443/team/proj.git",
    "http://ghe.acme.com/team/proj.git",
  ] {
    let f = forge::for_kind_in(
      ForgeKind::GitHub,
      forge::parse_remote_url(url).unwrap(),
      Some(dir.path().to_path_buf()),
    );
    assert!(gwm::github::gh_env(&forge::parse_remote_url(url).unwrap()).is_empty());
    assert_eq!(f.repo_selector(), "", "{url} pins no host, so it passes no slug");
  }

  // github.com needs no pin — it is gh's own default instance — so the
  // slug stays.
  let f = forge::for_kind_in(
    ForgeKind::GitHub,
    forge::parse_remote_url("git@github.com:team/proj.git").unwrap(),
    None,
  );
  assert_eq!(f.repo_selector(), "team/proj");
}

#[test]
fn the_no_origin_fallback_does_not_swallow_the_host_refusal() {
  // `resolve_or_default` absorbed *every* resolve error, so the gate on
  // unrecognised hosts stopped at the read paths: `gwm new` and `gwm pr`
  // fell through to a guessed forge with an empty slug and let the CLI
  // infer the repo from the cwd — which reads the very remote that was
  // just refused, and sends the inherited credentials there (Codex
  // review #458).
  //
  // The fallback exists for a repo with *no* origin. That is the only
  // case it covers now.
  let (_dir, repo) = init_repo();
  assert!(
    forge::resolve_or_default(&repo, &Config::default()).is_ok(),
    "no origin at all is what the fallback is for"
  );

  repo.remote("origin", "https://evil.example/team/proj.git").unwrap();
  let err = forge::resolve_or_default(&repo, &Config::default())
    .unwrap_err()
    .to_string();
  assert!(err.contains("evil.example"), "{err}");
}

#[test]
fn a_link_made_without_an_origin_is_not_purged_when_one_appears() {
  // `reconcile_links` gave up when `resolve` failed, and with
  // `forge = "gitlab"` but no origin yet that meant no marker was
  // written. Adding the origin later then read the absent marker as
  // "pre-#419 GitHub links" and purged the line the user had just made
  // (Codex review #458). The config names the backend; the origin is not
  // needed to know it.
  let (_dir, repo) = init_repo();
  let branch = repo.head().unwrap().shorthand().unwrap().to_string();
  let gitlab = Config {
    forge: Some(ForgeKind::GitLab),
    ..Default::default()
  };

  forge::reconcile_links(&repo, &gitlab);
  gwm::github::link_issue(&repo, &branch, 42).unwrap();

  repo.remote("origin", "https://gitlab.com/g/p.git").unwrap();
  forge::resolve(&repo, &gitlab).unwrap();

  assert_eq!(gwm::github::read_link(&repo, &branch).unwrap().issue, Some(42));
}

#[test]
fn only_a_missing_origin_reaches_the_creation_fallback() {
  // `origin_ref(repo).is_ok()` read every failure as "no origin". A
  // malformed or non-UTF-8 origin URL therefore landed in the fallback,
  // which builds a forge with no project *and no workdir* — so `gh` had
  // no repo to infer from, `GH_REPO` was deliberately left in place
  // (nothing to replace it with), and an ambient one decided where
  // `gwm new` / `gwm pr` created the issue or PR (Codex review #458).
  //
  // "There is no origin remote" is a missing input. Anything else is a
  // URL the user meant something by, and guessing past it writes to the
  // wrong place.
  let (_dir, repo) = init_repo();
  assert!(
    forge::resolve_or_default(&repo, &Config::default()).is_ok(),
    "no origin remote at all is the fallback's one case"
  );

  repo.remote("origin", "not-a-url").unwrap();
  let err = forge::resolve_or_default(&repo, &Config::default())
    .unwrap_err()
    .to_string();
  assert!(
    !err.contains("no 'origin' remote"),
    "an unparseable URL is not a missing remote: {err}"
  );
}
