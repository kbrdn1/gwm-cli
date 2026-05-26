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

/// Which content the sidebar previews (issue #34).
///
/// Toggled with the `s` key in the list view, dispatched through
/// `Action::ToggleSidebarMode` in the rebindable keymap. Default is
/// `Commits` so the pre-#34 sidebar behaviour is preserved verbatim.
/// The mode is per-session — not persisted across `gwm` launches —
/// because the low-frequency need to view stashes does not justify a
/// new `.gwm.toml` knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SidebarMode {
  /// `git log --oneline -n 10` + `git status --short`. Pre-#34
  /// behaviour, kept as the default so existing users see no change
  /// until they press `s`.
  Commits,
  /// `git stash list` + a per-stash quick view. New in #34.
  Stashes,
}

impl SidebarMode {
  /// Human-readable label rendered into the sidebar title bar
  /// (` Details — commits ` vs. ` Details — stashes `).
  pub fn label(self) -> &'static str {
    match self {
      SidebarMode::Commits => "commits",
      SidebarMode::Stashes => "stashes",
    }
  }
}

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
  /// path **and** the active mode (issue #34). `None` = cold cache
  /// (the renderer will rebuild and store). Invalidated on selection
  /// change ([`Self::on_navigation`]), worktree list mutation
  /// (`App::refresh` calls [`Self::invalidate`]), filter narrowing
  /// (`App::filter_push_char` / `filter_pop_char`), and mode toggle
  /// ([`Self::cycle_mode`]). Two-tuple key so a re-toggle re-shells
  /// `git stash list` / `git log` rather than serving stale content
  /// for the other mode.
  pub cache: Option<((PathBuf, SidebarMode), SidebarSections)>,
  /// Active preview mode. Defaults to [`SidebarMode::Commits`] so the
  /// pre-#34 sidebar behaviour is unchanged until the user presses
  /// `s`. Toggled by [`Self::cycle_mode`].
  pub mode: SidebarMode,
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
      mode: SidebarMode::Commits,
    }
  }

  /// Cycle the preview mode (issue #34). Pre-#34 the sidebar only
  /// ever showed `git log` + `git status`; now `s` flips between
  /// `Commits` and `Stashes`. The scroll offset resets to 0 because
  /// the new content has its own length and the previous offset
  /// becomes meaningless. The cache is invalidated because the key
  /// (path + mode) changes — the new mode re-shells the right git
  /// command on the next frame.
  pub fn cycle_mode(&mut self) {
    self.mode = match self.mode {
      SidebarMode::Commits => SidebarMode::Stashes,
      SidebarMode::Stashes => SidebarMode::Commits,
    };
    self.scroll = 0;
    self.cache = None;
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
