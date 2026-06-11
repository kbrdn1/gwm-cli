use super::app::{App, GitHubFetchState, LinkPromptStage, LinkTarget, View};
use super::keymap::{Action, Keymap};
use super::state::async_task::TaskKind;
use super::state::config_panel::{FieldKind, SettingField, SettingsTab};
use super::state::confirm::ConfirmButton;
use super::state::create_form::Field;
use super::state::pty_overlay::PtyKind;
use super::state::sidebar::SidebarMode;
use super::state::spinner::DOT_FRAMES;
use super::theme::Theme;
use crate::bootstrap::{BootstrapReport, StepStatus};
use crate::command_log::CommandStatus;
use crate::config::ConfigSource;
use crate::github::{IssueState, LinkSource, PrState};
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
    // #290: branch-rename inline modal renders over the list.
    View::Edit => draw_edit_worktree(f, app),
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
  let (table_pct, sidebar_pct) = match layout.split_percentages() {
    Some((t, s)) => (Constraint::Percentage(t), Constraint::Percentage(s)),
    None => {
      // Sidebar not rendered → no scrollable surface → no max scroll to track.
      app.sidebar.max_scroll = 0;
      draw_list(f, area, app);
      return;
    }
  };

  match layout {
    Resolved::Hidden => unreachable!("Hidden returns None from split_percentages, handled above"),
    Resolved::SideBySide { sidebar_left } => {
      let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if sidebar_left {
          [sidebar_pct, table_pct]
        } else {
          [table_pct, sidebar_pct]
        })
        .split(area);
      let (list_area, sidebar_area) = if sidebar_left {
        (split[1], split[0])
      } else {
        (split[0], split[1])
      };
      draw_list(f, list_area, app);
      draw_sidebar(f, sidebar_area, app);
    }
    Resolved::Stacked => {
      // Table on top, sidebar below — the default layout (issue #217) and the
      // narrow-terminal fallback. The left/right position does not apply to a
      // vertical stack.
      let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([table_pct, sidebar_pct])
        .split(area);
      draw_list(f, split[0], app);
      draw_sidebar(f, split[1], app);
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
    &app.repo_name,
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
) -> Line<'static> {
  let mut spans = vec![Span::raw(" [1] Worktrees ")];
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
pub fn status_pane_title() -> &'static str {
  " [2] Status "
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

  // Dynamic column widths derived from the visible subset so columns fit the
  // rows actually on screen. The path column is always last and absorbs the
  // remaining width.
  let name_w = column_width(visible.iter().map(|w| w.name.as_str()), 18, 38);
  let branch_w = column_width(visible.iter().map(|w| w.branch.as_deref().unwrap_or("-")), 18, 38);
  let status_w: u16 = 16;

  let header = Row::new(vec![
    // Age column lives at column 0 — recency-first, lazygit-style. No
    // caption; the glyphs (`2d`, `3w`, `1M`, `5y`, `-`) are self-evident
    // and a header would steal space from BRANCH on narrow terminals.
    Cell::from(""),
    Cell::from(""),
    Cell::from("NAME"),
    Cell::from("BRANCH"),
    Cell::from("STATUS"),
    Cell::from("PATH"),
  ])
  .style(Style::default().fg(theme.muted).add_modifier(Modifier::BOLD));

  let rows: Vec<Row> = visible
    .iter()
    .map(|w| build_row(w, name_w, branch_w, status_w, &theme))
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
  let widths = [
    Constraint::Length(4),
    Constraint::Length(3),
    Constraint::Min(name_w),
    Constraint::Min(branch_w),
    Constraint::Length(status_w),
    Constraint::Fill(1),
  ];

  let list_has_focus = !(app.sidebar.open && app.sidebar.focused);
  let border_color = panel_border_color(list_has_focus, &app.theme);

  let title = worktrees_pane_title(
    app.filter.query(),
    app.filter.active,
    visible.len(),
    app.worktrees.len(),
    app.theme.dirty,
  );

  // Bottom-right `selected of visible` counter (issue #217), mirroring the
  // Recent Commits footer. `list_state.selected()` is 0-based; render it
  // 1-based. Blank when nothing is visible so the footer disappears.
  let selected_1based = app.list_state.selected().map(|i| i + 1).unwrap_or(0);
  let counter = pane_counter(selected_1based, visible.len());

  let mut block = Block::default()
    .borders(Borders::ALL)
    .title(title)
    .border_style(Style::default().fg(border_color));
  if let Some(counter) = counter {
    block = block.title_bottom(Line::from(counter).right_aligned());
  }

  let table = Table::new(rows, widths)
    .header(header)
    .column_spacing(1)
    .block(block)
    .row_highlight_style(Style::default().bg(theme.selection_bg).add_modifier(Modifier::BOLD))
    .highlight_symbol("▶ ");

  f.render_stateful_widget(table, area, &mut app.list_state);
}

/// Details panel for the selected worktree — structured info, recent commits,
/// working-tree status, and a commands cheat-sheet (lazyssh-style layout).
///
/// Content is cached on `App` keyed by the selected worktree's path so the
/// underlying `git log` / `git status` only run when the selection changes
/// or `refresh()` invalidates the cache.
fn draw_sidebar(f: &mut Frame, area: Rect, app: &mut App) {
  let border_color = panel_border_color(app.sidebar.focused, &app.theme);
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
  let issue_pr_inner_width = area.width.saturating_sub(3) as usize;

  let Some(w) = app.selected().cloned() else {
    // Nothing selected: render the placeholder and bail. No cache to read,
    // so the borrow gymnastics below don't apply.
    let issue_pr_lines = github_status_lines(app, issue_pr_inner_width);
    let placeholder = [Line::from("(nothing selected)")];
    let h = |lines: usize| (lines as u16).saturating_add(2);
    let constraints = [
      Constraint::Length(h(placeholder.len())),
      Constraint::Length(h(issue_pr_lines.len())),
      Constraint::Length(0),
      Constraint::Min(3),
    ];
    let chunks = Layout::default()
      .direction(Direction::Vertical)
      .constraints(constraints)
      .split(area);
    app.sidebar.max_scroll = 0;
    app.sidebar.scroll = 0;
    render_section(
      f,
      chunks[0],
      status_pane_title(),
      SectionBody::new(&placeholder),
      border_color,
      0,
      None,
    );
    render_section(
      f,
      chunks[1],
      issue_pr_pane_title(&app.keymap),
      SectionBody::new(&issue_pr_lines),
      border_color,
      0,
      None,
    );
    render_section(
      f,
      chunks[3],
      recent_items_pane_title(active_mode, &app.keymap),
      SectionBody::new(&[]),
      border_color,
      0,
      None,
    );
    return;
  };

  // Populate (or refresh) the cache for the current selection. After this
  // short mutable borrow ends, `app.sidebar.cache` is guaranteed `Some`.
  let needs_refresh = match &app.sidebar.cache {
    Some(((p, m), _)) => *p != w.path || *m != active_mode,
    None => true,
  };
  if needs_refresh {
    // Committed diff of the branch vs its base trunk (issue #287). Resolved
    // through `config.doctor.trunks` so the figure matches the base
    // `gwm pr` would target; folded into the cached payload so the git
    // call only fires on a selection / mode change, not every frame.
    let diff = worktree::git_diff_stat_vs_base(&w.path, &app.config.doctor.trunks)
      .ok()
      .flatten();
    app.sidebar.cache = Some((
      (w.path.clone(), active_mode),
      build_sidebar_sections(&w, active_mode, diff, &theme),
    ));
  }

  // The live header line and the per-frame Issue / PR block are built BEFORE
  // the long cache borrow so they don't overlap it. The header is the only
  // line that is rebuilt fresh each frame (issue #73) — it's prefixed onto
  // the cached worktree section at render time instead of being spliced into
  // a cloned vec.
  let header_line = sidebar_header_line(&w, app);
  let issue_pr_lines = github_status_lines(app, issue_pr_inner_width);

  // Read the cached section lengths via a short immutable borrow so the
  // layout solver and scroll clamp can run before the render borrow. The
  // worktree section gains +1 row for the live header prefix.
  let (worktree_len, working_tree_len, working_tree_counts, commits_len) = {
    let cache = app.sidebar.cache.as_ref();
    let s = cache.map(|(_, s)| s);
    (
      s.map(|s| s.worktree.len()).unwrap_or(0) + 1,
      s.map(|s| s.working_tree.len()).unwrap_or(0),
      s.map(|s| s.working_tree_counts).unwrap_or_default(),
      s.map(|s| s.recent_commits.len()).unwrap_or(0) as u16,
    )
  };

  // Per-section block height = content rows + 2 border lines. Fixed
  // for the small sections (worktree / issue-PR / working-tree);
  // Recent Commits flexes to fill the rest of the sidebar height.
  // Issue #34: the Working Tree section is empty in `Stashes` mode
  // (no `git status --short` to render); collapse its constraint to
  // 0 so the empty titled block disappears instead of leaving a
  // bordered void.
  let h = |lines: usize| (lines as u16).saturating_add(2);
  let working_tree_height = if working_tree_len == 0 { 0 } else { h(working_tree_len) };
  let constraints = [
    Constraint::Length(h(worktree_len)),
    Constraint::Length(h(issue_pr_lines.len())),
    Constraint::Length(working_tree_height),
    Constraint::Min(3),
  ];
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints(constraints)
    .split(area);

  // Recent Commits is the only scrollable section. Clamp the scroll
  // offset to its visible area so `j` / `k` can't scroll past the end.
  // Done before the render borrow so no mutable `app` access overlaps it.
  let commits_area = chunks[3];
  let commits_visible = commits_area.height.saturating_sub(2);
  app.sidebar.max_scroll = commits_len.saturating_sub(commits_visible);
  if app.sidebar.scroll > app.sidebar.max_scroll {
    app.sidebar.scroll = app.sidebar.max_scroll;
  }
  let scroll = app.sidebar.scroll;

  // Issue #34: surface the active mode in the bottom-scrollable
  // panel title. The footer keeps the `i of N` counter; the bottom
  // hint switches to "Enter: copy stash@{N}" in stashes mode.
  let (panel_title, panel_footer) = match active_mode {
    super::state::sidebar::SidebarMode::Commits => {
      let title = recent_items_pane_title(active_mode, &app.keymap);
      let footer = if commits_len == 0 {
        None
      } else {
        let bottom = scroll.saturating_add(commits_visible).min(commits_len);
        Some(format!(" {} of {} ", bottom, commits_len))
      };
      (title, footer)
    }
    super::state::sidebar::SidebarMode::Stashes => {
      let title = recent_items_pane_title(active_mode, &app.keymap);
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
  let issue_pr_title = issue_pr_pane_title(&app.keymap);
  let working_tree_title = working_tree_pane_title(&app.keymap);
  // Working Tree footer (issue #287): colour-coded created / modified /
  // deleted counts. `None` in stashes mode (no section) and on a clean tree
  // (all-zero counts → `working_tree_counts_footer` returns `None`), so the
  // footer disappears instead of showing a bare ` 0 `.
  let working_tree_footer = if working_tree_len == 0 {
    None
  } else {
    working_tree_counts_footer(&working_tree_counts, &theme)
  };

  // The render borrow: cached sections are read by reference and never
  // cloned (issue #238). On a cache hit this copies zero commit text — the
  // up-to-300 `git log` lines stay put in `app.sidebar.cache`; `render_section`
  // only rebuilds the thin padded `Vec<Span>` per visible row, borrowing the
  // span content. `app` is only read immutably from here on (all mutation
  // already happened above), so this long borrow is conflict-free. The
  // `if let` is guaranteed to bind (the cache was populated above for the
  // selected worktree) — matching rather than `unwrap()` keeps the render
  // path panic-free per the house rules.
  if let Some((_, cache)) = app.sidebar.cache.as_ref() {
    render_section(
      f,
      chunks[0],
      status_pane_title(),
      SectionBody::with_prefix(&header_line, &cache.worktree),
      border_color,
      0,
      None,
    );
    render_section(
      f,
      chunks[1],
      issue_pr_title,
      SectionBody::new(&issue_pr_lines),
      border_color,
      0,
      None,
    );
    if !cache.working_tree.is_empty() {
      render_section(
        f,
        chunks[2],
        working_tree_title,
        SectionBody::new(&cache.working_tree),
        border_color,
        0,
        working_tree_footer,
      );
    }
    render_section(
      f,
      commits_area,
      panel_title,
      SectionBody::new(&cache.recent_commits),
      border_color,
      scroll,
      panel_footer.map(ratatui::text::Line::from),
    );
  }
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
  prefix: Option<&'a Line<'a>>,
  lines: &'a [Line<'a>],
}

impl<'a> SectionBody<'a> {
  /// Section body with no leading live line (Issue / PR, Working Tree,
  /// Recent Commits, and the `(nothing selected)` placeholder).
  fn new(lines: &'a [Line<'a>]) -> Self {
    Self { prefix: None, lines }
  }

  /// Section body whose first row is a per-frame live line — the worktree
  /// identity block, led by the `● <name>` status-dot header.
  fn with_prefix(prefix: &'a Line<'a>, lines: &'a [Line<'a>]) -> Self {
    Self {
      prefix: Some(prefix),
      lines,
    }
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
  border_color: Color,
  scroll: u16,
  footer: Option<ratatui::text::Line<'static>>,
) {
  let SectionBody { prefix, lines } = body;
  let mut block = Block::default()
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .title(title.into())
    .border_style(Style::default().fg(border_color));
  if let Some(f) = footer {
    block = block.title_bottom(f.right_aligned());
  }
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
  let padded: Vec<Line<'_>> = prefix.into_iter().chain(lines.iter()).map(pad).collect();
  // No `Wrap`: every section now relies on ratatui's view-level hard-clip,
  // matching lazygit's commits panel and ensuring 1 logical row = 1 visual
  // row (so the layout's `Constraint::Length` always matches what we draw).
  let paragraph = Paragraph::new(padded).block(block).scroll((scroll, 0));
  f.render_widget(paragraph, area);
}

/// Lazygit-style header line: `● <name>` where the dot's colour tracks
/// the linked PR / issue state. Rendered fresh every frame (not cached)
/// so the dot reflects the live fetch result without invalidating the
/// expensive git preview cache underneath.
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
    Ok(s) if s.trim().is_empty() => (
      vec![Line::from(Span::styled(
        "✓ clean".to_string(),
        Style::default().fg(theme.clean),
      ))],
      WorkingTreeCounts::default(),
    ),
    Ok(s) => {
      let counts = working_tree_status_counts(&s);
      let lines: Vec<Line<'static>> = s.lines().map(|line| working_tree_status_line(line, theme)).collect();
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

/// The single change-category a `git status --short` `XY` pair falls into
/// (issue #287). Shared by the Working-Tree footer counts and the per-row
/// colouring so a file's row colour always equals the footer segment it's
/// counted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WtCategory {
  Created,
  Modified,
  Deleted,
}

/// Classify a porcelain `XY` status pair into its dominant
/// [`WtCategory`], with a deterministic precedence (created > deleted >
/// modified) so each file maps to exactly one bucket:
///
/// - `??` (untracked) or an `A` in either column → **created**,
/// - else a `D` in either column → **deleted**,
/// - else anything changed (`M`, `R`, `C`, `T`, `U`, …) → **modified**.
fn working_tree_category(x: char, y: char) -> WtCategory {
  if (x == '?' && y == '?') || x == 'A' || y == 'A' {
    WtCategory::Created
  } else if x == 'D' || y == 'D' {
    WtCategory::Deleted
  } else {
    WtCategory::Modified
  }
}

/// Theme colour for a change category (issue #287): created → `untracked`
/// (green), modified → `modified` (yellow), deleted → `prunable` (red).
fn working_tree_category_color(cat: WtCategory, theme: &Theme) -> Color {
  match cat {
    WtCategory::Created => theme.untracked,
    WtCategory::Modified => theme.modified,
    WtCategory::Deleted => theme.prunable,
  }
}

/// Tally `git status --short` porcelain output into per-category
/// [`WorkingTreeCounts`] (issue #287) via [`working_tree_category`]. Lines
/// too short to carry an `XY` pair, or an all-blank pair, are skipped —
/// real porcelain output never produces them, but the helper is `pub` so a
/// non-git caller could.
pub fn working_tree_status_counts(status_short: &str) -> WorkingTreeCounts {
  let mut c = WorkingTreeCounts::default();
  for line in status_short.lines() {
    let mut chars = line.chars();
    let x = match chars.next() {
      Some(ch) => ch,
      None => continue,
    };
    let y = match chars.next() {
      Some(ch) => ch,
      None => continue,
    };
    if x == ' ' && y == ' ' {
      continue;
    }
    match working_tree_category(x, y) {
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

fn build_row(w: &WorktreeInfo, name_w: u16, branch_w: u16, status_w: u16, theme: &Theme) -> Row<'static> {
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
  let path_cell = Cell::from(w.path.to_string_lossy().to_string()).style(worktree_path_style(theme));

  Row::new(vec![
    age_cell,
    Cell::from(marker),
    name_cell,
    branch_cell,
    status_cell,
    path_cell,
  ])
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
  /// Resolve the displayed key from the keymap (honours `[tui.keys]`).
  Key(super::keymap::Action, &'static str),
  /// A fixed key + label for a non-rebindable contextual keystroke.
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
  /// Create-worktree form modal.
  Create,
  /// Confirm-delete modal.
  Confirm,
  /// Open issue/PR URL menu.
  OpenMenu,
  /// Two-stage issue/PR link prompt.
  LinkPrompt,
  /// Command palette overlay.
  CommandPalette,
  /// Bootstrap report overlay.
  Report,
  /// Keybindings help overlay.
  Help,
  /// PTY overlay (embedded lazygit / terminal). All keys pass through to the
  /// child process; Esc is the only gwm-level escape hatch.
  Pty,
  /// Branch-rename modal (`View::Edit`, #290).
  Rename,
}

impl HintContext {
  /// Short label rendered into the statusbar context chip and the help
  /// overlay subtitle.
  pub fn label(self) -> &'static str {
    match self {
      HintContext::Worktrees => "worktrees",
      HintContext::Status => "status",
      HintContext::Picker => "switch",
      HintContext::Create => "create",
      HintContext::Confirm => "confirm",
      HintContext::OpenMenu => "open",
      HintContext::LinkPrompt => "link",
      HintContext::CommandPalette => "command",
      HintContext::Report => "report",
      HintContext::Help => "help",
      HintContext::Pty => "terminal",
      HintContext::Rename => "rename",
    }
  }

  /// Static hint specs for this context. List-view contexts use rebindable
  /// [`Hint::Key`] verbs (resolved live by [`Self::resolve`]); modal /
  /// overlay contexts use [`Hint::Lit`] because their keys are hard-coded
  /// contextual escape hatches (Esc / Enter / digits), not keymap actions.
  fn hint_specs(self) -> &'static [Hint] {
    use super::keymap::Action::*;
    match self {
      HintContext::Worktrees => &[
        Hint::Key(Create, "new"),
        Hint::Key(DeleteConfirm, "del"),
        Hint::Key(Bootstrap, "boot"),
        Hint::Key(TerminalFullscreen, "open"),
        Hint::Key(YankPath, "yank"),
        Hint::Key(LazyGitFullscreen, "git"),
        Hint::Key(ReviewFullscreen, "review"),
        Hint::Key(FocusStatus, "status"),
        Hint::Key(CommandLogs, "logs"),
        Hint::Key(ConfigPanel, "settings"),
        Hint::Key(Filter, "filter"),
        Hint::Key(Help, "help"),
        Hint::Key(Quit, "quit"),
      ],
      HintContext::Status => &[
        Hint::Key(Down, "scroll"),
        Hint::Key(ToggleSidebarMode, "mode"),
        Hint::Key(CycleSidebarLayout, "layout"),
        Hint::Key(FetchGithub, "fetch"),
        Hint::Key(FocusWorktrees, "worktrees"),
        Hint::Key(CommandLogs, "logs"),
        Hint::Key(ConfigPanel, "settings"),
        Hint::Key(Filter, "filter"),
        Hint::Key(Help, "help"),
        Hint::Key(Quit, "quit"),
      ],
      HintContext::Picker => &[
        Hint::Lit("Enter", "select"),
        Hint::Lit("Esc", "cancel"),
        Hint::Key(TerminalFullscreen, "open"),
        Hint::Key(YankPath, "yank"),
        Hint::Key(LazyGitFullscreen, "git"),
        Hint::Key(Filter, "filter"),
        Hint::Key(Help, "help"),
        Hint::Key(Quit, "quit"),
      ],
      HintContext::Create => &[
        Hint::Lit("Tab", "field"),
        Hint::Lit("↑/↓", "type"),
        Hint::Lit("Enter", "submit"),
        Hint::Lit("Esc", "cancel"),
      ],
      HintContext::Confirm => &[
        Hint::Lit("y", "confirm"),
        Hint::Key(ToggleDeleteBranch, "branch"),
        Hint::Lit("←/→", "move"),
        Hint::Lit("Enter", "activate"),
        Hint::Lit("Esc", "cancel"),
      ],
      HintContext::OpenMenu => &[
        Hint::Lit("i", "issue"),
        Hint::Lit("p", "pr"),
        Hint::Key(FetchGithub, "fetch"),
        Hint::Lit("Esc", "close"),
      ],
      HintContext::LinkPrompt => &[
        Hint::Lit("j/k", "move"),
        Hint::Lit("i/p", "kind"),
        Hint::Lit("Enter", "link"),
        Hint::Key(FetchGithub, "fetch"),
        Hint::Lit("Esc", "cancel"),
      ],
      HintContext::CommandPalette => &[
        Hint::Lit("↑/↓", "move"),
        Hint::Lit("Enter", "run"),
        Hint::Lit("Esc", "cancel"),
      ],
      HintContext::Report => &[Hint::Lit("Enter/Esc", "close")],
      HintContext::Help => &[
        Hint::Lit("j/k", "scroll"),
        Hint::Lit("h/l", "pan"),
        Hint::Lit("Esc/q", "close"),
      ],
      HintContext::Pty => &[Hint::Lit("Esc", "close")],
      HintContext::Rename => &[
        Hint::Lit("Tab", "field"),
        Hint::Lit("↑/↓", "type"),
        Hint::Lit("Enter", "submit"),
        Hint::Lit("Esc", "cancel"),
      ],
    }
  }

  /// Resolve this context's hints to `(key, label)` pairs for the statusbar,
  /// reading the live keymap so rebindable verbs show the user's actual
  /// binding (issue #217 review) — the same `primary_chord` source the help
  /// overlay and the Issue/PR prompt use. An unbound action is dropped from
  /// the row rather than advertised with a phantom key.
  pub fn resolve(self, keymap: &super::keymap::Keymap) -> Vec<(String, String)> {
    self
      .hint_specs()
      .iter()
      .filter_map(|h| match h {
        Hint::Key(action, label) => keymap.primary_chord(*action).map(|k| (k, label.to_string())),
        Hint::Lit(key, label) => Some((key.to_string(), label.to_string())),
      })
      .collect()
  }
}

fn action_chord(keymap: &Keymap, action: Action, fallback: &str) -> String {
  keymap.primary_chord(action).unwrap_or_else(|| fallback.to_string())
}

pub fn issue_pr_pane_title(keymap: &Keymap) -> String {
  format!(" Issue / PR [{}] ", action_chord(keymap, Action::FetchGithub, "F"))
}

pub fn working_tree_pane_title(keymap: &Keymap) -> String {
  format!(
    " Working Tree [{}] ",
    action_chord(keymap, Action::ReviewFullscreen, "R")
  )
}

pub fn recent_items_pane_title(mode: SidebarMode, keymap: &Keymap) -> String {
  match mode {
    SidebarMode::Commits => format!(
      " Recent Commits [{}] ",
      action_chord(keymap, Action::LazyGitFullscreen, "l")
    ),
    SidebarMode::Stashes => format!(" Stashes [{}] ", action_chord(keymap, Action::LazyGitFullscreen, "l")),
  }
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

fn modal_hint_for_context(ctx: HintContext, keymap: &Keymap, theme: &Theme) -> Line<'static> {
  let resolved = ctx.resolve(keymap);
  let hints: Vec<(&str, &str)> = resolved.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
  modal_hint_line(&hints, theme)
}

fn push_modal_hint(lines: &mut Vec<Line<'static>>, ctx: HintContext, keymap: &Keymap, theme: &Theme) {
  lines.push(Line::from(String::new()));
  lines.push(modal_hint_for_context(ctx, keymap, theme));
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
  let resolved = ctx.resolve(&app.keymap);
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
pub fn help_rows(km: &super::keymap::Keymap, ctx: HintContext) -> Vec<HelpRow> {
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
  // key string (Ctrl-C, contextual Enter, create-form / confirm keys).
  let fixed = |keys: &str, label: &str| -> HelpRow {
    HelpRow::Entry {
      keys: keys.to_string(),
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
    entry(Action::Top, "jump to first worktree"),
    entry(Action::Bottom, "jump to last worktree"),
  ];
  if picker_mode {
    rows.push(fixed("enter", "select highlighted worktree (prints path on exit)"));
  } else {
    rows.push(entry(Action::Create, "new worktree"));
    rows.push(entry(Action::DeleteConfirm, "delete selected"));
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
    rows.push(entry(Action::BrowseLinks, "open menu — i=issue · p=pull request"));
    rows.push(entry(
      Action::LinkPrompt,
      "link prompt — j/k + enter, or i/p, then digits",
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
      fixed("←/→ ↑/↓", "change branch type"),
      fixed("Tab/Shift-Tab", "next/prev field"),
      fixed("Enter (desc)", "submit"),
      fixed("Esc", "cancel"),
      HelpRow::Blank,
      HelpRow::Section("Delete Worktree".to_string()),
      HelpRow::Blank,
      fixed("←/→ Tab", "move focus between Confirm / Cancel"),
      fixed("Enter", "activate the focused button (defaults to Cancel)"),
      fixed("y", "confirm"),
      fixed("n / Esc", "cancel"),
    ]);
  }
  rows
}

/// Flatten [`help_rows`] back into the legacy `Vec<String>` overlay body
/// (issue #87). Kept as the stable, terminal-free contract that
/// `tests/tui_chord_tests.rs` asserts against: every entry renders as
/// `  {keys:<13} {label}`, sections / title as their bare text, blanks
/// as empty strings. The width 13 is wide enough for `Ctrl+Shift+Tab`.
pub fn help_lines(km: &super::keymap::Keymap, picker_mode: bool) -> Vec<String> {
  // The bool signature is kept for `gwm tui keys` and the chord tests; map
  // it to the context enum (issue #217). The list-view help body is the same
  // for either pane, so `Worktrees` stands in for the non-picker case.
  let ctx = if picker_mode {
    HintContext::Picker
  } else {
    HintContext::Worktrees
  };
  help_rows(km, ctx)
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
  let rows = help_rows(&app.keymap, app.pane_hint_context());

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
    modal_hint_for_context(HintContext::Help, &app.keymap, &app.theme),
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
  f.render_widget(
    modal_hint_line(
      &[
        ("j/k", "scroll"),
        ("g/G", "top/bottom"),
        ("y", "copy"),
        ("Esc", "close"),
      ],
      &app.theme,
    ),
    footer_area,
  );
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

  // Body depends on the active tab.
  let body_lines = match tab {
    SettingsTab::All => settings_all_lines(app),
    other => settings_fields_lines(app, other.fields()),
  };

  // Footer hints — flat accent-bind + muted-action (issue #279), dynamic to
  // the current tab / edit mode.
  let footer_hints: Vec<(&str, &str)> = if editing {
    vec![("Enter", "save"), ("Esc", "cancel")]
  } else if tab == SettingsTab::All {
    vec![("j/k", "scroll"), ("Tab", "section"), ("L", "layer"), ("Esc", "close")]
  } else if selected_kind == Some(FieldKind::Choice) {
    vec![("Space", "cycle"), ("Tab", "section"), ("L", "layer"), ("Esc", "close")]
  } else {
    vec![("Enter", "edit"), ("Tab", "section"), ("L", "layer"), ("Esc", "close")]
  };

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
  let branch = ellipsize_middle(
    &format!("{}/#{}-{}", type_str, app.create_form.issue, app.create_form.desc),
    inner_w.saturating_sub("  Branch : ".len()),
  );
  let dirname = ellipsize_middle(
    &format!("{}-{}-{}", type_str, app.create_form.issue, app.create_form.desc),
    inner_w.saturating_sub("  Dir    : ".len()),
  );

  let mut lines = overlay_title_lines("New Worktree", clean);
  // Type selector first, then the live preview, then the editable fields —
  // the preview sits above the inputs so the resulting names stay in view
  // while typing (issue #217 follow-up).
  lines.push(type_selector_line(
    &label("Type"),
    type_str,
    type_desc,
    app.create_form.field == Field::Type,
    accent,
    muted,
  ));
  lines.push(Line::from(String::new()));
  lines.push(Line::from(vec![
    Span::raw("  Branch : "),
    Span::styled(branch, Style::default().fg(app.theme.branch)),
  ]));
  lines.push(Line::from(vec![
    Span::raw("  Dir    : "),
    Span::styled(dirname, Style::default().fg(app.theme.dirty)),
  ]));
  lines.push(Line::from(String::new()));
  lines.push(field_input_line(
    &label("Issue"),
    &app.create_form.issue,
    app.create_form.field == Field::Issue,
    value_w,
    accent,
    muted,
    surface,
  ));
  lines.push(Line::from(String::new()));
  lines.push(field_input_line(
    &label("Desc"),
    &app.create_form.desc,
    app.create_form.field == Field::Desc,
    value_w,
    accent,
    muted,
    surface,
  ));

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
      Paragraph::new(modal_hint_for_context(HintContext::Create, &app.keymap, &app.theme)),
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
  let primary = chip_style(accent);
  let idle = Style::default().fg(muted).add_modifier(Modifier::BOLD);
  Line::from(vec![
    Span::styled(" Create ", primary),
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

pub fn link_open_modal_lines(app: &App, title: &str, selected: Option<LinkTarget>) -> Vec<Line<'static>> {
  let accent = app.theme.accent;
  let muted = app.theme.muted;
  let mut lines = overlay_title_lines(title, accent);
  lines.extend(github_status_lines(app, 56));
  lines.push(Line::from(""));
  lines.push(link_target_line("i", "Issue", selected == Some(LinkTarget::Issue), accent, muted).centered());
  lines.push(link_target_line("p", "Pull Request", selected == Some(LinkTarget::Pr), accent, muted).centered());
  let ctx = if title == "Link" {
    HintContext::LinkPrompt
  } else {
    HintContext::OpenMenu
  };
  push_modal_hint(&mut lines, ctx, &app.keymap, &app.theme);
  lines
}

fn draw_confirm(f: &mut Frame, app: &App) {
  let muted = app.theme.muted;
  // The destructive modal reads in the theme's "danger" colour (the
  // same role the prunable `⚠` badge uses), so it tracks `[theme]`
  // instead of the pre-#187 hard-coded `Red`.
  let danger = app.theme.prunable;

  let block = overlay_block(danger);

  let Some(w) = app.selected() else {
    let mut lines = overlay_title_lines(delete_worktree_title(), danger);
    lines.push(Line::from("nothing selected").centered());
    let height = lines.len() as u16 + 2 /* border */ + 2 /* padding */;
    let area = centered_h(40, height, f.area());
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(block), area);
    return;
  };

  // Width first (a fixed % of the terminal) so a long path / name can be
  // middle-ellipsized to one line instead of wrapping mid-path (#187
  // review). `text_w` is the room inside the border + padding.
  let term = f.area();
  let outer_w = term.width.saturating_mul(62) / 100;
  let text_w = outer_w.saturating_sub(6) as usize;
  let label_w = "Delete Branch".chars().count();
  let value_w = text_w.saturating_sub(label_w + 2).max(1);

  let name = ellipsize_middle(&w.name, value_w);
  let path = ellipsize_middle(&tilde_compress(&w.path.display().to_string()), value_w);

  // Title stays centred; details use an aligned label/value grid so the
  // destructive target is easier to scan (#220 visual follow-up).
  let mut content: Vec<Line> = overlay_title_lines(delete_worktree_title(), danger);
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
  if let Some(b) = &w.branch {
    let branch = ellipsize_middle(b, value_w);
    content.push(confirm_detail_line(
      "Branch",
      branch,
      label_w,
      muted,
      Style::default().fg(app.theme.branch),
    ));
  }
  content.push(Line::from(""));
  content.push(confirm_delete_branch_line(
    app.delete_branch_on_remove,
    "p",
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
      Paragraph::new(modal_hint_for_context(HintContext::Confirm, &app.keymap, &app.theme)),
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
  render_section(f, layout[2], " Logs ", SectionBody::new(&logs), accent, 0, None);
  f.render_widget(
    Paragraph::new(modal_hint_for_context(HintContext::Report, &app.keymap, &app.theme)),
    layout[4],
  );
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

  let title = match app.pty_overlay.as_ref().map(|p| &p.kind) {
    Some(PtyKind::LazyGit) => " LazyGit ",
    Some(PtyKind::Terminal) => " Terminal ",
    Some(PtyKind::Review) => " Review ",
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

fn trunc(s: &str, max: usize) -> String {
  if s.chars().count() <= max {
    s.to_string()
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
      push_modal_hint(&mut lines, HintContext::LinkPrompt, &app.keymap, &app.theme);
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
  let branch = ellipsize_middle(
    &format!("{}/#{}-{}", type_str, app.create_form.issue, app.create_form.desc),
    inner_w.saturating_sub("  Branch : ".len()),
  );
  let dirname = ellipsize_middle(
    &format!("{}-{}-{}", type_str, app.create_form.issue, app.create_form.desc),
    inner_w.saturating_sub("  Dir    : ".len()),
  );

  let mut lines = overlay_title_lines("Rename Worktree", clean);
  lines.push(Line::from(vec![
    Span::raw("  From   : "),
    Span::styled(old_display, Style::default().fg(muted)),
  ]));
  lines.push(Line::from(String::new()));
  lines.push(type_selector_line(
    &label("Type"),
    type_str,
    type_desc,
    app.create_form.field == Field::Type,
    accent,
    muted,
  ));
  lines.push(Line::from(String::new()));
  lines.push(Line::from(vec![
    Span::raw("  Branch : "),
    Span::styled(branch, Style::default().fg(app.theme.branch)),
  ]));
  lines.push(Line::from(vec![
    Span::raw("  Dir    : "),
    Span::styled(dirname, Style::default().fg(app.theme.dirty)),
  ]));
  lines.push(Line::from(String::new()));
  lines.push(field_input_line(
    &label("Issue"),
    &app.create_form.issue,
    app.create_form.field == Field::Issue,
    value_w,
    accent,
    muted,
    surface,
  ));
  lines.push(Line::from(String::new()));
  lines.push(field_input_line(
    &label("Desc"),
    &app.create_form.desc,
    app.create_form.field == Field::Desc,
    value_w,
    accent,
    muted,
    surface,
  ));

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
      Paragraph::new(create_buttons_line(accent, muted)).alignment(Alignment::Center),
      inner[2],
    );
    f.render_widget(
      Paragraph::new(modal_hint_for_context(HintContext::Rename, &app.keymap, &app.theme)),
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
    title: &'a str,
  },
  Loading,
  Loaded {
    badge: &'a str,
    badge_color: Color,
    trailing: String,
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
      title,
    } => {
      let badge_seg_w = 1 + badge.chars().count() + 2;
      let fixed = prefix_w + badge_seg_w + trailing.chars().count() + 1;
      if fixed >= max_width {
        let mut spans = build_prefix(true);
        spans.push(Span::raw(" "));
        spans.push(Span::raw(format!(" {} ", badge)));
        spans.push(Span::raw(trailing));
        flatten_if_overflow(&mut spans, max_width);
        return Line::from(spans);
      }
      let budget = max_width - fixed;
      let mut spans = build_prefix(true);
      spans.push(Span::raw(" "));
      spans.push(Span::styled(format!(" {} ", badge), chip_style(badge_color)));
      spans.push(Span::raw(trailing));
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
        spans.push(Span::raw(trailing));
        flatten_if_overflow(&mut spans, max_width);
        return Line::from(spans);
      }
      let budget = max_width - fixed;
      let mut spans = build_prefix(true);
      spans.push(Span::raw(" "));
      spans.push(Span::styled(format!(" {} ", badge), chip_style(badge_color)));
      spans.push(Span::raw(trailing));
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
        title: &s.title,
      }
    }
    GitHubFetchState::Error(e) => SummaryState::Error(e),
  };
  summary_line(ISSUE_ICON, head, src, resolved, max_width, theme, spinner)
}

/// Render the Loaded / Idle / Loading / Error variants for a PR link
/// row in the sidebar. See [`issue_summary_line`] for the `max_width`
/// contract — same idea, with a `checks N/M` segment squeezed in between
/// badge and title when the rollup is non-zero.
pub fn pr_summary_line(
  n: u64,
  src: LinkSource,
  state: &GitHubFetchState<crate::github::PrStatus>,
  max_width: usize,
  theme: &Theme,
) -> Line<'static> {
  pr_summary_line_with_spinner(n, src, state, PersistedSummary::none(), max_width, theme, None)
}

fn pr_summary_line_with_spinner(
  n: u64,
  src: LinkSource,
  state: &GitHubFetchState<crate::github::PrStatus>,
  persisted: PersistedSummary<'_, PrState>,
  max_width: usize,
  theme: &Theme,
  spinner: Option<&str>,
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
      let trailing = if s.checks_total > 0 {
        format!(" · checks {}/{}", s.checks_passed, s.checks_total)
      } else {
        String::new()
      };
      SummaryState::Loaded {
        badge,
        badge_color: pr_badge_color(s.state, theme),
        trailing,
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
