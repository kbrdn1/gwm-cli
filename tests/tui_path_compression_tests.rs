//! The `PATH` cell is actually wired to the compression (issue #568).
//!
//! Its own test binary, and that is the whole design. The compression resolves
//! `$HOME` through `dirs::home_dir()` behind a `OnceLock`, so the first lookup
//! anywhere in a binary freezes the value for every later one. A single test
//! per binary makes that lookup deterministic: this file plants a temporary
//! home *before* anything reads one, so the fixture never touches the real home
//! directory and the test does not depend on it existing, being writable, or
//! being anywhere in particular.
//!
//! `#[cfg(unix)]` because the injection is: `dirs::home_dir()` reads `$HOME`
//! there, while on Windows it goes to `SHGetKnownFolderPath` and ignores the
//! variable entirely. The rule the compression applies on Windows is covered
//! instead by `tilde_compress_matches_across_the_two_windows_separators` in
//! `tui_app_tests.rs`, which needs no fixture. The wiring under test here —
//! `build_row` calling `display_path` at all — is not platform-dependent.

#![cfg(unix)]

use gwm::tui::{draw, App};
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

/// This binary's environment lock, same shape as `trust_tests`.
///
/// It serialises nothing today — one test lives here, which is the point of
/// the file. It exists because `env_guard_invariant_tests` derives its audit
/// from the `set_var` calls themselves and grants no exemptions, deliberately:
/// every hand-maintained exemption list written while fixing #507 was wrong
/// within the hour. A second test landing here later inherits the guard
/// instead of discovering it.
fn env_lock() -> &'static Mutex<()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn the_path_cell_renders_the_compressed_form_not_the_raw_one() {
  let _guard = env_lock().lock().unwrap();
  // Plant the home first: any earlier `dirs::home_dir()` would win the
  // `OnceLock` and this test would silently measure the real home instead.
  //
  // Canonicalised, because a real `$HOME` is and a `TempDir` is not: on macOS
  // `/var` is a symlink to `/private/var`, libgit2 resolves it when it reports
  // `WorktreeInfo::path` and the environment does not, so the raw temp path
  // would never prefix-match the rendered one.
  let home = TempDir::new().unwrap();
  let home_path = std::fs::canonicalize(home.path()).unwrap();
  std::env::set_var("HOME", &home_path);

  let dir = TempDir::new_in(&home_path).unwrap();
  let repo = git2::Repository::init(dir.path()).unwrap();
  repo.set_head("refs/heads/main").ok();
  let sig = git2::Signature::now("gwm-test", "gwm@test").unwrap();
  std::fs::write(dir.path().join("file.txt"), "seed").unwrap();
  repo.index().unwrap().add_path(Path::new("file.txt")).unwrap();
  repo.index().unwrap().write().unwrap();
  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  // Wide enough that the `Fill(1)` PATH column keeps the whole value: the
  // column is hard-clipped with no ellipsis, so a narrow frame would cut the
  // home prefix away and the negative assertion would pass for the wrong
  // reason.
  let mut terminal = Terminal::new(TestBackend::new(300, 40)).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  let buf = terminal.backend().buffer();
  let area = *buf.area();

  // The table row, not the header: both carry a path, and the header has
  // compressed since long before this change, so matching the wrong line would
  // make the assertion pass without `build_row` doing anything.
  let row = (0..area.height)
    .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect::<String>())
    .find(|line| line.contains("clean"))
    .expect("the fixture's single worktree must render a row");

  // `dirs::home_dir()` re-read rather than `home.path()`: on macOS a `TempDir`
  // under `/var` is handed back through `/private/var`, and the cell spells
  // whatever libgit2 resolved.
  let raw = dirs::home_dir().expect("HOME was just planted").display().to_string();
  assert!(
    !row.contains(&raw),
    "the PATH cell still spells $HOME out in full: {:?}",
    row.trim_end()
  );
  let name = dir.path().file_name().unwrap().to_string_lossy();
  assert!(
    row.contains(&format!("~/{name}")),
    "the PATH cell must render the compressed path, got {:?}",
    row.trim_end()
  );
}
