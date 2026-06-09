mod app;
/// Commit-graph topology renderer, ported from lazygit. **Not part of the
/// public SemVer surface** — exposed only so the integration tests under
/// `tests/` can pin the algorithm. Use `gwm::tui::recent_commits_lines`
/// (re-exported below) for the stable entry point that callers should
/// actually depend on.
#[doc(hidden)]
pub mod commit_graph;
pub mod keymap;
pub mod palette;
pub mod state;
pub mod theme;
mod ui;

use crate::error::Result;
use crate::tui::keymap::Action;
use crossterm::{
  event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
  execute,
  terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub use app::{App, CreateKey, LauncherPlan, LinkPromptKey, LinkPromptStage, LinkTarget, OpenTarget, View};
pub use state::async_task::{CreateWorktreeResult, TaskKind, TaskMsg, TaskRunner};
pub use state::command_logs::CommandLogs;
pub use state::config_panel::{ConfigPanel, FieldKind, SettingField, SettingsLayer, SettingsTab};
pub use state::confirm::{ConfirmButton, ConfirmKeyAction, ConfirmModal, CountdownTickOutcome};
pub use state::create_form::{CreateForm, Field};
pub use state::filter::FilterState;
pub use state::github_fetch::{FetchKey, GitHubFetch, GitHubFetchState};
pub use state::link_prompt::LinkPrompt;
pub use state::sidebar::SidebarState;

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
  author_initials, badge_group_width, bootstrap_report_lines, branch_name_color, branch_status_color,
  build_sidebar_sections, centered_abs, chip_style, confirm_buttons_line, confirm_delete_branch_line,
  confirm_detail_line, create_buttons_line, delete_worktree_title, ellipsize_middle, field_input_line,
  filled_cells_for_progress, footer_line, format_status, freshness_color, github_status_lines, header_line,
  help_body_section_color, help_entry_line, help_label_style, help_lines, help_rows, help_section_style,
  hint_key_style, hint_label_style, issue_badge_color, issue_pr_pane_title, issue_summary_line, link_open_modal_lines,
  link_prompt_modal_width, link_target_line, modal_hint_line, palette_name_style, pane_counter, panel_border_color,
  pr_badge_color, pr_summary_line, recent_commits_lines, recent_items_pane_title, status_line, status_pane_title,
  table_marker, tilde_compress_with_home, type_selector_line, working_tree_pane_title, working_tree_status_line,
  worktree_name_style, worktree_path_style, worktrees_pane_title, HelpRow, HintContext, SidebarSections,
  COMMIT_HASH_DISPLAY_LEN, ISSUE_ICON, PR_ICON, RECENT_COMMITS_LIMIT,
};

/// The single TUI render entry point. **Not part of the public SemVer
/// surface** — exposed only so the modal render net in `tests/` (issue
/// #235) can drive each overlay through the same `draw` path the event
/// loop uses, pinning modal layout against future `ui.rs` refactors. The
/// per-modal `draw_*` helpers stay private; this mirrors the
/// `#[doc(hidden)] pub mod commit_graph` convention above.
#[doc(hidden)]
pub use ui::draw;

pub fn run(trust_mode: crate::trust::TrustMode) -> Result<()> {
  // Construct the App BEFORE touching the terminal: if discovery / config
  // load fails (e.g. not inside a git repo), the user's terminal stays in
  // its pristine cooked state. Addresses Copilot's PR #53 review — the
  // previous order left raw mode + alt-screen on when `App::new()?`
  // bubbled up.
  //
  // `trust_mode` is threaded down so the TUI's bootstrap call sites
  // (`submit_create`, `bootstrap_selected`) take the same TOFU
  // decision as `gwm create` / `gwm bootstrap` — closes the bypass
  // flagged in PR #113 review (issue #95).
  let app = App::new()?.with_trust_mode(trust_mode);
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

/// Confirm-side of the delete modal: arm/fire the destructive action.
/// Shared by the `y` shortcut and the `Enter`-on-`[ Confirm ]` path so
/// the arm-then-fire countdown semantics stay identical (#187). In
/// countdown mode the first call arms (the loop ticks the bar), a second
/// disarms; in classic mode it fires immediately.
fn confirm_fire(app: &mut App) {
  if app.is_delete_worktree_loading() {
    app.status = TaskKind::DeleteWorktree.loading_label().into();
    return;
  }
  match app.confirm_press_y(Instant::now()) {
    ConfirmKeyAction::FireNow => {
      if let Err(e) = app.confirm_delete() {
        app.status = format!("delete failed: {}", e);
      }
    }
    // Armed / Disarmed update the status line; the loop keeps the modal
    // open and lets the countdown tick (or wait for another y / Esc).
    ConfirmKeyAction::Armed | ConfirmKeyAction::Disarmed => {}
  }
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> Result<Option<PathBuf>> {
  loop {
    let now = Instant::now();
    // Generic off-thread tasks (issue #231; GitHub fetch folded in by #255):
    // apply any worker results that landed since the last tick — the
    // off-thread worktree refresh and the `gh issue/pr view` fetches all
    // report over this one channel now. Drained before the draw so the frame
    // reflects the freshly-applied results, and the loader animates below
    // while any of them is still in flight (200ms poll cadence).
    app.drain_task_results();
    if app.is_github_loading() || app.is_task_loading() {
      app.spinner.tick();
    }

    if app.should_quit {
      if app.can_quit_now() {
        break;
      }
      app.defer_quit_for_mutating_task();
    }

    // Keep the Command Logs overlay live (issue #226): re-snapshot the
    // global log each tick while it is open so a command that finishes
    // off-thread (e.g. the GitHub fetch) appears without reopening. The
    // scroll cursor is preserved; the renderer re-clamps it.
    if app.view == View::CommandLogs {
      app.command_logs.sync();
    }
    app.maybe_auto_refresh(now);

    terminal.draw(|f| ui::draw(f, &mut app))?;

    // Tick the confirm-overlay safety countdown (issue #30) before
    // polling for input. Driving it from the poll cadence keeps the UI
    // smooth (the 200ms poll already drives the redraw); doing it after
    // the keypress branch would skip a tick whenever a poll-timeout
    // doesn't fire a key event, stretching a 3s countdown by the
    // input-handling latency of every armed iteration.
    if app.view == View::Confirm {
      // Advance the loader animation while the safety countdown is
      // armed (#187). The 200ms poll re-enters this block every tick,
      // so the spinner animates at the poll cadence; when idle (no
      // countdown) the frame stays put.
      if app.confirm.is_armed() {
        app.spinner.tick();
      }
      match app.tick_confirm_countdown(now) {
        CountdownTickOutcome::ReadyToFire => {
          if let Err(e) = app.confirm_delete() {
            app.status = format!("delete failed: {}", e);
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
      app.should_quit = true;
      if app.can_quit_now() {
        break;
      }
      app.defer_quit_for_mutating_task();
      continue;
    }
    if app.should_quit {
      continue;
    }

    match app.view {
      // When the inline filter bar is open, capture every key as filter input
      // so the user can type a query containing `q`, `?`, `/`, etc. The only
      // ways out are Enter (sticky filter) or Esc (clear filter).
      View::List if app.filter.active => match key.code {
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
        // Esc and Enter stay hard-coded because their semantics are
        // *contextual* (filter state, picker mode, sticky filter) —
        // folding them into the user-rebindable keymap would require
        // a modal grammar the config language cannot express. Their
        // pre-#87 behaviour is preserved verbatim; both also drop
        // any in-flight chord so a stray `g` followed by Esc never
        // leaks state into the next view.
        if key.code == KeyCode::Esc {
          app.cancel_pending_motion();
          if !app.filter.query().is_empty() {
            app.exit_filter_cancel();
          } else {
            app.should_quit = true;
          }
        } else if key.code == KeyCode::Enter {
          app.cancel_pending_motion();
          if app.picker_mode {
            app.picker_confirm();
          } else {
            app.copy_path_to_status();
          }
        } else if let Some(action) = app.dispatch_key(key) {
          // Issue #87: the View::List binding table is driven by the
          // resolved keymap. Routed through `run_action` so the
          // palette overlay (issue #32) and the key path stay
          // observationally identical: both call the same dispatch.
          if matches!(action, Action::Quit) {
            app.should_quit = true;
          } else {
            run_action(terminal, &mut app, action)?;
          }
        }
      }
      View::Help => match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => app.view = View::List,
        // Scroll the Keybindings overlay when it outgrows the modal (#217).
        KeyCode::Down | KeyCode::Char('j') => app.help_scroll_down(),
        KeyCode::Up | KeyCode::Char('k') => app.help_scroll_up(),
        KeyCode::Right | KeyCode::Char('l') => app.help_scroll_right(),
        KeyCode::Left | KeyCode::Char('h') => app.help_scroll_left(),
        KeyCode::Home | KeyCode::Char('g') => app.help_scroll = 0,
        KeyCode::End | KeyCode::Char('G') => app.help_scroll = app.help_max_scroll,
        _ => {}
      },
      // Command Logs overlay (issue #226). Scrolls like the help overlay;
      // closes on Esc / `q` or the bound `command_logs` key (default `3`)
      // so the open key toggles it shut even when rebound.
      View::CommandLogs => match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.view = View::List,
        // `y` copies the whole transcript to the clipboard (issue #279).
        KeyCode::Char('y') => copy_command_logs_to_clipboard(&mut app),
        KeyCode::Down | KeyCode::Char('j') => app.command_logs.scroll_down(),
        KeyCode::Up | KeyCode::Char('k') => app.command_logs.scroll_up(),
        KeyCode::Right | KeyCode::Char('l') => app.command_logs.scroll_right(),
        KeyCode::Left | KeyCode::Char('h') => app.command_logs.scroll_left(),
        KeyCode::Home | KeyCode::Char('g') => app.command_logs.scroll_to_top(),
        KeyCode::End | KeyCode::Char('G') => app.command_logs.scroll_to_bottom(),
        _ if app.key_matches_action(key, Action::CommandLogs) => app.view = View::List,
        _ => {}
      },
      // Settings panel (issue #232; editable in #279). While a numeric input
      // is armed, keystrokes route to the edit buffer and only Enter / Esc
      // escape — so `q` / `j` / Tab while typing a countdown never quit or
      // navigate. Otherwise: Tab/BackTab switch category tabs, `L` flips the
      // edit layer, Up/Down select fields (or scroll on the read-only `All`
      // tab), Space/Enter cycle a choice or open the numeric input, and
      // Esc / `q` / the bound `config_panel` key (default `4`) close.
      View::Config if app.config_panel.editing.is_some() => match key.code {
        KeyCode::Enter => app.commit_settings_edit(),
        KeyCode::Esc => app.config_panel.cancel_edit(),
        KeyCode::Backspace => app.config_panel.pop_edit_char(),
        KeyCode::Char(c) => app.config_panel.push_edit_char(c),
        _ => {}
      },
      View::Config => match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.view = View::List,
        KeyCode::Tab => app.config_panel.next_tab(),
        KeyCode::BackTab => app.config_panel.prev_tab(),
        KeyCode::Char('L') => app.config_panel.toggle_layer(),
        KeyCode::Char(' ') | KeyCode::Enter => app.activate_selected_setting(),
        KeyCode::Down | KeyCode::Char('j') => {
          if app.config_panel.tab == SettingsTab::All {
            app.config_panel.scroll_down();
          } else {
            app.config_panel.select_next();
          }
        }
        KeyCode::Up | KeyCode::Char('k') => {
          if app.config_panel.tab == SettingsTab::All {
            app.config_panel.scroll_up();
          } else {
            app.config_panel.select_prev();
          }
        }
        // Horizontal pan + jump only matter on the long read-only `All` tab.
        KeyCode::Right | KeyCode::Char('l') if app.config_panel.tab == SettingsTab::All => {
          app.config_panel.scroll_right()
        }
        KeyCode::Left | KeyCode::Char('h') if app.config_panel.tab == SettingsTab::All => {
          app.config_panel.scroll_left()
        }
        KeyCode::Home | KeyCode::Char('g') if app.config_panel.tab == SettingsTab::All => {
          app.config_panel.scroll_to_top()
        }
        KeyCode::End | KeyCode::Char('G') if app.config_panel.tab == SettingsTab::All => {
          app.config_panel.scroll_to_bottom()
        }
        _ if app.key_matches_action(key, Action::ConfigPanel) => app.view = View::List,
        _ => {}
      },
      // Create-overlay keys live in a testable `App` method (issue #217);
      // the loop only owns the two side effects (submit / close). While the
      // async create worker is in flight (#276), keep the modal locked so a
      // second submit/cancel does not race the mutating operation.
      View::Create if app.is_create_worktree_loading() => {}
      View::Create => match app.handle_create_key(key) {
        CreateKey::Submit => {
          if let Err(e) = app.submit_create() {
            app.status = format!("error: {}", e);
          }
        }
        CreateKey::Cancel => app.view = View::List,
        CreateKey::Handled => {}
      },
      View::Confirm if app.is_delete_worktree_loading() => {}
      View::Confirm => match key.code {
        // `y` confirms directly regardless of which button is focused
        // (unchanged muscle memory). `Enter` activates the *focused*
        // button — and focus defaults to Cancel (#187), so a stray
        // Enter on a freshly-opened modal cancels rather than deletes.
        KeyCode::Char('y') => confirm_fire(&mut app),
        KeyCode::Enter => match app.confirm.focused_button() {
          ConfirmButton::Confirm => confirm_fire(&mut app),
          ConfirmButton::Cancel => app.confirm_dismiss(),
        },
        KeyCode::Char('n') | KeyCode::Esc => app.confirm_dismiss(),
        _ if app.key_matches_action(key, Action::ToggleDeleteBranch) => app.toggle_delete_branch(),
        // Button focus navigation (#187). `←` / `h` → Confirm,
        // `→` / `l` → Cancel, `Tab` toggles.
        KeyCode::Left | KeyCode::Char('h') => app.confirm.focus_confirm(),
        KeyCode::Right | KeyCode::Char('l') => app.confirm.focus_cancel(),
        KeyCode::Tab => app.confirm.toggle_focus(),
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
        KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Down | KeyCode::Up => app.open_menu_toggle_selection(),
        KeyCode::Enter => {
          if let Some(url) = app.open_menu_pick(app.open_menu_selected) {
            open_url(&url, &mut app);
          }
        }
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
        _ if app.key_matches_action(key, Action::FetchGithub) => app.refresh_github_status(),
        _ => {}
      },
      // Link-prompt keys live in a testable `App` method (issue #217); the
      // loop only owns the two side effects (submit shell-out / close).
      View::LinkPrompt => match app.handle_link_prompt_key(key) {
        LinkPromptKey::Submit => {
          if let Err(e) = app.link_prompt_submit() {
            app.status = format!("link failed: {}", e);
          }
        }
        LinkPromptKey::Refresh => app.refresh_github_status(),
        LinkPromptKey::Cancel => app.link_prompt_cancel(),
        LinkPromptKey::Handled => {}
      },
      // Issue #32: command palette overlay. Palette entry names
      // are restricted to `[a-z0-9_-]` (see
      // `tests/palette_tests.rs::registry_names_are_unique_and_lowercase_words`),
      // so only those characters can usefully reach the buffer —
      // any other typed character would just shrink the match set
      // to empty. The accepted-character set is enforced explicitly
      // here so a stray `:` (the palette's own trigger) doesn't
      // self-append, and so future overlays (themes / fuzzy
      // search) that share the input bar don't inherit a "swallow
      // everything" contract by accident. Esc / Enter / arrows /
      // Tab still exit or navigate; Backspace edits.
      View::CommandPalette => match key.code {
        KeyCode::Esc => app.close_command_palette(),
        KeyCode::Enter => {
          if let Some(action) = app.accept_command_palette() {
            run_palette_action(terminal, &mut app, action)?;
          }
        }
        KeyCode::Up => app.palette_cycle_up(),
        KeyCode::Down | KeyCode::Tab => app.palette_cycle_down(),
        KeyCode::Backspace => app.palette_pop_char(),
        KeyCode::Char(c) if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' => {
          app.palette_push_char(c);
        }
        // Any other char (including the palette trigger `:`, the
        // help glyph `?`, uppercase letters) is dropped — there is
        // no palette entry name that could match it. Silently
        // ignoring is friendlier than appending and producing zero
        // matches with no explanation.
        KeyCode::Char(_) => {}
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
    // Issue #32/#267: every quit path raises this flag, then the loop
    // exits only once in-flight mutating workers have reported back. Read-
    // only workers may be abandoned immediately.
    if app.should_quit {
      if app.can_quit_now() {
        break;
      }
      app.defer_quit_for_mutating_task();
    }
  }
  Ok(app.picker_result)
}

/// Dispatch a [`LauncherPlan`] from [`App::prepare_git_tui`] /
/// [`App::prepare_review`]. When `fullscreen=true` the TUI is
/// suspended (raw mode off, alt-screen left) for the call and restored
/// on exit — same recipe as the previous hardcoded `lazygit` flow.
///
/// **Non-fullscreen launchers also run synchronously**: gwm stays in
/// the alt-screen, `Command::output()` waits for the child to exit,
/// then the first line of its stderr lands on the status bar. The
/// TUI is therefore unresponsive until the tool returns — fine for
/// print-only AI reviewers (`claude --print`, `gh pr view --web`)
/// that terminate quickly, but a long-running tool will visibly
/// block. Pick `fullscreen = true` (proper suspend/resume) for
/// anything that's not a quick one-shot. Caught by Copilot's review
/// on PR #76; the previous docstring claimed "run in the background"
/// which `output()` does not.
///
/// Apply a resolved `Action` (issue #87 dispatch) to `App`.
///
/// Centralised so the keystroke path (`View::List` → `dispatch_key`)
/// and the command-palette path (issue #32: `View::CommandPalette` →
/// `accept_command_palette`) fire identical side effects. Without
/// this single funnel the two surfaces would inevitably drift: a
/// future feature wired into one would silently miss the other.
///
/// `Action::Quit` raises `app.should_quit` so the event loop can
/// honour it from any caller and defer the actual exit while a mutating
/// worker is still in flight. The loop checks the flag at the top and
/// bottom of every iteration.
fn run_action(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App, action: Action) -> Result<()> {
  match action {
    // Issue #32/#267: signal quit via `app.should_quit` so palette
    // and keymap paths share the same graceful-shutdown gate.
    Action::Quit => app.should_quit = true,
    Action::Down => app.next(),
    Action::Up => app.prev(),
    Action::Top => app.first(),
    Action::Bottom => app.last(),
    Action::ToggleSidebar => app.toggle_sidebar(),
    // Issue #34: cycle the sidebar preview between commits and
    // stashes. Lands here as the merge resolution between #166
    // (which added the action) and #167 (which extracted run_action).
    Action::ToggleSidebarMode => app.cycle_sidebar_mode(),
    // Issue #188: responsive sidebar layout — cycle orientation and
    // flip the side-by-side position.
    Action::CycleSidebarLayout => app.cycle_sidebar_layout(),
    Action::ToggleSidebarPosition => app.toggle_sidebar_position(),
    Action::FocusSwap => app.toggle_focus(),
    Action::FocusWorktrees => app.focus_worktrees(),
    Action::FocusStatus => app.focus_status(),
    Action::Filter => app.enter_filter(),
    // Issue #231: the user-initiated refresh runs off-thread so a large
    // repo / slow filesystem no longer freezes the TUI. A failed re-list
    // now surfaces on the status bar instead of tearing down the loop.
    Action::Refresh => app.request_refresh(),
    Action::Help => app.enter_help(),
    Action::Yank => yank_selected_path_to_clipboard(app),
    Action::Open => match app.resolve_open_target() {
      None => app.status = "nothing selected".into(),
      Some(OpenTarget::Finder { .. }) => app.open_selected_in_finder(),
      Some(OpenTarget::Shell { path, command }) => run_subshell(terminal, &command, &[], Some(&path), app, "shell")?,
      Some(OpenTarget::Editor { path, command }) => {
        let path_str = path.display().to_string();
        run_subshell(terminal, &command, &[&path_str], None, app, "editor")?
      }
    },
    Action::GitTui => {
      if let Some(plan) = app.prepare_git_tui() {
        run_launcher(terminal, plan, app)?;
      }
    }
    Action::Create if !app.picker_mode => app.enter_create(),
    Action::DeleteConfirm if !app.picker_mode => app.enter_confirm_delete(),
    Action::Bootstrap if !app.picker_mode => app.bootstrap_selected(),
    // Issue #258: `gwm sync` of the selected worktree, off-thread on the
    // spine. Mutating, so disabled in picker mode like create / delete.
    Action::Sync if !app.picker_mode => app.request_sync(),
    Action::ToggleDeleteBranch if !app.picker_mode => app.toggle_delete_branch(),
    Action::OpenMenu if !app.picker_mode => app.enter_open_menu(),
    // Read-only and selection-independent, like `open` / `yank` / `git_tui`
    // — not picker-gated, so `gwm switch` can open the docs too (issue #233,
    // Codex review on #268). Gating it would silently no-op a key the help
    // overlay advertises in picker mode.
    Action::OpenDocs => open_url(DOCS_URL, app),
    Action::LinkPrompt if !app.picker_mode => app.enter_link_prompt(),
    Action::FetchGithub if !app.picker_mode => app.refresh_github_status(),
    Action::Review if !app.picker_mode => {
      if let Some(plan) = app.prepare_review() {
        run_launcher(terminal, plan, app)?;
      }
    }
    // Issue #32: pressing `:` (or any user-rebound key for
    // `Action::CommandPalette`) opens the palette overlay. Inside
    // the palette, the user can type `:command-palette` to reopen
    // it — harmless, but explicitly handled here so the palette →
    // CommandPalette → run_action loop terminates cleanly (the
    // overlay just stays open).
    Action::CommandPalette => app.open_command_palette(),
    // Issue #226: `3` opens the Command Logs overlay. Not picker-gated —
    // it is a read-only transcript, harmless inside `gwm switch`, and
    // mirrors Help / the palette which also open from any List state.
    Action::CommandLogs => app.enter_command_logs(),
    // Issue #232: `4` opens the Configuration panel. Like the Command Logs
    // overlay it is read-only and not picker-gated — harmless inside
    // `gwm switch`, opening from any List state.
    Action::ConfigPanel => app.enter_config_panel(),
    // Picker-mode-gated actions fall through to no-op when the
    // guard fails (i.e. the user pressed them inside `gwm switch`).
    // Same fallthrough catches future actions not yet wired into
    // the List view.
    _ => {}
  }
  Ok(())
}

/// Dispatch an action accepted from the command palette (issue #32).
/// Thin wrapper around [`run_action`] so the call site in
/// `View::CommandPalette` reads symmetrically with the keystroke
/// path. Distinct name keeps stack traces meaningful — if a feature
/// fires only from the palette and breaks, the frame name names it.
fn run_palette_action(
  terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
  app: &mut App,
  action: Action,
) -> Result<()> {
  run_action(terminal, app, action)
}

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
  let Some(path) = app.yank_selected_path() else {
    app.status = "nothing selected".into();
    return;
  };
  let text = path.display().to_string();
  copy_text_to_clipboard(app, &text, "yanked path");
}

/// Copy the Command Logs transcript to the clipboard (issue #279, `y`).
/// Builds the plain-text transcript from owned state, then hands it to the
/// shared clipboard helper. Empty transcript → a status note, no spawn.
fn copy_command_logs_to_clipboard(app: &mut App) {
  let text = app.command_logs_transcript();
  if text.is_empty() {
    app.status = "no commands to copy".into();
    return;
  }
  copy_text_to_clipboard(app, &text, "copied command logs");
}

/// Feed `text` to the first available clipboard tool from
/// [`clipboard_candidates`]. Walks the candidates in order, uses the first
/// one whose binary is on `$PATH`, and feeds the text through its stdin.
/// `success` is the status-bar label on a clean copy. Failures and "no tool
/// found" both surface in the status bar — the TUI must never die on a
/// clipboard miss.
fn copy_text_to_clipboard(app: &mut App, text: &str, success: &str) {
  use std::io::Write;
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
            app.status = format!("{} ({})", success, cmd);
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

/// Canonical documentation URL opened by the `.` key (issue #233).
///
/// Derived from the crate's `repository` (Cargo.toml) so a fork points at
/// its own docs without a patch — there is no standalone docs site
/// deployed yet, so the MVP target is the docs tree on the repo's default
/// branch. A `[docs]` config override is a possible follow-up.
pub const DOCS_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/tree/main/docs");

/// Spawn the OS opener for `url` (used by the OpenMenu key handler and the
/// `.` open-docs key, issue #233).
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
