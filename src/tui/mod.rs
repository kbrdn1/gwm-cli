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
  App, ConfirmKeyAction, CountdownTickOutcome, Field, GitHubFetchState, LauncherPlan, LinkPromptStage, LinkTarget, View,
};
pub use ui::filled_cells_for_progress;

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
            // Issue #75: `l` is driven by the configurable `[git_tui]`
            // launcher pipeline (default `lazygit -p {path}`,
            // fullscreen=true). Replaces the old hardcoded lazygit call.
            if let Some(plan) = app.prepare_git_tui() {
              run_launcher(terminal, plan, &mut app)?;
            }
          }
          // Issue #75 keybinding reshuffle (was `r` → refresh worktree list).
          // `f` is the new mnemonic; the `r` alias is kept for muscle memory.
          KeyCode::Char('f') | KeyCode::Char('r') => app.refresh()?,
          // Mutating actions are inert in picker mode — the issue explicitly
          // calls for a stripped-down picker that only navigates and selects.
          KeyCode::Char('n') if !app.picker_mode => app.enter_create(),
          KeyCode::Char('d') if !app.picker_mode => app.enter_confirm_delete(),
          KeyCode::Char('b') if !app.picker_mode => app.bootstrap_selected(),
          KeyCode::Char('p') if !app.picker_mode => app.toggle_delete_branch(),
          KeyCode::Char('o') => app.open_selected_in_finder(),
          // Issue/PR linking (issue #67).
          KeyCode::Char('O') if !app.picker_mode => app.enter_open_menu(),
          KeyCode::Char('L') if !app.picker_mode => app.enter_link_prompt(),
          // Issue #75 reshuffle: `F` is the new mnemonic for "fetch GitHub
          // status" (was `R`). `R` now triggers the configured review tool.
          KeyCode::Char('F') if !app.picker_mode => app.refresh_github_status(),
          KeyCode::Char('R') if !app.picker_mode => {
            if let Some(plan) = app.prepare_review() {
              run_launcher(terminal, plan, &mut app)?;
            }
          }
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

/// Dispatch a [`LauncherPlan`] from [`App::prepare_git_tui`] /
/// [`App::prepare_review`]. When `fullscreen=true` the TUI is
/// suspended (raw mode off, alt-screen left) for the call and restored
/// on exit — same recipe as the previous hardcoded `lazygit` flow.
/// Non-fullscreen launchers run in the background; their stderr's
/// first line is surfaced on the status bar so the user gets feedback
/// even when the tool doesn't take over the screen.
///
/// `LauncherPlan` is consumed by-value so the `{diff}` tempfile it
/// carries lives at least until the child process has been waited on.
/// Errors are never propagated — the user pressed a key in the TUI,
/// and surfacing failures via the status bar is the documented
/// contract (see [`Self::run_lazygit`] in the pre-issue-#75 codebase).
fn run_launcher(
  terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
  plan: app::LauncherPlan,
  app: &mut App,
) -> Result<()> {
  use std::process::{Command, Stdio};

  let argv = plan.expanded.argv.clone();
  let Some((bin, rest)) = argv.split_first() else {
    app.status = "launcher template produced an empty argv".into();
    return Ok(());
  };

  // Probe `$PATH` before paying the suspend/restore tax. Missing
  // binaries get a clean status-bar error instead of a flicker.
  if which::which(bin).is_err() {
    app.status = format!(
      "`{}` not on $PATH — install it or change [review]/[git_tui] in .gwm.toml",
      bin
    );
    return Ok(());
  }

  if plan.fullscreen {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    let spawn = Command::new(bin).args(rest).current_dir(&plan.cwd).status();

    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal.clear()?;

    match spawn {
      Ok(s) if s.success() => app.status = format!("{} exited ok", bin),
      Ok(s) => app.status = format!("{} exited with code {:?}", bin, s.code()),
      Err(e) => app.status = format!("failed to launch {}: {}", bin, e),
    }
  } else {
    // Non-TUI tool: capture stderr so its first line can land in the
    // status bar without taking over the screen. stdout is dropped on
    // the floor — printing it would crash through ratatui's frame.
    let out = Command::new(bin)
      .args(rest)
      .current_dir(&plan.cwd)
      .stdout(Stdio::null())
      .stderr(Stdio::piped())
      .output();
    match out {
      Ok(o) if o.status.success() => app.status = format!("{} done", bin),
      Ok(o) => {
        let first = String::from_utf8_lossy(&o.stderr)
          .lines()
          .next()
          .unwrap_or_default()
          .trim()
          .to_string();
        app.status = if first.is_empty() {
          format!("{} exited with code {:?}", bin, o.status.code())
        } else {
          format!("{}: {}", bin, first)
        };
      }
      Err(e) => app.status = format!("failed to launch {}: {}", bin, e),
    }
  }
  // `plan.expanded.diff_file` drops here, unlinking the tempfile if any.
  drop(plan);
  Ok(())
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
