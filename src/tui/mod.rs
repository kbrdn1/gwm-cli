mod app;
mod ui;

use crate::error::Result;
use crossterm::{
  event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
  execute,
  terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::Duration;

pub use app::{App, Field, View};

pub fn run() -> Result<()> {
  enable_raw_mode()?;
  let mut stdout = io::stdout();
  execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
  let backend = CrosstermBackend::new(stdout);
  let mut terminal = Terminal::new(backend)?;

  let result = run_app(&mut terminal);

  disable_raw_mode()?;
  execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
  terminal.show_cursor()?;

  result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
  let mut app = App::new()?;
  loop {
    terminal.draw(|f| ui::draw(f, &mut app))?;

    if !event::poll(Duration::from_millis(200))? {
      continue;
    }
    let Event::Key(key) = event::read()? else { continue };
    if key.kind != KeyEventKind::Press {
      continue;
    }

    // Global keys
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
      break;
    }

    match app.view {
      View::List => {
        // Two-keystroke vim motion `gg`: any non-'g' keypress disarms it.
        if !matches!(key.code, KeyCode::Char('g')) {
          app.cancel_pending_motion();
        }
        match key.code {
          KeyCode::Char('q') | KeyCode::Esc => break,
          KeyCode::Char('?') => app.view = View::Help,
          KeyCode::Char('j') | KeyCode::Down => app.next(),
          KeyCode::Char('k') | KeyCode::Up => app.prev(),
          KeyCode::Char('g') => app.handle_g(),
          KeyCode::Char('G') => app.last(),
          KeyCode::Char('v') => app.toggle_sidebar(),
          KeyCode::Tab => app.toggle_focus(),
          KeyCode::Char('l') => {
            if let Some(path) = app.launch_lazygit() {
              run_lazygit(terminal, &path, &mut app)?;
            }
          }
          KeyCode::Char('r') => app.refresh()?,
          KeyCode::Char('n') => app.enter_create(),
          KeyCode::Char('d') => app.enter_confirm_delete(),
          KeyCode::Char('b') => app.bootstrap_selected(),
          KeyCode::Char('p') => app.toggle_delete_branch(),
          KeyCode::Char('o') => app.open_selected_in_finder(),
          KeyCode::Enter => app.copy_path_to_status(),
          _ => {}
        }
      }
      View::Help => match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => app.view = View::List,
        _ => {}
      },
      View::Create => match key.code {
        KeyCode::Esc => app.view = View::List,
        KeyCode::Tab => app.create_next_field(),
        KeyCode::BackTab => app.create_prev_field(),
        KeyCode::Enter => {
          if app.create_field == Field::Desc {
            if let Err(e) = app.submit_create() {
              app.status = format!("error: {}", e);
            }
          } else {
            app.create_next_field();
          }
        }
        KeyCode::Up if app.create_field == Field::Type => app.create_prev_type(),
        KeyCode::Down if app.create_field == Field::Type => app.create_next_type(),
        KeyCode::Char(c) if app.create_field != Field::Type => app.create_push_char(c),
        KeyCode::Backspace if app.create_field != Field::Type => app.create_pop_char(),
        _ => {}
      },
      View::Confirm => match key.code {
        KeyCode::Char('y') | KeyCode::Enter => match app.confirm_delete() {
          Ok(_) => {}
          Err(e) => app.status = format!("delete failed: {}", e),
        },
        KeyCode::Char('n') | KeyCode::Esc => app.view = View::List,
        _ => {}
      },
      View::Report => match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
          app.view = View::List;
          app.refresh()?;
        }
        _ => {}
      },
    }
  }
  Ok(())
}

/// Suspend the TUI, run `lazygit -p <path>` inheriting the terminal, then restore.
/// Errors from lazygit itself are surfaced via the status bar, never propagated up.
fn run_lazygit(
  terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
  path: &std::path::Path,
  app: &mut App,
) -> Result<()> {
  // Release the terminal so lazygit can take over.
  disable_raw_mode()?;
  execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
  terminal.show_cursor()?;

  let spawn = std::process::Command::new("lazygit").arg("-p").arg(path).status();

  // Always restore the TUI, even if lazygit failed.
  enable_raw_mode()?;
  execute!(terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
  terminal.clear()?;

  match spawn {
    Ok(s) if s.success() => app.status = format!("lazygit exited ok ({})", path.display()),
    Ok(s) => app.status = format!("lazygit exited with code {:?}", s.code()),
    Err(e) => app.status = format!("failed to launch lazygit: {}", e),
  }
  Ok(())
}
