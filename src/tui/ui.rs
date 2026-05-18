use super::app::{App, Field, View};
use crate::bootstrap::StepStatus;
use crate::naming::BRANCH_TYPES;
use crate::worktree::{BranchStatus, WorktreeInfo};
use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span},
  widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
  Frame,
};

pub fn draw(f: &mut Frame, app: &mut App) {
  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(2)])
    .split(f.area());

  draw_header(f, chunks[0], app);
  draw_list(f, chunks[1], app);
  draw_footer(f, chunks[2], app);

  match app.view {
    View::Help => draw_help(f),
    View::Create => draw_create(f, app),
    View::Confirm => draw_confirm(f, app),
    View::Report => draw_report(f, app),
    View::List => {}
  }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
  let title = format!(" gwm — {} ({}) ", app.repo_name, app.workdir.display());
  let p = Paragraph::new(Line::from(vec![Span::styled(
    title,
    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
  )]))
  .block(
    Block::default()
      .borders(Borders::ALL)
      .border_style(Style::default().fg(Color::DarkGray)),
  );
  f.render_widget(p, area);
}

fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
  // Dynamic column widths: derive from current data, capped to keep room for the path column.
  // The path column is always last and takes whatever remains.
  let name_w = column_width(app.worktrees.iter().map(|w| w.name.as_str()), 18, 38);
  let branch_w = column_width(app.worktrees.iter().map(|w| w.branch.as_deref().unwrap_or("-")), 18, 38);
  let status_w: u16 = 16;

  let header = Row::new(vec![
    Cell::from(""),
    Cell::from("NAME"),
    Cell::from("BRANCH"),
    Cell::from("STATUS"),
    Cell::from("PATH"),
  ])
  .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD));

  let rows: Vec<Row> = app
    .worktrees
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

  let table = Table::new(rows, widths)
    .header(header)
    .column_spacing(1)
    .block(
      Block::default()
        .borders(Borders::ALL)
        .title(format!(" worktrees ({}) ", app.worktrees.len()))
        .border_style(Style::default().fg(Color::DarkGray)),
    )
    .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
    .highlight_symbol("▶ ");

  f.render_stateful_widget(table, area, &mut app.list_state);
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
  let help = "n:new  d:del  b:bootstrap  o:open  r:refresh  p:tog-branch  enter:path  ?:help  q:quit";
  let text = Line::from(vec![
    Span::styled(help, Style::default().fg(Color::DarkGray)),
    Span::raw("  "),
    Span::styled(format!("[{}]", app.status), Style::default().fg(Color::Yellow)),
  ]);
  f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
}

fn draw_help(f: &mut Frame) {
  let area = centered(60, 60, f.area());
  let lines = vec![
    Line::from(Span::styled(
      "gwm — keys",
      Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )),
    Line::from(""),
    Line::from("global"),
    Line::from("  q / Esc       quit"),
    Line::from("  Ctrl-C        force quit"),
    Line::from(""),
    Line::from("list view"),
    Line::from("  j / ↓         next"),
    Line::from("  k / ↑         prev"),
    Line::from("  n             new worktree"),
    Line::from("  d             delete selected"),
    Line::from("  b             bootstrap selected"),
    Line::from("  o             open dir in OS file manager (open / xdg-open / explorer)"),
    Line::from("  r             refresh"),
    Line::from("  p             toggle 'delete branch on remove'"),
    Line::from("  enter         show path in status bar"),
    Line::from("  ?             this help"),
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
  ];
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
      lines.push(Line::from(Span::styled(
        "y/Enter: confirm    n/Esc: cancel",
        Style::default().fg(Color::DarkGray),
      )));
      lines
    }
    None => vec![Line::from("nothing selected")],
  };
  f.render_widget(Paragraph::new(body).block(block).wrap(Wrap { trim: false }), area);
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
