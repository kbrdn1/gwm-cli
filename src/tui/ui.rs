use super::app::{App, GitHubFetchState, LinkPromptStage, View};
use super::state::create_form::Field;
use crate::bootstrap::StepStatus;
use crate::github::{IssueState, LinkSource, PrState};
use crate::worktree::{self, BranchStatus, WorktreeInfo};
use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span},
  widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
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
  /// Up to 10 oneline commits, or an empty / error notice.
  pub recent_commits: Vec<Line<'static>>,
}

/// Minimum total terminal width required to render the sidebar alongside the
/// worktree table without compressing the table beyond readability.
pub const SIDEBAR_MIN_WIDTH: u16 = 120;

pub fn draw(f: &mut Frame, app: &mut App) {
  // Filter bar is shown while the user is typing, AND while a sticky filter
  // remains in effect (so they can see what's filtering the list).
  let filter_visible = app.filter.active || !app.filter.query.is_empty();

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

/// Format the TUI header title `" gwm v<version> — <repo> (<path>) "`.
/// The version comes from `CARGO_PKG_VERSION` so it stays in lockstep
/// with `gwm --version` without a second source of truth — important
/// for users juggling multiple installs (a `cargo install`-ed gwm next
/// to a worktree-built dev binary). Extracted from `draw_header` so
/// the format can be pinned by a unit test without spinning up the
/// ratatui backend.
pub fn header_title(repo_name: &str, workdir_display: &str) -> String {
  format!(
    " gwm v{} — {} ({}) ",
    env!("CARGO_PKG_VERSION"),
    repo_name,
    workdir_display
  )
}

/// Single-line filter bar rendered between the table and the footer.
/// Mirrors Vim's `/` prompt: leading slash, the live query, and a block cursor
/// while the user is actively typing.
fn draw_filter_bar(f: &mut Frame, area: Rect, app: &App) {
  let mut spans = vec![
    Span::styled("/", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    Span::raw(app.filter.query.as_str().to_string()),
  ];
  if app.filter.active {
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
  let title = header_title(&app.repo_name, &app.workdir.to_string_lossy());
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
  // Borrow scoping: `filtered_indices` returns `&[usize]` rooted in
  // `&mut app.filter`, which conflicts with the immutable `app.worktrees`
  // read on the next line. Materialise the indices into an owned `Vec`
  // so the mutable borrow ends. The expensive path (nucleo pass) stays
  // memoised on `FilterState`; this per-frame clone is just a Vec<usize>
  // of length ≤ worktrees.len().
  let filtered: Vec<usize> = app.filtered_indices().to_vec();
  let visible: Vec<&WorktreeInfo> = filtered.iter().filter_map(|&i| app.worktrees.get(i)).collect();

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
  .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD));

  let rows: Vec<Row> = visible
    .iter()
    .map(|w| build_row(w, name_w, branch_w, status_w))
    .collect();

  // ratatui's Layout solver squeezes the FIRST `Length` column to
  // satisfy the others when terminal width is tight. We want the age
  // column rock-stable at 4 cells (the cost of losing the unit
  // letter to truncation — "22h" → "22" — is worse than name/branch
  // shrinking by a char or two). Strategy:
  //   - `Length(4)` for age, `Length(2)` for marker, `Length(16)` for
  //     status: hard-fixed lengths the solver must honour.
  //   - `Min(name_w)` / `Min(branch_w)`: these absorb the pressure
  //     when the terminal is narrow (they shrink down to 8) and grow
  //     to the original clamped width (or more) when there's room.
  //   - `Fill(1)` for path: takes whatever's left, vanishes last.
  // Verified by standalone probe down to 40-cell terminals: col 0
  // stays at 4 cells across every size.
  let widths = [
    Constraint::Length(4),
    Constraint::Length(2),
    Constraint::Min(name_w),
    Constraint::Min(branch_w),
    Constraint::Length(status_w),
    Constraint::Fill(1),
  ];

  let list_has_focus = !(app.sidebar_open && app.sidebar_focused);
  let border_color = if list_has_focus { Color::Cyan } else { Color::DarkGray };

  let title = if app.filter.query.is_empty() {
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
  // moves independently of the worktree info). The leading `●` status
  // dot line on the Worktree section is also rebuilt fresh each frame
  // (issue #73) so it tracks live PR / issue fetches without
  // invalidating the expensive git-preview cache underneath.
  let sections = match app.selected().cloned() {
    Some(w) => {
      let needs_refresh = match &app.sidebar_cache {
        Some((p, _)) => *p != w.path,
        None => true,
      };
      if needs_refresh {
        app.sidebar_cache = Some((w.path.clone(), build_sidebar_sections(&w)));
      }
      let mut cached = app.sidebar_cache.as_ref().map(|(_, s)| s.clone()).unwrap_or_default();
      let mut worktree = vec![sidebar_header_line(&w, app)];
      worktree.append(&mut cached.worktree);
      SidebarSections {
        worktree,
        working_tree: cached.working_tree,
        recent_commits: cached.recent_commits,
      }
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

  render_section(f, chunks[0], " Worktree ", sections.worktree, border_color, 0, None);
  render_section(f, chunks[1], " Issue / PR ", issue_pr_lines, border_color, 0, None);
  render_section(
    f,
    chunks[2],
    " Working Tree ",
    sections.working_tree,
    border_color,
    0,
    None,
  );

  // Recent Commits is the only scrollable section. Clamp the scroll
  // offset to its visible area so `j` / `k` can't scroll past the end.
  // The block's bottom-right title mirrors lazygit's footer
  // ("<i+1> of <N>") so the user can tell at a glance how much history
  // is queued and where the viewport sits.
  let commits_area = chunks[3];
  let commits_visible = commits_area.height.saturating_sub(2);
  let commits_len = sections.recent_commits.len() as u16;
  app.sidebar_max_scroll = commits_len.saturating_sub(commits_visible);
  if app.sidebar_scroll > app.sidebar_max_scroll {
    app.sidebar_scroll = app.sidebar_max_scroll;
  }
  let footer = if commits_len == 0 {
    None
  } else {
    let bottom = app.sidebar_scroll.saturating_add(commits_visible).min(commits_len);
    Some(format!(" {} of {} ", bottom, commits_len))
  };
  render_section(
    f,
    commits_area,
    " Recent Commits ",
    sections.recent_commits,
    border_color,
    app.sidebar_scroll,
    footer,
  );
}

fn render_section(
  f: &mut Frame,
  area: Rect,
  title: &'static str,
  lines: Vec<Line<'static>>,
  border_color: Color,
  scroll: u16,
  footer: Option<String>,
) {
  let mut block = Block::default()
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .title(title)
    .border_style(Style::default().fg(border_color));
  if let Some(f) = footer {
    block = block.title_bottom(ratatui::text::Line::from(f).right_aligned());
  }
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
    // there's *something* to refresh with `F`.
    return ("● ", Color::White);
  }
  ("● ", Color::DarkGray)
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
pub fn build_sidebar_sections(w: &WorktreeInfo) -> SidebarSections {
  SidebarSections {
    worktree: worktree_identity_lines(w),
    working_tree: working_tree_lines(w),
    recent_commits: recent_commits_lines(w, RECENT_COMMITS_LIMIT),
  }
}

/// Compact identity card for the Worktree block — `branch · head`,
/// `Created: <age>`, status + flag badges, tilde-compressed path. The
/// `●` status dot + bold name line is prepended live by `draw_sidebar`,
/// not cached here, so the dot can track GitHub fetch state without
/// invalidating the git-preview cache. Skips badges whose flags are
/// false to avoid visual noise.
fn worktree_identity_lines(w: &WorktreeInfo) -> Vec<Line<'static>> {
  let mut out: Vec<Line<'static>> = Vec::with_capacity(4);

  // Line 1 — "<branch> · <short head>". Branch colour follows the
  // lazygit scheme (PR #73): worst-state wins (dirty → red,
  // ahead/behind → yellow, unpublished → magenta, synced → green,
  // unknown → dark gray) so the most actionable signal stays at eye
  // level.
  let branch_color = branch_name_color(&w.status);
  let branch = w.branch.clone().unwrap_or_else(|| "-".into());
  let mut spans = vec![Span::styled(branch, Style::default().fg(branch_color))];
  if let Some(head) = w.head.as_deref() {
    spans.push(Span::styled("  ·  ".to_string(), Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled(short_oid(head), Style::default().fg(Color::Yellow)));
  }
  out.push(Line::from(spans));

  // Line 2 — "Created: <age>" (compact relative duration, colour-coded
  // by freshness — PR #73). Skipped when the branch has no measurable
  // age (trunk, detached HEAD, or repo open failure).
  out.push(Line::from(vec![
    Span::styled("Created: ".to_string(), Style::default().fg(Color::DarkGray)),
    Span::styled(branch_age_label(w), Style::default().fg(branch_age_color(w))),
  ]));

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
pub fn recent_commits_lines(w: &WorktreeInfo, limit: usize) -> Vec<Line<'static>> {
  match worktree::git_log_with_author(&w.path, limit) {
    Ok(rows) if !rows.is_empty() => {
      let graphs = super::commit_graph::render_commits(&rows);
      rows
        .into_iter()
        .zip(graphs)
        .map(|(row, graph_spans)| commit_row_line(row, graph_spans))
        .collect()
    }
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

fn commit_row_line(row: worktree::CommitRow, graph: Vec<Span<'static>>) -> Line<'static> {
  let short_hash: String = row.hash.chars().take(COMMIT_HASH_DISPLAY_LEN).collect();
  let initials = author_initials(&row.author);
  let mut spans: Vec<Span<'static>> = Vec::with_capacity(5 + graph.len());
  spans.push(Span::styled(short_hash, Style::default().fg(Color::Yellow)));
  spans.push(Span::raw("  "));
  spans.push(Span::styled(
    format!("{:<2}", initials),
    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
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

  // PR #74 follow-up: surface branch age right in the table so it stays
  // visible when the sidebar is hidden (<120 cols or `v` collapsed).
  // `branch_age_for` opens the worktree's repo and runs the libgit2
  // revwalk; per-frame cost is bounded by the number of visible rows
  // (typically <20) so we re-resolve on every draw without caching.
  // Colour stays uniform Gray — the saturated freshness palette
  // (green/yellow/darkgray) reads as noise next to the more important
  // BRANCH-status colour, so we keep it muted in the table and let the
  // sidebar's `Created:` row carry the colour-coded signal.
  let age = branch_age_for(w);
  let age_label = age.map(format_relative_duration_str).unwrap_or_else(|| "-".into());
  let age_cell = Cell::from(age_label).style(Style::default().fg(Color::DarkGray));

  let path_cell = Cell::from(w.path.to_string_lossy().to_string()).style(Style::default().fg(Color::Gray));

  Row::new(vec![
    age_cell,
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
    "enter:select esc:cancel o:open y:yank l:git_tui v:sidebar Tab:focus /:filter gg/G:top/bot j/k:nav f:refresh ?:help q:quit"
  } else {
    "n:new d:del b:boot o:open y:yank l:git_tui R:review v:sidebar Tab:focus /:filter gg/G:top/bot j/k:nav f:refresh F:gh ?:help q:quit"
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
    Line::from("  l             launch [git_tui] launcher (default lazygit -p, configurable)"),
    Line::from("  v             toggle git preview sidebar (auto-hidden < 120 cols)"),
    Line::from("  Tab           swap focus between worktree list and sidebar"),
    Line::from("  /             open fuzzy filter bar (enter: sticky, esc: clear)"),
    Line::from("  f             refresh worktree list"),
    Line::from("  F             refresh GitHub issue/PR status via `gh`"),
    Line::from("  R             run [review] launcher against the resolved base"),
  ]);
  if !app.picker_mode {
    lines.push(Line::from("  p             toggle 'delete branch on remove'"));
    lines.push(Line::from("  enter         show path in status bar"));
    lines.push(Line::from(""));
    lines.push(Line::from("issue / PR (#67)"));
    lines.push(Line::from("  O             open menu — i=issue · p=pull request"));
    lines.push(Line::from("  L             link prompt — i / p then digits"));
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

  let (type_str, type_desc) = app
    .branch_types
    .get(app.create_form.type_index)
    .map(|t| (t.name.as_str(), t.description.as_str()))
    .unwrap_or(("", "(no branch types configured)"));

  f.render_widget(
    field_input(
      "type (↑/↓)",
      &format!("{} — {}", type_str, type_desc),
      app.create_form.field == Field::Type,
    ),
    inner[0],
  );
  f.render_widget(
    field_input(
      "issue (digits)",
      &app.create_form.issue,
      app.create_form.field == Field::Issue,
    ),
    inner[1],
  );
  f.render_widget(
    field_input(
      "description (kebab)",
      &app.create_form.desc,
      app.create_form.field == Field::Desc,
    ),
    inner[2],
  );

  // Preview line
  let branch = format!("{}/#{}-{}", type_str, app.create_form.issue, app.create_form.desc);
  let dirname = format!("{}-{}-{}", type_str, app.create_form.issue, app.create_form.desc);
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
        if app.confirm.started_at.is_some() {
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
