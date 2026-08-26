//! Full-size Working Tree overlay state (issue #592).
//!
//! The sidebar's Working Tree pane is a fraction of the screen shared with
//! four other blocks, so a worktree with more than a handful of changed
//! files can only be read two rows at a time through `J` / `K`. This is the
//! same listing given the whole terminal.
//!
//! The rows are an owned snapshot requested when the overlay opens
//! ([`crate::tui::App::enter_working_tree`]) rather than a read of
//! `SidebarState::cache`: that cache is keyed by `(path, mode)` and is only
//! rebuilt while the sidebar is *open* and in `Commits` mode, so reading it
//! would leave the overlay blank in exactly the two states — sidebar hidden,
//! or `Stashes` selected — where seeing the change set is most useful.
//!
//! The read itself runs on a worker (`TaskKind::WorkingTree`), not inline on
//! the keypress. `STATUS_SCAN_CAP` bounds how many records `git status`
//! yields, not how long git takes to reach the first one, so on a cold or
//! network filesystem an inline call would freeze the event loop for the
//! length of the walk. The overlay opens on `loading` and fills in when the
//! worker lands (Copilot review, PR #612).
//!
//! Scroll follows the help / command-logs contract: the cursor lives here,
//! but `max_scroll` is republished by the renderer each frame against the
//! real viewport, since only the renderer knows both the row count and the
//! inner modal height.

use super::super::ui::WorkingTreeCounts;
use ratatui::text::Line;
use std::path::{Path, PathBuf};

/// Owned state for the full-size Working Tree overlay: the snapshotted
/// file-explorer rows, their per-category counts, and the scroll cursor
/// with its renderer-published bound.
#[derive(Debug, Default)]
pub struct WorkingTreeModal {
  /// The tree rows, exactly as the sidebar pane paints them (nerd-font
  /// icons, connector prefixes, per-category colour) — or the `✓ clean`
  /// row, or a load error.
  pub lines: Vec<Line<'static>>,
  /// Created / modified / deleted counts for the footer (issue #287),
  /// captured with the same `git status` read as [`Self::lines`].
  pub counts: WorkingTreeCounts,
  /// Vertical scroll offset, in rows. Clamped to `max_scroll`.
  pub scroll: u16,
  /// Maximum vertical scroll offset, republished by the renderer each
  /// frame as `content_rows.saturating_sub(viewport_rows)`.
  pub max_scroll: u16,
  /// `true` between the open and the worker's payload. The renderer paints
  /// a loader rather than an empty canvas, which would read as "no changes".
  pub loading: bool,
  /// The worktree [`Self::lines`] describe, so a payload for a selection the
  /// user has navigated away from can be dropped instead of shown.
  pub path: Option<PathBuf>,
}

impl WorkingTreeModal {
  /// An empty overlay at the origin.
  pub fn new() -> Self {
    Self::default()
  }

  /// Arm the overlay for `path`: drop the previous listing, rewind the
  /// scroll, and show the loader until [`Self::load`] lands. Called on every
  /// open, so a previously-scrolled visit starts fresh and a stale change
  /// set is never mistaken for the current one.
  pub fn begin(&mut self, path: Option<&Path>) {
    self.lines.clear();
    self.counts = WorkingTreeCounts::default();
    self.scroll = 0;
    self.max_scroll = 0;
    self.path = path.map(Path::to_path_buf);
    // With nothing selected there is nothing to wait for.
    self.loading = path.is_some();
  }

  /// Install the worker's payload and clear the loader.
  pub fn load(&mut self, lines: Vec<Line<'static>>, counts: WorkingTreeCounts) {
    self.lines = lines;
    self.counts = counts;
    self.scroll = 0;
    self.loading = false;
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
