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

use gwm::tui::{build_sidebar_payload, draw, App};
use ratatui::{backend::TestBackend, Terminal};
use std::path::Path;
use tempfile::TempDir;

/// Warm `app.sidebar.cache` the way the async worker + drain would (issue
/// #343): build the payload for the currently selected worktree + mode and
/// store it under that key. The render path no longer shells out, so a render
/// test that wants real git content in the sidebar must seed the cache first —
/// the deterministic analogue of `maybe_refresh_sidebar` spawning a worker and
/// `drain_task_results` applying its `TaskMsg::Sidebar`, with no OS thread.
fn warm_sidebar(app: &mut App) {
  let w = app.selected().expect("a worktree must be selected").clone();
  let mode = app.sidebar.mode;
  let payload = build_sidebar_payload(&w, mode, &app.config.doctor.trunks, &app.theme);
  app.sidebar.cache = Some(((w.path.clone(), mode), payload));
}

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
fn draw_does_not_shell_out_to_git_or_warm_the_sidebar_cache() {
  // Issue #343: the sidebar's git subprocesses (`git_diff_stat_vs_base`,
  // `git status --porcelain -z`, `git log`, `git stash list`) must run on the
  // async worker, never inside `terminal.draw()`. The observable contract:
  // a draw over a cold cache leaves the cache cold — the render path no longer
  // computes it, so it can't have shelled out. The worker + drain populate it.
  let dir = repo_with_commits(3);
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  app.sidebar.cache = None; // cold

  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  assert!(
    app.sidebar.cache.is_none(),
    "draw must not run git subprocesses to warm the sidebar cache; the async worker does (issue #343)"
  );
}

#[test]
fn stale_key_cache_renders_the_loading_placeholder_not_another_worktrees_preview() {
  // Issue #343: the sidebar cache is a single slot, so "render the last-known
  // value" on a key-miss would mean showing a *different* worktree's commits
  // under the live header of the current selection. The render deliberately
  // shows the muted "loading…" placeholder instead — this guards that
  // deviation against a future refactor silently reinstating the stale render.
  use gwm::tui::SidebarSections;
  use ratatui::text::Line;
  use std::path::PathBuf;

  let dir = repo_with_commits(4);
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let mode = app.sidebar.mode;
  // Cache holds a payload keyed to a DIFFERENT worktree (stale key), carrying a
  // distinctive commit subject that must never surface under the current one.
  app.sidebar.cache = Some((
    (PathBuf::from("/tmp/gwm-test/some-other-worktree"), mode),
    SidebarSections {
      recent_commits: vec![Line::from("GHOST-COMMIT-XYZ")],
      ..SidebarSections::default()
    },
  ));

  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  let text = buffer_text(&terminal);
  assert!(
    text.contains("loading…"),
    "a stale-key cache must render the loading placeholder: {text}"
  );
  assert!(
    !text.contains("GHOST-COMMIT-XYZ"),
    "the other worktree's cached commits must NOT render under the current header: {text}"
  );
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

  // Issue #343: the render path no longer warms the cache (the async worker
  // does), so seed it explicitly to render real git content.
  warm_sidebar(&mut app);
  assert!(
    app.sidebar.cache.is_some(),
    "the warm helper must seed the sidebar cache"
  );

  // Warm-cache render path the #238 refactor optimizes — the visible content
  // must carry the header name and the most recent commit subject.
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
  warm_sidebar(&mut app);

  terminal.draw(|f| draw(f, &mut app)).unwrap();
  let first = buffer_text(&terminal);

  terminal.draw(|f| draw(f, &mut app)).unwrap();
  let second = buffer_text(&terminal);

  assert_eq!(
    first, second,
    "two consecutive warm-cache draws must produce byte-identical sidebar buffers"
  );
}

#[test]
fn working_tree_section_renders_colored_status_counts_footer() {
  // 11 fresh untracked files → all land in the `created` bucket (issue
  // #287), so the footer renders the created glyph + count rather than a
  // bare total.
  let dir = repo_with_commits(1);
  for i in 0..11 {
    std::fs::write(dir.path().join(format!("dirty-{i}.txt")), "dirty").unwrap();
  }
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  warm_sidebar(&mut app);

  terminal.draw(|f| draw(f, &mut app)).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  let text = buffer_text(&terminal);
  assert!(
    text.contains("Working Tree"),
    "sidebar must render the Working Tree pane: {text}"
  );
  assert!(
    text.contains(gwm::tui::WT_CREATED_ICON),
    "footer must render the created (diff-added) nerdfont glyph: {text}"
  );
  assert!(text.contains("11"), "footer must render the created-file count: {text}");
}

#[test]
fn working_tree_section_renders_file_tree_with_icons() {
  // A nested untracked file must render as an explorer tree (issue #300):
  // the collapsed `src/app` directory row with a folder glyph, then the
  // `mod.rs` leaf with the Rust file-type glyph — not a flat `?? src/app/mod.rs`.
  let dir = repo_with_commits(1);
  std::fs::create_dir_all(dir.path().join("src/app")).unwrap();
  std::fs::write(dir.path().join("src/app/mod.rs"), "fn x() {}").unwrap();

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  warm_sidebar(&mut app);
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  let text = buffer_text(&terminal);
  assert!(
    text.contains("src/app"),
    "single-child dir chain renders collapsed: {text}"
  );
  assert!(text.contains("mod.rs"), "the leaf file name renders: {text}");
  assert!(
    text.contains(gwm::tui::wt_tree::WT_DIR_OPEN_ICON),
    "directory row carries a folder glyph: {text}"
  );
  assert!(
    text.contains(gwm::tui::wt_tree::WT_RUST_ICON),
    "the .rs leaf carries the Rust file-type glyph: {text}"
  );
  assert!(
    text.contains('└') || text.contains('├'),
    "rows are drawn with tree connector lines: {text}"
  );
}

/// Run a `git` CLI command in `dir`, asserting success. Lets a render test
/// build a feature branch with a deterministic diff against `main`.
fn git_in(dir: &std::path::Path, args: &[&str]) {
  let out = std::process::Command::new("git")
    .current_dir(dir)
    .args(args)
    .env("GIT_AUTHOR_NAME", "gwm-test")
    .env("GIT_AUTHOR_EMAIL", "gwm@test")
    .env("GIT_COMMITTER_NAME", "gwm-test")
    .env("GIT_COMMITTER_EMAIL", "gwm@test")
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "git {:?} failed: {}",
    args,
    String::from_utf8_lossy(&out.stderr)
  );
}

#[test]
fn status_pane_renders_diff_vs_base_line_on_a_feature_branch() {
  // Repo seeded on `main`; branch off and change a line so the three-dot
  // diff against `main` is non-empty. The main worktree (HEAD = the feature
  // branch) must then show the `Diff +N -M` line in the Status pane (issue
  // #287).
  let dir = repo_with_commits(1);
  let path = dir.path();
  std::fs::write(path.join("f.txt"), "a\nb\nc\n").unwrap();
  git_in(path, &["add", "f.txt"]);
  git_in(path, &["commit", "-m", "base file"]);
  git_in(path, &["checkout", "-b", "feat/#287-diff"]);
  std::fs::write(path.join("f.txt"), "a\nB\nc\nd\n").unwrap();
  git_in(path, &["commit", "-am", "tweak"]);

  let mut app = App::new_at_layered(Some(path), None).unwrap();
  let backend = TestBackend::new(120, 40);
  let mut terminal = Terminal::new(backend).unwrap();
  warm_sidebar(&mut app);
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  let text = buffer_text(&terminal);
  assert!(text.contains("Diff"), "Status pane must show the Diff label: {text}");
  // +2 insertions (line B replaced + line d added), -1 deletion (line b).
  assert!(text.contains("+2"), "Status pane must show insertions: {text}");
  assert!(text.contains("-1"), "Status pane must show deletions: {text}");
}
