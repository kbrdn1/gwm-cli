//! `gwm doctor` checks. Each test exercises one diagnostic in isolation.

mod common;

use common::init_repo;
use gwm::config::Config;
use gwm::doctor::{self, CheckStatus, DoctorCtx, Severity};
use std::sync::{Mutex, OnceLock};

/// Serialise the env-mutating tests in this binary (only the forge-CLI
/// override probe today), since `set_var` is process-wide.
fn env_lock() -> &'static Mutex<()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  LOCK.get_or_init(|| Mutex::new(()))
}

fn ctx_for<'a>(repo: &'a git2::Repository, workdir: &'a std::path::Path, config: &'a Config) -> DoctorCtx<'a> {
  DoctorCtx {
    repo_workdir: workdir,
    repo,
    config,
    // Isolated by default: no global layer is read for injected test contexts.
    global_config_path: None,
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

#[test]
fn semantically_invalid_profile_marks_config_check_failed() {
  // #324 review (P2): a profile that parses but is semantically invalid
  // (`dirs = [".."]` escapes the worktree) must fail the `.gwm.toml` check,
  // not be reported green — doctor mirrors what the loader/commands reject.
  let (dir, repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[clean.profiles.default]\ndirs = [\"..\"]\n",
  )
  .unwrap();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let cfg = report
    .checks
    .iter()
    .find(|c| c.name.contains(".gwm.toml"))
    .expect("expected a `.gwm.toml` check");
  assert_eq!(cfg.status, CheckStatus::Failed);
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
// Checks #3 and #4 also see `[hooks.*]`, not just `[[bootstrap.command]]`
// --------------------------------------------------------------------------

/// Every lifecycle phase that carries commands. `[hooks.*]` steps have the
/// same `run` / `when` shape as `[[bootstrap.command]]`, so a check that
/// inspects one surface and not the other reports on half the config. The
/// list is the whole of `LifecycleHooksConfig`; the loops below run each
/// case against all six so a phase cannot be covered by accident.
const HOOK_PHASES: &[&str] = &[
  "pre_create",
  "post_create",
  "pre_bootstrap",
  "post_bootstrap",
  "pre_remove",
  "post_remove",
];

/// Load a `.gwm.toml` written into a fresh repo, so the test exercises the
/// real `[hooks.<phase>]` key names rather than field access.
fn config_from_toml(dir: &std::path::Path, body: &str) -> Config {
  std::fs::write(dir.join(".gwm.toml"), body).unwrap();
  Config::load_layered(dir, None).expect("test config must load")
}

#[test]
fn unsupported_when_predicate_on_a_hook_is_failed() {
  // Pre-fix the `when` check walked `[[bootstrap.command]]` only, so a
  // typo in a hook's predicate was reported as "no `when:` predicates
  // configured": the check passed vacuously on a config it never read.
  for phase in HOOK_PHASES {
    let (dir, repo) = init_repo();
    let config = config_from_toml(
      dir.path(),
      &format!("[[hooks.{phase}]]\nname = \"noop\"\nrun = \"true\"\nwhen = \"bogus_predicate:FOO\"\n"),
    );

    let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
    let c = report
      .checks
      .iter()
      .find(|c| c.name.contains("when"))
      .expect("expected a `when` predicate check");
    assert_eq!(
      c.status,
      CheckStatus::Failed,
      "phase {phase} went unchecked: {}",
      c.detail
    );
    assert!(
      c.detail.contains("bogus_predicate"),
      "phase {phase} must name the offending atom, got: {}",
      c.detail
    );
    assert!(
      c.detail.contains(phase),
      "phase {phase} must be named so the user knows where to look, got: {}",
      c.detail
    );
  }
}

#[test]
fn supported_when_predicate_on_a_hook_counts_as_recognised() {
  // The other half of the vacuous pass: a hook with a *valid* predicate was
  // reported as "no `when:` predicates configured", which reads as "nothing
  // to check here" on a config that has plenty.
  for phase in HOOK_PHASES {
    let (dir, repo) = init_repo();
    let config = config_from_toml(
      dir.path(),
      &format!("[[hooks.{phase}]]\nname = \"noop\"\nrun = \"true\"\nwhen = \"file_exists:composer.json\"\n"),
    );

    let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
    let c = report.checks.iter().find(|c| c.name.contains("when")).unwrap();
    assert_eq!(c.status, CheckStatus::Ok);
    assert!(
      c.detail.contains("1 predicate"),
      "phase {phase} predicate went uncounted, got: {}",
      c.detail
    );
  }
}

#[test]
fn missing_hook_binary_is_warning() {
  // Same blind spot on the PATH check: a hook invoking a binary that is not
  // installed produced a clean report, so `gwm doctor` said the config was
  // fine right up to the moment the hook failed at `gwm create` time.
  for phase in HOOK_PHASES {
    let (dir, repo) = init_repo();
    let config = config_from_toml(
      dir.path(),
      &format!("[[hooks.{phase}]]\nname = \"phantom\"\nrun = \"definitely-not-on-path-xyz123 --help\"\n"),
    );

    let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
    let c = report
      .checks
      .iter()
      .find(|c| c.name.contains("PATH"))
      .expect("expected a PATH check");
    assert_eq!(c.status, CheckStatus::Warning, "phase {phase}: {}", c.detail);
    assert!(
      c.detail.contains("definitely-not-on-path-xyz123"),
      "phase {phase} must name the missing binary, got: {}",
      c.detail
    );
  }
}

#[test]
fn a_step_its_predicate_switches_off_is_not_probed() {
  // The `node` preset ships two mutually exclusive install hooks: `bun
  // install` when bun is on PATH, `npm ci` when it is not. Probing a step's
  // binary regardless of its `when` warns about whichever of the two the
  // predicate has just switched off, and a Warning takes `gwm doctor` to exit
  // code 1, so a built-in preset that works perfectly reports as not green.
  //
  // Both surfaces, because both feed the same probe. The bootstrap-command
  // half predates the hooks fix and was wrong the same way, it just had no
  // preset with mutually exclusive steps to make it visible.
  let off = "cmd_exists:definitely-not-a-command-xyz123";
  let mut bodies: Vec<String> = vec![format!(
    "[[bootstrap.command]]\nname = \"off\"\nrun = \"definitely-not-on-path-xyz123 --help\"\nwhen = \"{off}\"\n"
  )];
  bodies.extend(HOOK_PHASES.iter().map(|phase| {
    format!("[[hooks.{phase}]]\nname = \"off\"\nrun = \"definitely-not-on-path-xyz123 --help\"\nwhen = \"{off}\"\n")
  }));

  for body in &bodies {
    let (dir, repo) = init_repo();
    let config = config_from_toml(dir.path(), body);
    let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
    let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
    assert!(
      !c.detail.contains("definitely-not-on-path-xyz123"),
      "a step gated off by its predicate must not be probed, got: {}\nconfig: {}",
      c.detail,
      body
    );
  }
}

#[test]
fn a_step_its_predicate_switches_on_is_still_probed() {
  // The other polarity, so the fix above cannot be "stop probing anything
  // that carries a `when`". `.gwm.toml` is written by `config_from_toml`, so
  // `file_exists:.gwm.toml` is true by construction.
  let on = "file_exists:.gwm.toml";
  let mut bodies: Vec<String> = vec![format!(
    "[[bootstrap.command]]\nname = \"on\"\nrun = \"definitely-not-on-path-xyz123 --help\"\nwhen = \"{on}\"\n"
  )];
  bodies.extend(HOOK_PHASES.iter().map(|phase| {
    format!("[[hooks.{phase}]]\nname = \"on\"\nrun = \"definitely-not-on-path-xyz123 --help\"\nwhen = \"{on}\"\n")
  }));

  for body in &bodies {
    let (dir, repo) = init_repo();
    let config = config_from_toml(dir.path(), body);
    let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
    let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
    assert_eq!(c.status, CheckStatus::Warning, "config: {}\n{}", body, c.detail);
    assert!(
      c.detail.contains("definitely-not-on-path-xyz123"),
      "a step its predicate switches on must still be probed, got: {}\nconfig: {}",
      c.detail,
      body
    );
  }
}

#[test]
fn doctor_declines_to_evaluate_a_predicate_it_cannot_bound() {
  // `gwm doctor` reads a `.gwm.toml` that never went through the trust gate
  // (#473), so evaluating one of its predicates means evaluating input from a
  // repo nobody has vetted, on a command that also runs as an advisory CI job
  // on every PR. `glob_exists:` walks the filesystem from wherever its pattern
  // points, so `glob_exists:/**/nope` is a whole-disk walk; `file_exists:../..`
  // reaches outside the repo. Neither gets evaluated.
  //
  // Declining is safe by construction: the step stays probed, which is what
  // the check did before it evaluated anything, so it can never silence a
  // warning. The three patterns below are bounded and match nothing, so an
  // implementation that DID evaluate them would gate the step off and drop the
  // binary from the report. That difference is what the assertion rides on,
  // and it keeps the test instant instead of walking a real disk.
  for when in [
    "glob_exists:definitely-nope-xyz123-*",
    "file_exists:../definitely-nope-xyz123",
    // One unbounded atom taints the whole expression, however sound the rest.
    "cmd_exists:sh && glob_exists:definitely-nope-xyz123-*",
  ] {
    let (dir, repo) = init_repo();
    let config = config_from_toml(
      dir.path(),
      &format!(
        "[[hooks.post_create]]\nname = \"probe\"\nrun = \"definitely-not-on-path-xyz123 --help\"\nwhen = \"{when}\"\n"
      ),
    );

    let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
    let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
    assert!(
      c.detail.contains("definitely-not-on-path-xyz123"),
      "`{when}` must not be evaluated, so its step stays probed; got: {}",
      c.detail
    );
  }
}

#[test]
fn a_step_whose_binary_cannot_be_resolved_statically_is_not_probed() {
  // Two ways the probed string is not the binary the step will launch, both
  // ending in a Warning and exit code 1 on a step that works fine:
  //
  //   - `lifecycle::run_step` expands `{path}` / `{repo}` in `run` before
  //     spawning, so the raw string names `{path}/scripts/setup` and `which`
  //     would look that up literally;
  //   - a step that sets its own `PATH` in `env` resolves against that, since
  //     `run_step` hands it to `Command::env`, not against the ambient `$PATH`
  //     `gwm doctor` happens to have.
  //
  // Not probing is the safe side here, the mirror of the predicate rule above:
  // there, declining to evaluate leaves the step probed; here, declining to
  // resolve leaves it unprobed. Both refuse to emit an answer we know is wrong.
  for body in [
    "[[hooks.post_create]]\nname = \"templated\"\nrun = \"{path}/definitely-not-on-path-xyz123\"\n",
    "[[hooks.post_create]]\nname = \"own-path\"\nrun = \"definitely-not-on-path-xyz123 --help\"\nenv = { PATH = \"/opt/project/bin\" }\n",
  ] {
    let (dir, repo) = init_repo();
    let config = config_from_toml(dir.path(), body);
    let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
    let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
    assert!(
      !c.detail.contains("definitely-not-on-path-xyz123"),
      "a step whose binary cannot be resolved statically must not be probed, got: {}\nconfig: {}",
      c.detail,
      body
    );
  }
}

#[test]
fn a_shell_keyword_is_not_probed_as_a_binary() {
  // `run` is a shell script handed whole to `sh -c`, not an argv, so its
  // first token is a shell word at least as often as it is a program. `cd`,
  // `export`, `set` and `if` have no binary on disk to find, so probing them
  // warns about a hook that works and takes the exit code to 1. This got
  // sharper when the probe started walking hooks: a hook is a script far more
  // often than a bootstrap command is.
  for run in [
    "cd sub && ./setup.sh",
    "export APP_ENV=dev; ./setup.sh",
    "set -e; ./setup.sh",
    "if [ -f composer.json ]; then ./setup.sh; fi",
    "while [ ! -f ready ]; do ./wait.sh; done",
  ] {
    let (dir, repo) = init_repo();
    let config = config_from_toml(
      dir.path(),
      &format!("[[hooks.post_create]]\nname = \"scripted\"\nrun = \"{run}\"\n"),
    );

    let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
    let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
    let missing: Vec<String> = c
      .detail
      .split("not on PATH:")
      .nth(1)
      .unwrap_or("")
      .split([',', '\n'])
      .map(|s| s.trim().to_string())
      .collect();
    let keyword = run.split_whitespace().next().unwrap();
    assert!(
      !missing.iter().any(|m| m == keyword),
      "`{keyword}` is a shell keyword, not a binary to probe; got: {}",
      c.detail
    );
  }
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
fn missing_review_binary_is_warning_not_failure() {
  // Issue #75: [review] is opt-in. A missing review binary should
  // surface as Warning (exit code 1), never Failed (exit code 2),
  // so CI / pre-commit hooks that already gate on doctor still pass
  // when the user only set [review] for their own local convenience.
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.review.command = Some("definitely-not-on-path-review-xyz {base}..{head}".into());

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.contains("PATH"))
    .expect("expected a PATH check");
  assert_eq!(c.status, CheckStatus::Warning);
  assert!(
    c.detail.contains("definitely-not-on-path-review-xyz"),
    "missing review binary must appear in the detail: {}",
    c.detail
  );
}

#[test]
fn missing_review_tool_preset_is_warning() {
  // `tool = "lumen"` resolves to `lumen diff ...`. If the user names a
  // preset but `lumen` itself isn't installed, the doctor warns about
  // the binary name from the preset table, not about the literal
  // string `"lumen"` token in their config.
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.review.tool = Some("lumen".into());

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
  // Lumen is almost certainly not on the CI matrix. If it happens to
  // be installed locally we don't have anything to assert; the loose
  // contract is "if it's missing, the report names it".
  if c.status == CheckStatus::Warning && c.detail.contains("lumen") {
    assert!(
      c.detail.to_lowercase().contains("lumen"),
      "preset's resolved binary must be named in the warning: {}",
      c.detail
    );
  }
}

#[test]
fn launcher_wrapped_by_env_warns_on_real_binary_not_wrapper() {
  // PR #76 Copilot review: `extract_binary` returned the first token
  // that didn't contain '=', which for `env FOO=bar phantom-bin diff`
  // was `env` itself. The doctor then checked `env` (always on PATH)
  // and missed the real launcher binary. Confirm that the doctor now
  // names the actual tool when an `env` wrapper is present.
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.review.command = Some("env FOO=bar definitely-not-on-path-wrapped-zz {base}..{head}".into());

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
  assert_eq!(c.status, CheckStatus::Warning, "wrapped missing binary must warn");
  assert!(
    c.detail.contains("definitely-not-on-path-wrapped-zz"),
    "wrapper must be peeled: detail should name the real binary, got: {}",
    c.detail
  );
  assert!(
    !c.detail.split("not on PATH:").nth(1).unwrap_or("").contains("env"),
    "`env` must not appear in the missing-binaries section: {}",
    c.detail
  );
}

#[test]
fn launcher_wrapped_by_command_warns_on_real_binary_not_wrapper() {
  // Sibling case for the POSIX `command` builtin: `command tool ...`
  // (used e.g. inside shell aliases to bypass functions) should peel
  // the wrapper just like `env`. Same shape as the env test above.
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.git_tui.command = Some("command definitely-not-on-path-cmd-yy -d {path}".into());

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
  assert!(
    c.detail.contains("definitely-not-on-path-cmd-yy"),
    "command wrapper must be peeled: detail should name the real binary, got: {}",
    c.detail
  );
}

#[test]
fn missing_git_tui_binary_is_warning() {
  // A user overriding [git_tui] to a missing binary deserves the same
  // visibility treatment as a missing review tool — gwm's `l` keybinding
  // would surface a status-bar error at runtime, but doctor should
  // catch it upfront.
  let (dir, repo) = init_repo();
  let mut config = Config::default();
  config.git_tui.command = Some("definitely-not-on-path-tui-xyz -d {path}".into());

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
  assert_eq!(c.status, CheckStatus::Warning);
  assert!(
    c.detail.contains("definitely-not-on-path-tui-xyz"),
    "missing git_tui binary must appear in the detail: {}",
    c.detail
  );
}

#[test]
fn review_unset_does_not_force_lazygit_warning_to_failure() {
  // Pre-issue-#75 the doctor flagged a missing `lazygit` as Warning.
  // Confirm that adding the new launcher checks doesn't tip the
  // severity over to Failed when only [review] is unconfigured.
  let (dir, repo) = init_repo();
  let config = Config::default(); // nothing review-related set
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  // The aggregate severity must stay at Warning or Ok — never Failed.
  assert!(
    matches!(report.severity(), CheckStatus::Ok | CheckStatus::Warning),
    "default config must not push doctor into Failed: severity = {:?}",
    report.severity()
  );
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

// --- TUI keymap check (issue #87) ---------------------------------------

#[test]
fn doctor_passes_with_default_keymap() {
  // No `[tui.keys]` overrides → the resolved keymap is the built-in
  // default. Every required action is bound, so the check is green.
  let (dir, repo) = init_repo();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.to_lowercase().contains("keymap"))
    .expect("expected a TUI keymap check in the report");
  assert_eq!(
    c.status,
    CheckStatus::Ok,
    "default keymap must pass cleanly, got: {} — {}",
    match c.status {
      CheckStatus::Ok => "ok",
      CheckStatus::Warning => "warning",
      CheckStatus::Failed => "failed",
    },
    c.detail
  );
}

#[test]
fn doctor_warns_when_user_unbinds_quit() {
  // `quit` is the only action with a hard-coded escape hatch
  // (`Ctrl+C` in `run_app`). Even so, leaving it without any other
  // binding is hostile UX: warn the user so they can either restore
  // a binding or acknowledge the choice.
  let (dir, repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[tui.keys]
quit = []
"#,
  )
  .unwrap();
  let config = Config::load_layered(dir.path(), None).unwrap();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.to_lowercase().contains("keymap"))
    .expect("expected a TUI keymap check in the report");
  assert_eq!(c.status, CheckStatus::Warning);
  assert!(
    c.detail.to_lowercase().contains("quit"),
    "expected message to name the missing action, got: {}",
    c.detail
  );
  assert!(
    c.detail.to_lowercase().contains("ctrl"),
    "expected message to mention the Ctrl+C fallback, got: {}",
    c.detail
  );
}

#[test]
fn doctor_reports_modal_binding_count_on_default_keymap() {
  // Issue #219: the keymap check now also resolves the contextual modal
  // keymap and folds its bound count into the detail line.
  let (dir, repo) = init_repo();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.to_lowercase().contains("keymap"))
    .expect("expected a TUI keymap check");
  assert_eq!(c.status, CheckStatus::Ok);
  assert!(
    c.detail.contains("modal"),
    "detail must mention the modal binding count, got: {}",
    c.detail
  );
}

#[test]
fn doctor_fails_on_in_context_modal_conflict() {
  // A `[tui.keys.modal.confirm]` block binding two verbs to the same key is a
  // per-context conflict. Written to disk (load-time validation rejects it, so
  // the lenient doctor context defaults ctx.config away) to prove `gwm doctor`
  // re-reads the file and independently catches it.
  let (dir, repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[tui.keys.modal.confirm]\nconfirm = [\"x\"]\ncancel = [\"x\"]\n",
  )
  .unwrap();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.to_lowercase().contains("keymap"))
    .expect("expected a TUI keymap check");
  assert_eq!(c.status, CheckStatus::Failed);
  assert!(
    c.detail.contains("conflict"),
    "detail must explain the conflict, got: {}",
    c.detail
  );
}

#[test]
fn doctor_keymap_check_reads_only_the_threaded_global_layer() {
  // #219 review (P2): the keymap check must merge the global layer that was
  // *threaded into the context*, never an ambient `global_config_path()` read —
  // otherwise a developer's real `~/.config/gwm` would make an isolated temp
  // repo's check flap. A bad global is seen only when explicitly threaded.
  let (dir, repo) = init_repo();
  let global = dir.path().join("global.toml");
  std::fs::write(&global, "[tui.keys.modal.confirm]\nconfirm = [\"g g\"]\n").unwrap();
  let config = Config::default();

  // Threaded → the bad global keymap surfaces.
  let with = DoctorCtx {
    repo_workdir: dir.path(),
    repo: &repo,
    config: &config,
    global_config_path: Some(global.as_path()),
  };
  let c = doctor::run(&with)
    .unwrap()
    .checks
    .into_iter()
    .find(|c| c.name.to_lowercase().contains("keymap"))
    .expect("keymap check");
  assert_eq!(
    c.status,
    CheckStatus::Failed,
    "a threaded bad global must fail the check: {}",
    c.detail
  );

  // Not threaded (None) → the very same file is ignored; the check is clean.
  let without = DoctorCtx {
    repo_workdir: dir.path(),
    repo: &repo,
    config: &config,
    global_config_path: None,
  };
  let c2 = doctor::run(&without)
    .unwrap()
    .checks
    .into_iter()
    .find(|c| c.name.to_lowercase().contains("keymap"))
    .expect("keymap check");
  assert_eq!(
    c2.status,
    CheckStatus::Ok,
    "an un-threaded global config must be ignored: {}",
    c2.detail
  );
}

#[test]
fn doctor_modal_error_outranks_the_quit_warning() {
  // #219 review (P2): an invalid modal binding is a hard Failed. When the same
  // config also unbinds `quit` (a Warning), the modal error must still win —
  // otherwise the quit-warning early return hides the actionable modal error.
  let (dir, repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[tui.keys]\nquit = []\n\n[tui.keys.modal.confirm]\nconfirm = [\"g g\"]\n",
  )
  .unwrap();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.to_lowercase().contains("keymap"))
    .expect("expected a TUI keymap check");
  assert_eq!(
    c.status,
    CheckStatus::Failed,
    "the modal error must outrank the quit warning, got: {}",
    c.detail
  );
  assert!(
    c.detail.contains("single keystroke"),
    "detail must be the modal error, not the quit warning, got: {}",
    c.detail
  );
}

#[test]
fn doctor_fails_on_disk_modal_error_even_when_context_defaulted() {
  // #219 review (P2): `repo_context_lenient` returns `Config::default()` when
  // `load_for_repo` rejects the user's `.gwm.toml` (here a multi-stroke modal
  // chord, which load-time validation refuses). If doctor only re-validated
  // that defaulted ctx.config it would report OK for a config that actually
  // refuses to start the TUI — so the keymap check must re-read the on-disk
  // file. This reproduces the real lenient path (the existing conflict test
  // injects straight into ctx.config and never exercises the default-away).
  let (dir, repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[tui.keys.modal.confirm]\nconfirm = [\"g g\"]\n",
  )
  .unwrap();
  // The file must be load-invalid (so the lenient loader would default it).
  assert!(
    Config::load_layered(dir.path(), None).is_err(),
    "the on-disk modal chord must be rejected at load time for this regression"
  );
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report
    .checks
    .iter()
    .find(|c| c.name.to_lowercase().contains("keymap"))
    .expect("expected a TUI keymap check");
  assert_eq!(
    c.status,
    CheckStatus::Failed,
    "doctor must flag the on-disk modal error, not the defaulted ctx.config: {}",
    c.detail
  );
  assert!(
    c.detail.contains("single keystroke"),
    "detail must explain the multi-stroke modal chord rejection, got: {}",
    c.detail
  );
}

// Check #4 — the forge CLI is probed only when `forge` is set explicitly
// (issue #419).

#[test]
fn forge_cli_is_not_probed_when_the_key_is_unset() {
  // The non-regression half, and the one that runs identically everywhere:
  // a config that never opts into a forge must not start warning about a
  // missing `gh` / `glab` for users who don't touch issue/PR linking.
  let (dir, repo) = init_repo();
  let config = Config::default();

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
  let missing = c.detail.split("not on PATH:").nth(1).unwrap_or("");

  assert!(
    !missing.contains("glab") && !missing.contains("gh"),
    "no forge CLI should be probed without an explicit `forge` key, got: {}",
    c.detail
  );
}

#[test]
fn an_explicit_gitlab_forge_probes_glab() {
  let (dir, repo) = init_repo();
  let config = Config {
    forge: Some(gwm::forge::ForgeKind::GitLab),
    ..Default::default()
  };

  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();

  // `glab` may legitimately be installed on a contributor's machine, so
  // the positive assertion is scoped to the case where it is genuinely
  // absent — which is every CI runner. Asserting the warning
  // unconditionally would be exactly the ambient-`$PATH` flake the repo
  // rules call out.
  if which::which("glab").is_err() {
    assert_eq!(c.status, CheckStatus::Warning, "missing glab must warn: {}", c.detail);
    assert!(
      c.detail.contains("glab"),
      "the missing forge CLI should be named, got: {}",
      c.detail
    );
  }
  // Env-independent in both directions: selecting GitLab must never probe
  // for the GitHub CLI.
  let missing = c.detail.split("not on PATH:").nth(1).unwrap_or("");
  assert!(
    !missing.split(',').any(|b| b.trim() == "gh"),
    "selecting GitLab must not probe for `gh`, got: {}",
    c.detail
  );
}

#[test]
fn a_forge_hosts_entry_is_also_an_opt_in_for_the_cli_probe() {
  // `[forge_hosts]` is the second way to say "I talk to this forge", and it
  // says it about *this host* specifically — a stronger opt-in signal than
  // the bare `forge` key, not a weaker one. Probing only on `forge` left a
  // user who authorises purely through the global table with a clean
  // `gwm doctor` and no `glab` installed.
  let (dir, repo) = init_repo();
  repo
    .remote("origin", "https://gitlab.acme.internal/team/proj.git")
    .unwrap();
  let global = dir.path().join("global.toml");
  std::fs::write(&global, "[forge_hosts]\n\"gitlab.acme.internal\" = \"gitlab\"\n").unwrap();
  // No `forge` key anywhere: the host entry is the whole opt-in.
  let config = Config::default();

  let report = doctor::run(&DoctorCtx {
    repo_workdir: dir.path(),
    repo: &repo,
    config: &config,
    global_config_path: Some(&global),
  })
  .unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();

  // Same env-independence rule as `an_explicit_gitlab_forge_probes_glab`:
  // the positive assertion only holds where `glab` is genuinely absent.
  if which::which("glab").is_err() {
    assert_eq!(c.status, CheckStatus::Warning, "missing glab must warn: {}", c.detail);
    assert!(
      c.detail.contains("glab"),
      "the forge CLI should be named, got: {}",
      c.detail
    );
  }
  let missing = c.detail.split("not on PATH:").nth(1).unwrap_or("");
  assert!(
    !missing.split(',').any(|b| b.trim() == "gh"),
    "a GitLab host entry must not probe for `gh`, got: {}",
    c.detail
  );
}

#[test]
fn a_forge_hosts_entry_for_another_host_probes_nothing() {
  // The bound: the table authorises *named* hosts, so an entry that does not
  // match this repo's origin is not an opt-in for it. Without this, adding
  // one host to the global config would start warning in every unrelated
  // repo — the same over-probing the `forge` key was careful to avoid.
  let (dir, repo) = init_repo();
  repo.remote("origin", "https://github.com/team/proj.git").unwrap();
  let global = dir.path().join("global.toml");
  std::fs::write(&global, "[forge_hosts]\n\"gitlab.acme.internal\" = \"gitlab\"\n").unwrap();
  let config = Config::default();

  let report = doctor::run(&DoctorCtx {
    repo_workdir: dir.path(),
    repo: &repo,
    config: &config,
    global_config_path: Some(&global),
  })
  .unwrap();
  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
  let missing = c.detail.split("not on PATH:").nth(1).unwrap_or("");

  assert!(
    !missing.contains("glab") && !missing.split(',').any(|b| b.trim() == "gh"),
    "an unrelated host entry must not probe any forge CLI, got: {}",
    c.detail
  );
}

#[test]
fn the_forge_cli_probe_honours_the_gwm_gh_override() {
  // `$GWM_GH` / `$GWM_GLAB` point at an alternative binary; probing the
  // bare name `gh` regardless warned about a setup that works, and pushed
  // the exit code to 1 (Codex review #458).
  let (dir, repo) = init_repo();
  // Windows resolves executability from the extension, not a mode bit, so
  // an extensionless file is genuinely not runnable there — the probe was
  // right to report it missing and the test was wrong to expect otherwise.
  // A real override on Windows names a `.exe`, so the fixture does too.
  let fake = dir.path().join(if cfg!(windows) { "my-gh.exe" } else { "my-gh" });
  std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&fake).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake, perms).unwrap();
  }
  let config = Config {
    forge: Some(gwm::forge::ForgeKind::GitHub),
    ..Default::default()
  };

  let _env = env_lock().lock().unwrap_or_else(|p| p.into_inner());
  let prior = std::env::var("GWM_GH").ok();
  // SAFETY: env mutation guarded by the lock above; restored below.
  unsafe {
    std::env::set_var("GWM_GH", &fake);
  }
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();
  unsafe {
    match prior {
      Some(v) => std::env::set_var("GWM_GH", v),
      None => std::env::remove_var("GWM_GH"),
    }
  }

  let c = report.checks.iter().find(|c| c.name.contains("PATH")).unwrap();
  let missing = c.detail.split("not on PATH:").nth(1).unwrap_or("");
  assert!(
    !missing.contains("gh"),
    "the overridden binary exists, so nothing should be reported missing: {}",
    c.detail
  );
}

/// Issue #415: a `worktree.branch_pattern` gwm cannot read back silently
/// disables every feature that re-parses a branch name, so `gwm doctor`
/// states the limitation instead of leaving it silent.
///
/// Issue #417 derived the parser from the pattern, which shrank the set this
/// applies to: `{type}-{issue}-{desc}` was the example here and round-trips
/// now. A `~`-leading pattern is what still reaches the "everything inactive"
/// verdict, because the writer ends with a tilde expansion the reader cannot
/// undo.
#[test]
fn a_branch_pattern_nothing_reads_back_warns_that_the_parser_is_blind() {
  let (dir, repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[worktree]\nbranch_pattern = \"~/{type}/#{issue}-{desc}\"\n",
  )
  .unwrap();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let check = report
    .checks
    .iter()
    .find(|c| c.name.contains("branch_pattern"))
    .expect("expected a `branch_pattern` check in the report");

  assert_eq!(check.status, CheckStatus::Warning);
  // The message has to name the consequence, not just the divergence —
  // the whole point is connecting cause to effect.
  for expected in ["auto-linking", "gitmoji", "branch-convention"] {
    assert!(
      check.detail.contains(expected),
      "warning should name the '{}' consequence, got: {}",
      expected,
      check.detail
    );
  }
}

#[test]
fn default_branch_pattern_does_not_warn() {
  let (dir, repo) = init_repo();
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let check = report
    .checks
    .iter()
    .find(|c| c.name.contains("branch_pattern"))
    .expect("expected a `branch_pattern` check in the report");

  assert_eq!(check.status, CheckStatus::Ok);
}

/// Issue #415 (Codex review): `repo_context_lenient` hands `doctor` a
/// `Config::default()` when the on-disk config fails to load for an
/// unrelated semantic reason. Reading `ctx.config` would then report the
/// default pattern as fine while the file on disk carries a broken one —
/// a false `✓` from the very check that exists to stop silent failures.
/// Re-derive from disk, as `check_tui_keymap` already does.
#[test]
fn branch_pattern_check_reads_the_on_disk_config_not_the_lenient_fallback() {
  let (dir, repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[worktree]\nbranch_pattern = \"~/{type}/#{issue}-{desc}\"\n",
  )
  .unwrap();
  // What `repo_context_lenient` would have handed us after a load failure.
  let config = Config::default();
  let report = doctor::run(&ctx_for(&repo, dir.path(), &config)).unwrap();

  let check = report
    .checks
    .iter()
    .find(|c| c.name.contains("branch_pattern"))
    .expect("expected a `branch_pattern` check in the report");

  assert_eq!(check.status, CheckStatus::Warning);
}
