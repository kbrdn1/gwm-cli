//! Full-size commit listing overlay state (issue #593).
//!
//! The sidebar's Commits pane shares the sidebar with the identity, status
//! and working-tree blocks, and it is capped at
//! [`crate::tui::RECENT_COMMITS_LIMIT`] rows. This is the same graph
//! renderer given the whole terminal, plus a way to ask for more history
//! instead of leaving gwm for lazygit.
//!
//! The rows are an owned snapshot taken when the overlay opens
//! ([`crate::tui::App::enter_commits`]) rather than a read of
//! `SidebarState::cache`: that cache is only rebuilt while the sidebar is
//! *open* and in `Commits` mode, so reading it would leave the overlay
//! blank in exactly the two states, sidebar hidden or `Stashes` selected,
//! where the listing is most useful.
//!
//! The read itself runs on a worker (`TaskKind::Commits`), not inline on
//! the keypress. The revwalk sorts `TIME | TOPOLOGICAL`, which walks the
//! whole reachable graph before it yields the first row: measured on this
//! repo (2058 commits), asking for 300 costs the same as asking for all of
//! them, so the limit truncates the output and bounds nothing about the
//! latency. On a large history an inline call would freeze the event loop
//! for the length of the walk. The overlay opens on a loader and fills in
//! when the worker lands, the shape #592 settled on for the same reason.
//!
//! Paging is a re-read at a larger limit rather than an append.
//! [`crate::worktree::recent_commits_cached`] is keyed by
//! `(repo, tip, limit)`, so each page is a fresh entry rather than an
//! invalidation, and the graph renderer needs the whole list anyway: a
//! connector on row N depends on the parents of rows below it.
//!
//! Scroll follows the help / command-logs contract: the cursor lives here,
//! but `max_scroll` is republished by the renderer each frame against the
//! real viewport, since only the renderer knows both the row count and the
//! inner modal height.

use super::super::ui::RECENT_COMMITS_LIMIT;
use ratatui::text::Line;
use std::path::{Path, PathBuf};

/// One page of history. Matched to the sidebar's own limit so the first
/// snapshot hits the cache entry a sidebar in Commits mode already warmed
/// instead of paying a second revwalk.
pub const COMMITS_PAGE: usize = RECENT_COMMITS_LIMIT;

/// Ceiling on the paged limit, in commits.
///
/// Two reasons to have one at all. Each page is a whole graph walk, so a
/// deeper page is not cheaper than the one before it. And the memo in
/// [`crate::worktree::recent_commits_cached`] holds
/// `RECENT_COMMITS_CACHE_MAX_ENTRIES` (64) entries keyed on the limit,
/// evicting an arbitrary one when full: unbounded paging would push other
/// worktrees' sidebar entries out and make them re-walk. Five pages per
/// worktree keeps that well inside the budget.
pub const COMMITS_MAX: usize = COMMITS_PAGE * 5;

/// Owned state for the full-size commit listing: the snapshotted graph
/// rows, the limit they were read at, and the scroll cursor with its
/// renderer-published bound.
#[derive(Debug, Default)]
pub struct CommitsModal {
  /// The commit rows, exactly as the sidebar pane paints them (short hash,
  /// author initials, `○` / `◎` graph, subject) — or the `(no commits)`
  /// row, or a load error.
  pub lines: Vec<Line<'static>>,
  /// The limit [`Self::lines`] was read at.
  pub limit: usize,
  /// Rows actually returned. `recent_commits_lines` paints exactly one row
  /// per commit, so this IS the commit count; the `(no commits)` and error
  /// sentinels are a single row, far under a page, which reads as an
  /// exhausted history and correctly disables paging on them.
  pub loaded: usize,
  /// Vertical scroll offset, in rows. Clamped to `max_scroll`.
  pub scroll: u16,
  /// Maximum vertical scroll offset, republished by the renderer each
  /// frame as `content_rows.saturating_sub(viewport_rows)`.
  pub max_scroll: u16,
  /// `true` between the request and the worker's payload. The renderer
  /// paints a loader rather than an empty canvas, which would read as "no
  /// commits".
  pub loading: bool,
  /// The worktree [`Self::lines`] describe, so a payload for a selection the
  /// user has navigated away from can be dropped instead of shown.
  pub path: Option<PathBuf>,
}

impl CommitsModal {
  /// An empty overlay at the origin.
  pub fn new() -> Self {
    Self::default()
  }

  /// Arm the overlay for `path` at the first page: drop the previous
  /// listing, rewind the scroll, and show the loader until [`Self::load`]
  /// lands. Called on every open, so a previously-scrolled visit starts
  /// fresh and a stale listing is never mistaken for the current one.
  pub fn begin(&mut self, path: Option<&Path>, limit: usize) {
    self.lines.clear();
    self.loaded = 0;
    self.limit = limit;
    self.scroll = 0;
    self.max_scroll = 0;
    self.path = path.map(Path::to_path_buf);
    // With nothing selected there is nothing to wait for.
    self.loading = path.is_some();
  }

  /// Arm a deeper page, **keeping** the rows and the scroll cursor on
  /// screen while the worker runs. The user pressed load-more from the
  /// bottom of the list: blanking the canvas there would throw away both
  /// the page they were reading and the position they paged from.
  pub fn begin_more(&mut self, limit: usize) {
    self.limit = limit;
    self.loading = true;
  }

  /// Install the worker's payload and clear the loader. The scroll cursor
  /// is whatever [`Self::begin`] (top) or [`Self::begin_more`] (unchanged)
  /// left; the renderer re-clamps it against the new content.
  pub fn load(&mut self, lines: Vec<Line<'static>>) {
    self.loaded = lines.len();
    self.lines = lines;
    self.loading = false;
  }

  /// Whether a deeper page exists and is allowed.
  ///
  /// `loaded < limit` means the revwalk ran out of history before the limit
  /// did, so there is nothing deeper to fetch however high the limit goes.
  /// A read already in flight also blocks: `loaded` still describes the
  /// previous page, so a second load-more would otherwise queue a duplicate
  /// walk on the same keypress-repeat.
  pub fn can_load_more(&self) -> bool {
    !self.loading && self.loaded >= self.limit && self.limit < COMMITS_MAX
  }

  /// The limit the next page reads at, clamped to [`COMMITS_MAX`].
  pub fn next_limit(&self) -> usize {
    (self.limit + COMMITS_PAGE).min(COMMITS_MAX)
  }

  /// Scroll down one row, never past the last line.
  pub fn scroll_down(&mut self) {
    self.scroll = (self.scroll + 1).min(self.max_scroll);
  }

  /// Scroll up one row, never above the top.
  pub fn scroll_up(&mut self) {
    self.scroll = self.scroll.saturating_sub(1);
  }

  /// Jump to the first row (`g`).
  pub fn scroll_to_top(&mut self) {
    self.scroll = 0;
  }

  /// Jump to the last row (`G`).
  pub fn scroll_to_bottom(&mut self) {
    self.scroll = self.max_scroll;
  }
}
