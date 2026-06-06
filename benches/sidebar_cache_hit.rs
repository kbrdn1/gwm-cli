//! Issue #238: warm-cache sidebar render bench.
//!
//! `draw_sidebar` populates `app.sidebar.cache` on the first frame and then
//! re-renders the cached worktree-preview sections on every subsequent frame.
//! Before #238 each of those warm frames deep-cloned the cached
//! `SidebarSections` (up to `RECENT_COMMITS_LIMIT` = 300 commit `Line`s, each
//! with owned `String` spans) purely to dodge a borrow conflict. That hot path
//! was previously un-benched.
//!
//! This bench drives the public `draw` entry point against a ratatui
//! `TestBackend` with the sidebar visible and a *warm* cache (we render one
//! frame to populate it, then time the redraw). It is a full-frame draw, not a
//! `draw_sidebar`-only micro-bench — `draw_sidebar` is private — but the
//! worktree table cost is constant across before/after, so the delta is
//! attributable to the sidebar clone removal.

use criterion::{criterion_group, criterion_main, Criterion};
use git2::{Repository, Signature};
use gwm::tui::{draw, App};
use ratatui::{backend::TestBackend, Terminal};
use std::hint::black_box;
use std::path::Path;
use tempfile::TempDir;

/// Build a temp git repo carrying `commit_count` commits so the sidebar's
/// Recent Commits section is fully populated (300 → the `RECENT_COMMITS_LIMIT`
/// cap) on a warm cache.
fn repo_with_commits(commit_count: usize) -> TempDir {
  let dir = TempDir::new().unwrap();
  let repo = Repository::init(dir.path()).unwrap();
  repo.set_head("refs/heads/main").ok();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();

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

fn sidebar_cache_hit(c: &mut Criterion) {
  let dir = repo_with_commits(300);
  // `None` global path keeps construction off the host's real config.
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();

  // Warm the cache: the first frame runs `git log` and stores the rendered
  // sections. Every timed frame below is a pure cache hit.
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  assert!(app.sidebar.cache.is_some(), "cache must be warm before benching");

  c.bench_function("draw_sidebar_warm_cache_300_commits", |b| {
    b.iter(|| {
      terminal.draw(|f| draw(f, black_box(&mut app))).unwrap();
    });
  });
}

criterion_group!(benches, sidebar_cache_hit);
criterion_main!(benches);
