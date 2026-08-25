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
  mux_pane_status, read_pins_from_sources, App, ConfirmKind, CreateKey, ExecPickerKey, LauncherPlan, LinkPromptKey,
  LinkPromptStage, LinkTarget, NoteKey, OpenTarget, PendingMerge, RepoMeta, View, WorkspaceState,
};
pub use state::async_task::{
  CreateWorktreeResult, DeleteBatchOutcome, DeleteFailure, DeleteTarget, TaskKind, TaskMsg, TaskRunner,
};
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
pub use state::note_editor::NoteEditor;
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
  agent_cell_label, agent_pane_lines, agents_pane_title, author_initials, badge_group_width, bootstrap_report_lines,
  branch_name_color, branch_status_color, build_sidebar_payload, build_sidebar_sections, cells, centered_abs,
  chip_style, ci_indicator, clean_dir_icon, command_logs_footer_hints, compact_header_line,
  config_capture_footer_hints, config_edit_footer_hints, config_nav_footer_hints, confirm_buttons_line,
  confirm_delete_branch_line, confirm_detail_line, create_buttons_line, delete_batch_title, delete_worktree_title,
  detail_overlay_width, detail_visible_rows, display_path_with_home, ellipsize_middle, field_input_line,
  filled_cells_for_progress, folded_status_line, footer_line, form_field_scroll, format_status, freshness_color,
  github_status_lines, header_line, help_body_section_color, help_entry_line, help_label_style, help_lines, help_rows,
  help_section_style, hint_key_style, hint_label_style, issue_badge_color, issue_pr_pane_title, issue_summary_line,
  link_open_modal_lines, link_prompt_modal_width, link_target_keys, link_target_line, list_pane_counter,
  markdown_style, modal_height, modal_hint_for_context, modal_hint_for_context_with_fields, modal_hint_line,
  modal_width, overlay_modal_width, pad_cells, palette_name_style, pane_counter, panel_border_color, picker_window,
  pr_badge_color, pr_summary_line, recent_commits_lines, recent_items_pane_title, reclaim_size_color,
  rename_buttons_line, rich_view_modal_width, skip_cells, status_line, status_pane_title, table_marker,
  tilde_compress_with_home, type_selector_line, working_tree_counts_footer, working_tree_pane_title,
  working_tree_status_counts, working_tree_status_line, worktree_name_style, worktree_path_style, worktrees_pane_title,
  HelpRow, HintContext, SidebarSections, WorkingTreeCounts, CI_FAILING_ICON, CI_PASSING_ICON, CI_RUNNING_ICON,
  COMMIT_HASH_DISPLAY_LEN, ISSUE_ICON, PR_ICON, RECENT_COMMITS_LIMIT, WT_CREATED_ICON, WT_DELETED_ICON,
  WT_MODIFIED_ICON,
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

/// Clear the whole screen and force a full repaint on the next `draw`,
/// **without** asking the terminal where the cursor is (issue #548).
///
/// `Terminal::clear` snapshots the cursor first — `backend.get_cursor_position()`
/// writes `ESC [ 6 n` and blocks until the terminal answers with a DSR report.
/// Returning from a fullscreen surface (PTY overlay, `exec` run, review launch)
/// is exactly the moment that answer is most likely to be late: crossterm then
/// returns `The cursor position could not be read within a normal duration` and
/// the `?` ended the whole session over a cosmetic operation.
///
/// Dropping the snapshot costs nothing here: every caller repaints the entire
/// frame on the very next loop iteration, so the position `Terminal::clear`
/// would have restored is overwritten before anyone could see it. What the
/// callers actually need from `clear` is the other half — wiping the screen and
/// resetting the back buffer so the next `draw` is a full repaint rather than a
/// diff against stale content. `Terminal::resize` does precisely that pair for a
/// `Fullscreen` viewport (`clear_region(All)` + back-buffer reset) and never
/// touches the cursor, and gwm only ever builds a fullscreen terminal
/// (`enter_terminal` above). Nothing left under the `?` can time out either:
/// `backend.size()` is a `TIOCGWINSZ` ioctl (falling back to a `tput` spawn),
/// never a request the terminal has to answer — and `Terminal::draw` already
/// calls it on every frame through `autoresize`.
pub fn clear_without_cursor_query<B: ratatui::backend::Backend>(
  terminal: &mut Terminal<B>,
) -> std::result::Result<(), B::Error> {
  let area = terminal.size()?.into();
  terminal.resize(area)
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
    // Routed on what the modal is actually about (#551). Exhaustive: the
    // countdown and the danger border are shared, the consequence is not.
    ConfirmKeyAction::FireNow => match app.confirm_kind() {
      ConfirmKind::DeleteWorktree => {
        if let Err(e) = app.confirm_delete() {
          app.status = format!("delete failed: {}", e);
        }
      }
      ConfirmKind::MergePr => app.confirm_merge(),
    },
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
    // Advance the elapsed duration of any Running check while the CI
    // overlay is up (Codex review #455) — same 200ms cadence, no-op
    // otherwise.
    app.tick_ci_overlay_durations();
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
    // #325: don't auto-refresh while a destructive overlay (exec picker /
    // clean report) is open — a re-list would drift the live selection /
    // active repo out from under the target the overlay captured at open.
    if !app.destructive_overlay_open() {
      app.maybe_auto_refresh(now);
    }

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
        // `mark_finished` also reaps the process group now (so a backgrounded
        // descendant is cleaned in the safe window, not after the linger).
        Some((PtyKind::Exec, false)) => {
          if let Some(p) = app.pty_overlay.as_mut() {
            p.mark_finished();
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
    // #325: suspended while a destructive overlay is open so the active repo
    // (and its config) can't swap under the captured exec/clean target.
    if !app.destructive_overlay_open() {
      app.sync_active_repo();
      // Issue #343: keep the details sidebar's git preview off the render path.
      // Runs after `sync_active_repo` so the active repo's `doctor.trunks` are
      // correct when a workspace-mode rebuild spawns. A no-op when the cache is
      // already current for the selection; otherwise it spawns one coalesced
      // worker (the render draws the placeholder until it lands).
      app.maybe_refresh_sidebar();
      // Agent-session detection (issue #408): same off-thread + coalesce
      // discipline as the sidebar — a no-op while the snapshot is fresh.
      app.maybe_refresh_agent_sessions();
    }

    // #420: the rich view's wrap budget is the terminal width. `Resize`
    // covers the changes, this covers the FIRST frame — an overlay opened
    // before any resize event would otherwise wrap against the 80-column
    // default whatever the real terminal is. A no-op once they agree.
    let size = terminal.size().unwrap_or_default();
    app.set_term_width(size.width);
    app.set_term_height(size.height);

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
    // Issue #343: also tighten it while a sidebar rebuild is in flight so the
    // "loading…" placeholder swaps to the real preview within ~50 ms instead of
    // up to a full 200 ms poll after a fast `j` / `k` on a large repo. Scoped
    // to the `Sidebar` slot on purpose — a multi-second `sync` / `push` / list
    // refresh doesn't need the loop spinning at 20 fps for its whole duration.
    let poll_ms = if app.view == View::Pty || app.tasks.is_loading(TaskKind::Sidebar) {
      50
    } else {
      200
    };
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
      // #420: the rich view wraps against the width the App carries, so a
      // resize that never reached it would leave the rows wrapped for the
      // previous terminal and the renderer would ellipsise the overflow.
      app.set_term_width(cols);
      app.set_term_height(rows);
      clear_without_cursor_query(terminal)?;
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
            // #436: the key path applies the contextual pre-resolution
            // (c → CI checks while the status pane is focused); the
            // palette path deliberately does not — its entries dispatch
            // by name (Codex review #455).
            let action = app.resolve_contextual_action(action);
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
      // Typing routes before the modal context here too (Codex review
      // #456) — see `App::settings_edit_input_key` (no-op unless an edit
      // is live, so plain Config navigation falls through).
      // The whole route lives in a testable App method (Codex review
      // #456): reserved typing, then the modal resolution, then the
      // AltGr reinjection of unresolved printables.
      View::Config if app.config_panel.editing.is_some() => app.handle_settings_edit_key(key),
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
      // Keys are inert while either mutation runs: the modal is showing a
      // loader for something already in flight.
      View::Confirm if app.is_delete_worktree_loading() || app.is_merge_loading() => {}
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
        Some(ModalAction::ConfirmCycleMethod) => app.cycle_merge_method(),
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
      // #515: the note editor. Every printable, `Enter`, `Backspace` and
      // `Delete` are text — `App::handle_note_key` routes them before the
      // modal resolution, which is what keeps `d` from reaching the global
      // delete verb while someone writes "done".
      View::Note => {
        if let NoteKey::LaunchEditor(command, path) = app.handle_note_key(key) {
          run_subshell(terminal, &command, &[path.as_os_str()], None, &mut app, "note")?;
          app.reload_note_after_editor();
        }
      }
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
          if let Some((argv, cwd, teardown)) = app.exec_picker_resolve() {
            let sz = terminal.size().unwrap_or_default();
            let inner_cols = ((sz.width as u32 * 90 / 100) as u16).saturating_sub(6).max(20);
            let inner_rows = ((sz.height as u32 * 90 / 100) as u16).saturating_sub(4).max(5);
            let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            match PtyOverlay::spawn(PtyKind::Exec, &argv_refs, &cwd, inner_cols, inner_rows) {
              // A containerised profile carries a teardown: the container
              // survives its own client, so closing the overlay must remove it.
              Ok(pty) => app.open_pty_overlay(match teardown {
                // Same cwd as the spawn, so a relative `runtime` resolves
                // identically on the way out.
                Some(argv) => pty.with_teardown(argv, cwd.clone()),
                None => pty,
              }),
              Err(e) => {
                // `spawn` can fail AFTER the child is launched (the reader
                // clone and the writer take are both fallible), and a
                // containerised run that reached the daemon would then keep
                // going with no overlay to close. Tear it down here too.
                if let Some(argv) = teardown {
                  crate::tui::state::pty_overlay::run_teardown_now(&argv, &cwd);
                }
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
      // Detail overlay (issue #408): j/k move the selection, `a` pins the
      // selected session, `d` unpins, `i` opens the attach-by-id prompt
      // (user feedback 2026-07-22). While the prompt is active, keys are
      // captured as query input (palette convention): printable chars type,
      // Backspace pops, arrows move the candidate highlight, Enter
      // attaches, Esc falls back to the list.
      // Issue #436: the same shell serves two consumers — route the input
      // prompt AND the list verbs by `detail_overlay.kind` (agents attach
      // by id; CI checks filter their own rows and open URLs).
      View::DetailOverlay if app.detail_overlay.mode == crate::tui::state::detail_overlay::DetailMode::Input => {
        let ci = app.detail_overlay.kind == crate::tui::state::detail_overlay::DetailKind::CiChecks;
        match key.code {
          KeyCode::Esc if ci => app.ci_input_cancel(),
          KeyCode::Esc => app.agent_input_cancel(),
          KeyCode::Enter if ci => match app.ci_input_selected_url() {
            Some(url) => open_url(&url, &mut app),
            // The method flips back to List only when a row WAS picked —
            // report the missing URL like the List-mode Enter does (Codex
            // review #455). A query with no match keeps the filter open.
            None if app.detail_overlay.mode == crate::tui::state::detail_overlay::DetailMode::List => {
              app.status = "this check exposes no details URL".into()
            }
            None => {}
          },
          KeyCode::Enter => app.agent_input_submit(),
          KeyCode::Backspace if ci => app.ci_input_pop(),
          KeyCode::Backspace => app.agent_input_pop(),
          KeyCode::Down if ci => app.ci_input_next(),
          KeyCode::Down => app.agent_input_next(),
          KeyCode::Up if ci => app.ci_input_prev(),
          KeyCode::Up => app.agent_input_prev(),
          KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if ci {
              app.ci_input_push(c)
            } else {
              app.agent_input_push(c)
            }
          }
          _ => {}
        }
      }
      View::DetailOverlay if app.detail_overlay.kind == crate::tui::state::detail_overlay::DetailKind::CiChecks => {
        match app.resolve_modal(KeyContext::CiChecks, key) {
          Some(ModalAction::CiChecksClose) => app.close_detail_overlay(),
          Some(ModalAction::CiChecksNext) => app.detail_overlay.select_next(),
          Some(ModalAction::CiChecksPrev) => app.detail_overlay.select_prev(),
          Some(ModalAction::CiChecksOpen) => match app.ci_selected_url() {
            Some(url) => open_url(&url, &mut app),
            None => app.status = "this check exposes no details URL".into(),
          },
          Some(ModalAction::CiChecksFilter) => app.ci_input_open(),
          // Validation feedback on PR #455: `f` re-fetches the PR from
          // inside the overlay; the landing refreshes the rows in place.
          Some(ModalAction::CiChecksRefresh) => app.ci_checks_refresh(),
          _ => {}
        }
      }
      // #420: the rich view shares the shell with the agent pane but not
      // its verbs, so it resolves in its own context.
      View::DetailOverlay
        if matches!(
          app.detail_overlay.kind,
          crate::tui::state::detail_overlay::DetailKind::RichIssue
            | crate::tui::state::detail_overlay::DetailKind::RichPr
        ) =>
      {
        match app.resolve_modal(KeyContext::RichView, key) {
          Some(ModalAction::RichViewClose) => app.close_detail_overlay(),
          Some(ModalAction::RichViewNext) => app.detail_overlay.select_next(),
          Some(ModalAction::RichViewPrev) => app.detail_overlay.select_prev(),
          Some(ModalAction::RichViewOpen) => match app.rich_selected_url() {
            Some(url) => open_url(&url, &mut app),
            None => app.status = "this row has nothing to open".into(),
          },
          Some(ModalAction::RichViewRefresh) => app.rich_view_refresh(),
          Some(ModalAction::RichViewTab) => app.rich_view_next_tab(),
          Some(ModalAction::RichViewYankUrl) => match app.rich_yank_url() {
            Some(url) => copy_text_to_clipboard(&mut app, &url, "url copied"),
            None => app.status = "no url to copy".into(),
          },
          Some(ModalAction::RichViewYankBody) => match app.rich_yank_body() {
            Some(body) => copy_text_to_clipboard(&mut app, &body, "description copied"),
            None => app.status = "this one has no description".into(),
          },
          Some(ModalAction::RichViewMerge) => app.enter_confirm_merge(),
          Some(ModalAction::RichViewHalfDown) => {
            let n = app.rich_half_page();
            app.detail_overlay.select_page_down(n);
          }
          Some(ModalAction::RichViewHalfUp) => {
            let n = app.rich_half_page();
            app.detail_overlay.select_page_up(n);
          }
          Some(ModalAction::RichViewTop) => app.detail_overlay.select_first(),
          Some(ModalAction::RichViewBottom) => app.detail_overlay.select_last(),
          Some(ModalAction::RichViewCiChecks) => app.enter_ci_checks(),
          Some(ModalAction::RichViewLeft) => app.rich_view_scroll_left(),
          Some(ModalAction::RichViewRight) => app.rich_view_scroll_right(),
          _ => {}
        }
      }
      View::DetailOverlay => match app.resolve_modal(KeyContext::Detail, key) {
        Some(ModalAction::DetailClose) => app.close_detail_overlay(),
        Some(ModalAction::DetailSelectNext) => app.detail_overlay.select_next(),
        Some(ModalAction::DetailSelectPrev) => app.detail_overlay.select_prev(),
        Some(ModalAction::DetailAttach) => app.attach_selected_agent(),
        Some(ModalAction::DetailDetach) => app.detach_selected_agent(),
        Some(ModalAction::DetailInput) => app.open_agent_input(),
        _ => {}
      },
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
      // Typing routes before the modal context (Codex review #456) — see
      // `App::palette_input_key` for the reserved-typing contract (a
      // testable method, per the repo's TDD rule for event-loop routes).
      // Typing routes before the modal context (Codex review #456) — see
      // `App::palette_input_key` for the reserved-typing contract (a
      // testable method, per the repo's TDD rule for event-loop routes).
      // Plain `if`, not a match guard: guards cannot borrow mutably.
      // Typing routes before the modal context (Codex review #456) — see
      // `App::palette_input_key` for the reserved-typing contract (a
      // testable method; an `if` in the arm body because match guards
      // cannot borrow mutably). The charset / swallow rules for plain
      // characters live in that method now; only Ctrl-modified keys and
      // non-character keys reach the modal resolution.
      View::CommandPalette => {
        if !app.palette_input_key(key) {
          match app.resolve_modal(KeyContext::CommandPalette, key) {
            Some(ModalAction::CommandPaletteClose) => app.close_command_palette(),
            Some(ModalAction::CommandPaletteAccept) => {
              if let Some(action) = app.accept_command_palette() {
                run_palette_action(terminal, &mut app, action)?;
              }
            }
            Some(ModalAction::CommandPalettePrev) => app.palette_cycle_up(),
            Some(ModalAction::CommandPaletteNext) => app.palette_cycle_down(),
            // Unresolved keys fall back to typing (AltGr / modified
            // Backspace parity — Codex #456); testable App method.
            _ => app.palette_unresolved_fallback(key),
          }
        }
      }
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
    app.status = "workspace: selected repo is unavailable (moved/deleted?); press r to refresh".into();
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
    // Issue #437: Working Tree pane scroll — no-ops unless the status
    // pane holds the focus (gate lives on the `App` methods).
    Action::WtScrollDown => app.wt_scroll_down(),
    Action::WtScrollUp => app.wt_scroll_up(),
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
      // #515 review: the path goes through as an `OsStr`. `display()` is
      // lossy, so a repo path carrying non-UTF-8 bytes handed the editor a
      // DIFFERENT path than the one gwm resolved.
      Some(OpenTarget::Editor { path, command }) => {
        run_subshell(terminal, &command, &[path.as_os_str()], None, app, "editor")?
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
    // #484: `Space` marks the cursor row. Picker-gated — `gwm switch` picks
    // exactly one path, so a mark set has nothing to act on there.
    Action::ToggleSelect if !app.picker_mode => app.toggle_select(),
    Action::Bootstrap if !app.picker_mode => app.bootstrap_selected(),
    // Issue #258: `gwm sync` of the selected worktree, off-thread.
    Action::Sync if !app.picker_mode => app.request_sync(),
    // #290: `p` pulls, `P` pushes, both off-thread.
    Action::Pull if !app.picker_mode => app.request_pull(),
    Action::Push if !app.picker_mode => app.request_push(),
    // #290: `c` opens the branch-rename modal.
    Action::EditWorktree if !app.picker_mode => app.enter_edit_worktree(),
    // #515: `N` opens the selected worktree's note in $EDITOR, through the
    // same suspend-and-restore loop `o` uses in `mode = "editor"`. The
    // marker is re-read for that one row on the way back — the editor may
    // have created the note, or emptied it.
    // #515: `N` opens the note in the TUI. It used to suspend the whole
    // terminal to spawn `$EDITOR`, which is a heavier gesture than the
    // three lines a note usually is; `Ctrl+e` inside the modal still gets
    // there in one keystroke.
    Action::EditNote if !app.picker_mode => app.open_note_editor(),
    Action::CiChecks if !app.picker_mode => app.enter_ci_checks(),
    // #420: `I` opens the rich PR / issue view on the linked side.
    Action::RichView if !app.picker_mode => app.enter_rich_view(),
    // #551 validation feedback: merge the selected worktree's linked PR.
    Action::MergePr if !app.picker_mode => app.enter_confirm_merge(),
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
    // Issue #408: `a` opens the agent-session detail overlay. Read-only, but
    // picker-gated like the other overlays — the stripped-down `gwm switch`
    // picker advertises pick/cancel only.
    Action::AgentSessions if !app.picker_mode => app.open_agent_overlay(),
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
      "`{}` not on $PATH: install it or change [review]/[git_tui] in .gwm.toml",
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
    clear_without_cursor_query(terminal)?;

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
  args: &[&std::ffi::OsStr],
  cwd: Option<&std::path::Path>,
  app: &mut App,
  label: &str,
) -> Result<()> {
  // Split before spawning (issue #515, Codex review PR #530): `cmd` comes
  // from `editor_cmd` / `shell_cmd` or `$EDITOR` / `$SHELL`, which are shell
  // lines, so `code --wait` is a program plus an argument rather than an
  // executable whose name contains a space.
  let argv = launch_argv(cmd);
  let (program, leading) = argv.split_first().expect("launch_argv never returns empty");

  disable_raw_mode()?;
  execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
  terminal.show_cursor()?;

  let mut command = std::process::Command::new(program);
  command.args(leading);
  command.args(args);
  if let Some(dir) = cwd {
    command.current_dir(dir);
  }
  route_fullscreen_child_stdout(&mut command);
  let spawn = command.status();

  // Always restore the TUI, even if the child failed to spawn or exited non-zero.
  enable_raw_mode()?;
  execute!(terminal.backend_mut(), EnterAlternateScreen, EnableMouseCapture)?;
  clear_without_cursor_query(terminal)?;

  match spawn {
    Ok(s) if s.success() => app.status = format!("{} exited ok ({})", label, cmd),
    Ok(s) => app.status = format!("{} exited with code {:?}", label, s.code()),
    Err(e) => app.status = format!("failed to launch {} ({}): {}", label, cmd, e),
  }
  Ok(())
}

/// Split a configured launch command into argv.
///
/// `$EDITOR`, `$SHELL` and their `.gwm.toml` overrides are shell lines by
/// convention — git, cargo and systemctl all word-split them — and this repo
/// already reads `[review]` tools and hook `run =` lines the same way
/// (`launcher.rs`, `doctor.rs`). Handing the whole string to `Command::new`
/// looked for an executable literally named `code --wait`, so an editor
/// configured with a flag could never launch (issue #515, Codex review on PR
/// #530). A program path that genuinely contains a space is quoted, which the
/// splitter honours.
///
/// A string that already **names a file** is not a shell line and is handed
/// over whole. Word-splitting is POSIX and filenames are not: `shell_words`
/// drops an unprotected backslash, so `EDITOR=C:\Tools\nvim.exe` — an
/// absolute path `Command::new` launched fine before any splitting existed —
/// came back as `C:Toolsnvim.exe` and stopped launching (Codex review, PR
/// #530). One `is_file` before the splitter is what keeps that working. It
/// cannot see a path that is not on *this* machine, nor one carrying flags;
/// those are written quoted, and POSIX preserves a backslash inside double
/// quotes unless it precedes `$`, `` ` ``, `"`, `\` or a newline.
///
/// Never empty: an unbalanced quote or a blank value falls back to the raw
/// string, which keeps the previous behaviour and lets the spawn failure name
/// what was actually configured.
pub fn launch_argv(cmd: &str) -> Vec<String> {
  if std::path::Path::new(cmd).is_file() {
    return vec![cmd.to_string()];
  }
  match shell_words::split(cmd) {
    Ok(argv) if !argv.is_empty() => argv,
    _ => vec![cmd.to_string()],
  }
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
    app.status = format!("macro{} not configured: add [tui.macro{}] to .gwm.toml", n, n);
    return Ok(());
  };
  use crate::multiplexer::{detect_split_command, Multiplexer};
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
    match detect_split_command(
      &label,
      &path,
      std::env::var("TMUX").ok(),
      std::env::var("ZELLIJ").ok(),
      std::env::var("HERDR_ENV").ok(),
    ) {
      // Herdr is detected but its argv is deliberately dropped (#588): a
      // macro needs the pane to run a command, and `herdr pane split` has no
      // trailing-command form the way `tmux split-window <cmd>` and
      // `zellij action new-pane -- <cmd>` do. Running one takes
      // `herdr pane run <pane-id> <cmd>`, and the id only comes back in the
      // JSON `pane split` prints, so it is two processes and a parse, not an
      // argv (#599). Splitting anyway would open an empty pane and silently
      // drop the macro, so the PTY overlay stays the honest fallback and the
      // status says why.
      Some((Multiplexer::Herdr, _)) => {
        app.status = format!("macro{}: herdr panes take no command; falling back to PTY overlay", n);
        None
      }
      Some((_, cmd)) => Some(cmd),
      None => {
        app.status = format!("macro{}: no multiplexer; falling back to PTY overlay", n);
        None
      }
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

/// Put `text` on the clipboard, honouring `[tui] clipboard` (issue #367).
///
/// The single chokepoint for every yank action, so routing lives here rather
/// than in each caller. [`crate::clipboard::plan_clipboard_write`] makes the
/// decision (it is pure and unit-tested); this function only performs it.
///
/// `success` is the status-bar label on a clean copy — suffixed with the path
/// that actually ran (`(osc52)` / `(pbcopy)`). That suffix is load-bearing:
/// OSC52 is never acknowledged by the terminal, so when a paste comes back
/// empty the status line is the only clue about which route was taken.
fn copy_text_to_clipboard(app: &mut App, text: &str, success: &str) {
  use crate::clipboard::{plan_clipboard_write, ClipboardPlan};
  use std::io::Write;

  let plan = plan_clipboard_write(
    text,
    app.config.tui.clipboard,
    // `$SSH_TTY` covers an interactive login; `$SSH_CONNECTION` also covers
    // the cases where no tty was allocated.
    std::env::var_os("SSH_TTY").is_some() || std::env::var_os("SSH_CONNECTION").is_some(),
    crate::multiplexer::detect_tmux(std::env::var("TMUX").ok()),
    std::env::var_os("STY").is_some(),
  );
  match plan {
    ClipboardPlan::Osc52(bytes) => {
      // The TUI renders to stderr, so the sequence goes to the same fd. It is
      // an escape sequence, not cells, so ratatui's next draw won't erase it —
      // but it must be flushed, or it sits in the buffer until the next frame.
      let mut err = std::io::stderr();
      match err.write_all(&bytes).and_then(|_| err.flush()) {
        Ok(()) => app.status = format!("{} (osc52)", success),
        Err(e) => app.status = format!("osc52 write failed: {}", e),
      }
      return;
    }
    ClipboardPlan::TooLarge { bytes } => {
      // Refuse rather than emit a sequence the terminal will truncate into
      // corrupt paste content. Round the reported size *up*: truncating
      // division renders 65_537 bytes as "64 KiB > 64 KiB", which reads as a
      // bug in the check rather than as a reason for the refusal.
      app.status = format!(
        "too large for osc52 ({} KiB > {} KiB): set [tui] clipboard = \"tools\"",
        bytes.div_ceil(1024),
        crate::clipboard::MAX_OSC52_BYTES / 1024
      );
      return;
    }
    ClipboardPlan::Tools => {}
  }

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
