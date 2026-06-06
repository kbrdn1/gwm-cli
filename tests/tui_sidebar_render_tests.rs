//! Render-level characterization tests for the details sidebar (issue
//! #238). The state-machine tests in `tui_state_sidebar_tests.rs` are
//! ratatui-free and pin focus / scroll / cache *state* — they never put
//! a glyph on a buffer. The #238 perf refactor reworks `render_section`
//! (cached commit lines are now rendered by reference rather than
//! deep-cloned every frame) and prepends the live header line separately,
//! so we need a net that actually rasterizes the sidebar and asserts the
//! visible content / order is unchanged. These tests warm the cache, draw
//! to a `TestBackend`, and assert the buffer carries the header name and a
//! known commit subject — content + order, not a brittle full-ANSI
//! snapshot.

use gwm::tui::{draw, App};
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;
use tempfile::TempDir;

/// Build a temp git repo with `commit_count` commits whose subjects are
/// `commit-<i>` so the render test can assert a known subject lands in the
/// recent-commits section.
fn repo_with_commits(commit_count: usize) -> TempDir {
  let dir = TempDir::new().unwrap();
  let repo = git2::Repository::init(dir.path()).unwrap();
  repo.set_head("refs/heads/main").ok();
  let sig = git2::Signature::now("gwm-test", "gwm@test").unwrap();

  std::fs::write(dir.path().join("file.txt"), "seed").unwrap();
  repo.index().unwrap().add_path(Path::new("file.txt")).unwrap();
  repo.index().unwrap().write().unwrap();
  {
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
  }
  for i in 0..commit_count {
    std::fs::write(dir.path().join("file.txt"), format!("commit-{i}")).unwrap();
    repo.index().unwrap().add_path(Path::new("file.txt")).unwrap();
    repo.index().unwrap().write().unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo
      .commit(Some("HEAD"), &sig, &sig, &format!("commit-{i}"), &tree, &[&parent])
      .unwrap();
  }
  dir
}

/// Flatten a `TestBackend` buffer into a single string of cell symbols so
/// `contains` can look for a substring regardless of wrapping/styling.
fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
  let buffer = terminal.backend().buffer();
  buffer.content().iter().map(|c| c.symbol()).collect()
}

#[test]
fn sidebar_renders_header_name_and_commit_subject_on_warm_cache() {
  let dir = repo_with_commits(8);
  // `None` global path keeps construction off the runner's real config.
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  // The main worktree must be selected for the sidebar to have content.
  assert!(app.selected().is_some(), "expected the main worktree to be selected");
  let name = app.selected().unwrap().name.clone();

  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();

  // First draw populates `app.sidebar.cache` (cold → warm).
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  assert!(app.sidebar.cache.is_some(), "first draw must warm the sidebar cache");

  // Second draw is the warm-cache path the #238 refactor optimizes — the
  // visible content must be identical.
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  let text = buffer_text(&terminal);
  assert!(
    text.contains(&name),
    "sidebar header must render the worktree name, got:\n{text}"
  );
  assert!(
    text.contains("commit-7"),
    "sidebar recent-commits must render the most recent commit subject, got:\n{text}"
  );
}

#[test]
fn sidebar_warm_cache_render_is_stable_across_frames() {
  let dir = repo_with_commits(8);
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();

  terminal.draw(|f| draw(f, &mut app)).unwrap();
  let first = buffer_text(&terminal);

  terminal.draw(|f| draw(f, &mut app)).unwrap();
  let second = buffer_text(&terminal);

  assert_eq!(
    first, second,
    "two consecutive warm-cache draws must produce byte-identical sidebar buffers"
  );
}
