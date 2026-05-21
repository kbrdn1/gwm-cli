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
use std::time::{Duration, Instant};

pub use app::{
  App, ConfirmKeyAction, CountdownTickOutcome, Field, GitHubFetchState, LinkPromptStage, LinkTarget, OpenTarget, View,
};

/// Ordered list of clipboard tools to try for the host OS (issue #73).
/// First entry that resolves on `$PATH` wins. Returned in the
/// platform's preferred order — `pbcopy` first on macOS, `wl-copy`
/// then `xclip` then `xsel` on Linux, `clip.exe` on Windows. Exposed
/// from the crate root so the tests in `tui_app_tests.rs` can pin the
/// non-empty contract without spawning anything.
pub fn clipboard_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
  if cfg!(target_os = "macos") {
    vec![("pbcopy", vec![])]
  } else if cfg!(target_os = "windows") {
    vec![("clip", vec![])]
  } else {
    vec![
      ("wl-copy", vec![]),
      ("xclip", vec!["-selection", "clipboard"]),
      ("xsel", vec!["--clipboard", "--input"]),
    ]
  }
}
pub use ui::{
  branch_name_color, filled_cells_for_progress, freshness_color, issue_badge_color, pr_badge_color, table_marker,
};

pub fn run() -> Result<()> {
  // Construct the App BEFORE touching the terminal: if discovery / config
  // load fails (e.g. not inside a git repo), the user's terminal stays in
  // its pristine cooked state. Addresses Copilot's PR #53 review — the
  // previous order left raw mode + alt-screen on when `App::new()?`
  // bubbled up.
  let app = App::new()?;
  let mut terminal = enter_terminal()?;
  let result = run_app(&mut terminal, app);
  leave_terminal(&mut terminal)?;
  result.map(|_| ())
}

/// `gwm switch` entry point: open the same TUI in picker mode and return
/// the user's pick (Some(path) on Enter, None on Esc / Ctrl-C / q).
///
/// Drives the terminal setup separately from `run` so the alternate screen
/// is always torn down before the caller prints the chosen path on stdout.
pub fn run_picker() -> Result<Option<PathBuf>> {
  // Same teardown-safety pattern as `run`: any error from
  // `App::new_picker_at` (repo discovery, config load) bubbles up with the
  // terminal still in cooked mode.
  let app = App::new_picker_at(None)?;
  let mut terminal = enter_terminal()?;
  let result = run_app(&mut terminal, app);
  leave_terminal(&mut terminal)?;
  result
}

/// Enable raw mode + alternate screen + mouse capture and hand back a
/// configured `Terminal`. Centralised so `run` and `run_picker` cannot
/// drift on the setup recipe.
fn enter_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
  enable_raw_mode()?;
  let mut stdout = io::stdout();
  execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
  Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

/// Inverse of `enter_terminal`. Always called from the same scope as
/// `enter_terminal` so the order of teardown matches the order of setup.
fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
  disable_raw_mode()?;
  execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
  terminal.show_cursor()?;
  Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> Result<Option<PathBuf>> {
  loop {
    terminal.draw(|f| ui::draw(f, &mut app))?;

    // Tick the confirm-overlay safety countdown (issue #30) before
    // polling for input. Driving it from the poll cadence keeps the UI
    // smooth (the 200ms poll already drives the redraw); doing it after
    // the keypress branch would skip a tick whenever a poll-timeout
    // doesn't fire a key event, stretching a 3s countdown by the
    // input-handling latency of every armed iteration.
    if app.view == View::Confirm {
      match app.tick_confirm_countdown(Instant::now()) {
        CountdownTickOutcome::ReadyToFire => {
          if let Err(e) = app.confirm_delete() {
            app.status = format!("delete failed: {}", e);
            app.view = View::List;
          }
        }
        CountdownTickOutcome::Pending | CountdownTickOutcome::NotArmed => {}
      }
    }

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
        KeyCode::Esc => {
          // Picker contract (footer `esc:cancel`): Esc inside the filter
          // bar quits the picker, it doesn't merely clear the filter.
          // Regular TUI keeps the two-step Esc (clear → quit) so a typo'd
          // filter doesn't accidentally close the long-lived session.
          if app.picker_mode {
            app.picker_cancel();
          } else {
            app.exit_filter_cancel();
          }
        }
        KeyCode::Enter => {
          // In picker mode (`gwm switch`), Enter doubles as "stop typing the
          // filter AND commit the highlighted pick". Exiting the filter bar
          // first lets `selected()` resolve against the narrowed set.
          app.exit_filter_keep();
          if app.picker_mode {
            app.picker_confirm();
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
          KeyCode::Char('y') => yank_selected_path_to_clipboard(&mut app),
          KeyCode::Char('o') => match app.resolve_open_target() {
            None => app.status = "nothing selected".into(),
            Some(OpenTarget::Finder { .. }) => app.open_selected_in_finder(),
            Some(OpenTarget::Shell { path, command }) => {
              run_subshell(terminal, &command, &[], Some(&path), &mut app, "shell")?
            }
            Some(OpenTarget::Editor { path, command }) => {
              let path_str = path.display().to_string();
              run_subshell(terminal, &command, &[&path_str], None, &mut app, "editor")?
            }
          },
          // Issue/PR linking (issue #67).
          KeyCode::Char('O') if !app.picker_mode => app.enter_open_menu(),
          KeyCode::Char('L') if !app.picker_mode => app.enter_link_prompt(),
          KeyCode::Char('R') if !app.picker_mode => app.refresh_github_status(),
          KeyCode::Enter => {
            if app.picker_mode {
              app.picker_confirm();
            } else {
              app.copy_path_to_status();
            }
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
        KeyCode::Char('y') | KeyCode::Enter => match app.confirm_press_y(Instant::now()) {
          ConfirmKeyAction::FireNow => match app.confirm_delete() {
            Ok(_) => {}
            Err(e) => app.status = format!("delete failed: {}", e),
          },
          // Armed / Disarmed update the App's status line; the loop
          // keeps the modal open and lets the countdown tick (or wait
          // for another y / Esc).
          ConfirmKeyAction::Armed | ConfirmKeyAction::Disarmed => {}
        },
        KeyCode::Char('n') | KeyCode::Esc => app.confirm_dismiss(),
        _ => {}
      },
      View::Report => match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
          app.view = View::List;
          app.refresh()?;
        }
        _ => {}
      },
      View::OpenMenu => match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.exit_open_menu(),
        KeyCode::Char('i') => {
          if let Some(url) = app.open_menu_pick(LinkTarget::Issue) {
            open_url(&url, &mut app);
          }
        }
        KeyCode::Char('p') => {
          if let Some(url) = app.open_menu_pick(LinkTarget::Pr) {
            open_url(&url, &mut app);
          }
        }
        _ => {}
      },
      View::LinkPrompt => match (app.link_prompt_stage(), key.code) {
        (_, KeyCode::Esc) => app.link_prompt_cancel(),
        (app::LinkPromptStage::ChooseTarget, KeyCode::Char('i')) => app.link_prompt_choose(LinkTarget::Issue),
        (app::LinkPromptStage::ChooseTarget, KeyCode::Char('p')) => app.link_prompt_choose(LinkTarget::Pr),
        (app::LinkPromptStage::InputNumber, KeyCode::Enter) => {
          if let Err(e) = app.link_prompt_submit() {
            app.status = format!("link failed: {}", e);
          }
        }
        (app::LinkPromptStage::InputNumber, KeyCode::Char(c)) => app.link_prompt_push_char(c),
        (app::LinkPromptStage::InputNumber, KeyCode::Backspace) => app.link_prompt_pop_char(),
        _ => {}
      },
    }

    // Picker contract (Copilot PR #53): only break when the App has
    // explicitly signalled exit — set by `picker_confirm` (only if a
    // worktree was actually selected) and `picker_cancel`. Replaces the
    // unconditional `break` after Enter that turned an empty-match
    // Enter into a surprise exit-1.
    if app.picker_should_exit {
      break;
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

/// Suspend the TUI, spawn `cmd args...` (optionally with `cwd`), wait for
/// its exit, then restore the TUI. Used by the `o: open` dispatch when the
/// resolved [`OpenTarget`] is `Shell` or `Editor`. The lifecycle is
/// identical to [`run_lazygit`] so the user can't observe a difference
/// between pressing `l` (lazygit) and pressing `o` with `mode = "shell"`.
///
/// `label` is the noun used in status-bar messages (`"shell"`, `"editor"`).
fn run_subshell(
  terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
  cmd: &str,
  args: &[&str],
  cwd: Option<&std::path::Path>,
  app: &mut App,
  label: &str,
) -> Result<()> {
  disable_raw_mode()?;
  execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
  terminal.show_cursor()?;

  let mut command = std::process::Command::new(cmd);
  command.args(args);
  if let Some(dir) = cwd {
    command.current_dir(dir);
  }
  let spawn = command.status();

  // Always restore the TUI, even if the child failed to spawn or exited non-zero.
  enable_raw_mode()?;
  execute!(terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
  terminal.clear()?;

  match spawn {
    Ok(s) if s.success() => app.status = format!("{} exited ok ({})", label, cmd),
    Ok(s) => app.status = format!("{} exited with code {:?}", label, s.code()),
    Err(e) => app.status = format!("failed to launch {} ({}): {}", label, cmd, e),
  }
  Ok(())
}

/// Push the selected worktree's path into the system clipboard via
/// [`clipboard_candidates`]. Walks the candidates in order, uses the
/// first one whose binary is on `$PATH`, and feeds the path through
/// its stdin. Failures and "no tool found" both surface in the status
/// bar — no propagation, the TUI must never die on a clipboard miss.
fn yank_selected_path_to_clipboard(app: &mut App) {
  use std::io::Write;
  let Some(path) = app.yank_selected_path() else {
    app.status = "nothing selected".into();
    return;
  };
  let text = path.display().to_string();
  for (cmd, args) in clipboard_candidates() {
    if which::which(cmd).is_err() {
      continue;
    }
    let child = std::process::Command::new(cmd)
      .args(&args)
      .stdin(std::process::Stdio::piped())
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .spawn();
    match child {
      Ok(mut c) => {
        if let Some(mut stdin) = c.stdin.take() {
          let _ = stdin.write_all(text.as_bytes());
        }
        match c.wait() {
          Ok(s) if s.success() => {
            app.status = format!("yanked path ({})", cmd);
            return;
          }
          Ok(s) => {
            app.status = format!("{} exited with code {:?}", cmd, s.code());
            return;
          }
          Err(e) => {
            app.status = format!("{} wait failed: {}", cmd, e);
            return;
          }
        }
      }
      Err(e) => {
        // Tool was resolvable on PATH but spawning failed — surface and stop;
        // trying the next candidate would mask the real error.
        app.status = format!("failed to spawn {}: {}", cmd, e);
        return;
      }
    }
  }
  app.status = "y: no clipboard tool found (install pbcopy / wl-copy / xclip / xsel / clip)".into();
}

/// Spawn the OS opener for `url` (used by the OpenMenu key handler).
/// Failures land in the status bar — we never propagate up.
fn open_url(url: &str, app: &mut App) {
  let opener = if cfg!(target_os = "macos") {
    "open"
  } else if cfg!(target_os = "windows") {
    "explorer"
  } else {
    "xdg-open"
  };
  match std::process::Command::new(opener).arg(url).spawn() {
    Ok(_) => app.status = format!("opened {}", url),
    Err(e) => app.status = format!("failed to open {}: {}", url, e),
  }
}
