use super::app::{App, Field, GitHubFetchState, LinkPromptStage, View};
use crate::bootstrap::StepStatus;
use crate::github::{IssueState, LinkSource, PrState};
use crate::naming::BRANCH_TYPES;
use crate::worktree::{self, BranchStatus, WorktreeInfo};
use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span},
  widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
  Frame,
};
use std::time::{Duration, Instant};

/// Minimum total terminal width required to render the sidebar alongside the
/// worktree table without compressing the table beyond readability.
pub const SIDEBAR_MIN_WIDTH: u16 = 120;

pub fn draw(f: &mut Frame, app: &mut App) {
  // Filter bar is shown while the user is typing, AND while a sticky filter
  // remains in effect (so they can see what's filtering the list).
  let filter_visible = app.filter_active || !app.filter_query.is_empty();

  let chunks = if filter_visible {
    Layout::default()
      .direction(Direction::Vertical)
      .constraints([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(2),
      ])
      .split(f.area())
  } else {
    Layout::default()
      .direction(Direction::Vertical)
      .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(2)])
      .split(f.area())
  };

  draw_header(f, chunks[0], app);
  draw_body(f, chunks[1], app);
  if filter_visible {
    draw_filter_bar(f, chunks[2], app);
    draw_footer(f, chunks[3], app);
  } else {
    draw_footer(f, chunks[2], app);
  }

  match app.view {
    View::Help => draw_help(f, app),
    View::Create => draw_create(f, app),
    View::Confirm => draw_confirm(f, app),
    View::Report => draw_report(f, app),
    View::OpenMenu => draw_open_menu(f, app),
    View::LinkPrompt => draw_link_prompt(f, app),
    View::List => {}
  }
}

/// Single-line filter bar rendered between the table and the footer.
/// Mirrors Vim's `/` prompt: leading slash, the live query, and a block cursor
/// while the user is actively typing.
fn draw_filter_bar(f: &mut Frame, area: Rect, app: &App) {
  let mut spans = vec![
    Span::styled("/", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    Span::raw(app.filter_query.as_str().to_string()),
  ];
  if app.filter_active {
    spans.push(Span::styled(
      "█",
      Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
    ));
  } else {
    // Sticky filter: hint how to clear / refine without re-entering the bar.
    spans.push(Span::styled(
      "   (sticky — / to refine, esc on list to clear)",
      Style::default().fg(Color::DarkGray),
    ));
  }
  f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Decide whether to split horizontally for a sidebar, based on terminal width
/// and user preference. Sidebar is hidden on narrow terminals to keep the
/// worktree table readable.
fn draw_body(f: &mut Frame, area: Rect, app: &mut App) {
  let show_sidebar = app.sidebar_open && area.width >= SIDEBAR_MIN_WIDTH;
  if show_sidebar {
    let split = Layout::default()
      .direction(Direction::Horizontal)
      .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
      .split(area);
    draw_list(f, split[0], app);
    draw_sidebar(f, split[1], app);
  } else {
    // Sidebar not rendered → no scrollable surface → no max scroll to track.
    app.sidebar_max_scroll = 0;
    draw_list(f, area, app);
  }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
  let title = format!(" gwm — {} ({}) ", app.repo_name, app.workdir.display());
  let title_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
  let mut spans = vec![Span::styled(title, title_style)];
  // Picker mode flags the header so the user can never confuse a `gwm switch`
  // session with the full TUI — the action keybindings are different.
  if app.picker_mode {
    spans.push(Span::styled(
      "[picker] ",
      Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    ));
  }
  let p = Paragraph::new(Line::from(spans)).block(
    Block::default()
      .borders(Borders::ALL)
      .border_style(Style::default().fg(Color::DarkGray)),
  );
  f.render_widget(p, area);
}

fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
  // Filter-aware: the visible rows are the filtered subset (issue #21). When
  // there is no active filter, this is the identity over `app.worktrees`.
  let filtered = app.filtered_indices();
  let visible: Vec<&WorktreeInfo> = filtered.iter().filter_map(|&i| app.worktrees.get(i)).collect();

  // Dynamic column widths derived from the visible subset so columns fit the
  // rows actually on screen.
  let name_w = column_width(visible.iter().map(|w| w.name.as_str()), 18, 38);
  let branch_w = column_width(visible.iter().map(|w| w.branch.as_deref().unwrap_or("-")), 18, 38);
  let status_w: u16 = 16;

  let list_has_focus = !(app.sidebar_open && app.sidebar_focused);
  let border_color = if list_has_focus { Color::Cyan } else { Color::DarkGray };

  let title = if app.filter_query.is_empty() {
    format!(" worktrees ({}) ", app.worktrees.len())
  } else {
    format!(" worktrees ({}/{}) ", visible.len(), app.worktrees.len())
  };

  // Pre-allocate the marker + age strips OUTSIDE ratatui's Table
  // widget so their fixed widths are never squeezed by the layout
  // solver. Per ratatui docs the constraint priority is
  // `Min > Max > Length > Percentage > Ratio > Fill`, and when the
  // Table area is tight (sidebar open ⇒ ~60% of frame, narrow term),
  // `Length` columns still get shrunk proportionally — that's how
  // the previous `Length(2)` marker and `Length(4)` age both dropped
  // below their nominal widths.
  //
  // Outer layout: borders → inner → `[2 marker | 4 age | 1 gap | Fill(table)]`.
  // The four zones share the same row baseline (header on row 0,
  // data rows from row 1 down), so the manual strips align cell-for-cell
  // with the Table's rows. On the selected row we paint a DarkGray
  // background across all four zones so the highlight reads as one
  // continuous band rather than three disconnected fragments.
  let outer_block = Block::default()
    .borders(Borders::ALL)
    .title(title)
    .border_style(Style::default().fg(border_color));
  let inner_area = outer_block.inner(area);
  f.render_widget(outer_block, area);

  let inner_split = Layout::horizontal([
    Constraint::Length(2),
    Constraint::Length(4),
    Constraint::Length(1),
    Constraint::Fill(1),
  ])
  .split(inner_area);
  let marker_strip = inner_split[0];
  let age_strip = inner_split[1];
  let gap_strip = inner_split[2];
  let table_area = inner_split[3];

  // Table now carries only the four growable columns; marker + age
  // are rendered manually in their pre-allocated strips below.
  let header = Row::new(vec![
    Cell::from("NAME"),
    Cell::from("BRANCH"),
    Cell::from("STATUS"),
    Cell::from("PATH"),
  ])
  .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD));

  let rows: Vec<Row> = visible
    .iter()
    .map(|w| build_row(w, name_w, branch_w, status_w))
    .collect();

  let widths = [
    Constraint::Min(name_w),
    Constraint::Min(branch_w),
    Constraint::Length(status_w),
    Constraint::Fill(1),
  ];

  // No `highlight_symbol` — the manual marker strip already carries
  // the `★ / ●` glyphs and an arrow would now appear after the
  // strips, breaking visual alignment with the marker column.
  let table = Table::new(rows, widths)
    .header(header)
    .column_spacing(1)
    .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));

  f.render_stateful_widget(table, table_area, &mut app.list_state);

  render_left_strips(
    f,
    marker_strip,
    age_strip,
    gap_strip,
    &visible,
    app.list_state.selected(),
  );
}

/// Paint the manually-allocated marker + age strips, plus the 1-cell
/// gap between them and the Table. Mirrors the Table's
/// `row_highlight_style` (DarkGray background, bold) on the selected
/// row across all three zones so the highlight reads as a single
/// continuous band rather than three disconnected fragments.
fn render_left_strips(
  f: &mut Frame,
  marker_strip: Rect,
  age_strip: Rect,
  gap_strip: Rect,
  visible: &[&WorktreeInfo],
  selected: Option<usize>,
) {
  if marker_strip.height == 0 {
    return;
  }

  let mut marker_lines: Vec<Line<'static>> = Vec::with_capacity(visible.len() + 1);
  let mut age_lines: Vec<Line<'static>> = Vec::with_capacity(visible.len() + 1);
  // Row 0 = header (kept empty — the column glyphs are self-evident).
  marker_lines.push(Line::from(""));
  age_lines.push(Line::from(""));

  for (i, w) in visible.iter().enumerate() {
    let (marker_label, marker_color) = table_marker(w);
    let mut marker_style = Style::default().fg(marker_color);
    let mut age_style = Style::default().fg(Color::DarkGray);
    if Some(i) == selected {
      marker_style = marker_style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
      age_style = age_style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
    }
    marker_lines.push(Line::from(Span::styled(marker_label, marker_style)));
    let age = branch_age_for(w);
    let label = age.map(format_relative_duration_str).unwrap_or_else(|| "-".into());
    age_lines.push(Line::from(Span::styled(label, age_style)));
  }

  f.render_widget(Paragraph::new(marker_lines), marker_strip);
  f.render_widget(Paragraph::new(age_lines), age_strip);

  // The 1-cell gap is empty space between the age strip and the
  // Table. Without painting it, the selected-row highlight would
  // show a 1-cell white slot between marker/age (DarkGray) and the
  // Table (DarkGray). Paint the gap on the selected row only.
  if let Some(sel) = selected {
    // +1 for the header row that lives on `gap_strip.y`.
    let row_y = gap_strip.y.saturating_add(1).saturating_add(sel as u16);
    if row_y < gap_strip.y + gap_strip.height {
      let gap_row = Rect {
        x: gap_strip.x,
        y: row_y,
        width: gap_strip.width,
        height: 1,
      };
      f.buffer_mut().set_style(gap_row, Style::default().bg(Color::DarkGray));
    }
  }
}

/// Details panel for the selected worktree — structured info, recent commits,
/// working-tree status, and a commands cheat-sheet (lazyssh-style layout).
///
/// Content is cached on `App` keyed by the selected worktree's path so the
/// underlying `git log` / `git status` only run when the selection changes
/// or `refresh()` invalidates the cache.
fn draw_sidebar(f: &mut Frame, area: Rect, app: &mut App) {
  let border_color = if app.sidebar_focused {
    Color::Cyan
  } else {
    Color::DarkGray
  };

  // Build the (selection-dependent but stateless) header fresh each
  // frame so the PR-status dot tracks live fetch progress without
  // invalidating the cache. The cached chunk underneath is the git
  // preview, which only changes when the user picks a different
  // worktree or `refresh()` flushes the cache (issue #73).
  let mut lines: Vec<Line<'static>> = match app.selected().cloned() {
    Some(w) => {
      let mut head = vec![sidebar_header_line(&w, app)];
      let needs_refresh = match &app.sidebar_cache {
        Some((p, _)) => *p != w.path,
        None => true,
      };
      if needs_refresh {
        app.sidebar_cache = Some((w.path.clone(), sidebar_lines(&w)));
      }
      head.extend(app.sidebar_cache.as_ref().map(|(_, l)| l.clone()).unwrap_or_default());
      head
    }
    None => vec![Line::from("(nothing selected)")],
  };
  // Append the live Issue / PR block.
  lines.push(Line::from(""));
  lines.extend(github_status_lines(app));

  // Track the maximum scrollable offset so `sidebar_scroll_down` can clamp.
  // `area.height - 2` accounts for the top + bottom border lines.
  let content_len = lines.len() as u16;
  let visible = area.height.saturating_sub(2);
  app.sidebar_max_scroll = content_len.saturating_sub(visible);
  if app.sidebar_scroll > app.sidebar_max_scroll {
    app.sidebar_scroll = app.sidebar_max_scroll;
  }

  let block = Block::default()
    .borders(Borders::ALL)
    .title(" Details ")
    .border_style(Style::default().fg(border_color));

  let paragraph = Paragraph::new(lines)
    .block(block)
    .wrap(Wrap { trim: false })
    .scroll((app.sidebar_scroll, 0));

  f.render_widget(paragraph, area);
}

/// Lazygit-style header line: `● <name>` where the dot's colour tracks
/// the linked PR / issue state. Rendered fresh every frame (not cached)
/// so the dot reflects the live fetch result without invalidating the
/// expensive git preview cache underneath.
fn sidebar_header_line(w: &WorktreeInfo, app: &App) -> Line<'static> {
  let (dot, dot_color) = sidebar_status_dot(app);
  let name_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
  Line::from(vec![
    Span::styled(dot, Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
    Span::styled(w.name.clone(), name_style),
  ])
}

/// Resolve the leading status dot for the sidebar header. PR state wins
/// over issue state (a worktree most often tracks a PR); falls back to a
/// neutral darkgray dot when the worktree has no link at all so the
/// alignment stays consistent across rows.
fn sidebar_status_dot(app: &App) -> (&'static str, Color) {
  if let GitHubFetchState::Loaded(pr) = app.pr_fetch_state() {
    return ("● ", pr_badge_color(pr.state));
  }
  if let GitHubFetchState::Loaded(issue) = app.issue_fetch_state() {
    return ("● ", issue_badge_color(issue.state));
  }
  let link = app.current_link();
  if link.pr.is_some() || link.issue.is_some() {
    // Link exists but not fetched yet — neutral white so the user sees
    // there's *something* to refresh with `R`.
    return ("● ", Color::White);
  }
  ("● ", Color::DarkGray)
}

fn sidebar_lines(w: &WorktreeInfo) -> Vec<Line<'static>> {
  // Branch name colour follows the lazygit scheme (issue #73): worst-state
  // wins (`dirty` → `ahead/behind` → `no upstream` → `synced`) so the most
  // actionable signal stays at eye level. `branch_status_color` is kept for
  // the `Status:` row below; both use the same `BranchStatus` source.
  let branch_color = branch_name_color(&w.status);
  let mut out: Vec<Line> = vec![
    Line::from(""),
    // Basic Settings block.
    section_header("Basic Settings:"),
    kv("Branch", w.branch.clone().unwrap_or_else(|| "-".into()), branch_color),
    kv("Path", w.path.display().to_string(), Color::Gray),
    kv(
      "Head",
      w.head.as_deref().map(short_oid).unwrap_or_else(|| "-".into()),
      Color::Yellow,
    ),
    kv("Created", branch_age_label(w), branch_age_color(w)),
    kv("Main", yes_no(w.is_main), Color::Yellow),
    kv("Locked", yes_no(w.is_locked), Color::Magenta),
    kv("Prunable", yes_no(w.is_prunable), Color::Red),
    kv("Status", branch_status_label(&w.status), branch_status_color(&w.status)),
    Line::from(""),
  ];

  // Recent commits block.
  out.push(section_header("Recent commits:"));
  match worktree::git_log_oneline(&w.path, 10) {
    Ok(s) if !s.trim().is_empty() => {
      for line in s.lines() {
        out.push(Line::from(format!("  {}", line)));
      }
    }
    Ok(_) => out.push(Line::from(Span::styled(
      "  (no commits)",
      Style::default().fg(Color::DarkGray),
    ))),
    Err(e) => out.push(Line::from(Span::styled(
      format!("  ! {}", e),
      Style::default().fg(Color::Red),
    ))),
  }
  out.push(Line::from(""));

  // Working tree block.
  out.push(section_header("Working tree:"));
  match worktree::git_status_short(&w.path) {
    Ok(s) if s.trim().is_empty() => out.push(Line::from(Span::styled("  ✓ clean", Style::default().fg(Color::Green)))),
    Ok(s) => {
      for line in s.lines() {
        out.push(Line::from(format!("  {}", line)));
      }
    }
    Err(e) => out.push(Line::from(Span::styled(
      format!("  ! {}", e),
      Style::default().fg(Color::Red),
    ))),
  }
  out.push(Line::from(""));

  // Commands cheat-sheet (lazyssh style).
  out.push(section_header("Commands:"));
  for (key, label) in [
    ("Enter", "Copy path to status"),
    ("    l", "Launch lazygit fullscreen"),
    ("    o", "Open per [tui.open] (shell/editor/finder)"),
    ("    y", "Yank path to system clipboard"),
    ("    b", "Bootstrap worktree"),
    ("    n", "New worktree"),
    ("    d", "Delete worktree"),
    ("    p", "Toggle delete-branch-on-remove"),
    ("    r", "Refresh"),
    ("    v", "Toggle this sidebar"),
    ("  Tab", "Swap focus list ↔ sidebar"),
    ("    /", "Fuzzy filter worktrees"),
    ("   gg", "Jump to first worktree"),
    ("    G", "Jump to last worktree"),
    ("  j/k", "Next / Prev (or scroll sidebar)"),
    ("    ?", "Help"),
    ("    q", "Quit"),
  ] {
    out.push(Line::from(vec![
      Span::styled(format!("  {}: ", key), Style::default().fg(Color::Cyan)),
      Span::raw(label),
    ]));
  }
  out
}

/// Render the "Created" line value: compact relative duration (`2d`,
/// `3w`, `1M`, …) computed from the worktree's own repository handle, or
/// `"-"` when the branch has no measurable age (trunk, detached HEAD, or
/// repo open failure). The cost is one libgit2 revwalk on each sidebar
/// rebuild — gated by `sidebar_cache` so it only runs on selection
/// change, not every frame.
fn branch_age_label(w: &WorktreeInfo) -> String {
  branch_age_for(w)
    .map(worktree::format_relative_duration)
    .unwrap_or_else(|| "-".into())
}

fn branch_age_color(w: &WorktreeInfo) -> Color {
  branch_age_for(w).map(freshness_color).unwrap_or(Color::DarkGray)
}

fn branch_age_for(w: &WorktreeInfo) -> Option<Duration> {
  let branch = w.branch.as_ref()?;
  let repo = git2::Repository::open(&w.path).ok()?;
  worktree::branch_age(&repo, branch)
}

fn section_header(text: &str) -> Line<'static> {
  Line::from(Span::styled(
    text.to_string(),
    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
  ))
}

fn kv(key: &str, value: String, value_color: Color) -> Line<'static> {
  Line::from(vec![
    Span::styled(format!("  {}: ", key), Style::default().fg(Color::DarkGray)),
    Span::styled(value, Style::default().fg(value_color)),
  ])
}

fn yes_no(b: bool) -> String {
  if b {
    "true".into()
  } else {
    "false".into()
  }
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

fn branch_status_color(s: &BranchStatus) -> Color {
  if s.unknown {
    Color::DarkGray
  } else if s.is_dirty || s.behind > 0 {
    Color::Yellow
  } else if s.ahead > 0 {
    Color::Cyan
  } else {
    Color::Green
  }
}

/// Constraint-friendly column width based on observed content, clamped to [min, max].
fn column_width<'a>(items: impl Iterator<Item = &'a str>, min: u16, max: u16) -> u16 {
  let observed = items.map(|s| s.chars().count() as u16).max().unwrap_or(min);
  observed.clamp(min, max)
}

fn build_row(w: &WorktreeInfo, name_w: u16, branch_w: u16, status_w: u16) -> Row<'static> {
  let (marker_label, marker_color) = table_marker(w);
  let branch_text = w.branch.clone().unwrap_or_else(|| "-".into());

  let name_cell =
    Cell::from(trunc(&w.name, name_w as usize)).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

  // Issue #73: branch column tracks the worst-state colour so the
  // colour-coded signal is visible without expanding the sidebar.
  let branch_cell =
    Cell::from(trunc(&branch_text, branch_w as usize)).style(Style::default().fg(branch_name_color(&w.status)));

  let status_cell = build_status_cell(w, status_w as usize);

  let path_cell = Cell::from(w.path.to_string_lossy().to_string()).style(Style::default().fg(Color::Gray));

  // Age column lives OUTSIDE the Table (pre-allocated in draw_list)
  // to guarantee a fixed 4-cell width regardless of layout pressure;
  // see `render_age_strip`.
  Row::new(vec![
    Cell::from(marker_label).style(Style::default().fg(marker_color)),
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

fn build_status_cell(w: &WorktreeInfo, width: usize) -> Cell<'static> {
  // Priority: prunable > locked > dirty/sync info.
  if w.is_prunable {
    return Cell::from("prunable").style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
  }
  if w.is_locked {
    return Cell::from("locked").style(Style::default().fg(Color::Magenta));
  }

  let s = &w.status;
  let (label, color) = format_status(s, width);
  Cell::from(label).style(Style::default().fg(color))
}

/// Pick a compact label + accent colour for a `BranchStatus`.
fn format_status(s: &BranchStatus, width: usize) -> (String, Color) {
  if s.unknown {
    return ("unknown".into(), Color::DarkGray);
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

  // Worst-status colour: dirty/behind = yellow, ahead-only = cyan, synced/clean = green.
  let color = if s.is_dirty || s.behind > 0 {
    Color::Yellow
  } else if s.ahead > 0 {
    Color::Cyan
  } else {
    Color::Green
  };
  (label, color)
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
  // Picker mode hides the mutating actions (n/d/b/p) — they're inert in the
  // event loop, so advertising them would be a lie.
  let help = if app.picker_mode {
    "enter:select esc:cancel o:open y:yank l:lazygit v:sidebar Tab:focus /:filter j/k:nav r:refresh ?:help q:quit"
  } else {
    "n:new d:del b:boot o:open y:yank l:lazygit v:sidebar Tab:focus /:filter j/k:nav r:refresh ?:help q:quit"
  };
  let text = Line::from(vec![
    Span::styled(help, Style::default().fg(Color::DarkGray)),
    Span::raw("  "),
    Span::styled(format!("[{}]", app.status), Style::default().fg(Color::Yellow)),
  ]);
  f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
}

fn draw_help(f: &mut Frame, app: &App) {
  let area = centered(60, 60, f.area());
  let title_text = if app.picker_mode {
    "gwm switch — keys"
  } else {
    "gwm — keys"
  };
  let mut lines = vec![
    Line::from(Span::styled(
      title_text,
      Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )),
    Line::from(""),
    Line::from("global"),
    Line::from("  q / Esc       quit"),
    Line::from("  Ctrl-C        force quit"),
    Line::from(""),
    Line::from("list view"),
    Line::from("  j / ↓         next (scrolls sidebar when focused)"),
    Line::from("  k / ↑         prev (scrolls sidebar when focused)"),
    Line::from("  gg            jump to first worktree"),
    Line::from("  G             jump to last worktree"),
  ];
  if app.picker_mode {
    lines.push(Line::from(
      "  enter         select highlighted worktree (prints path on exit)",
    ));
  } else {
    lines.push(Line::from("  n             new worktree"));
    lines.push(Line::from("  d             delete selected"));
    lines.push(Line::from("  b             bootstrap selected"));
  }
  lines.extend([
    Line::from("  o             open per [tui.open] — shell (default) / editor / finder"),
    Line::from("  y             yank selected path to system clipboard"),
    Line::from("  l             launch lazygit fullscreen on selected worktree"),
    Line::from("  v             toggle git preview sidebar (auto-hidden < 120 cols)"),
    Line::from("  Tab           swap focus between worktree list and sidebar"),
    Line::from("  /             open fuzzy filter bar (enter: sticky, esc: clear)"),
    Line::from("  r             refresh"),
  ]);
  if !app.picker_mode {
    lines.push(Line::from("  p             toggle 'delete branch on remove'"));
    lines.push(Line::from("  enter         show path in status bar"));
    lines.push(Line::from(""));
    lines.push(Line::from("issue / PR (#67)"));
    lines.push(Line::from("  O             open menu — i=issue · p=pull request"));
    lines.push(Line::from("  L             link prompt — i / p then digits"));
    lines.push(Line::from("  R             refresh GitHub status via `gh`"));
  }
  lines.push(Line::from("  ?             this help"));
  if !app.picker_mode {
    lines.extend([
      Line::from(""),
      Line::from("create form"),
      Line::from("  ↑/↓           change branch type"),
      Line::from("  Tab/Shift-Tab next/prev field"),
      Line::from("  Enter (desc)  submit"),
      Line::from("  Esc           cancel"),
      Line::from(""),
      Line::from("confirm delete"),
      Line::from("  y / Enter     confirm"),
      Line::from("  n / Esc       cancel"),
    ]);
  }
  let block = Block::default()
    .borders(Borders::ALL)
    .title(" help ")
    .border_style(Style::default().fg(Color::Cyan));
  f.render_widget(Clear, area);
  f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_create(f: &mut Frame, app: &App) {
  let area = centered(70, 60, f.area());
  f.render_widget(Clear, area);

  let block = Block::default()
    .borders(Borders::ALL)
    .title(" new worktree ")
    .border_style(Style::default().fg(Color::Green));
  f.render_widget(block, area);

  let inner = Layout::default()
    .direction(Direction::Vertical)
    .margin(2)
    .constraints([
      Constraint::Length(3),
      Constraint::Length(3),
      Constraint::Length(3),
      Constraint::Min(0),
    ])
    .split(area);

  let type_str = BRANCH_TYPES[app.create_type_index].0;
  let type_desc = BRANCH_TYPES[app.create_type_index].1;

  f.render_widget(
    field_input(
      "type (↑/↓)",
      &format!("{} — {}", type_str, type_desc),
      app.create_field == Field::Type,
    ),
    inner[0],
  );
  f.render_widget(
    field_input("issue (digits)", &app.create_issue, app.create_field == Field::Issue),
    inner[1],
  );
  f.render_widget(
    field_input("description (kebab)", &app.create_desc, app.create_field == Field::Desc),
    inner[2],
  );

  // Preview line
  let branch = format!("{}/#{}-{}", type_str, app.create_issue, app.create_desc);
  let dirname = format!("{}-{}-{}", type_str, app.create_issue, app.create_desc);
  let preview = vec![
    Line::from(Span::styled("preview", Style::default().fg(Color::DarkGray))),
    Line::from(vec![
      Span::raw("  branch : "),
      Span::styled(branch, Style::default().fg(Color::Green)),
    ]),
    Line::from(vec![
      Span::raw("  dir    : "),
      Span::styled(dirname, Style::default().fg(Color::Yellow)),
    ]),
  ];
  f.render_widget(Paragraph::new(preview), inner[3]);
}

fn field_input(label: &str, value: &str, focused: bool) -> Paragraph<'static> {
  let border_style = if focused {
    Style::default().fg(Color::Yellow)
  } else {
    Style::default().fg(Color::DarkGray)
  };
  let title = format!(" {} ", label);
  Paragraph::new(value.to_string()).block(
    Block::default()
      .borders(Borders::ALL)
      .title(title)
      .border_style(border_style),
  )
}

fn draw_confirm(f: &mut Frame, app: &App) {
  let area = centered(60, 30, f.area());
  f.render_widget(Clear, area);
  let block = Block::default()
    .borders(Borders::ALL)
    .title(" confirm delete ")
    .border_style(Style::default().fg(Color::Red));
  let body = match app.selected() {
    Some(w) => {
      let mut lines = vec![
        Line::from(""),
        Line::from(vec![
          Span::raw("delete "),
          Span::styled(&w.name, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(format!("at {}", w.path.display())),
      ];
      if let Some(b) = &w.branch {
        lines.push(Line::from(vec![
          Span::raw("branch: "),
          Span::styled(b.clone(), Style::default().fg(Color::Green)),
        ]));
      }
      lines.push(Line::from(""));
      lines.push(Line::from(format!(
        "delete branch too: {}  (press p before to toggle)",
        app.delete_branch_on_remove
      )));
      lines.push(Line::from(""));

      // Footer + (optional) countdown progress bar. Countdown applies
      // only when delete_branch is ON and config secs > 0 (issue #30);
      // otherwise the modal stays single-keystroke as before.
      if app.confirm_is_countdown_mode() {
        let now = Instant::now();
        let total_secs = app.confirm_countdown_total().as_secs();
        if app.confirm_countdown_started_at.is_some() {
          lines.push(Line::from(countdown_bar(
            app.confirm_countdown_progress(now),
            app.confirm_countdown_remaining_secs(now),
          )));
          lines.push(Line::from(Span::styled(
            "y: cancel countdown    n/Esc: cancel",
            Style::default().fg(Color::DarkGray),
          )));
        } else {
          lines.push(Line::from(Span::styled(
            format!("y/Enter: arm {total_secs}s countdown    n/Esc: cancel"),
            Style::default().fg(Color::DarkGray),
          )));
        }
      } else {
        lines.push(Line::from(Span::styled(
          "y/Enter: confirm    n/Esc: cancel",
          Style::default().fg(Color::DarkGray),
        )));
      }
      lines
    }
    None => vec![Line::from("nothing selected")],
  };
  f.render_widget(Paragraph::new(body).block(block).wrap(Wrap { trim: false }), area);
}

/// Build the `[████░░] Ns — Esc to cancel` countdown line. Width is fixed
/// at 10 cells so the bar reads the same regardless of modal size.
fn countdown_bar<'a>(progress: f64, remaining_secs: u64) -> Vec<Span<'a>> {
  const CELLS: usize = 10;
  let filled = filled_cells_for_progress(progress, CELLS);
  let bar: String = std::iter::repeat_n('█', filled)
    .chain(std::iter::repeat_n('░', CELLS - filled))
    .collect();
  vec![
    Span::styled("  [", Style::default().fg(Color::DarkGray)),
    Span::styled(bar, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
    Span::styled("] ", Style::default().fg(Color::DarkGray)),
    Span::styled(
      format!("{remaining_secs}s"),
      Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    ),
    Span::styled(" — Esc to cancel", Style::default().fg(Color::DarkGray)),
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

fn draw_report(f: &mut Frame, app: &App) {
  let area = centered(80, 80, f.area());
  f.render_widget(Clear, area);
  let block = Block::default()
    .borders(Borders::ALL)
    .title(" bootstrap report ")
    .border_style(Style::default().fg(Color::Cyan));

  let mut lines: Vec<Line> = Vec::new();
  if let Some(report) = &app.report {
    for step in &report.steps {
      let (sigil, color) = match step.status {
        StepStatus::Ok => ("✓", Color::Green),
        StepStatus::Skipped => ("·", Color::DarkGray),
        StepStatus::Warning => ("!", Color::Yellow),
        StepStatus::Failed => ("✗", Color::Red),
      };
      lines.push(Line::from(vec![
        Span::styled(
          format!(" {} ", sigil),
          Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(step.label.clone(), Style::default().fg(Color::White)),
      ]));
      for detail_line in step.detail.lines() {
        lines.push(Line::from(format!("      {}", detail_line)));
      }
    }
  } else {
    lines.push(Line::from("(no report)"));
  }
  lines.push(Line::from(""));
  lines.push(Line::from(Span::styled(
    "Enter / Esc — close",
    Style::default().fg(Color::DarkGray),
  )));

  f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
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

fn draw_open_menu(f: &mut Frame, _app: &App) {
  let area = centered(40, 22, f.area());
  f.render_widget(Clear, area);
  let block = Block::default()
    .borders(Borders::ALL)
    .title(" open ")
    .border_style(Style::default().fg(Color::Magenta));
  let lines = vec![
    Line::from(Span::styled(
      "open in browser",
      Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
    )),
    Line::from(""),
    Line::from("  i   linked issue"),
    Line::from("  p   linked pull request"),
    Line::from(""),
    Line::from(Span::styled("  esc to cancel", Style::default().fg(Color::DarkGray))),
  ];
  f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_link_prompt(f: &mut Frame, app: &App) {
  let area = centered(50, 30, f.area());
  f.render_widget(Clear, area);
  let block = Block::default()
    .borders(Borders::ALL)
    .title(" link ")
    .border_style(Style::default().fg(Color::Yellow));

  let lines = match app.link_prompt_stage() {
    LinkPromptStage::ChooseTarget => vec![
      Line::from(Span::styled(
        "link this worktree to:",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
      )),
      Line::from(""),
      Line::from("  i   a GitHub issue"),
      Line::from("  p   a pull request"),
      Line::from(""),
      Line::from(Span::styled("  esc to cancel", Style::default().fg(Color::DarkGray))),
    ],
    LinkPromptStage::InputNumber => {
      let label = match app.link_prompt_target() {
        Some(super::app::LinkTarget::Issue) => "issue #",
        Some(super::app::LinkTarget::Pr) => "PR #",
        None => "#",
      };
      vec![
        Line::from(Span::styled(
          format!("type the {} number", label.trim_end_matches('#').trim()),
          Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("  {}{}_", label, app.link_prompt_number_input())),
        Line::from(""),
        Line::from(Span::styled(
          "  enter confirms · esc cancels · backspace deletes",
          Style::default().fg(Color::DarkGray),
        )),
      ]
    }
  };
  f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Append the issue/PR status block to the bottom of the sidebar. Called
/// from `draw_sidebar` after the git preview, so it shows below the recent
/// commits / status block.
pub(super) fn github_status_lines(app: &App) -> Vec<Line<'static>> {
  let link = app.current_link();
  let mut lines: Vec<Line<'static>> = Vec::new();

  lines.push(Line::from(Span::styled(
    "─── Issue / PR ───",
    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
  )));

  if link.issue.is_none() && link.pr.is_none() {
    lines.push(Line::from(Span::styled(
      "  no link · press L to link",
      Style::default().fg(Color::DarkGray),
    )));
    return lines;
  }

  if let Some(n) = link.issue {
    lines.push(issue_summary_line(n, link.issue_source, app.issue_fetch_state()));
  }
  if let Some(n) = link.pr {
    lines.push(pr_summary_line(n, link.pr_source, app.pr_fetch_state()));
  }
  if matches!(app.issue_fetch_state(), GitHubFetchState::Idle) && matches!(app.pr_fetch_state(), GitHubFetchState::Idle)
  {
    lines.push(Line::from(Span::styled(
      "  press R to fetch status",
      Style::default().fg(Color::DarkGray),
    )));
  }
  lines
}

fn source_marker(s: LinkSource) -> &'static str {
  match s {
    LinkSource::None => "",
    LinkSource::BranchName => " (auto)",
    LinkSource::Explicit => "",
  }
}

fn issue_summary_line(n: u64, src: LinkSource, state: &GitHubFetchState<crate::github::IssueStatus>) -> Line<'static> {
  let head = format!("Issue #{}{}", n, source_marker(src));
  match state {
    GitHubFetchState::Idle => Line::from(Span::styled(head, Style::default().fg(Color::White))),
    GitHubFetchState::Loading => Line::from(format!("{} …loading", head)),
    GitHubFetchState::Loaded(s) => {
      let badge_color = match s.state {
        IssueState::Open => Color::Green,
        IssueState::Closed => Color::Red,
      };
      let badge = match s.state {
        IssueState::Open => "open",
        IssueState::Closed => "closed",
      };
      Line::from(vec![
        Span::styled(head, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" ["),
        Span::styled(
          badge.to_string(),
          Style::default().fg(badge_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("] "),
        Span::raw(trunc(&s.title, 40)),
      ])
    }
    GitHubFetchState::Error(e) => Line::from(vec![
      Span::styled(head, Style::default().fg(Color::White)),
      Span::raw(" "),
      Span::styled(format!("!{}", trunc(e, 30)), Style::default().fg(Color::Red)),
    ]),
  }
}

fn pr_summary_line(n: u64, src: LinkSource, state: &GitHubFetchState<crate::github::PrStatus>) -> Line<'static> {
  let head = format!("PR    #{}{}", n, source_marker(src));
  match state {
    GitHubFetchState::Idle => Line::from(Span::styled(head, Style::default().fg(Color::White))),
    GitHubFetchState::Loading => Line::from(format!("{} …loading", head)),
    GitHubFetchState::Loaded(s) => {
      let (badge, badge_color) = match s.state {
        PrState::Open => ("open", Color::Green),
        PrState::Draft => ("draft", Color::DarkGray),
        PrState::Closed => ("closed", Color::Red),
        PrState::Merged => ("merged", Color::Magenta),
      };
      let checks = if s.checks_total > 0 {
        format!(" · checks {}/{}", s.checks_passed, s.checks_total)
      } else {
        String::new()
      };
      Line::from(vec![
        Span::styled(head, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" ["),
        Span::styled(
          badge.to_string(),
          Style::default().fg(badge_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("]"),
        Span::raw(checks),
        Span::raw(" "),
        Span::raw(trunc(&s.title, 36)),
      ])
    }
    GitHubFetchState::Error(e) => Line::from(vec![
      Span::styled(head, Style::default().fg(Color::White)),
      Span::raw(" "),
      Span::styled(format!("!{}", trunc(e, 30)), Style::default().fg(Color::Red)),
    ]),
  }
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
pub fn branch_name_color(s: &BranchStatus) -> Color {
  if s.unknown {
    return Color::DarkGray;
  }
  if s.is_dirty {
    return Color::Red;
  }
  if s.ahead > 0 || s.behind > 0 {
    return Color::Yellow;
  }
  if !s.has_upstream {
    // Lazygit's `?` marker — branch never pushed yet. Distinct from
    // synced so the user knows whether they need to run `git push`.
    return Color::Magenta;
  }
  Color::Green
}

/// Map a branch age to a freshness colour: green < 7d, yellow < 30d,
/// darkgray otherwise. Cutoffs are wide on purpose — a 6-day branch
/// is "fresh", a 4-week one is "ageing", a 5-week one is "stale" —
/// so the colour shift registers as signal rather than noise.
pub fn freshness_color(age: Duration) -> Color {
  const WEEK: u64 = 7 * 86_400;
  const MONTH: u64 = 30 * 86_400;
  let s = age.as_secs();
  if s < WEEK {
    Color::Green
  } else if s < MONTH {
    Color::Yellow
  } else {
    Color::DarkGray
  }
}

/// Pick a colour for the PR-status dot rendered in the sidebar header.
/// Ports the lazygit `WithPrColor` palette (open=green, draft=gray,
/// merged=magenta, closed=red) but uses 16-colour names instead of
/// hex RGB so the badge respects the user's terminal theme.
pub fn pr_badge_color(state: PrState) -> Color {
  match state {
    PrState::Open => Color::Green,
    PrState::Draft => Color::DarkGray,
    PrState::Merged => Color::Magenta,
    PrState::Closed => Color::Red,
  }
}

/// Same idea as [`pr_badge_color`] but for a linked issue. Closed maps
/// to magenta (treated as "moved on") rather than red so a routinely
/// resolved issue doesn't read as alarming.
pub fn issue_badge_color(state: IssueState) -> Color {
  match state {
    IssueState::Open => Color::Green,
    IssueState::Closed => Color::Magenta,
  }
}

/// Pick the marker glyph + colour for the table's first column. `★`
/// for the main worktree (preserves the pre-#73 convention), `●` for
/// any other worktree that carries an issue or PR link, blank space
/// otherwise so unlinked rows don't read as "claimed". Colour stays
/// neutral (Cyan) — the live PR / issue state isn't known at the
/// table layer (only the selected worktree triggers a fetch), so the
/// table dot signals "has link" rather than a specific status. The
/// colour-coded `●` lives in the sidebar header where the fetch state
/// is available.
pub fn table_marker(w: &WorktreeInfo) -> (&'static str, Color) {
  if w.is_main {
    return ("★", Color::Yellow);
  }
  if w.link.issue.is_some() || w.link.pr.is_some() {
    return ("●", Color::Cyan);
  }
  (" ", Color::Reset)
}
