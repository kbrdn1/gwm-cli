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
