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

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  // The ledger is redirected too, so nothing here can read (or write) the
  // runner's real `~/.config/gwm/trust.toml`.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  select_row(&mut app, "wt-521-journal");
  // The path as gwm knows it, read before the removal. Not
  // `doomed.canonicalize()`: `Worktree::path()` and a canonicalised tempdir
  // carry different normalisations, and they disagree on two of the three
  // runners — `/var` versus `/private/var` on macOS, and the `\\?\` verbatim
  // prefix Windows prepends. `worktree::remove_verified` compares paths
  // verbatim for the same reason, so the assertion below reads them the way
  // the production code does.
  let listed_path = app
    .worktrees
    .iter()
    .find(|w| w.name == "wt-521-journal")
    .expect("the worktree is listed")
    .path
    .clone();

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
  assert_eq!(entry.path, listed_path, "the entry names the worktree that was removed");

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  // Written out rather than factored into a helper: the #507 guard is
  // positional, so whichever function calls `set_var` is the one that has to
  // hold the lock.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
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
  write_remove_hooks(dir.path(), &format!("printf gone > '{}'", shell_path(&witness)));

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  // The ledger is redirected too, so nothing here can read (or write) the
  // runner's real `~/.config/gwm/trust.toml`.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

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

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  // Written out rather than factored into a helper: the #507 guard is
  // positional, so whichever function calls `set_var` is the one that has to
  // hold the lock.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
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

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  // The ledger is redirected too, so nothing here can read (or write) the
  // runner's real `~/.config/gwm/trust.toml`.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

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

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  // Written out rather than factored into a helper: the #507 guard is
  // positional, so whichever function calls `set_var` is the one that has to
  // hold the lock.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
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
  write_remove_hooks(dir.path(), &format!("printf gone > '{}'", shell_path(&witness)));

  let sandbox = TempDir::new().unwrap();
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  // The ledger points at an empty tempdir, which is the whole subject here:
  // this config has never been approved.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", sandbox.path().join("history.toml"));
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  // `TrustMode::Prompt` is the default a plain `gwm` launch carries.
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  select_row(&mut app, "wt-521-untrusted");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(
    doomed.exists(),
    "an untrusted config must not have its worktree removed"
  );
  assert!(!witness.exists(), "the hook must not have run");
  let banner = app.delete_failure.clone().unwrap_or_default();
  assert!(
    banner.contains("trust"),
    "the failure must point at the trust ledger, got: {banner:?} (status: {})",
    app.status
  );

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  // Written out rather than factored into a helper: the #507 guard is
  // positional, so whichever function calls `set_var` is the one that has to
  // hold the lock.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
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

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  // The ledger is redirected too, so nothing here can read (or write) the
  // runner's real `~/.config/gwm/trust.toml`.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  select_row(&mut app, "wt-521-moved");
  app.enter_confirm_delete();
  assert_eq!(app.pending_delete().len(), 1, "status was: {}", app.status);

  // The worktree moves between the confirm and the keystroke that fires it:
  // git keeps the id, so an id-only removal would destroy the new location.
  let moved = wt_root.path().join("wt-521-elsewhere");
  worktree::run_git(
    dir.path(),
    &[
      "worktree",
      "move",
      &original.to_string_lossy(),
      &moved.to_string_lossy(),
    ],
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

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  // Written out rather than factored into a helper: the #507 guard is
  // positional, so whichever function calls `set_var` is the one that has to
  // hold the lock.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn a_failing_post_remove_hook_is_not_reported_as_a_failed_removal() {
  // Same contract the CLI got from the Codex review on PR #520: `post_remove`
  // runs on a worktree that IS gone, so an `on_fail = "abort"` there is not a
  // failed removal. Reporting it as one would keep the confirm overlay open
  // offering to remove a row that no longer exists.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-post-fails");
  worktree::add(&repo, "wt-521-post-fails", &doomed, "feat/#521-post", false).unwrap();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "\n[[hooks.post_remove]]\nname = \"noisy cleanup\"\nrun = \"false\"\n",
  )
  .unwrap();

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  let mut app = App::new_at_layered(Some(dir.path()), None)
    .unwrap()
    .with_trust_mode(gwm::trust::TrustMode::Allow);
  select_row(&mut app, "wt-521-post-fails");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(!doomed.exists(), "the worktree is gone (status: {})", app.status);
  assert_eq!(
    app.delete_failure, None,
    "a hook failing after the removal must not read as a failed removal"
  );
  assert!(
    app.status.contains("post_remove"),
    "the hook failure still has to be surfaced, got: {}",
    app.status
  );
  assert_eq!(
    gwm::history::Journal::load(&journal_path).unwrap().entries().len(),
    1,
    "the removal happened, so it is recorded"
  );

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn gwm_undo_restores_what_the_tui_deleted() {
  // The acceptance criterion for #521 is a round trip, not a well-shaped
  // journal entry: `gwm undo` filters on `repo_root` verbatim, so an entry
  // whose path was canonicalised differently than undo resolves it satisfies
  // every field assertion and is still invisible. Only the binary can say.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-roundtrip");
  worktree::add(&repo, "wt-521-roundtrip", &doomed, "feat/#521-roundtrip", false).unwrap();

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  select_row(&mut app, "wt-521-roundtrip");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);
  assert!(!doomed.exists(), "the worktree is gone (status: {})", app.status);

  // `gwm undo` from the main checkout, reading the same journal.
  let undo = std::process::Command::new(env!("CARGO_BIN_EXE_gwm"))
    .current_dir(dir.path())
    .env("GWM_HISTORY_FILE", &journal_path)
    .arg("undo")
    .output()
    .unwrap();
  assert!(
    undo.status.success(),
    "gwm undo failed: {}{}",
    String::from_utf8_lossy(&undo.stdout),
    String::from_utf8_lossy(&undo.stderr)
  );
  assert!(
    doomed.exists(),
    "undo must put the worktree back where the TUI removed it: {}",
    String::from_utf8_lossy(&undo.stdout)
  );

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn a_moved_target_runs_no_hook_before_it_is_refused() {
  // Codex review on PR #526 (P1). The worker re-resolves its target to name
  // the branch in the journal entry, so `found.path` is where the worktree is
  // NOW. `pre_remove` runs with its cwd there, and the mismatch with the
  // confirmed path was only caught afterwards, inside `remove_verified`: a
  // destructive hook had already run against a directory the user never
  // confirmed. Nothing may execute before the path is checked.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let original = wt_root.path().join("wt-521-hook-race");
  worktree::add(&repo, "wt-521-hook-race", &original, "feat/#521-race", false).unwrap();

  // The hook writes outside the worktree, so the witness survives whatever
  // happens to the directory.
  let witness = wt_root.path().join("hook-ran.txt");
  write_remove_hooks(dir.path(), &format!("printf ran > '{}'", shell_path(&witness)));

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  let mut app = App::new_at_layered(Some(dir.path()), None)
    .unwrap()
    .with_trust_mode(gwm::trust::TrustMode::Allow);
  select_row(&mut app, "wt-521-hook-race");
  app.enter_confirm_delete();
  assert_eq!(app.pending_delete().len(), 1, "status was: {}", app.status);

  // It moves between the confirm and the keystroke that fires it.
  let moved = wt_root.path().join("wt-521-hook-race-moved");
  worktree::run_git(
    dir.path(),
    &[
      "worktree",
      "move",
      &original.to_string_lossy(),
      &moved.to_string_lossy(),
    ],
  )
  .unwrap();

  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(moved.exists(), "the worktree at its new path must survive");
  assert!(
    !witness.exists(),
    "no hook may run against a path the user did not confirm (status: {})",
    app.status
  );
  assert!(
    app.delete_failure.is_some(),
    "the batch must report the refusal (status: {})",
    app.status
  );

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn a_warn_hook_reaches_the_status_line() {
  // Codex review on PR #526 (P2). `on_fail = "warn"` is a success carrying a
  // Warning step. The CLI prints the whole report, so the user sees the `!`;
  // the TUI prints no report, so unless the step reaches the status line the
  // phase means nothing there.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-warn");
  worktree::add(&repo, "wt-521-warn", &doomed, "feat/#521-warn", false).unwrap();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "\n[[hooks.post_remove]]\nname = \"noisy cleanup\"\nrun = \"false\"\non_fail = \"warn\"\n",
  )
  .unwrap();

  let sandbox = TempDir::new().unwrap();
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", sandbox.path().join("history.toml"));
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  let mut app = App::new_at_layered(Some(dir.path()), None)
    .unwrap()
    .with_trust_mode(gwm::trust::TrustMode::Allow);
  select_row(&mut app, "wt-521-warn");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(!doomed.exists(), "a warn hook does not stop the removal");
  assert_eq!(app.delete_failure, None, "a warning is not a failure");
  assert!(
    app.status.contains("noisy cleanup"),
    "the warning must name its step, got: {}",
    app.status
  );

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn a_remove_hook_from_the_global_config_needs_no_repo_approval() {
  // Codex review on PR #526 (P2). The trust ledger is about the repo's
  // `.gwm.toml`. A remove hook coming from `~/.config/gwm/config.toml` is the
  // user's own, so gating on the MERGED config sent an unapproved repo file
  // to the ledger even though no line of it would run during the removal.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-global-hook");
  worktree::add(&repo, "wt-521-global-hook", &doomed, "feat/#521-global", false).unwrap();

  // The repo file carries an executable surface the ledger has never seen,
  // but nothing that runs on a removal.
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "\n[[hooks.post_create]]\nname = \"unrelated\"\nrun = \"true\"\n",
  )
  .unwrap();

  let sandbox = TempDir::new().unwrap();
  let witness = sandbox.path().join("global-hook-ran.txt");
  let global = sandbox.path().join("config.toml");
  std::fs::write(
    &global,
    format!(
      "\n[[hooks.post_remove]]\nname = \"global cleanup\"\nrun = \"printf ran > '{}'\"\n",
      shell_path(&witness)
    ),
  )
  .unwrap();

  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  // The ledger is empty, which is the point: this repo has never been
  // approved.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", sandbox.path().join("history.toml"));
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  // `TrustMode::Prompt` is what a plain `gwm` launch carries.
  let mut app = App::new_at_layered(Some(dir.path()), Some(&global)).unwrap();
  select_row(&mut app, "wt-521-global-hook");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(
    !doomed.exists(),
    "an unapproved repo file that runs nothing on a removal must not block it (status: {}, failure: {:?})",
    app.status,
    app.delete_failure
  );
  assert!(witness.exists(), "the user's own global hook still runs");

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn a_hook_that_invalidates_its_own_target_leaves_no_journal_entry() {
  // Codex review on PR #526 (P2). The path check runs before the hooks, so a
  // hook that invalidates its OWN target passed it, the entry was written,
  // and the removal was then refused: the journal claimed a removal that
  // never happened, and `gwm undo` would pop that instead of a recoverable
  // one. Hence the second check, after the hooks and before the write.
  //
  // The hook deletes the admin entry rather than moving the worktree: a move
  // renames the directory the hook is sitting in and that libgit2 has just
  // memory-mapped an index from, which Windows refuses outright
  // (`Permission denied`, red on windows-latest only). Deleting
  // `.git/worktrees/<id>` invalidates the target just as thoroughly and
  // touches nothing anyone holds open.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-self-invalidate");
  worktree::add(&repo, "wt-521-self-invalidate", &doomed, "feat/#521-self", false).unwrap();
  let admin = dir.path().join(".git").join("worktrees").join("wt-521-self-invalidate");
  assert!(admin.is_dir(), "the admin entry exists before the hook runs");

  std::fs::write(
    dir.path().join(".gwm.toml"),
    format!(
      "\n[[hooks.pre_remove]]\nname = \"pull the rug\"\nrun = \"rm -rf '{}'\"\n",
      shell_path(&admin)
    ),
  )
  .unwrap();

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  let mut app = App::new_at_layered(Some(dir.path()), None)
    .unwrap()
    .with_trust_mode(gwm::trust::TrustMode::Allow);
  select_row(&mut app, "wt-521-self-invalidate");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);

  assert!(!admin.exists(), "the hook ran, so the check below is not vacuous");
  assert!(
    doomed.exists(),
    "nothing was removed from disk (status: {})",
    app.status
  );
  assert!(
    history::Journal::load(&journal_path).unwrap().entries().is_empty(),
    "nothing was removed, so nothing may claim to be undoable"
  );

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn the_journal_names_the_branch_the_removal_actually_deletes() {
  // Codex review on PR #526 (P1). `worktree::remove` deletes the branch it
  // reads off the worktree's HEAD at removal time, while the journal entry
  // took it from the listing the caller resolved earlier. Anything that
  // checks out another branch in between — a `pre_remove` hook here, a hook
  // on an earlier target of the same batch in the wild — made `gwm undo`
  // restore a ref that was never deleted while the deleted one stayed lost.
  //
  // Driven by the target's own hook rather than by a sibling's: `list()` does
  // not sort, so batch order follows `.git/worktrees/` and is a filesystem
  // detail (this test read the branch of whichever ran first on ubuntu). The
  // divergence under test is snapshot-versus-HEAD, which one target shows.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-branch-swap");
  worktree::add(&repo, "wt-521-branch-swap", &doomed, "feat/#521-before", false).unwrap();

  std::fs::write(
    dir.path().join(".gwm.toml"),
    format!(
      "\n[[hooks.pre_remove]]\nname = \"swap the branch\"\nrun = \"git -C '{}' checkout -b feat/#521-after\"\n",
      shell_path(&doomed)
    ),
  )
  .unwrap();

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  let mut app = App::new_at_layered(Some(dir.path()), None)
    .unwrap()
    .with_trust_mode(gwm::trust::TrustMode::Allow);
  select_row(&mut app, "wt-521-branch-swap");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);
  assert!(!doomed.exists(), "the worktree is gone (status: {})", app.status);

  let journal = history::Journal::load(&journal_path).unwrap();
  let entry = journal.entries().first().expect("the removal is recorded");
  assert_eq!(
    entry.branch.as_deref(),
    Some("feat/#521-after"),
    "the entry must name the branch the removal saw, not the one the listing had"
  );

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}

#[test]
fn a_detached_head_is_recorded_as_no_branch() {
  // Codex review on PR #526 (P2). `branch_at_removal_time` treated "opened
  // the worktree and found a detached HEAD" and "could not open the worktree"
  // as the same `None`, then fell back to the listing for both. A hook that
  // detaches HEAD therefore produced an entry naming a branch the removal
  // deleted nothing of, and `gwm undo` would recreate the worktree on it
  // instead of reporting a detached-HEAD entry.
  let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

  let (dir, repo) = init_repo();
  let wt_root = TempDir::new().unwrap();
  let doomed = wt_root.path().join("wt-521-detach");
  worktree::add(&repo, "wt-521-detach", &doomed, "feat/#521-detach", false).unwrap();

  // Attached at listing time, detached by the hook, so the fallback is what
  // decides what the entry says.
  std::fs::write(
    dir.path().join(".gwm.toml"),
    format!(
      "\n[[hooks.pre_remove]]\nname = \"detach\"\nrun = \"git -C '{}' checkout --detach\"\n",
      shell_path(&doomed)
    ),
  )
  .unwrap();

  let sandbox = TempDir::new().unwrap();
  let journal_path = sandbox.path().join("history.toml");
  let prev_history = std::env::var("GWM_HISTORY_FILE").ok();
  let prev_ledger = std::env::var("GWM_TRUST_LEDGER").ok();
  // SAFETY: `env_lock` above serialises every env mutation in this binary.
  // Both variables are restored at the end of the test, under the same guard.
  unsafe {
    std::env::set_var("GWM_HISTORY_FILE", &journal_path);
    std::env::set_var("GWM_TRUST_LEDGER", sandbox.path().join("trust.toml"));
  }

  let mut app = App::new_at_layered(Some(dir.path()), None)
    .unwrap()
    .with_trust_mode(gwm::trust::TrustMode::Allow);
  select_row(&mut app, "wt-521-detach");
  app.enter_confirm_delete();
  app.confirm_delete().unwrap();
  wait_for_delete(&mut app);
  assert!(!doomed.exists(), "the worktree is gone (status: {})", app.status);

  let journal = history::Journal::load(&journal_path).unwrap();
  let entry = journal.entries().first().expect("the removal is recorded");
  assert_eq!(
    entry.branch, None,
    "a detached HEAD deletes no branch, so the entry must claim none"
  );

  // SAFETY: still under the `env_lock` guard taken at the top of this test.
  unsafe {
    match prev_history {
      Some(v) => std::env::set_var("GWM_HISTORY_FILE", v),
      None => std::env::remove_var("GWM_HISTORY_FILE"),
    }
    match prev_ledger {
      Some(v) => std::env::set_var("GWM_TRUST_LEDGER", v),
      None => std::env::remove_var("GWM_TRUST_LEDGER"),
    }
  }
}
