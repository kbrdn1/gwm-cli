//! The removal sequence shared by `gwm remove` and the TUI `d` (issue #521).
//!
//! Its own test binary rather than a block in `tui_app_tests.rs` because
//! these tests rewrite `GWM_HISTORY_FILE`, and the env-guard invariant
//! (#507) derives the set of tests that must serialise from a transitive
//! walk of `src/`: putting a `set_var` in `tui_app_tests.rs` would demand
//! the lock from every test in that binary that reaches `history::record`,
//! which after this change is every delete test there.

mod common;

use common::init_repo;
use gwm::tui::App;
use gwm::{history, worktree};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Process-global lock for every test here that rewrites `GWM_HISTORY_FILE`.
/// Same contract as the one in `history_tests.rs` — `set_var` races a
/// concurrent `getenv`, and the delete worker reads the variable from
/// another thread.
fn env_lock() -> &'static Mutex<()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  LOCK.get_or_init(|| Mutex::new(()))
}

/// Point the undo journal at `path` for the duration of the test. Returns the
/// previous value so the caller can restore it before dropping the env lock.
///
/// # Safety
/// Callers must hold [`env_lock`].
unsafe fn set_journal(path: &Path) -> Option<String> {
  let prev = std::env::var("GWM_HISTORY_FILE").ok();
  unsafe { std::env::set_var("GWM_HISTORY_FILE", path) };
  prev
}

/// # Safety
/// Callers must hold [`env_lock`].
unsafe fn restore_journal(prev: Option<String>) {
  unsafe {
    match prev {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
  }
}

/// Pump the task channel until the delete worker has posted its batch.
/// Panics rather than hanging if the worker never reports.
fn wait_for_delete(app: &mut App) {
  let deadline = Instant::now() + Duration::from_secs(30);
  while app.is_delete_worktree_loading() {
    app.drain_task_results();
    assert!(
      Instant::now() < deadline,
      "the delete worker never posted its batch (status: {})",
      app.status
    );
    std::thread::sleep(Duration::from_millis(10));
  }
}

/// Select the row named `name` and mark nothing — the cursor-row batch.
fn select_row(app: &mut App, name: &str) {
  let row = app
    .worktrees
    .iter()
    .position(|w| w.name == name)
    .unwrap_or_else(|| panic!("{name} is not listed"));
  app.list_state.select(Some(row));
}

/// A path as a `sh -c` script reads it. The hook body goes through a shell,
/// and a Windows shell eats the backslashes before the command sees them
/// (PR #520) — `/` works on every platform.
fn shell_path(p: &Path) -> String {
  p.to_string_lossy().replace('\\', "/")
}

/// A `.gwm.toml` whose only surface is the two remove hooks. Each writes a
/// witness file into the main checkout, which is the one directory still
/// around after the worktree is gone.
fn write_remove_hooks(workdir: &Path, pre_run: &str) {
  let config = format!(
    r#"
[[hooks.pre_remove]]
name = "pre witness"
run = "{pre_run}"

[[hooks.post_remove]]
name = "post witness"
run = "printf gone > post-remove.txt"
"#
  );
  std::fs::write(workdir.join(".gwm.toml"), config).unwrap();
}

#[test]
fn a_tui_delete_writes_the_undo_journal_entry() {
  // Issue #521: `gwm remove` records the removal so `gwm undo` can put it
  // back; `d` in the TUI did not, so an interactive delete was unrecoverable.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-journal");
  worktree::add(&repo, "wt-521-journal", &doomed, "feat/#521-journal", false).unwrap();

  let journal_dir = TempDir::new().unwrap();
  let journal_path = journal_dir.path().join("history.toml");
  // SAFETY: guarded by `env_lock` above; restored before the lock drops.
  let prev = unsafe { set_journal(&journal_path) };

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  select_row(&mut app, "wt-521-journal");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  // Without this the journal assertion below can pass vacuously: a removal
  // that never happened writes no entry either.
  assert!(
    !doomed.exists(),
    "the worktree must actually be gone (status: {}, failure: {:?})",
    app.status,
    app.delete_failure
  );

  let journal = history::Journal::load(&journal_path).unwrap();
  let entry = journal
    .entries()
    .iter()
    .find(|e| e.worktree == "wt-521-journal")
    .expect("the TUI delete must record an undo journal entry");
  assert_eq!(entry.kind, history::OpKind::Remove);
  assert_eq!(entry.branch.as_deref(), Some("feat/#521-journal"));
  assert!(
    entry.branch_oid.is_some(),
    "the branch tip must be captured so `gwm undo` can recreate the ref"
  );
  assert_eq!(entry.path, doomed);

  // SAFETY: paired with the `set_journal` above, still under the lock.
  unsafe { restore_journal(prev) };
}

#[test]
fn a_tui_delete_runs_the_remove_hooks() {
  // Issue #521: a `pre_remove` hook that guards a removal only held for
  // whoever used the CLI — `d` in the TUI walked straight past it.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-hooks");
  worktree::add(&repo, "wt-521-hooks", &doomed, "feat/#521-hooks", false).unwrap();

  // The pre hook runs inside the worktree that is about to be destroyed, so
  // its witness goes to an absolute path outside it.
  let witness = wt_root.path().join("pre-remove.txt");
  write_remove_hooks(
    dir.path(),
    &format!("printf gone > '{}'", shell_path(&witness)),
  );

  let journal_dir = TempDir::new().unwrap();
  // SAFETY: guarded by `env_lock` above; restored before the lock drops.
  let prev = unsafe { set_journal(&journal_dir.path().join("history.toml")) };

  let mut app = App::new_at_layered(Some(dir.path()), None)
    .unwrap()
    .with_trust_mode(gwm::trust::TrustMode::Allow);
  select_row(&mut app, "wt-521-hooks");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(
    !doomed.exists(),
    "the worktree must be gone (status: {}, failure: {:?})",
    app.status,
    app.delete_failure
  );
  assert!(witness.exists(), "the pre_remove hook must run on the TUI path");
  assert!(
    dir.path().join("post-remove.txt").exists(),
    "the post_remove hook must run on the TUI path"
  );

  // SAFETY: paired with the `set_journal` above, still under the lock.
  unsafe { restore_journal(prev) };
}

#[test]
fn a_pre_remove_refusal_keeps_the_worktree_and_reports_it() {
  // Same contract as `gwm remove`: a failing `pre_remove` refuses that
  // target, and nothing downstream of it runs — no destruction, no journal
  // entry, no `post_remove`.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-refused");
  worktree::add(&repo, "wt-521-refused", &doomed, "feat/#521-refused", false).unwrap();
  write_remove_hooks(dir.path(), "false");

  let journal_dir = TempDir::new().unwrap();
  let journal_path = journal_dir.path().join("history.toml");
  // SAFETY: guarded by `env_lock` above; restored before the lock drops.
  let prev = unsafe { set_journal(&journal_path) };

  let mut app = App::new_at_layered(Some(dir.path()), None)
    .unwrap()
    .with_trust_mode(gwm::trust::TrustMode::Allow);
  select_row(&mut app, "wt-521-refused");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(doomed.exists(), "a refused pre_remove must leave the worktree alone");
  let banner = app.delete_failure.clone().unwrap_or_default();
  assert!(
    banner.contains("pre_remove"),
    "the failure must name the hook that refused, got: {banner:?} (status: {})",
    app.status
  );
  assert!(
    !dir.path().join("post-remove.txt").exists(),
    "post_remove must not run when the removal never happened"
  );
  assert!(
    history::Journal::load(&journal_path).unwrap().entries().is_empty(),
    "a refused removal must not be recorded as undoable"
  );

  // SAFETY: paired with the `set_journal` above, still under the lock.
  unsafe { restore_journal(prev) };
}

#[test]
fn remove_hooks_are_gated_on_the_trust_ledger() {
  // Running `[hooks.pre_remove]` is arbitrary code execution out of a file
  // the repo author controls, which is what the TOFU ledger exists for
  // (#95). The TUI cannot host a stdin prompt, so an unapproved config
  // refuses the removal rather than silently skipping the guard hook.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-untrusted");
  worktree::add(&repo, "wt-521-untrusted", &doomed, "feat/#521-untrusted", false).unwrap();
  let witness = wt_root.path().join("pre-remove.txt");
  write_remove_hooks(
    dir.path(),
    &format!("printf gone > '{}'", shell_path(&witness)),
  );

  let journal_dir = TempDir::new().unwrap();
  let ledger = journal_dir.path().join("trust.toml");
  // SAFETY: guarded by `env_lock` above; both restored before the lock drops.
  let prev = unsafe { set_journal(&journal_dir.path().join("history.toml")) };
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  unsafe { std::env::set_var("GWM_TRUST_LEDGER", &ledger) };

  // `TrustMode::Prompt` is the default a plain `gwm` launch carries.
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  select_row(&mut app, "wt-521-untrusted");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(doomed.exists(), "an untrusted config must not have its worktree removed");
  assert!(!witness.exists(), "the hook must not have run");
  let banner = app.delete_failure.clone().unwrap_or_default();
  assert!(
    banner.contains("trust"),
    "the failure must point at the trust ledger, got: {banner:?} (status: {})",
    app.status
  );

  // SAFETY: paired with the sets above, still under the lock.
  unsafe {
    restore_journal(prev);
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn the_batch_removes_the_path_it_confirmed_not_whatever_the_id_now_names() {
  // Guard from PR #520 (`remove_verified`), restated here because #521 makes
  // the worker re-resolve the target to build its journal entry: a removal
  // whose expected path comes from that fresh resolution compares live state
  // against itself and can never refuse. The expected path must stay the one
  // captured in the confirm snapshot.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let original = wt_root.path().join("wt-521-moved");
  worktree::add(&repo, "wt-521-moved", &original, "feat/#521-moved", false).unwrap();

  let journal_dir = TempDir::new().unwrap();
  let journal_path = journal_dir.path().join("history.toml");
  // SAFETY: guarded by `env_lock` above; restored before the lock drops.
  let prev = unsafe { set_journal(&journal_path) };

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  select_row(&mut app, "wt-521-moved");
  app.enter_confirm_delete();
  assert_eq!(app.pending_delete().len(), 1, "status was: {}", app.status);

  // The worktree moves between the confirm and the keystroke that fires it:
  // git keeps the id, so an id-only removal would destroy the new location.
  let moved = wt_root.path().join("wt-521-elsewhere");
  worktree::run_git(
    dir.path(),
    &["worktree", "move", &original.to_string_lossy(), &moved.to_string_lossy()],
  )
  .unwrap();

  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(moved.exists(), "the worktree at its new path must survive");
  assert!(
    app.delete_failure.is_some(),
    "the batch must report the refusal (status: {})",
    app.status
  );
  assert!(
    history::Journal::load(&journal_path).unwrap().entries().is_empty(),
    "a refused removal must not be recorded as undoable"
  );

  // SAFETY: paired with the `set_journal` above, still under the lock.
  unsafe { restore_journal(prev) };
}
