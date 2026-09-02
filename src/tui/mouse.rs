//! Mouse hit-testing (issue #624).
//!
//! gwm has enabled `EnableMouseCapture` since the first TUI frame and never
//! read a single mouse event, which is strictly worse than not supporting the
//! mouse: capture takes the terminal's own text selection away and nothing was
//! bought with it. This module is the half that was missing.
//!
//! # Geometry is published, never re-derived
//!
//! The layout is recomputed from scratch on every frame — a resize, a sidebar
//! toggle, a filter keystroke and a compact/bordered switch all move every
//! rect. So the click targets are not re-derived at click time from what `App`
//! knows; they are *published by the renderer* into a [`MouseMap`] as it draws,
//! and read back by [`MouseMap::hit`]. The arithmetic that places a glyph is
//! the arithmetic that reports where it landed, so the two cannot drift.
//!
//! The map is cleared at the top of `ui::draw`, which is what makes the whole
//! thing safe: a surface that is not on screen published no zone this frame and
//! therefore cannot be hit. There is no `view`-dependent branching in `hit`,
//! and a modal that covers the header simply publishes over it.
//!
//! Zones are walked in **reverse** publication order, so the last thing drawn
//! is the first thing hit — the same rule the screen itself follows.

use super::state::config_panel::SettingsTab;
use super::wt_tree::WtCategory;
use ratatui::layout::Rect;

/// The three mouse gestures gwm acts on.
///
/// Deliberately not `crossterm::event::MouseEventKind`: `EnableMouseCapture`
/// turns on any-event tracking (`?1003h`), so the terminal reports a motion
/// event per cell the cursor crosses. The event loop narrows the stream to
/// these three before anything else runs, and the state transitions below stay
/// testable without building a crossterm event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
  /// Left button pressed.
  Click,
  /// Wheel rolled away from the user.
  WheelUp,
  /// Wheel rolled towards the user.
  WheelDown,
}

/// A list whose rows are individually addressable — clicking one selects it,
/// and the wheel over it moves the selection.
///
/// Only surfaces with a *cursor* appear here. A modal that merely scrolls
/// (Keybindings, Command Logs, Commits, the Working Tree listing) publishes a
/// [`PaneId::Modal`] instead: it has no row to select, so a click on it is a
/// no-op and the wheel scrolls the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowList {
  /// The worktree table in the list view. Indices are into the *filtered*
  /// view, which is what `list_state` is indexed by.
  Worktrees,
  /// The Settings panel body (issue #232).
  Config,
  /// The exec profile picker (issue #325).
  ExecPicker,
  /// The clean reclaim profile picker (issue #325).
  CleanPicker,
  /// The browse-links menu.
  OpenMenu,
  /// The command palette candidate list (issue #32).
  Palette,
  /// The generic detail overlay — agent sessions and the rich PR/issue view
  /// (issues #408 / #420).
  Detail,
}

/// A focusable or scrollable region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneId {
  /// The worktrees pane, header and body alike.
  Worktrees,
  /// The details sidebar.
  Status,
  /// The sidebar's Working Tree sub-pane, which scrolls on its own axis
  /// (`J` / `K`, issue #437) rather than with the sidebar body.
  WorkingTree,
  /// The body of whichever scroll-only modal is open.
  Modal,
}

/// A point-like target: a click fires it, the wheel ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spot {
  /// The `▤` header affordance — opens the Command Logs panel, same as `3`.
  CommandLogs,
  /// The `⚙` header affordance — opens the Settings panel, same as `4`.
  Settings,
  /// One tab of the Settings panel's tab strip.
  ConfigTab(SettingsTab),
  /// One letter of the Working Tree counts footer — scrolls the pane to the
  /// first row of that category.
  WtCategory(WtCategory),
}

/// What a click at a given cell resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
  /// Row `index` of `list`.
  Row { list: RowList, index: usize },
  /// Anywhere in a pane that carries no finer target.
  Pane(PaneId),
  /// A point target.
  Spot(Spot),
}

/// One published region.
#[derive(Debug, Clone)]
enum Zone {
  Rows {
    list: RowList,
    /// Index of the item drawn on `rect`'s first row.
    offset: usize,
    /// Number of items; a click past the last one resolves to nothing rather
    /// than to a row that is not there.
    len: usize,
    /// Item index per *body line*, for a listing whose lines are not one per
    /// item (section rules, blank spacers). Indexed absolutely — line
    /// `offset + n` for the row at `rect.y + n` — so scrolling needs no
    /// second translation. `None` means the listing is one line per item.
    map: Option<Vec<Option<usize>>>,
  },
  Pane(PaneId),
  Spot(Spot),
}

/// The click targets of the frame currently on screen.
#[derive(Debug, Clone, Default)]
pub struct MouseMap {
  zones: Vec<(Rect, Zone)>,
}

impl MouseMap {
  /// An empty map — no zone published, every cell resolves to nothing.
  pub fn new() -> Self {
    Self::default()
  }

  /// Drop everything published for the previous frame. Called once at the top
  /// of `ui::draw`; every zone below is republished by the renderer that owns
  /// it, so a surface that stopped being drawn stops being clickable.
  pub fn clear(&mut self) {
    self.zones.clear();
  }

  /// Publish a strip of rows, one line per item.
  ///
  /// `offset` is the index of the item drawn on `rect`'s **first** row, which
  /// for a scrolled surface is not zero. For the worktree table it is
  /// `TableState::offset()`, and that is only correct once ratatui has
  /// rendered the frame — the render call is what updates it to bring the
  /// selection into view — so the table publishes *after* its
  /// `render_stateful_widget`, not before.
  pub fn push_rows(&mut self, rect: Rect, list: RowList, offset: usize, len: usize) {
    self.zones.push((
      rect,
      Zone::Rows {
        list,
        offset,
        len,
        map: None,
      },
    ));
  }

  /// Publish a strip of rows whose lines do not map one-to-one onto items.
  ///
  /// `map` is indexed by absolute body line: `map[i]` is the item drawn on
  /// line `i`, or `None` for a line that is not an item (a section rule, a
  /// blank spacer). Built by the same pass that builds the lines, for the
  /// reason the module header gives.
  pub fn push_mapped_rows(&mut self, rect: Rect, list: RowList, offset: usize, len: usize, map: Vec<Option<usize>>) {
    self.zones.push((
      rect,
      Zone::Rows {
        list,
        offset,
        len,
        map: Some(map),
      },
    ));
  }

  /// Publish a focusable / scrollable pane.
  pub fn push_pane(&mut self, rect: Rect, pane: PaneId) {
    self.zones.push((rect, Zone::Pane(pane)));
  }

  /// Publish a point target. `rect` is usually one row tall.
  pub fn push_spot(&mut self, rect: Rect, spot: Spot) {
    self.zones.push((rect, Zone::Spot(spot)));
  }

  /// Resolve a terminal cell to a target, or `None` when the cell carries
  /// none.
  ///
  /// Walks in reverse publication order: the renderer draws back to front, so
  /// the last zone published is the one the user can actually see at that
  /// cell.
  pub fn hit(&self, col: u16, row: u16) -> Option<Hit> {
    for (rect, zone) in self.zones.iter().rev() {
      if !contains(*rect, col, row) {
        continue;
      }
      return match zone {
        Zone::Pane(p) => Some(Hit::Pane(*p)),
        Zone::Spot(s) => Some(Hit::Spot(*s)),
        Zone::Rows { list, offset, len, map } => {
          let line = offset + (row - rect.y) as usize;
          let index = match map {
            Some(m) => (*m.get(line)?)?,
            None => line,
          };
          (index < *len).then_some(Hit::Row { list: *list, index })
        }
      };
    }
    None
  }

  /// Whether anything at all was published — `false` before the first frame,
  /// which is the one window where a mouse event can arrive with no geometry
  /// behind it.
  pub fn is_empty(&self) -> bool {
    self.zones.is_empty()
  }
}

/// `Rect::contains` takes a `Position`, which pulls ratatui's cursor type into
/// every call site for a two-comparison test. Zero-area rects answer `false`,
/// which matters: a modal collapsed to nothing must not swallow clicks.
fn contains(rect: Rect, col: u16, row: u16) -> bool {
  col >= rect.x && col < rect.x.saturating_add(rect.width) && row >= rect.y && row < rect.y.saturating_add(rect.height)
}
