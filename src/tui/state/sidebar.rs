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

/// Re-exported from [`crate::config`], where both sidebar knobs live now
/// that the orientation is persisted to `.gwm.toml` alongside the
/// position (issue #365). Kept re-exported here so the state module
/// still reads as the owner of the sidebar contract.
pub use crate::config::{SidebarOrientation, SidebarPosition};

/// Minimum total terminal width (in columns) required to render the
/// sidebar *beside* the worktree table without squeezing the table
/// beyond readability. At or above this width the `Auto` orientation
/// picks the side-by-side split; below it, `Auto` stacks the sidebar
/// under the table (issue #188) rather than hiding it (pre-#188).
pub const SIDEBAR_MIN_WIDTH: u16 = 120;

/// The concrete layout the renderer should draw for the current frame,
/// resolved from `open` + orientation + position + terminal width by
/// [`SidebarState::resolve_layout`]. Kept ratatui-free so the decision
/// is unit-testable against the width contract without a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedSidebarLayout {
  /// Sidebar closed — draw the worktree table full-area.
  Hidden,
  /// Side-by-side split. `sidebar_left` mirrors [`SidebarPosition`]:
  /// `true` draws the sidebar on the left of the table, `false` on the
  /// right.
  SideBySide { sidebar_left: bool },
  /// Stacked split — table on top, sidebar below.
  Stacked,
}

impl ResolvedSidebarLayout {
  /// The `(table_pct, sidebar_pct)` split this layout draws, or `None`
  /// when the sidebar is hidden (the table takes the whole area). Issue
  /// #217 tuned the ratios per axis: stacked vertically the status pane
  /// gets the larger share (42% table / 58% status) so commits + issue/PR
  /// have room; side-by-side the table stays dominant (55% / 45%). Pure +
  /// ratatui-free so the contract is pinned without a backend.
  pub fn split_percentages(self) -> Option<(u16, u16)> {
    match self {
      ResolvedSidebarLayout::Hidden => None,
      ResolvedSidebarLayout::SideBySide { .. } => Some((55, 45)),
      ResolvedSidebarLayout::Stacked => Some((42, 58)),
    }
  }
}

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

/// Split the sidebar height left over for the three variable sections —
/// Agents / Working Tree / Recent Commits — into per-section block
/// heights, borders included (issue #438). Pure and ratatui-free so the
/// policy is unit-testable without a backend, like [`SidebarState::resolve_layout`].
///
/// - When every natural height (`content + 2` borders) fits, each section
///   keeps it and Recent Commits absorbs the slack — the exact behaviour
///   the old `Min(3)` constraint produced.
/// - On overflow, every **visible** section is guaranteed
///   `min(natural, 5)` lines (border + at least 3 content rows) and the
///   surplus is distributed proportionally to content size, capped at the
///   natural height; the integer-division residue cascades to Recent
///   Commits, then Working Tree, then Agents. The total need exceeds the
///   surplus by construction, so the residue is always absorbed.
/// - A section with no content stays collapsed at 0 (Agents with no
///   session, Working Tree on a clean tree) and never eats a floor.
///   Recent Commits is never collapsed: an empty history still renders
///   its bordered panel, floored at the historical `Min(3)`.
/// - Below the floors' sum (tiny terminal) sections are served in the
///   order commits → working tree → agents with whatever remains.
pub fn split_section_heights(available: u16, agents_len: u16, wt_len: u16, commits_len: u16) -> (u16, u16, u16) {
  let natural = |len: u16| if len == 0 { 0 } else { len.saturating_add(2) };
  let natural_a = natural(agents_len);
  let natural_w = natural(wt_len);
  let natural_c = commits_len.saturating_add(2);

  if natural_a as u32 + natural_w as u32 + natural_c as u32 <= available as u32 {
    return (natural_a, natural_w, available - natural_a - natural_w);
  }

  let floor_a = natural_a.min(5);
  let floor_w = natural_w.min(5);
  let floor_c = natural_c.clamp(3, 5);

  let base = floor_a + floor_w + floor_c;
  if base > available {
    let c = floor_c.min(available);
    let rest = available - c;
    let w = floor_w.min(rest);
    let a = floor_a.min(rest - w);
    return (a, w, c);
  }

  let surplus = available - base;
  let total = agents_len as u32 + wt_len as u32 + commits_len as u32;
  let give = |len: u16, floor_v: u16, nat: u16| -> u16 {
    if total == 0 {
      return 0;
    }
    ((surplus as u32 * len as u32 / total) as u16).min(nat - floor_v)
  };
  let mut a = floor_a + give(agents_len, floor_a, natural_a);
  let mut w = floor_w + give(wt_len, floor_w, natural_w);
  let mut c = floor_c + give(commits_len, floor_c, natural_c);
  let mut residue = available - a - w - c;
  let mut top_up = |h: &mut u16, nat: u16| {
    let room = (nat - *h).min(residue);
    *h += room;
    residue -= room;
  };
  top_up(&mut c, natural_c);
  top_up(&mut w, natural_w);
  top_up(&mut a, natural_a);
  (a, w, c)
}

/// Pure sidebar state. Use [`Self::new`] (or the [`Default`] impl below)
/// to get the initial state that matches the previous `App::new_at`
/// behaviour (open + unfocused + zero scroll + cold cache) — the
/// `#[derive(Default)]` Copilot would normally synthesise here would
/// set `open = false`, which contradicts both the doc above and
/// `new()`. The hand-written `Default` keeps the contract single-sourced.
#[derive(Debug)]
pub struct SidebarState {
  /// `true` when the sidebar is visible. On a narrow terminal the
  /// renderer no longer hides it (pre-#188 behaviour) but stacks it
  /// under the table instead — see [`Self::resolve_layout`]. Closing
  /// the panel (`open = false`) is the only way to reclaim the full
  /// width for the table.
  pub open: bool,
  /// Which side the sidebar sits on in the side-by-side layout
  /// (issue #188). Seeded from `[tui] sidebar_position` at `App`
  /// construction, toggled live by [`Self::toggle_position`] (`H`).
  /// Ignored by the stacked layout (sidebar always at the bottom).
  pub position: SidebarPosition,
  /// How the sidebar is arranged relative to the table (issue #188).
  /// Defaults to [`SidebarOrientation::Stacked`] since #217 — *not*
  /// `Auto`, which this comment claimed until #365. Seeded from
  /// `[tui] sidebar_orientation` at `App` construction and re-seeded on
  /// config reload, exactly like `position`; cycled live by
  /// [`Self::cycle_orientation`].
  pub orientation: SidebarOrientation,
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
  /// First-visible line index of the Working Tree section (issue
  /// #437). Independent from `scroll` — the file tree and the commit
  /// list overflow at different rates. Bumped by
  /// [`Self::wt_scroll_down`] / [`Self::wt_scroll_up`]; reset to 0 by
  /// [`Self::on_navigation`] and [`Self::cycle_mode`].
  pub wt_scroll: u16,
  /// Upper bound for `wt_scroll`, republished by the renderer every
  /// frame against the Working Tree section's clamped viewport (the
  /// layout solver may hand the section less height than its content
  /// on a large change set — exactly the case #437 exists for).
  pub wt_max_scroll: u16,
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
      position: SidebarPosition::default(),
      orientation: SidebarOrientation::default(),
      focused: false,
      scroll: 0,
      max_scroll: 0,
      wt_scroll: 0,
      wt_max_scroll: 0,
      cache: None,
      mode: SidebarMode::Commits,
    }
  }

  /// Resolve the concrete layout for a frame of `width` columns from
  /// the current visibility, orientation, and position. Pure and
  /// ratatui-free so the width contract is unit-testable:
  ///
  /// - closed → [`ResolvedSidebarLayout::Hidden`];
  /// - `Auto` → side-by-side at `width >= SIDEBAR_MIN_WIDTH`, else
  ///   stacked;
  /// - `SideBySide` / `Stacked` → that layout regardless of width.
  ///
  /// In a side-by-side result `sidebar_left` mirrors [`Self::position`].
  pub fn resolve_layout(&self, width: u16) -> ResolvedSidebarLayout {
    if !self.open {
      return ResolvedSidebarLayout::Hidden;
    }
    let side_by_side = ResolvedSidebarLayout::SideBySide {
      sidebar_left: self.position.is_left(),
    };
    match self.orientation {
      SidebarOrientation::SideBySide => side_by_side,
      SidebarOrientation::Stacked => ResolvedSidebarLayout::Stacked,
      SidebarOrientation::Auto => {
        if width >= SIDEBAR_MIN_WIDTH {
          side_by_side
        } else {
          ResolvedSidebarLayout::Stacked
        }
      }
    }
  }

  /// Cycle the orientation `Auto → SideBySide → Stacked → Auto`
  /// (issue #188, `V`). The cache survives — orientation changes the
  /// frame geometry, not the previewed git content.
  pub fn cycle_orientation(&mut self) {
    self.orientation = self.orientation.next();
  }

  /// Flip the side-by-side position left ↔ right (issue #188, `H`).
  /// The cache survives for the same reason as [`Self::cycle_orientation`].
  pub fn toggle_position(&mut self) {
    self.position = match self.position {
      SidebarPosition::Left => SidebarPosition::Right,
      SidebarPosition::Right => SidebarPosition::Left,
    };
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
    self.wt_scroll = 0;
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
    self.wt_scroll = 0;
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

  /// Scroll the Working Tree viewport down by one line, clamped at
  /// `wt_max_scroll` (issue #437). Same clamp invariant as
  /// [`Self::scroll_down`], applied to the file-tree section.
  pub fn wt_scroll_down(&mut self) {
    self.wt_scroll = self.wt_scroll.saturating_add(1).min(self.wt_max_scroll);
  }

  /// Scroll the Working Tree viewport up by one line, saturating at 0
  /// (issue #437).
  pub fn wt_scroll_up(&mut self) {
    self.wt_scroll = self.wt_scroll.saturating_sub(1);
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

  /// Direct-focus the worktree table (issue #217, `1`). Releases the
  /// sidebar's navigation focus so `j` / `k` walk the worktree list. The
  /// sidebar stays open — `1` is about *where the cursor is*, not
  /// visibility (that's `v` / [`Self::toggle_open`]).
  pub fn focus_table(&mut self) {
    self.focused = false;
  }

  /// Direct-focus the status (sidebar) pane (issue #217, `2`). Opens the
  /// sidebar if it was closed and moves the navigation focus onto it, so a
  /// single keystroke both reveals and targets the pane.
  pub fn focus_panel(&mut self) {
    self.open = true;
    self.focused = true;
  }
}
