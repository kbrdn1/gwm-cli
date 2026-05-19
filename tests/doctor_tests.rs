//! `gwm doctor` checks. Each test exercises one diagnostic in isolation.

mod common;

use common::init_repo;
use gwm::config::Config;
use gwm::doctor::{self, CheckStatus, DoctorCtx, Severity};

fn ctx_for<'a>(repo: &'a git2::Repository, workdir: &'a std::path::Path, config: &'a Config) -> DoctorCtx<'a> {
  DoctorCtx {
    repo_workdir: workdir,
    repo,
    config,
  }
}

#[test]
fn fresh_repo_without_config_reports_defaults_assumed() {
  let (dir, repo) = init_repo();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let cfg = report
    .checks
    .iter()
    .find(|c| c.name.contains(".gwm.toml"))
    .expect("expected a `.gwm.toml` check in the report");

  // Missing config is not an error — defaults are perfectly usable.
  assert_eq!(cfg.status, CheckStatus::Ok);
  assert!(
    cfg.detail.to_lowercase().contains("default"),
    "missing config should mention 'defaults assumed', got: {}",
    cfg.detail
  );
}

#[test]
fn invalid_toml_marks_config_check_failed_with_severity_failed() {
  let (dir, repo) = init_repo();
  std::fs::write(dir.path().join(".gwm.toml"), "this is = not valid [toml").unwrap();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let cfg = report
    .checks
    .iter()
    .find(|c| c.name.contains(".gwm.toml"))
    .expect("expected a `.gwm.toml` check");

  assert_eq!(cfg.status, CheckStatus::Failed);
  assert_eq!(report.severity(), Severity::Failed);
  assert_eq!(report.exit_code(), 2);
}

#[test]
fn valid_toml_marks_config_check_ok() {
  let (dir, repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"[worktree]
base = "{home}/wt/{repo}"
path_pattern = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"
"#,
  )
  .unwrap();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let cfg = report
    .checks
    .iter()
    .find(|c| c.name.contains(".gwm.toml"))
    .expect("expected a `.gwm.toml` check");
  assert_eq!(cfg.status, CheckStatus::Ok);
}

// Severity/exit-code arithmetic is asserted on hand-built reports so the
// test is independent of the environment (whether `lazygit` happens to be
// on PATH, whether `~/cc-worktree/` already exists, etc.). The end-to-end
// `doctor::run` is exercised by the per-check tests above.

#[test]
fn severity_ok_when_all_checks_ok() {
  let mut report = gwm::doctor::DoctorReport::new();
  report.checks.push(gwm::doctor::Check::ok("a", "fine"));
  report.checks.push(gwm::doctor::Check::ok("b", "fine"));
  assert_eq!(report.severity(), Severity::Ok);
  assert_eq!(report.exit_code(), 0);
}

#[test]
fn severity_warning_when_any_check_warns() {
  let mut report = gwm::doctor::DoctorReport::new();
  report.checks.push(gwm::doctor::Check::ok("a", "fine"));
  report.checks.push(gwm::doctor::Check::warning("b", "meh"));
  report.checks.push(gwm::doctor::Check::ok("c", "fine"));
  assert_eq!(report.severity(), Severity::Warning);
  assert_eq!(report.exit_code(), 1);
}

#[test]
fn severity_failed_dominates_warning() {
  let mut report = gwm::doctor::DoctorReport::new();
  report.checks.push(gwm::doctor::Check::warning("a", "meh"));
  report.checks.push(gwm::doctor::Check::failed("b", "broken"));
  report.checks.push(gwm::doctor::Check::warning("c", "meh"));
  // A single Failed must lift the report to Failed regardless of how many
  // Warnings sit alongside — that's the contract the exit-code 2 relies on.
  assert_eq!(report.severity(), Severity::Failed);
  assert_eq!(report.exit_code(), 2);
}

// --------------------------------------------------------------------------
// Check #2 — guard references resolve
// --------------------------------------------------------------------------

#[test]
fn dangling_guard_reference_is_failed() {
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.bootstrap.copy.push(gwm::config::CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: false,
    guards: vec!["does-not-exist".into()],
    fallback: None,
  });

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("guard"))
    .expect("expected a guard-references check");
  assert_eq!(c.status, CheckStatus::Failed);
  assert!(c.detail.contains("does-not-exist"));
  assert_eq!(report.severity(), Severity::Failed);
}

#[test]
fn matching_guard_reference_is_ok() {
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.bootstrap.guard.push(gwm::config::Guard {
    name: "no-aws-rds".into(),
    deny_patterns: vec!["amazonaws".into()],
    on_match: "abort".into(),
    example_file: None,
  });
  config.bootstrap.copy.push(gwm::config::CopyStep {
    from: ".env".into(),
    to: ".env".into(),
    required: false,
    guards: vec!["no-aws-rds".into()],
    fallback: None,
  });

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("guard")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
}

// --------------------------------------------------------------------------
// Check #3 — `when` predicates use a supported keyword
// --------------------------------------------------------------------------

#[test]
fn unsupported_when_predicate_is_failed() {
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.bootstrap.command.push(gwm::config::CommandStep {
    name: "noop".into(),
    run: "true".into(),
    when: Some("bogus_predicate:FOO".into()),
    env: Default::default(),
  });

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("when"))
    .expect("expected a `when` predicate check");
  assert_eq!(c.status, CheckStatus::Failed);
  assert!(c.detail.contains("bogus_predicate"));
}

#[test]
fn negated_supported_keyword_is_ok() {
  // `!env_set:CI` should be accepted: the doctor must reach past the
  // leading `!` (and any other boolean operator) and validate each
  // atom against the supported-keyword list.
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.bootstrap.command.push(gwm::config::CommandStep {
    name: "skip-in-ci".into(),
    run: "./scripts/full-build.sh".into(),
    when: Some("!env_set:CI".into()),
    env: Default::default(),
  });

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("when"))
    .expect("expected a `when` predicate check");
  assert_eq!(c.status, CheckStatus::Ok);
}

#[test]
fn unsupported_keyword_on_rhs_of_and_is_failed() {
  // Compound expressions need atom-level validation. A LHS that looks
  // legitimate (`file_exists:a`) must not paper over a bogus RHS.
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.bootstrap.command.push(gwm::config::CommandStep {
    name: "compound".into(),
    run: "true".into(),
    when: Some("file_exists:a && bogus_predicate:1".into()),
    env: Default::default(),
  });

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("when"))
    .expect("expected a `when` predicate check");
  assert_eq!(c.status, CheckStatus::Failed);
  assert!(c.detail.contains("bogus_predicate"));
}

#[test]
fn file_exists_when_predicate_is_ok() {
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.bootstrap.command.push(gwm::config::CommandStep {
    name: "direnv allow".into(),
    run: "direnv allow .".into(),
    when: Some("file_exists:.envrc".into()),
    env: Default::default(),
  });

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("when")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
}

#[test]
fn no_when_predicates_is_ok() {
  let (dir, repo) = init_repo();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("when")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
}

#[test]
fn when_predicates_detail_counts_checked_predicates_not_keywords() {
  // regression: doctor detail used SUPPORTED_WHEN_PREFIXES.len() and reported
  // "1 predicate" regardless of the number of `when:` clauses actually checked.
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  // Three commands carrying a `when:` predicate. The detail message must
  // reflect the count we actually checked (3), not the count of supported
  // keywords (1, `file_exists:`). Pre-fix the impl wrote
  // `format!("{} predicate(s) recognised", SUPPORTED_WHEN_PREFIXES.len()…)`
  // which always reported 1 regardless of the number of predicates.
  for n in 0..3 {
    config.bootstrap.command.push(gwm::config::CommandStep {
      name: format!("step-{n}"),
      run: "true".into(),
      when: Some("file_exists:.envrc".into()),
      env: Default::default(),
    });
  }

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("when")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
  assert!(
    c.detail.contains("3 predicate"),
    "expected detail to mention 3 checked predicates, got: {}",
    c.detail
  );
}

#[test]
fn when_predicates_detail_says_none_when_no_predicates_configured() {
  // regression: same SUPPORTED_WHEN_PREFIXES.len() miscount as above, surfaced
  // even when zero `when:` predicates were configured.
  let (dir, repo) = init_repo();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("when")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
  // Pre-fix the impl said "1 predicate(s) recognised" even with zero
  // configured. After the fix, no predicates → detail mentions 0 or "none".
  assert!(
    !c.detail.contains("1 predicate"),
    "no predicates were configured; detail must not claim 1, got: {}",
    c.detail
  );
}

// --------------------------------------------------------------------------
// Check #4 — binaries referenced by bootstrap commands resolve on PATH
// --------------------------------------------------------------------------

#[test]
fn missing_command_binary_is_warning() {
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.bootstrap.command.push(gwm::config::CommandStep {
    name: "phantom".into(),
    run: "definitely-not-on-path-xyz123 --help".into(),
    when: None,
    env: Default::default(),
  });

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("PATH"))
    .expect("expected a PATH check");
  // A missing optional binary should not be a hard failure — the user may
  // not need that step. But it must surface as a Warning so it's visible.
  assert_eq!(c.status, CheckStatus::Warning);
  assert!(c.detail.contains("definitely-not-on-path-xyz123"));
}

#[test]
fn resolvable_command_binary_is_ok() {
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  // `sh` is on every POSIX system; CI macOS + Linux both have it.
  config.bootstrap.command.push(gwm::config::CommandStep {
    name: "noop".into(),
    run: "sh -c 'true'".into(),
    when: None,
    env: Default::default(),
  });

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
  // We don't assert Ok strictly — `lazygit` may be missing on a CI runner.
  // The relevant assertion is: when the doctor reports missing binaries, `sh`
  // is not in that list. Distinguished from the previous loose `!contains("sh ")`
  // which would pass even on `[sh,other]` or `sh\n` formatting.
  if c.status == CheckStatus::Warning {
    let missing_section = c.detail.split("not on PATH:").nth(1).unwrap_or("");
    let missing: Vec<&str> = missing_section.split([',', '\n']).map(str::trim).collect();
    assert!(
      !missing.contains(&"sh"),
      "sh must not be reported missing, got: {}",
      c.detail
    );
  }
}

#[test]
fn extract_binary_handles_shell_quoted_run_strings() {
  // regression: `extract_binary` used `split_whitespace` and returned the
  // leading-quoted token `"my` for `"my tool" --flag` before the shell-words
  // migration.
  // Pre-fix, `extract_binary` used `split_whitespace` and returned `"my`
  // as the binary name for a quoted run-string like `"my tool" --flag`,
  // producing a "binary not on PATH" warning that doesn't match anything
  // the user actually wrote. After the shell-words migration, the
  // binary is correctly identified as the full quoted command name.
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.bootstrap.command.push(gwm::config::CommandStep {
    name: "quoted".into(),
    run: r#""definitely-not-on-path-quoted-xyz" --help"#.into(),
    when: None,
    env: Default::default(),
  });

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
  assert!(
    c.detail.contains("definitely-not-on-path-quoted-xyz"),
    "shell-quoted binary name must be unquoted in the report, got: {}",
    c.detail
  );
  assert!(
    !c.detail.contains("\"definitely"),
    "the leading quote must be stripped, got: {}",
    c.detail
  );
}

// --------------------------------------------------------------------------
// Check #7 — base directory exists and is writable
// --------------------------------------------------------------------------

#[test]
fn base_dir_existing_and_writable_is_ok() {
  let (dir, repo) = init_repo();
  // Override base to a guaranteed-writable tempdir-scoped path.
  let base_dir = dir.path().join("wt-base");
  std::fs::create_dir(&base_dir).unwrap();
  let mut config = Config::default();
  config.worktree.base = base_dir.to_string_lossy().into_owned();

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("base"))
    .expect("expected a base-dir check");
  assert_eq!(c.status, CheckStatus::Ok);
}

#[test]
fn base_dir_missing_but_parent_writable_is_ok() {
  let (dir, repo) = init_repo();
  // Point at a not-yet-existing subdir of the tempdir. gwm creates the
  // worktree base on first `create`, so absence is a routine state.
  let base_dir = dir.path().join("future-base");
  let mut config = Config::default();
  config.worktree.base = base_dir.to_string_lossy().into_owned();

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("base")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
}

// --------------------------------------------------------------------------
// Check #5 — no prunable worktrees
// --------------------------------------------------------------------------

#[test]
fn fresh_repo_has_no_prunable_worktrees() {
  let (dir, repo) = init_repo();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("prunable"))
    .expect("expected a prunable check");
  assert_eq!(c.status, CheckStatus::Ok);
}

// --------------------------------------------------------------------------
// Check #6 — orphan branches matching <type>/#<issue>-<desc>
// --------------------------------------------------------------------------

#[test]
fn orphan_unmerged_gwm_branch_is_warning() {
  let (dir, repo) = init_repo();
  // Build a commit that is NOT reachable from main, then branch off it.
  // This is what an in-flight WIP branch looks like: still divergent from
  // the trunk, so leaving it around is genuine dead weight.
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  let sig = git2::Signature::now("test", "test@test").unwrap();
  let tree = head.tree().unwrap();
  let oid = repo
    .commit(None, &sig, &sig, "off-main commit", &tree, &[&head])
    .unwrap();
  let commit = repo.find_commit(oid).unwrap();
  repo.branch("feat/#99-stale-thing", &commit, false).unwrap();

  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("orphan"))
    .expect("expected an orphan-branches check");
  assert_eq!(c.status, CheckStatus::Warning);
  assert!(
    c.detail.contains("feat/#99-stale-thing"),
    "orphan branch should be quoted in the detail, got: {}",
    c.detail
  );
}

#[test]
fn merged_gwm_branch_is_not_flagged_as_orphan() {
  // CONTRIBUTING.md mandates "never delete the source branch after merge".
  // So a branch fully merged into a trunk (`dev` or `main`) is preserved
  // on purpose — flagging it would be noise on every doctor run. The
  // doctor must filter it out.
  //
  // This test exercises the *equality* short-circuit: the branch tip is
  // the same commit as main's tip. See
  // `merged_via_merge_commit_gwm_branch_is_not_flagged_as_orphan` for the
  // descendant-of case, which is what every real "merge commit" flow
  // produces.
  let (dir, repo) = init_repo();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("feat/#99-already-merged", &head, false).unwrap();

  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("orphan")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
  assert!(
    !c.detail.contains("feat/#99-already-merged"),
    "merged branch must not appear in the orphan list, got: {}",
    c.detail
  );
}

#[test]
fn merged_via_merge_commit_gwm_branch_is_not_flagged_as_orphan() {
  // The realistic case: a feature branch had its own commit, then a
  // merge commit on `main` joined it back. After that, `main`'s tip is
  // a descendant of the feature tip, but they're NOT equal. The
  // equality short-circuit alone would miss this; the descendant check
  // (`graph_descendant_of`) is what catches it.
  let (dir, repo) = init_repo();
  let main_initial = repo.head().unwrap().peel_to_commit().unwrap();
  let sig = git2::Signature::now("test", "test@test").unwrap();
  let tree = main_initial.tree().unwrap();

  // Feature branch with its own commit, not on main yet.
  let feature_oid = repo
    .commit(None, &sig, &sig, "feature work", &tree, &[&main_initial])
    .unwrap();
  let feature_commit = repo.find_commit(feature_oid).unwrap();
  repo
    .branch("feat/#88-merged-via-merge", &feature_commit, false)
    .unwrap();

  // Merge commit on main combining the initial commit and the feature.
  // Main now points at a commit that has the feature tip as one of its
  // parents — `graph_descendant_of(main_tip, feature_tip) == true`,
  // but `main_tip != feature_tip`.
  repo
    .commit(
      Some("refs/heads/main"),
      &sig,
      &sig,
      "merge feat/#88",
      &tree,
      &[&main_initial, &feature_commit],
    )
    .unwrap();

  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("orphan")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
  assert!(
    !c.detail.contains("feat/#88-merged-via-merge"),
    "branch merged via a merge commit must not appear in the orphan list, got: {}",
    c.detail
  );
}

#[test]
fn non_gwm_branch_is_not_flagged_as_orphan() {
  let (dir, repo) = init_repo();
  // Branches that don't match the <type>/#<issue>-<desc> shape are user-
  // managed (release branches, dependabot bumps, etc.) and must be left alone.
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("release-2.0", &head, false).unwrap();
  repo.branch("dependabot/cargo/serde-1.0.200", &head, false).unwrap();

  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("orphan")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
}

#[test]
fn orphan_check_honours_configured_trunks() {
  // Repos with non-standard trunk conventions (`master`, `release-3.x`,
  // …) must be able to opt in via `[doctor].trunks`. Pre-#59 the trunk
  // list was hardcoded to `["dev", "main"]` and `[doctor].trunks` was
  // silently ignored, so any repo with a different trunk saw every
  // merged gwm-style branch flagged as "unmerged orphan".
  let (dir, repo) = init_repo();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  let sig = git2::Signature::now("test", "test@test").unwrap();
  let tree = head.tree().unwrap();

  // Divergent commit off main's HEAD. This is what an in-flight feature
  // branch looks like before merge.
  let feature_oid = repo.commit(None, &sig, &sig, "feature work", &tree, &[&head]).unwrap();
  let feature_commit = repo.find_commit(feature_oid).unwrap();
  repo.branch("feat/#77-on-custom-trunk", &feature_commit, false).unwrap();

  // `custom-trunk` carries the feature work — i.e. the gwm branch is
  // fully merged into the configured trunk but NOT into `main`.
  repo.branch("custom-trunk", &feature_commit, false).unwrap();

  let mut config = Config::default();
  config.doctor.trunks = vec!["custom-trunk".into()];

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("orphan"))
    .expect("expected an orphan-branches check");
  assert_eq!(c.status, CheckStatus::Ok);
  assert!(
    !c.detail.contains("feat/#77-on-custom-trunk"),
    "merged branch must not appear in the orphan list when its trunk is configured, got: {}",
    c.detail
  );
}

#[test]
fn orphan_check_with_empty_trunks_disables_merge_filter() {
  // `trunks = []` is the documented escape hatch: report every unclaimed
  // gwm-style branch, regardless of whether it's merged. Pre-#59 the
  // empty config silently fell back to the hardcoded `["dev", "main"]`
  // because the value lived in a `const`. Confirms the config value is
  // actually wired through.
  let (dir, repo) = init_repo();
  let head = repo.head().unwrap().peel_to_commit().unwrap();

  // A gwm-style branch pointing at main's tip. With the default config
  // this would be filtered out (equality short-circuit, merged into main).
  repo.branch("feat/#88-merged-into-main", &head, false).unwrap();

  let mut config = Config::default();
  config.doctor.trunks = vec![];

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("orphan"))
    .expect("expected an orphan-branches check");
  assert_eq!(c.status, CheckStatus::Warning);
  assert!(
    c.detail.contains("feat/#88-merged-into-main"),
    "with no configured trunks every gwm branch must surface as orphan, got: {}",
    c.detail
  );
}

#[test]
fn orphan_check_ignores_configured_trunks_that_do_not_exist() {
  // A trunk listed in config but absent from the repo must not crash
  // the check — doctor should silently skip the missing trunk and use
  // the rest of the list. Matches the existing tolerance for "no `dev`
  // branch" in a fresh `gwm init` repo.
  let (dir, repo) = init_repo();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("feat/#99-merged-into-main", &head, false).unwrap();

  let mut config = Config::default();
  // `phantom-trunk` doesn't exist; `main` does and reaches the gwm branch.
  config.doctor.trunks = vec!["phantom-trunk".into(), "main".into()];

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("orphan")).unwrap();
  assert_eq!(c.status, CheckStatus::Ok);
}
