use super::app::{App, GitHubFetchState, LinkPromptStage, LinkTarget, View};
use super::keymap::{Action, KeyStroke, Keymap};
use super::modal_keymap::{KeyContext, ModalAction, ModalKeymap};
use super::state::async_task::TaskKind;
use super::state::config_panel::{FieldKind, SettingField, SettingsTab};
use super::state::confirm::ConfirmButton;
use super::state::create_form::{Field, Mode};

/// The field set of the canonical `<type>/#<issue>-<desc>` triple, used as the
/// default by the hint helpers that have no form in reach (issue #418). Every
/// row those helpers can drop is present in it, so the default is "advertise
/// everything", which is what they did before the field set became dynamic.
const CANONICAL_TRIPLE: [Field; 3] = [Field::Type, Field::Issue, Field::Desc];
use super::state::pty_overlay::PtyKind;
use super::state::sidebar::SidebarMode;
use super::state::spinner::DOT_FRAMES;
use super::theme::Theme;
use super::wt_tree::{self, working_tree_category, WtCategory, WtNode, WT_DIR_OPEN_ICON};
use crate::bootstrap::{BootstrapReport, StepStatus};
use crate::command_log::CommandStatus;
use crate::config::ConfigSource;
use crate::github::{CiState, IssueState, LinkSource, PrState};
use crate::worktree::{self, BranchStatus, WorktreeInfo};
use ratatui::{
  buffer::Buffer,
  layout::{Alignment, Constraint, Direction, Layout, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span},
  widgets::{
    Block, BorderType, Borders, Cell, Clear, Padding, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Table, Widget, Wrap,
  },
  Frame,
};
use std::time::{Duration, Instant};

/// Per-section content of the worktree details sidebar. Rendered by
/// [`draw_sidebar`] into separate rounded-border blocks (no outer
/// `Details` frame, so each section reads as an independent card).
///
/// The Issue / PR section is intentionally absent here: it depends on
/// live `App` fetch state and is built per-frame via
/// [`github_status_lines`], not cached on the worktree.
#[derive(Debug, Clone, Default)]
pub struct SidebarSections {
  /// Compact identity block: name (bold), `branch · head`, badges
  /// (`✓ synced` / `● dirty` / `↑N` / `↓M` plus optional `★ main`,
  /// `🔒 locked`, `⚠ prunable`), tilde-compressed path.
  pub worktree: Vec<Line<'static>>,
  /// `git status --short` lines, or `✓ clean`, or a load error.
  pub working_tree: Vec<Line<'static>>,
  /// Per-category counts of changed files (issue #287): created / modified
  /// / deleted, driving the colour-coded nerdfont footer of the Working
  /// Tree pane.
  pub working_tree_counts: WorkingTreeCounts,
  /// Up to 10 oneline commits, or an empty / error notice.
  pub recent_commits: Vec<Line<'static>>,
}

/// Reusable one-line loader for dedicated panel/modal areas (issue #257).
#[derive(Debug, Clone, Copy)]
pub enum LoaderWidgetState<'a> {
  Running {
    glyph: &'a str,
    label: &'a str,
    detail: Option<&'a str>,
  },
  Failed {
    message: &'a str,
    detail: Option<&'a str>,
  },
}

#[derive(Debug, Clone, Copy)]
pub struct LoaderWidget<'a> {
  state: LoaderWidgetState<'a>,
  accent: Color,
  text: Color,
  muted: Color,
  failed: Color,
  alignment: Alignment,
}

impl<'a> LoaderWidget<'a> {
  pub fn running(glyph: &'a str, label: &'a str, detail: Option<&'a str>, theme: &Theme) -> Self {
    Self {
      state: LoaderWidgetState::Running { glyph, label, detail },
      accent: theme.accent,
      text: theme.name,
      muted: theme.muted,
      failed: theme.prunable,
      alignment: Alignment::Left,
    }
  }

  pub fn failed(message: &'a str, detail: Option<&'a str>, theme: &Theme) -> Self {
    Self {
      state: LoaderWidgetState::Failed { message, detail },
      accent: theme.accent,
      text: theme.name,
      muted: theme.muted,
      failed: theme.prunable,
      alignment: Alignment::Left,
    }
  }

  pub fn alignment(mut self, alignment: Alignment) -> Self {
    self.alignment = alignment;
    self
  }

  fn line(self) -> Line<'static> {
    let mut spans = match self.state {
      LoaderWidgetState::Running { glyph, label, .. } => vec![
        Span::styled(
          format!("{glyph} "),
          Style::default().fg(self.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
          label.to_string(),
          Style::default().fg(self.text).add_modifier(Modifier::BOLD),
        ),
      ],
      LoaderWidgetState::Failed { message, .. } => vec![
        Span::styled("! ", Style::default().fg(self.failed).add_modifier(Modifier::BOLD)),
        Span::styled(
          message.to_string(),
          Style::default().fg(self.failed).add_modifier(Modifier::BOLD),
        ),
      ],
    };

    let detail = match self.state {
      LoaderWidgetState::Running { detail, .. } | LoaderWidgetState::Failed { detail, .. } => detail,
    };
    if let Some(detail) = detail {
      spans.push(Span::styled(" — ", Style::default().fg(self.muted)));
      spans.push(Span::styled(detail.to_string(), Style::default().fg(self.muted)));
    }
    Line::from(spans)
  }
}

impl Widget for LoaderWidget<'_> {
  fn render(self, area: Rect, buf: &mut Buffer) {
    Paragraph::new(self.line()).alignment(self.alignment).render(area, buf);
  }
}

pub fn draw(f: &mut Frame, app: &mut App) {
  // Header and footer are single borderless rows (#185); the body fills the
  // rest. The fuzzy filter no longer claims its own row — it renders inside
  // the worktrees pane title (#262), so the layout is a stable header / body /
  // footer split whether or not a filter is active.
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
    .split(f.area());

  draw_header(f, chunks[0], app);
  draw_body(f, chunks[1], app);
  draw_footer(f, chunks[2], app);

  match app.view {
    View::Help => draw_help(f, app),
    View::Create => draw_create(f, app),
    View::Confirm => draw_confirm(f, app),
    View::Report => draw_report(f, app),
    View::OpenMenu => draw_open_menu(f, app),
    View::LinkPrompt => draw_link_prompt(f, app),
    View::CommandPalette => draw_command_palette(f, app),
    View::CommandLogs => draw_command_logs(f, app),
    View::Config => draw_config_panel(f, app),
    View::Pty => draw_pty_overlay(f, app),
    View::Note => draw_note_editor(f, app),
    // #325: exec profile picker renders as a small centred modal.
    View::ExecPicker => draw_exec_picker(f, app),
    // #325: clean reclaim report renders as a centred modal.
    View::CleanReport => draw_clean_overlay(f, app),
    // #290: branch-rename inline modal renders over the list.
    View::Edit => draw_edit_worktree(f, app),
    // #408: generic detail overlay (agent sessions) as a centred modal.
    View::DetailOverlay => draw_detail_overlay(f, app),
    View::List => {}
  }
}

/// Styled, width-driven header builder (issue #185). Replaces the flat
/// header-title string in the rendered TUI with a clear visual hierarchy
/// that mirrors the #180 footer's chip language:
///
/// - **Current directory** — a leading reverse-video badge.
/// - **Working directory** — dimmed (`DarkGray`), stable after the badge and
///   dropped/truncated under width pressure.
/// - **`picker`** — an accent-distinct (yellow) chip flagging a `gwm switch`
///   picker session.
/// - **Version** — a right-pinned reverse-video badge (` gwm <version> `)
///   painted on `accent`. The version still comes from `CARGO_PKG_VERSION`,
///   so `gwm --version` parity is preserved.
///
/// Priority when the terminal is narrow: the version chip survives (clipped
/// only if it alone exceeds `width`), then the current-dir badge, then the
/// picker chip, and the path is sacrificed first. Pure and measured with
/// `chars().count()` so the contract is pinned by `tests/tui_header_tests.rs`
/// without a ratatui backend; control chars are collapsed to spaces so a
/// pathological path can never split the single row.
pub fn header_line(
  repo_name: &str,
  workdir_display: &str,
  picker_mode: bool,
  width: usize,
  theme: &Theme,
) -> Line<'static> {
  // A zero-width row can hold nothing — return an empty line rather than let
  // `trunc` floor a 1-column `…` into existence.
  if width == 0 {
    return Line::default();
  }

  let sanitize = |s: &str| -> String { s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect() };
  let repo = sanitize(repo_name);
  let path = sanitize(workdir_display);

  let version_style = chip_style(theme.accent);
  let dir_badge_style = chip_style(theme.name);
  // Picker chip uses the `dirty` role (not the accent) so the mode warning
  // reads as distinct from the always-present version chip — pre-theme this
  // was a hard-coded `Color::Yellow`.
  let picker_style = chip_style(theme.dirty);
  let path_style = Style::default().fg(theme.muted);

  let version_text = format!(" gwm {} ", env!("CARGO_PKG_VERSION"));
  let version_w = version_text.chars().count();
  let dir_text = format!(" {} ", repo);
  let dir_w = dir_text.chars().count();

  // Priority floor: if even the right-pinned version chip cannot fit, show it
  // clipped alone — never an empty header.
  if width < version_w {
    return Line::from(Span::styled(trunc(&version_text, width), version_style));
  }

  let mut spans: Vec<Span<'static>> = Vec::new();
  let mut used = 0usize;

  // Current-directory badge first. If the row is too narrow for the full
  // badge plus the pinned version, trim the badge rather than move the path
  // or version around.
  let dir_budget = width.saturating_sub(version_w + 1);
  if dir_w <= dir_budget {
    spans.push(Span::styled(dir_text, dir_badge_style));
    used += dir_w;
  } else if dir_budget > 0 {
    let clipped = trunc(&dir_text, dir_budget);
    used += clipped.chars().count();
    spans.push(Span::styled(clipped, dir_badge_style));
  }

  // Picker chip — mode-safety indicator, kept right after the current-dir
  // badge when there is room.
  if picker_mode {
    let picker_text = " picker ".to_string();
    let need = 1 + picker_text.chars().count(); // leading space + chip
    if used + need + version_w < width {
      spans.push(Span::raw(" "));
      spans.push(Span::styled(picker_text, picker_style));
      used += need;
    }
  }

  // Path — dimmed secondary context. It stays immediately after the current
  // directory badge and is dropped/truncated under pressure; the version chip
  // remains pinned at the end of the row.
  let path_gap = 2usize;
  if used + path_gap + version_w < width {
    let avail = width - used - path_gap - version_w;
    let path_disp = trunc(&path, avail);
    if !path_disp.is_empty() {
      let w = path_disp.chars().count();
      spans.push(Span::raw("  "));
      spans.push(Span::styled(path_disp, path_style));
      used += path_gap + w;
    }
  }

  let pad = width.saturating_sub(used + version_w);
  if pad > 0 {
    spans.push(Span::raw(" ".repeat(pad)));
  }
  spans.push(Span::styled(version_text, version_style));

  Line::from(spans)
}

/// Lay out the worktree table and the optional preview sidebar for the
/// body region. The layout (hidden / side-by-side / stacked) and the
/// left-or-right side are decided by the pure
/// [`SidebarState::resolve_layout`](super::state::sidebar::SidebarState::resolve_layout),
/// so this function only translates that decision into ratatui splits
/// (issue #188). The table/sidebar ratio is per-axis (issue #217): 55/45
/// side-by-side, 42/58 stacked — see
/// [`ResolvedSidebarLayout::split_percentages`](super::state::sidebar::ResolvedSidebarLayout::split_percentages).
fn draw_body(f: &mut Frame, area: Rect, app: &mut App) {
  use super::state::sidebar::ResolvedSidebarLayout as Resolved;

  let layout = app.sidebar.resolve_layout(area.width);
  let (table_share, table_pct, sidebar_pct) = match layout.split_percentages() {
    Some((t, s)) => (t, Constraint::Percentage(t), Constraint::Percentage(s)),
    None => {
      // Sidebar not rendered → no scrollable surface → no max scroll to track.
      app.sidebar.max_scroll = 0;
      app.sidebar.wt_max_scroll = 0;
      draw_list(f, area, app);
      return;
    }
  };

  // Compact mode spends one line or column on a rule between the two
  // panes (validation feedback on PR #546). Without it, the boundary
  // between the worktrees pane and the sidebar reads exactly like the
  // boundary between two sidebar sections — both are just a filled
  // header — so nothing says where one focusable pane ends and the other
  // begins. The bordered layout does not need it: its box rules already
  // do. Zero-width in the bordered mode so the split is unchanged there.
  let separator = if app.config.tui.layout.is_compact() { 1 } else { 0 };

  match layout {
    Resolved::Hidden => unreachable!("Hidden returns None from split_percentages, handled above"),
    Resolved::SideBySide { sidebar_left } => {
      let (first, second) = if sidebar_left {
        (sidebar_pct, table_pct)
      } else {
        (table_pct, sidebar_pct)
      };
      let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([first, Constraint::Length(separator), second])
        .split(area);
      let (list_area, sidebar_area) = if sidebar_left {
        (split[2], split[0])
      } else {
        (split[0], split[2])
      };
      if separator > 0 {
        draw_pane_separator(f, split[1], Direction::Horizontal, &app.theme);
      }
      draw_list(f, list_area, app);
      draw_sidebar(f, sidebar_area, app);
    }
    Resolved::Stacked => {
      // Table on top, sidebar below — the default layout (issue #217) and the
      // narrow-terminal fallback. The left/right position does not apply to a
      // vertical stack.
      //
      // Compact mode sizes the pane to its rows instead of to its share
      // (issue #545), so a short list stops reserving a column of blank
      // rows above a scrolling sidebar. The share stays the ceiling.
      //
      // The sidebar then has to be `Fill`, not its percentage: two
      // constraints that no longer add up to the body height leave the
      // remainder as dead space *after* the sidebar under ratatui's
      // default flex, so the rows the pane gave back would reach nobody
      // (Codex review, PR #546). Pinned by
      // `compact_mode_lets_the_sidebar_absorb_the_whole_split`.
      let (table_constraint, sidebar_constraint) = if app.config.tui.layout.is_compact() {
        let quota = area.height.saturating_mul(table_share) / 100;
        let rows = app.filtered_indices().len() as u16;
        let table = super::state::sidebar::stacked_table_height(quota, rows, Chrome::COMPACT_ROWS);
        (Constraint::Length(table), Constraint::Fill(1))
      } else {
        (table_pct, sidebar_pct)
      };
      let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([table_constraint, Constraint::Length(separator), sidebar_constraint])
        .split(area);
      if separator > 0 {
        draw_pane_separator(f, split[1], Direction::Vertical, &app.theme);
      }
      draw_list(f, split[0], app);
      draw_sidebar(f, split[2], app);
    }
  }
}

/// The rule between the two focusable panes in compact mode.
///
/// `direction` is the one the *split* runs in, so a vertical split
/// (stacked) draws a horizontal rule and vice versa. Painted in `muted`:
/// it is a boundary, not a focus signal — the headers carry that, and a
/// separator that also changed with focus would compete with them.
fn draw_pane_separator(f: &mut Frame, area: Rect, split: Direction, theme: &Theme) {
  let (glyph, count) = match split {
    Direction::Vertical => ("─", area.width),
    Direction::Horizontal => ("│", area.height),
  };
  let style = Style::default().fg(theme.muted);
  match split {
    Direction::Vertical => {
      let line = Line::from(Span::styled(glyph.repeat(count as usize), style));
      f.render_widget(Paragraph::new(line), area);
    }
    Direction::Horizontal => {
      let lines: Vec<Line<'static>> = (0..count).map(|_| Line::from(Span::styled(glyph, style))).collect();
      f.render_widget(Paragraph::new(lines), area);
    }
  }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
  // Tilde-compress the workdir so `$HOME`-rooted paths read as `~/…` — same
  // treatment as the sidebar identity block. The styled, width-driven layout
  // (version chip, bold repo, dimmed path, optional picker chip) lives in
  // `header_line` (issue #185) so it can be pinned without a ratatui backend.
  let workdir = tilde_compress(&app.workdir.to_string_lossy());
  // Borderless single row (#185): the builder gets the full area width and the
  // line renders flush, mirroring the footer. No `Wrap` — `header_line`
  // guarantees one visual line clipped to `width`.
  let line = header_line(
    // The workspace label, which is what the user is looking at; `repo_name` is
    // the naming name and can be the same string in a workspace of one (#480).
    &app.display_repo_name,
    &workdir,
    app.picker_mode,
    area.width as usize,
    &app.theme,
  );
  f.render_widget(Paragraph::new(line), area);
}

/// Border colour for a focus-swappable panel (worktree list ↔ sidebar,
/// toggled with `Tab`): the theme `focus` role when the panel holds focus,
/// else a muted `DarkGray`. Extracted as a pure fn so the focus→theme wiring
/// is pinned by `tests/tui_app_tests.rs` without a ratatui backend — and so a
/// regression hardcoding a colour (the pre-#185 `Color::Cyan`) is caught.
pub fn panel_border_color(focused: bool, theme: &super::theme::Theme) -> Color {
  if focused {
    theme.focus
  } else {
    theme.muted
  }
}

/// What a pane's frame costs and how it is painted (issue #545).
///
/// Two shapes, resolved once per pane and threaded down rather than
/// re-decided at each site: the boxed default (a rule on all four sides,
/// the title in the top one, the counter in the bottom one) and compact
/// (a single filled header line, no rules at all).
///
/// Threading the *cost* rather than a bare `compact` flag is deliberate.
/// The chrome budget is read at half a dozen places — layout constraints,
/// the two scroll clamps, the inner width that trims a PR title — and a
/// flag re-tested at each of them drifts. When it does, the failure is the
/// #437 class: the solver hands a section less than it asked for and its
/// trailing rows become unreachable.
#[derive(Debug, Clone, Copy)]
pub struct Chrome {
  /// `true` when the pane draws a filled header instead of box rules.
  pub compact: bool,
  /// Focus signal: `theme.focus` when the pane holds focus, `theme.muted`
  /// otherwise. Paints the border when boxed and the header text when
  /// compact — with no rules left, the header *is* where focus reads.
  pub accent: Color,
  /// Header background, compact only. Carries the focus signal too
  /// (validation feedback on PR #546: the text colour alone did not read
  /// at a glance): `selection_bg` on the focused pane, `section_bg`
  /// elsewhere. Both roles already exist and the theme guarantees they
  /// differ, so the two header states are distinct by construction on
  /// every preset — no third background role to keep in tune.
  pub fill: Color,
  /// `true` when this pane holds focus. Drives [`Self::body_style`].
  pub focused: bool,
}

impl Chrome {
  /// Rows a compact frame costs — the single header line. Named so the
  /// layout can budget for it without building a `Chrome` just to read
  /// a constant off it.
  pub const COMPACT_ROWS: u16 = 1;

  /// Chrome for a surface that stays boxed whatever `[tui] compact`
  /// says — the modals, where a rule separates the panel from the
  /// content it floats over.
  pub fn boxed(accent: Color) -> Self {
    Self {
      compact: false,
      accent,
      fill: Color::Reset,
      focused: true,
    }
  }

  pub fn resolve(compact: bool, focused: bool, theme: &super::theme::Theme) -> Self {
    Self {
      compact,
      accent: panel_border_color(focused, theme),
      fill: if focused { theme.selection_bg } else { theme.section_bg },
      focused,
    }
  }

  /// Base style for a pane's *content* rows.
  ///
  /// Compact dims the inactive pane (validation feedback on PR #546):
  /// with no border to grey out, two panes of equally bright content
  /// read as equally live. `DIM` rather than repainting in `muted`
  /// because the body's colours are semantic — a dirty branch, a staged
  /// file — and flattening them to grey would cost more information than
  /// the focus signal is worth. Terminals that ignore `DIM` simply keep
  /// the pre-#545 look, where the header still carries the signal.
  ///
  /// `Bordered` is left alone on purpose: it exists to reproduce gwm's
  /// layout up to 1.7, and its rules already say which pane is active.
  pub fn body_style(self) -> Style {
    if self.compact && !self.focused {
      Style::default().add_modifier(Modifier::DIM)
    } else {
      Style::default()
    }
  }

  /// Rows the frame costs: the top and bottom rules, or the single
  /// header line.
  pub fn rows(self) -> u16 {
    if self.compact {
      Self::COMPACT_ROWS
    } else {
      2
    }
  }

  /// Columns unavailable to content: the two side rules plus the one
  /// leading pad column, or just that pad when there are no rules.
  pub fn cols(self) -> u16 {
    if self.compact {
      1
    } else {
      3
    }
  }

  /// The content rect inside a section frame — what a scrollbar or an
  /// inner overlay must aim at. Boxed: inset on all four sides. Compact:
  /// only the header row is spent, so the content keeps the full width
  /// and the right column stays available for the scrollbar.
  pub fn inner(self, area: Rect) -> Rect {
    if self.compact {
      Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(1),
      }
    } else {
      Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
      }
    }
  }
}

/// Title for the worktree pane block (issue #217; carries the inline fuzzy
/// filter since #262). Always leads with the `[1]` focus mnemonic (the pane
/// is focusable with the `1` key). When a filter is live — the user is typing
/// (`active`) or a sticky query remains — the title embeds the `/query`
/// prompt (in `filter_color`), a block cursor while `active`, and the
/// `(visible/total)` ratio so the user sees how much the filter narrowed the
/// list. With no filter it shows just the `(total)` count. This replaces the
/// standalone filter bar row (#262): the filter now reads in the pane border,
/// attached to the list it narrows. Pure + width-free so the copy + the
/// prompt colour are pinned by `tests/tui_ui_helpers_tests.rs` without a
/// ratatui backend.
pub fn worktrees_pane_title(
  query: &str,
  active: bool,
  visible: usize,
  total: usize,
  filter_color: Color,
  compact: bool,
) -> Line<'static> {
  let mut spans = vec![Span::raw(if compact { " [1] WORKTREES " } else { " [1] Worktrees " })];
  // Live filter (typing or sticky): show the `/query` prompt + optional
  // cursor, mirroring the Vim-style bar the title replaced.
  if active || !query.is_empty() {
    spans.push(Span::styled(
      "/",
      Style::default().fg(filter_color).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(query.to_string()));
    if active {
      spans.push(Span::styled(
        "\u{2588}",
        Style::default().fg(filter_color).add_modifier(Modifier::SLOW_BLINK),
      ));
    }
    spans.push(Span::raw(" "));
  }
  // Counter: the visible/total ratio only once a query actually narrows the
  // list; an empty query (even while the bar is open) matches all, so the
  // plain `(total)` form reads cleaner.
  let counter = if query.is_empty() {
    format!("({}) ", total)
  } else {
    format!("({}/{}) ", visible, total)
  };
  spans.push(Span::raw(counter));
  Line::from(spans)
}

/// Title for the head section of the status (sidebar) pane (issue #217).
/// Carries the `[2]` focus mnemonic (focusable with the `2` key), mirroring
/// [`worktrees_pane_title`]'s `[1]`. The sidebar is a stack of sub-sections;
/// this labels the first one so the pane reads as `[2] Status` without
/// nesting an extra bordered frame.
///
/// `compact` switches to the mode's idiom — see [`pane_title`]. Both arms
/// stay `&'static str` so `render_section` keeps handing ratatui a
/// borrowed title rather than allocating one per frame.
pub fn status_pane_title(compact: bool) -> &'static str {
  if compact {
    " [2] STATUS "
  } else {
    " [2] Status "
  }
}

/// Render a pane title in the idiom of the current mode (issue #545).
///
/// Both modes keep the same shape — `[<focus key>] Label [<action key>]`,
/// bracketed, chord trailing for a sub-pane and leading for a focusable
/// pane. Compact only shouts the label, so the header reads as chrome
/// rather than as a row of content now that no rule delimits it.
///
/// Compact first led with a bare chord (` F ISSUE / PR `); validation
/// feedback on PR #546 sent it back. The bracket convention is how every
/// other surface in the TUI writes a key — the footer, the help overlay,
/// the palette — and the compact mode has no business forking it.
fn pane_title(compact: bool, label: &str, chord: &str) -> String {
  if compact {
    format!(" {} [{}] ", label.to_uppercase(), chord)
  } else {
    format!(" {} [{}] ", label, chord)
  }
}

/// Compose the single header line a compact pane spends instead of a box
/// (issue #545): the title on the left, the counter flushed right, and
/// padding in between so the line spans `width` exactly.
///
/// Padding to the full width is not cosmetic. The caller paints the fill
/// by styling the whole header row, and a line that stopped at its text
/// would leave the boundary reading as a stray highlighted word rather
/// than as the edge of a section.
///
/// `accent` carries the focus signal — with the rules gone, the header
/// text is where "which pane am I in" now lives. It is applied only to
/// spans that have no colour of their own; a span that already carries
/// one (the filter `/` prompt) encodes something other than focus and is
/// left alone.
///
/// On a pane too narrow for both, the counter is dropped whole rather
/// than overlapped — the title names the section and carries its focus
/// mnemonic, so it is the half worth keeping — and the title itself is
/// truncated only once it is alone and still too wide.
///
/// Pure and width-explicit so `tests/tui_ui_helpers_tests.rs` pins the
/// layout without a ratatui backend.
pub fn compact_header_line(
  title: Line<'static>,
  counter: Option<Line<'static>>,
  width: u16,
  accent: Color,
) -> Line<'static> {
  let width = width as usize;
  let accent_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
  // Only spans with no colour of their own take the accent; the filter `/`
  // prompt and the Working Tree's per-category counts already encode
  // something that is not focus.
  let accentuate = |s: Span<'static>| {
    if s.style.fg.is_none() {
      Span::styled(s.content, accent_style.patch(s.style))
    } else {
      s
    }
  };
  let mut spans: Vec<Span<'static>> = title.spans.into_iter().map(accentuate).collect();

  let title_w: usize = spans.iter().map(|s| s.content.chars().count()).sum();
  // Truncate the title when it alone overflows. Cutting from the tail
  // keeps the leading chord — the actionable half — visible longest.
  if title_w > width {
    let mut left = width;
    for s in spans.iter_mut() {
      let w = s.content.chars().count();
      if w <= left {
        left -= w;
      } else {
        *s = Span::styled(s.content.chars().take(left).collect::<String>(), s.style);
        left = 0;
      }
    }
    spans.retain(|s| !s.content.is_empty());
    return Line::from(spans);
  }

  let counter_w = |c: &Line<'static>| -> usize { c.spans.iter().map(|s| s.content.chars().count()).sum() };
  let counter = counter.filter(|c| title_w + counter_w(c) <= width);
  let pad = width - title_w - counter.as_ref().map(counter_w).unwrap_or(0);
  if pad > 0 {
    spans.push(Span::styled(" ".repeat(pad), accent_style));
  }
  if let Some(counter) = counter {
    spans.extend(counter.spans.into_iter().map(accentuate));
  }
  Line::from(spans)
}

/// Bottom-right `selected of visible` counter for a pane footer (issue
/// #217), lazygit-style. `selected` is the 1-based cursor position;
/// `visible` is the count of rows currently on screen. Returns `None` when
/// the pane is empty so the footer disappears instead of rendering ` 0 of 0 `
/// — mirroring the Recent Commits section, which also drops its counter when
/// there is nothing to scroll.
pub fn pane_counter(selected: usize, visible: usize) -> Option<String> {
  if visible == 0 {
    None
  } else {
    Some(format!(" {} of {} ", selected, visible))
  }
}

/// Worktrees-pane counter, with the mark count appended while rows are marked
/// (issue #484). Only `d` reads the mark set: every other verb acts on the
/// cursor row, so a live selection has to stay visible or `b` / `s` / `p`
/// would silently look like they ignored it. `marked = 0` is the pre-#484
/// counter, verbatim.
pub fn list_pane_counter(selected: usize, visible: usize, marked: usize) -> Option<String> {
  let base = pane_counter(selected, visible)?;
  if marked == 0 {
    return Some(base);
  }
  Some(format!("{}· {} marked ", base, marked))
}

fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
  // Filter-aware: the visible rows are the filtered subset (issue #21). When
  // there is no active filter, this is the identity over `app.worktrees`.
  // Borrow scoping: `filtered_indices` returns `&[usize]` rooted in
  // `&mut app.filter`, which conflicts with the immutable `app.worktrees`
  // read on the next line. Materialise the indices into an owned `Vec`
  // so the mutable borrow ends. The expensive path (nucleo pass) stays
  // memoised on `FilterState`; this per-frame clone is just a Vec<usize>
  // of length ≤ worktrees.len().
  let filtered: Vec<usize> = app.filtered_indices().to_vec();
  let visible: Vec<&WorktreeInfo> = filtered.iter().filter_map(|&i| app.worktrees.get(i)).collect();
  // `Theme` is `Copy`; snapshot it so the row/header builders below can
  // read roles without conflicting with the mutable `app.list_state`
  // borrow handed to `render_stateful_widget`.
  let theme = app.theme;

  // Workspace mode (issue #36): a leading REPO column naming each row's repo.
  // Names are resolved per visible row up front so the immutable `app` reads
  // don't clash with the mutable `list_state` borrow at render time.
  let is_workspace = app.is_workspace();
  let repo_names: Vec<String> = if is_workspace {
    filtered
      .iter()
      .map(|&raw| app.row_repo_name(raw).unwrap_or("?").to_string())
      .collect()
  } else {
    Vec::new()
  };
  let repo_w = if is_workspace {
    column_width(repo_names.iter().map(|s| s.as_str()), 6, 24)
  } else {
    0
  };

  // Dynamic column widths derived from the visible subset so columns fit the
  // rows actually on screen. The path column is always last and absorbs the
  // remaining width.
  let name_w = column_width(visible.iter().map(|w| w.name.as_str()), 18, 38);
  let branch_w = column_width(visible.iter().map(|w| w.branch.as_deref().unwrap_or("-")), 18, 38);
  let status_w: u16 = 16;
  let row_widths = RowWidths {
    name: name_w,
    branch: branch_w,
    status: status_w,
  };

  // Header cells, with an optional REPO column after the (caption-less) age
  // column in workspace mode.
  // #484: the mark column leads, right under the cursor arrow. Caption-less,
  // like the age and I/P columns.
  let mut header_cells = if app.marked_count() > 0 {
    vec![Cell::from(""), Cell::from("")]
  } else {
    vec![Cell::from("")]
  };
  if is_workspace {
    header_cells.push(Cell::from("REPO"));
  }
  // AGENT column only when at least one session is detected (Codex review
  // round D): a no-agent setup keeps the exact pre-#408 table instead of an
  // empty fixed column squeezing NAME/BRANCH/PATH on narrow terminals.
  let show_agent = app.any_agent_sessions();
  // #515: the note column follows the same rule as AGENT and the mark
  // column — it only exists once something is in it, so a user with no
  // notes keeps the exact pre-#515 table instead of an empty column eating
  // two cells on a narrow terminal. Caption-less: the marker is binary.
  let show_note = visible.iter().any(|w| w.has_note);
  header_cells.push(Cell::from("I/P"));
  if show_note {
    header_cells.push(Cell::from(""));
  }
  header_cells.push(Cell::from("NAME"));
  header_cells.push(Cell::from("BRANCH"));
  header_cells.push(Cell::from("STATUS"));
  if show_agent {
    header_cells.push(Cell::from("AGENT"));
  }
  header_cells.push(Cell::from("PATH"));
  let header = Row::new(header_cells).style(Style::default().fg(theme.muted).add_modifier(Modifier::BOLD));

  // Agent cells resolved up front (issue #408) for the same borrow reason as
  // `repo_names`: `agents_for` reads `app` immutably, `list_state` is borrowed
  // mutably at render time. Pure snapshot lookups — no I/O on the render path.
  let now = std::time::SystemTime::now();
  let agent_cells: Vec<Option<(&'static str, crate::agent_sessions::Freshness)>> = visible
    .iter()
    .map(|w| agent_cell_label(app.agents_for(w), now))
    .collect();

  // #484: the mark column only exists while something is marked, so a user
  // who never presses `Space` keeps the exact pre-#484 table instead of an
  // empty column eating two cells on a narrow terminal (same rule the AGENT
  // column follows).
  let marked_count = app.marked_count();
  let marks: Vec<bool> = if marked_count > 0 {
    visible.iter().map(|w| app.is_marked(&w.path)).collect()
  } else {
    Vec::new()
  };

  let rows: Vec<Row> = visible
    .iter()
    .enumerate()
    .map(|(vi, w)| {
      let repo = is_workspace.then(|| (repo_names[vi].as_str(), repo_w));
      let agent = show_agent.then_some(agent_cells[vi]);
      let mark = marks.get(vi).copied();
      build_row(w, mark, repo, row_widths, agent, show_note, &theme)
    })
    .collect();

  // ratatui's Layout solver squeezes the FIRST `Length` column to
  // satisfy the others when terminal width is tight. We want the age
  // column rock-stable at 4 cells (the cost of losing the unit
  // letter to truncation — "22h" → "22" — is worse than name/branch
  // shrinking by a char or two). Strategy:
  //   - `Length(4)` for age, `Length(3)` for marker (`●/●`, `●/-`, etc.),
  //     `Length(16)` for status: hard-fixed lengths the solver must honour.
  //   - `Min(name_w)` / `Min(branch_w)`: these absorb the pressure
  //     when the terminal is narrow (they shrink down to 8) and grow
  //     to the original clamped width (or more) when there's room.
  //   - `Fill(1)` for path: takes whatever's left, vanishes last.
  // Verified by standalone probe down to 40-cell terminals: col 0
  // stays at 4 cells across every size.
  let mut widths = if marked_count > 0 {
    // #484: mark glyph + its trailing space.
    vec![Constraint::Length(2), Constraint::Length(4)]
  } else {
    vec![Constraint::Length(4)]
  };
  if is_workspace {
    // REPO column sits between age and the I/P marker; a hard length so the
    // solver doesn't starve it on narrow terminals.
    widths.push(Constraint::Length(repo_w));
  }
  widths.push(Constraint::Length(3));
  if show_note {
    // #515: one cell for the note marker, hard-fixed like the I/P column.
    widths.push(Constraint::Length(1));
  }
  widths.extend([
    Constraint::Min(name_w),
    Constraint::Min(branch_w),
    Constraint::Length(status_w),
  ]);
  if show_agent {
    // AGENT (issue #408): hard length sized to the longest agent name
    // ("opencode"), so the solver never starves it into ambiguity.
    widths.push(Constraint::Length(8));
  }
  widths.push(Constraint::Fill(1));

  let list_has_focus = !(app.sidebar.open && app.sidebar.focused);
  let chrome = Chrome::resolve(app.config.tui.layout.is_compact(), list_has_focus, &app.theme);

  let title = worktrees_pane_title(
    app.filter.query(),
    app.filter.active,
    visible.len(),
    app.worktrees.len(),
    app.theme.dirty,
    chrome.compact,
  );

  // Bottom-right `selected of visible` counter (issue #217), mirroring the
  // Recent Commits footer. `list_state.selected()` is 0-based; render it
  // 1-based. Blank when nothing is visible so the footer disappears.
  let selected_1based = app.list_state.selected().map(|i| i + 1).unwrap_or(0);
  let counter = list_pane_counter(selected_1based, visible.len(), marked_count);

  let mut table = Table::new(rows, widths)
    .header(header)
    .column_spacing(1)
    .style(chrome.body_style())
    .row_highlight_style(Style::default().bg(theme.selection_bg).add_modifier(Modifier::BOLD))
    .highlight_symbol("▶ ");

  // Compact mode: the counter has no bottom rule to sit in, so it moves to
  // the right of the header line and the whole box collapses to that one
  // row, painted here. The table then renders into the area below with no
  // block at all — one row of chrome instead of two, two columns back.
  let table_area = if chrome.compact {
    let header_area = Rect { height: 1, ..area };
    let line = compact_header_line(title, counter.map(Line::from), header_area.width, chrome.accent);
    f.render_widget(
      Paragraph::new(line).style(Style::default().bg(chrome.fill)),
      header_area,
    );
    Rect {
      y: area.y.saturating_add(1),
      height: area.height.saturating_sub(1),
      ..area
    }
  } else {
    let mut block = Block::default()
      .borders(Borders::ALL)
      .title(title)
      .border_style(Style::default().fg(chrome.accent));
    if let Some(counter) = counter {
      block = block.title_bottom(Line::from(counter).right_aligned());
    }
    table = table.block(block);
    area
  };

  f.render_stateful_widget(table, table_area, &mut app.list_state);
}

/// Details panel for the selected worktree — structured info, recent commits,
/// working-tree status, and a commands cheat-sheet (lazyssh-style layout).
///
/// Content is cached on `App` keyed by the selected worktree's path so the
/// underlying `git log` / `git status` only run when the selection changes
/// or `refresh()` invalidates the cache.
fn draw_sidebar(f: &mut Frame, area: Rect, app: &mut App) {
  let chrome = Chrome::resolve(app.config.tui.layout.is_compact(), app.sidebar.focused, &app.theme);
  // `Theme` is `Copy`; snapshot it so the cached section builder can read
  // roles while `app.sidebar.cache` is mutably borrowed below.
  let theme = app.theme;

  // Resolve (or populate) the cached worktree sections for the current
  // selection. Issue / PR block is rebuilt every frame (its fetch state
  // moves independently of the worktree info). The leading `●` status
  // dot line on the Worktree section is also rebuilt fresh each frame
  // (issue #73) so it tracks live PR / issue fetches without
  // invalidating the expensive git-preview cache underneath.
  // Cache key carries the active mode (issue #34) so toggling between
  // commits / stashes re-shells the right git command instead of
  // serving the previous mode's pre-rendered lines.
  let active_mode = app.sidebar.mode;

  // Inner width = block area − 2 border columns − 1 leading-padding column
  // (applied by `render_section`). Summary lines trim their variable parts
  // (title / error blob) so the total visible width fits — without this,
  // long PR titles would either overflow the block right border or be
  // wrapped onto a second visual row that the `Constraint::Length` below
  // never budgeted for, breaking the layout.
  let issue_pr_inner_width = area.width.saturating_sub(chrome.cols()) as usize;

  let Some(w) = app.selected().cloned() else {
    // Nothing selected: render the placeholder and bail. No cache to read,
    // so the borrow gymnastics below don't apply.
    let issue_pr_lines = github_status_lines(app, issue_pr_inner_width);
    let placeholder = [Line::from("(nothing selected)")];
    let h = |lines: usize| (lines as u16).saturating_add(chrome.rows());
    let constraints = [
      Constraint::Length(h(placeholder.len())),
      Constraint::Length(h(issue_pr_lines.len())),
      Constraint::Length(0),
      Constraint::Length(0),
      Constraint::Min(3),
    ];
    let chunks = Layout::default()
      .direction(Direction::Vertical)
      .constraints(constraints)
      .split(area);
    app.sidebar.max_scroll = 0;
    app.sidebar.scroll = 0;
    app.sidebar.wt_max_scroll = 0;
    app.sidebar.wt_scroll = 0;
    render_section(
      f,
      chunks[0],
      status_pane_title(chrome.compact),
      SectionBody::new(&placeholder),
      chrome,
      0,
      None,
    );
    render_section(
      f,
      chunks[1],
      issue_pr_pane_title(&app.keymap, chrome.compact),
      SectionBody::new(&issue_pr_lines),
      chrome,
      0,
      None,
    );
    render_section(
      f,
      chunks[4],
      recent_items_pane_title(active_mode, &app.keymap, chrome.compact),
      SectionBody::new(&[]),
      chrome,
      0,
      None,
    );
    return;
  };

  // Issue #343: the render path NEVER shells out. The git-backed sections
  // (`git_diff_stat_vs_base` + `git status` + `git log` / `git stash list`)
  // are rebuilt off-thread by the async sidebar worker (`App::
  // maybe_refresh_sidebar` → `TaskKind::Sidebar`), which stores the payload in
  // `app.sidebar.cache` keyed by `(path, mode)`. Here we only READ it, and
  // only when it was built for the *current* selection + mode — a stale-key or
  // cold cache renders a muted "loading…" placeholder while the worker catches
  // up. The identity card (branch / head / path / badges) comes straight from
  // `w`, so it shows instantly on navigation; only the diff figure and the
  // status / commits blocks wait on the worker. The live `● <name>` header and
  // the Issue / PR block are still built per-frame below (no subprocess).
  // Whether the cached payload is authoritative for the current selection.
  // The sections to render are resolved from this at each read site (lengths
  // below, render pass further down) as a direct match so the immutable cache
  // borrow stays scoped and never overlaps the `app.sidebar.max_scroll` /
  // `scroll` writes in between — the pre-#343 borrow discipline, unchanged.
  let cache_is_current = matches!(
    &app.sidebar.cache,
    Some(((p, m), _)) if *p == w.path && *m == active_mode
  );

  // The loading placeholder, built ONLY when the cache is stale — on the warm
  // path it stays an empty `default()` (no per-frame identity-line allocation
  // in what is, after all, a render-perf change). The identity card renders
  // straight from `w` (no subprocess), so it shows instantly on navigation;
  // the status / commits blocks read "loading…" until the worker's payload
  // lands.
  let placeholder = if cache_is_current {
    SidebarSections::default()
  } else {
    SidebarSections {
      worktree: worktree_identity_lines(&w, None, &theme),
      working_tree: match active_mode {
        super::state::sidebar::SidebarMode::Commits => {
          vec![Line::from(Span::styled("loading…", Style::default().fg(theme.muted)))]
        }
        super::state::sidebar::SidebarMode::Stashes => Vec::new(),
      },
      working_tree_counts: WorkingTreeCounts::default(),
      recent_commits: vec![Line::from(Span::styled("loading…", Style::default().fg(theme.muted)))],
    }
  };

  // The live header line and the per-frame Issue / PR block are built BEFORE
  // the long cache borrow so they don't overlap it. The header is the only
  // line that is rebuilt fresh each frame (issue #73) — it's prefixed onto
  // the cached worktree section at render time instead of being spliced into
  // a cloned vec.
  let prefix_lines = vec![sidebar_header_line(&w, app)];
  let issue_pr_lines = github_status_lines(app, issue_pr_inner_width);
  // Agents pane body (issue #408): per-frame pure snapshot + pins lookup
  // (no config I/O — `app.agent_pins` is refreshed off-render), its
  // bordered block collapses to zero height when nothing is pinned.
  let agent_pins: &[String] = app
    .agent_pins
    .get(&crate::agent_sessions::path_display_key(&w.path))
    .map(|v| v.as_slice())
    .unwrap_or(&[]);
  let agent_lines = agent_pane_lines(app.agents_for(&w), agent_pins, std::time::SystemTime::now(), &theme);

  // Read the resolved section lengths via a short immutable borrow so the
  // layout solver and scroll clamp can run before the render borrow. The
  // worktree section gains the live prefix rows (header + optional agent
  // summary).
  let (worktree_len, working_tree_len, working_tree_counts, commits_len) = {
    let s = if cache_is_current {
      app.sidebar.cache.as_ref().map(|(_, s)| s).unwrap_or(&placeholder)
    } else {
      &placeholder
    };
    (
      s.worktree.len() + prefix_lines.len(),
      s.working_tree.len(),
      s.working_tree_counts,
      s.recent_commits.len() as u16,
    )
  };

  // Per-section block height = content rows + 2 border lines. Fixed
  // for the small sections (worktree / issue-PR); the three variable
  // sections — Agents / Working Tree / Recent Commits — share the rest
  // through the responsive solver (issue #438): natural heights while
  // everything fits (commits absorbs the slack, as the old `Min(3)`
  // did), a 5-line floor per visible section plus proportional sharing
  // on overflow. Empty sections keep their collapse behaviour (issue
  // #34: Working Tree is empty in `Stashes` mode; #408: Agents hidden
  // with no session).
  let h = |lines: usize| (lines as u16).saturating_add(chrome.rows());
  let fixed = h(worktree_len).saturating_add(h(issue_pr_lines.len()));
  let (agents_height, working_tree_height, commits_height) = super::state::sidebar::split_section_heights(
    area.height.saturating_sub(fixed),
    chrome.rows(),
    agent_lines.len() as u16,
    working_tree_len as u16,
    commits_len,
  );
  let constraints = [
    Constraint::Length(h(worktree_len)),
    Constraint::Length(h(issue_pr_lines.len())),
    Constraint::Length(agents_height),
    Constraint::Length(working_tree_height),
    Constraint::Length(commits_height),
  ];
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints(constraints)
    .split(area);

  // Recent Commits is the only scrollable section. Clamp the scroll
  // offset to its visible area so `j` / `k` can't scroll past the end.
  // Done before the render borrow so no mutable `app` access overlaps it.
  let commits_area = chunks[4];
  let commits_visible = commits_area.height.saturating_sub(chrome.rows());
  app.sidebar.max_scroll = commits_len.saturating_sub(commits_visible);
  if app.sidebar.scroll > app.sidebar.max_scroll {
    app.sidebar.scroll = app.sidebar.max_scroll;
  }
  let scroll = app.sidebar.scroll;

  // Working Tree scroll (issue #437). The section asks for
  // `Length(content + 2)` but the layout solver may hand it less when
  // the sidebar column is shorter than the sum of its sections — the
  // exact case where entries used to be unreachable. Republish the max
  // against the clamped viewport, same contract as Recent Commits.
  let wt_visible = chunks[3].height.saturating_sub(chrome.rows());
  app.sidebar.wt_max_scroll = (working_tree_len as u16).saturating_sub(wt_visible);
  if app.sidebar.wt_scroll > app.sidebar.wt_max_scroll {
    app.sidebar.wt_scroll = app.sidebar.wt_max_scroll;
  }
  let wt_scroll = app.sidebar.wt_scroll;

  // Issue #34: surface the active mode in the bottom-scrollable
  // panel title. The footer keeps the `i of N` counter; the bottom
  // hint switches to "Enter: copy stash@{N}" in stashes mode.
  let (panel_title, panel_footer) = match active_mode {
    super::state::sidebar::SidebarMode::Commits => {
      let title = recent_items_pane_title(active_mode, &app.keymap, chrome.compact);
      let footer = if commits_len == 0 {
        None
      } else {
        let bottom = scroll.saturating_add(commits_visible).min(commits_len);
        Some(format!(" {} of {} ", bottom, commits_len))
      };
      (title, footer)
    }
    super::state::sidebar::SidebarMode::Stashes => {
      let title = recent_items_pane_title(active_mode, &app.keymap, chrome.compact);
      // The "Enter on stash …" hint from the issue is the operative
      // affordance in this mode — it's worth more than the i/N
      // counter because the user needs to know they can paste the
      // ref name.
      let footer = if commits_len == 0 {
        None
      } else {
        Some(" Enter: copy stash@{N} to status ".to_string())
      };
      (title, footer)
    }
  };
  let issue_pr_title = issue_pr_pane_title(&app.keymap, chrome.compact);
  let working_tree_title = working_tree_pane_title(&app.keymap, chrome.compact);
  // Working Tree footer (issue #287): colour-coded created / modified /
  // deleted counts. `None` in stashes mode (no section) and on a clean tree
  // (all-zero counts → `working_tree_counts_footer` returns `None`), so the
  // footer disappears instead of showing a bare ` 0 `.
  let working_tree_footer = if working_tree_len == 0 {
    None
  } else {
    working_tree_counts_footer(&working_tree_counts, &theme)
  };

  // The render borrow: sections are read by reference and never cloned (issue
  // #238). On a cache hit this copies zero commit text — the up-to-300 `git
  // log` lines stay put in `app.sidebar.cache`; `render_section` only rebuilds
  // the thin padded `Vec<Span>` per visible row, borrowing the span content.
  // `app` is only read immutably from here on (all mutation already happened
  // above), so this long borrow is conflict-free. Resolved from the same
  // `cache_is_current` decision as the length block: the cached payload when it
  // is authoritative for the current selection, else the loading placeholder
  // (issue #343) — never `unwrap()`, keeping the render path panic-free per the
  // house rules.
  let sections = if cache_is_current {
    app.sidebar.cache.as_ref().map(|(_, s)| s).unwrap_or(&placeholder)
  } else {
    &placeholder
  };
  render_section(
    f,
    chunks[0],
    status_pane_title(chrome.compact),
    SectionBody::with_prefix(&prefix_lines, &sections.worktree),
    chrome,
    0,
    None,
  );
  render_section(
    f,
    chunks[1],
    issue_pr_title,
    SectionBody::new(&issue_pr_lines),
    chrome,
    0,
    None,
  );
  if !agent_lines.is_empty() {
    render_section(
      f,
      chunks[2],
      agents_pane_title(&app.keymap, chrome.compact),
      SectionBody::new(&agent_lines),
      chrome,
      0,
      None,
    );
  }
  if !sections.working_tree.is_empty() {
    render_section(
      f,
      chunks[3],
      working_tree_title,
      SectionBody::new(&sections.working_tree),
      chrome,
      wt_scroll,
      working_tree_footer,
    );
    // Scrollbar over the inner right column when the tree overflows the
    // viewport the responsive split granted (user feedback on PR #454) —
    // the scroll existed (#437) but nothing showed where the viewport
    // sat. Same herdr-style helper as the overflowing modals; no-op when
    // everything fits.
    let inner = chrome.inner(chunks[3]);
    if inner.height > 0 {
      let _ = scrollable_body_area(f, inner, wt_scroll, working_tree_len, &theme);
    }
  }
  render_section(
    f,
    commits_area,
    panel_title,
    SectionBody::new(&sections.recent_commits),
    chrome,
    scroll,
    panel_footer.map(ratatui::text::Line::from),
  );
}

/// Borrowed content for one [`render_section`] block (issue #238).
///
/// `lines` are rendered straight out of their owner — for the sidebar that
/// is `app.sidebar.cache`, so a warm-cache frame copies none of the up-to-300
/// commit `Line`s (each holding owned `String` spans) that the previous code
/// deep-cloned every frame just to dodge a borrow conflict. `prefix` carries
/// the single live line (the `● <name>` header) that must lead the worktree
/// section; it's rebuilt fresh per frame anyway, so passing it separately
/// costs nothing and keeps the cached `worktree` vec immutable.
struct SectionBody<'a> {
  prefix: &'a [Line<'a>],
  lines: &'a [Line<'a>],
}

impl<'a> SectionBody<'a> {
  /// Section body with no leading live line (Issue / PR, Working Tree,
  /// Recent Commits, and the `(nothing selected)` placeholder).
  fn new(lines: &'a [Line<'a>]) -> Self {
    Self { prefix: &[], lines }
  }

  /// Section body whose first rows are per-frame live lines — the worktree
  /// identity block, led by the `● <name>` status-dot header (and, since
  /// issue #408, an optional agent summary line).
  fn with_prefix(prefix: &'a [Line<'a>], lines: &'a [Line<'a>]) -> Self {
    Self { prefix, lines }
  }
}

fn render_section(
  f: &mut Frame,
  area: Rect,
  // Title is `impl Into<Line<'static>>` so static-literal call
  // sites (` Worktree ` / ` Issue / PR ` / ` Working Tree `) pass
  // through to ratatui zero-copy (a `&'static str` becomes a
  // `Line<'static>` borrowing the slice), while the dynamic
  // mode-aware title for the bottom panel (` Recent Commits —
  // commits ` / ` Stashes — stashes `) moves in as an owned
  // `String`. Pre-review the signature was `impl Into<String>`,
  // which copied every static literal on every render frame.
  title: impl Into<ratatui::text::Line<'static>>,
  body: SectionBody<'_>,
  chrome: Chrome,
  scroll: u16,
  footer: Option<ratatui::text::Line<'static>>,
) {
  let SectionBody { prefix, lines } = body;
  // Pad content with one leading space per line for breathing room against
  // the left border. Each padded line BORROWS its span content from the
  // source line (`Span::styled(&str, style)` yields a `Cow::Borrowed`, zero
  // allocation) so a warm cache hit copies no commit text — only the thin
  // per-row `Vec<Span>` is rebuilt, which the old code did anyway.
  fn pad<'a>(l: &'a Line<'_>) -> Line<'a> {
    let mut spans = Vec::with_capacity(l.spans.len() + 1);
    spans.push(Span::raw(" "));
    spans.extend(l.spans.iter().map(|s| Span::styled(s.content.as_ref(), s.style)));
    Line::from(spans)
  }
  let padded: Vec<Line<'_>> = prefix.iter().chain(lines.iter()).map(pad).collect();
  // No `Wrap`: every section now relies on ratatui's view-level hard-clip,
  // matching lazygit's commits panel and ensuring 1 logical row = 1 visual
  // row (so the layout's `Constraint::Length` always matches what we draw).
  if chrome.compact {
    // One filled row of chrome instead of a rounded box: the title on the
    // left, the footer (counter / hint) flushed right on that same row
    // rather than in a bottom rule that no longer exists.
    let header_area = Rect { height: 1, ..area };
    let header = compact_header_line(title.into(), footer, header_area.width, chrome.accent);
    f.render_widget(
      Paragraph::new(header).style(Style::default().bg(chrome.fill)),
      header_area,
    );
    let body_area = Rect {
      y: area.y.saturating_add(1),
      height: area.height.saturating_sub(1),
      ..area
    };
    f.render_widget(
      Paragraph::new(padded).style(chrome.body_style()).scroll((scroll, 0)),
      body_area,
    );
    return;
  }
  let mut block = Block::default()
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .title(title.into())
    .border_style(Style::default().fg(chrome.accent));
  if let Some(f) = footer {
    block = block.title_bottom(f.right_aligned());
  }
  let paragraph = Paragraph::new(padded).block(block).scroll((scroll, 0));
  f.render_widget(paragraph, area);
}

/// Lazygit-style header line: `● <name>` where the dot's colour tracks
/// the linked PR / issue state. Rendered fresh every frame (not cached)
/// so the dot reflects the live fetch result without invalidating the
/// expensive git preview cache underneath.
/// Title of the Agents sidebar pane (issue #408, user feedback 2026-07-22):
/// advertises the overlay key like `Issue / PR [F]` does its fetch key.
pub fn agents_pane_title(keymap: &Keymap, compact: bool) -> String {
  pane_title(compact, "Agents", &action_chord(keymap, Action::AgentSessions, "a"))
}

/// Per-frame body of the Agents sidebar pane: one line per **pinned**
/// session (user feedback 2026-07-22 — the pane is the deliberate view;
/// the full detected list lives in the `a` overlay), capped at three —
/// agent kind coloured by freshness, a human-readable recency, and the
/// session name (full id when unnamed). Empty when nothing is pinned, so
/// the bordered block collapses like the Working Tree pane does in stashes
/// mode. Pure — pinned by `tests/tui_app_tests.rs::agent_pane`.
pub fn agent_pane_lines(
  agents: Option<&crate::agent_sessions::WorktreeAgents>,
  pinned: &[String],
  now: std::time::SystemTime,
  theme: &Theme,
) -> Vec<Line<'static>> {
  const MAX_ROWS: usize = 3;
  let Some(agents) = agents else {
    return Vec::new();
  };
  let shown: Vec<&crate::agent_sessions::AgentSession> = agents
    .sessions
    .iter()
    .filter(|s| pinned.iter().any(|p| p == &s.id))
    .collect();
  let mut lines: Vec<Line<'static>> = shown
    .iter()
    .take(MAX_ROWS)
    .map(|s| {
      let freshness = crate::agent_sessions::Freshness::classify(s.last_activity, s.ended, now);
      let (word, color) = match freshness {
        crate::agent_sessions::Freshness::Active => ("active", theme.clean),
        crate::agent_sessions::Freshness::Idle => ("idle", theme.muted),
      };
      let ago = now
        .duration_since(s.last_activity)
        .map(worktree::format_relative_duration)
        .unwrap_or_else(|_| "now".into());
      let identity = s.name.as_deref().unwrap_or(&s.id);
      Line::from(vec![
        Span::styled(
          s.kind.display().to_string(),
          Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" · {word} · {ago} ago · "), Style::default().fg(theme.muted)),
        Span::styled(identity.to_string(), Style::default().fg(theme.name)),
      ])
    })
    .collect();
  let extra = shown.len().saturating_sub(MAX_ROWS);
  if extra > 0 {
    lines.push(Line::from(Span::styled(
      format!("+{extra} more"),
      Style::default().fg(theme.muted),
    )));
  }
  lines
}

fn sidebar_header_line(w: &WorktreeInfo, app: &App) -> Line<'static> {
  let (dot, dot_color) = sidebar_status_dot(app);
  Line::from(vec![
    Span::styled(dot, Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
    Span::styled(w.name.clone(), worktree_name_style(&app.theme)),
  ])
}

/// Resolve the leading status dot for the sidebar header. PR state wins
/// over issue state (a worktree most often tracks a PR); falls back to a
/// neutral darkgray dot when the worktree has no link at all so the
/// alignment stays consistent across rows.
fn sidebar_status_dot(app: &App) -> (&'static str, Color) {
  if let GitHubFetchState::Loaded(pr) = app.pr_fetch_state() {
    return ("● ", pr_badge_color(pr.state, &app.theme));
  }
  if let GitHubFetchState::Loaded(issue) = app.issue_fetch_state() {
    return ("● ", issue_badge_color(issue.state, &app.theme));
  }
  let link = app.current_link();
  if link.pr.is_some() || link.issue.is_some() {
    // Link exists but not fetched yet — neutral white so the user sees
    // there's *something* to refresh with `F`. White carries no theme
    // role (it is "not yet known", not a status), so it stays white.
    return ("● ", Color::White);
  }
  ("● ", app.theme.muted)
}

/// Build the per-section content of the details sidebar for one worktree.
///
/// The Commands cheat-sheet block is intentionally not produced here — it
/// duplicated the `?` help overlay and consumed ~15 vertical lines for no
/// new information. Press `?` for the full key map.
///
/// The `●` status-dot header is intentionally NOT in `worktree` here either —
/// it's rebuilt fresh by `draw_sidebar` on every frame so the dot tracks
/// live PR / issue fetch state without invalidating this cached payload.
pub fn build_sidebar_sections(
  w: &WorktreeInfo,
  mode: super::state::sidebar::SidebarMode,
  diff: Option<worktree::DiffLineStat>,
  theme: &Theme,
) -> SidebarSections {
  use super::state::sidebar::SidebarMode;
  let body = match mode {
    // Pre-#34 behaviour. The `Working Tree` section is unconditionally
    // rendered alongside; both come from `git log` / `git status` and
    // share a single cache invalidation cycle.
    SidebarMode::Commits => recent_commits_lines(w, RECENT_COMMITS_LIMIT, theme),
    // Stashes view (issue #34). `working_tree` is left empty: the
    // user's current dirty state has nothing to do with the stashed
    // contents they're auditing, so a separate `git status` block
    // alongside would only distract. A per-stash file summary
    // (`+/-` counts via `git diff-tree --numstat`) is on the
    // follow-up list; v1 ships `<ref>  <subject>` only.
    SidebarMode::Stashes => stash_lines(w, STASHES_DISPLAY_LIMIT, theme),
  };
  let (working_tree, working_tree_counts) = match mode {
    SidebarMode::Commits => working_tree_lines(w, theme),
    SidebarMode::Stashes => (Vec::new(), WorkingTreeCounts::default()),
  };
  SidebarSections {
    worktree: worktree_identity_lines(w, diff.as_ref(), theme),
    working_tree,
    working_tree_counts,
    recent_commits: body,
  }
}

/// Build the full sidebar payload for one worktree — the diff-vs-base stat
/// plus [`build_sidebar_sections`] — as a single owned, `Send` value (issue
/// #343). This is the unit of work the async sidebar worker runs off-thread:
/// every git subprocess the sidebar needs (`git_diff_stat_vs_base`,
/// `git status --porcelain -z`, `git log`, `git stash list`) fires here, on a
/// worker, so `terminal.draw` never shells out. The result is stored into
/// `SidebarState::cache` by `App::drain_task_results`. `trunks` is the active
/// repo's `doctor.trunks` (the base `gwm pr` targets) captured at spawn.
pub fn build_sidebar_payload(
  w: &WorktreeInfo,
  mode: super::state::sidebar::SidebarMode,
  trunks: &[String],
  theme: &Theme,
) -> SidebarSections {
  let diff = worktree::git_diff_stat_vs_base(&w.path, trunks).ok().flatten();
  build_sidebar_sections(w, mode, diff, theme)
}

/// Number of stash entries shown in `SidebarMode::Stashes`. Set to
/// match `RECENT_COMMITS_LIMIT` so the panel stays a comparable height
/// across modes. Stashes beyond this are still listed by
/// `git stash list` — the limit only governs the in-panel preview.
pub const STASHES_DISPLAY_LIMIT: usize = 10;

/// Render `git stash list` output (issue #34) into ratatui lines for
/// the stashes mode of the sidebar. One stash per row, formatted as
/// `<ref>  <subject>` with the ref in yellow (to mimic git's own
/// colourisation). When the worktree has no stashes the renderer
/// shows a single muted "(no stashes)" line so the panel never reads
/// as broken on a fresh worktree.
fn stash_lines(w: &WorktreeInfo, limit: usize, theme: &Theme) -> Vec<Line<'static>> {
  match crate::worktree::git_stash_list(&w.path, limit) {
    Ok(stashes) if stashes.is_empty() => {
      vec![Line::from(Span::styled(
        "(no stashes)",
        Style::default().fg(theme.muted),
      ))]
    }
    Ok(stashes) => stashes
      .into_iter()
      .map(|s| {
        Line::from(vec![
          Span::styled(s.ref_name, Style::default().fg(theme.dirty)),
          Span::raw("  "),
          Span::raw(s.subject),
        ])
      })
      .collect(),
    Err(e) => vec![Line::from(Span::styled(
      format!("git stash list failed: {}", e),
      Style::default().fg(theme.prunable),
    ))],
  }
}

/// Compact identity card for the Worktree block — `branch · head`,
/// `Created: <age>`, status + flag badges, tilde-compressed path. The
/// `●` status dot + bold name line is prepended live by `draw_sidebar`,
/// not cached here, so the dot can track GitHub fetch state without
/// invalidating the git-preview cache. Skips badges whose flags are
/// false to avoid visual noise.
fn worktree_identity_lines(
  w: &WorktreeInfo,
  diff: Option<&worktree::DiffLineStat>,
  theme: &Theme,
) -> Vec<Line<'static>> {
  let mut out: Vec<Line<'static>> = Vec::with_capacity(5);
  let label_w = "Created".chars().count();
  let label_style = Style::default().fg(theme.muted);

  // Line 1 — "Branch  <branch> · <short head>". Branch colour follows the
  // lazygit scheme (PR #73): worst-state wins (dirty → red,
  // ahead/behind → yellow, unpublished → magenta, synced → green,
  // unknown → dark gray) so the most actionable signal stays at eye
  // level.
  let branch_color = branch_name_color(&w.status, theme);
  let branch = w.branch.clone().unwrap_or_else(|| "-".into());
  let mut spans = vec![
    Span::styled(format!("{:<label_w$}  ", "Branch", label_w = label_w), label_style),
    Span::styled(branch, Style::default().fg(branch_color)),
  ];
  if let Some(head) = w.head.as_deref() {
    spans.push(Span::styled("  ·  ".to_string(), Style::default().fg(theme.muted)));
    spans.push(Span::styled(short_oid(head), Style::default().fg(theme.dirty)));
  }
  out.push(Line::from(spans));

  // Line 2 — "Created  <age>" (compact relative duration, colour-coded
  // by freshness — PR #73). Skipped when the branch has no measurable
  // age (trunk, detached HEAD, or repo open failure).
  out.push(Line::from(vec![
    Span::styled(format!("{:<label_w$}  ", "Created", label_w = label_w), label_style),
    Span::styled(branch_age_label(w), Style::default().fg(branch_age_color(w, theme))),
  ]));

  // Line 3 (issue #287) — "Diff  +<ins> -<del>" of the branch versus its
  // base trunk (three-dot merge-base diff, matching `gwm pr`'s base).
  // Insertions paint green (`untracked` role), deletions red (`prunable`
  // role). Skipped when there's no base, HEAD is the trunk, or the branch
  // has no committed diff yet — `diff` arrives `None` / empty in those
  // cases so the card stays compact.
  if let Some(d) = diff {
    if !d.is_empty() {
      out.push(Line::from(vec![
        Span::styled(format!("{:<label_w$}  ", "Diff", label_w = label_w), label_style),
        Span::styled(format!("+{}", d.insertions), Style::default().fg(theme.untracked)),
        Span::raw(" "),
        Span::styled(format!("-{}", d.deletions), Style::default().fg(theme.prunable)),
      ]));
    }
  }

  // Line 4 — "State  <badges>" with optional flag badges. Only renders the badges
  // that are *true* / *interesting*; the false cases stay invisible.
  let mut state_spans = vec![Span::styled(
    format!("{:<label_w$}  ", "State", label_w = label_w),
    label_style,
  )];
  state_spans.extend(badges_line(w, theme).spans);
  out.push(Line::from(state_spans));

  // Line 4 — "Path  <path>", tilde-compressed for compactness.
  out.push(Line::from(vec![
    Span::styled(format!("{:<label_w$}  ", "Path", label_w = label_w), label_style),
    Span::styled(
      tilde_compress(&w.path.display().to_string()),
      Style::default().fg(theme.muted),
    ),
  ]));

  out
}

/// Render the "Created" line value: compact relative duration (`2d`,
/// `3w`, `1M`, …) read from the pre-computed `WorktreeInfo.age` field,
/// or `"-"` when the branch has no measurable age (trunk, detached HEAD,
/// repo open failure). Issue #103: previously this opened a fresh
/// `git2::Repository` per row per frame; the libgit2 work now happens
/// once at `worktree::list()` time.
fn branch_age_label(w: &WorktreeInfo) -> String {
  w.age
    .map(worktree::format_relative_duration)
    .unwrap_or_else(|| "-".into())
}

fn branch_age_color(w: &WorktreeInfo, theme: &Theme) -> Color {
  w.age.map(|age| freshness_color(age, theme)).unwrap_or(theme.muted)
}

fn badges_line(w: &WorktreeInfo, theme: &Theme) -> Line<'static> {
  let mut spans: Vec<Span<'static>> = Vec::new();
  // Status sigil:
  //   `?`     — unknown
  //   `●`     — dirty (working tree or index)
  //   `✓`     — synced / clean (no divergence)
  //   (none)  — ahead / behind / both — the label already carries `↑N` /
  //             `↓M` / `↑N ↓M`. Prefixing `✓` here would lie about
  //             divergence (raised by PR #70 Copilot review).
  let status_label = branch_status_label(&w.status);
  let status_color = branch_status_color(&w.status, theme);
  let is_diverged = w.status.has_upstream && (w.status.ahead > 0 || w.status.behind > 0);
  let badge_text = if w.status.unknown {
    format!("? {}", status_label)
  } else if w.status.is_dirty {
    format!("● {}", status_label)
  } else if is_diverged {
    status_label
  } else {
    format!("✓ {}", status_label)
  };
  spans.push(Span::styled(badge_text, Style::default().fg(status_color)));

  let sep = || Span::styled("  ".to_string(), Style::default().fg(theme.muted));
  if w.is_main {
    spans.push(sep());
    spans.push(Span::styled("★ main".to_string(), Style::default().fg(theme.main)));
  }
  if w.is_locked {
    spans.push(sep());
    spans.push(Span::styled("🔒 locked".to_string(), Style::default().fg(theme.locked)));
  }
  if w.is_prunable {
    spans.push(sep());
    spans.push(Span::styled(
      "⚠ prunable".to_string(),
      Style::default().fg(theme.prunable),
    ));
  }
  Line::from(spans)
}

fn working_tree_lines(w: &WorktreeInfo, theme: &Theme) -> (Vec<Line<'static>>, WorkingTreeCounts) {
  match worktree::git_status_short(&w.path) {
    Ok((s, _)) if s.trim().is_empty() => (
      vec![Line::from(Span::styled(
        "✓ clean".to_string(),
        Style::default().fg(theme.clean),
      ))],
      WorkingTreeCounts::default(),
    ),
    Ok((s, scan_truncated)) => {
      let counts = working_tree_status_counts(&s);
      let records = wt_tree::parse_status_z(&s);
      // Cap the explorer for a pathological untracked-dir explosion (issue
      // #300): build at most WT_TREE_MAX_FILES leaves and surface the
      // remainder as a single muted `… N more` row, so the non-scrollable
      // section can't be sized from tens of thousands of files.
      let (tree, overflow) = wt_tree::build_capped_tree(&records, wt_tree::WT_TREE_MAX_FILES);
      let mut lines = working_tree_tree_lines(&tree, theme);
      if overflow > 0 {
        // After a scan truncation the real remainder is unknown (git was
        // killed at the cap), so `overflow` is only a lower bound — render
        // `… N+ more` rather than claiming an exact count.
        let label = if scan_truncated {
          format!("… {}+ more", overflow)
        } else {
          format!("… {} more", overflow)
        };
        lines.push(Line::from(Span::styled(label, Style::default().fg(theme.muted))));
      }
      (lines, counts)
    }
    Err(e) => (
      vec![Line::from(Span::styled(
        format!("! {}", e),
        Style::default().fg(theme.prunable),
      ))],
      WorkingTreeCounts::default(),
    ),
  }
}

/// Render the Working Tree file-explorer model (issue #300) into styled
/// sidebar rows.
///
/// - **Connector lines**: each row is prefixed with box-drawing branches
///   (`├─ ` / `└─ ` with `│  ` / `   ` carried down from ancestors) in the
///   muted role, so the hierarchy reads like `tree(1)`.
/// - **Directory colour is retroactive**: a folder is painted by the
///   aggregate git category of its subtree — only-modified → yellow,
///   only-new → green, only-deleted → red, mixed (or none) → neutral
///   `accent`.
/// - **Files** carry a category-coloured status badge + a nerd-font
///   file-type icon + the leaf name, painted in the file's change-category
///   colour so a row's colour matches the footer count it belongs to (the
///   #287 invariant, preserved).
/// - An **extra space** follows each nerd-font glyph: most glyphs render
///   double-width but occupy a single terminal cell, so the pad keeps the
///   following text from being clipped.
fn working_tree_tree_lines(nodes: &[WtNode], theme: &Theme) -> Vec<Line<'static>> {
  let mut out = Vec::new();
  push_wt_nodes(&mut out, nodes, String::new(), theme);
  out
}

/// Depth-first walk used by [`working_tree_tree_lines`]. `prefix` is the
/// accumulated ancestor connector string; each child appends `├─ `/`└─ `
/// for its own row and `│  `/`   ` for its descendants.
fn push_wt_nodes(out: &mut Vec<Line<'static>>, nodes: &[WtNode], prefix: String, theme: &Theme) {
  let last = nodes.len().saturating_sub(1);
  for (i, node) in nodes.iter().enumerate() {
    let is_last = i == last;
    let connector = format!("{}{}", prefix, if is_last { "└─ " } else { "├─ " });
    match node {
      WtNode::Dir {
        name,
        children,
        category,
      } => {
        let color = match category {
          Some(c) => working_tree_category_color(*c, theme),
          None => theme.accent,
        };
        out.push(Line::from(vec![
          Span::styled(connector, Style::default().fg(theme.muted)),
          Span::styled(
            format!("{}  {}", WT_DIR_OPEN_ICON, wt_tree::sanitize_name(name)),
            Style::default().fg(color),
          ),
        ]));
        let child_prefix = format!("{}{}", prefix, if is_last { "   " } else { "│  " });
        push_wt_nodes(out, children, child_prefix, theme);
      }
      WtNode::File {
        name,
        icon,
        badge,
        category,
      } => {
        let color = working_tree_category_color(*category, theme);
        out.push(Line::from(vec![
          Span::styled(connector, Style::default().fg(theme.muted)),
          Span::styled(format!("{} ", badge), Style::default().fg(color)),
          Span::styled(
            format!("{}  {}", icon, wt_tree::sanitize_name(name)),
            Style::default().fg(color),
          ),
        ]));
      }
    }
  }
}

/// Per-category counts of changed files in the Working Tree pane (issue
/// #287), derived from `git status --short`. Each tracked / untracked file
/// is counted once, into the single category that dominates its porcelain
/// `XY` status pair.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkingTreeCounts {
  /// Untracked or added files (`??`, or `A` in either column).
  pub created: usize,
  /// Files changed in place (`M`, `R`, `C`, `T`, `U`, …).
  pub modified: usize,
  /// Files removed (`D` in either column).
  pub deleted: usize,
}

impl WorkingTreeCounts {
  /// True when no file falls in any category — a clean (or empty) status,
  /// so the Working Tree footer renders nothing rather than a bare ` 0 `.
  pub fn is_empty(&self) -> bool {
    self.created == 0 && self.modified == 0 && self.deleted == 0
  }
}

/// Nerdfont codicon glyphs for the Working Tree footer counts (issue #287):
/// `diff-added` / `diff-modified` / `diff-removed`, the purpose-built file-
/// status trio.
pub const WT_CREATED_ICON: &str = "\u{eadc}";
pub const WT_MODIFIED_ICON: &str = "\u{eadd}";
pub const WT_DELETED_ICON: &str = "\u{eade}";

/// Theme colour for a change category (issue #287): created → `untracked`
/// (green), modified → `modified` (yellow), deleted → `prunable` (red).
fn working_tree_category_color(cat: WtCategory, theme: &Theme) -> Color {
  match cat {
    WtCategory::Created => theme.untracked,
    WtCategory::Modified => theme.modified,
    WtCategory::Deleted => theme.prunable,
  }
}

/// Tally `git status --porcelain -z` output into per-category
/// [`WorkingTreeCounts`] (issue #287) via [`working_tree_category`]. Shares
/// the NUL-delimited parser ([`wt_tree::parse_status_z`]) with the file
/// tree, so a rename counts once (its source token is dropped) and the
/// footer total always matches the number of rows the tree renders.
pub fn working_tree_status_counts(status_z: &str) -> WorkingTreeCounts {
  let mut c = WorkingTreeCounts::default();
  for rec in wt_tree::parse_status_z(status_z) {
    match working_tree_category(rec.x, rec.y) {
      WtCategory::Created => c.created += 1,
      WtCategory::Modified => c.modified += 1,
      WtCategory::Deleted => c.deleted += 1,
    }
  }
  c
}

/// Build the Working Tree pane footer (issue #287): per-category file
/// counts as colour-coded nerdfont segments — created (green / `untracked`
/// role), modified (yellow / `modified` role), deleted (red / `prunable`
/// role). Each segment renders only when its count is non-zero; an all-zero
/// (clean) tally yields `None` so the footer disappears entirely instead of
/// showing a bare ` 0 `.
pub fn working_tree_counts_footer(counts: &WorkingTreeCounts, theme: &Theme) -> Option<Line<'static>> {
  if counts.is_empty() {
    return None;
  }
  let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
  if counts.created > 0 {
    spans.push(Span::styled(
      format!("{} {} ", WT_CREATED_ICON, counts.created),
      Style::default().fg(theme.untracked),
    ));
  }
  if counts.modified > 0 {
    spans.push(Span::styled(
      format!("{} {} ", WT_MODIFIED_ICON, counts.modified),
      Style::default().fg(theme.modified),
    ));
  }
  if counts.deleted > 0 {
    spans.push(Span::styled(
      format!("{} {} ", WT_DELETED_ICON, counts.deleted),
      Style::default().fg(theme.prunable),
    ));
  }
  Some(Line::from(spans))
}

/// Colourise one `git status --short` porcelain line (issue #179, recoloured
/// in #287).
///
/// The short format is `XY<space>PATH`. The whole row — both status columns
/// and the file name — is painted by the file's single change category, so
/// a row's colour always equals the Working-Tree footer segment it's
/// counted in:
///
/// - created (`??` / `A`) → green (`untracked` role),
/// - modified (`M`, `R`, `C`, `T`, `U`, …) → yellow (`modified` role),
/// - deleted (`D`) → red (`prunable` role).
///
/// Precedence created > deleted > modified mirrors
/// [`working_tree_status_counts`] via the shared [`working_tree_category`].
/// The pre-#287 staged-vs-worktree (cyan `X` column) distinction is dropped
/// in favour of this add/modify/delete scheme. The separator space is left
/// unstyled; the rendered text is byte-for-byte identical to the input.
pub fn working_tree_status_line(raw: &str, theme: &Theme) -> Line<'static> {
  // Porcelain short output is always `XY<space>PATH` with ASCII status
  // codes, but the helper is `pub` — a non-git caller could pass arbitrary
  // input. Split on char boundaries (not byte offsets) so a multi-byte
  // leading codepoint can never slice mid-character and panic. Anything
  // shorter than the two status columns + separator is rendered verbatim.
  let mut indices = raw.char_indices();
  let (x_at, x) = match indices.next() {
    Some(c) => c,
    None => return Line::from(raw.to_string()),
  };
  let (_y_at, y) = match indices.next() {
    Some(c) => c,
    None => return Line::from(raw.to_string()),
  };
  let (sep_at, sep) = match indices.next() {
    Some(c) => c,
    None => return Line::from(raw.to_string()),
  };
  // Byte offset where the path begins (just past the separator char).
  let path_at = sep_at + sep.len_utf8();

  // One colour for the whole row, from the file's change category — so the
  // row and the footer count agree (issue #287).
  let style = Style::default().fg(working_tree_category_color(working_tree_category(x, y), theme));

  Line::from(vec![
    Span::styled(raw[x_at..sep_at].to_string(), style),
    Span::raw(raw[sep_at..path_at].to_string()),
    Span::styled(raw[path_at..].to_string(), style),
  ])
}

/// Default number of commits pulled into the Recent Commits block — chosen
/// to match lazygit's initial `git log -300` window so the panel stays
/// dense on tall terminals without paginating.
pub const RECENT_COMMITS_LIMIT: usize = 300;

/// Number of hex chars rendered for each commit's SHA in the sidebar.
/// Matches lazygit's `Gui.CommitHashLength` default of 8.
pub const COMMIT_HASH_DISPLAY_LEN: usize = 8;

/// Produce the styled rows of the Recent Commits sidebar block for a
/// worktree, limited to `limit` entries. Each `Line` mirrors lazygit's
/// per-row format:
///
/// ```text
/// <8-char hash>  <author initials>  <graph>  <subject>
/// ```
///
/// where `<graph>` is the per-row output of the topology renderer in
/// [`super::commit_graph`] — a sequence of `2 * (max_pos + 1)` cells
/// drawing `○` / `◎` nodes plus the `│ ─ ╮ ╭ ╯ ╰ …` connectors that
/// link consecutive commits across branch / merge boundaries. The
/// graph width is deterministic on the commit list — independent of
/// terminal width — so the cache stays valid across resizes.
///
/// The subject is **not** truncated here — the renderer relies on
/// ratatui's view-level hard-clip (no `Wrap`) to match lazygit's gocui
/// behaviour: one commit per visual line, overflow cut at the right
/// edge without `…`.
pub fn recent_commits_lines(w: &WorktreeInfo, limit: usize, theme: &Theme) -> Vec<Line<'static>> {
  match worktree::recent_commits_cached(w, limit) {
    Ok(rows) if !rows.is_empty() => {
      let graphs = super::commit_graph::render_commits(&rows, theme);
      rows
        .into_iter()
        .zip(graphs)
        .map(|(row, graph_spans)| commit_row_line(row, graph_spans, theme))
        .collect()
    }
    Ok(_) => vec![Line::from(Span::styled(
      "(no commits)".to_string(),
      Style::default().fg(theme.muted),
    ))],
    Err(e) => vec![Line::from(Span::styled(
      format!("! {}", e),
      Style::default().fg(theme.prunable),
    ))],
  }
}

fn commit_row_line(row: worktree::CommitRow, graph: Vec<Span<'static>>, theme: &Theme) -> Line<'static> {
  let mut short_hash = row.hash.to_string();
  short_hash.truncate(COMMIT_HASH_DISPLAY_LEN);
  let initials = author_initials(&row.author);
  let mut spans: Vec<Span<'static>> = Vec::with_capacity(5 + graph.len());
  spans.push(Span::styled(short_hash, Style::default().fg(theme.dirty)));
  spans.push(Span::raw("  "));
  spans.push(Span::styled(
    format!("{:<2}", initials),
    Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
  ));
  spans.push(Span::raw("  "));
  spans.extend(graph);
  spans.push(Span::raw(" "));
  spans.push(Span::raw(row.subject));
  Line::from(spans)
}

/// Derive lazygit-style author initials from a full name. Closely
/// mirrors `getInitials` in lazygit's
/// `pkg/gui/presentation/authors/authors.go`:
///
/// - Empty / whitespace-only → empty.
/// - Single word → first 2 Unicode scalar values of that word.
/// - ≥ 2 words → first scalar of split[0] + first scalar of split[1].
///
/// "Kylian Bardini" → `KB`. "Linus" → `Li`. "🦀 Crab" → `🦀C`.
/// Capped at 2 visible characters (`CommitAuthorShortLength` in
/// lazygit).
///
/// **Divergence from lazygit** (PR #72 review, Copilot): lazygit uses
/// `uniseg.FirstGraphemeClusterInString` and keeps multi-scalar
/// grapheme clusters intact (e.g. regional-indicator flags like
/// "🇫🇷"). gwm slices on Unicode scalar values via `str::chars()`,
/// so the French flag is split into its two regional indicators and
/// only the first survives. We accept this divergence intentionally
/// — pulling in `unicode-segmentation` for a near-zero-impact author
/// renderer would inflate the dependency tree without user-visible
/// benefit on the typical "FirstName LastName" pattern.
pub fn author_initials(author: &str) -> String {
  let trimmed = author.trim();
  if trimmed.is_empty() {
    return String::new();
  }
  let mut parts = trimmed.split_whitespace();
  let first = parts.next().unwrap_or("");
  match parts.next() {
    Some(second) => {
      let a: String = first.chars().take(1).collect();
      let b: String = second.chars().take(1).collect();
      format!("{}{}", a, b)
    }
    None => first.chars().take(2).collect(),
  }
}

/// Replace the user's home prefix with `~` so paths render compactly in
/// the narrow sidebar. Falls back to the raw path if `$HOME` is unset or
/// the path doesn't live under it.
fn tilde_compress(path: &str) -> String {
  if let Some(home) = dirs::home_dir() {
    tilde_compress_with_home(path, &home)
  } else {
    path.to_string()
  }
}

/// Pure variant of [`tilde_compress`] that takes the home directory
/// explicitly. Exposed for tests — the production `tilde_compress`
/// wrapper just looks up `dirs::home_dir()` and delegates.
///
/// Enforces a path-separator boundary at the end of the home prefix so
/// `/home/al` does not slice into `/home/alice/repo` and produce
/// `~ice/repo` (raised by PR #70 Copilot review).
pub fn tilde_compress_with_home(path: &str, home: &std::path::Path) -> String {
  let home_s = home.display().to_string();
  if let Some(rest) = path.strip_prefix(&home_s) {
    // Accept exact-home (`rest.is_empty()`) and home-followed-by-separator
    // matches. Reject prefix matches that bleed into a longer dir name.
    if rest.is_empty() || rest.starts_with('/') || rest.starts_with(std::path::MAIN_SEPARATOR) {
      return format!("~{}", rest);
    }
  }
  path.to_string()
}

fn short_oid(oid: &str) -> String {
  oid.chars().take(7).collect()
}

fn branch_status_label(s: &BranchStatus) -> String {
  if s.unknown {
    return "unknown".into();
  }
  let mut parts: Vec<String> = Vec::new();
  if s.is_dirty {
    parts.push("dirty".into());
  }
  if s.has_upstream {
    if s.ahead > 0 {
      parts.push(format!("↑{}", s.ahead));
    }
    if s.behind > 0 {
      parts.push(format!("↓{}", s.behind));
    }
    if !s.is_dirty && s.synced() {
      parts.push("synced".into());
    }
  } else if !s.is_dirty {
    parts.push("clean".into());
  }
  if parts.is_empty() {
    "clean".into()
  } else {
    parts.join(" ")
  }
}

/// Worst-status accent colour for a [`BranchStatus`]: `unknown` → `muted`,
/// `dirty`/`behind` → `dirty`, `ahead`-only → `accent`, else `clean`. The
/// single source of truth shared by the sidebar status badge (`badges_line`)
/// and the table status cell ([`format_status`], issue #241) — each builds its
/// own label/sigils, but the colour is derived here once. Exported so the
/// dedup is pinned by `tests/tui_theme_audit_tests.rs` (both call sites are
/// private render code).
pub fn branch_status_color(s: &BranchStatus, theme: &Theme) -> Color {
  if s.unknown {
    theme.muted
  } else if s.is_dirty || s.behind > 0 {
    theme.dirty
  } else if s.ahead > 0 {
    theme.accent
  } else {
    theme.clean
  }
}

/// Constraint-friendly column width based on observed content, clamped to [min, max].
fn column_width<'a>(items: impl Iterator<Item = &'a str>, min: u16, max: u16) -> u16 {
  let observed = items.map(|s| s.chars().count() as u16).max().unwrap_or(min);
  observed.clamp(min, max)
}

/// Style for the worktree *name* — the row's primary identity text in
/// the table and the sidebar header. Uses the `name` role (default
/// `White`, issue #210), rendered bold so the name anchors each row.
/// Extracted so the role wiring is unit-testable (`build_row` /
/// `sidebar_header_line` are private render code).
pub fn worktree_name_style(theme: &Theme) -> Style {
  Style::default().fg(theme.name).add_modifier(Modifier::BOLD)
}

/// Style for the table's worktree *path* column. Uses the `path` role
/// (default `Gray`, issue #210) — a structural mid-grey distinct from
/// `muted` (`DarkGray`). Extracted alongside [`worktree_name_style`]
/// for the same testability reason.
pub fn worktree_path_style(theme: &Theme) -> Style {
  Style::default().fg(theme.path)
}

/// The shared "chip" style: a reverse-video, bold badge painted on `color`
/// (issue #240). This is the single source of truth for the `` key `` /
/// button / badge treatment that recurs across the header, footer,
/// statusbar, help overlay and modal buttons — `REVERSED` paints `color`
/// as the chip's background, `BOLD` keeps the glyph legible against it.
/// Extracted so the ~14 inline `fg(c).add_modifier(REVERSED | BOLD)`
/// repetitions resolve through one definition; sites that add a `bg` or
/// extra modifiers keep their bespoke style.
pub fn chip_style(color: Color) -> Style {
  Style::default()
    .fg(color)
    .add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/// The hint *bind* style (issue #279): the accent-coloured, **bold** key
/// glyph that leads every statusbar / modal hint. This replaces the
/// pre-#279 reverse-video [`chip_style`] badge with a flat herdr-style
/// "accent bind + space + muted action" treatment — no box around the key.
/// Action *buttons* (Create / confirm / type selector) and the statusbar
/// context anchor keep [`chip_style`]; only the which-key hints are flat.
pub fn hint_key_style(theme: &Theme) -> Style {
  Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
}

/// The hint *action* style (issue #279): the muted description trailing a
/// [`hint_key_style`] bind. Routed through the `muted` role so a theme
/// override recolours it with the rest of the dim chrome.
pub fn hint_label_style(theme: &Theme) -> Style {
  Style::default().fg(theme.muted)
}

/// Style for a *non-highlighted* command name in the command palette
/// (issue #210 follow-up). Routes through the `name` role (default
/// `White`) so a `[theme]` override / light preset recolours it, instead
/// of the pre-#240 hard-coded `Color::White` that bypassed the theme.
/// Extracted so the route is pinned by `tests/tui_theme_audit_tests.rs`
/// (`draw_command_palette` is private Frame render code).
pub fn palette_name_style(theme: &Theme) -> Style {
  Style::default().fg(theme.name)
}

/// Style for a Keybindings-overlay entry *label* (the action description
/// trailing each key chip). Routes through the `name` role (default
/// `White`) for the same reason as [`palette_name_style`]: the pre-#240
/// literal `Color::White` ignored a `[theme]` override. Pinned by
/// `tests/tui_theme_audit_tests.rs`.
pub fn help_label_style(theme: &Theme) -> Style {
  Style::default().fg(theme.name)
}

/// Label + freshness for a worktree's AGENT cell (issue #408). `None` — for
/// a missing snapshot (startup) or a session-less worktree — renders an empty
/// cell, indistinguishable from today (spec US1 scenario 5). Pure so the
/// contract is pinned ratatui-free by `tests/tui_app_tests.rs`.
pub fn agent_cell_label(
  agents: Option<&crate::agent_sessions::WorktreeAgents>,
  now: std::time::SystemTime,
) -> Option<(&'static str, crate::agent_sessions::Freshness)> {
  let top = agents?.top()?;
  let freshness = crate::agent_sessions::Freshness::classify(top.last_activity, top.ended, now);
  Some((top.kind.display(), freshness))
}

/// The mark cell for one row (issue #484). Plain `✓` in the danger role: the
/// only verb that reads the mark set is the destructive one, so the column
/// says up front what the batch is for. Unmarked rows keep the slot blank so
/// the columns stay aligned.
fn mark_cell(marked: bool, theme: &Theme) -> Cell<'static> {
  if marked {
    Cell::from("✓").style(Style::default().fg(theme.prunable).add_modifier(Modifier::BOLD))
  } else {
    Cell::from("")
  }
}

/// The note marker (issue #515). Binary by design: this row carries a note
/// or it does not — no preview, no length, no freshness colour, and no
/// second meaning layered onto a glyph that already has one (`★`, `●` and
/// `✓` are all spoken for). It paints with the neutral `name` role, the
/// same one the empty I/P slots use, because presence is not a status.
///
/// A row without a note in a shown column renders an empty cell so the
/// columns stay aligned — the rule [`mark_cell`] follows.
fn note_cell(has_note: bool, theme: &Theme) -> Cell<'static> {
  if has_note {
    Cell::from("≡").style(Style::default().fg(theme.name))
  } else {
    Cell::from("")
  }
}

/// The three width-constrained column budgets a row truncates against.
/// Grouped rather than passed one by one so the mark column (#484) could join
/// `build_row`'s signature without pushing it past the argument limit.
#[derive(Debug, Clone, Copy)]
struct RowWidths {
  name: u16,
  branch: u16,
  status: u16,
}

/// Build one worktree table row. In workspace mode (issue #36) `repo` is
/// `Some((name, width))` and a leading `REPO` cell is inserted after the age
/// column, painted in the `accent` role; in single-repo mode it is `None` and
/// the row keeps its historical shape.
fn build_row(
  w: &WorktreeInfo,
  // #484: `Some(is_marked)` while the mark column is shown (i.e. at least one
  // row is marked anywhere in the list), `None` when it is absent entirely.
  mark: Option<bool>,
  repo: Option<(&str, u16)>,
  widths: RowWidths,
  // Outer `Option` = is the AGENT column shown at all (round D:
  // conditional on any detected session); inner = this row's top agent.
  agent: Option<Option<(&'static str, crate::agent_sessions::Freshness)>>,
  // #515: is the note column shown at all (any visible row carries one)?
  // The row's own answer is `w.has_note`.
  show_note: bool,
  theme: &Theme,
) -> Row<'static> {
  let RowWidths {
    name: name_w,
    branch: branch_w,
    status: status_w,
  } = widths;
  let marker = table_marker(w, theme);
  let branch_text = w.branch.clone().unwrap_or_else(|| "-".into());

  // The worktree name is the row's primary identity text. It paints with
  // the `name` role (issue #210; default `White`, bold) so a `[theme]`
  // override / preset can recolour it.
  let name_cell = Cell::from(trunc(&w.name, name_w as usize)).style(worktree_name_style(theme));

  // Issue #73: branch column tracks the worst-state colour so the
  // colour-coded signal is visible without expanding the sidebar.
  let branch_cell =
    Cell::from(trunc(&branch_text, branch_w as usize)).style(Style::default().fg(branch_name_color(&w.status, theme)));

  let status_cell = build_status_cell(w, status_w as usize, theme);

  // PR #74 follow-up: surface branch age right in the table so it stays
  // visible when the sidebar is hidden (<120 cols or `v` collapsed).
  // Issue #103: `w.age` is now pre-computed at `worktree::list()` time,
  // so the table render path is pure field access — no libgit2 handle is
  // opened per row per frame. Colour stays uniform Gray — the saturated
  // freshness palette (green/yellow/darkgray) reads as noise next to the
  // more important BRANCH-status colour, so we keep it muted in the table
  // and let the sidebar's `Created:` row carry the colour-coded signal.
  let age_label = w.age.map(format_relative_duration_str).unwrap_or_else(|| "-".into());
  let age_cell = Cell::from(age_label).style(Style::default().fg(theme.muted));

  // The path column paints with the `path` role (issue #210; default
  // `Gray`) — a structural mid-grey distinct from `muted`/`DarkGray`.
  // Not width-constrained, so it does not pass through `trunc`'s funnel and
  // has to say so itself: a path carries the worktree directory name, which is
  // as unvetted as the branch (issue #506).
  let path_cell =
    Cell::from(crate::naming::sanitise_for_terminal(&w.path.to_string_lossy())).style(worktree_path_style(theme));

  let mut cells = Vec::with_capacity(8);
  if let Some(marked) = mark {
    cells.push(mark_cell(marked, theme));
  }
  cells.push(age_cell);
  if let Some((repo_name, repo_w)) = repo {
    cells.push(
      Cell::from(trunc(repo_name, repo_w as usize))
        .style(Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
    );
  }
  cells.push(Cell::from(marker));
  if show_note {
    cells.push(note_cell(w.has_note, theme));
  }
  cells.push(name_cell);
  cells.push(branch_cell);
  cells.push(status_cell);
  // AGENT (issue #408): the most recently active session's agent, coloured by
  // freshness — `clean` (active) vs `muted` (idle). Sessionless row in a
  // shown column → empty cell; column hidden → no cell at all (round D).
  if let Some(agent) = agent {
    cells.push(match agent {
      Some((label, crate::agent_sessions::Freshness::Active)) => {
        Cell::from(label).style(Style::default().fg(theme.clean).add_modifier(Modifier::BOLD))
      }
      Some((label, crate::agent_sessions::Freshness::Idle)) => {
        Cell::from(label).style(Style::default().fg(theme.muted))
      }
      None => Cell::from(""),
    });
  }
  cells.push(path_cell);
  Row::new(cells)
}

/// Owned-String wrapper around `worktree::format_relative_duration` so
/// the table-row builder can hand a `Cell::from` an owned value without
/// re-allocating downstream. Centralised here purely to keep `build_row`
/// readable.
fn format_relative_duration_str(d: std::time::Duration) -> String {
  worktree::format_relative_duration(d)
}

fn build_status_cell(w: &WorktreeInfo, width: usize, theme: &Theme) -> Cell<'static> {
  // Priority: prunable > locked > dirty/sync info.
  if w.is_prunable {
    return Cell::from("prunable").style(Style::default().fg(theme.prunable).add_modifier(Modifier::BOLD));
  }
  if w.is_locked {
    return Cell::from("locked").style(Style::default().fg(theme.locked));
  }

  let s = &w.status;
  let (label, color) = format_status(s, width, theme);
  Cell::from(label).style(Style::default().fg(color))
}

/// Pick a compact label + accent colour for a `BranchStatus`. The colour is
/// derived through the shared [`branch_status_color`] so the table cell and
/// the sidebar status agree (issue #241); the label/sigil logic stays
/// table-specific (the sidebar builds its own badge in `badges_line`).
/// Exported so the colour route is pinned by `tests/tui_theme_audit_tests.rs`.
pub fn format_status(s: &BranchStatus, width: usize, theme: &Theme) -> (String, Color) {
  if s.unknown {
    return ("unknown".into(), theme.muted);
  }

  let mut parts: Vec<String> = Vec::new();
  if s.is_dirty {
    parts.push("● dirty".into());
  }
  if s.has_upstream {
    if s.ahead > 0 {
      parts.push(format!("↑{}", s.ahead));
    }
    if s.behind > 0 {
      parts.push(format!("↓{}", s.behind));
    }
    if !s.is_dirty && s.synced() {
      parts.push("✓ synced".into());
    }
  } else if !s.is_dirty {
    parts.push("clean".into());
  }

  let joined = parts.join(" ");
  let label = trunc(&joined, width.max(4));

  // Worst-status colour, shared with the sidebar (issue #241). `unknown` was
  // already handled by the early return above, so reaching `branch_status_color`
  // here is byte-identical to the former inline `dirty/behind → ahead → clean`
  // chain while keeping a single source of truth.
  (label, branch_status_color(s, theme))
}

/// One statusbar hint specification (issue #217). Either a rebindable
/// keymap [`Action`](super::keymap::Action) whose key is resolved live from
/// the keymap, or a fixed literal for keys that are hard-coded contextual
/// escape hatches (Esc / Enter / digits inside a modal) and so cannot be
/// rebound.
#[derive(Debug, Clone, Copy)]
enum Hint {
  /// Resolve the displayed key from the global keymap (honours `[tui.keys]`).
  Key(super::keymap::Action, &'static str),
  /// Resolve the displayed key from the contextual modal keymap (honours
  /// `[tui.keys.modal.<context>]`, issue #219). Used for modal verbs whose hint
  /// is a single rebindable key (cancel / submit / confirm / issue / pr).
  Modal(ModalAction, &'static str),
  /// A fixed key + label for a non-rebindable keystroke or a multi-key
  /// movement pair (`↑/↓`, `j/k`) that no single resolved key captures.
  Lit(&'static str, &'static str),
}

/// Which pane / mode / overlay the TUI is in — the single source the help
/// overlay subtitle and the contextual statusbar both read (issue #217).
/// Keeping them on one enum means the discoverable hints (`?`) and the
/// always-on statusbar chips can never advertise a different verb set for
/// the same context. An open modal takes priority over the pane focus (see
/// [`App::hint_context`](super::app::App::hint_context)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintContext {
  /// Worktree table focused — the default list-view context.
  Worktrees,
  /// Status (sidebar) pane focused — `j` / `k` scroll the preview.
  Status,
  /// `gwm switch` picker — mutating verbs are inert, Enter/Esc pick/cancel.
  Picker,
  /// Create-worktree form modal, structured `<type> <issue> <desc>` mode.
  Create,
  /// Create-worktree form modal, free-form mode (issue #416). A separate
  /// context because the two modes present different inputs: free-form has
  /// one field and no type selector, so the `field` / `type` hints would
  /// name verbs that do nothing there.
  CreateFreeform,
  /// Confirm-delete modal.
  Confirm,
  /// Open issue/PR URL menu.
  OpenMenu,
  /// Issue/PR link prompt, stage 1 — choose issue vs PR.
  LinkPrompt,
  /// Issue/PR link prompt, stage 2 — typing the number (#219): submit /
  /// cancel resolve from `[tui.keys.modal.link.input_number]`, not the choose
  /// stage's keys.
  LinkInputNumber,
  /// Command palette overlay.
  CommandPalette,
  /// Bootstrap report overlay.
  Report,
  /// Keybindings help overlay.
  Help,
  /// PTY overlay (embedded lazygit / terminal). All keys pass through to the
  /// child process; Esc is the only gwm-level escape hatch.
  Pty,
  /// Exec profile picker overlay (issue #325): j/k pick, Enter runs, Esc
  /// cancels.
  ExecPicker,
  /// Clean reclaim overlay (issue #325): j/k pick a profile, confirm
  /// reclaims (safety countdown), Esc cancels.
  Clean,
  /// Branch-rename modal (`View::Edit`, #290), structured triple.
  Rename,
  /// Branch-rename modal in free-form mode (issue #479). A separate context
  /// for the same reason `CreateFreeform` is one: the two modes present
  /// different inputs, so advertising a type selector free-form does not
  /// render would name a key that does nothing.
  RenameFreeform,
  /// Generic detail overlay (issue #408): scroll / close. Agent sessions
  /// today, the rich PR/Issue view tomorrow.
  Detail,
  /// CI checks overlay (issue #436): the detail-overlay shell on the linked
  /// PR's per-check rollup — j/k select, Enter opens the details URL,
  /// f filters, Esc closes.
  CiChecks,
  /// Rich PR / issue view (issue #420): the same shell on the linked
  /// PR's or issue's description, reviews and conversation — j/k select,
  /// Enter opens the row's URL, f re-fetches, Esc closes.
  RichView,
  /// The in-TUI note editor (issue #515): every printable is text, so the
  /// only verbs advertised are the two ways out.
  Note,
}

impl HintContext {
  /// Short label rendered into the statusbar context chip and the help
  /// overlay subtitle.
  pub fn label(self) -> &'static str {
    match self {
      HintContext::Worktrees => "worktrees",
      HintContext::Status => "status",
      HintContext::Picker => "switch",
      HintContext::Create | HintContext::CreateFreeform => "create",
      HintContext::Confirm => "confirm",
      HintContext::OpenMenu => "open",
      HintContext::LinkPrompt => "link",
      HintContext::LinkInputNumber => "link",
      HintContext::CommandPalette => "command",
      HintContext::Report => "report",
      HintContext::Help => "help",
      HintContext::Pty => "terminal",
      HintContext::ExecPicker => "exec",
      HintContext::Clean => "clean",
      HintContext::Rename | HintContext::RenameFreeform => "rename",
      HintContext::Detail => "agents",
      HintContext::CiChecks => "checks",
      HintContext::RichView => "pr/issue",
      HintContext::Note => "note",
    }
  }

  /// Static hint specs for this context. List-view contexts use rebindable
  /// [`Hint::Key`] verbs (resolved against the global keymap); modal /
  /// overlay contexts use [`Hint::Modal`] for their single-key rebindable
  /// verbs (resolved against the contextual keymap, issue #219) and
  /// [`Hint::Lit`] only for movement pairs (`↑/↓`, `j/k`) and the genuinely
  /// hard-coded escape hatches (the PTY overlay's `Esc`). All resolved live
  /// by [`Self::resolve`].
  fn hint_specs(self) -> &'static [Hint] {
    use super::keymap::Action::*;
    match self {
      // Grouped by family, most-used verb of each first; the order doubles as
      // the right-to-left truncation priority (#290 footer reorg).
      HintContext::Worktrees => &[
        // Worktree lifecycle.
        Hint::Key(Create, "new"),
        Hint::Key(DeleteConfirm, "del"),
        // #484: the mark sits next to the verb it feeds.
        Hint::Key(ToggleSelect, "mark"),
        Hint::Key(Bootstrap, "boot"),
        // Act on the selected worktree. #453 re-audit: exec and agent
        // sessions joined the family; clean / mux / macros stay
        // overlay-only — the footer is a teaser, `?` is the manual.
        Hint::Key(TerminalFullscreen, "open"),
        Hint::Key(LazyGitFullscreen, "git"),
        Hint::Key(ExecOverlay, "exec"),
        Hint::Key(AgentSessions, "agents"),
        // #515: the note is written far more often than a review is
        // launched, so it sits ahead of `review` / `yank` in the
        // right-to-left truncation order rather than at the tail.
        Hint::Key(EditNote, "note"),
        Hint::Key(ReviewFullscreen, "review"),
        Hint::Key(YankPath, "yank"),
        // Find / navigate panes.
        Hint::Key(Filter, "filter"),
        Hint::Key(FocusStatus, "status"),
        Hint::Key(CommandLogs, "logs"),
        Hint::Key(ConfigPanel, "settings"),
        // Global.
        Hint::Key(Help, "help"),
        Hint::Key(Quit, "quit"),
      ],
      HintContext::Status => &[
        // Read the status pane.
        Hint::Key(Down, "scroll"),
        Hint::Key(WtScrollDown, "wt scroll"),
        Hint::Key(FetchGithub, "fetch"),
        // #436: `c` routes to the CI checks overlay in this context.
        Hint::Key(EditWorktree, "ci checks"),
        // Sidebar mode / layout.
        Hint::Key(ToggleSidebarMode, "mode"),
        Hint::Key(CycleSidebarLayout, "layout"),
        // Navigate panes.
        Hint::Key(FocusWorktrees, "worktrees"),
        Hint::Key(Filter, "filter"),
        Hint::Key(CommandLogs, "logs"),
        Hint::Key(ConfigPanel, "settings"),
        // Global.
        Hint::Key(Help, "help"),
        Hint::Key(Quit, "quit"),
      ],
      HintContext::Picker => &[
        // Pick / dismiss.
        Hint::Lit("Enter", "select"),
        Hint::Lit("Esc", "cancel"),
        // Act on the highlighted worktree.
        Hint::Key(TerminalFullscreen, "open"),
        Hint::Key(LazyGitFullscreen, "git"),
        Hint::Key(YankPath, "yank"),
        // Find / global.
        Hint::Key(Filter, "filter"),
        Hint::Key(Help, "help"),
        Hint::Key(Quit, "quit"),
      ],
      // #219: single-key modal verbs use Hint::Modal so a rebind shows
      // through; multi-key movement pairs (↑/↓, j/k, ←/→) stay literal
      // because no single resolved key captures them.
      HintContext::Create => &[
        Hint::Modal(ModalAction::CreateNextField, "field"),
        Hint::Lit("↑/↓", "type"),
        Hint::Modal(ModalAction::CreateToggleMode, "free-form"),
        Hint::Modal(ModalAction::CreateSubmit, "submit"),
        Hint::Modal(ModalAction::CreateCancel, "cancel"),
      ],
      // Free-form has one field and no type selector, so `field` and `type`
      // are dropped rather than shown inert — the same reason `draw_create`
      // stops rendering those rows. `toggle_mode` leads the row: it is the
      // only way back, and unlike the create form's other verbs it is not
      // guessable from the visible inputs.
      HintContext::CreateFreeform => &[
        Hint::Modal(ModalAction::CreateToggleMode, "structured"),
        Hint::Modal(ModalAction::CreateSubmit, "submit"),
        Hint::Modal(ModalAction::CreateCancel, "cancel"),
      ],
      HintContext::Confirm => &[
        Hint::Modal(ModalAction::ConfirmConfirm, "confirm"),
        Hint::Key(ToggleDeleteBranch, "branch"),
        Hint::Lit("←/→", "move"),
        Hint::Modal(ModalAction::ConfirmActivate, "activate"),
        Hint::Modal(ModalAction::ConfirmCancel, "cancel"),
      ],
      HintContext::OpenMenu => &[
        Hint::Modal(ModalAction::OpenMenuIssue, "issue"),
        Hint::Modal(ModalAction::OpenMenuPr, "pr"),
        Hint::Key(FetchGithub, "fetch"),
        Hint::Modal(ModalAction::OpenMenuClose, "close"),
      ],
      HintContext::LinkPrompt => &[
        Hint::Modal(ModalAction::LinkChoosePrev, "prev"),
        Hint::Modal(ModalAction::LinkChooseNext, "next"),
        Hint::Modal(ModalAction::LinkChooseIssue, "issue"),
        Hint::Modal(ModalAction::LinkChoosePr, "pr"),
        Hint::Modal(ModalAction::LinkChooseAccept, "link"),
        Hint::Key(FetchGithub, "fetch"),
        Hint::Modal(ModalAction::LinkChooseCancel, "cancel"),
      ],
      // #219: while typing the number, submit / cancel come from the
      // input-number context — not the choose-target keys above.
      HintContext::LinkInputNumber => &[
        Hint::Lit("0-9", "number"),
        Hint::Modal(ModalAction::LinkInputSubmit, "submit"),
        Hint::Key(FetchGithub, "fetch"),
        Hint::Modal(ModalAction::LinkInputCancel, "cancel"),
      ],
      HintContext::CommandPalette => &[
        Hint::Lit("↑/↓", "move"),
        Hint::Modal(ModalAction::CommandPaletteAccept, "run"),
        Hint::Modal(ModalAction::CommandPaletteClose, "cancel"),
      ],
      // #219: `close` is a single rebindable verb, so it resolves through the
      // modal keymap; the scroll/pan pairs stay literal (no single resolved
      // key captures `j/k` / `h/l`, matching the Create/Confirm convention).
      HintContext::Report => &[Hint::Modal(ModalAction::ReportClose, "close")],
      // #408: detail overlay — selection pair stays literal like Help's;
      // attach / detach / close resolve through the modal keymap so a
      // rebind shows through.
      HintContext::Detail => &[
        Hint::Lit("j/k", "select"),
        Hint::Modal(ModalAction::DetailAttach, "attach"),
        Hint::Modal(ModalAction::DetailDetach, "detach"),
        Hint::Modal(ModalAction::DetailInput, "by id"),
        Hint::Modal(ModalAction::DetailClose, "close"),
      ],
      HintContext::CiChecks => &[
        Hint::Lit("j/k", "select"),
        Hint::Modal(ModalAction::CiChecksOpen, "open"),
        Hint::Modal(ModalAction::CiChecksFilter, "filter"),
        Hint::Modal(ModalAction::CiChecksRefresh, "refresh"),
        Hint::Modal(ModalAction::CiChecksClose, "close"),
      ],
      // #420: no filter verb — a rich view is prose, not a row set.
      HintContext::RichView => &[
        Hint::Lit("j/k", "select"),
        Hint::Modal(ModalAction::RichViewOpen, "open"),
        Hint::Modal(ModalAction::RichViewRefresh, "refresh"),
        Hint::Modal(ModalAction::RichViewClose, "close"),
      ],
      // #515: no verbs beyond the exits — j/k are letters here, and the
      // arrows are hard-coded for the same reason `Esc` is elsewhere.
      HintContext::Note => &[
        Hint::Lit("↑/↓/←/→", "move"),
        Hint::Modal(ModalAction::NoteOpenEditor, "$EDITOR"),
        Hint::Modal(ModalAction::NoteClose, "save & close"),
      ],
      HintContext::Help => &[
        Hint::Lit("j/k", "scroll"),
        Hint::Lit("h/l", "pan"),
        Hint::Modal(ModalAction::HelpClose, "close"),
      ],
      HintContext::Pty => &[Hint::Lit("Esc", "close")],
      // #325: pick a profile then run it in a PTY. The j/k movement pair
      // stays literal (no single resolved key captures it), matching the
      // palette / create convention.
      HintContext::ExecPicker => &[
        Hint::Lit("↑/↓", "pick"),
        Hint::Modal(ModalAction::ExecPickerAccept, "run"),
        Hint::Modal(ModalAction::ExecPickerCancel, "cancel"),
      ],
      // #325: the profile picker pair stays literal; confirm / cancel are
      // rebindable modal verbs (the safety countdown reuses the delete
      // confirm's `y` / Enter convention).
      HintContext::Clean => &[
        Hint::Lit("↑/↓", "profile"),
        Hint::Modal(ModalAction::CleanConfirm, "reclaim"),
        Hint::Modal(ModalAction::CleanCancel, "cancel"),
      ],
      // Rename reuses the create-form input handler, hence the `create`
      // context's verbs (#290 / #219).
      HintContext::Rename => &[
        Hint::Modal(ModalAction::CreateToggleMode, "free-form"),
        Hint::Modal(ModalAction::CreateNextField, "field"),
        Hint::Lit("↑/↓", "type"),
        Hint::Modal(ModalAction::CreateSubmit, "submit"),
        Hint::Modal(ModalAction::CreateCancel, "cancel"),
      ],
      // Free-form has one field and no type selector, so `field` and `type`
      // are left out rather than advertised inert (issue #479).
      HintContext::RenameFreeform => &[
        Hint::Modal(ModalAction::CreateToggleMode, "structured"),
        Hint::Modal(ModalAction::CreateSubmit, "submit"),
        Hint::Modal(ModalAction::CreateCancel, "cancel"),
      ],
    }
  }

  /// Resolve this context's hints to `(key, label)` pairs for the statusbar,
  /// reading the live keymap so rebindable verbs show the user's actual
  /// binding (issue #217 review) — the same `primary_chord` source the help
  /// overlay and the Issue/PR prompt use. An unbound action is dropped from
  /// the row rather than advertised with a phantom key.
  pub fn resolve(self, keymap: &super::keymap::Keymap, modal: &ModalKeymap) -> Vec<(String, String)> {
    self.resolve_with_fields(keymap, modal, &CANONICAL_TRIPLE)
  }

  /// As [`Self::resolve`], but told which fields the create / rename form
  /// actually presents (issue #418), so the structured rows stop advertising
  /// verbs that do nothing.
  ///
  /// Two of them go inert once the field set follows the repo's patterns, and
  /// this codebase's rule is to never name a key that does nothing (the reason
  /// free-form drops the same two rows since #416):
  ///
  /// - `↑/↓ type` when no pattern carries `{type}`. The selector is not
  ///   rendered and `handle_create_key` gates the cycling verbs on
  ///   `Field::Type`, so the arrows are a no-op.
  /// - `field` when the pattern presents one field. `next_field` rotates
  ///   within a one-element list, so Tab does nothing.
  ///
  /// `fields` is ignored outside the structured create / rename contexts:
  /// free-form drops both rows already, and no other context carries them.
  pub fn resolve_with_fields(
    self,
    keymap: &super::keymap::Keymap,
    modal: &ModalKeymap,
    fields: &[Field],
  ) -> Vec<(String, String)> {
    let structured_form = matches!(self, HintContext::Create | HintContext::Rename);
    self
      .hint_specs()
      .iter()
      .filter(|h| {
        if !structured_form {
          return true;
        }
        match h {
          Hint::Lit("↑/↓", "type") => fields.contains(&Field::Type),
          Hint::Modal(ModalAction::CreateNextField, _) => fields.len() > 1,
          _ => true,
        }
      })
      .filter_map(|h| match h {
        // #219: a global verb whose key is claimed by a modal binding in the
        // active context is resolved as that modal verb first — the event loop
        // never reaches the global action. Drop the hint rather than advertise
        // a duplicate key for an unreachable action. Same for a key the
        // context's reserved typing consumes (Codex review #456, iteration
        // 13): `fetch_github` rebound to a digit never fires while typing the
        // link number.
        Hint::Key(action, label) => keymap
          .primary_chord(*action)
          .filter(|k| !self.key_shadowed_by_modal(k, modal) && !self.key_swallowed_by_typing(k))
          .map(|k| (k, label.to_string())),
        Hint::Modal(action, label) => modal.primary_key(*action).map(|k| (k, label.to_string())),
        Hint::Lit(key, label) => Some((key.to_string(), label.to_string())),
      })
      .collect()
  }

  /// The modal [`KeyContext`] this hint context renders, when it is a modal /
  /// overlay surface (the global panes have none). Used to detect a global
  /// hint key shadowed by a modal binding in the same context.
  fn modal_context(self) -> Option<KeyContext> {
    Some(match self {
      HintContext::Create | HintContext::CreateFreeform | HintContext::Rename | HintContext::RenameFreeform => {
        KeyContext::Create
      }
      HintContext::Confirm => KeyContext::Confirm,
      HintContext::OpenMenu => KeyContext::OpenMenu,
      HintContext::LinkPrompt => KeyContext::LinkChooseTarget,
      HintContext::LinkInputNumber => KeyContext::LinkInputNumber,
      HintContext::CommandPalette => KeyContext::CommandPalette,
      HintContext::Report => KeyContext::Report,
      HintContext::Help => KeyContext::Help,
      HintContext::Detail => KeyContext::Detail,
      HintContext::CiChecks => KeyContext::CiChecks,
      HintContext::RichView => KeyContext::RichView,
      HintContext::Note => KeyContext::Note,
      HintContext::ExecPicker => KeyContext::ExecPicker,
      HintContext::Clean => KeyContext::Clean,
      HintContext::Worktrees | HintContext::Status | HintContext::Picker | HintContext::Pty => return None,
    })
  }

  /// `true` when `key` is bound to a modal verb in this context — i.e. the
  /// modal keymap intercepts it before any global action with the same key.
  fn key_shadowed_by_modal(self, key: &str, modal: &ModalKeymap) -> bool {
    match self.modal_context() {
      Some(ctx) => modal
        .bindings_for(ctx)
        .iter()
        .any(|b| b.keys.iter().any(|ks| ks.to_string() == key)),
      None => false,
    }
  }

  /// `true` when this context's reserved typing consumes `key` before any
  /// resolution — a global fallback on it (e.g. `fetch_github` rebound to a
  /// digit at the link number stage) can never fire, so its hint is dead.
  /// A multi-stroke chord dies with its opening stroke: the typing route
  /// eats it before the pending-chord machinery sees it.
  fn key_swallowed_by_typing(self, key: &str) -> bool {
    match self.modal_context() {
      Some(ctx) => KeyStroke::parse_chord(key)
        .ok()
        .and_then(|strokes| strokes.first().cloned())
        .is_some_and(|ks| ctx.reserved_typing_stroke(&ks)),
      None => false,
    }
  }
}

fn action_chord(keymap: &Keymap, action: Action, fallback: &str) -> String {
  keymap.primary_chord(action).unwrap_or_else(|| fallback.to_string())
}

pub fn issue_pr_pane_title(keymap: &Keymap, compact: bool) -> String {
  pane_title(compact, "Issue / PR", &action_chord(keymap, Action::FetchGithub, "F"))
}

pub fn working_tree_pane_title(keymap: &Keymap, compact: bool) -> String {
  pane_title(
    compact,
    "Working Tree",
    &action_chord(keymap, Action::ReviewFullscreen, "R"),
  )
}

pub fn recent_items_pane_title(mode: SidebarMode, keymap: &Keymap, compact: bool) -> String {
  let label = match mode {
    SidebarMode::Commits => "Recent Commits",
    SidebarMode::Stashes => "Stashes",
  };
  pane_title(compact, label, &action_chord(keymap, Action::LazyGitFullscreen, "l"))
}

pub fn modal_hint_line(hints: &[(&str, &str)], theme: &Theme) -> Line<'static> {
  let key_style = hint_key_style(theme);
  let label_style = hint_label_style(theme);
  let mut spans: Vec<Span<'static>> = Vec::new();
  for (i, (key, label)) in hints.iter().enumerate() {
    if i > 0 {
      // Two spaces between hint pairs keep `key action` groups visually
      // distinct now that the badge box is gone (issue #279).
      spans.push(Span::raw("  "));
    }
    spans.push(Span::styled((*key).to_string(), key_style));
    spans.push(Span::styled(format!(" {}", label), label_style));
  }
  Line::from(spans).centered()
}

/// Settings-panel footer hints shown while a field is being edited (#219
/// review): `save` / `cancel` resolve from the `ConfigEdit*` modal bindings so
/// a rebind of `[tui.keys.modal.config.edit]` shows through instead of the literal
/// `Enter` / `Esc`. An unbound verb is dropped rather than advertised with a
/// phantom key, mirroring the statusbar's `HintContext::resolve`.
pub fn config_edit_footer_hints(modal: &ModalKeymap) -> Vec<(String, String)> {
  [
    (ModalAction::ConfigEditSubmit, "save"),
    (ModalAction::ConfigEditCancel, "cancel"),
  ]
  .into_iter()
  .filter_map(|(action, label)| modal.primary_key(action).map(|k| (k, label.to_string())))
  .collect()
}

/// Settings-panel footer hints in *navigation* mode (#219 review): the
/// single-key verbs (`section` / `layer` / `close`, plus `cycle` / `edit` via
/// `activate`) resolve from the `Config*` modal bindings so a rebind of
/// `[tui.keys.modal.config]` shows through. The `j/k` scroll pair stays literal —
/// no single resolved key captures a movement pair (same rule as Help). The
/// leading verb depends on the active tab / field kind, mirroring the historic
/// hard-coded branches.
pub fn config_nav_footer_hints(
  modal: &ModalKeymap,
  tab: SettingsTab,
  selected_kind: Option<FieldKind>,
) -> Vec<(String, String)> {
  let mut hints: Vec<(String, String)> = Vec::new();
  if tab == SettingsTab::All {
    hints.push(("j/k".to_string(), "scroll".to_string()));
  } else {
    let label = if tab == SettingsTab::Keys {
      "rebind"
    } else if selected_kind == Some(FieldKind::Choice) {
      "cycle"
    } else {
      "edit"
    };
    if let Some(k) = modal.primary_key(ModalAction::ConfigActivate) {
      hints.push((k, label.to_string()));
    }
  }
  for (action, label) in [
    (ModalAction::ConfigNextTab, "section"),
    (ModalAction::ConfigToggleLayer, "layer"),
    (ModalAction::ConfigClose, "close"),
  ] {
    if let Some(k) = modal.primary_key(action) {
      hints.push((k, label.to_string()));
    }
  }
  hints
}

/// Settings-panel footer hints while a live keystroke capture is armed on the
/// Keys tab (issue #294). `cancel` (and, for a multi-stroke global chord,
/// `save`) resolve from the `ConfigEdit*` modal bindings so a rebind of
/// `[tui.keys.modal.config.edit]` shows through. A single-stroke modal capture
/// auto-commits on the first key, so it advertises that instead of a `save`
/// verb; the multi-stroke global path adds the literal `Backspace` deletes-last
/// affordance (no modal verb binds it).
pub fn config_capture_footer_hints(modal: &ModalKeymap, single_only: bool) -> Vec<(String, String)> {
  let mut hints: Vec<(String, String)> = Vec::new();
  if single_only {
    hints.push(("any key".to_string(), "bind".to_string()));
  } else {
    if let Some(k) = modal.primary_key(ModalAction::ConfigEditSubmit) {
      hints.push((k, "save".to_string()));
    }
    hints.push(("Backspace".to_string(), "delete".to_string()));
  }
  if let Some(k) = modal.primary_key(ModalAction::ConfigEditCancel) {
    hints.push((k, "cancel".to_string()));
  }
  hints
}

/// Command Logs overlay footer hints (#219 review): `copy` / `close` resolve
/// from the `CommandLogs*` modal bindings so a rebind of
/// `[tui.keys.modal.command_logs]` shows through; the scroll / top-bottom movement
/// pairs stay literal (no single resolved key captures `j/k` / `g/G`).
pub fn command_logs_footer_hints(modal: &ModalKeymap) -> Vec<(String, String)> {
  let mut hints: Vec<(String, String)> = vec![
    ("j/k".to_string(), "scroll".to_string()),
    ("g/G".to_string(), "top/bottom".to_string()),
  ];
  for (action, label) in [
    (ModalAction::CommandLogsCopy, "copy"),
    (ModalAction::CommandLogsClose, "close"),
  ] {
    if let Some(k) = modal.primary_key(action) {
      hints.push((k, label.to_string()));
    }
  }
  hints
}

pub fn modal_hint_for_context(ctx: HintContext, keymap: &Keymap, modal: &ModalKeymap, theme: &Theme) -> Line<'static> {
  modal_hint_for_context_with_fields(ctx, keymap, modal, theme, &CANONICAL_TRIPLE)
}

/// As [`modal_hint_for_context`], for the two footers whose form knows which
/// fields it presents (issue #418). Every other modal keeps the plain call.
pub fn modal_hint_for_context_with_fields(
  ctx: HintContext,
  keymap: &Keymap,
  modal: &ModalKeymap,
  theme: &Theme,
  fields: &[Field],
) -> Line<'static> {
  let resolved = ctx.resolve_with_fields(keymap, modal, fields);
  let hints: Vec<(&str, &str)> = resolved.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
  modal_hint_line(&hints, theme)
}

fn push_modal_hint(
  lines: &mut Vec<Line<'static>>,
  ctx: HintContext,
  keymap: &Keymap,
  modal: &ModalKeymap,
  theme: &Theme,
) {
  lines.push(Line::from(String::new()));
  lines.push(modal_hint_for_context(ctx, keymap, modal, theme));
}

/// Build the single-line statusline (issue #180).
///
/// Layout, left-to-right:
///
/// ```text
///  n  new  d  del  …                                   [<status>]
/// ```
///
/// Each hint renders as a reverse-video badge chip (` key ` painted with the
/// theme `accent` as background via `REVERSED`) followed by a dim label. The
/// status message (the action log) is pinned flush-right and has **absolute
/// priority**: when `width` is too small for every hint, the hint list is cut
/// short with an `…` marker, but the status is always kept — clipped only if
/// it alone exceeds `width`. There is no wrapping: the caller renders this
/// without `Wrap`, so the row is hard-clipped at the terminal edge.
///
/// Pure and width-driven so the contract is pinned by
/// `tests/tui_footer_tests.rs` without spinning up a ratatui backend. Widths
/// are measured with `chars().count()` to match the rest of `ui.rs` (keys,
/// labels and the bracketed status are ASCII / single-width in practice).
pub fn footer_line(hints: &[(&str, &str)], status: &str, width: usize, theme: &Theme) -> Line<'static> {
  let key_style = hint_key_style(theme);
  let label_style = hint_label_style(theme);
  let status_style = Style::default().fg(theme.dirty);

  // A zero-width row can hold nothing — return an empty line rather than let
  // the `trunc()` floor below emit a 1-column `…`.
  if width == 0 {
    return Line::default();
  }

  // Action logs are sometimes error strings carrying embedded newlines /
  // tabs. `Wrap` is disabled, but a raw `\n` would still split the row in
  // two, so collapse every control char to a single space first — the footer
  // must stay one visual line.
  let status: String = status.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
  let status_text = format!("[{}]", status);
  let status_w = status_text.chars().count();

  // Priority floor: if even the status cannot fit, show a clipped status
  // alone — never a hint at the log's expense.
  if width <= status_w {
    return Line::from(Span::styled(trunc(&status_text, width), status_style));
  }

  // Budget for the hint badges: the width left after the right-pinned status,
  // minus one column reserved for the `…` truncation marker. The gap between
  // the hints and the status is best-effort — it shows up as left-over
  // padding only when at least one badge fits; in the tight band just above
  // `status_w` there may be room for neither a badge nor a gap.
  let hint_budget = (width - status_w - 1).saturating_sub(1);

  let mut spans: Vec<Span<'static>> = Vec::new();
  let mut used = 0usize; // display columns consumed by hint groups so far
  let mut truncated = false;
  for (i, (key, label)) in hints.iter().enumerate() {
    let sep = if i > 0 { 2 } else { 0 }; // two spaces between hint groups (#279)
                                         // flat bind `key` + ` label` (label + 1 leading space)
    let badge_w = key.chars().count() + 1 + label.chars().count();
    if used + sep + badge_w > hint_budget {
      truncated = true;
      break;
    }
    if sep > 0 {
      spans.push(Span::raw(" ".repeat(sep)));
      used += sep;
    }
    spans.push(Span::styled((*key).to_string(), key_style));
    spans.push(Span::styled(format!(" {}", label), label_style));
    used += badge_w;
  }

  if truncated {
    if used > 0 {
      spans.push(Span::raw(" "));
      used += 1;
    }
    spans.push(Span::styled("…", label_style));
    used += 1;
  }

  // Pad so the status sits flush right (priority: the log is at the end).
  let pad = width.saturating_sub(used + status_w);
  if pad > 0 {
    spans.push(Span::raw(" ".repeat(pad)));
  }
  spans.push(Span::styled(status_text, status_style));
  Line::from(spans)
}

/// Contextual statusbar (issue #217) — a superset of [`footer_line`] that
/// leads with a context chip and an optional loading spinner. Layout,
/// left-to-right:
///
/// ```text
///  worktrees  ⠋  n  new  d  del  …                       [<status>]
/// ```
///
/// Priority when space is tight, from most to least protected: the status
/// log (right, clipped only if it alone overflows), the context chip and
/// spinner (left, the load-bearing "where am I / am I busy" signals), then
/// the hints (truncated with `…`). Pure + width-driven so the contract is
/// pinned by `tests/tui_footer_tests.rs`; `footer_line` is kept intact for
/// its own callers and tests.
pub fn status_line(
  context: &str,
  hints: &[(&str, &str)],
  status: &str,
  spinner: Option<&str>,
  width: usize,
  theme: &Theme,
) -> Line<'static> {
  let context_style = chip_style(theme.focus);
  let key_style = hint_key_style(theme);
  let label_style = hint_label_style(theme);
  let status_style = Style::default().fg(theme.dirty);
  let spinner_style = Style::default().fg(theme.accent).add_modifier(Modifier::BOLD);

  if width == 0 {
    return Line::default();
  }

  let status: String = status.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
  let status_text = format!("[{}]", status);
  let status_w = status_text.chars().count();

  // Priority floor: if even the status cannot fit, show a clipped status
  // alone — never a chip or hint at the log's expense.
  if width <= status_w {
    return Line::from(Span::styled(trunc(&status_text, width), status_style));
  }

  let avail = width - status_w; // columns to the left of the right-pinned status
  let mut spans: Vec<Span<'static>> = Vec::new();
  let mut used = 0usize;

  // Context chip — load-bearing, kept whenever it fits at all.
  let ctx_chip = format!(" {} ", context);
  let ctx_w = ctx_chip.chars().count();
  if ctx_w <= avail {
    spans.push(Span::styled(ctx_chip, context_style));
    used += ctx_w;
  }

  // Loading spinner — optional, rendered right after the chip when present
  // and there is room.
  if let Some(glyph) = spinner {
    let padded = format!(" {} ", glyph);
    let gw = padded.chars().count();
    if used + gw <= avail {
      spans.push(Span::styled(padded, spinner_style));
      used += gw;
    }
  }

  // Hint badges fill whatever is left, minus one column for the `…` marker.
  let hint_budget = avail.saturating_sub(used).saturating_sub(1);
  let mut truncated = false;
  let mut hint_used = 0usize;
  for (i, (key, label)) in hints.iter().enumerate() {
    // Two spaces between hint groups (#279); a single space after the left
    // cluster (context chip / spinner) before the first hint.
    let sep = if i > 0 { 2 } else { usize::from(used > 0) };
    let badge_w = key.chars().count() + 1 + label.chars().count();
    if hint_used + sep + badge_w > hint_budget {
      truncated = true;
      break;
    }
    if sep > 0 {
      spans.push(Span::raw(" ".repeat(sep)));
      hint_used += sep;
    }
    spans.push(Span::styled((*key).to_string(), key_style));
    spans.push(Span::styled(format!(" {}", label), label_style));
    hint_used += badge_w;
  }
  used += hint_used;
  if truncated {
    if used > 0 {
      spans.push(Span::raw(" "));
      used += 1;
    }
    spans.push(Span::styled("…", label_style));
    used += 1;
  }

  let pad = width.saturating_sub(used + status_w);
  if pad > 0 {
    spans.push(Span::raw(" ".repeat(pad)));
  }
  spans.push(Span::styled(status_text, status_style));
  Line::from(spans)
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
  let ctx = app.hint_context();
  // Spinner shows while any async op is inflight: a GitHub fetch (issue
  // #217) or a generic background task such as the off-thread worktree
  // refresh (issue #231). Both render through the same statusbar spinner +
  // per-op label (carried on `app.status`) so "loading" reads consistently
  // across every async site. The frame advances at the poll cadence.
  let spinner = if app.is_github_loading() || app.is_task_loading() {
    Some(app.spinner.glyph(DOT_FRAMES))
  } else {
    None
  };
  // Resolve the rebindable hint keys against the live keymap (issue #217
  // review) so a user override shows through, then borrow into the slice
  // `status_line` expects.
  // Same field set the two footers use (#418), so the bar behind a modal and
  // the modal's own footer cannot advertise different verbs.
  let resolved = ctx.resolve_with_fields(&app.keymap, &app.modal_keymap, app.create_form.fields());
  let hints: Vec<(&str, &str)> = resolved.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
  let line = status_line(
    ctx.label(),
    &hints,
    &app.status,
    spinner,
    area.width as usize,
    &app.theme,
  );
  // No `Wrap`: the statusbar is a single hard-clipped row (issue #180).
  f.render_widget(Paragraph::new(line), area);
}

/// A single logical row of the help overlay (#187).
///
/// Decouples *what* the overlay documents from *how* it is painted.
/// [`help_rows`] produces this structured form so [`draw_help`] can
/// render coloured section headers and key *badges* (the same chip
/// style as the bottom statusline), while [`help_lines`] flattens it
/// back to the legacy `  {keys:<13} {label}` strings the chord tests
/// in `tests/tui_chord_tests.rs` pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpRow {
  /// Overlay title — always the first row.
  Title(String),
  /// Context subtitle under the title (`worktrees` / `status` / `switch`),
  /// issue #217. Reflects the focused pane / mode when `?` was opened.
  Subtitle(String),
  /// Section header (`global`, `list view`, `issue / PR (#67)`, …).
  Section(String),
  /// Blank spacer row.
  Blank,
  /// A documented binding: the resolved chord(s) and the human label.
  /// `keys` is empty only for an unbound action; the flattening in
  /// [`help_lines`] renders that as `(unbound)`.
  Entry { keys: String, label: String },
}

/// Structured builder for the help overlay (issue #87 logic,
/// restructured into rows in #187).
///
/// Reads every list-view binding from the resolved `Keymap` so user
/// overrides under `[tui.keys]` show through verbatim — a user who
/// rebinds `down = ["Ctrl+n"]` sees `Ctrl+n` next to "next" instead
/// of the historical `j / ↓`. Rows that document non-rebindable
/// surfaces (Ctrl-C escape hatch, contextual Esc / Enter, create-
/// form keys, confirm-delete keys) carry a fixed key string.
///
/// Exposed as `pub` (and re-exported through `tui::help_rows`) so the
/// renderer and the state-machine tests share one source of truth.
///
/// `ctx` (issue #217) drives the title's context subtitle and whether the
/// picker-only / non-picker sections render. `HintContext::Picker` is the
/// `gwm switch` overlay; `Worktrees` / `Status` are the two list-view panes
/// (same body, the subtitle just names the focused pane).
pub fn help_rows(km: &super::keymap::Keymap, modal: &ModalKeymap, ctx: HintContext) -> Vec<HelpRow> {
  use super::keymap::Action;

  let picker_mode = matches!(ctx, HintContext::Picker);

  // Snapshot the keymap once. The pre-#87-review version called
  // `km.list()` inside `keys_for`, which cloned the entire bindings
  // vector for every help row — measurable churn for the ~20 rows
  // the overlay renders. Single clone here, indexed by action below.
  let bindings = km.list();

  // Format every chord bound to `action` as a comma-separated list
  // (`"j, Down"` or `"g g"` or `""` for unbound). The width 13 is
  // wide enough for `Ctrl+Shift+Tab` while keeping the help overlay
  // narrow enough for an 80-column terminal.
  let keys_for = |action: Action| -> String {
    bindings
      .iter()
      .find(|b| b.action == action)
      .map(|b| {
        b.chords
          .iter()
          .map(|c| c.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(" "))
          .collect::<Vec<_>>()
          .join(", ")
      })
      .unwrap_or_default()
  };
  // A rebindable entry: keys resolved from the keymap.
  let entry = |action: Action, label: &str| -> HelpRow {
    HelpRow::Entry {
      keys: keys_for(action),
      label: label.to_string(),
    }
  };
  // A fixed entry: a non-rebindable surface documented with a literal
  // key string (Ctrl-C, contextual Enter, the picker's hard-coded Enter).
  let fixed = |keys: &str, label: &str| -> HelpRow {
    HelpRow::Entry {
      keys: keys.to_string(),
      label: label.to_string(),
    }
  };
  // A rebindable modal entry: keys resolved from the contextual keymap
  // (issue #219) so the create-form / delete-confirm rows track
  // `[tui.keys.modal.<context>]` overrides instead of a frozen literal.
  // No display-side filtering is needed for the always-typing contexts:
  // a binding their reserved input would swallow is refused at config
  // time (`ModalKeymap::apply_override`, Codex review #456), so every
  // advertised chord is reachable by construction.
  let modal_entry = |action: ModalAction, label: &str| -> HelpRow {
    HelpRow::Entry {
      keys: modal.keys_display(action),
      label: label.to_string(),
    }
  };

  let mut rows: Vec<HelpRow> = vec![
    HelpRow::Title("Keybindings".to_string()),
    HelpRow::Subtitle(ctx.label().to_string()),
    HelpRow::Blank,
    HelpRow::Section("Global".to_string()),
    HelpRow::Blank,
    entry(Action::Quit, "quit (Esc also quits when filter is clear)"),
    fixed("Ctrl-C", "quit (hard-coded escape hatch)"),
    HelpRow::Blank,
    HelpRow::Section("List View".to_string()),
    HelpRow::Blank,
    entry(Action::Down, "next (scrolls sidebar when focused)"),
    entry(Action::Up, "prev (scrolls sidebar when focused)"),
    entry(Action::WtScrollDown, "scroll the Working Tree pane down (status focus)"),
    entry(Action::WtScrollUp, "scroll the Working Tree pane up (status focus)"),
    entry(Action::Top, "jump to first worktree"),
    entry(Action::Bottom, "jump to last worktree"),
  ];
  if picker_mode {
    rows.push(fixed("enter", "select highlighted worktree (prints path on exit)"));
  } else {
    rows.push(entry(Action::Create, "new worktree"));
    // #484: the mark set is what `d` acts on when it is non-empty.
    rows.push(entry(
      Action::ToggleSelect,
      "mark / unmark this worktree for a bulk delete",
    ));
    rows.push(entry(
      Action::DeleteConfirm,
      "delete the marked worktrees (or this one)",
    ));
    rows.push(entry(Action::Bootstrap, "bootstrap selected"));
  }
  rows.push(entry(
    Action::TerminalFullscreen,
    "open per [tui.open] — shell / editor / finder",
  ));
  rows.push(entry(Action::TerminalPty, "open native $SHELL in embedded PTY overlay"));
  rows.push(entry(Action::OpenDocs, "open the gwm documentation in the browser"));
  rows.push(entry(Action::YankPath, "yank selected worktree path to clipboard"));
  rows.push(entry(Action::YankBranchName, "yank selected branch name to clipboard"));
  rows.push(entry(
    Action::YankWorktreeName,
    "yank selected worktree name to clipboard",
  ));
  rows.push(entry(Action::LazyGitFullscreen, "launch lazygit fullscreen"));
  rows.push(entry(Action::LazyGitPty, "open lazygit in embedded PTY overlay"));
  rows.push(entry(Action::ToggleSidebar, "toggle git preview sidebar"));
  rows.push(entry(
    Action::ToggleSidebarMode,
    "cycle sidebar mode (commits / stashes)",
  ));
  rows.push(entry(
    Action::CycleSidebarLayout,
    "cycle sidebar layout (auto / side-by-side / stacked)",
  ));
  rows.push(entry(
    Action::ToggleSidebarPosition,
    "toggle sidebar position (left / right)",
  ));
  rows.push(entry(Action::FocusSwap, "swap focus between worktree list and sidebar"));
  rows.push(entry(Action::FocusWorktrees, "focus the worktrees pane"));
  rows.push(entry(Action::FocusStatus, "focus the status pane (opens it if hidden)"));
  rows.push(entry(Action::CommandLogs, "show the command logs overlay"));
  rows.push(entry(Action::ConfigPanel, "show the resolved configuration panel"));
  // #334 review: the exec / clean overlays are picker-gated (`run_action`
  // no-ops them in `gwm switch`), so only advertise them outside picker mode.
  if !picker_mode {
    rows.push(entry(
      Action::ExecOverlay,
      "pick an [exec.profiles] profile and run it in a PTY",
    ));
    rows.push(entry(
      Action::CleanOverlay,
      "preview and reclaim build artifacts (with confirm)",
    ));
    rows.push(entry(
      Action::AgentSessions,
      "show the agent sessions attached to this worktree",
    ));
    rows.push(entry(
      Action::CiChecks,
      "list the linked PR's CI checks (also `c` with status focus)",
    ));
    rows.push(entry(
      Action::RichView,
      "open the linked PR/issue: description, checks, reviews, comments",
    ));
  }
  rows.push(entry(
    Action::Filter,
    "open fuzzy filter bar (enter: sticky, esc: clear)",
  ));
  rows.push(entry(Action::Refresh, "refresh worktree list"));
  if !picker_mode {
    rows.push(entry(Action::Sync, "sync selected worktree onto its upstream (rebase)"));
    rows.push(entry(Action::Pull, "pull selected worktree's branch from upstream"));
    rows.push(entry(Action::Push, "push selected worktree's branch to remote"));
    rows.push(entry(Action::EditWorktree, "rename the selected worktree's branch"));
    rows.push(entry(Action::EditNote, "edit the selected worktree's note"));
    rows.push(entry(
      Action::ExitToWorktree,
      "quit TUI and print selected path to stdout",
    ));
    rows.push(entry(Action::MuxPane, "open selected worktree in new mux pane/tab"));
    rows.push(entry(Action::Macro1, "run [tui.macro1] command"));
    rows.push(entry(Action::Macro2, "run [tui.macro2] command"));
    rows.push(entry(Action::FetchGithub, "refresh GitHub issue/PR status via `gh`"));
    rows.push(entry(Action::ReviewFullscreen, "run [review] launcher fullscreen"));
    rows.push(entry(
      Action::ReviewPty,
      "run [review] launcher in embedded PTY overlay",
    ));
    rows.push(entry(Action::ToggleDeleteBranch, "toggle 'delete branch on remove'"));
    rows.push(fixed("enter", "show path in status bar"));
    rows.push(HelpRow::Blank);
    rows.push(HelpRow::Section("Issue / PR".to_string()));
    rows.push(HelpRow::Blank);
    // #219 review: the direct-pick keys named in these descriptions are the
    // OpenMenu / LinkChooseTarget modal verbs — resolve them so a rebind shows
    // through, and DROP any verb the user explicitly unbound rather than
    // advertise a phantom literal (matching every other modal hint).
    let open_picks: Vec<String> = [
      (ModalAction::OpenMenuIssue, "issue"),
      (ModalAction::OpenMenuPr, "pull request"),
    ]
    .into_iter()
    .filter_map(|(a, l)| modal.primary_key(a).map(|k| format!("{k}={l}")))
    .collect();
    let open_desc = if open_picks.is_empty() {
      "open menu".to_string()
    } else {
      format!("open menu — {}", open_picks.join(" · "))
    };
    rows.push(entry(Action::BrowseLinks, &open_desc));

    let key = |a: ModalAction| modal.primary_key(a);
    let nav: Vec<String> = [ModalAction::LinkChooseNext, ModalAction::LinkChoosePrev]
      .into_iter()
      .filter_map(key)
      .collect();
    let picks: Vec<String> = [ModalAction::LinkChooseIssue, ModalAction::LinkChoosePr]
      .into_iter()
      .filter_map(key)
      .collect();
    let mut parts: Vec<String> = Vec::new();
    match (nav.is_empty(), key(ModalAction::LinkChooseAccept)) {
      (false, Some(a)) => parts.push(format!("{} + {a}", nav.join("/"))),
      (false, None) => parts.push(nav.join("/")),
      (true, Some(a)) => parts.push(a),
      (true, None) => {}
    }
    if !picks.is_empty() {
      parts.push(format!("or {}", picks.join("/")));
    }
    parts.push("then digits".to_string());
    rows.push(entry(
      Action::LinkPrompt,
      &format!("link prompt — {}", parts.join(", ")),
    ));
  }
  rows.push(entry(Action::Help, "this help"));
  if !picker_mode {
    rows.push(entry(Action::CommandPalette, "open the command palette"));
  }
  if !picker_mode {
    rows.extend([
      HelpRow::Blank,
      HelpRow::Section("Create Form".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::CreatePrevType, "previous branch type"),
      modal_entry(ModalAction::CreateNextType, "next branch type"),
      modal_entry(ModalAction::CreateNextField, "next field"),
      modal_entry(ModalAction::CreatePrevField, "previous field"),
      modal_entry(ModalAction::CreateSubmit, "submit (on the last field) / next field"),
      modal_entry(
        ModalAction::CreateToggleMode,
        "toggle structured fields ↔ free-form name",
      ),
      modal_entry(ModalAction::CreateCancel, "cancel"),
      fixed(
        "0-9",
        "type into the issue field, where the patterns ask for one (digits only)",
      ),
      fixed("any char", "type into the focused text field"),
      fixed("Backspace", "delete the last character"),
      HelpRow::Blank,
      HelpRow::Section("Delete Worktree".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::ConfirmFocusConfirm, "focus the Confirm button"),
      modal_entry(ModalAction::ConfirmFocusCancel, "focus the Cancel button"),
      modal_entry(ModalAction::ConfirmToggleFocus, "toggle the focused button"),
      modal_entry(
        ModalAction::ConfirmActivate,
        "activate the focused button (defaults to Cancel)",
      ),
      modal_entry(ModalAction::ConfirmConfirm, "confirm"),
      modal_entry(ModalAction::ConfirmCancel, "cancel"),
    ]);
    // #453: one section per modal context, in workflow order, every verb
    // resolved live against the modal keymap so rebinds show through (and
    // an explicitly unbound verb renders `(unbound)` like every other
    // entry). Completeness pinned per section by
    // `help_overlay_documents_every_modal_action_in_its_section`. Only the
    // sections whose actions `run_action` picker-gates live in this block
    // (Codex review #456): the palette, the Command Logs / Settings
    // overlays and the PTY stay reachable from `gwm switch` and render
    // below for every context.
    rows.extend([
      HelpRow::Blank,
      HelpRow::Section("Browse Links".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::OpenMenuToggle, "toggle issue / pull request"),
      modal_entry(
        ModalAction::OpenMenuAccept,
        "open the highlighted target in the browser",
      ),
      modal_entry(ModalAction::OpenMenuIssue, "open the linked issue directly"),
      modal_entry(ModalAction::OpenMenuPr, "open the linked pull request directly"),
      modal_entry(ModalAction::OpenMenuClose, "close"),
      HelpRow::Blank,
      HelpRow::Section("Link Prompt".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::LinkChooseNext, "next target (issue / PR)"),
      modal_entry(ModalAction::LinkChoosePrev, "previous target"),
      modal_entry(ModalAction::LinkChooseIssue, "pick issue directly"),
      modal_entry(ModalAction::LinkChoosePr, "pick pull request directly"),
      modal_entry(ModalAction::LinkChooseAccept, "accept the highlighted target"),
      modal_entry(ModalAction::LinkChooseCancel, "cancel"),
      fixed("0-9", "type the issue / PR number"),
      fixed("Backspace", "erase the last digit"),
      modal_entry(ModalAction::LinkInputSubmit, "submit the typed number"),
      modal_entry(ModalAction::LinkInputCancel, "cancel the number input"),
      HelpRow::Blank,
      HelpRow::Section("Exec Profiles".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::ExecPickerNext, "next profile"),
      modal_entry(ModalAction::ExecPickerPrev, "previous profile"),
      modal_entry(ModalAction::ExecPickerAccept, "run the profile in a PTY overlay"),
      modal_entry(ModalAction::ExecPickerCancel, "cancel"),
      HelpRow::Blank,
      HelpRow::Section("Clean Reclaim".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::CleanNext, "next profile"),
      modal_entry(ModalAction::CleanPrev, "previous profile"),
      modal_entry(ModalAction::CleanConfirm, "reclaim (starts the safety countdown)"),
      modal_entry(ModalAction::CleanCancel, "cancel"),
      HelpRow::Blank,
      HelpRow::Section("Agent Sessions".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::DetailSelectNext, "next session"),
      modal_entry(ModalAction::DetailSelectPrev, "previous session"),
      modal_entry(ModalAction::DetailAttach, "attach to the selected session"),
      modal_entry(ModalAction::DetailDetach, "detach the selected session"),
      modal_entry(ModalAction::DetailInput, "attach by id (palette-style prompt)"),
      fixed("any char", "attach prompt: type to filter the session ids"),
      fixed("Backspace", "attach prompt: delete the last character"),
      fixed("Up/Down", "attach prompt: move the highlight"),
      fixed("enter", "attach prompt: attach the highlighted session"),
      fixed("Esc", "attach prompt: back to the list"),
      modal_entry(ModalAction::DetailClose, "close"),
      HelpRow::Blank,
      HelpRow::Section("CI Checks".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::CiChecksNext, "next check"),
      modal_entry(ModalAction::CiChecksPrev, "previous check"),
      modal_entry(ModalAction::CiChecksOpen, "open the check's details URL in the browser"),
      modal_entry(ModalAction::CiChecksFilter, "filter the checks by name"),
      modal_entry(ModalAction::CiChecksRefresh, "re-fetch the PR and refresh the rows"),
      fixed("any char", "filter: type to narrow the checks"),
      fixed("Backspace", "filter: delete the last character"),
      fixed("Up/Down", "filter: move the highlight"),
      fixed("enter", "filter: open the highlighted check's URL"),
      fixed("Esc", "filter: back to the list"),
      modal_entry(ModalAction::CiChecksClose, "close"),
      HelpRow::Blank,
      HelpRow::Section("PR / Issue View".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::RichViewNext, "next row"),
      modal_entry(ModalAction::RichViewPrev, "previous row"),
      modal_entry(ModalAction::RichViewOpen, "open the selected row's URL in the browser"),
      modal_entry(ModalAction::RichViewRefresh, "re-fetch and refresh the view"),
      modal_entry(ModalAction::RichViewClose, "close"),
      HelpRow::Blank,
      HelpRow::Section("Note Editor".to_string()),
      HelpRow::Blank,
      fixed("Left/Right/Up/Down", "move the cursor"),
      fixed("Home/End", "start / end of line"),
      fixed("PgUp/PgDn", "page through the note"),
      modal_entry(ModalAction::NoteOpenEditor, "open the same file in $EDITOR"),
      modal_entry(ModalAction::NoteClose, "save and close (empty the note to delete it)"),
      HelpRow::Blank,
      HelpRow::Section("Bootstrap Report".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::ReportClose, "close"),
    ]);
  }
  // In a real `gwm switch` the filter bar is ALWAYS active — its only
  // exits confirm (Enter) or cancel (Esc) the pick — so no overlay is
  // reachable there, whatever `run_action` would allow: every printable
  // key (`?`, `:`, `3`, `4`, `o`) types into the filter instead (Codex
  // review #456, iteration 8). The modal sections all stay non-picker.
  if !picker_mode {
    rows.extend([
      HelpRow::Blank,
      HelpRow::Section("Command Palette".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::CommandPaletteNext, "next command"),
      modal_entry(ModalAction::CommandPalettePrev, "previous command"),
      modal_entry(ModalAction::CommandPaletteAccept, "run the highlighted command"),
      fixed("a-z 0-9 _ -", "fuzzy-filter the commands (lowercase input only)"),
      fixed("Backspace", "delete the last filter character"),
      modal_entry(ModalAction::CommandPaletteClose, "close"),
      HelpRow::Blank,
      HelpRow::Section("Command Logs".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::CommandLogsScrollDown, "scroll down"),
      modal_entry(ModalAction::CommandLogsScrollUp, "scroll up"),
      modal_entry(ModalAction::CommandLogsScrollLeft, "pan left"),
      modal_entry(ModalAction::CommandLogsScrollRight, "pan right"),
      modal_entry(ModalAction::CommandLogsScrollTop, "jump to the top"),
      modal_entry(ModalAction::CommandLogsScrollBottom, "jump to the bottom"),
      modal_entry(
        ModalAction::CommandLogsCopy,
        "copy the full transcript to the clipboard",
      ),
      modal_entry(ModalAction::CommandLogsClose, "close"),
      HelpRow::Blank,
      HelpRow::Section("Settings".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::ConfigNextTab, "next tab"),
      modal_entry(ModalAction::ConfigPrevTab, "previous tab"),
      modal_entry(ModalAction::ConfigToggleLayer, "toggle the Project / Global layer"),
      modal_entry(ModalAction::ConfigSelectNext, "next setting (All tab: scroll down)"),
      modal_entry(ModalAction::ConfigSelectPrev, "previous setting (All tab: scroll up)"),
      modal_entry(
        ModalAction::ConfigActivate,
        "toggle / edit the selected setting (Keys tab: start a key capture — a modal verb commits on its first stroke)",
      ),
      modal_entry(ModalAction::ConfigScrollLeft, "pan left (All tab)"),
      modal_entry(ModalAction::ConfigScrollRight, "pan right (All tab)"),
      modal_entry(ModalAction::ConfigScrollTop, "jump to the top (All tab)"),
      modal_entry(ModalAction::ConfigScrollBottom, "jump to the bottom (All tab)"),
      modal_entry(
        ModalAction::ConfigEditSubmit,
        "commit the edited value / the captured global chord",
      ),
      modal_entry(ModalAction::ConfigEditCancel, "cancel the edit / the key capture"),
      fixed(
        "any char",
        "type the value — free text for text fields, digits for numeric ones",
      ),
      fixed(
        "Backspace",
        "erase the last character / drop the last stroke of a global capture",
      ),
      fixed("enter", "capture: commit the global chord (reserved, despite rebinds)"),
      fixed("Esc", "capture: cancel (reserved, despite rebinds)"),
      modal_entry(ModalAction::ConfigClose, "close"),
      HelpRow::Blank,
      HelpRow::Section("PTY Overlay".to_string()),
      HelpRow::Blank,
      fixed(
        "Esc",
        "close the overlay — other keys pass through (any key but Ctrl-C closes a finished exec run)",
      ),
    ]);
    rows.extend([
      HelpRow::Blank,
      HelpRow::Section("Help Overlay".to_string()),
      HelpRow::Blank,
      modal_entry(ModalAction::HelpScrollDown, "scroll down"),
      modal_entry(ModalAction::HelpScrollUp, "scroll up"),
      modal_entry(ModalAction::HelpScrollLeft, "pan left"),
      modal_entry(ModalAction::HelpScrollRight, "pan right"),
      modal_entry(ModalAction::HelpScrollTop, "jump to the top"),
      modal_entry(ModalAction::HelpScrollBottom, "jump to the bottom"),
      modal_entry(ModalAction::HelpClose, "close"),
    ]);
  }
  rows
}

/// Flatten [`help_rows`] back into the legacy `Vec<String>` overlay body
/// (issue #87). Kept as the stable, terminal-free contract that
/// `tests/tui_chord_tests.rs` asserts against: every entry renders as
/// `  {keys:<13} {label}`, sections / title as their bare text, blanks
/// as empty strings. The width 13 is wide enough for `Ctrl+Shift+Tab`.
pub fn help_lines(km: &super::keymap::Keymap, modal: &ModalKeymap, picker_mode: bool) -> Vec<String> {
  // The bool signature is kept for `gwm tui keys` and the chord tests; map
  // it to the context enum (issue #217). The list-view help body is the same
  // for either pane, so `Worktrees` stands in for the non-picker case.
  let ctx = if picker_mode {
    HintContext::Picker
  } else {
    HintContext::Worktrees
  };
  help_rows(km, modal, ctx)
    .into_iter()
    .map(|row| match row {
      HelpRow::Title(s) | HelpRow::Subtitle(s) | HelpRow::Section(s) => s,
      HelpRow::Blank => String::new(),
      HelpRow::Entry { keys, label } => {
        let keys = if keys.is_empty() { "(unbound)".to_string() } else { keys };
        format!("  {:<13} {}", keys, label)
      }
    })
    .collect()
}

/// Display width of a help row's key *badges* once split into one badge
/// per chord (#187 review). Each badge renders as ` chord ` (chord + 2
/// pad cells); badges are separated by a single space. `(unbound)` /
/// empty render as one muted badge. Used to right-pad the badge column
/// so the labels line up regardless of how many chords a row binds.
pub fn badge_group_width(keys: &str) -> usize {
  if keys.is_empty() || keys == "(unbound)" {
    return "(unbound)".chars().count();
  }
  let chords: Vec<&str> = keys.split(", ").collect();
  // Flat accent-bold glyphs now (issue #279), no `` key `` padding box: a
  // group is the sum of bare chord widths plus one space between adjacent
  // chords.
  let glyphs: usize = chords.iter().map(|c| c.chars().count()).sum();
  glyphs + chords.len().saturating_sub(1)
}

/// One documented-binding row for the Keybindings overlay (issue #279):
/// the chord(s) as flat accent-bold glyphs (no reverse-video badge),
/// padded to `max_group_w` so every label lines up in one column, then the
/// human label. An unbound action reads as a muted `(unbound)` placeholder.
/// Extracted as a pure builder so the de-badged treatment is pinned by
/// `tests/tui_ui_helpers_tests.rs` without a ratatui backend.
pub fn help_entry_line(keys: &str, label: &str, max_group_w: usize, theme: &Theme) -> Line<'static> {
  let key_style = hint_key_style(theme);
  let muted_style = Style::default().fg(theme.muted);
  let mut spans: Vec<Span<'static>> = vec![Span::raw("  ")];
  if keys.is_empty() || keys == "(unbound)" {
    spans.push(Span::styled("(unbound)", muted_style));
  } else {
    for (i, chord) in keys.split(", ").enumerate() {
      if i > 0 {
        spans.push(Span::raw(" "));
      }
      spans.push(Span::styled(chord.to_string(), key_style));
    }
  }
  let pad = max_group_w.saturating_sub(badge_group_width(keys)) + 1;
  spans.push(Span::raw(" ".repeat(pad)));
  spans.push(Span::styled(label.to_string(), help_label_style(theme)));
  Line::from(spans)
}

fn draw_help(f: &mut Frame, app: &mut App) {
  let area = centered(60, 60, f.area());
  // Use the underlying pane context, not the view-priority `hint_context`
  // (which would be `Help` while this overlay is up) — `?` documents the
  // pane you opened it from, and the picker gating depends on it.
  let rows = help_rows(&app.keymap, &app.modal_keymap, app.pane_hint_context());

  // Theme-driven colours so the overlay tracks `[theme]` like the rest
  // of the TUI (pre-#187 it was hard-coded `Cyan` + plain text).
  let accent = app.theme.accent;

  let heading_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
  // Subtitle reads in a distinct accent hue (the theme's branch colour) +
  // italic, so the context name is clearly a different colour from both the
  // bold title and the muted key labels (issue #217 follow-up).
  let subtitle_style = Style::default().fg(app.theme.branch).add_modifier(Modifier::ITALIC);

  // Align every label to the same column: pad each chord *group* out to the
  // widest one so the descriptions line up under one another.
  let max_group_w = rows
    .iter()
    .filter_map(|r| match r {
      HelpRow::Entry { keys, .. } => Some(badge_group_width(keys)),
      _ => None,
    })
    .max()
    .unwrap_or(0);

  // Issue #279: split the overlay into a FIXED header (title + subtitle), a
  // SCROLLABLE body (sections + entries), and a FIXED footer hint. Pre-#279
  // the whole content scrolled in one `Paragraph`, so the title and the
  // close hint rolled off the top/bottom as soon as the body outgrew the
  // modal. Title/subtitle are the leading rows; everything else is body.
  let mut header_lines: Vec<Line<'static>> = Vec::new();
  let mut body_lines: Vec<Line<'static>> = Vec::new();
  for row in rows {
    match row {
      // Title + subtitle are centred (issue #217) and pinned in the header.
      HelpRow::Title(t) => header_lines.push(Line::from(Span::styled(t, heading_style)).centered()),
      HelpRow::Subtitle(t) => header_lines.push(Line::from(Span::styled(t, subtitle_style)).centered()),
      // Section headers stay left-aligned so they anchor their groups
      // lazygit-style.
      HelpRow::Section(t) => body_lines.push(Line::from(Span::styled(
        t,
        help_section_style(help_body_section_color(&app.theme)),
      ))),
      HelpRow::Blank => body_lines.push(Line::from(String::new())),
      HelpRow::Entry { keys, label } => {
        body_lines.push(help_entry_line(&keys, &label, max_group_w, &app.theme));
      }
    }
  }

  let block = overlay_block(accent);
  let inner_area = block.inner(area);
  f.render_widget(Clear, area);
  f.render_widget(block, area);

  // header (fixed) | body (scrollable) | footer hint (fixed). The header is
  // exactly as tall as its line count; the footer is one row; the body
  // takes the rest.
  let header_h = header_lines.len() as u16;
  let [header_area, body_area, footer_area] =
    Layout::vertical([Constraint::Length(header_h), Constraint::Min(1), Constraint::Length(1)]).areas(inner_area);

  f.render_widget(Paragraph::new(header_lines), header_area);

  // Publish the scroll bounds against the BODY viewport only (issue #279) —
  // not the whole inner height — so the clamp matches what actually scrolls
  // and the last body rows stay reachable.
  let body_viewport = body_area.height as usize;
  app.help_max_scroll = (body_lines.len().saturating_sub(body_viewport)) as u16;
  app.help_scroll = app.help_scroll.min(app.help_max_scroll);
  let scroll = app.help_scroll;
  // Reserve the scrollbar column FIRST, then bound the horizontal pan against
  // the reduced text width so the final cell stays reachable (review P3).
  let text_area = scrollable_body_area(f, body_area, scroll, body_lines.len(), &app.theme);
  let content_width = body_lines.iter().map(Line::width).max().unwrap_or(0);
  app.help_max_x_scroll = content_width.saturating_sub(text_area.width as usize) as u16;
  app.help_x_scroll = app.help_x_scroll.min(app.help_max_x_scroll);
  let x_scroll = app.help_x_scroll;
  f.render_widget(Paragraph::new(body_lines).scroll((scroll, x_scroll)), text_area);
  f.render_widget(
    modal_hint_for_context(HintContext::Help, &app.keymap, &app.modal_keymap, &app.theme),
    footer_area,
  );
}

/// Render the Command Logs overlay (issue #226): a ~90% fullscreen modal
/// over the dimmed list showing the lazygit-style transcript of the
/// external commands gwm ran, newest-first. Scrolls like the help overlay —
/// the renderer republishes `command_logs.max_scroll` / `max_x_scroll`
/// against the live viewport so `App`'s scroll cursor can never run past
/// the content. Colours track `[theme]` roles (`clean` ok / `prunable`
/// fail / `muted` output) so a theme override applies here too.
fn draw_command_logs(f: &mut Frame, app: &mut App) {
  let area = centered(90, 85, f.area());
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let ok_color = app.theme.clean;
  let err_color = app.theme.prunable;
  let label_style = help_label_style(&app.theme);
  let muted_style = Style::default().fg(muted);
  let heading_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);

  // Fixed header (title) / scrollable body / fixed footer hint (issue #279) —
  // the title and the close hint stay pinned while the transcript scrolls.
  let block = overlay_block(accent);
  let inner = block.inner(area);
  f.render_widget(Clear, area);
  f.render_widget(block, area);

  let [header_area, body_area, footer_area] =
    Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(inner);

  f.render_widget(
    Paragraph::new(Line::from(Span::styled("Command Logs", heading_style)).centered()),
    header_area,
  );

  // A full-width `-` rule, padded by a blank line above and below, separates
  // adjacent log entries (issue #279 follow-up).
  let rule = "-".repeat(body_area.width as usize);
  let mut lines: Vec<Line<'static>> = Vec::new();

  if app.command_logs.entries.is_empty() {
    lines.push(Line::from(Span::styled("No commands run yet.", muted_style)));
  } else {
    // Newest-first: the most recent command is what the user opened the
    // overlay to see, so it sits at the top without scrolling.
    for (i, entry) in app.command_logs.entries.iter().rev().enumerate() {
      if i > 0 {
        lines.push(Line::from(String::new()));
        lines.push(Line::from(Span::styled(rule.clone(), muted_style)));
        lines.push(Line::from(String::new()));
      }
      // The resolved argv, prefixed lazygit-style with `$`.
      lines.push(Line::from(vec![
        Span::styled("$ ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
        Span::styled(entry.command.clone(), label_style),
      ]));
      // Outcome line, coloured by exit status.
      let (color, detail) = match &entry.status {
        CommandStatus::Exited(Some(0)) => (ok_color, format!("→ exit 0 ({} ms)", entry.duration.as_millis())),
        CommandStatus::Exited(Some(code)) => (
          err_color,
          format!("→ exit {} ({} ms)", code, entry.duration.as_millis()),
        ),
        CommandStatus::Exited(None) => (err_color, format!("→ terminated ({} ms)", entry.duration.as_millis())),
        CommandStatus::Spawn => (err_color, "✗ failed to spawn".to_string()),
      };
      lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(detail, Style::default().fg(color)),
      ]));
      // Captured output, tail-capped so one chatty command cannot dominate
      // the transcript (the tail is where errors surface).
      if !entry.output.is_empty() {
        const MAX_OUTPUT_LINES: usize = 6;
        let out: Vec<&str> = entry.output.lines().collect();
        let start = out.len().saturating_sub(MAX_OUTPUT_LINES);
        if start > 0 {
          lines.push(Line::from(Span::styled(
            format!("    … {} earlier line(s)", start),
            muted_style,
          )));
        }
        for l in &out[start..] {
          lines.push(Line::from(Span::styled(format!("    {}", l), muted_style)));
        }
      }
    }
  }

  // Publish the scroll bounds against the BODY viewport only (issue #279).
  let body_viewport = body_area.height as usize;
  app.command_logs.max_scroll = (lines.len().saturating_sub(body_viewport)) as u16;
  app.command_logs.scroll = app.command_logs.scroll.min(app.command_logs.max_scroll);
  let scroll = app.command_logs.scroll;
  // Reserve the scrollbar column first, then bound the pan (review P3).
  let text_area = scrollable_body_area(f, body_area, scroll, lines.len(), &app.theme);
  let content_w = lines.iter().map(Line::width).max().unwrap_or(0);
  app.command_logs.max_x_scroll = content_w.saturating_sub(text_area.width as usize) as u16;
  app.command_logs.x_scroll = app.command_logs.x_scroll.min(app.command_logs.max_x_scroll);
  let x_scroll = app.command_logs.x_scroll;
  f.render_widget(Paragraph::new(lines).scroll((scroll, x_scroll)), text_area);
  // #219 review: copy / close resolve from the command_logs modal bindings so
  // a rebind shows through; the scroll / top-bottom pairs stay literal.
  let footer_owned = command_logs_footer_hints(&app.modal_keymap);
  let footer_hints: Vec<(&str, &str)> = footer_owned.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
  f.render_widget(modal_hint_line(&footer_hints, &app.theme), footer_area);
}

/// Render a vertical scrollbar on the right edge of `area` when the content
/// overflows the viewport (issue #279, herdr-style), and return the text
/// area shrunk by one column to make room. When everything fits, the area
/// is returned unchanged and no scrollbar is drawn. The thumb tracks the
/// theme `accent`; the track reads `muted`.
fn scrollable_body_area(f: &mut Frame, area: Rect, offset: u16, content_len: usize, theme: &Theme) -> Rect {
  let viewport = area.height as usize;
  if content_len <= viewport || area.width < 2 {
    return area;
  }
  // ratatui maps the thumb over `content_length - 1`, but our scroll offset
  // is clamped to `content_len - viewport` (the last page stays full). Pass
  // `content_length = max_scroll + 1` with the real viewport length so the
  // thumb size stays proportional AND reaches the bottom at full scroll
  // (issue #279 follow-up: the thumb used to top out early).
  let max_scroll = content_len - viewport;
  let mut state = ScrollbarState::new(max_scroll + 1)
    .position(offset as usize)
    .viewport_content_length(viewport);
  let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .begin_symbol(None)
    .end_symbol(None)
    .thumb_style(Style::default().fg(theme.accent))
    .track_style(Style::default().fg(theme.muted));
  f.render_stateful_widget(bar, area, &mut state);
  Rect {
    width: area.width.saturating_sub(1),
    ..area
  }
}

/// Build the read-only `All`-tab body: the resolved config grouped by
/// top-level section with a colour-coded source column (repo / user /
/// default). The pre-#279 Configuration view, now one tab of the Settings
/// overlay.
fn settings_all_lines(app: &App) -> Vec<Line<'static>> {
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let label_style = help_label_style(&app.theme);
  let muted_style = Style::default().fg(muted);
  let mut lines: Vec<Line<'static>> = Vec::new();

  if app.config_panel.rows.is_empty() {
    lines.push(Line::from(Span::styled("No configuration resolved.", muted_style)));
    return lines;
  }
  let mut current_section: Option<String> = None;
  for row in &app.config_panel.rows {
    let section = row.key.split(['.', '[']).next().unwrap_or("").to_string();
    if current_section.as_deref() != Some(section.as_str()) {
      if current_section.is_some() {
        lines.push(Line::from(String::new()));
      }
      lines.push(Line::from(Span::styled(
        format!("[{section}]"),
        help_section_style(accent),
      )));
      current_section = Some(section);
    }
    let src_color = match row.source {
      ConfigSource::Repo => app.theme.clean,
      ConfigSource::User => app.theme.branch,
      ConfigSource::Default => muted,
    };
    lines.push(Line::from(vec![
      Span::raw("  "),
      Span::styled(format!("{:<7}", row.source.label()), Style::default().fg(src_color)),
      Span::raw("  "),
      Span::styled(row.key.clone(), label_style),
      Span::styled(" = ", muted_style),
      Span::styled(row.value.clone(), Style::default().fg(Color::White)),
    ]));
  }
  lines
}

/// Build an editable-tab body: one row per [`SettingField`], the selected
/// row marked and its value in the accent. The `Uint` field under edit
/// shows its live buffer with a cursor; a field whose effective value is
/// shadowed by a higher-precedence layer carries an inline guidance note
/// (issue #279 — honours "edit both layers" without a silent dead edit).
fn settings_fields_lines(app: &App, fields: &[SettingField]) -> Vec<Line<'static>> {
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let label_style = help_label_style(&app.theme);
  let muted_style = Style::default().fg(muted);
  let panel = &app.config_panel;
  let mut lines: Vec<Line<'static>> = Vec::new();

  for (i, field) in fields.iter().enumerate() {
    let selected = i == panel.selected;
    let editing = selected && panel.editing.is_some();
    let value = if editing {
      format!("{}_", panel.editing.as_deref().unwrap_or(""))
    } else {
      field.current(&app.config)
    };
    let marker = if selected { "›" } else { " " };
    let marker_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
    let value_style = if selected {
      Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(Color::White)
    };
    let mut spans = vec![
      Span::styled(format!(" {marker} "), marker_style),
      Span::styled(format!("{:<24}", field.label()), label_style),
      Span::styled(value, value_style),
    ];
    // Shadow guidance: editing the Global layer for a field the repo
    // overrides won't change the effective value (repo wins). Surface it
    // rather than silently no-op or hard-disable the field.
    if selected && panel.layer.source() == ConfigSource::User && panel.field_source(*field) == Some(ConfigSource::Repo)
    {
      spans.push(Span::styled("  — set in .gwm.toml; switch to Project", muted_style));
    }
    lines.push(Line::from(spans));
  }
  lines
}

/// Build the Keys-tab body (issue #294): the rebindable bindings grouped by
/// scope (`[global]`, `[modal.<context>]`), each row showing its source badge,
/// label and current key(s). The selected row is marked; while a live capture
/// is armed its key column becomes a `[ … ]` input echoing the captured
/// strokes. Returns the line index of the selected row so the caller can keep
/// it in view (this body is far taller than the viewport). Mirrors
/// [`settings_all_lines`]'s section grouping + source colours.
fn settings_keys_lines(app: &App) -> (Vec<Line<'static>>, Option<usize>) {
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let label_style = help_label_style(&app.theme);
  let muted_style = Style::default().fg(muted);
  let panel = &app.config_panel;
  let mut lines: Vec<Line<'static>> = Vec::new();
  let mut selected_line: Option<usize> = None;

  if panel.key_rows.is_empty() {
    lines.push(Line::from(Span::styled("No bindings resolved.", muted_style)));
    return (lines, None);
  }

  let mut current_scope: Option<String> = None;
  for (i, row) in panel.key_rows.iter().enumerate() {
    if current_scope.as_deref() != Some(row.scope.as_str()) {
      if current_scope.is_some() {
        lines.push(Line::from(String::new()));
      }
      lines.push(Line::from(Span::styled(
        format!("[{}]", row.scope),
        help_section_style(accent),
      )));
      current_scope = Some(row.scope.clone());
    }

    let selected = i == panel.selected;
    if selected {
      selected_line = Some(lines.len());
    }
    let capturing = selected && panel.capture.is_some();
    let marker = if selected { "›" } else { " " };
    let marker_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
    let src_color = match row.source {
      ConfigSource::Repo => app.theme.clean,
      ConfigSource::User => app.theme.branch,
      ConfigSource::Default => muted,
    };

    let key_span = if capturing {
      let pending = panel
        .capture
        .as_ref()
        .map(|c| c.pending.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
      Span::styled(
        format!("[ {pending}_ ]"),
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
      )
    } else {
      let shown = if row.keys.is_empty() {
        "(unbound)".to_string()
      } else {
        row.keys.clone()
      };
      let style = if row.keys.is_empty() {
        muted_style
      } else if selected {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
      } else {
        Style::default().fg(Color::White)
      };
      Span::styled(shown, style)
    };

    lines.push(Line::from(vec![
      Span::styled(format!(" {marker} "), marker_style),
      Span::styled(format!("{:<7}", row.source.label()), Style::default().fg(src_color)),
      Span::raw(" "),
      Span::styled(format!("{:<24}", row.label), label_style),
      key_span,
    ]));
  }
  (lines, selected_line)
}

/// Render the Settings overlay (issue #232; editable in #279): same modal
/// size as the Keybindings overlay, with a fixed header (title + the edit
/// layer as a subtitle + category tabs), a scrollable body (the active
/// tab's fields, or the read-only resolved config on the `All` tab) with a
/// herdr-style scrollbar, and a fixed footer hint. The renderer republishes
/// `config_panel.max_scroll` against the live body viewport.
fn draw_config_panel(f: &mut Frame, app: &mut App) {
  let area = centered(60, 60, f.area());
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let muted_style = Style::default().fg(muted);
  let heading_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
  // Subtitle reads in the branch hue + italic, mirroring the Keybindings
  // overlay's context subtitle.
  let subtitle_style = Style::default().fg(app.theme.branch).add_modifier(Modifier::ITALIC);

  let tab = app.config_panel.tab;
  let editing = app.config_panel.editing.is_some();
  let selected_kind = app.config_panel.selected_field().map(SettingField::kind);

  // Header: title + the active edit layer as a subtitle + a blank spacer +
  // the tab strip (all fixed). The layer-switch key lives in the footer
  // hints, so the subtitle stays a plain context label.
  let title = Line::from(Span::styled("Settings", heading_style)).centered();
  let subtitle = Line::from(Span::styled(app.config_panel.layer.label(), subtitle_style)).centered();
  let mut tab_spans: Vec<Span<'static>> = vec![Span::raw(" ")];
  for (i, t) in SettingsTab::ALL.iter().enumerate() {
    if i > 0 {
      tab_spans.push(Span::raw("  "));
    }
    let style = if *t == tab { chip_style(accent) } else { muted_style };
    tab_spans.push(Span::styled(format!(" {} ", t.label()), style));
  }
  let header_lines = vec![title, subtitle, Line::from(String::new()), Line::from(tab_spans)];

  // Body depends on the active tab. Every tab with a selection reports the line
  // index of the selected row so the renderer can scroll it into view.
  //
  // This used to be Keys-only, on the reasoning that "the field tabs are short
  // enough to never need this". That was wrong on any short terminal: the modal
  // is 60% of the height, so 24 lines leave ~6 body rows, and the TUI tab has
  // had more fields than that since well before #367 added an 8th. The result
  // was a selection that walked off screen — the user cycling or editing a row
  // they cannot see (Codex review #368 P2).
  //
  // `settings_fields_lines` renders exactly one line per field, so the field
  // index *is* the line index.
  let mut selected_line: Option<usize> = None;
  let body_lines = match tab {
    SettingsTab::All => settings_all_lines(app),
    SettingsTab::Keys => {
      let (lines, sel) = settings_keys_lines(app);
      selected_line = sel;
      lines
    }
    other => {
      let fields = other.fields();
      if !fields.is_empty() {
        selected_line = Some(app.config_panel.selected.min(fields.len().saturating_sub(1)));
      }
      settings_fields_lines(app, fields)
    }
  };

  // Footer hints — flat accent-bind + muted-action (issue #279), dynamic to
  // the current tab / edit / capture mode. The edit, capture and nav rows
  // resolve their single-key verbs from the Config* modal bindings (#219
  // review) so a rebind of `[tui.keys.modal.config(.edit)]` shows through
  // instead of literal keys.
  let capture_single = app.config_panel.capture.as_ref().map(|c| c.single_only);
  let footer_owned = if let Some(single) = capture_single {
    config_capture_footer_hints(&app.modal_keymap, single)
  } else if editing {
    config_edit_footer_hints(&app.modal_keymap)
  } else {
    config_nav_footer_hints(&app.modal_keymap, tab, selected_kind)
  };
  let footer_hints: Vec<(&str, &str)> = footer_owned.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();

  let block = overlay_block(accent);
  let inner = block.inner(area);
  f.render_widget(Clear, area);
  f.render_widget(block, area);

  let header_h = header_lines.len() as u16;
  let [header_area, body_area, footer_area] =
    Layout::vertical([Constraint::Length(header_h), Constraint::Min(1), Constraint::Length(1)]).areas(inner);

  f.render_widget(Paragraph::new(header_lines), header_area);

  // Publish scroll bounds against the BODY viewport only (issue #279).
  let body_viewport = body_area.height as usize;
  app.config_panel.max_scroll = (body_lines.len().saturating_sub(body_viewport)) as u16;
  // Follow the selected row so it stays on screen as the selection moves —
  // every tab that has a selection, not just Keys (see the note above).
  if let Some(sel) = selected_line {
    let scroll = app.config_panel.scroll as usize;
    if sel < scroll {
      app.config_panel.scroll = sel as u16;
    } else if body_viewport > 0 && sel >= scroll + body_viewport {
      app.config_panel.scroll = (sel + 1 - body_viewport) as u16;
    }
  }
  app.config_panel.scroll = app.config_panel.scroll.min(app.config_panel.max_scroll);
  let scroll = app.config_panel.scroll;
  // Reserve the scrollbar column first, then bound the pan (review P3).
  let text_area = scrollable_body_area(f, body_area, scroll, body_lines.len(), &app.theme);
  let content_w = body_lines.iter().map(Line::width).max().unwrap_or(0);
  app.config_panel.max_x_scroll = content_w.saturating_sub(text_area.width as usize) as u16;
  app.config_panel.x_scroll = app.config_panel.x_scroll.min(app.config_panel.max_x_scroll);
  let x_scroll = app.config_panel.x_scroll;
  f.render_widget(Paragraph::new(body_lines).scroll((scroll, x_scroll)), text_area);
  f.render_widget(modal_hint_line(&footer_hints, &app.theme), footer_area);
}

/// The branch and directory a structured form would produce, expanded from
/// **this repo's own** `branch_pattern` / `path_pattern`.
///
/// Issue #417: those patterns are what `gwm create` and the rename actually
/// write, so a live preview has to come from them. Both modals hardcoded the
/// default `<type>/#<issue>-<desc>` shape instead, and under a custom pattern
/// they promised names the repo would never create. The rename case was the
/// loud one: with `feat/#{issue}-{desc}`, picking `docs` in the type selector
/// previewed `docs/#42-x` while submitting wrote `feat/#42-x`, because the
/// pattern has no `{type}` to write into.
///
/// The form's fields are expanded exactly as they stand, mid-typing and all,
/// so this never validates and never refuses: an empty issue expands to
/// nothing, which is what a preview should show. An expansion that cannot be
/// resolved at all yields an empty string rather than a stale or invented one.
/// Issue #481. Whether submitting this rename would close an open pull request,
/// as a message ready to render, or `None` when nothing is at risk.
///
/// The remote half of a rename is `git push --atomic origin :<old> <new>:<new>`,
/// a delete followed by a create, and GitHub closes a pull request whose head
/// branch is renamed. gwm cannot route around that. GitHub's own rename
/// endpoint retargets a pull request whose *base* is the renamed branch and
/// closes one whose *head* it is, and a worktree branch is always the head of
/// its own pull request, so both paths end in the same place. GitLab has no
/// rename operation at all, only create-then-delete. Saying so before the push
/// is the whole of what is available.
///
/// Keyed on the **branch** rather than on the rename: an edit that only moves
/// the directory returns from `worktree::rename_worktree` before touching a
/// single ref, so it is never at risk, and warning there would train the user
/// to ignore the line. Recomputed per frame, so it appears the moment the form
/// would write a different branch and goes away when the user reverts.
///
/// A merged or closed pull request has nothing left to lose, and an unfetched
/// state says nothing either way: this reports what gwm knows, and claims
/// nothing about what it has not looked up.
fn rename_pr_warning(app: &App, new_branch: &str, old_branch: &str) -> Option<String> {
  if new_branch == old_branch {
    return None;
  }
  let w = app.selected()?;
  if !matches!(w.pr_state.or(w.link.pr_state)?, PrState::Open | PrState::Draft) {
    return None;
  }
  Some(match w.link.pr {
    Some(number) => format!("⚠ renaming the branch closes PR #{}", number),
    None => "⚠ renaming the branch closes its open pull request".into(),
  })
}

/// The `(branch, directory)` pair the form would produce, for the live preview.
///
/// Both modes, because both are reachable from the create form (#416) and now
/// from the rename modal too (#479) — and a preview that handled only one of
/// them would show a branch the submit is not going to write. That defect
/// shipped once already, with the preview hardcoding `<type>/#<issue>-<desc>`
/// while the submit expanded the repo's patterns; free-form is the same trap
/// one mode over, since there nothing is expanded at all.
///
/// Deliberately **unvalidated**, unlike [`App::edit_target`]: this runs on every
/// keystroke and has to show what a half-typed form is heading towards. So the
/// free-form arm builds `WorktreeName::Freeform` directly rather than through
/// `WorktreeName::freeform`, which would refuse an incomplete name and blank the
/// preview mid-word. The flattening still comes from `worktree_dirname`, so the
/// preview and the submit cannot drift on it.
fn pattern_preview(app: &App, type_str: &str) -> (String, String) {
  if app.create_form.mode == Mode::Freeform {
    let name = crate::naming::WorktreeName::Freeform(app.create_form.name.clone());
    return (
      name
        .branch_name(&app.config.worktree, &app.repo_name)
        .unwrap_or_default(),
      name
        .worktree_dirname(&app.config.worktree, &app.repo_name)
        .unwrap_or_default(),
    );
  }
  let expand = |pattern: &str| {
    crate::config::expand_placeholders(
      pattern,
      &app.repo_name,
      Some(type_str),
      Some(&app.create_form.issue),
      Some(&app.create_form.desc),
      None,
    )
    .unwrap_or_default()
  };
  (
    expand(&app.config.worktree.branch_pattern),
    expand(&app.config.worktree.path_pattern),
  )
}

/// One row per field the form presents, in the order the patterns write them
/// (issue #418), blank-line separated.
///
/// Both the create overlay and the rename modal draw from this, so they cannot
/// come to disagree about which inputs exist — they used to hardcode the same
/// `Type` / `Issue` / `Desc` triple twice, which is two places to forget a
/// pattern that carries only some of them.
///
/// Visual order is focus order by construction, which is the property that
/// makes a custom pattern legible: `{desc}-{issue}` reads top-to-bottom in the
/// same order it writes left-to-right. That does move the type selector below
/// the preview, where the old layout kept it above.
fn form_field_lines(app: &App, type_str: &str, type_desc: &str, value_w: usize, label_w: usize) -> Vec<Line<'static>> {
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let surface = app.theme.selection_bg;
  let label = |s: &str| format!("{:<label_w$}", s);

  let mut lines: Vec<Line<'static>> = Vec::new();
  for field in app.create_form.fields() {
    if !lines.is_empty() {
      lines.push(Line::from(String::new()));
    }
    lines.push(match field {
      Field::Type => type_selector_line(
        &label("Type"),
        type_str,
        type_desc,
        app.create_form.field == Field::Type,
        accent,
        muted,
      ),
      Field::Issue => field_input_line(
        &label("Issue"),
        &app.create_form.issue,
        app.create_form.field == Field::Issue,
        value_w,
        accent,
        muted,
        surface,
      ),
      Field::Desc => field_input_line(
        &label("Desc"),
        &app.create_form.desc,
        app.create_form.field == Field::Desc,
        value_w,
        accent,
        muted,
        surface,
      ),
      // `Name` belongs to free-form mode, which never reaches this list.
      Field::Name => continue,
    });
  }
  lines
}

fn draw_create(f: &mut Frame, app: &App) {
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let clean = app.theme.clean;
  let surface = app.theme.selection_bg;

  let (type_str, type_desc) = app
    .branch_types
    .get(app.create_form.type_index)
    .map(|t| (t.name.as_str(), t.description.as_str()))
    .unwrap_or(("", "(no branch types configured)"));

  let block = overlay_block(clean);
  let term = f.area();
  let outer = centered_box(70, 72, 1, term);
  let inner_w = block.inner(outer).width as usize;

  // Width of the background-filled value field: the inner width minus the
  // `  label  ` gutter (2 indent + label column + 2 gap).
  let label_w = 5usize;
  let gutter = 2 + label_w + 2;
  let value_w = inner_w.saturating_sub(gutter);

  let label = |s: &str| format!("{:<label_w$}", s);
  let freeform = app.create_form.mode == Mode::Freeform;
  // Issue #416: a free-form name IS the branch, and the directory is that
  // name with `/` flattened — mirroring `WorktreeName::worktree_dirname`.
  let (branch_raw, dir_raw) = if freeform {
    (app.create_form.name.clone(), app.create_form.name.replace('/', "-"))
  } else {
    pattern_preview(app, type_str)
  };
  let branch = ellipsize_middle(&branch_raw, inner_w.saturating_sub("  Branch : ".len()));
  let dirname = ellipsize_middle(&dir_raw, inner_w.saturating_sub("  Dir    : ".len()));

  let mut lines = overlay_title_lines(
    if freeform {
      "New Worktree — free-form"
    } else {
      "New Worktree"
    },
    clean,
  );
  // The live preview first, then the editable fields — the preview sits above
  // the inputs so the resulting names stay in view while typing (issue #217
  // follow-up). Which inputs those are comes from the patterns (#418), so a
  // repo whose convention writes no issue number is not shown a field for one.
  lines.push(Line::from(vec![
    Span::raw("  Branch : "),
    Span::styled(branch, Style::default().fg(app.theme.branch)),
  ]));
  lines.push(Line::from(vec![
    Span::raw("  Dir    : "),
    Span::styled(dirname, Style::default().fg(app.theme.dirty)),
  ]));
  lines.push(Line::from(String::new()));
  if freeform {
    lines.push(field_input_line(
      &label("Name"),
      &app.create_form.name,
      app.create_form.field == Field::Name,
      value_w,
      accent,
      muted,
      surface,
    ));
  } else {
    lines.extend(form_field_lines(app, type_str, type_desc, value_w, label_w));
  }

  let height = lines.len() as u16 + 4 + 2 /* border */ + 2 /* vertical padding */;
  let area = centered_box(70, 72, height, term);
  let inner = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Min(1),    // title + form fields
      Constraint::Length(1), // loader / failure
      Constraint::Length(1), // buttons
      Constraint::Length(1), // hint gap
      Constraint::Length(1), // hint
    ])
    .split(block.inner(area));

  f.render_widget(Clear, area);
  f.render_widget(block, area);
  f.render_widget(Paragraph::new(lines), inner[0]);

  if app.is_create_worktree_loading() {
    f.render_widget(
      LoaderWidget::running(
        app.spinner.glyph(DOT_FRAMES),
        TaskKind::CreateWorktree.loading_label(),
        None,
        &app.theme,
      )
      .alignment(Alignment::Center),
      inner[1],
    );
  } else if let Some(error) = app.create_failure.as_deref() {
    f.render_widget(
      LoaderWidget::failed("create failed", Some(error), &app.theme).alignment(Alignment::Center),
      inner[1],
    );
  }

  if !app.is_create_worktree_loading() {
    f.render_widget(
      Paragraph::new(create_buttons_line(accent, muted)).alignment(Alignment::Center),
      inner[2],
    );
    f.render_widget(
      Paragraph::new(modal_hint_for_context_with_fields(
        app.create_hint_context(),
        &app.keymap,
        &app.modal_keymap,
        &app.theme,
        app.create_form.fields(),
      )),
      inner[4],
    );
  }
}

/// The create overlay's ` Create ` / ` Cancel ` button row (issue #217).
/// Mirrors [`confirm_buttons_line`]'s flat coloured chips, but — the create
/// action being non-destructive — primes `Create` as the reversed-accent
/// chip rather than defaulting focus to Cancel. Pure so the chip contract
/// is pinned by `tests/tui_ui_helpers_tests.rs`.
pub fn create_buttons_line(accent: Color, muted: Color) -> Line<'static> {
  primary_cancel_buttons_line(" Create ", accent, muted)
}

/// Button row for the rename (`c`) modal: a reversed-accent `Rename` chip
/// beside a muted `Cancel`. Mirrors [`create_buttons_line`] but labels the
/// primary action "Rename" so the modal's button matches its title and the
/// Enter action (Codex review on PR #292, P3).
pub fn rename_buttons_line(accent: Color, muted: Color) -> Line<'static> {
  primary_cancel_buttons_line(" Rename ", accent, muted)
}

fn primary_cancel_buttons_line(primary_label: &'static str, accent: Color, muted: Color) -> Line<'static> {
  let primary = chip_style(accent);
  let idle = Style::default().fg(muted).add_modifier(Modifier::BOLD);
  Line::from(vec![
    Span::styled(primary_label, primary),
    Span::raw("   "),
    Span::styled(" Cancel ", idle),
  ])
}

/// A horizontal `‹ name ›` branch-type selector row for the create overlay
/// (issue #217 — replaces the bordered up/down box). `label` leads the row
/// muted; the arrows + selected name read in the accent when focused, and
/// the type's description trails muted. Pure for
/// `tests/tui_ui_helpers_tests.rs`.
pub fn type_selector_line(
  label: &str,
  name: &str,
  desc: &str,
  focused: bool,
  accent: Color,
  muted: Color,
) -> Line<'static> {
  let arrow_style = if focused {
    Style::default().fg(accent).add_modifier(Modifier::BOLD)
  } else {
    Style::default().fg(muted)
  };
  // Focused, the selected value reads as a reversed-accent chip (the same
  // badge style as the buttons) so it stands out as an editable control;
  // idle it is plain white text between muted arrows.
  let name_style = if focused {
    chip_style(accent)
  } else {
    Style::default().fg(Color::White)
  };
  Line::from(vec![
    Span::raw("  "),
    Span::styled(label.to_string(), Style::default().fg(muted)),
    Span::raw("  "),
    Span::styled("‹ ", arrow_style),
    Span::styled(format!(" {name} "), name_style),
    Span::styled(" ›", arrow_style),
    Span::raw("  "),
    Span::styled(desc.to_string(), Style::default().fg(muted)),
  ])
}

/// A single-row labelled input with a background surface for the create
/// overlay (issue #217 — replaces the 3-row bordered field). `label` leads
/// muted; the value sits in a `value_width`-wide background-filled field so
/// it reads as one input row. The focused field brightens to the accent
/// background and shows a `_` cursor. Pure for
/// `tests/tui_ui_helpers_tests.rs`.
pub fn field_input_line(
  label: &str,
  value: &str,
  focused: bool,
  value_width: usize,
  accent: Color,
  muted: Color,
  surface: Color,
) -> Line<'static> {
  let cursor = if focused { "_" } else { "" };
  let mut field = format!(" {value}{cursor}");
  let len = field.chars().count();
  if len < value_width {
    field.push_str(&" ".repeat(value_width - len));
  }
  let field_style = if focused {
    Style::default().fg(Color::Black).bg(accent)
  } else {
    Style::default().fg(Color::White).bg(surface)
  };
  Line::from(vec![
    Span::raw("  "),
    Span::styled(label.to_string(), Style::default().fg(muted)),
    Span::raw("  "),
    Span::styled(field, field_style),
  ])
}

/// A single selectable row of the link prompt's `ChooseTarget` picker
/// (issue #217, polished in #220): the selected row uses the same
/// reversed-bold accent chip treatment as modal buttons; idle rows stay
/// muted. `key` is the direct-pick shortcut (`i` / `p`). Pure so the
/// highlight contract is pinned by `tests/tui_ui_helpers_tests.rs`.
pub fn link_target_line(key: &str, label: &str, selected: bool, accent: Color, muted: Color) -> Line<'static> {
  const BUTTON_WIDTH: usize = 17; // " p  Pull Request "
  let button = format!(" {key}  {label} ");
  let button = format!("{button:<BUTTON_WIDTH$}");
  if selected {
    let chip = chip_style(accent);
    return Line::from(vec![Span::raw("  "), Span::styled(button, chip)]);
  }

  let idle = Style::default().fg(muted);
  Line::from(vec![Span::raw("  "), Span::styled(button, idle)])
}

/// Modal width for the Link prompt. Pure so the visual budget remains pinned
/// without a terminal renderer in `tests/tui_ui_helpers_tests.rs`.
pub fn link_prompt_modal_width(term_width: u16) -> u16 {
  let width = if term_width <= 80 {
    term_width.saturating_mul(80) / 100
  } else {
    term_width.saturating_mul(60) / 100
  };
  width.min(72).min(term_width)
}

/// Modal width for the exec / clean overlays (issue #334 polish). A bit wider
/// than the link-prompt modal so the full-width clean report (icon + dir name
/// pinned left, size pinned right) uses the horizontal space — but capped so
/// the name↔size gap never stretches absurdly on an ultra-wide terminal.
/// ~62 % of the width (90 % when ≤ 80 cols), clamped to `[48, 88]`.
pub fn overlay_modal_width(term_width: u16) -> u16 {
  let pct = if term_width <= 80 { 90 } else { 62 };
  (term_width.saturating_mul(pct) / 100).clamp(48, 88).min(term_width)
}

/// Section-heading style for the Keybindings overlay body. Kept pure so the
/// title/body colour split is pinned outside the ratatui renderer.
pub fn help_section_style(section: Color) -> Style {
  Style::default().fg(section).add_modifier(Modifier::BOLD)
}

/// One aligned detail row for destructive confirmation summaries.
pub fn confirm_detail_line(
  label: &str,
  value: impl Into<String>,
  label_width: usize,
  label_color: Color,
  value_style: Style,
) -> Line<'static> {
  Line::from(vec![
    Span::styled(
      format!("{label:<label_width$}  ", label_width = label_width),
      Style::default().fg(label_color),
    ),
    Span::styled(value.into(), value_style),
  ])
}

pub fn delete_worktree_title() -> &'static str {
  "Delete Worktree"
}

/// Title of the confirm overlay for a batch of `count` targets (issue #484).
/// A batch of one is the pre-#484 single delete, title included.
pub fn delete_batch_title(count: usize) -> String {
  if count > 1 {
    format!("Delete {} Worktrees", count)
  } else {
    delete_worktree_title().to_string()
  }
}

pub fn confirm_delete_branch_line(
  enabled: bool,
  key: &str,
  label_width: usize,
  accent: Color,
  muted: Color,
) -> Line<'static> {
  let key_style = chip_style(accent);
  let value_style = chip_style(if enabled { accent } else { muted });
  Line::from(vec![
    Span::styled(
      format!("{:<label_width$}  ", "Delete Branch", label_width = label_width),
      Style::default().fg(muted),
    ),
    Span::styled(format!(" {key} "), key_style),
    Span::raw("  "),
    Span::styled(format!(" {enabled} "), value_style),
  ])
}

pub fn help_body_section_color(theme: &Theme) -> Color {
  theme.locked
}

/// Direct-pick keys (`issue`, `pr`) for the link / open-menu target chips,
/// resolved from the active context's modal bindings (#219 review) so a
/// rebind of `[tui.keys.modal.link.choose_target]` / `[tui.keys.modal.open_menu]` shows
/// through instead of the literal `i` / `p`. An unbound verb yields an empty
/// string — the chip then renders label-only rather than a phantom key.
pub fn link_target_keys(ctx: HintContext, modal: &ModalKeymap) -> (String, String) {
  let (issue, pr) = match ctx {
    HintContext::OpenMenu => (ModalAction::OpenMenuIssue, ModalAction::OpenMenuPr),
    _ => (ModalAction::LinkChooseIssue, ModalAction::LinkChoosePr),
  };
  (
    modal.primary_key(issue).unwrap_or_default(),
    modal.primary_key(pr).unwrap_or_default(),
  )
}

pub fn link_open_modal_lines(app: &App, title: &str, selected: Option<LinkTarget>) -> Vec<Line<'static>> {
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let ctx = if title == "Link" {
    HintContext::LinkPrompt
  } else {
    HintContext::OpenMenu
  };
  // #219: the direct-pick chips track the active context's issue/pr bindings
  // (like the footer below) so a rebind shows through instead of `i` / `p`.
  let (issue_key, pr_key) = link_target_keys(ctx, &app.modal_keymap);
  let mut lines = overlay_title_lines(title, accent);
  lines.extend(github_status_lines(app, 56));
  lines.push(Line::from(""));
  lines.push(link_target_line(&issue_key, "Issue", selected == Some(LinkTarget::Issue), accent, muted).centered());
  lines.push(link_target_line(&pr_key, "Pull Request", selected == Some(LinkTarget::Pr), accent, muted).centered());
  push_modal_hint(&mut lines, ctx, &app.keymap, &app.modal_keymap, &app.theme);
  lines
}

fn draw_confirm(f: &mut Frame, app: &App) {
  let muted = app.theme.muted;
  // The destructive modal reads in the theme's "danger" colour (the
  // same role the prunable `⚠` badge uses), so it tracks `[theme]`
  // instead of the pre-#187 hard-coded `Red`.
  let danger = app.theme.prunable;

  let block = overlay_block(danger);

  // #484: the overlay is about the batch snapshotted when it opened, not
  // about wherever the cursor sits now.
  let targets = app.pending_delete();
  if targets.is_empty() {
    let mut lines = overlay_title_lines(delete_worktree_title(), danger);
    lines.push(Line::from("nothing selected").centered());
    let height = lines.len() as u16 + 2 /* border */ + 2 /* padding */;
    let area = centered_h(40, height, f.area());
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(block), area);
    return;
  }

  // Width first (a fixed % of the terminal) so a long path / name can be
  // middle-ellipsized to one line instead of wrapping mid-path (#187
  // review). `text_w` is the room inside the border + padding.
  let term = f.area();
  let outer_w = term.width.saturating_mul(62) / 100;
  let text_w = outer_w.saturating_sub(6) as usize;
  let label_w = "Delete Branch".chars().count();
  let value_w = text_w.saturating_sub(label_w + 2).max(1);

  // Title stays centred; details use an aligned label/value grid so the
  // destructive target is easier to scan (#220 visual follow-up).
  let mut content: Vec<Line> = overlay_title_lines(&delete_batch_title(targets.len()), danger);
  if targets.len() > 1 {
    // A batch reports its size, not its members (#484): the user picked the
    // rows deliberately and the list is already on screen behind the modal.
    content.push(confirm_detail_line(
      "Worktrees",
      format!("{} selected", targets.len()),
      label_w,
      muted,
      Style::default().fg(app.theme.dirty).add_modifier(Modifier::BOLD),
    ));
    let with_branch = targets
      .iter()
      .filter(|t| app.worktrees.iter().any(|w| w.path == t.path && w.branch.is_some()))
      .count();
    content.push(confirm_detail_line(
      "Branches",
      format!("{} of {} carry a branch", with_branch, targets.len()),
      label_w,
      muted,
      Style::default().fg(app.theme.branch),
    ));
  } else {
    // Resolve the row from the snapshot's path rather than from the cursor:
    // a refresh landing during the countdown can have moved the cursor.
    let target = &targets[0];
    let row = app.worktrees.iter().find(|w| w.path == target.path);
    let name = ellipsize_middle(row.map(|w| w.name.as_str()).unwrap_or(&target.id), value_w);
    let path = ellipsize_middle(&tilde_compress(&target.path.display().to_string()), value_w);
    content.push(confirm_detail_line(
      "Worktree",
      name,
      label_w,
      muted,
      Style::default().fg(app.theme.dirty).add_modifier(Modifier::BOLD),
    ));
    content.push(confirm_detail_line(
      "Path",
      path,
      label_w,
      muted,
      Style::default().fg(muted),
    ));
    if let Some(b) = row.and_then(|w| w.branch.as_deref()) {
      let branch = ellipsize_middle(b, value_w);
      content.push(confirm_detail_line(
        "Branch",
        branch,
        label_w,
        muted,
        Style::default().fg(app.theme.branch),
      ));
    }
  }
  content.push(Line::from(""));
  content.push(confirm_delete_branch_line(
    app.delete_branch_on_remove,
    // Derive the live chord (Codex review on PR #292): ToggleDeleteBranch is
    // `D` since #290, not the pre-#290 `p`, and tracks `[tui.keys]` overrides.
    &action_chord(&app.keymap, Action::ToggleDeleteBranch, "D"),
    label_w,
    app.theme.accent,
    muted,
  ));

  // Size the modal to its content: the title + description rows plus the
  // fixed rows (loader / buttons / hint gap / hint), the rounded border and the
  // shared interior padding — no more fixed 44%-tall box that dwarfed its
  // few lines (#187 review).
  let height = content.len() as u16 + 4 + 2 /* border */ + 2 /* padding */;
  let area = centered_h(62, height, term);
  f.render_widget(Clear, area);

  // Five stacked regions inside the padded frame: the title + description,
  // a loader/countdown row, the button row, a gap, and a statusbar-style hint. The
  // loader row stays reserved (Length 1) even when idle so the buttons
  // never jump as the countdown arms. Split `block.inner` so the shared
  // padding owns the breathing room (issue #217).
  let inner = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Min(1),    // title + description
      Constraint::Length(1), // loader / countdown
      Constraint::Length(1), // buttons
      Constraint::Length(1), // hint gap
      Constraint::Length(1), // hint
    ])
    .split(block.inner(area));
  f.render_widget(block, area);

  f.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }), inner[0]);

  // --- loader + countdown ---
  if app.is_delete_worktree_loading() {
    f.render_widget(
      LoaderWidget::running(
        app.spinner.glyph(DOT_FRAMES),
        TaskKind::DeleteWorktree.loading_label(),
        None,
        &app.theme,
      )
      .alignment(Alignment::Center),
      inner[1],
    );
  } else if let Some(error) = app.delete_failure.as_deref() {
    f.render_widget(
      LoaderWidget::failed("delete failed", Some(error), &app.theme).alignment(Alignment::Center),
      inner[1],
    );
  } else if app.confirm_is_countdown_mode() && app.confirm.is_armed() {
    let now = Instant::now();
    let mut spans = vec![Span::styled(
      format!("{} ", app.spinner.glyph(DOT_FRAMES)),
      Style::default().fg(danger).add_modifier(Modifier::BOLD),
    )];
    spans.extend(countdown_bar(
      app.confirm_countdown_progress(now),
      app.confirm_countdown_remaining_secs(now),
      danger,
      app.theme.dirty,
      muted,
    ));
    f.render_widget(Paragraph::new(Line::from(spans)).alignment(Alignment::Center), inner[1]);
  }

  // --- buttons (focused one highlighted) ---
  if !app.is_delete_worktree_loading() {
    f.render_widget(
      Paragraph::new(confirm_buttons_line(
        app.confirm.focused_button(),
        app.theme.accent,
        muted,
      ))
      .alignment(Alignment::Center),
      inner[2],
    );

    f.render_widget(
      Paragraph::new(modal_hint_for_context(
        HintContext::Confirm,
        &app.keymap,
        &app.modal_keymap,
        &app.theme,
      )),
      inner[4],
    );
  }
}

/// The ` Confirm ` ` Cancel ` button row (#187, restyled in #217). The
/// buttons are flat coloured chips — no square brackets: the focused one
/// gets the reversed-bold accent chip (the same badge style as the bottom
/// statusline and the help overlay), the idle one reads muted-bold. Focus
/// defaults to Cancel, so the destructive button is never the one a stray
/// `Enter` lands on. Pure so the chip contract is pinned by
/// `tests/tui_ui_helpers_tests.rs`.
pub fn confirm_buttons_line(focus: ConfirmButton, accent: Color, muted: Color) -> Line<'static> {
  let focused = chip_style(accent);
  let idle = Style::default().fg(muted).add_modifier(Modifier::BOLD);
  let (confirm_style, cancel_style) = match focus {
    ConfirmButton::Confirm => (focused, idle),
    ConfirmButton::Cancel => (idle, focused),
  };
  Line::from(vec![
    Span::styled(" Confirm ", confirm_style),
    Span::raw("   "),
    Span::styled(" Cancel ", cancel_style),
  ])
}

/// Build the `[████░░] Ns` countdown line, themed by the caller (#187
/// review: was hard-coding `Red` / `Yellow` / `DarkGray`, which clashed
/// with non-default themes). Width is fixed at 10 cells so the bar reads
/// the same regardless of modal size. The control hint (`n` / `Esc` to
/// cancel) lives in the modal's hint row, not here, so the controls have
/// a single source of truth.
fn countdown_bar<'a>(
  progress: f64,
  remaining_secs: u64,
  filled_color: Color,
  secs_color: Color,
  frame_color: Color,
) -> Vec<Span<'a>> {
  const CELLS: usize = 10;
  let filled = filled_cells_for_progress(progress, CELLS);
  let bar: String = std::iter::repeat_n('█', filled)
    .chain(std::iter::repeat_n('░', CELLS - filled))
    .collect();
  vec![
    Span::styled("  [", Style::default().fg(frame_color)),
    Span::styled(bar, Style::default().fg(filled_color).add_modifier(Modifier::BOLD)),
    Span::styled("] ", Style::default().fg(frame_color)),
    Span::styled(
      format!("{remaining_secs}s"),
      Style::default().fg(secs_color).add_modifier(Modifier::BOLD),
    ),
  ]
}

/// Compute the number of filled cells for a countdown progress bar.
///
/// Contract pinned by Copilot review on PR #66:
/// - Returns `0` when `progress <= 0.0`.
/// - Returns `cells` only when `progress >= 1.0`. For any
///   `progress in (0.0, 1.0)`, the result is strictly less than
///   `cells` — the last cell stays empty so the visual "bar full"
///   moment lines up with the actual delete firing (not 50ms before).
/// - Clamps to `cells` for `progress > 1.0` (handles float drift on
///   an overshooting tick).
///
/// Uses `floor` rather than `round` so a progress of `0.95` paints 9
/// cells, not 10 — the previous `round()` behaviour painted a full bar
/// before the destructive action actually fired.
pub fn filled_cells_for_progress(progress: f64, cells: usize) -> usize {
  if progress >= 1.0 {
    return cells;
  }
  if progress <= 0.0 || cells == 0 {
    return 0;
  }
  let raw = (progress * cells as f64).floor() as usize;
  // Reserve the last cell for the progress >= 1.0 moment.
  raw.min(cells.saturating_sub(1))
}

pub fn bootstrap_report_lines(report: Option<&BootstrapReport>, theme: &Theme) -> Vec<Line<'static>> {
  let mut lines: Vec<Line<'static>> = Vec::new();
  if let Some(report) = report {
    for step in &report.steps {
      let sigil = step.status.sigil();
      let color = match step.status {
        StepStatus::Ok => theme.clean,
        StepStatus::Skipped => theme.muted,
        StepStatus::Warning => theme.dirty,
        StepStatus::Failed => theme.prunable,
      };
      lines.push(Line::from(vec![
        Span::styled(
          format!(" {} ", sigil),
          Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(step.label.clone(), Style::default().fg(theme.name)),
      ]));
      for detail_line in step.detail.lines() {
        lines.push(Line::from(Span::styled(
          format!("      {}", detail_line),
          Style::default().fg(theme.muted),
        )));
      }
    }
  } else {
    lines.push(Line::from("(no report)"));
  }
  lines
}

fn draw_report(f: &mut Frame, app: &App) {
  let accent = app.theme.accent;
  let logs = bootstrap_report_lines(app.report.as_ref(), &app.theme);

  // Size to the report length (+ border + padding), capped at 80% of the
  // screen so a long report stays on-screen rather than a fixed 80%-tall
  // box (#187).
  let term = f.area();
  let logs_height = (logs.len() as u16 + 2/* nested border */).max(3);
  let height = (2 /* title */ + logs_height + 2 /* gap + hint */ + 2 /* border */ + 2/* padding */)
    .min(term.height.saturating_mul(80) / 100);
  let area = centered_h(80, height, term);
  let block = overlay_block(accent);
  let inner = block.inner(area);
  let layout = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Length(1), // title
      Constraint::Length(1), // title gap
      Constraint::Min(3),    // logs pane
      Constraint::Length(1), // hint gap
      Constraint::Length(1), // hint
    ])
    .split(inner);
  f.render_widget(Clear, area);
  f.render_widget(block, area);
  f.render_widget(
    Paragraph::new(
      Line::from(Span::styled(
        "Bootstrap Report",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
      ))
      .centered(),
    ),
    layout[0],
  );
  // A modal keeps its rules whatever `[tui] compact` says: a panel
  // floating over content is exactly where a border earns its keep.
  render_section(
    f,
    layout[2],
    " Logs ",
    SectionBody::new(&logs),
    Chrome::boxed(accent),
    0,
    None,
  );
  f.render_widget(
    Paragraph::new(modal_hint_for_context(
      HintContext::Report,
      &app.keymap,
      &app.modal_keymap,
      &app.theme,
    )),
    layout[4],
  );
}

// ── Note editor (issue #515) ───────────────────────────────────────────────

/// Render the in-TUI note editor: the branch in the title, the buffer in an
/// 80% x 80% centred box, and the cursor placed on the terminal so the
/// caret blinks where the next character lands.
///
/// The scroll is clamped **here**, with the height the layout actually gave,
/// which is why [`crate::tui::state::note_editor::NoteEditor`] does not try
/// to know its own viewport ahead of a resize. That same call teaches the
/// editor what a page key should move by.
fn draw_note_editor(f: &mut Frame, app: &mut App) {
  let area = centered(80, 80, f.area());
  f.render_widget(Clear, area);

  let title = match app.note_editor.as_ref() {
    Some(editor) => format!(" note · {} ", crate::naming::sanitise_for_terminal(&editor.branch)),
    None => " note ".to_string(),
  };
  let block = overlay_block(app.theme.accent)
    .title(title)
    .title_alignment(ratatui::layout::Alignment::Center);
  let inner = block.inner(area);
  f.render_widget(block, area);

  let Some(editor) = app.note_editor.as_mut() else {
    return;
  };
  editor.clamp_scroll(inner.height as usize);

  let visible: Vec<Line> = editor
    .lines
    .iter()
    .skip(editor.scroll)
    .take(inner.height as usize)
    // Not wrapped: a wrapped line makes the screen row the cursor sits on
    // stop matching its buffer line, and the caret would land somewhere
    // else entirely. Long lines scroll out of view instead — `Ctrl+e` is
    // the answer for prose that needs the width.
    .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(app.theme.name))))
    .collect();
  f.render_widget(Paragraph::new(visible), inner);

  // The caret. Columns are `char`s in the buffer and cells on screen, which
  // is the same approximation the cursor itself makes (see the state module
  // note): a wide CJK glyph puts the caret one cell left of the glyph it is
  // about to push.
  let row = editor.cursor_line.saturating_sub(editor.scroll) as u16;
  if row < inner.height {
    let col = (editor.cursor_col as u16).min(inner.width.saturating_sub(1));
    f.set_cursor_position((inner.x + col, inner.y + row));
  }
}

// ── PTY overlay (issue #35) ────────────────────────────────────────────────

/// Render the embedded PTY overlay (lazygit or native terminal). The overlay
/// occupies ~90 % × 90 % of the terminal, centred and drawn over the list
/// view. The rendered PTY content fills the entire inner area of the block
/// so the child process gets as much screen real-estate as possible.
fn draw_pty_overlay(f: &mut Frame, app: &mut App) {
  let term = f.area();
  let area = centered(90, 90, term);

  f.render_widget(Clear, area);

  let title = match app.pty_overlay.as_ref().map(|p| (p.kind, p.finished)) {
    Some((PtyKind::LazyGit, _)) => " LazyGit ",
    Some((PtyKind::Terminal, _)) => " Terminal ",
    Some((PtyKind::Review, _)) => " Review ",
    Some((PtyKind::Exec, false)) => " Exec ",
    // #325: once the one-shot command exits, the title invites dismissal.
    Some((PtyKind::Exec, true)) => " Exec · done — press any key ",
    None => " Overlay ",
  };
  let block = overlay_block(app.theme.accent)
    .title(title)
    .title_alignment(ratatui::layout::Alignment::Center);
  let inner = block.inner(area);
  f.render_widget(block, area);

  if let Some(pty) = app.pty_overlay.as_ref() {
    let pseudo_terminal = tui_term::widget::PseudoTerminal::new(pty.parser.screen());
    f.render_widget(pseudo_terminal, inner);
  }
}

fn centered(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
  let v = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Percentage((100 - pct_y) / 2),
      Constraint::Percentage(pct_y),
      Constraint::Percentage((100 - pct_y) / 2),
    ])
    .split(area);
  Layout::default()
    .direction(Direction::Horizontal)
    .constraints([
      Constraint::Percentage((100 - pct_x) / 2),
      Constraint::Percentage(pct_x),
      Constraint::Percentage((100 - pct_x) / 2),
    ])
    .split(v[1])[1]
}

/// Center a box of an **absolute** `width`/`height` (in cells) inside `area`,
/// clamping each dimension to the area so an oversized modal cannot overflow
/// the frame. Shared by the open-menu and link-prompt modals (issue #243) and
/// the percentage-based [`centered_h`], unifying the three centering paths.
pub fn centered_abs(width: u16, height: u16, area: Rect) -> Rect {
  let width = width.min(area.width);
  let height = height.min(area.height);
  let x = area.x + area.width.saturating_sub(width) / 2;
  let y = area.y + area.height.saturating_sub(height) / 2;
  Rect { x, y, width, height }
}

/// Centre a box of `width_pct`% width and a fixed `height` (rows) in
/// `area`. Unlike [`centered`], the height is absolute so an overlay can
/// size itself to its content rather than a fixed percentage of the
/// screen (#187 — the confirm modal was far taller than its few lines).
/// Delegates the centering arithmetic to [`centered_abs`].
fn centered_h(width_pct: u16, height: u16, area: Rect) -> Rect {
  let width = area.width.saturating_mul(width_pct) / 100;
  centered_abs(width, height, area)
}

/// Like [`centered_h`] but also caps the width at `max_width` columns so a
/// form modal does not stretch edge-to-edge on a wide terminal (issue #217
/// — the create overlay's input surfaces spanned the whole screen).
fn centered_box(width_pct: u16, max_width: u16, height: u16, area: Rect) -> Rect {
  let height = height.min(area.height);
  let width = (area.width.saturating_mul(width_pct) / 100)
    .min(max_width)
    .min(area.width);
  let x = area.x + area.width.saturating_sub(width) / 2;
  let y = area.y + area.height.saturating_sub(height) / 2;
  Rect { x, y, width, height }
}

/// A modal overlay frame: a rounded border in `color` with interior
/// padding on every side. Shared by every overlay (#187) so the confirm /
/// help / create / report / open / link / palette modals read consistently.
/// The title is *not* embedded in the border any more (issue #217): it
/// lives inside the frame as its own centred line via [`overlay_title_lines`]
/// so the border stays clean and no content hugs the edge. The padding
/// (2 cols horizontal, 1 row vertical) is the breathing room callers must
/// account for when sizing — inner height shrinks by 2 rows, inner width by
/// 4 cols, on top of the 2-cell border.
fn overlay_block(color: Color) -> Block<'static> {
  Block::default()
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .padding(Padding::symmetric(2, 1))
    .border_style(Style::default().fg(color))
}

/// The detached modal title: a centred bold line in `color` followed by a
/// blank spacer row, prepended to a modal's content so the title sits
/// inside the rounded frame rather than embedded in the top border
/// (issue #217). Returns two lines, so callers sizing to content add 2.
fn overlay_title_lines(title: &str, color: Color) -> Vec<Line<'static>> {
  vec![
    Line::from(Span::styled(
      title.to_string(),
      Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
    .centered(),
    Line::from(String::new()),
  ]
}

/// Middle-ellipsize `s` to at most `max` display columns, keeping the
/// head and tail so a long path keeps both its root and the worktree
/// name (e.g. `~/Projects/…/feat-187-modal`). Returns `s` unchanged when
/// it already fits, and a lone `…` when `max` is too small to keep
/// anything either side. Counts by `char`, not byte, so multi-byte path
/// segments are not sliced mid-codepoint.
pub fn ellipsize_middle(s: &str, max: usize) -> String {
  let count = s.chars().count();
  if count <= max {
    return s.to_string();
  }
  if max <= 1 {
    return "…".to_string();
  }
  let keep = max - 1; // reserve one column for the ellipsis
  let head = keep.div_ceil(2);
  let tail = keep - head;
  let head_str: String = s.chars().take(head).collect();
  let tail_str: String = s.chars().skip(count - tail).collect();
  format!("{head_str}…{tail_str}")
}

/// Clip `s` to `max` columns, neutralising what must never reach the terminal
/// first (issue #506).
///
/// The sanitisation is here rather than at each call site because this is the
/// one funnel every width-constrained cell already goes through, table rows
/// included, so a cell added later inherits it instead of having to remember
/// it. It is a no-op on ordinary text.
///
/// Measured, ratatui 0.30 drops zero-width control bytes on every render path,
/// but `List` and `Table` keep the `Bidi_Control` characters, which is exactly
/// where a branch name lands: git's ref rules refuse the ASCII controls and
/// `~^:?*[`, not the Unicode format characters, so a fetched ref can carry one
/// and a row can then read in an order the ref is not stored in.
///
/// It runs **before** the width count, so what is measured is what is drawn: a
/// replacement is one char for one char, but a `Bidi_Control` character can
/// measure zero columns where `?` measures one.
fn trunc(s: &str, max: usize) -> String {
  let s = crate::naming::sanitise_for_terminal(s);
  if s.chars().count() <= max {
    s
  } else {
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
  }
}

// ---- Issue/PR linking (issue #67) ---------------------------------------

fn draw_open_menu(f: &mut Frame, app: &App) {
  let accent = app.theme.accent;
  let lines = link_open_modal_lines(app, "Open in Browser", Some(app.open_menu_selected));
  let height = lines.len() as u16 + 2 /* border */ + 2 /* padding */;
  let term = f.area();
  let width = link_prompt_modal_width(term.width);
  let area = centered_abs(width, height, term);
  f.render_widget(Clear, area);
  f.render_widget(Paragraph::new(lines).block(overlay_block(accent)), area);
}

fn draw_link_prompt(f: &mut Frame, app: &App) {
  let accent = app.theme.accent;
  let lines = match app.link_prompt_stage() {
    LinkPromptStage::ChooseTarget => {
      // A vertical selectable list (#217): j/k move the highlight, Enter
      // links the highlighted row, i/p stay direct picks. The highlighted
      // row reads in the accent.
      let selected = app.link_prompt_selected();
      link_open_modal_lines(app, "Link", Some(selected))
    }
    LinkPromptStage::InputNumber => {
      let label = match app.link_prompt_target() {
        Some(super::app::LinkTarget::Issue) => "issue #",
        Some(super::app::LinkTarget::Pr) => "PR #",
        None => "#",
      };
      let mut lines = overlay_title_lines(
        &format!("type the {} number", label.trim_end_matches('#').trim()),
        accent,
      );
      lines.push(Line::from(format!("  {}{}_", label, app.link_prompt_number_input())));
      push_modal_hint(
        &mut lines,
        HintContext::LinkInputNumber,
        &app.keymap,
        &app.modal_keymap,
        &app.theme,
      );
      lines
    }
  };
  let height = lines.len() as u16 + 2 /* border */ + 2 /* padding */;
  let term = f.area();
  let width = link_prompt_modal_width(term.width);
  let area = centered_abs(width, height, term);
  f.render_widget(Clear, area);
  f.render_widget(Paragraph::new(lines).block(overlay_block(accent)), area);
}

/// Magnitude heatmap for a reclaimable size (issue #325 overlay polish):
/// green (small) → yellow (medium) → red (large) so a big reclaim stands out
/// at a glance. Thresholds tuned for build artifacts (50 MiB / 500 MiB).
pub fn reclaim_size_color(bytes: u64, theme: &Theme) -> Color {
  const MIB: u64 = 1024 * 1024;
  if bytes >= 500 * MIB {
    theme.prunable
  } else if bytes >= 50 * MIB {
    theme.dirty
  } else {
    theme.clean
  }
}

/// A nerd-font glyph matched to a reclaimable directory name (issue #334
/// polish) — the ecosystem the artifact belongs to (`node_modules` → node,
/// `target` → Rust, `vendor` → PHP, `.venv` → Python, `dist`/`build` →
/// package, `.cache` → archive…), falling back to a generic folder. Leading
/// dots are stripped so `.venv` / `.nuxt` match like `venv` / `nuxt`.
pub fn clean_dir_icon(rel: &str) -> &'static str {
  match rel.trim_start_matches('.').to_ascii_lowercase().as_str() {
    "node_modules" => "\u{e718}",      // nf-dev-nodejs
    "target" => wt_tree::WT_RUST_ICON, // nf-dev-rust
    "vendor" => "\u{e73d}",            // nf-dev-php
    "venv" | "__pycache__" | "pytest_cache" | "mypy_cache" | "tox" => "\u{e73c}", // nf-dev-python
    "dist" | "build" | "out" | "output" | "bin" => "\u{f487}", // nf-oct-package
    "cache" | "turbo" | "parcel-cache" => "\u{f187}", // nf-fa-archive
    "nuxt" | "next" | "svelte-kit" | "astro" | "vite" => "\u{e74e}", // nf-dev-javascript
    "coverage" => "\u{f201}",          // nf-fa-line_chart
    _ => wt_tree::WT_DIR_ICON,         // generic folder
  }
}

/// The visible `[start, end)` slice of a `len`-item picker when at most
/// `max_visible` rows fit, keeping `selected` in view (centred while
/// scrolling). Returns the whole list when it fits (issue #325 polish).
pub fn picker_window(len: usize, selected: usize, max_visible: usize) -> (usize, usize) {
  if max_visible == 0 || len <= max_visible {
    return (0, len);
  }
  let half = max_visible / 2;
  let start = selected.saturating_sub(half).min(len - max_visible);
  (start, start + max_visible)
}

/// Build the full-width, scrollable rows for an overlay profile picker (issue
/// #334 polish). Each row spans the modal's `inner` width — left-aligned so
/// the labels start at the same column and the selection highlight reads as a
/// full-width bar — and the visible window follows `selected` with
/// `↑ / ↓ N more` markers (centred) when the list overflows `max_visible`.
fn picker_lines(
  labels: &[&str],
  selected: usize,
  max_visible: usize,
  inner: usize,
  theme: &Theme,
) -> Vec<Line<'static>> {
  let mut out = Vec::new();
  if labels.is_empty() {
    return out;
  }
  // Width available for the label text after the ` ▸ ` marker gutter.
  let textw = inner.saturating_sub(3);
  let (start, end) = picker_window(labels.len(), selected, max_visible);
  if start > 0 {
    out.push(
      Line::from(Span::styled(
        format!("↑ {start} more"),
        Style::default().fg(theme.muted),
      ))
      .centered(),
    );
  }
  for (i, label) in labels.iter().enumerate().take(end).skip(start) {
    let marker = if i == selected { "▸" } else { " " };
    // Pad to the full inner width so the selection bar fills the whole row.
    let txt = format!(" {marker} {:<textw$}", ellipsize_middle(label, textw));
    let style = if i == selected {
      Style::default()
        .fg(theme.accent)
        .bg(theme.selection_bg)
        .add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(theme.muted)
    };
    out.push(Line::from(Span::styled(txt, style)));
  }
  if end < labels.len() {
    out.push(
      Line::from(Span::styled(
        format!("↓ {} more", labels.len() - end),
        Style::default().fg(theme.muted),
      ))
      .centered(),
    );
  }
  out
}

/// Render the exec profile picker overlay (issue #325). A small centred
/// modal listing the `[exec.profiles.*]` names; the highlighted row reads in
/// the accent (with a selection bar) and a `▸` marker, the rest muted. The
/// list is aligned, same-width, and scrolls to keep the highlight in view.
/// `Enter` resolves the highlight and the run loop spawns it in a PTY overlay.
fn draw_exec_picker(f: &mut Frame, app: &App) {
  let accent = app.theme.accent;
  let term = f.area();
  let width = overlay_modal_width(term.width);
  let inner = width.saturating_sub(6) as usize; // inside borders (1) + overlay_block padding (2) each side
  let mut lines = overlay_title_lines("Run an exec profile", accent);
  // Leave room for the title + hint + borders; the picker scrolls past that.
  let max_visible = (term.height as usize).saturating_sub(8).max(3);
  let labels: Vec<&str> = app.exec_picker.profiles().iter().map(String::as_str).collect();
  lines.extend(picker_lines(
    &labels,
    app.exec_picker.selected_index(),
    max_visible,
    inner,
    &app.theme,
  ));
  push_modal_hint(
    &mut lines,
    HintContext::ExecPicker,
    &app.keymap,
    &app.modal_keymap,
    &app.theme,
  );
  let height = lines.len() as u16 + 2 /* border */ + 2 /* padding */;
  let area = centered_abs(width, height, term);
  f.render_widget(Clear, area);
  f.render_widget(Paragraph::new(lines).block(overlay_block(accent)), area);
}

/// Render the generic detail overlay (issue #408). A centred modal listing
/// `(label, value)` rows with theme-mapped roles — agent sessions today, the
/// rich PR/Issue view tomorrow. Content is prebuilt state
/// ([`crate::tui::state::detail_overlay::DetailOverlay`]); this function
/// only paints it, so the render path stays pure.
fn draw_detail_overlay(f: &mut Frame, app: &App) {
  use crate::tui::state::detail_overlay::{DetailMode, DetailRole};
  let accent = app.theme.accent;
  let term = f.area();
  let width = overlay_modal_width(term.width);
  let inner = width.saturating_sub(6) as usize; // borders (1) + padding (2) each side
  let ov = &app.detail_overlay;

  // CI checks filter (issue #436): palette-style query over the overlay's
  // own rows — the highlight follows the filtered set, Enter opens the
  // highlighted check's details URL. Same fixed-height frame contract as
  // the attach prompt below (#445: typing must not resize the window).
  if ov.mode == DetailMode::Input && ov.kind == crate::tui::state::detail_overlay::DetailKind::CiChecks {
    let matches = app.ci_input_matches();
    let list_h = (term.height as usize).saturating_sub(12).clamp(3, 10);
    let (start, end) = picker_window(matches.len(), ov.input_selected, list_h);

    let mut lines = overlay_title_lines("Filter CI checks", accent);
    lines.push(Line::from(vec![
      Span::styled("filter: ", Style::default().fg(app.theme.muted)),
      Span::styled(
        ov.input.clone(),
        Style::default().fg(app.theme.name).add_modifier(Modifier::BOLD),
      ),
      Span::styled("▏", Style::default().fg(accent)),
    ]));
    lines.push(Line::from(String::new()));
    if matches.is_empty() {
      lines.push(Line::from(Span::styled(
        "no matching check",
        Style::default().fg(app.theme.muted),
      )));
    }
    for (i, row_idx) in matches.iter().enumerate().take(end).skip(start) {
      let Some(row) = ov.rows.get(*row_idx) else { continue };
      let label_color = match row.role {
        DetailRole::Success => app.theme.clean,
        DetailRole::Failure => app.theme.prunable,
        DetailRole::Running => app.theme.dirty,
        _ => app.theme.name,
      };
      let text = format!("{}  {}", row.label, row.value);
      let pad = inner.saturating_sub(text.chars().count());
      let mut style = Style::default().fg(label_color);
      if i == ov.input_selected {
        style = style.bg(app.theme.selection_bg).add_modifier(Modifier::BOLD);
      }
      lines.push(Line::from(Span::styled(format!("{}{}", text, " ".repeat(pad)), style)));
    }
    for _ in matches.len().min(end).saturating_sub(start)..list_h {
      lines.push(Line::from(String::new()));
    }
    let height = (2 + list_h + 2) as u16 + 2 /* border */ + 2 /* padding */;
    let area = centered_abs(width, height, term);
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(overlay_block(accent)), area);
    // Scrollbar over the LISTING sub-area (Codex review #455): the rows
    // start after border (1) + padding (1) + two title lines + the query
    // line + its blank spacer = y + 6 — anchoring at + 4 overlapped the
    // query and stopped short of the last rows.
    let list_rect = Rect {
      x: area.x + 1,
      y: area.y + 6,
      width: area.width.saturating_sub(2),
      height: list_h as u16,
    }
    .intersection(area);
    if list_rect.height > 0 {
      let _ = scrollable_body_area(f, list_rect, start as u16, matches.len(), &app.theme);
    }
    return;
  }

  // Attach-by-id prompt (user feedback 2026-07-22): palette-style query
  // over every detected session; Enter pins the highlighted candidate.
  if ov.mode == DetailMode::Input {
    let candidates = app.agent_input_candidates();
    // FIXED listing height (issue #445): the window is sized by the
    // terminal alone, never by the filtered candidate count — typing must
    // not resize the frame. Short lists blank-pad the remaining rows.
    // Capped at 10 rows: a full-terminal prompt reads as a takeover, not
    // a palette (user feedback 2026-07-23); the scrollbar covers the rest.
    let list_h = (term.height as usize).saturating_sub(12).clamp(3, 10);
    let (start, end) = picker_window(candidates.len(), ov.input_selected, list_h);
    let now = std::time::SystemTime::now();

    let mut lines = overlay_title_lines("Attach a session", accent);
    lines.push(Line::from(vec![
      Span::styled("id: ", Style::default().fg(app.theme.muted)),
      Span::styled(
        ov.input.clone(),
        Style::default().fg(app.theme.name).add_modifier(Modifier::BOLD),
      ),
      Span::styled("▏", Style::default().fg(accent)),
    ]));
    lines.push(Line::from(String::new()));
    if candidates.is_empty() {
      lines.push(Line::from(Span::styled(
        "no matching session",
        Style::default().fg(app.theme.muted),
      )));
    }
    for (i, sess) in candidates.iter().enumerate().take(end).skip(start) {
      let freshness = crate::agent_sessions::Freshness::classify(sess.last_activity, sess.ended, now);
      let color = match freshness {
        crate::agent_sessions::Freshness::Active => app.theme.clean,
        crate::agent_sessions::Freshness::Idle => app.theme.muted,
      };
      let identity = sess.name.as_deref().unwrap_or(&sess.id);
      let text = format!("{:<9} {}", sess.kind.display(), identity);
      let pad = inner.saturating_sub(text.chars().count());
      let mut style = Style::default().fg(color);
      let mut pad_style = Style::default();
      if i == ov.input_selected {
        style = style.bg(app.theme.selection_bg).add_modifier(Modifier::BOLD);
        pad_style = pad_style.bg(app.theme.selection_bg);
      }
      lines.push(Line::from(vec![
        Span::styled(text, style),
        Span::styled(" ".repeat(pad), pad_style),
      ]));
    }
    // Blank-pad up to the fixed window so the frame height is constant
    // whatever the filter matched (the empty-state line counts as one row).
    let shown = if candidates.is_empty() { 1 } else { end - start };
    for _ in shown..list_h {
      lines.push(Line::from(String::new()));
    }
    lines.push(Line::from(String::new()));
    lines.push(modal_hint_line(
      &[
        ("type", "filter"),
        ("↑/↓", "pick"),
        ("Enter", "attach"),
        ("Esc", "back"),
      ],
      &app.theme,
    ));
    let height = lines.len() as u16 + 2 /* border */ + 2 /* padding */;
    let area = centered_abs(width, height, term);
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(overlay_block(accent)), area);
    // Scrollbar over the listing sub-area when the candidates overflow the
    // fixed window — same affordance as the detail mode below (issue #445).
    // Intersected with the modal's real area: on a tiny terminal
    // `centered_abs` clamps the frame, and an un-clipped rect would render
    // past the ratatui buffer and panic (Codex review #445).
    let list_rect = Rect {
      x: area.x + 1,
      y: area.y + 2 /* border + padding */ + 2 /* title */ + 2, /* id line + blank */
      width: area.width.saturating_sub(2),
      height: list_h as u16,
    }
    .intersection(area);
    if list_rect.height > 0 {
      let _ = scrollable_body_area(f, list_rect, start as u16, candidates.len(), &app.theme);
    }
    return;
  }
  let total = ov.rows.len();
  // The modal height is derived from the VISIBLE row count, which is
  // constant while navigating — scrolling must never resize the frame
  // (user feedback 2026-07-22). The window follows the selection.
  let max_visible = (term.height as usize).saturating_sub(10).max(3);
  let visible = total.min(max_visible);
  let (start, end) = picker_window(total, ov.selected, visible);

  let label_w = ov.rows.iter().map(|r| r.label.chars().count()).max().unwrap_or(0);
  let mut lines = overlay_title_lines(&ov.title, accent);
  for (i, row) in ov.rows.iter().enumerate().take(end).skip(start) {
    let (label_color, value_color, value_bold) = match row.role {
      DetailRole::Active => (app.theme.clean, app.theme.clean, true),
      DetailRole::Muted => (app.theme.muted, app.theme.muted, false),
      DetailRole::Normal => (app.theme.name, app.theme.name, false),
      // #436: per-check CI outcomes, same theme roles as `ci_indicator`.
      DetailRole::Success => (app.theme.clean, app.theme.name, false),
      DetailRole::Failure => (app.theme.prunable, app.theme.name, false),
      DetailRole::Running => (app.theme.dirty, app.theme.name, false),
    };
    // Selection paints a full-width bar (picker convention): pad the row
    // out to the modal's inner width so the highlight reads as one block.
    // The optional `extra` detail (#436: workflow · duration) sits right-
    // aligned inside that bar, rendered muted. Its width is RESERVED
    // (Codex review #455): a long check name truncates with an ellipsis
    // instead of pushing the detail column past the clipping edge.
    let mut extra: String = row.extra.as_deref().unwrap_or("").to_string();
    let mut extra_cols = extra.chars().count();
    // The detail column is bounded too (Codex review #455): on a narrow
    // modal an oversized workflow name would run past the clipping edge
    // and ratatui cuts its RIGHT end — the duration, the very info the
    // column carries. Truncate from the workflow side instead (leading
    // ellipsis, the tail survives), reserving the value a dozen columns
    // or its full width when shorter.
    if extra_cols > 0 {
      let reserve = row.value.chars().count().min(12);
      let extra_budget = inner.saturating_sub(label_w + 2 + reserve + 2);
      if extra_cols > extra_budget {
        if extra_budget == 0 {
          extra.clear();
        } else {
          let tail: String = extra.chars().skip(extra_cols - (extra_budget - 1)).collect();
          extra = format!("…{tail}");
        }
        extra_cols = extra.chars().count();
      }
    }
    let value_budget = inner.saturating_sub(label_w + 2 + if extra_cols > 0 { extra_cols + 2 } else { 0 });
    let value: String = if row.value.chars().count() > value_budget {
      let mut v: String = row.value.chars().take(value_budget.saturating_sub(1)).collect();
      v.push('…');
      v
    } else {
      row.value.clone()
    };
    let text_cols = label_w + 2 + value.chars().count();
    let pad = inner.saturating_sub(text_cols + extra_cols);
    let mut label_style = Style::default().fg(label_color);
    let mut value_style = Style::default().fg(value_color);
    if value_bold {
      value_style = value_style.add_modifier(Modifier::BOLD);
    }
    let mut pad_style = Style::default();
    let mut extra_style = Style::default().fg(app.theme.muted);
    if i == ov.selected {
      label_style = label_style.bg(app.theme.selection_bg).add_modifier(Modifier::BOLD);
      value_style = value_style.bg(app.theme.selection_bg);
      pad_style = pad_style.bg(app.theme.selection_bg);
      extra_style = extra_style.bg(app.theme.selection_bg);
    }
    lines.push(Line::from(vec![
      Span::styled(format!("{:label_w$}  ", row.label), label_style),
      Span::styled(value, value_style),
      Span::styled(" ".repeat(pad), pad_style),
      Span::styled(extra, extra_style),
    ]));
  }
  // #436 validation feedback: the CI checks consumer advertises ITS verbs,
  // not the agents' attach / detach — the hint context follows the kind.
  let hint_ctx = match ov.kind {
    crate::tui::state::detail_overlay::DetailKind::CiChecks => HintContext::CiChecks,
    crate::tui::state::detail_overlay::DetailKind::RichIssue
    | crate::tui::state::detail_overlay::DetailKind::RichPr => HintContext::RichView,
    crate::tui::state::detail_overlay::DetailKind::Agents => HintContext::Detail,
  };
  push_modal_hint(&mut lines, hint_ctx, &app.keymap, &app.modal_keymap, &app.theme);
  let height = (2 + visible + 2) as u16 + 2 /* border */ + 2 /* padding */;
  let area = centered_abs(width, height, term);
  f.render_widget(Clear, area);
  f.render_widget(Paragraph::new(lines).block(overlay_block(accent)), area);
  // Scrollbar over the rows sub-area (right padding column) when the list
  // overflows — the missing affordance from the feedback.
  // Intersected with the modal's real area for the same tiny-terminal
  // clamp as the attach prompt above (Codex review #445).
  let rows_rect = Rect {
    x: area.x + 1,
    y: area.y + 2 /* border + padding */ + 2, /* title lines */
    width: area.width.saturating_sub(2),
    height: visible as u16,
  }
  .intersection(area);
  if rows_rect.height > 0 {
    let _ = scrollable_body_area(f, rows_rect, start as u16, total, &app.theme);
  }
}

/// Render the clean reclaim overlay (issue #325). A centred modal showing
/// the gated reclaim report for the selected worktree (per-artifact sizes +
/// total), the `[clean.profiles.*]` picker when configured, the gate-
/// preserved names, and a danger-coloured armed indicator while the safety
/// countdown runs. The live countdown progresses on the status bar; the
/// border switches to the danger colour once armed.
fn draw_clean_overlay(f: &mut Frame, app: &App) {
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let danger = app.theme.prunable;
  let armed = app.clean_overlay.confirm.is_armed();
  let border = if armed { danger } else { accent };
  let term = f.area();
  let width = overlay_modal_width(term.width);
  let inner = width.saturating_sub(6) as usize; // inside borders (1) + overlay_block padding (2) each side

  let mut lines = overlay_title_lines("Reclaim build artifacts", border);

  // Profile picker — the `(default)` choice plus any `[clean.profiles]`.
  // Full-width, scrollable; only rendered when named profiles exist.
  if app.clean_overlay.has_profiles() {
    let labels = app.clean_overlay.choice_labels();
    let max_visible = (term.height as usize).saturating_sub(14).max(3);
    lines.extend(picker_lines(
      &labels,
      app.clean_overlay.selected_index(),
      max_visible,
      inner,
      &app.theme,
    ));
    lines.push(Line::from(""));
  }

  // The gated reclaim report — only the git-ignored, untracked artifacts.
  // Each row fills the modal's inner width: a matched nerd-font icon (#334) +
  // dir name pinned left, the heatmap-coloured size pinned to the right edge,
  // so the columns use the whole box. Capped to the modal height with a
  // `… N more` overflow marker.
  match app.clean_overlay.reclaim() {
    Some(reclaim) if !reclaim.artifacts.is_empty() => {
      // Name column = inner width minus the ` icon  ` gutter (4) and the
      // `<size> ` tail (11), so the size lands flush on the right edge.
      let namew = inner.saturating_sub(15).max(5);
      let row = |icon: &str, left: &str, left_style: Style, bytes: u64, size_style: Style| -> Line<'static> {
        Line::from(vec![
          Span::styled(format!(" {icon}  "), Style::default().fg(accent)),
          Span::styled(format!("{:<namew$}", ellipsize_middle(left, namew)), left_style),
          Span::styled(format!("{:>10} ", crate::clean::human_size(bytes)), size_style),
        ])
      };
      let max_rows = (term.height as usize).saturating_sub(14).max(3);
      let shown = reclaim.artifacts.len().min(max_rows);
      for a in reclaim.artifacts.iter().take(shown) {
        lines.push(row(
          clean_dir_icon(&a.rel),
          &a.rel,
          Style::default().fg(muted),
          a.bytes,
          Style::default().fg(reclaim_size_color(a.bytes, &app.theme)),
        ));
      }
      if reclaim.artifacts.len() > shown {
        lines.push(
          Line::from(Span::styled(
            format!("… {} more", reclaim.artifacts.len() - shown),
            Style::default().fg(muted),
          ))
          .centered(),
        );
      }
      // The total row uses an aggregate (sigma) glyph in the icon column.
      lines.push(row(
        "\u{f03a}",
        "total",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
        reclaim.total_bytes,
        Style::default()
          .fg(reclaim_size_color(reclaim.total_bytes, &app.theme))
          .add_modifier(Modifier::BOLD),
      ));
    }
    _ => {
      lines.push(
        Line::from("nothing to reclaim")
          .style(Style::default().fg(muted))
          .centered(),
      );
    }
  }

  // Gate-preserved names — explain why a visible `target/` was not counted.
  for rel in app.clean_overlay.skipped() {
    lines.push(
      Line::from(format!("skipped {rel} — not git-ignored / holds tracked files"))
        .style(Style::default().fg(muted))
        .centered(),
    );
  }

  // Danger-coloured armed indicator; the live countdown shows on the status
  // bar (set by `clean_confirm_press`), so the render stays time-free.
  if armed {
    lines.push(Line::from(""));
    lines.push(
      Line::from("⚠ armed — confirm again or cancel to abort")
        .style(Style::default().fg(danger).add_modifier(Modifier::BOLD))
        .centered(),
    );
  }

  push_modal_hint(
    &mut lines,
    HintContext::Clean,
    &app.keymap,
    &app.modal_keymap,
    &app.theme,
  );
  let height = lines.len() as u16 + 2 /* border */ + 2 /* padding */;
  let area = centered_abs(width, height, term);
  f.render_widget(Clear, area);
  f.render_widget(Paragraph::new(lines).block(overlay_block(border)), area);
}

/// Render the command palette overlay (issue #32).
///
/// Layout: a centered modal sized at 60% × 50% of the frame
/// (matches the `centered(60, 50, …)` call below). Matches list
/// occupies the top of the inner area, input bar is pinned to the
/// bottom row. The highlight follows the user's cycle key
/// (`Up` / `Down` / `Tab`); `Enter` fires the highlighted entry,
/// Worktree-rename modal (#290). Mirrors [`draw_create`] — it reuses the
/// same Create form state (Type / Issue / Desc) pre-filled from the current
/// branch — plus a `From :` line showing the original branch, an async
/// "renaming…" loader, and an inline failure surfaced from
/// `App::edit_failure`. State lives on `App::create_form` +
/// `App::edit_original_branch`.
fn draw_edit_worktree(f: &mut Frame, app: &App) {
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let clean = app.theme.clean;
  let surface = app.theme.selection_bg;

  let (type_str, type_desc) = app
    .branch_types
    .get(app.create_form.type_index)
    .map(|t| (t.name.as_str(), t.description.as_str()))
    .unwrap_or(("", "(no branch types configured)"));

  let block = overlay_block(clean);
  let term = f.area();
  let outer = centered_box(70, 72, 1, term);
  let inner_w = block.inner(outer).width as usize;
  let label_w = 5usize;
  let gutter = 2 + label_w + 2;
  let value_w = inner_w.saturating_sub(gutter);

  let label = |s: &str| format!("{:<label_w$}", s);
  let old_branch = app
    .edit_original_branch
    .as_deref()
    .or_else(|| app.selected().and_then(|w| w.branch.as_deref()))
    .unwrap_or("(none)");
  let old_display = ellipsize_middle(old_branch, inner_w.saturating_sub("  From   : ".len()));
  let (branch_raw, dir_raw) = pattern_preview(app, type_str);
  let branch = ellipsize_middle(&branch_raw, inner_w.saturating_sub("  Branch : ".len()));
  let dirname = ellipsize_middle(&dir_raw, inner_w.saturating_sub("  Dir    : ".len()));

  let freeform = app.create_form.mode == Mode::Freeform;
  let mut lines = overlay_title_lines("Rename Worktree", clean);
  lines.push(Line::from(vec![
    Span::raw("  From   : "),
    Span::styled(old_display, Style::default().fg(muted)),
  ]));
  lines.push(Line::from(String::new()));
  lines.push(Line::from(vec![
    Span::raw("  Branch : "),
    Span::styled(branch, Style::default().fg(app.theme.branch)),
  ]));
  lines.push(Line::from(vec![
    Span::raw("  Dir    : "),
    Span::styled(dirname, Style::default().fg(app.theme.dirty)),
  ]));
  if let Some(warning) = rename_pr_warning(app, &branch_raw, old_branch) {
    lines.push(Line::from(vec![
      Span::raw("  "),
      Span::styled(
        ellipsize_middle(&warning, inner_w.saturating_sub(2)),
        Style::default().fg(app.theme.prunable),
      ),
    ]));
  }
  lines.push(Line::from(String::new()));
  // Issue #479: free-form has one field and no type selector, so the rows the
  // structured triple needs are not rendered rather than rendered inert — the
  // same shape `draw_create` uses, and the reason #474 suppressed the toggle
  // here in the first place was that these rows did not exist. Which rows the
  // structured side needs comes from the patterns (#418).
  if freeform {
    lines.push(field_input_line(
      &label("Name"),
      &app.create_form.name,
      app.create_form.field == Field::Name,
      value_w,
      accent,
      muted,
      surface,
    ));
  } else {
    lines.extend(form_field_lines(app, type_str, type_desc, value_w, label_w));
  }

  let height = lines.len() as u16 + 4 + 2 /* border */ + 2 /* vertical padding */;
  let area = centered_box(70, 72, height, term);
  let inner = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Min(1),    // title + form fields
      Constraint::Length(1), // loader / failure
      Constraint::Length(1), // buttons
      Constraint::Length(1), // hint gap
      Constraint::Length(1), // hint
    ])
    .split(block.inner(area));

  f.render_widget(Clear, area);
  f.render_widget(block, area);
  f.render_widget(Paragraph::new(lines), inner[0]);

  if app.is_edit_worktree_loading() {
    f.render_widget(
      LoaderWidget::running(
        app.spinner.glyph(DOT_FRAMES),
        TaskKind::EditWorktree.loading_label(),
        None,
        &app.theme,
      )
      .alignment(Alignment::Center),
      inner[1],
    );
  } else if let Some(error) = app.edit_failure.as_deref() {
    f.render_widget(
      LoaderWidget::failed("rename failed", Some(error), &app.theme).alignment(Alignment::Center),
      inner[1],
    );
  }

  if !app.is_edit_worktree_loading() {
    f.render_widget(
      Paragraph::new(rename_buttons_line(accent, muted)).alignment(Alignment::Center),
      inner[2],
    );
    f.render_widget(
      Paragraph::new(modal_hint_for_context_with_fields(
        // One source with the statusbar (`App::rename_hint_context`), so the
        // footer and the bar behind it cannot disagree about the mode.
        app.rename_hint_context(),
        &app.keymap,
        &app.modal_keymap,
        &app.theme,
        app.create_form.fields(),
      )),
      inner[4],
    );
  }
}

fn draw_command_palette(f: &mut Frame, app: &App) {
  let area = centered(60, 50, f.area());
  f.render_widget(Clear, area);

  let accent = app.theme.accent;
  let outer = overlay_block(accent);
  let inner = outer.inner(area);
  f.render_widget(outer, area);

  // Input-first layout (issue #262): a detached centred title, a blank
  // spacer, the `:` input field (background-filled, mirroring the New
  // Worktree modal's `field_input_line`), a spacer, the matches list (flex),
  // a hint gap, and the statusbar-style hint. The input moved to the top so
  // the modal reads input-then-results like the create form.
  let layout = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Length(1), // title
      Constraint::Length(1), // spacer
      Constraint::Length(1), // input field
      Constraint::Length(1), // spacer
      Constraint::Min(3),    // matches
      Constraint::Length(1), // hint gap
      Constraint::Length(1), // hint
    ])
    .split(inner);

  f.render_widget(
    Paragraph::new(
      Line::from(Span::styled(
        "Command Palette",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
      ))
      .centered(),
    ),
    layout[0],
  );

  // The `:` input field, styled like the create modal's fields: a `:` label
  // then a background-filled value box. The palette input is always focused
  // (the user is typing into it), so it carries the accent fill + cursor.
  let label = ":";
  let gutter = 2 + label.chars().count() + 2; // field_input_line's `  label  ` gutter
  let value_w = (inner.width as usize).saturating_sub(gutter);
  f.render_widget(
    Paragraph::new(field_input_line(
      label,
      app.palette.buffer(),
      true,
      value_w,
      accent,
      app.theme.muted,
      app.theme.selection_bg,
    )),
    layout[2],
  );

  let entries = app.palette.matches();
  let highlight = app.palette.highlight();
  let mut lines: Vec<Line<'_>> = entries
    .iter()
    .enumerate()
    .map(|(i, entry)| {
      let prefix = if i == highlight { "▶ " } else { "  " };
      let name_style = if i == highlight {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
      } else {
        palette_name_style(&app.theme)
      };
      Line::from(vec![
        Span::raw(prefix),
        Span::styled(format!("{:<22}", entry.name), name_style),
        Span::raw("  "),
        Span::styled(entry.description, Style::default().fg(app.theme.muted)),
      ])
    })
    .collect();
  if lines.is_empty() {
    lines.push(Line::from(Span::styled(
      "  (no matching command — backspace to broaden)",
      Style::default().fg(app.theme.prunable),
    )));
  }
  f.render_widget(Paragraph::new(lines), layout[4]);
  f.render_widget(
    Paragraph::new(modal_hint_for_context(
      HintContext::CommandPalette,
      &app.keymap,
      &app.modal_keymap,
      &app.theme,
    )),
    layout[6],
  );
}

/// Body of the Issue / PR sidebar block. The block title (`" Issue / PR "`)
/// is supplied by [`draw_sidebar`] via the surrounding `Block`, so this
/// function only returns the content rows. `max_width` is the inner
/// width of the Issue / PR block (chunk width minus 2 borders and the
/// 1-char left padding applied by [`render_section`]); summary lines
/// trim their variable parts so total visible width ≤ `max_width`.
pub fn github_status_lines(app: &App, max_width: usize) -> Vec<Line<'static>> {
  let link = app.current_link();
  let mut lines: Vec<Line<'static>> = Vec::new();

  if link.issue.is_none() && link.pr.is_none() {
    // Derive LinkPrompt's chord from the live keymap so the hint tracks the
    // binding (and any `[tui.keys]` override) instead of drifting — the `L`
    // hardcoded pre-#290 now belongs to LazyGitFullscreen.
    let chord = action_chord(&app.keymap, Action::LinkPrompt, "i");
    lines.push(Line::from(Span::styled(
      trunc(&format!("no link · press {chord} to link"), max_width),
      Style::default().fg(app.theme.muted),
    )));
    return lines;
  }

  if let Some(n) = link.issue {
    let spinner = app.spinner.glyph(DOT_FRAMES);
    lines.push(issue_summary_line_with_spinner(
      n,
      link.issue_source,
      app.issue_fetch_state(),
      PersistedSummary {
        title: link.issue_title.as_deref(),
        state: link.issue_state,
      },
      max_width,
      &app.theme,
      Some(spinner),
    ));
  }
  if let Some(n) = link.pr {
    let spinner = app.spinner.glyph(DOT_FRAMES);
    // #436: advertise the key that opens the CI checks overlay right after
    // the indicator, resolved live so a rebind shows through. The key is
    // context-accurate (Codex review #455): the contextual `c`
    // (EditWorktree's chord) only while the status pane holds the focus —
    // in the worktrees context that key opens the rename modal, so the
    // global `ci_checks` binding is advertised instead. An unbound
    // EditWorktree falls back to the global binding (still live in that
    // context); only when both are unbound does the suffix disappear. In
    // picker mode (`gwm switch`) run_action drops Action::CiChecks —
    // printable keys feed the filter — so no key is advertised at all.
    let ci_key = if app.picker_mode {
      None
    } else if app.sidebar.open && app.sidebar.focused {
      app
        .keymap
        .primary_chord(Action::EditWorktree)
        .or_else(|| app.keymap.primary_chord(Action::CiChecks))
    } else {
      app.keymap.primary_chord(Action::CiChecks)
    };
    lines.push(pr_summary_line_with_spinner(
      n,
      link.pr_source,
      app.pr_fetch_state(),
      PersistedSummary {
        title: link.pr_title.as_deref(),
        state: link.pr_state,
      },
      max_width,
      &app.theme,
      Some(spinner),
      ci_key.as_deref(),
    ));
  }
  lines
}

/// Nerdfont glyph leading the pane's Issue line (issue #283):
/// `nf-oct-issue_opened`.
pub const ISSUE_ICON: &str = "\u{f41b}";
/// Nerdfont glyph leading the pane's PR line (issue #283):
/// `nf-oct-git_pull_request`.
pub const PR_ICON: &str = "\u{f407}";

/// Nerdfont glyphs for the per-PR CI indicator (issue #299):
/// `nf-oct-check` (passing), `nf-oct-x` (failing), `nf-oct-sync` (running).
pub const CI_PASSING_ICON: &str = "\u{f42e}";
pub const CI_FAILING_ICON: &str = "\u{f467}";
pub const CI_RUNNING_ICON: &str = "\u{f46a}";
/// A check whose state this build does not recognise (issue #419) — most
/// likely a GitLab pipeline status added upstream. Rendered distinctly so
/// it never passes for green, and never for a running check with a clock.
pub const CI_UNKNOWN_ICON: &str = "\u{f059}";

/// The pane's source chip (issue #283): `auto` for a branch-name inference,
/// `detected` for a live GitHub match. Explicit / none carry no chip — the
/// number already speaks for an explicit link. Rendered version-badge style
/// (reverse-video [`chip_style`]); `auto` stays muted, `detected` takes the
/// accent so a freshly-found PR draws the eye.
fn source_chip(s: LinkSource, theme: &Theme) -> Option<(&'static str, Color)> {
  match s {
    LinkSource::BranchName => Some(("auto", theme.muted)),
    LinkSource::Detected => Some(("detected", theme.accent)),
    LinkSource::Explicit | LinkSource::None => None,
  }
}

/// Collapse a multi-span line into a single truncated plain span when the
/// styled spans together overflow `max_width`. Used by the pane's narrow
/// fallback so the icon + chips never push the line past the block border
/// (issue #283). Width is counted in display columns (`chars().count()`),
/// matching the budget arithmetic in [`summary_line`].
fn flatten_if_overflow(spans: &mut Vec<Span<'static>>, max_width: usize) {
  let w: usize = spans.iter().map(|s| s.content.chars().count()).sum();
  if w > max_width {
    let raw: String = spans.iter().map(|s| s.content.as_ref()).collect();
    *spans = vec![Span::raw(trunc(&raw, max_width))];
  }
}

/// Render the Loaded / Idle / Loading / Error variants for an issue link
/// row in the sidebar. `max_width` is the number of columns the line is
/// allowed to occupy (sidebar inner width minus padding); the variable
/// part (title or error blob) is trimmed so the total visible width
/// stays ≤ `max_width`. Fixed elements (head, badge) are preserved.
pub fn issue_summary_line(
  n: u64,
  src: LinkSource,
  state: &GitHubFetchState<crate::github::IssueStatus>,
  max_width: usize,
  theme: &Theme,
) -> Line<'static> {
  issue_summary_line_with_spinner(n, src, state, PersistedSummary::none(), max_width, theme, None)
}

#[derive(Clone, Copy)]
struct PersistedSummary<'a, S> {
  title: Option<&'a str>,
  state: Option<S>,
}

impl<S> PersistedSummary<'_, S> {
  fn none() -> Self {
    Self {
      title: None,
      state: None,
    }
  }
}

/// Resolved render inputs for a GitHub summary line, after the caller has
/// collapsed the issue/PR-specific `match` into the shared shape. The
/// `Loaded` arm carries the already-picked badge label + colour and an
/// optional `trailing` segment (issue: empty; PR: ` · checks N/M`) placed
/// between the closing `]` and the final space+title.
enum SummaryState<'a> {
  Idle,
  CachedTitle {
    title: &'a str,
  },
  CachedStatus {
    badge: &'a str,
    badge_color: Color,
    trailing: String,
    /// Colour for the `trailing` segment, e.g. the CI indicator
    /// (issue #299). `None` paints it with the default foreground.
    trailing_color: Option<Color>,
    title: &'a str,
  },
  Loading,
  Loaded {
    badge: &'a str,
    badge_color: Color,
    trailing: String,
    /// See [`SummaryState::CachedStatus::trailing_color`].
    trailing_color: Option<Color>,
    title: &'a str,
  },
  Error(&'a str),
}

/// Shared renderer behind [`issue_summary_line`] and [`pr_summary_line`]
/// (issue #283). Both twins pass their leading nerdfont `icon`, their `head`
/// identity ("Issue #…" / "PR    #…"), the link `source`, and — for `Loaded`
/// — a state `badge` + optional `trailing` segment. Every line leads with
/// `<icon> <head>`, then an optional version-badge-style source chip
/// (`auto` / `detected`), then the state-specific tail.
///
/// `trailing` keeps the two twins identical past the badge: issue passes ""
/// → renders ` badge  title`; PR passes ` · checks 1/2` → renders
/// ` badge · checks 1/2 title`. Widths are counted in display columns (the
/// `·` is U+00B7: 1 column) so the budget arithmetic holds.
/// Render a `trailing` segment, styling it with `color` when present
/// (the CI indicator, issue #299) and falling back to the default
/// foreground otherwise (the legacy uncoloured `· checks N/M`).
fn trailing_span(trailing: String, color: Option<Color>) -> Span<'static> {
  match color {
    Some(c) => Span::styled(trailing, Style::default().fg(c)),
    None => Span::raw(trailing),
  }
}

fn summary_line(
  icon: &str,
  head: String,
  source: LinkSource,
  state: SummaryState,
  max_width: usize,
  theme: &Theme,
  spinner: Option<&str>,
) -> Line<'static> {
  // `<icon> <head>` plus an optional source chip are common to every state.
  // `prefix_w` tracks the visible width so the variable tail (title / error
  // blob) can be trimmed to fit `max_width`.
  let icon_seg = format!("{}  ", icon); // glyph + two trailing gaps
  let chip = source_chip(source, theme);
  // Source chip segment = " " + " <label> " (a leading gap + the padded chip).
  let source_seg_w = chip.map(|(l, _)| 1 + l.chars().count() + 2).unwrap_or(0);
  let prefix_w = icon_seg.chars().count() + head.chars().count() + source_seg_w;

  // The `head` carries no status signal, only identity — it paints with the
  // `name` role (issue #210); the icon mirrors loaded state colour and falls
  // back to muted while no fresh status exists.
  let icon_color = match &state {
    SummaryState::CachedStatus { badge_color, .. } | SummaryState::Loaded { badge_color, .. } => *badge_color,
    SummaryState::Idle | SummaryState::CachedTitle { .. } | SummaryState::Loading | SummaryState::Error(_) => {
      theme.muted
    }
  };
  let build_prefix = |head_bold: bool| -> Vec<Span<'static>> {
    let head_style = if head_bold {
      Style::default().fg(theme.name).add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(theme.name)
    };
    let mut spans = vec![
      Span::styled(icon_seg.clone(), Style::default().fg(icon_color)),
      Span::styled(head.clone(), head_style),
    ];
    if let Some((label, color)) = chip {
      spans.push(Span::raw(" "));
      spans.push(Span::styled(format!(" {} ", label), chip_style(color)));
    }
    spans
  };

  match state {
    SummaryState::Idle => {
      let mut spans = build_prefix(false);
      flatten_if_overflow(&mut spans, max_width);
      Line::from(spans)
    }
    SummaryState::CachedTitle { title } => {
      let fixed = prefix_w + 1;
      let budget = max_width.saturating_sub(fixed);
      let mut spans = build_prefix(false);
      spans.push(Span::raw(" "));
      spans.push(Span::raw(trunc(title, budget)));
      flatten_if_overflow(&mut spans, max_width);
      Line::from(spans)
    }
    SummaryState::CachedStatus {
      badge,
      badge_color,
      trailing,
      trailing_color,
      title,
    } => {
      let badge_seg_w = 1 + badge.chars().count() + 2;
      let fixed = prefix_w + badge_seg_w + trailing.chars().count() + 1;
      if fixed >= max_width {
        let mut spans = build_prefix(true);
        spans.push(Span::raw(" "));
        spans.push(Span::raw(format!(" {} ", badge)));
        spans.push(trailing_span(trailing, trailing_color));
        flatten_if_overflow(&mut spans, max_width);
        return Line::from(spans);
      }
      let budget = max_width - fixed;
      let mut spans = build_prefix(true);
      spans.push(Span::raw(" "));
      spans.push(Span::styled(format!(" {} ", badge), chip_style(badge_color)));
      spans.push(trailing_span(trailing, trailing_color));
      spans.push(Span::raw(" "));
      spans.push(Span::raw(trunc(title, budget)));
      Line::from(spans)
    }
    SummaryState::Loading => {
      let glyph = spinner.unwrap_or("…");
      let mut spans = build_prefix(false);
      spans.push(Span::raw(format!(" {} loading", glyph)));
      flatten_if_overflow(&mut spans, max_width);
      Line::from(spans)
    }
    SummaryState::Loaded {
      badge,
      badge_color,
      trailing,
      trailing_color,
      title,
    } => {
      // Tail fixed cost past the prefix = " " + " <badge> " + trailing + " ".
      let badge_seg_w = 1 + badge.chars().count() + 2;
      let fixed = prefix_w + badge_seg_w + trailing.chars().count() + 1;
      if fixed >= max_width {
        // Very narrow pane: keep the prefix + badge, drop the title, and
        // flatten to fit rather than overflow the block border.
        let mut spans = build_prefix(true);
        spans.push(Span::raw(" "));
        spans.push(Span::raw(format!(" {} ", badge)));
        spans.push(trailing_span(trailing, trailing_color));
        flatten_if_overflow(&mut spans, max_width);
        return Line::from(spans);
      }
      let budget = max_width - fixed;
      let mut spans = build_prefix(true);
      spans.push(Span::raw(" "));
      spans.push(Span::styled(format!(" {} ", badge), chip_style(badge_color)));
      spans.push(trailing_span(trailing, trailing_color));
      spans.push(Span::raw(" "));
      spans.push(Span::raw(trunc(title, budget)));
      Line::from(spans)
    }
    SummaryState::Error(e) => {
      let fixed = prefix_w + 2; // " " + "!"
      let budget = max_width.saturating_sub(fixed);
      let mut spans = build_prefix(false);
      spans.push(Span::raw(" "));
      spans.push(Span::styled(
        format!("!{}", trunc(e, budget)),
        Style::default().fg(theme.prunable),
      ));
      flatten_if_overflow(&mut spans, max_width);
      Line::from(spans)
    }
  }
}

fn issue_summary_line_with_spinner(
  n: u64,
  src: LinkSource,
  state: &GitHubFetchState<crate::github::IssueStatus>,
  persisted: PersistedSummary<'_, IssueState>,
  max_width: usize,
  theme: &Theme,
  spinner: Option<&str>,
) -> Line<'static> {
  let head = format!("Issue #{}", n);
  let resolved = match state {
    GitHubFetchState::Idle => match persisted.state {
      Some(state) => {
        let badge = match state {
          IssueState::Open => "open",
          IssueState::Closed => "closed",
        };
        SummaryState::CachedStatus {
          badge,
          badge_color: issue_badge_color(state, theme),
          trailing: String::new(),
          trailing_color: None,
          title: persisted.title.unwrap_or(""),
        }
      }
      None => persisted
        .title
        .map(|title| SummaryState::CachedTitle { title })
        .unwrap_or(SummaryState::Idle),
    },
    GitHubFetchState::Loading => match persisted.state {
      Some(state) => {
        let badge = match state {
          IssueState::Open => "open",
          IssueState::Closed => "closed",
        };
        SummaryState::CachedStatus {
          badge,
          badge_color: issue_badge_color(state, theme),
          trailing: format!(" · {} loading", spinner.unwrap_or("…")),
          trailing_color: None,
          title: persisted.title.unwrap_or(""),
        }
      }
      None => SummaryState::Loading,
    },
    GitHubFetchState::Loaded(s) => {
      // Mirror `issue_badge_color` exactly so the summary line and the
      // sidebar header dot never disagree for the same issue: closed maps
      // to `locked` ("moved on"), not `prunable` ("alarming"). Pre-#170
      // this site hard-coded `Color::Red` while the dot used `Magenta` —
      // a latent inconsistency the audit closes (Copilot review #209).
      let badge = match s.state {
        IssueState::Open => "open",
        IssueState::Closed => "closed",
      };
      SummaryState::Loaded {
        badge,
        badge_color: issue_badge_color(s.state, theme),
        trailing: String::new(),
        trailing_color: None,
        title: &s.title,
      }
    }
    GitHubFetchState::Error(e) => SummaryState::Error(e),
  };
  summary_line(ISSUE_ICON, head, src, resolved, max_width, theme, spinner)
}

/// Render the Loaded / Idle / Loading / Error variants for a PR link
/// row in the sidebar. See [`issue_summary_line`] for the `max_width`
/// contract — same idea, with a coloured CI indicator ([`ci_indicator`],
/// issue #299) squeezed in between badge and title when the rollup is
/// non-empty.
pub fn pr_summary_line(
  n: u64,
  src: LinkSource,
  state: &GitHubFetchState<crate::github::PrStatus>,
  max_width: usize,
  theme: &Theme,
  ci_hint: Option<&str>,
) -> Line<'static> {
  pr_summary_line_with_spinner(n, src, state, PersistedSummary::none(), max_width, theme, None, ci_hint)
}

#[allow(clippy::too_many_arguments)] // one arg past the limit; splitting a builder for a single call site is worse
fn pr_summary_line_with_spinner(
  n: u64,
  src: LinkSource,
  state: &GitHubFetchState<crate::github::PrStatus>,
  persisted: PersistedSummary<'_, PrState>,
  max_width: usize,
  theme: &Theme,
  spinner: Option<&str>,
  ci_hint: Option<&str>,
) -> Line<'static> {
  let head = format!("PR    #{}", n);
  let resolved = match state {
    GitHubFetchState::Idle => match persisted.state {
      Some(state) => {
        let badge = match state {
          PrState::Open => "open",
          PrState::Draft => "draft",
          PrState::Closed => "closed",
          PrState::Merged => "merged",
        };
        SummaryState::CachedStatus {
          badge,
          badge_color: pr_badge_color(state, theme),
          trailing: String::new(),
          trailing_color: None,
          title: persisted.title.unwrap_or(""),
        }
      }
      None => persisted
        .title
        .map(|title| SummaryState::CachedTitle { title })
        .unwrap_or(SummaryState::Idle),
    },
    GitHubFetchState::Loading => match persisted.state {
      Some(state) => {
        let badge = match state {
          PrState::Open => "open",
          PrState::Draft => "draft",
          PrState::Closed => "closed",
          PrState::Merged => "merged",
        };
        SummaryState::CachedStatus {
          badge,
          badge_color: pr_badge_color(state, theme),
          trailing: format!(" · {} loading", spinner.unwrap_or("…")),
          trailing_color: None,
          title: persisted.title.unwrap_or(""),
        }
      }
      None => SummaryState::Loading,
    },
    GitHubFetchState::Loaded(s) => {
      // Route the badge colour through `pr_badge_color` (mirroring how the
      // issue side calls `issue_badge_color`) so the summary line and the
      // sidebar header dot never disagree for the same PR. Only the label
      // stays inline. Pre-#239 this site duplicated the colour map (Copilot
      // review #209).
      let badge = match s.state {
        PrState::Open => "open",
        PrState::Draft => "draft",
        PrState::Closed => "closed",
        PrState::Merged => "merged",
      };
      // Issue #299: surface the derived CI state (icon + label + N/M, coloured)
      // instead of the bare ` · checks N/M`, so pass / fail / running reads at a
      // glance. `ci_indicator` returns `None` when the PR has no checks.
      // #436 validation feedback: the indicator ends with the resolved key
      // that opens the CI checks overlay, mirroring the pane titles' `[F]`.
      let (trailing, trailing_color) = match ci_indicator(s.ci, s.checks_passed, s.checks_total, theme) {
        Some((text, color)) => (
          match ci_hint {
            Some(key) => format!("{text} [{key}]"),
            None => text,
          },
          Some(color),
        ),
        None => (String::new(), None),
      };
      SummaryState::Loaded {
        badge,
        badge_color: pr_badge_color(s.state, theme),
        trailing,
        trailing_color,
        title: &s.title,
      }
    }
    GitHubFetchState::Error(e) => SummaryState::Error(e),
  };
  summary_line(PR_ICON, head, src, resolved, max_width, theme, spinner)
}

// ---- Issue #73: lazygit-style colour helpers -------------------------------
// Pure functions exposed at the crate boundary so the table-driven tests
// in `tests/tui_app_tests.rs` can pin the visual contract without spinning
// up a real terminal. Anything that takes `BranchStatus` / `PrState` /
// `IssueState` / a `Duration` and returns a `Color` belongs here.

/// Pick a colour for a branch name based on its `BranchStatus`. Worst
/// signal wins so the most actionable state stays visible at a glance.
/// Priority (top down): `unknown` → `dirty` → `ahead/behind` → no
/// upstream → synced/clean. Mirrors lazygit's branches view scheme
/// (`pkg/gui/presentation/branches.go::getBranchDisplayStrings`) with
/// one local addition: `dirty` lands on red because for a worktree
/// manager the most actionable "do something" signal is uncommitted
/// work.
pub fn branch_name_color(s: &BranchStatus, theme: &Theme) -> Color {
  if s.unknown {
    return theme.muted;
  }
  if s.is_dirty {
    return theme.prunable;
  }
  if s.ahead > 0 || s.behind > 0 {
    return theme.dirty;
  }
  if !s.has_upstream {
    // Lazygit's `?` marker — branch never pushed yet. Distinct from
    // synced so the user knows whether they need to run `git push`.
    return theme.locked;
  }
  theme.branch
}

/// Map a branch age to a freshness colour: green < 7d, yellow < 30d,
/// darkgray otherwise. Cutoffs are wide on purpose — a 6-day branch
/// is "fresh", a 4-week one is "ageing", a 5-week one is "stale" —
/// so the colour shift registers as signal rather than noise.
pub fn freshness_color(age: Duration, theme: &Theme) -> Color {
  const WEEK: u64 = 7 * 86_400;
  const MONTH: u64 = 30 * 86_400;
  let s = age.as_secs();
  if s < WEEK {
    theme.clean
  } else if s < MONTH {
    theme.dirty
  } else {
    theme.muted
  }
}

/// Pick a colour for the PR-status dot rendered in the sidebar header.
/// Ports the lazygit `WithPrColor` palette (open=green, draft=gray,
/// merged=magenta, closed=red) but uses 16-colour names instead of
/// hex RGB so the badge respects the user's terminal theme.
pub fn pr_badge_color(state: PrState, theme: &Theme) -> Color {
  match state {
    PrState::Open => theme.clean,
    PrState::Draft => theme.muted,
    PrState::Merged => theme.locked,
    PrState::Closed => theme.prunable,
  }
}

/// Build the CI indicator segment for a loaded PR (issue #299): a nerd-font
/// glyph + short label + `passed/total` count, plus the theme colour it
/// paints with. Returns `None` for [`CiState::None`] so a PR with no checks
/// renders nothing. Colours reuse the status-dot roles already used elsewhere
/// in the sidebar: passing → `clean` (green), failing → `prunable` (red),
/// running → `dirty` (yellow). The leading space keeps it flush against the
/// preceding badge, mirroring the old ` · checks N/M` trailing.
pub fn ci_indicator(ci: CiState, passed: u32, total: u32, theme: &Theme) -> Option<(String, Color)> {
  let (icon, label, color) = match ci {
    CiState::None => return None,
    CiState::Passing => (CI_PASSING_ICON, "passing", theme.clean),
    CiState::Failing => (CI_FAILING_ICON, "failing", theme.prunable),
    CiState::Running => (CI_RUNNING_ICON, "running", theme.dirty),
  };
  Some((format!(" {} CI {} {}/{}", icon, label, passed, total), color))
}

/// Same idea as [`pr_badge_color`] but for a linked issue. Closed maps
/// to magenta (treated as "moved on") rather than red so a routinely
/// resolved issue doesn't read as alarming.
pub fn issue_badge_color(state: IssueState, theme: &Theme) -> Color {
  match state {
    IssueState::Open => theme.clean,
    IssueState::Closed => theme.locked,
  }
}

/// Build the table's first-column marker (issue #283). The main worktree
/// keeps its single `★` (painted with the `main` role, preserving the
/// pre-#73 convention). Every other row renders two Issue/PR slots:
///
/// - left = **Issue** — `●` with the loaded issue-state colour when known,
///   `●` in `clean` green when only a link is known, else `-` in white.
/// - right = **PR** — `●` with the loaded PR-state colour when known, `●`
///   in `locked` violet when only a link is known, else `-` in white.
/// - a `muted` `/` separates them.
///
/// The table is normally the no-fetch read path. Once GitHub status has been
/// fetched for linked rows, their snapshots carry loaded states so
/// the Issue/PR pastilles can mirror open/closed/draft/merged without a
/// per-frame `gh` call. A detected PR shows here on every row only because it
/// is persisted to `gwm-pr-detected` (#283) and read back by
/// [`crate::github::read_link`].
pub fn table_marker(w: &WorktreeInfo, theme: &Theme) -> Line<'static> {
  if w.is_main {
    return Line::from(Span::styled("★", Style::default().fg(theme.main)));
  }
  // An empty slot stays `name`-white so "no link" reads as a neutral
  // placeholder rather than borrowing a status colour that would claim the
  // row. A linked slot takes its accent unless a live loaded state exists.
  let issue_color = match (w.link.issue, w.issue_state) {
    (Some(_), Some(state)) => issue_badge_color(state, theme),
    (Some(_), None) => theme.clean,
    (None, _) => theme.name,
  };
  let pr_color = match (w.link.pr, w.pr_state) {
    (Some(_), Some(state)) => pr_badge_color(state, theme),
    (Some(_), None) => theme.locked,
    (None, _) => theme.name,
  };
  Line::from(vec![
    Span::styled(
      if w.link.issue.is_some() { "●" } else { "-" },
      Style::default().fg(issue_color),
    ),
    Span::styled("/", Style::default().fg(theme.muted)),
    Span::styled(
      if w.link.pr.is_some() { "●" } else { "-" },
      Style::default().fg(pr_color),
    ),
  ])
}
