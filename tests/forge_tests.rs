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

// --- Codex review #458, round 7 -------------------------------------------

#[test]
fn gh_host_is_still_unpinned_without_a_slug() {
  // The `gwm new` / `gwm pr` fallback for an unparseable origin: the
  // caller wants `gh` to infer everything locally.
  let (_dir, repo) = init_repo();
  let f = forge::resolve_or_default(&repo, &Config::default());

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
  assert_eq!(gwm::github::gh_env_remove(&gh), vec!["GH_REPO"]);

  // `GITLAB_GROUP` belongs to the same class: glab documents it as the
  // default group for listing merge requests and issues.
  let gl = forge::parse_remote_url("https://gitlab.com/g/p.git").unwrap();
  for v in ["GITLAB_REPO", "GITLAB_GROUP", "REMOTE_ALIAS", "GIT_REMOTE_URL_VAR"] {
    assert!(gwm::gitlab::glab_env_remove(&gl).contains(&v), "{v} must be cleared");
  }
}

#[test]
fn pinning_the_host_also_closes_the_ways_around_the_pin() {
  // "Clear only what you can replace" — and the two variables that look
  // alike under it but are not. `GITLAB_URI` is a documented ALIAS of
  // the variable gwm is setting, so an inherited one is pure ambiguity
  // and clearing it loses nothing. `GITLAB_API_HOST` is ORTHOGONAL: it
  // names the API endpoint on instances that split Git and API across
  // hostnames, which is exactly what a Git remote URL cannot reveal.
  // Round 9 pinned only `GITLAB_HOST`, round 10 cleared both, round 11
  // caught that the second hardened nothing and broke the only setups
  // that need it.
  let gl = forge::parse_remote_url("https://gitlab.example.com/g/p.git").unwrap();
  let removed = gwm::gitlab::glab_env_remove(&gl);

  assert!(
    !gwm::gitlab::glab_env(&gl).is_empty(),
    "precondition: this origin IS pinned"
  );
  assert!(
    removed.contains(&"GITLAB_URI"),
    "an alias of the var we pin must not outrank it"
  );
  assert!(
    !removed.contains(&"GITLAB_API_HOST"),
    "gwm has no API host to put in its place: clearing it only breaks split-host installs"
  );
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
      !gwm::github::gh_env_remove(&gh).contains(&v),
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
      !gwm::gitlab::glab_env_remove(&gl).contains(&v),
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
  let ssh = forge::parse_remote_url("git@gitlab-ssh.acme:team/proj.git").unwrap();

  assert!(gwm::gitlab::glab_env(&ssh).is_empty());
  let removed = gwm::gitlab::glab_env_remove(&ssh);
  for v in ["GITLAB_HOST", "GITLAB_URI", "GITLAB_API_HOST"] {
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
    vec![("GITLAB_HOST".to_string(), "https://gitlab.com".to_string())]
  );

  let gh = forge::parse_remote_url("ssh://git@ssh.github.com:443/owner/repo.git").unwrap();
  assert_eq!(gh.trust, forge::OriginTrust::FromUrl);
}

#[test]
fn a_known_alias_keeps_its_explicit_repo_selector() {
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
    vec![("GITLAB_HOST".to_string(), "https://ghe.acme.com".to_string())]
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
fn no_github_call_smuggles_the_slug_past_the_selector() {
  // Round 18 added `repo_selector()` returning "" for a guessed origin
  // and then every method kept reading `self.origin.path` directly, so
  // the policy was inert — the same shape as the `create_pr` bypass
  // found in round 7. Asserting on the built argv rather than on the
  // accessor is what makes that unrepeatable.
  let dir = tempfile::tempdir().unwrap();
  let ssh = forge::parse_remote_url("git@ghe-ssh.acme.com:team/proj.git").unwrap();
  let slug = "team/proj";

  let f = forge::for_kind_in(ForgeKind::GitHub, ssh, Some(dir.path().to_path_buf()));
  assert_eq!(f.repo_selector(), "");

  // Every argv builder the backend routes through, fed the selector the
  // backend actually exposes.
  let argvs = vec![
    gwm::github::issue_view_argv(f.repo_selector(), 1),
    gwm::github::pr_view_argv(f.repo_selector(), 1),
    gwm::github::pr_head_argv(f.repo_selector(), 1),
    gwm::github::milestone_list_argv(f.repo_selector()),
  ];
  for argv in argvs {
    assert!(
      !argv.iter().any(|a| a.contains(slug)),
      "the slug reached the argv despite an empty selector: {argv:?}"
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
