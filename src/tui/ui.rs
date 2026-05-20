use super::app::{App, Field, GitHubFetchState, LinkPromptStage, View};
use crate::bootstrap::StepStatus;
use crate::github::{IssueState, LinkSource, PrState};
use crate::naming::BRANCH_TYPES;
use crate::worktree::{self, BranchStatus, WorktreeInfo};
use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span},
  widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
  Frame,
};
use std::time::Instant;

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
  /// Up to 10 oneline commits, or an empty / error notice.
  pub recent_commits: Vec<Line<'static>>,
}

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
  // rows actually on screen. The path column is always last and absorbs the
  // remaining width.
  let name_w = column_width(visible.iter().map(|w| w.name.as_str()), 18, 38);
  let branch_w = column_width(visible.iter().map(|w| w.branch.as_deref().unwrap_or("-")), 18, 38);
  let status_w: u16 = 16;

  let header = Row::new(vec![
    Cell::from(""),
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
    Constraint::Length(2),
    Constraint::Length(name_w),
    Constraint::Length(branch_w),
    Constraint::Length(status_w),
    Constraint::Min(20),
  ];

  let list_has_focus = !(app.sidebar_open && app.sidebar_focused);
  let border_color = if list_has_focus { Color::Cyan } else { Color::DarkGray };

  let title = if app.filter_query.is_empty() {
    format!(" worktrees ({}) ", app.worktrees.len())
  } else {
    format!(" worktrees ({}/{}) ", visible.len(), app.worktrees.len())
  };

  let table = Table::new(rows, widths)
    .header(header)
    .column_spacing(1)
    .block(
      Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color)),
    )
    .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
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
  let border_color = if app.sidebar_focused {
    Color::Cyan
  } else {
    Color::DarkGray
  };

  // Resolve (or populate) the cached worktree sections for the current
  // selection. Issue / PR block is rebuilt every frame (its fetch state
  // moves independently of the worktree info).
  let sections = match app.selected().cloned() {
    Some(w) => {
      let needs_refresh = match &app.sidebar_cache {
        Some((p, _)) => *p != w.path,
        None => true,
      };
      if needs_refresh {
        app.sidebar_cache = Some((w.path.clone(), build_sidebar_sections(&w)));
      }
      app.sidebar_cache.as_ref().map(|(_, s)| s.clone()).unwrap_or_default()
    }
    None => SidebarSections {
      worktree: vec![Line::from("(nothing selected)")],
      working_tree: vec![],
      recent_commits: vec![],
    },
  };
  // Inner width = block area − 2 border columns − 1 leading-padding column
  // (applied by `render_section`). Summary lines trim their variable parts
  // (title / error blob) so the total visible width fits — without this,
  // long PR titles would either overflow the block right border or be
  // wrapped onto a second visual row that the `Constraint::Length` below
  // never budgeted for, breaking the layout.
  let issue_pr_inner_width = area.width.saturating_sub(3) as usize;
  let issue_pr_lines = github_status_lines(app, issue_pr_inner_width);

  // Per-section block height = content rows + 2 border lines. Fixed for
  // the small sections (worktree / issue-PR / working-tree); Recent
  // Commits flexes to fill the rest of the sidebar height.
  let h = |lines: usize| (lines as u16).saturating_add(2);
  let constraints = [
    Constraint::Length(h(sections.worktree.len())),
    Constraint::Length(h(issue_pr_lines.len())),
    Constraint::Length(h(sections.working_tree.len())),
    Constraint::Min(3),
  ];
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints(constraints)
    .split(area);

  render_section(f, chunks[0], " Worktree ", sections.worktree, border_color, 0);
  render_section(f, chunks[1], " Issue / PR ", issue_pr_lines, border_color, 0);
  render_section(f, chunks[2], " Working Tree ", sections.working_tree, border_color, 0);

  // Recent Commits is the only scrollable section. Clamp the scroll
  // offset to its visible area so `j` / `k` can't scroll past the end.
  let commits_area = chunks[3];
  let commits_visible = commits_area.height.saturating_sub(2);
  app.sidebar_max_scroll = (sections.recent_commits.len() as u16).saturating_sub(commits_visible);
  if app.sidebar_scroll > app.sidebar_max_scroll {
    app.sidebar_scroll = app.sidebar_max_scroll;
  }
  render_section(
    f,
    commits_area,
    " Recent Commits ",
    sections.recent_commits,
    border_color,
    app.sidebar_scroll,
  );
}

fn render_section(
  f: &mut Frame,
  area: Rect,
  title: &'static str,
  lines: Vec<Line<'static>>,
  border_color: Color,
  scroll: u16,
) {
  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .title(title)
    .border_style(Style::default().fg(border_color));
  // Pad content with one leading space per line for breathing room against
  // the left border. Cheap and avoids per-call `format!` churn.
  let padded: Vec<Line<'static>> = lines
    .into_iter()
    .map(|l| {
      let mut spans = Vec::with_capacity(l.spans.len() + 1);
      spans.push(Span::raw(" "));
      spans.extend(l.spans);
      Line::from(spans)
    })
    .collect();
  let paragraph = Paragraph::new(padded)
    .block(block)
    .wrap(Wrap { trim: false })
    .scroll((scroll, 0));
  f.render_widget(paragraph, area);
}

/// Build the per-section content of the details sidebar for one worktree.
///
/// The Commands cheat-sheet block is intentionally not produced here — it
/// duplicated the `?` help overlay and consumed ~15 vertical lines for no
/// new information. Press `?` for the full key map.
pub fn build_sidebar_sections(w: &WorktreeInfo) -> SidebarSections {
  SidebarSections {
    worktree: worktree_identity_lines(w),
    working_tree: working_tree_lines(w),
    recent_commits: recent_commits_lines(w),
  }
}

/// 3–4 line identity card: name, `branch · head`, status + flag badges,
/// tilde-compressed path. Skips badges whose flags are false to avoid
/// visual noise (`false`/`true` columns scaled poorly with the old layout).
fn worktree_identity_lines(w: &WorktreeInfo) -> Vec<Line<'static>> {
  let mut out: Vec<Line<'static>> = Vec::with_capacity(4);

  // Line 1 — worktree name in bold white.
  out.push(Line::from(Span::styled(
    w.name.clone(),
    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
  )));

  // Line 2 — "<branch> · <short head>". Skip head sub-span when absent.
  let branch = w.branch.clone().unwrap_or_else(|| "-".into());
  let mut spans = vec![Span::styled(branch, Style::default().fg(Color::Green))];
  if let Some(head) = w.head.as_deref() {
    spans.push(Span::styled("  ·  ".to_string(), Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled(short_oid(head), Style::default().fg(Color::Yellow)));
  }
  out.push(Line::from(spans));

  // Line 3 — status badge + optional flag badges. Only renders the badges
  // that are *true* / *interesting*; the false cases stay invisible.
  out.push(badges_line(w));

  // Line 4 — path, tilde-compressed for compactness.
  out.push(Line::from(Span::styled(
    tilde_compress(&w.path.display().to_string()),
    Style::default().fg(Color::DarkGray),
  )));

  out
}

fn badges_line(w: &WorktreeInfo) -> Line<'static> {
  let mut spans: Vec<Span<'static>> = Vec::new();
  // Status sigil:
  //   `?`     — unknown
  //   `●`     — dirty (working tree or index)
  //   `✓`     — synced / clean (no divergence)
  //   (none)  — ahead / behind / both — the label already carries `↑N` /
  //             `↓M` / `↑N ↓M`. Prefixing `✓` here would lie about
  //             divergence (raised by PR #70 Copilot review).
  let status_label = branch_status_label(&w.status);
  let status_color = branch_status_color(&w.status);
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

  let sep = || Span::styled("  ".to_string(), Style::default().fg(Color::DarkGray));
  if w.is_main {
    spans.push(sep());
    spans.push(Span::styled("★ main".to_string(), Style::default().fg(Color::Yellow)));
  }
  if w.is_locked {
    spans.push(sep());
    spans.push(Span::styled(
      "🔒 locked".to_string(),
      Style::default().fg(Color::Magenta),
    ));
  }
  if w.is_prunable {
    spans.push(sep());
    spans.push(Span::styled("⚠ prunable".to_string(), Style::default().fg(Color::Red)));
  }
  Line::from(spans)
}

fn working_tree_lines(w: &WorktreeInfo) -> Vec<Line<'static>> {
  match worktree::git_status_short(&w.path) {
    Ok(s) if s.trim().is_empty() => vec![Line::from(Span::styled(
      "✓ clean".to_string(),
      Style::default().fg(Color::Green),
    ))],
    Ok(s) => s.lines().map(|l| Line::from(l.to_string())).collect(),
    Err(e) => vec![Line::from(Span::styled(
      format!("! {}", e),
      Style::default().fg(Color::Red),
    ))],
  }
}

fn recent_commits_lines(w: &WorktreeInfo) -> Vec<Line<'static>> {
  match worktree::git_log_oneline(&w.path, 10) {
    Ok(s) if !s.trim().is_empty() => s.lines().map(|l| Line::from(l.to_string())).collect(),
    Ok(_) => vec![Line::from(Span::styled(
      "(no commits)".to_string(),
      Style::default().fg(Color::DarkGray),
    ))],
    Err(e) => vec![Line::from(Span::styled(
      format!("! {}", e),
      Style::default().fg(Color::Red),
    ))],
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
  let marker = if w.is_main { "★" } else { " " };
  let branch_text = w.branch.clone().unwrap_or_else(|| "-".into());

  let name_cell =
    Cell::from(trunc(&w.name, name_w as usize)).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

  let branch_cell = Cell::from(trunc(&branch_text, branch_w as usize)).style(Style::default().fg(Color::Green));

  let status_cell = build_status_cell(w, status_w as usize);

  let path_cell = Cell::from(w.path.to_string_lossy().to_string()).style(Style::default().fg(Color::Gray));

  Row::new(vec![
    Cell::from(marker).style(Style::default().fg(Color::Yellow)),
    name_cell,
    branch_cell,
    status_cell,
    path_cell,
  ])
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
    "enter:select esc:cancel o:open l:lazygit v:sidebar Tab:focus /:filter gg/G:top/bot j/k:nav r:refresh ?:help q:quit"
  } else {
    "n:new d:del b:boot o:open l:lazygit v:sidebar Tab:focus /:filter gg/G:top/bot j/k:nav r:refresh ?:help q:quit"
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
    Line::from("  o             open dir in OS file manager (open / xdg-open / explorer)"),
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

/// Body of the Issue / PR sidebar block. The block title (`" Issue / PR "`)
/// is supplied by [`draw_sidebar`] via the surrounding `Block`, so this
/// function only returns the content rows. `max_width` is the inner
/// width of the Issue / PR block (chunk width minus 2 borders and the
/// 1-char left padding applied by [`render_section`]); summary lines
/// trim their variable parts so total visible width ≤ `max_width`.
pub(super) fn github_status_lines(app: &App, max_width: usize) -> Vec<Line<'static>> {
  let link = app.current_link();
  let mut lines: Vec<Line<'static>> = Vec::new();

  if link.issue.is_none() && link.pr.is_none() {
    lines.push(Line::from(Span::styled(
      trunc("no link · press L to link", max_width),
      Style::default().fg(Color::DarkGray),
    )));
    return lines;
  }

  if let Some(n) = link.issue {
    lines.push(issue_summary_line(
      n,
      link.issue_source,
      app.issue_fetch_state(),
      max_width,
    ));
  }
  if let Some(n) = link.pr {
    lines.push(pr_summary_line(n, link.pr_source, app.pr_fetch_state(), max_width));
  }
  if matches!(app.issue_fetch_state(), GitHubFetchState::Idle) && matches!(app.pr_fetch_state(), GitHubFetchState::Idle)
  {
    lines.push(Line::from(Span::styled(
      trunc("press R to fetch status", max_width),
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
) -> Line<'static> {
  let head = format!("Issue #{}{}", n, source_marker(src));
  match state {
    GitHubFetchState::Idle => Line::from(Span::styled(trunc(&head, max_width), Style::default().fg(Color::White))),
    GitHubFetchState::Loading => Line::from(trunc(&format!("{} …loading", head), max_width)),
    GitHubFetchState::Loaded(s) => {
      let badge_color = match s.state {
        IssueState::Open => Color::Green,
        IssueState::Closed => Color::Red,
      };
      let badge = match s.state {
        IssueState::Open => "open",
        IssueState::Closed => "closed",
      };
      // Fixed prefix = "<head> [<badge>] " — try to preserve in full and
      // trim the title to whatever budget remains. If the prefix alone
      // already exceeds the width budget (very narrow sidebar), fall
      // back to flattening the line into a single styled string and
      // truncating it — preserves no badge color but stays inside the
      // block.
      let fixed = head.chars().count() + 4 + badge.chars().count(); // " [" + badge + "] "
      if fixed >= max_width {
        let raw = format!("{} [{}] {}", head, badge, s.title);
        return Line::from(trunc(&raw, max_width));
      }
      let budget = max_width - fixed;
      Line::from(vec![
        Span::styled(head, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" ["),
        Span::styled(
          badge.to_string(),
          Style::default().fg(badge_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("] "),
        Span::raw(trunc(&s.title, budget)),
      ])
    }
    GitHubFetchState::Error(e) => {
      let fixed = head.chars().count() + 2; // " " + "!"
      let budget = max_width.saturating_sub(fixed);
      Line::from(vec![
        Span::styled(head, Style::default().fg(Color::White)),
        Span::raw(" "),
        Span::styled(format!("!{}", trunc(e, budget)), Style::default().fg(Color::Red)),
      ])
    }
  }
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
) -> Line<'static> {
  let head = format!("PR    #{}{}", n, source_marker(src));
  match state {
    GitHubFetchState::Idle => Line::from(Span::styled(trunc(&head, max_width), Style::default().fg(Color::White))),
    GitHubFetchState::Loading => Line::from(trunc(&format!("{} …loading", head), max_width)),
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
      let fixed = head.chars().count() + 3 + badge.chars().count() + checks.chars().count() + 1; // " [" + badge + "]" + checks + " "
      if fixed >= max_width {
        // Very narrow sidebar — fall back to a single truncated string.
        // Drops the badge color but keeps the line inside the block.
        let raw = format!("{} [{}]{} {}", head, badge, checks, s.title);
        return Line::from(trunc(&raw, max_width));
      }
      let budget = max_width - fixed;
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
        Span::raw(trunc(&s.title, budget)),
      ])
    }
    GitHubFetchState::Error(e) => {
      let fixed = head.chars().count() + 2; // " " + "!"
      let budget = max_width.saturating_sub(fixed);
      Line::from(vec![
        Span::styled(head, Style::default().fg(Color::White)),
        Span::raw(" "),
        Span::styled(format!("!{}", trunc(e, budget)), Style::default().fg(Color::Red)),
      ])
    }
  }
}
