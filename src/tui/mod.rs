mod app;
/// Commit-graph topology renderer, ported from lazygit. **Not part of the
/// public SemVer surface** — exposed only so the integration tests under
/// `tests/` can pin the algorithm. Use `gwm::tui::recent_commits_lines`
/// (re-exported below) for the stable entry point that callers should
/// actually depend on.
#[doc(hidden)]
pub mod commit_graph;
pub mod keymap;
pub mod modal_keymap;
pub mod palette;
pub mod state;
pub mod theme;
mod ui;
pub mod wt_tree;

use crate::error::Result;
use crate::tui::keymap::Action;
use crate::tui::modal_keymap::{KeyContext, ModalAction};
use crossterm::{
  event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
  execute,
  terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub use app::{
  App, CreateKey, ExecPickerKey, LauncherPlan, LinkPromptKey, LinkPromptStage, LinkTarget, OpenTarget, View,
};
pub use state::async_task::{CreateWorktreeResult, TaskKind, TaskMsg, TaskRunner};
pub use state::clean_overlay::CleanOverlay;
pub use state::command_logs::CommandLogs;
pub use state::config_panel::{
  build_key_rows, ConfigPanel, FieldKind, KeyCapture, KeyRow, KeyTarget, SettingField, SettingsLayer, SettingsTab,
};
pub use state::confirm::{ConfirmButton, ConfirmKeyAction, ConfirmModal, CountdownTickOutcome};
pub use state::create_form::{CreateForm, Field};
pub use state::exec_picker::ExecPicker;
pub use state::filter::FilterState;
pub use state::github_fetch::{FetchKey, GitHubFetch, GitHubFetchState};
pub use state::link_prompt::LinkPrompt;
pub use state::pty_overlay::{key_to_bytes, PtyKind, PtyOverlay};
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
  build_sidebar_sections, centered_abs, chip_style, ci_indicator, command_logs_footer_hints,
  config_capture_footer_hints, config_edit_footer_hints, config_nav_footer_hints, confirm_buttons_line,
  confirm_delete_branch_line, confirm_detail_line, create_buttons_line, delete_worktree_title, ellipsize_middle,
  field_input_line, filled_cells_for_progress, footer_line, format_status, freshness_color, github_status_lines,
  header_line, help_body_section_color, help_entry_line, help_label_style, help_lines, help_rows, help_section_style,
  hint_key_style, hint_label_style, issue_badge_color, issue_pr_pane_title, issue_summary_line, link_open_modal_lines,
  link_prompt_modal_width, link_target_keys, link_target_line, modal_hint_line, palette_name_style, pane_counter,
  panel_border_color, pr_badge_color, pr_summary_line, recent_commits_lines, recent_items_pane_title,
  rename_buttons_line, status_line, status_pane_title, table_marker, tilde_compress_with_home, type_selector_line,
  working_tree_counts_footer, working_tree_pane_title, working_tree_status_counts, working_tree_status_line,
  worktree_name_style, worktree_path_style, worktrees_pane_title, HelpRow, HintContext, SidebarSections,
  WorkingTreeCounts, COMMIT_HASH_DISPLAY_LEN, ISSUE_ICON, PR_ICON, RECENT_COMMITS_LIMIT, WT_CREATED_ICON,
  WT_DELETED_ICON, WT_MODIFIED_ICON,
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
  // #290: ExitToWorktree prints the selected path to stdout so a shell
  // wrapper (`cd "$(gwm)"`) can change directory.
  if let Some(path) = result? {
    println!("{}", path.display());
  }
  Ok(())
}

/// Workspace-mode entry point (issue #36): open the TUI over every git repo
/// one level below `root`. Same teardown-safety contract as [`run`] — App
/// construction (discovery + per-repo config load) happens before the
/// terminal is touched, so a failure (no repos, bad config) leaves the
/// terminal cooked.
pub fn run_workspace(root: &Path, trust_mode: crate::trust::TrustMode) -> Result<()> {
  let app =
    App::new_workspace_at_layered(root, crate::config::global_config_path().as_deref())?.with_trust_mode(trust_mode);
  let mut terminal = enter_terminal()?;
  let result = run_app(&mut terminal, app);
  leave_terminal(&mut terminal)?;
  if let Some(path) = result? {
    println!("{}", path.display());
  }
  Ok(())
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
fn enter_terminal() -> Result<Terminal<CrosstermBackend<io::Stderr>>> {
  enable_raw_mode()?;
  // Render the TUI to STDERR, not stdout: `exit_to_worktree` (#290) prints the
  // selected path to stdout for the `cd "$(gwm)"` shell wrapper, so stdout must
  // stay free of alt-screen / ANSI frames (the fzf/skim pattern). stderr is the
  // tty in an interactive session, so the UI still draws (Codex review #292).
  let mut stderr = io::stderr();
  execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
  Ok(Terminal::new(CrosstermBackend::new(stderr))?)
}

/// Inverse of `enter_terminal`. Always called from the same scope as
/// `enter_terminal` so the order of teardown matches the order of setup.
fn leave_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stderr>>) -> Result<()> {
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

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stderr>>, mut app: App) -> Result<Option<PathBuf>> {
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

    // Issue #35: drain PTY output and detect process death before drawing.
    // `poll_bytes` feeds pending reader-thread bytes into the vt100 parser
    // so the next frame reflects the freshest output. If the process has
    // already exited, close the overlay so the list view is rendered instead.
    if app.view == View::Pty {
      let status = app.pty_overlay.as_mut().map(|p| {
        p.poll_bytes();
        (p.kind, p.is_alive())
      });
      match status {
        // #325: a one-shot exec command exits the instant it finishes — keep
        // its final output on screen and let any key dismiss it, instead of
        // the lazygit / shell behaviour of closing the overlay on child death.
        Some((PtyKind::Exec, false)) => {
          if let Some(p) = app.pty_overlay.as_mut() {
            p.finished = true;
          }
        }
        // Interactive overlays (lazygit / shell / review) close on child exit.
        Some((_, false)) | None => app.close_pty_overlay(),
        Some((_, true)) => {}
      }
    }

    // Issue #36: in workspace mode keep the active repo aligned with the
    // selected worktree's repo before drawing (so the sidebar preview reads
    // the right repo) and before the next key fires an action against it. A
    // no-op in single-repo mode and when the selection hasn't crossed repos.
    app.sync_active_repo();

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

    // #325: drive the clean overlay's safety countdown off the same poll
    // cadence as the delete confirm above, so an armed reclaim auto-fires
    // after the configured delay even when no key event arrives.
    if app.view == View::CleanReport {
      if app.clean_overlay.confirm.is_armed() {
        app.spinner.tick();
      }
      match app.tick_clean_countdown(now) {
        CountdownTickOutcome::ReadyToFire => app.clean_overlay_delete(),
        CountdownTickOutcome::Pending | CountdownTickOutcome::NotArmed => {}
      }
    }

    // Issue #35: tighten the poll cadence while the PTY is open so typed
    // characters and arrow keys feel responsive (< 50 ms round-trip vs.
    // the normal 200 ms status-refresh interval).
    let poll_ms = if app.view == View::Pty { 50 } else { 200 };
    if !event::poll(Duration::from_millis(poll_ms))? {
      continue;
    }
    let ev = event::read()?;
    // Issue #35: resize the PTY when the host terminal is resized so the
    // child program (lazygit, shell) sees the updated dimensions.
    if let Event::Resize(cols, rows) = ev {
      if app.view == View::Pty {
        if let Some(ref mut pty) = app.pty_overlay {
          // 90% × 90% overlay minus overlay_block overhead (6 cols, 4 rows).
          let inner_cols = ((cols as u32 * 90 / 100) as u16).saturating_sub(6).max(10);
          let inner_rows = ((rows as u32 * 90 / 100) as u16).saturating_sub(4).max(5);
          pty.resize(inner_cols, inner_rows);
        }
      }
      terminal.clear()?;
      continue;
    }
    let Event::Key(key) = ev else { continue };
    if key.kind != KeyEventKind::Press {
      continue;
    }

    // Global keys
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
      // Inside the PTY overlay, Ctrl+C must reach the child process (interrupt
      // a running command) rather than quit gwm. Forward the byte and continue.
      if app.view == View::Pty {
        if let Some(ref mut pty) = app.pty_overlay {
          let _ = pty.write_key(key);
        }
        continue;
      }
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
      // #219: keys resolved through the `help` modal context. Scroll the
      // Keybindings overlay when it outgrows the modal (#217).
      View::Help => match app.resolve_modal(KeyContext::Help, key) {
        Some(ModalAction::HelpClose) => app.view = View::List,
        Some(ModalAction::HelpScrollDown) => app.help_scroll_down(),
        Some(ModalAction::HelpScrollUp) => app.help_scroll_up(),
        Some(ModalAction::HelpScrollRight) => app.help_scroll_right(),
        Some(ModalAction::HelpScrollLeft) => app.help_scroll_left(),
        Some(ModalAction::HelpScrollTop) => app.help_scroll = 0,
        Some(ModalAction::HelpScrollBottom) => app.help_scroll = app.help_max_scroll,
        _ => {}
      },
      // Command Logs overlay (issue #226). Scrolls like the help overlay;
      // closes on Esc / `q` or the bound `command_logs` key (default `3`)
      // so the open key toggles it shut even when rebound.
      // #219: keys resolved through the `command_logs` modal context. The
      // bound global `command_logs` key still toggles the overlay shut.
      View::CommandLogs => match app.resolve_modal(KeyContext::CommandLogs, key) {
        Some(ModalAction::CommandLogsClose) => app.view = View::List,
        // `y` copies the whole transcript to the clipboard (issue #279).
        Some(ModalAction::CommandLogsCopy) => copy_command_logs_to_clipboard(&mut app),
        Some(ModalAction::CommandLogsScrollDown) => app.command_logs.scroll_down(),
        Some(ModalAction::CommandLogsScrollUp) => app.command_logs.scroll_up(),
        Some(ModalAction::CommandLogsScrollRight) => app.command_logs.scroll_right(),
        Some(ModalAction::CommandLogsScrollLeft) => app.command_logs.scroll_left(),
        Some(ModalAction::CommandLogsScrollTop) => app.command_logs.scroll_to_top(),
        Some(ModalAction::CommandLogsScrollBottom) => app.command_logs.scroll_to_bottom(),
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
      // #219: edit sub-mode keys resolve through the `config.edit` context;
      // anything else is literal input into the numeric edit buffer.
      // Keys tab live capture (issue #294). While a capture is armed every
      // keystroke is recorded into the binding rather than navigating. The
      // logic lives in a testable `App` method (mirrors `handle_create_key` /
      // `handle_link_prompt_key`): `cancel` (def Esc) aborts, `submit` (def
      // Enter) commits a multi-stroke global chord, Backspace drops its last
      // stroke, a single-stroke modal verb auto-commits on the first key. Esc /
      // Enter / Backspace stay reserved controls and can't be assigned via
      // capture — hand-edit `.gwm.toml` for those (same hard-coded escape-hatch
      // trade-off as the rest of the keymap).
      View::Config if app.config_panel.capture.is_some() => app.handle_capture_key(key),
      View::Config if app.config_panel.editing.is_some() => match app.resolve_modal(KeyContext::ConfigEdit, key) {
        Some(ModalAction::ConfigEditSubmit) => app.commit_settings_edit(),
        Some(ModalAction::ConfigEditCancel) => app.config_panel.cancel_edit(),
        _ => match key.code {
          KeyCode::Backspace => app.config_panel.pop_edit_char(),
          KeyCode::Char(c) => app.config_panel.push_edit_char(c),
          _ => {}
        },
      },
      // #219: nav keys resolve through the `config` context. Select vs scroll
      // and the horizontal pan / jump verbs stay gated on the read-only `All`
      // tab exactly as before; the bound global `config_panel` key still
      // toggles the overlay shut.
      View::Config => {
        let on_all = app.config_panel.tab == SettingsTab::All;
        match app.resolve_modal(KeyContext::Config, key) {
          Some(ModalAction::ConfigClose) => app.view = View::List,
          Some(ModalAction::ConfigNextTab) => app.config_panel.next_tab(),
          Some(ModalAction::ConfigPrevTab) => app.config_panel.prev_tab(),
          Some(ModalAction::ConfigToggleLayer) => app.config_panel.toggle_layer(),
          // On the Keys tab `activate` arms a live keystroke capture for the
          // selected binding (issue #294); elsewhere it cycles a choice or
          // opens the numeric/text edit buffer.
          Some(ModalAction::ConfigActivate) => {
            if app.config_panel.tab == SettingsTab::Keys {
              app.config_panel.begin_capture();
            } else {
              app.activate_selected_setting();
            }
          }
          Some(ModalAction::ConfigSelectNext) => {
            if on_all {
              app.config_panel.scroll_down();
            } else {
              app.config_panel.select_next();
            }
          }
          Some(ModalAction::ConfigSelectPrev) => {
            if on_all {
              app.config_panel.scroll_up();
            } else {
              app.config_panel.select_prev();
            }
          }
          Some(ModalAction::ConfigScrollRight) if on_all => app.config_panel.scroll_right(),
          Some(ModalAction::ConfigScrollLeft) if on_all => app.config_panel.scroll_left(),
          Some(ModalAction::ConfigScrollTop) if on_all => app.config_panel.scroll_to_top(),
          Some(ModalAction::ConfigScrollBottom) if on_all => app.config_panel.scroll_to_bottom(),
          _ if app.key_matches_action(key, Action::ConfigPanel) => app.view = View::List,
          _ => {}
        }
      }
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
      // #219: keys resolve through the `confirm` context. `confirm` (def `y`)
      // fires regardless of focus (unchanged muscle memory); `activate` (def
      // Enter) acts on the *focused* button — focus defaults to Cancel (#187),
      // so a stray Enter on a freshly-opened modal cancels rather than
      // deletes. The bound global `delete_branch` key still toggles the
      // branch-deletion checkbox. Focus nav (#187): `focus_confirm` (←/h),
      // `focus_cancel` (→/l), `toggle_focus` (Tab).
      View::Confirm => match app.resolve_modal(KeyContext::Confirm, key) {
        Some(ModalAction::ConfirmConfirm) => confirm_fire(&mut app),
        Some(ModalAction::ConfirmActivate) => match app.confirm.focused_button() {
          ConfirmButton::Confirm => confirm_fire(&mut app),
          ConfirmButton::Cancel => app.confirm_dismiss(),
        },
        Some(ModalAction::ConfirmCancel) => app.confirm_dismiss(),
        Some(ModalAction::ConfirmFocusConfirm) => app.confirm.focus_confirm(),
        Some(ModalAction::ConfirmFocusCancel) => app.confirm.focus_cancel(),
        Some(ModalAction::ConfirmToggleFocus) => app.confirm.toggle_focus(),
        _ if app.key_matches_action(key, Action::ToggleDeleteBranch) => app.toggle_delete_branch(),
        _ => {}
      },
      // #219: the bootstrap-report overlay closes (and refreshes) on the
      // `report` context's `close` verb (def Esc / q / Enter).
      View::Report => {
        if let Some(ModalAction::ReportClose) = app.resolve_modal(KeyContext::Report, key) {
          app.view = View::List;
          app.refresh()?;
        }
      }
      // #219: keys resolve through the `open_menu` context. The bound global
      // `fetch_github` key still refreshes the GitHub status in place.
      View::OpenMenu => match app.resolve_modal(KeyContext::OpenMenu, key) {
        Some(ModalAction::OpenMenuClose) => app.exit_open_menu(),
        Some(ModalAction::OpenMenuToggle) => app.open_menu_toggle_selection(),
        Some(ModalAction::OpenMenuAccept) => {
          if let Some(url) = app.open_menu_pick(app.open_menu_selected) {
            open_url(&url, &mut app);
          }
        }
        Some(ModalAction::OpenMenuIssue) => {
          if let Some(url) = app.open_menu_pick(LinkTarget::Issue) {
            open_url(&url, &mut app);
          }
        }
        Some(ModalAction::OpenMenuPr) => {
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
      // Issue #35: PTY overlay. All keys are forwarded to the child process
      // via `write_key` — lazygit and the shell consume them directly. `Esc`
      // is the only key gwm intercepts: it kills the child and closes the
      // overlay so the user can exit even if the program does not respond to
      // `q`. Process death (natural exit via lazygit's `q`) is detected by
      // the pre-draw `is_alive()` check above and also closes the overlay.
      //
      // #219: this `Esc` stays hard-coded by design — it is an *emergency*
      // detach, and routing it through a rebindable context would silently
      // steal a keystroke from the child program. See the `modal_keymap`
      // module note ("What stays hard-coded").
      View::Pty => {
        // #325: once a one-shot exec command has finished, the overlay is
        // just showing its final output — there is no live child to receive
        // input, so any key dismisses it. Otherwise `Esc` is the emergency
        // detach and every other key passes through to the child.
        let exec_finished = app.pty_overlay.as_ref().is_some_and(|p| p.finished);
        if key.code == KeyCode::Esc || exec_finished {
          app.close_pty_overlay();
        } else if let Some(ref mut pty) = app.pty_overlay {
          let _ = pty.write_key(key);
        }
      }
      // #325: exec profile picker. The testable handler owns the highlight;
      // `Submit` resolves the profile to an argv and spawns it in a PTY
      // overlay rooted at the selected worktree (mirrors `LazyGitPty`).
      View::ExecPicker => match app.handle_exec_picker_key(key) {
        ExecPickerKey::Submit => {
          if let Some((argv, cwd)) = app.exec_picker_resolve() {
            let sz = terminal.size().unwrap_or_default();
            let inner_cols = ((sz.width as u32 * 90 / 100) as u16).saturating_sub(6).max(20);
            let inner_rows = ((sz.height as u32 * 90 / 100) as u16).saturating_sub(4).max(5);
            let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            match PtyOverlay::spawn(PtyKind::Exec, &argv_refs, &cwd, inner_cols, inner_rows) {
              Ok(pty) => app.open_pty_overlay(pty),
              Err(e) => {
                app.status = format!("exec overlay failed: {}", e);
                app.close_exec_picker();
              }
            }
          } else {
            // Resolve failed (status already set) — close back to the list.
            app.close_exec_picker();
          }
        }
        ExecPickerKey::Cancel => app.close_exec_picker(),
        ExecPickerKey::Handled => {}
      },
      // #325: clean reclaim overlay. Mirrors the delete-confirm routing —
      // `confirm` arms / fires the safety countdown, `cancel` aborts, j/k
      // cycle the `[clean.profiles]` picker (re-scanning each time). The
      // countdown auto-fire is driven by the tick block above.
      View::CleanReport => match app.resolve_modal(KeyContext::Clean, key) {
        Some(ModalAction::CleanCancel) => app.close_clean_overlay(),
        Some(ModalAction::CleanConfirm) => {
          if app.clean_confirm_press(now) == ConfirmKeyAction::FireNow {
            app.clean_overlay_delete();
          }
        }
        Some(ModalAction::CleanNext) => app.clean_overlay_next(),
        Some(ModalAction::CleanPrev) => app.clean_overlay_prev(),
        _ => {}
      },
      // #290: worktree-rename modal. Reuses the Create form input handler
      // (Type / Issue / Desc), but routes submit to the rename worker. Input
      // is swallowed while the async rename is in flight, mirroring create.
      View::Edit if app.is_edit_worktree_loading() => {}
      View::Edit => match app.handle_create_key(key) {
        CreateKey::Submit => {
          if let Err(e) = app.submit_edit_worktree() {
            app.status = format!("rename failed: {}", e);
          }
        }
        CreateKey::Cancel => app.cancel_edit_worktree(),
        CreateKey::Handled => {}
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
      // #219: close / accept / prev / next resolve through the `palette`
      // context; every other key is literal input into the fuzzy buffer.
      View::CommandPalette => match app.resolve_modal(KeyContext::CommandPalette, key) {
        Some(ModalAction::CommandPaletteClose) => app.close_command_palette(),
        Some(ModalAction::CommandPaletteAccept) => {
          if let Some(action) = app.accept_command_palette() {
            run_palette_action(terminal, &mut app, action)?;
          }
        }
        Some(ModalAction::CommandPalettePrev) => app.palette_cycle_up(),
        Some(ModalAction::CommandPaletteNext) => app.palette_cycle_down(),
        _ => match key.code {
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
  // #290: ExitToWorktree stores the path; normal quit leaves it None.
  Ok(app.should_exit_to.or(app.picker_result))
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
fn run_action(terminal: &mut Terminal<CrosstermBackend<io::Stderr>>, app: &mut App, action: Action) -> Result<()> {
  // Issue #304: in workspace mode, block repo-mutating actions while the
  // selected row's repo could not be activated (moved/deleted/corrupt since
  // listing) — `app.repo`/`workdir`/`config` still point at the previously
  // active repo, so the write would hit the wrong repository. Navigation and
  // refresh stay live so the user can recover (a refresh drops the dead repo).
  if app.workspace_active_stale && action.is_repo_mutating() {
    app.status = "workspace: selected repo is unavailable (moved/deleted?) — press r to refresh".into();
    return Ok(());
  }

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
    // #290: `Y` yanks the worktree path (was `y` before #290).
    Action::YankPath => yank_selected_path_to_clipboard(app),
    // #290: `y` yanks the branch name.
    Action::YankBranchName => yank_selected_branch_to_clipboard(app),
    // #290: `w` yanks the worktree slug/name.
    Action::YankWorktreeName => yank_selected_worktree_name_to_clipboard(app),
    // #290: TerminalFullscreen replaces Open — open the shell/editor/finder
    // target fullscreen (honours [tui.open] config, same as the old `o`).
    Action::TerminalFullscreen => match app.resolve_open_target() {
      None => app.status = "nothing selected".into(),
      Some(OpenTarget::Finder { .. }) => app.open_selected_in_finder(),
      Some(OpenTarget::Shell { path, command }) => run_subshell(terminal, &command, &[], Some(&path), app, "shell")?,
      Some(OpenTarget::Editor { path, command }) => {
        let path_str = path.display().to_string();
        run_subshell(terminal, &command, &[&path_str], None, app, "editor")?
      }
    },
    // #290: TerminalPty replaces OpenTerminalOverlay — open a native $SHELL
    // in an embedded PTY overlay rooted at the selected worktree's path.
    Action::TerminalPty => {
      let cwd = app.selected().map(|wt| wt.path.clone());
      match cwd {
        None => app.status = "nothing selected".into(),
        Some(path) => {
          #[cfg(windows)]
          let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
          #[cfg(not(windows))]
          let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
          let sz = terminal.size().unwrap_or_default();
          let inner_cols = ((sz.width as u32 * 90 / 100) as u16).saturating_sub(6).max(20);
          let inner_rows = ((sz.height as u32 * 90 / 100) as u16).saturating_sub(4).max(5);
          match PtyOverlay::spawn(PtyKind::Terminal, &[shell.as_str()], &path, inner_cols, inner_rows) {
            Ok(pty) => app.open_pty_overlay(pty),
            Err(e) => app.status = format!("terminal overlay failed: {}", e),
          }
        }
      }
    }
    // #290: LazyGitFullscreen replaces GitTui.
    Action::LazyGitFullscreen => {
      if let Some(plan) = app.prepare_git_tui() {
        run_launcher(terminal, plan, app)?;
      }
    }
    // #290: LazyGitPty replaces GitTuiOverlay — open lazygit in an embedded
    // PTY overlay sized to 90% × 90% of the terminal.
    Action::LazyGitPty => {
      if let Some(plan) = app.prepare_git_tui() {
        let sz = terminal.size().unwrap_or_default();
        let inner_cols = ((sz.width as u32 * 90 / 100) as u16).saturating_sub(6).max(20);
        let inner_rows = ((sz.height as u32 * 90 / 100) as u16).saturating_sub(4).max(5);
        let argv: Vec<String> = plan.expanded.argv.clone();
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        match PtyOverlay::spawn(PtyKind::LazyGit, &argv_refs, &plan.cwd, inner_cols, inner_rows) {
          Ok(pty) => app.open_pty_overlay(pty),
          Err(e) => app.status = format!("lazygit overlay failed: {}", e),
        }
      }
    }
    // #290: ReviewPty replaces ReviewOverlay — open the review tool in an
    // embedded PTY overlay. Picker-gated: branch-specific, meaningless in
    // `gwm switch`.
    Action::ReviewPty if !app.picker_mode => {
      if let Some(mut plan) = app.prepare_review() {
        let sz = terminal.size().unwrap_or_default();
        let inner_cols = ((sz.width as u32 * 90 / 100) as u16).saturating_sub(6).max(20);
        let inner_rows = ((sz.height as u32 * 90 / 100) as u16).saturating_sub(4).max(5);
        let argv: Vec<String> = plan.expanded.argv.clone();
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        match PtyOverlay::spawn(PtyKind::Review, &argv_refs, &plan.cwd, inner_cols, inner_rows) {
          Ok(mut pty) => {
            pty.diff_file = plan.expanded.diff_file.take();
            app.open_pty_overlay(pty);
          }
          Err(e) => app.status = format!("review overlay failed: {}", e),
        }
      }
    }
    Action::Create if !app.picker_mode => app.enter_create(),
    Action::DeleteConfirm if !app.picker_mode => app.enter_confirm_delete(),
    Action::Bootstrap if !app.picker_mode => app.bootstrap_selected(),
    // Issue #258: `gwm sync` of the selected worktree, off-thread.
    Action::Sync if !app.picker_mode => app.request_sync(),
    // #290: `p` pulls, `P` pushes, both off-thread.
    Action::Pull if !app.picker_mode => app.request_pull(),
    Action::Push if !app.picker_mode => app.request_push(),
    // #290: `c` opens the branch-rename modal.
    Action::EditWorktree if !app.picker_mode => app.enter_edit_worktree(),
    // #290: `e` exits TUI and prints selected path to stdout.
    Action::ExitToWorktree => app.exit_to_worktree(),
    // #290: `t` opens the selected worktree in a new mux pane/tab.
    Action::MuxPane if !app.picker_mode => app.open_in_mux_pane(),
    // #290: `h`/`H` fire user macros from [tui.macro1]/[tui.macro2].
    Action::Macro1 if !app.picker_mode => run_macro(terminal, app, 1)?,
    Action::Macro2 if !app.picker_mode => run_macro(terminal, app, 2)?,
    Action::ToggleDeleteBranch if !app.picker_mode => app.toggle_delete_branch(),
    // #290: BrowseLinks replaces OpenMenu.
    Action::BrowseLinks if !app.picker_mode => app.enter_open_menu(),
    // Not picker-gated — `gwm switch` can open docs too.
    Action::OpenDocs => open_url(DOCS_URL, app),
    Action::LinkPrompt if !app.picker_mode => app.enter_link_prompt(),
    Action::FetchGithub if !app.picker_mode => app.refresh_github_status(),
    // #290: ReviewFullscreen replaces Review.
    Action::ReviewFullscreen if !app.picker_mode => {
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
    // Issue #325: `x` opens the exec profile picker. Picker-gated —
    // running a profile in a PTY is a focus-mode action, meaningless in
    // the stripped-down `gwm switch` picker.
    Action::ExecOverlay if !app.picker_mode => app.enter_exec_picker(),
    // Issue #325: `X` opens the clean reclaim overlay. Picker-gated — it
    // deletes from the selected worktree, a focus-mode action.
    Action::CleanOverlay if !app.picker_mode => app.enter_clean_overlay(),
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
  terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
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
/// Whether a fullscreen child's stdout must be re-routed to the controlling
/// terminal. True exactly when gwm's own stdout is *not* a tty — i.e. it is
/// the command-substitution pipe of a `cd "$(gwm)"` wrapper reading the
/// exit-to-worktree path (#290). Inheriting that pipe would send the child's
/// TUI frames / ANSI into the captured path (Codex review on PR #292). Pure
/// so the policy is unit-testable without a real pipe.
pub fn wants_child_stdout_on_tty(stdout_is_terminal: bool) -> bool {
  !stdout_is_terminal
}

/// Point `command`'s stdout at `/dev/tty` when gwm's stdout is captured, so a
/// fullscreen child never writes into the `cd "$(gwm)"` pipe. No-op when
/// stdout is already a tty, on non-unix, or when `/dev/tty` can't be opened
/// (then inherit and accept the captured-pipe risk rather than fail the
/// launch).
fn route_fullscreen_child_stdout(command: &mut std::process::Command) {
  use std::io::IsTerminal;
  if !wants_child_stdout_on_tty(std::io::stdout().is_terminal()) {
    return;
  }
  #[cfg(unix)]
  if let Ok(tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
    command.stdout(std::process::Stdio::from(tty));
  }
  // No `/dev/tty` equivalent here, so the child inherits stdout. Bind the
  // param to silence the unused-variable error under `-D warnings` on the
  // non-unix build (CI windows-latest caught this).
  #[cfg(not(unix))]
  let _ = command;
}

fn run_launcher(
  terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
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

    let mut cmd = Command::new(bin);
    cmd.args(rest).current_dir(&plan.cwd);
    route_fullscreen_child_stdout(&mut cmd);
    let spawn = cmd.status();

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
  terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
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
  route_fullscreen_child_stdout(&mut command);
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

fn yank_selected_branch_to_clipboard(app: &mut App) {
  let Some(branch) = app.yank_selected_branch() else {
    app.status = "nothing selected or no branch (detached HEAD)".into();
    return;
  };
  copy_text_to_clipboard(app, &branch, "yanked branch name");
}

fn yank_selected_worktree_name_to_clipboard(app: &mut App) {
  let Some(name) = app.yank_selected_worktree_name() else {
    app.status = "nothing selected".into();
    return;
  };
  copy_text_to_clipboard(app, &name, "yanked worktree name");
}

/// Fire a user macro (#290). `n` is 1 for `Macro1`/`h`, 2 for `Macro2`/`H`.
/// Reads `[tui.macro1]` / `[tui.macro2]` from config; no-ops when absent.
fn run_macro(terminal: &mut Terminal<CrosstermBackend<io::Stderr>>, app: &mut App, n: u8) -> Result<()> {
  use crate::config::MacroOpenMode;
  let cfg = if n == 1 {
    app.config.tui.macro1.clone()
  } else {
    app.config.tui.macro2.clone()
  };
  let Some(macro_cfg) = cfg else {
    app.status = format!("macro{} not configured — add [tui.macro{}] to .gwm.toml", n, n);
    return Ok(());
  };
  use crate::multiplexer::{build_tmux_command, build_zellij_command, detect_tmux, detect_zellij, SpawnMode};
  // Macros run in the selected worktree. With nothing selected (e.g. a filter
  // with no matches), refuse rather than silently running in the main repo —
  // a destructive command must not hit the wrong tree (Codex review on #292).
  let Some(path) = app.selected().map(|w| w.path.clone()) else {
    app.status = format!("macro{}: nothing selected", n);
    return Ok(());
  };

  #[cfg(windows)]
  let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
  #[cfg(not(windows))]
  let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
  let shell_flag = if cfg!(windows) { "/C" } else { "-c" };

  // Resolve the mux command up front so a `mux_pane` macro can fall back to the
  // PTY overlay when no multiplexer is active (the documented behaviour — Codex
  // review on PR #292), rather than no-oping.
  let mux_cmd = if matches!(macro_cfg.open_in, MacroOpenMode::MuxPane) {
    let label = format!("macro{}", n);
    if detect_tmux(std::env::var("TMUX").ok()) {
      Some(build_tmux_command(&label, &path, SpawnMode::Split))
    } else if detect_zellij(std::env::var("ZELLIJ").ok()) {
      Some(build_zellij_command(&label, &path, SpawnMode::Split))
    } else {
      app.status = format!("macro{}: no multiplexer — falling back to PTY overlay", n);
      None
    }
  } else {
    None
  };

  if let Some(cmd) = mux_cmd {
    let bin = cmd[0].as_str();
    let mut full_cmd: Vec<&str> = cmd[1..].iter().map(String::as_str).collect();
    if bin == "zellij" {
      // `zellij action new-pane` runs the trailing argv DIRECTLY, not via a
      // shell, so a command with spaces/shell syntax must be wrapped in
      // `-- <shell> -c <cmd>` (Codex review on PR #292).
      full_cmd.push("--");
      full_cmd.push(shell.as_str());
      full_cmd.push(shell_flag);
      full_cmd.push(macro_cfg.command.as_str());
    } else {
      // tmux takes the command as a SINGLE shell-command operand and hands it
      // to the shell itself, so we pass it as one trailing argument rather than
      // pre-splitting into `sh -c <cmd>`.
      full_cmd.push(macro_cfg.command.as_str());
    }
    match std::process::Command::new(bin).args(&full_cmd).spawn() {
      Ok(_) => app.status = format!("macro{} opened in mux pane", n),
      Err(e) => app.status = format!("macro{} mux failed: {}", n, e),
    }
  } else {
    // PTY overlay: the explicit `pty` mode, or the `mux_pane` fallback above.
    let sz = terminal.size().unwrap_or_default();
    let inner_cols = ((sz.width as u32 * 90 / 100) as u16).saturating_sub(6).max(20);
    let inner_rows = ((sz.height as u32 * 90 / 100) as u16).saturating_sub(4).max(5);
    let argv = [shell.as_str(), shell_flag, macro_cfg.command.as_str()];
    match PtyOverlay::spawn(PtyKind::Terminal, &argv, &path, inner_cols, inner_rows) {
      Ok(pty) => app.open_pty_overlay(pty),
      Err(e) => app.status = format!("macro{} overlay failed: {}", n, e),
    }
  }
  Ok(())
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
