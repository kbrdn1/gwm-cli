//! Issue #238: warm-cache sidebar render bench.
//!
//! `draw_sidebar` re-renders the cached worktree-preview sections on every
//! frame. Before #238 each of those warm frames deep-cloned the cached
//! `SidebarSections` (up to `RECENT_COMMITS_LIMIT` = 300 commit `Line`s, each
//! with owned `String` spans) purely to dodge a borrow conflict. That hot path
//! was previously un-benched.
//!
//! Since #343 the render path never shells out, so a frame does NOT warm the
//! cache any more: `App::maybe_refresh_sidebar` spawns a worker and
//! `App::drain_task_results` stores its `TaskMsg::Sidebar` payload. This bench
//! seeds that payload directly, the deterministic analogue of the pair, with
//! no OS thread, exactly as `tests/tui_sidebar_render_tests.rs` does. It went
//! dead for 1086 commits because it kept drawing a frame and asserting the
//! cache had filled itself, which is the pre-#343 contract (issue #634).
//!
//! This bench drives the public `draw` entry point against a ratatui
//! `TestBackend` with the sidebar visible and a *warm* cache. It is a
//! full-frame draw, not a `draw_sidebar`-only micro-bench (`draw_sidebar` is
//! private), but the worktree table cost is constant across before/after, so
//! the delta is attributable to the sidebar clone removal.

use criterion::{criterion_group, criterion_main, Criterion};
use git2::{Repository, Signature};
use gwm::tui::{build_sidebar_payload, draw, App};
use ratatui::{backend::TestBackend, Terminal};
use std::hint::black_box;
use std::path::Path;
use tempfile::TempDir;

/// Build a temp git repo carrying `commit_count` commits so the sidebar's
/// Recent Commits section is fully populated (300 → the `RECENT_COMMITS_LIMIT`
/// cap) on a warm cache. Subjects are `commit-<i>` so the warm-cache assertion
/// below has a known string to look for on the rendered buffer.
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

/// Seed `app.sidebar.cache` for the selected worktree + mode. The key has to
/// come from `app.selected()`, not from the `TempDir` path: libgit2
/// canonicalises the workdir, so on macOS the two differ (`/private/var/…` vs
/// `/var/…`) and `draw_sidebar`, which serves the cache only on an exact key
/// match, would render the "loading…" placeholder instead.
fn warm_sidebar(app: &mut App) {
  let w = app.selected().expect("the main worktree is always listed").clone();
  let mode = app.sidebar.mode;
  let payload = build_sidebar_payload(
    &w,
    mode,
    &app.config.doctor.trunks,
    &app.theme,
    app.config.tui.status_one_line,
  );
  assert!(
    !payload.recent_commits.is_empty(),
    "a payload built over a 300-commit repo must carry commit lines"
  );
  app.sidebar.cache = Some(((w.path.clone(), mode), payload));
}

fn sidebar_cache_hit(c: &mut Criterion) {
  let dir = repo_with_commits(300);
  // `None` global path keeps construction off the host's real config.
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();

  warm_sidebar(&mut app);
  // One frame outside the timing loop, asserted on. `cache.is_some()`, the
  // assertion this bench used to carry, is not the invariant: a payload under
  // a key that does not match the selection renders the placeholder, and the
  // bench would still report a plausible number for drawing it. Reading a
  // commit subject off the buffer pins the warm path itself.
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  let rendered: String = terminal
    .backend()
    .buffer()
    .content()
    .iter()
    .map(|cell| cell.symbol())
    .collect();
  assert!(
    rendered.contains("commit-"),
    "the timed frame must render cached commit subjects, not the loading placeholder"
  );

  c.bench_function("draw_sidebar_warm_cache_300_commits", |b| {
    b.iter(|| {
      terminal.draw(|f| draw(f, black_box(&mut app))).unwrap();
    });
  });
}

criterion_group!(benches, sidebar_cache_hit);
criterion_main!(benches);
