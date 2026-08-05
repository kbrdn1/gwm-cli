//! Lifecycle hook execution → Command Logs transcript (issue #226).
//!
//! A lifecycle hook is one of the external commands gwm runs, so running a
//! phase must record its shell step on the command log. `HookContext` has
//! public fields, so the test builds one directly (no `git2::Repository`
//! fixture needed) and drives `run_phase` against a `tempdir` cwd. Unique
//! sentinel keeps the assertion robust against entries a sibling test
//! recorded concurrently (presence, not count).

use gwm::config::{Config, HookStep};
use gwm::lifecycle::{self, HookContext, HookPhase, HookSkips};
use std::collections::HashMap;
use std::path::Path;

fn ctx_at(cwd: &Path) -> HookContext {
  HookContext {
    main_repo: cwd.to_path_buf(),
    cwd: cwd.to_path_buf(),
    path: cwd.to_path_buf(),
    branch: "feat/#226-demo".into(),
    branch_type: "feat".into(),
    issue: "226".into(),
    desc: "demo".into(),
    user: "tester".into(),
    owner: "kbrdn1".into(),
    repo: "gwm-cli".into(),
  }
}

/// Run one `post_create` hook in `dir` and hand back the step's captured
/// output. Hooks are the only place gwm builds a shell command out of
/// values it did not author, so most assertions below are about what a
/// hook *sees*, not about the report shape.
fn run_one_hook(dir: &Path, ctx: &HookContext, run: &str, env: HashMap<String, String>) -> String {
  let mut cfg = Config::default();
  cfg.hooks.post_create.push(HookStep {
    name: "probe".into(),
    run: run.into(),
    when: None,
    env,
    on_fail: gwm::config::HookOnFail::default(),
  });
  let ctx = ctx.with_cwd(dir);
  let report = lifecycle::run_phase(&cfg, HookPhase::PostCreate, &ctx, &HookSkips::default(), false).unwrap();
  report
    .steps
    .iter()
    .map(|s| s.detail.clone())
    .collect::<Vec<_>>()
    .join("\n")
}

// ── placeholder values are data, never code ────────────────────────────────
//
// `run_step` hands its expanded string to `sh -c`. Every placeholder value
// comes from somewhere gwm does not control — the branch name is whatever
// git says it is, and a branch can be created by a colleague, a fork PR, or
// a `git fetch`. Git permits `; | & $ ` ( ) < >` in a ref name, so an
// unescaped substitution lets a branch name terminate the hook's command and
// start its own.

#[test]
fn a_branch_name_cannot_terminate_the_hook_command() {
  let dir = tempfile::TempDir::new().unwrap();
  let proof = dir.path().join("pwned");
  let mut ctx = ctx_at(dir.path());
  // A payload with no spaces, because git rejects those in a ref — the
  // constraint an attacker actually works under.
  ctx.branch = format!("x;id>{}", proof.display());

  let out = run_one_hook(dir.path(), &ctx, "echo {branch}", HashMap::new());

  assert!(
    !proof.exists(),
    "the branch name started a second command: {} was created",
    proof.display()
  );
  assert!(
    out.contains(&ctx.branch),
    "the hook must receive the whole branch name as one literal value, got: {out}"
  );
}

#[test]
fn every_placeholder_is_escaped_not_just_the_branch() {
  // `{branch}` is the one with the most obvious attacker-controlled path,
  // but `for_worktree` derives `{type}` / `{issue}` / `{desc}` from that
  // same branch, and `{repo}` / `{owner}` come from a remote URL.
  let dir = tempfile::TempDir::new().unwrap();
  for (field, template) in [
    ("desc", "echo {desc}"),
    ("issue", "echo {issue}"),
    ("branch_type", "echo {type}"),
    ("repo", "echo {repo}"),
    ("owner", "echo {owner}"),
    ("user", "echo {user}"),
  ] {
    let proof = dir.path().join(format!("pwned-{field}"));
    let payload = format!("x;id>{}", proof.display());
    let mut ctx = ctx_at(dir.path());
    match field {
      "desc" => ctx.desc = payload,
      "issue" => ctx.issue = payload,
      "branch_type" => ctx.branch_type = payload,
      "repo" => ctx.repo = payload,
      "owner" => ctx.owner = payload,
      _ => ctx.user = payload,
    }
    run_one_hook(dir.path(), &ctx, template, HashMap::new());
    assert!(!proof.exists(), "`{{{field}}}` executed its payload");
  }
}

#[test]
fn a_value_that_looks_like_a_placeholder_is_not_expanded_again() {
  // Substitution has to be single-pass. Replacing `{branch}` first and
  // `{issue}` after would rewrite the token *inside* the value that was just
  // substituted — and with escaping in play it would splice quote characters
  // into the middle of another value.
  let dir = tempfile::TempDir::new().unwrap();
  let mut ctx = ctx_at(dir.path());
  ctx.branch = "spike-{issue}".into();
  ctx.issue = "42".into();

  let out = run_one_hook(dir.path(), &ctx, "echo {branch}", HashMap::new());

  assert!(
    out.contains("spike-{issue}"),
    "the branch name must reach the hook verbatim, got: {out}"
  );
  assert!(!out.contains("spike-42"), "the value was re-expanded: {out}");
}

#[test]
fn env_values_are_passed_raw_because_they_never_reach_a_shell() {
  // `step.env` entries go to `Command::env`, not into the `sh -c` string, so
  // escaping them would push literal quote characters into the value the
  // hook reads back. Same placeholders, deliberately different treatment.
  let dir = tempfile::TempDir::new().unwrap();
  let ctx = ctx_at(dir.path());
  let env = HashMap::from([("GWM_TEST_BRANCH".to_string(), "{branch}".to_string())]);

  let out = run_one_hook(dir.path(), &ctx, "printenv GWM_TEST_BRANCH", env);

  assert!(
    out.contains("feat/#226-demo"),
    "the env value must be the raw branch, got: {out}"
  );
  assert!(!out.contains('\''), "the env value must not carry shell quotes: {out}");
}

#[test]
fn the_context_is_also_exported_as_environment_variables() {
  // The escape-on-expansion fix makes `{branch}` safe, but a hook that wants
  // the value without thinking about quoting at all can read `"$GWM_BRANCH"`:
  // parameter expansion never re-parses metacharacters, so no substitution
  // there can start a second command.
  let dir = tempfile::TempDir::new().unwrap();
  let mut ctx = ctx_at(dir.path());
  ctx.branch = "x;id".into();

  let out = run_one_hook(
    dir.path(),
    &ctx,
    "printenv GWM_BRANCH; printenv GWM_REPO",
    HashMap::new(),
  );

  assert!(out.contains("x;id"), "GWM_BRANCH must carry the raw branch: {out}");
  assert!(out.contains("gwm-cli"), "GWM_REPO must be exported too: {out}");
}

#[test]
fn a_legacy_bootstrap_command_gets_the_same_escaping() {
  // `[[bootstrap.command]]` steps are folded into the post_create phase via
  // `HookStep::from`, so they run through the same `run_step`. That surface
  // is older and more widely configured than `[[hooks.*]]`, so it must not
  // be left behind by the fix.
  let dir = tempfile::TempDir::new().unwrap();
  let proof = dir.path().join("pwned-legacy");
  let mut ctx = ctx_at(dir.path());
  ctx.branch = format!("x;id>{}", proof.display());

  let mut cfg = Config::default();
  cfg.bootstrap.command.push(gwm::config::CommandStep {
    name: "legacy".into(),
    run: "echo {branch}".into(),
    when: None,
    env: HashMap::new(),
  });
  let ctx = ctx.with_cwd(dir.path());
  lifecycle::run_phase(&cfg, HookPhase::PostCreate, &ctx, &HookSkips::default(), true).unwrap();

  assert!(!proof.exists(), "a legacy bootstrap command executed the payload");
}

#[test]
fn post_create_hook_is_recorded_on_the_command_log() {
  let sentinel = "gwm-hook-cmdlog-5b7a";
  let dir = tempfile::TempDir::new().unwrap();
  let mut cfg = Config::default();
  cfg.hooks.post_create.push(HookStep {
    name: "echo".into(),
    run: format!("echo {sentinel}"),
    when: None,
    env: HashMap::new(),
    on_fail: gwm::config::HookOnFail::default(),
  });

  let ctx = ctx_at(dir.path());
  lifecycle::run_phase(&cfg, HookPhase::PostCreate, &ctx, &HookSkips::default(), false).unwrap();

  let recorded = gwm::command_log::snapshot();
  let mine = recorded
    .iter()
    .find(|e| e.command.contains(sentinel))
    .expect("post_create hook recorded on the command log");
  assert!(mine.is_success(), "the echo hook exited cleanly");
  assert!(mine.output.contains(sentinel), "captured stdout is stored");
}

#[test]
fn an_empty_placeholder_expands_to_nothing_not_to_an_empty_argument() {
  // `shell_words::quote("")` is `''`, so escaping an empty value would turn
  // `mycmd {issue}` into `mycmd ''` — one argument where every release up to
  // 1.5.0 passed none. `{type}` / `{issue}` / `{desc}` are empty on any
  // branch that does not match the convention, so that is not a rare shape.
  //
  // An empty value has nothing to inject, so it passes through untouched.
  // That keeps the escaping's blast radius to exactly the vulnerability it
  // closes: a user applying a security patch should not also be absorbing an
  // unrelated semantic change they did not opt into.
  let dir = tempfile::tempdir().unwrap();
  let mut ctx = ctx_at(dir.path());
  ctx.issue = String::new();
  let out = run_one_hook(dir.path(), &ctx, "set -- {issue}; echo $#", HashMap::new());
  assert_eq!(
    out.trim(),
    "0",
    "an empty placeholder must not start passing an empty argument"
  );

  // The escaping still applies the moment there is anything to escape.
  let mut hostile = ctx_at(dir.path());
  hostile.issue = "1;id".into();
  let out = run_one_hook(dir.path(), &hostile, "set -- {issue}; echo $#", HashMap::new());
  assert_eq!(out.trim(), "1", "a non-empty value stays a single quoted argument");
}

/// Issue #531: the removal hook context must parse the branch with the config
/// the caller already loaded, not with a fresh read of `.gwm.toml`.
///
/// `HookContext::for_worktree` used to build its parser through
/// `BranchParser::for_repo`, which calls `Config::load_for_repo` — a fourth
/// open of the same file inside one delete, and one that re-resolves the
/// global config path the caller may have deliberately overridden (#194).
#[test]
fn the_hook_context_parses_the_branch_with_the_config_it_was_given() {
  let dir = tempfile::tempdir().unwrap();
  let repo = git2::Repository::init(dir.path()).unwrap();
  // On disk: a pattern no `feat/#531-x` branch can match, so a re-read
  // leaves every derived placeholder empty.
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[worktree]\nbranch_pattern = \"wt/{desc}\"\n",
  )
  .unwrap();

  let ctx = HookContext::for_worktree(
    &repo,
    dir.path(),
    dir.path(),
    dir.path(),
    Some("feat/#531-x"),
    &Config::default(),
  );

  assert_eq!(ctx.branch_type, "feat", "{{type}} must come from the given config");
  assert_eq!(ctx.issue, "531", "{{issue}} must come from the given config");
  assert_eq!(ctx.desc, "x", "{{desc}} must come from the given config");
}
