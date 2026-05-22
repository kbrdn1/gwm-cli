//! Sidebar (git preview) panel state, extracted from `tui::app::App` per
//! #127 / #102.
//!
//! Concerns:
//!
//! 1. **Visibility + focus** — `open` (toggled by `v`), `focused`
//!    (toggled by `Tab`). A closed sidebar can never be focused; the
//!    `toggle_open` invariant enforces that so `j` / `k` walks the
//!    worktree list when the panel goes away.
//!
//! 2. **Scroll offset** — `scroll` is the first-visible line index of
//!    the Recent Commits section; `max_scroll` is its upper bound,
//!    republished every frame by the renderer (`tui/ui.rs::draw_sidebar`)
//!    against the actual rendered content height. Scrolling is clamped
//!    against `max_scroll` so the user can't push the panel content
//!    entirely off-screen.
//!
//! 3. **Cache** — `cache` memoises the pre-rendered `SidebarSections`
//!    keyed by the selected worktree's path. Without it, every TUI
//!    redraw would re-shell `git log` / `git status` for the preview
//!    panel; the cache means those run only on selection change (via
//!    [`Self::on_navigation`]) or explicit invalidation (via
//!    [`Self::invalidate`], called by `App::refresh` after the
//!    worktrees list mutates).
//!
//! 4. **Navigation triple dedupe** — pre-extraction, the `App` body
//!    repeated `sidebar_scroll = 0; invalidate_sidebar_cache();
//!    refresh_link();` verbatim in `next`, `prev`, `first`, `last`,
//!    and `clamp_selection_to_filter`'s neighbours. [`Self::on_navigation`]
//!    collapses the first two pieces here; the `App` orchestrator
//!    wraps them with `refresh_link()` in a single `App::on_navigation`
//!    so the literal triple can't drift back into duplicated copies.

use crate::tui::ui::SidebarSections;
use std::path::PathBuf;

/// Pure sidebar state. Use [`Self::new`] (or the [`Default`] impl below)
/// to get the initial state that matches the previous `App::new_at`
/// behaviour (open + unfocused + zero scroll + cold cache) — the
/// `#[derive(Default)]` Copilot would normally synthesise here would
/// set `open = false`, which contradicts both the doc above and
/// `new()`. The hand-written `Default` keeps the contract single-sourced.
#[derive(Debug)]
pub struct SidebarState {
  /// `true` when the sidebar is visible. The renderer additionally
  /// hides it on narrow terminals (`area.width < SIDEBAR_MIN_WIDTH`)
  /// without flipping this flag — a wider terminal then re-shows the
  /// panel without re-toggling.
  pub open: bool,
  /// `true` when keyboard navigation (`j` / `k`) targets the sidebar
  /// (scrolling Recent Commits) instead of the worktree list.
  /// Invariant: `focused` is `false` whenever `open` is `false`.
  pub focused: bool,
  /// First-visible line index of the Recent Commits section. Bumped
  /// by [`Self::scroll_down`] / [`Self::scroll_up`]; reset to 0 by
  /// [`Self::on_navigation`].
  pub scroll: u16,
  /// Upper bound for `scroll`, republished by the renderer every
  /// frame against the actual rendered Recent Commits height. Used
  /// by [`Self::scroll_down`] to clamp so the panel content can never
  /// be pushed entirely off-screen.
  pub max_scroll: u16,
  /// Cached pre-rendered sections keyed by the selected worktree's
  /// path. `None` = cold cache (the renderer will rebuild and store).
  /// Invalidated on selection change ([`Self::on_navigation`]),
  /// worktree list mutation (`App::refresh` calls [`Self::invalidate`]),
  /// and filter narrowing (`App::filter_push_char` /
  /// `filter_pop_char`).
  pub cache: Option<(PathBuf, SidebarSections)>,
}

impl Default for SidebarState {
  fn default() -> Self {
    Self::new()
  }
}

impl SidebarState {
  pub fn new() -> Self {
    Self {
      open: true,
      focused: false,
      scroll: 0,
      max_scroll: 0,
      cache: None,
    }
  }

  /// Navigation-driven reset: drop the scroll back to the top AND
  /// invalidate the cache so the new selection's preview renders fresh.
  /// Paired with `App::refresh_link()` inside `App::on_navigation` to
  /// collapse the pre-extraction `sidebar_scroll = 0;
  /// invalidate_sidebar_cache(); refresh_link();` triple that the
  /// `App` body repeated 4+ times across `next` / `prev` / `first` /
  /// `last`.
  ///
  /// Deliberately does NOT touch `open`, `focused`, or `max_scroll`:
  /// navigation moves selection within the existing layout; visibility
  /// is a separate concern owned by the toggle methods, and
  /// `max_scroll` is owned by the renderer (a stale value resets
  /// itself on the next frame anyway).
  pub fn on_navigation(&mut self) {
    self.scroll = 0;
    self.cache = None;
  }

  /// Standalone cache flush. Used outside the navigation path —
  /// `App::refresh` after the worktrees list mutates, and the filter
  /// `push_char` / `pop_char` wrappers that re-narrow the visible set
  /// without moving the cursor. Scroll state survives so a user
  /// scrolled halfway through the preview keeps their viewport.
  pub fn invalidate(&mut self) {
    self.cache = None;
  }

  /// Scroll the Recent Commits viewport down by one line, clamped at
  /// `max_scroll`. The clamp is the load-bearing invariant — without
  /// it, `j` on a focused sidebar would walk the content entirely off
  /// the bottom of the panel.
  pub fn scroll_down(&mut self) {
    self.scroll = self.scroll.saturating_add(1).min(self.max_scroll);
  }

  /// Scroll the Recent Commits viewport up by one line, saturating at
  /// 0. Matches `k`-on-sidebar; safe to call from `scroll == 0`.
  pub fn scroll_up(&mut self) {
    self.scroll = self.scroll.saturating_sub(1);
  }

  /// Flip `open`. When closing, also drops `focused` — a hidden
  /// sidebar can never hold the navigation focus, so the worktree
  /// list takes back `j` / `k` automatically. Status-bar copy is the
  /// `App` orchestrator's concern.
  pub fn toggle_open(&mut self) {
    self.open = !self.open;
    if !self.open {
      self.focused = false;
    }
  }

  /// Flip `focused`. No-op when the sidebar is closed — focus cannot
  /// move to a hidden panel. Matches the `Tab` keybinding semantics.
  pub fn toggle_focus(&mut self) {
    if !self.open {
      return;
    }
    self.focused = !self.focused;
  }
}
