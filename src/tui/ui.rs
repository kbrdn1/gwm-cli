use super::app::{App, Field, View};
use crate::bootstrap::StepStatus;
use crate::naming::BRANCH_TYPES;
use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span},
  widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
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
  let items: Vec<ListItem> = app
    .worktrees
    .iter()
    .map(|w| {
      let marker = if w.is_main { "★" } else { " " };
      let branch = w.branch.clone().unwrap_or_else(|| "-".into());
      let mut tags = String::new();
      if w.is_locked {
        tags.push_str(" [locked]");
      }
      if w.is_prunable {
        tags.push_str(" [prunable]");
      }
      let line = Line::from(vec![
        Span::styled(format!("{} ", marker), Style::default().fg(Color::Yellow)),
        Span::styled(
          format!("{:<28}", trunc(&w.name, 28)),
          Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
          format!(" {:<32}", trunc(&branch, 32)),
          Style::default().fg(Color::Green),
        ),
        Span::raw(format!(" {}", w.path.display())),
        Span::styled(tags, Style::default().fg(Color::Magenta)),
      ]);
      ListItem::new(line)
    })
    .collect();

  let list = List::new(items)
    .block(
      Block::default()
        .borders(Borders::ALL)
        .title(format!(" worktrees ({}) ", app.worktrees.len()))
        .border_style(Style::default().fg(Color::DarkGray)),
    )
    .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
    .highlight_symbol("▶ ");

  f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
  let help = "n:new  d:del  b:bootstrap  r:refresh  p:tog-branch  enter:path  ?:help  q:quit";
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
