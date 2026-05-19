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
use std::path::PathBuf;
use std::time::Duration;

pub use app::{App, Field, View};

pub fn run() -> Result<()> {
  enable_raw_mode()?;
  let mut stdout = io::stdout();
  execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
  let backend = CrosstermBackend::new(stdout);
  let mut terminal = Terminal::new(backend)?;

  let result = run_app(&mut terminal, App::new()?);

  disable_raw_mode()?;
  execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
  terminal.show_cursor()?;

  result.map(|_| ())
}

/// `gwm switch` entry point: open the same TUI in picker mode and return
/// the user's pick (Some(path) on Enter, None on Esc / Ctrl-C / q).
///
/// Drives the terminal setup separately from `run` so the alternate screen
/// is always torn down before the caller prints the chosen path on stdout.
pub fn run_picker() -> Result<Option<PathBuf>> {
  enable_raw_mode()?;
  let mut stdout = io::stdout();
  execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
  let backend = CrosstermBackend::new(stdout);
  let mut terminal = Terminal::new(backend)?;

  let app = App::new_picker_at(None)?;
  let result = run_app(&mut terminal, app);

  disable_raw_mode()?;
  execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
  terminal.show_cursor()?;

  result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> Result<Option<PathBuf>> {
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
      // When the inline filter bar is open, capture every key as filter input
      // so the user can type a query containing `q`, `?`, `/`, etc. The only
      // ways out are Enter (sticky filter) or Esc (clear filter).
      View::List if app.filter_active => match key.code {
        KeyCode::Esc => app.exit_filter_cancel(),
        KeyCode::Enter => {
          // In picker mode (`gwm switch`), Enter doubles as "stop typing the
          // filter AND commit the highlighted pick". Exiting the filter bar
          // first lets `selected()` resolve against the narrowed set.
          app.exit_filter_keep();
          if app.picker_mode {
            app.picker_confirm();
            break;
          }
        }
        KeyCode::Backspace => app.filter_pop_char(),
        KeyCode::Char(c) => app.filter_push_char(c),
        _ => {}
      },
      View::List => {
        // Two-keystroke vim motion `gg`: any non-'g' keypress disarms it.
        if !matches!(key.code, KeyCode::Char('g')) {
          app.cancel_pending_motion();
        }
        match key.code {
          KeyCode::Char('q') => break,
          // Esc on the list clears a sticky filter first, then quits if the
          // list is already in its plain state. Avoids the trap where a user
          // hits Esc expecting to clear /-filter and accidentally exits.
          KeyCode::Esc => {
            if !app.filter_query.is_empty() {
              app.exit_filter_cancel();
            } else {
              break;
            }
          }
          KeyCode::Char('?') => app.view = View::Help,
          KeyCode::Char('j') | KeyCode::Down => app.next(),
          KeyCode::Char('k') | KeyCode::Up => app.prev(),
          KeyCode::Char('g') => app.handle_g(),
          KeyCode::Char('G') => app.last(),
          KeyCode::Char('v') => app.toggle_sidebar(),
          KeyCode::Tab => app.toggle_focus(),
          KeyCode::Char('/') => app.enter_filter(),
          KeyCode::Char('l') => {
            if let Some(path) = app.launch_lazygit() {
              run_lazygit(terminal, &path, &mut app)?;
            }
          }
          KeyCode::Char('r') => app.refresh()?,
          // Mutating actions are inert in picker mode — the issue explicitly
          // calls for a stripped-down picker that only navigates and selects.
          KeyCode::Char('n') if !app.picker_mode => app.enter_create(),
          KeyCode::Char('d') if !app.picker_mode => app.enter_confirm_delete(),
          KeyCode::Char('b') if !app.picker_mode => app.bootstrap_selected(),
          KeyCode::Char('p') if !app.picker_mode => app.toggle_delete_branch(),
          KeyCode::Char('o') => app.open_selected_in_finder(),
          KeyCode::Enter => {
            if app.picker_mode {
              app.picker_confirm();
              break;
            }
            app.copy_path_to_status();
          }
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
  Ok(app.picker_result)
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
