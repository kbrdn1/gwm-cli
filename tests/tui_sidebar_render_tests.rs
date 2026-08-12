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

use gwm::config::TuiLayout;
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
  // Checked in both layouts (issue #545): compact moves this footer out
  // of the bottom rule and onto the right of the header line, so the
  // counts have to survive the move — including their per-category
  // colours, which the header fill must not flatten.
  for (layout, title) in [
    (TuiLayout::Bordered, "Working Tree"),
    (TuiLayout::Compact, "WORKING TREE"),
  ] {
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    app.config.tui.layout = layout;
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();
    warm_sidebar(&mut app);

    terminal.draw(|f| draw(f, &mut app)).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();

    let text = buffer_text(&terminal);
    assert!(
      text.contains(title),
      "{layout:?}: sidebar must render the Working Tree pane: {text}"
    );
    assert!(
      text.contains(gwm::tui::WT_CREATED_ICON),
      "{layout:?}: footer must render the created (diff-added) nerdfont glyph: {text}"
    );
    assert!(
      text.contains("11"),
      "{layout:?}: footer must render the created-file count: {text}"
    );
    // The count keeps the `untracked` role rather than inheriting the
    // header's accent — the fill carries focus, not category.
    assert_eq!(
      fg_of(&terminal, gwm::tui::WT_CREATED_ICON),
      Some(gwm::tui::theme::Theme::default().untracked),
      "{layout:?}: the created glyph keeps its category colour"
    );
  }
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

#[test]
fn working_tree_section_shows_a_scrollbar_when_the_tree_overflows() {
  // User feedback on PR #454: the Working Tree pane scrolls (#437) and gets
  // clamped by the responsive split (#438), but nothing showed *where* the
  // viewport sits — the scrollbar affordance was missing. On a short
  // terminal with a large change set the pane must paint the herdr-style
  // thumb on its inner right column, like the overflowing modals do.
  let dir = repo_with_commits(20);
  for i in 0..30 {
    std::fs::write(dir.path().join(format!("file-{i:02}.rs")), "x").unwrap();
  }

  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  // 50 rows: tall enough that the responsive split hands the Working Tree a
  // real (but clamped) viewport — on a shorter terminal the tiny-terminal
  // path can shrink the pane to its borders, where there is nothing to
  // scroll over and no bar to draw.
  let backend = TestBackend::new(120, 50);
  let mut terminal = Terminal::new(backend).unwrap();
  warm_sidebar(&mut app);
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  assert!(
    app.sidebar.wt_max_scroll > 0,
    "fixture must actually overflow the Working Tree viewport"
  );
  let text = buffer_text(&terminal);
  assert!(
    text.contains('█'),
    "an overflowing Working Tree pane must show the scrollbar thumb: {text}"
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

// ---------------------------------------------------------------------------
// Compact mode (issue #545)
// ---------------------------------------------------------------------------

/// Count the box *corners* the panes and sidebar sections draw. The
/// worktrees pane uses square corners, the sidebar sections rounded ones;
/// both are covered.
///
/// Corners rather than every box-drawing glyph: since the compact mode
/// gained a separator rule between the two panes, a bare `─` no longer
/// implies a box. A corner still does — nothing else draws one — so this
/// stays the honest test for "the frames are gone".
fn box_corner_count(terminal: &Terminal<TestBackend>) -> usize {
  terminal
    .backend()
    .buffer()
    .content()
    .iter()
    .filter(|c| matches!(c.symbol(), "┌" | "┐" | "└" | "┘" | "╭" | "╮" | "╰" | "╯"))
    .count()
}

/// Draw the list view once at a fixed size with `[tui] compact` set as
/// asked, and hand back the terminal for inspection. Two draws: the first
/// settles the layout-dependent state (scroll clamps republished against
/// the areas the solver granted), exactly like the Diff test above.
fn draw_list_view(dir: &Path, compact: bool) -> Terminal<TestBackend> {
  let mut app = App::new_at_layered(Some(dir), None).unwrap();
  app.config.tui.layout = if compact {
    TuiLayout::Compact
  } else {
    TuiLayout::Bordered
  };
  let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
  warm_sidebar(&mut app);
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  terminal
}

#[test]
fn compact_mode_drops_the_frames_and_default_mode_keeps_them() {
  // The observable half of issue #545: with `compact = true` no pane and
  // no sidebar section is boxed. Asserted against the default render of
  // the *same* repo, so a change that silently stopped drawing borders
  // everywhere would fail the second half rather than pass both.
  let dir = repo_with_commits(6);
  let bordered = box_corner_count(&draw_list_view(dir.path(), false));
  let compact = box_corner_count(&draw_list_view(dir.path(), true));

  assert!(bordered > 0, "the default layout must still draw its boxes");
  assert_eq!(compact, 0, "compact mode must draw no box at all");
}

#[test]
fn compact_mode_rules_the_boundary_between_the_two_panes() {
  // Validation feedback on PR #546: with every section delimited the same
  // way — a filled header — nothing said where the worktrees pane ended
  // and the sidebar began. The two are separately focusable, so that
  // boundary is worth one line.
  //
  // Checked as a full-width run of `─` on its own row, which is what
  // distinguishes the separator from a stray glyph in a commit subject,
  // and absent by default (the box rules already do the job there).
  let dir = repo_with_commits(6);
  let full_width_rule_rows = |compact: bool| {
    let terminal = draw_list_view(dir.path(), compact);
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    (area.y..area.y + area.height)
      .filter(|&y| (area.x..area.x + area.width).all(|x| buffer[(x, y)].symbol() == "─"))
      .count()
  };
  assert_eq!(
    full_width_rule_rows(true),
    1,
    "compact must rule the pane boundary exactly once"
  );
  assert_eq!(
    full_width_rule_rows(false),
    0,
    "the bordered layout draws no such rule — its boxes already separate the panes"
  );
}

#[test]
fn compact_mode_headers_carry_the_chord_and_the_fill() {
  // The rules are replaced, not merely removed: each section keeps a
  // one-line header, led by its keybinding, painted on the `section_bg`
  // role. Without the fill the headers would read as ordinary content
  // rows and the layout would lose its boundaries entirely.
  let dir = repo_with_commits(4);
  let terminal = draw_list_view(dir.path(), true);
  let text = buffer_text(&terminal);
  assert!(text.contains("[1] WORKTREES"), "compact worktrees header: {text}");
  assert!(text.contains("[2] STATUS"), "compact status header: {text}");

  let theme = gwm::tui::theme::Theme::default();
  let buffer = terminal.backend().buffer();
  let filled = buffer.content().iter().filter(|c| c.bg == theme.section_bg).count();
  assert!(
    filled > 0,
    "section headers must be painted with the section_bg role, found none"
  );
}

#[test]
fn compact_mode_hands_the_saved_rows_to_content() {
  // The point of the mode. At an identical terminal size the compact
  // render must fit strictly more commit subjects in the Recent Commits
  // section than the bordered one — otherwise the borders were dropped
  // for nothing.
  let dir = repo_with_commits(40);
  let count_subjects = |t: &Terminal<TestBackend>| {
    let text = buffer_text(t);
    (0..40).filter(|i| text.contains(&format!("commit-{i}"))).count()
  };
  let bordered = count_subjects(&draw_list_view(dir.path(), false));
  let compact = count_subjects(&draw_list_view(dir.path(), true));
  assert!(
    compact > bordered,
    "compact must show more commits than bordered at the same size (bordered {bordered}, compact {compact})"
  );
}

#[test]
fn compact_mode_scrolls_recent_commits_to_the_end() {
  // The scroll clamps read the same `chrome.rows()` the layout does, so a
  // compact section publishes a viewport one row taller than the bordered
  // one. What is observable from a buffer is the end state: parked at the
  // published `max_scroll`, the oldest commit is on screen.
  //
  // Note what this does NOT pin: whether `max_scroll` overshoots the end
  // by a row. Both a correct clamp and one still subtracting a hard-coded
  // 2 put `init` on screen here, and the geometry needed to tell them
  // apart is not exposed. The arithmetic itself is pinned upstream, on
  // the pure solver (`section_heights_*` in tui_app_tests).
  let dir = repo_with_commits(40);
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  app.config.tui.layout = TuiLayout::Compact;
  let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
  warm_sidebar(&mut app);
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  assert!(
    app.sidebar.max_scroll > 0,
    "the fixture must overflow the section: {}",
    app.sidebar.max_scroll
  );
  app.sidebar.scroll = app.sidebar.max_scroll;
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  let text = buffer_text(&terminal);
  assert!(
    text.contains("init"),
    "at max_scroll the oldest commit must be on screen: {text}"
  );
}

/// Row index of the first line whose rendered text contains `needle`,
/// or `None`. Row-level rather than whole-buffer, so a test can assert
/// *where* a marker landed and not merely that it exists.
fn row_of(terminal: &Terminal<TestBackend>, needle: &str) -> Option<u16> {
  let buffer = terminal.backend().buffer();
  let area = buffer.area;
  (area.y..area.y + area.height).find(|&y| {
    let line: String = (area.x..area.x + area.width).map(|x| buffer[(x, y)].symbol()).collect();
    line.contains(needle)
  })
}

#[test]
fn compact_mode_gives_a_short_lists_blank_rows_to_the_sidebar() {
  // Issue #545, the "half empty demo" complaint: the stacked table pane
  // reserved 42% of the height whatever the row count, so a repo with a
  // handful of worktrees showed a column of blank rows above a sidebar
  // that was scrolling.
  //
  // Measured as *where the sidebar starts*: with one worktree the pane
  // draws three rows (header fill, column header, the row itself), so the
  // Status header must land near the top rather than at the 42% mark.
  // Counting sidebar content instead would pass on the chrome saving
  // alone and never see the pane sizing.
  let dir = repo_with_commits(4);
  let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
  assert_eq!(
    app.sidebar.resolve_layout(120),
    gwm::tui::state::sidebar::ResolvedSidebarLayout::Stacked,
    "this test measures the stacked layout"
  );
  assert_eq!(app.worktrees.len(), 1, "fixture is a single-worktree repo");
  app.config.tui.layout = TuiLayout::Compact;

  let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
  warm_sidebar(&mut app);
  terminal.draw(|f| draw(f, &mut app)).unwrap();
  terminal.draw(|f| draw(f, &mut app)).unwrap();

  let status_row = row_of(&terminal, "[2] STATUS").expect("the compact Status header must render");
  // The pane's 42% share of a 40-row terminal is ~16 rows; sized to its
  // single row it spends 3. Anything past the share means the pane is
  // still reserving space it does not draw.
  assert!(
    status_row < 10,
    "the sidebar must start right under a one-row table, got row {status_row}"
  );
}

/// Foreground colour of the first cell of `needle` on the row that
/// carries it.
fn fg_of(terminal: &Terminal<TestBackend>, needle: &str) -> Option<ratatui::style::Color> {
  let buffer = terminal.backend().buffer();
  let area = buffer.area;
  for y in area.y..area.y + area.height {
    let line: String = (area.x..area.x + area.width).map(|x| buffer[(x, y)].symbol()).collect();
    if let Some(byte_idx) = line.find(needle) {
      let col = line[..byte_idx].chars().count() as u16;
      return Some(buffer[(area.x + col, y)].fg);
    }
  }
  None
}

#[test]
fn compact_headers_carry_the_focus_signal_the_borders_used_to() {
  // Issue #545, unknown #1 — the one the issue calls the real half.
  // Without rules, the border colour has nowhere to live, so the focus
  // signal moves onto the header text. Both panes are checked in both
  // configurations because focus is exclusive: `list_has_focus` is the
  // negation of the sidebar's, so a header wired to a constant (rather
  // than to focus) would show the two agreeing in at least one of them.
  let dir = repo_with_commits(4);
  let theme = gwm::tui::theme::Theme::default();
  let headers_when = |sidebar_focused: bool| {
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    app.config.tui.layout = TuiLayout::Compact;
    app.sidebar.focused = sidebar_focused;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    warm_sidebar(&mut app);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    (
      fg_of(&terminal, "[1] WORKTREES").expect("worktrees header"),
      fg_of(&terminal, "[2] STATUS").expect("status header"),
    )
  };

  let (worktrees, status) = headers_when(true);
  assert_eq!(status, theme.focus, "focused sidebar header wears the focus role");
  assert_eq!(worktrees, theme.muted, "the unfocused pane header is muted");

  let (worktrees, status) = headers_when(false);
  assert_eq!(worktrees, theme.focus, "focus moves to the list header");
  assert_eq!(status, theme.muted, "and leaves the sidebar header muted");
}

#[test]
fn compact_mode_lets_the_sidebar_absorb_the_whole_split() {
  // Codex review, PR #546: sizing the table with `Length` next to the
  // sidebar's `Percentage(58)` does not add up to the body height, and
  // ratatui's default flex leaves the remainder as dead space *after* the
  // sidebar. The rows the pane gives back would then reach nobody — the
  // opposite of what the mode claims.
  //
  // Measured on a repo whose commit section overflows, so the sidebar has
  // content for every row it is granted: the last body line (just above
  // the footer) must be painted, in both layouts.
  let dir = repo_with_commits(40);
  let painted_last_body_row = |compact: bool| {
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    app.config.tui.layout = if compact {
      TuiLayout::Compact
    } else {
      TuiLayout::Bordered
    };
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    warm_sidebar(&mut app);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    // Screen = header row, body, footer row. The last body row is the
    // one above the footer.
    let y = area.y + area.height - 2;
    (area.x..area.x + area.width).any(|x| !buffer[(x, y)].symbol().trim().is_empty())
  };
  assert!(
    painted_last_body_row(false),
    "bordered baseline: the split already fills the body"
  );
  assert!(
    painted_last_body_row(true),
    "compact must hand the pane's unused rows to the sidebar, not to dead space"
  );
}

/// Background colour of the first cell of `needle` on the row that
/// carries it.
fn bg_of(terminal: &Terminal<TestBackend>, needle: &str) -> Option<ratatui::style::Color> {
  let buffer = terminal.backend().buffer();
  let area = buffer.area;
  for y in area.y..area.y + area.height {
    let line: String = (area.x..area.x + area.width).map(|x| buffer[(x, y)].symbol()).collect();
    if let Some(byte_idx) = line.find(needle) {
      let col = line[..byte_idx].chars().count() as u16;
      return Some(buffer[(area.x + col, y)].bg);
    }
  }
  None
}

#[test]
fn compact_header_fill_follows_the_focus_too() {
  // Validation feedback on PR #546: moving the focus signal onto the
  // header *text* alone did not read at a glance. The fill carries it as
  // well — `selection_bg` on the focused pane, `section_bg` elsewhere.
  //
  // Both roles already exist and the theme guarantees they differ
  // (`section_bg_never_collides_with_selection_bg`), so the two header
  // states are distinct on every preset without a third background role.
  let dir = repo_with_commits(4);
  let theme = gwm::tui::theme::Theme::default();
  let fills_when = |sidebar_focused: bool| {
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    app.config.tui.layout = TuiLayout::Compact;
    app.sidebar.focused = sidebar_focused;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    warm_sidebar(&mut app);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    (
      bg_of(&terminal, "[1] WORKTREES").expect("worktrees header"),
      bg_of(&terminal, "[2] STATUS").expect("status header"),
    )
  };

  let (worktrees, status) = fills_when(true);
  assert_eq!(status, theme.selection_bg, "focused sidebar header takes the loud fill");
  assert_eq!(worktrees, theme.section_bg, "the unfocused pane keeps the quiet one");

  let (worktrees, status) = fills_when(false);
  assert_eq!(worktrees, theme.selection_bg, "the fill follows focus to the list");
  assert_eq!(status, theme.section_bg, "and leaves the sidebar quiet");
}

#[test]
fn compact_side_by_side_puts_the_sidebar_on_the_asked_side() {
  // Validation feedback on PR #546: `[tui] sidebar_position` stopped
  // working in compact mode. The separator turned the two-constraint
  // split into three, and the areas are picked by index — an off-by-one
  // there silently swaps the panes or hands one a zero-width rect.
  //
  // No render test covered the side-by-side layout at all before this,
  // which is exactly why the regression shipped.
  use gwm::config::SidebarPosition;
  use gwm::tui::state::sidebar::SidebarOrientation;

  let dir = repo_with_commits(4);
  let render = |compact: bool, position: SidebarPosition| {
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    app.config.tui.layout = if compact {
      TuiLayout::Compact
    } else {
      TuiLayout::Bordered
    };
    app.sidebar.orientation = SidebarOrientation::SideBySide;
    app.sidebar.position = position;
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    warm_sidebar(&mut app);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    terminal
  };

  // Column at which the Status header starts tells which side the sidebar
  // landed on.
  let status_col = |terminal: &Terminal<TestBackend>, needle: &str| -> u16 {
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    for y in area.y..area.y + area.height {
      let line: String = (area.x..area.x + area.width).map(|x| buffer[(x, y)].symbol()).collect();
      if let Some(i) = line.find(needle) {
        return line[..i].chars().count() as u16;
      }
    }
    panic!("{needle:?} never rendered");
  };

  for compact in [false, true] {
    let needle = if compact { "[2] STATUS" } else { "[2] Status" };
    let right = status_col(&render(compact, SidebarPosition::Right), needle);
    let left = status_col(&render(compact, SidebarPosition::Left), needle);
    assert!(
      right > 60,
      "compact={compact}: sidebar on the right must start past the middle, got column {right}"
    );
    assert!(
      left < 10,
      "compact={compact}: sidebar on the left must start near column 0, got column {left}"
    );
  }
}

#[test]
fn dim_unfocused_dims_the_body_of_the_pane_without_focus() {
  // Validation feedback on PR #546, twice over: first that an inactive
  // pane of equally bright content reads as equally live, then that the
  // dimming is a matter of taste and must be opt-in. `DIM`, not a repaint
  // in `muted`, because the body's colours are semantic (a dirty branch,
  // a staged file) and flattening them would cost more than the focus
  // signal is worth.
  let dir = repo_with_commits(4);
  let dimmed_when = |sidebar_focused: bool| {
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    app.config.tui.layout = TuiLayout::Compact;
    app.config.tui.dim_unfocused = true;
    app.sidebar.focused = sidebar_focused;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    warm_sidebar(&mut app);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    // A body row of each pane: the `Branch` line belongs to the sidebar,
    // the `BRANCH` column header to the list.
    let dim_at = |needle: &str| -> bool {
      for y in area.y..area.y + area.height {
        let line: String = (area.x..area.x + area.width).map(|x| buffer[(x, y)].symbol()).collect();
        if let Some(i) = line.find(needle) {
          let col = line[..i].chars().count() as u16;
          return buffer[(area.x + col, y)]
            .modifier
            .contains(ratatui::style::Modifier::DIM);
        }
      }
      panic!("{needle:?} never rendered");
    };
    (dim_at("BRANCH"), dim_at("Branch  "))
  };

  let (list_dim, sidebar_dim) = dimmed_when(true);
  assert!(list_dim, "the unfocused list body must be dimmed");
  assert!(!sidebar_dim, "the focused sidebar body must not be");

  let (list_dim, sidebar_dim) = dimmed_when(false);
  assert!(!list_dim, "focus moved to the list, its body is bright again");
  assert!(sidebar_dim, "and the sidebar body is dimmed");
}

#[test]
fn dim_unfocused_is_off_by_default_in_both_layouts() {
  // The dimming is a trade-off, not a strict improvement: the inactive
  // pane's content is still readable information, and dimming costs
  // contrast on a surface that is often a screenshot. So it ships off,
  // and a config that never mentions it renders exactly as it did.
  let dir = repo_with_commits(4);
  for layout in [TuiLayout::Compact, TuiLayout::Bordered] {
    let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
    app.config.tui.layout = layout;
    app.sidebar.focused = true;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    warm_sidebar(&mut app);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();

    let buffer = terminal.backend().buffer();
    assert!(
      !buffer
        .content()
        .iter()
        .any(|c| c.modifier.contains(ratatui::style::Modifier::DIM)),
      "{layout:?}: nothing may be dimmed with dim_unfocused off"
    );
  }
}

#[test]
fn dim_unfocused_dims_the_unfocused_pane_in_both_layouts() {
  // The signal is about focus, not about how a pane is framed.
  //
  // The first version of this guard only asked whether *any* cell was
  // dimmed. It passed while the bordered sidebar was not dimmed at all:
  // the table applies the style in both layouts, so one dimmed cell
  // always existed and the missing half went unseen (Codex review, PR
  // #546). It now names the surface it checks, on both sides of the
  // focus, in both layouts — four assertions that cannot all hold unless
  // the style reaches every render path.
  let dir = repo_with_commits(4);
  let dim_at = |terminal: &Terminal<TestBackend>, needle: &str| -> bool {
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    for y in area.y..area.y + area.height {
      let line: String = (area.x..area.x + area.width).map(|x| buffer[(x, y)].symbol()).collect();
      if let Some(i) = line.find(needle) {
        let col = line[..i].chars().count() as u16;
        return buffer[(area.x + col, y)]
          .modifier
          .contains(ratatui::style::Modifier::DIM);
      }
    }
    panic!("{needle:?} never rendered");
  };

  for layout in [TuiLayout::Compact, TuiLayout::Bordered] {
    for sidebar_focused in [true, false] {
      let mut app = App::new_at_layered(Some(dir.path()), None).unwrap();
      app.config.tui.layout = layout;
      app.config.tui.dim_unfocused = true;
      app.sidebar.focused = sidebar_focused;
      let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
      warm_sidebar(&mut app);
      terminal.draw(|f| draw(f, &mut app)).unwrap();
      terminal.draw(|f| draw(f, &mut app)).unwrap();

      // `BRANCH` is a list column header; `Branch  ` a sidebar body row.
      assert_eq!(
        dim_at(&terminal, "BRANCH"),
        sidebar_focused,
        "{layout:?}, sidebar_focused={sidebar_focused}: the list body must be dimmed exactly when it lacks focus"
      );
      assert_eq!(
        dim_at(&terminal, "Branch  "),
        !sidebar_focused,
        "{layout:?}, sidebar_focused={sidebar_focused}: the sidebar body must be dimmed exactly when it lacks focus"
      );
    }
  }
}
