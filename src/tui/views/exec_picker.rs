//! The Exec picker view (issues #325, #421, #635): its state, its rendering
//! and its key handling, in one module.
//!
//! A sorted list of `[exec.profiles.*]` names, a wrapping highlight, and
//! the worktree it was opened for. `Enter` accepts the highlight, `Esc`
//! cancels; the `j`/`k`/arrow navigation lives in
//! [`crate::tui::modal_keymap`] under `KeyContext::ExecPicker`.
//!
//! The picker resolves the highlighted profile to an argv; the run loop
//! owns the side effect, spawning that argv in a PTY overlay. That split
//! is why [`App::handle_exec_picker_key`] returns an
//! [`ExecPickerKey`] rather than acting: a handler that spawned a pty
//! could not be driven from a test.
//!
//! ## What the picker captures at open, and why
//!
//! Three fields on [`ExecPicker`] exist so that `Enter` runs in *this*
//! worktree against *this* config rather than whatever is live later: an
//! auto-refresh moves the selection, and in workspace mode
//! `sync_active_repo` swaps `App::config` to another repo, both while the
//! overlay sits open (Codex #333 review). They lived flat on `App` as
//! `exec_picker_cfg` / `_common_dir` / `_container_seq` until #635 —
//! satellites of a state type that was already extracted, which is the
//! shape that made `App` 85 fields wide.
//!
//! `container_seq` is the one that must NOT be reset by
//! [`ExecPicker::open`]: it is the monotonic half of an overlay run's
//! container name (issue #421), and the pid alone would collide across two
//! overlays opened on the same worktree within one session. It is
//! initialised once, with the `App`, and counts for that `App`'s lifetime.

use super::super::app::{App, ExecPickerKey, View};
use super::super::modal_keymap::{KeyContext, ModalAction};
use super::super::mouse::{MouseMap, RowList};
use super::super::ui::{
  centered_abs, overlay_modal_width, picker_lines, picker_window, push_modal_hint, HintContext, ModalFrame,
};
use crate::config::ExecConfig;
use crossterm::event::KeyEvent;
use ratatui::{layout::Rect, text::Line, widgets::Paragraph, Frame};
use std::path::{Path, PathBuf};

/// Pure state for the exec profile picker. `Default` is an empty, closed
/// picker (no profiles, no target, highlight at 0).
#[derive(Debug, Default)]
pub struct ExecPicker {
  /// Exec profile names from `[exec.profiles.*]`, in the sorted order the
  /// orchestrator hands them over (a `BTreeMap` key walk).
  profiles: Vec<String>,
  /// Highlighted row index. Always a valid index into `profiles` while
  /// non-empty; the `next`/`prev` wrappers keep it in range.
  selected: usize,
  /// The worktree directory the picker was opened for. Captured at open so
  /// `Enter` runs the profile in *that* worktree even if an auto-refresh
  /// drifts the live selection while the overlay sits open (Codex #333).
  cwd: Option<PathBuf>,
  /// The `[exec]` config captured when the picker opened (issue #325). In
  /// workspace mode `sync_active_repo` can swap `App::config` to another
  /// repo while the overlay is open, so `Enter` resolves the argv against
  /// this snapshot — the active repo's `[exec]` at open time — not the live
  /// config (Codex #333 review).
  cfg: ExecConfig,
  /// The active repo's `commondir` (`<main>/.git`), captured alongside
  /// [`Self::cfg`] for the same reason. Only read when the picked profile
  /// carries a `[container]` block (issue #421), which mounts it so git
  /// answers inside the container.
  common_dir: PathBuf,
  /// Monotonic counter behind the container name of an overlay run (issue
  /// #421). The pid alone would collide across two overlays opened on the
  /// same worktree within one session.
  ///
  /// Deliberately NOT touched by [`Self::open`], and that is the whole
  /// contract: it counts for the lifetime of the `App` that owns this
  /// picker, exactly as it did while it was `App::exec_container_seq`.
  /// Reset it on open and two container runs in one session can collide.
  container_seq: u64,
}

impl ExecPicker {
  pub fn new() -> Self {
    Self::default()
  }

  /// (Re)populate the picker from a list of profile names + the worktree it
  /// targets, resetting the highlight to the top. Called by the
  /// orchestrator's `enter_exec_picker` each time the overlay opens.
  pub fn open(&mut self, profiles: Vec<String>, cwd: PathBuf) {
    self.profiles = profiles;
    self.selected = 0;
    self.cwd = Some(cwd);
  }

  /// Capture the `[exec]` config and the active repo's `commondir` the
  /// picked profile will be resolved against (issues #325, #421). Called
  /// by [`App::enter_exec_picker`] alongside [`Self::open`], and separate
  /// from it so the pure-state tests can drive the picker without a
  /// `Config` in reach.
  pub fn capture_context(&mut self, cfg: ExecConfig, common_dir: PathBuf) {
    self.cfg = cfg;
    self.common_dir = common_dir;
  }

  /// The worktree directory the picker was opened for, if any.
  pub fn cwd(&self) -> Option<&Path> {
    self.cwd.as_deref()
  }

  /// The profile names, in display order.
  pub fn profiles(&self) -> &[String] {
    &self.profiles
  }

  /// `true` when no profiles are loaded (the orchestrator refuses to open
  /// the overlay in this state, but the picker stays well-defined).
  pub fn is_empty(&self) -> bool {
    self.profiles.is_empty()
  }

  /// The highlighted row index (clamped to `profiles`).
  pub fn selected_index(&self) -> usize {
    self.selected
  }

  /// The highlighted profile name, or `None` when the picker is empty.
  pub fn selected_profile(&self) -> Option<&str> {
    self.profiles.get(self.selected).map(String::as_str)
  }

  /// Move the highlight down one row, wrapping to the top. No-op when
  /// empty.
  /// Point the highlight straight at `index` (issue #624 — a click lands on
  /// an arbitrary row, not one step away). Out-of-range is ignored rather
  /// than clamped: the caller is reporting where the user clicked, and a
  /// click past the last profile picked nothing.
  pub fn select_index(&mut self, index: usize) {
    if index < self.profiles.len() {
      self.selected = index;
    }
  }

  pub fn next(&mut self) {
    if self.profiles.is_empty() {
      return;
    }
    self.selected = (self.selected + 1) % self.profiles.len();
  }

  /// Move the highlight up one row, wrapping to the bottom. No-op when
  /// empty.
  pub fn prev(&mut self) {
    if self.profiles.is_empty() {
      return;
    }
    self.selected = (self.selected + self.profiles.len() - 1) % self.profiles.len();
  }
}

// ── Rendering ──────────────────────────────────────────────────────────────

/// Render the exec profile picker overlay (issue #325). A small centred
/// modal listing the `[exec.profiles.*]` names; the highlighted row reads in
/// the accent (with a selection bar) and a `▸` marker, the rest muted. The
/// list is aligned, same-width, and scrolls to keep the highlight in view.
/// `Enter` resolves the highlight and the run loop spawns it in a PTY overlay.
pub(crate) fn draw_exec_picker(f: &mut Frame, app: &App, map: &mut MouseMap) {
  let accent = app.theme.accent;
  let term = f.area();
  let width = overlay_modal_width(term.width);
  let frame = ModalFrame::resolve_for(app, accent);
  let inner = width.saturating_sub(frame.cols()) as usize;
  let mut lines: Vec<Line<'static>> = Vec::new();
  // Leave room for the title + hint + borders; the picker scrolls past that.
  let max_visible = (term.height as usize).saturating_sub(8).max(3);
  let labels: Vec<&str> = app.exec_picker.profiles().iter().map(String::as_str).collect();
  lines.extend(picker_lines(
    &labels,
    app.exec_picker.selected_index(),
    max_visible,
    inner,
    &app.theme,
  ));
  push_modal_hint(
    &mut lines,
    HintContext::ExecPicker,
    &app.keymap,
    &app.modal_keymap,
    &app.theme,
  );
  let height = lines.len() as u16 + frame.rows();
  let area = centered_abs(width, height, term);
  let content = frame.render(f, map, area, "Run an exec profile", None);
  f.render_widget(Paragraph::new(lines), content);

  // The rows `picker_lines` painted, resolved through the same
  // `picker_window` it used: a `↑ N more` marker takes the first line when
  // the window is scrolled, so the profiles start one row lower (issue #624).
  let (start, end) = picker_window(labels.len(), app.exec_picker.selected_index(), max_visible);
  if end > start {
    map.push_rows(
      Rect {
        y: content.y + u16::from(start > 0),
        height: (end - start) as u16,
        ..content
      },
      RowList::ExecPicker,
      start,
      labels.len(),
    );
  }
}

// ── Behaviour ──────────────────────────────────────────────────────────────

impl App {
  /// Open the exec profile picker (issue #325). Populates it from
  /// `[exec.profiles.*]` and switches to [`View::ExecPicker`]. Refuses
  /// (status-bar message, no transition) when nothing is selected or no
  /// exec profiles are configured — there is nothing to pick.
  pub fn enter_exec_picker(&mut self) {
    let Some(cwd) = self.selected().map(|wt| wt.path.clone()) else {
      self.status = "nothing selected".into();
      return;
    };
    let names: Vec<String> = self.config.exec.profiles.keys().cloned().collect();
    if names.is_empty() {
      self.status = "no [exec.profiles] configured: add one to .gwm.toml".into();
      return;
    }
    // Capture the target worktree path AND the active repo's `[exec]` config
    // now: an auto-refresh can drift the live selection (and, in workspace
    // mode, the active repo) while the picker is open, so `Enter` must run in
    // *this* worktree against *this* config — not whatever is live later
    // (Codex #333 review).
    self
      .exec_picker
      .capture_context(self.config.exec.clone(), self.repo.commondir().to_path_buf());
    self.exec_picker.open(names, cwd);
    self.view = View::ExecPicker;
  }

  /// Handle a key inside the exec picker overlay (issue #325). The
  /// testable handler owns the highlight movement; the run loop owns the
  /// two side effects (resolve + spawn, or close). Keys resolve through
  /// [`KeyContext::ExecPicker`] so they honour `[tui.keys.modal.exec]`.
  pub fn handle_exec_picker_key(&mut self, key: KeyEvent) -> ExecPickerKey {
    match self.resolve_modal(KeyContext::ExecPicker, key) {
      Some(ModalAction::ExecPickerCancel) => ExecPickerKey::Cancel,
      Some(ModalAction::ExecPickerAccept) => ExecPickerKey::Submit,
      Some(ModalAction::ExecPickerNext) => {
        self.exec_picker.next();
        ExecPickerKey::Handled
      }
      Some(ModalAction::ExecPickerPrev) => {
        self.exec_picker.prev();
        ExecPickerKey::Handled
      }
      _ => ExecPickerKey::Handled,
    }
  }

  /// Resolve the highlighted exec profile to an `(argv, cwd, teardown)` triple
  /// for the run loop to spawn in a PTY overlay (issue #325). `None` (with a
  /// status-bar message) when nothing is selected or the profile fails to
  /// resolve — e.g. an empty `command` array. The argv is the frozen
  /// `[exec.profiles.<name>].command` verbatim (no shell), matching the
  /// 1.0 exec contract; the run loop spawns `argv[0]` directly.
  ///
  /// `teardown` is `Some` only for a containerised profile (issue #421):
  /// killing the pty leader kills the `docker` client, never the container it
  /// asked the daemon for, so the overlay removes it by name on close.
  pub fn exec_picker_resolve(&mut self) -> Option<(Vec<String>, PathBuf, Option<Vec<String>>)> {
    let profile = self.exec_picker.selected_profile()?.to_string();
    // Resolve against the worktree captured when the picker opened, NOT the
    // live selection (which an auto-refresh may have drifted) — #333 review.
    let Some(cwd) = self.exec_picker.cwd().map(Path::to_path_buf) else {
      self.status = "nothing selected".into();
      return None;
    };
    // Resolve against the `[exec]` config captured at open, not the live one.
    let mut teardown: Option<Vec<String>> = None;
    match crate::exec::resolve_exec_command(Some(&profile), &[], &self.exec_picker.cfg) {
      Ok(mut argv) => {
        // Pin a worktree-relative executable (`./run.sh`, `scripts/build`) to
        // the captured worktree, exactly like the CLI exec path — otherwise
        // `argv[0]` would resolve against gwm's own cwd (Codex #333 review).
        // A bare command (`cargo`) or an absolute path is returned unchanged
        // (PATH lookup / as-is).
        if let Some(first) = argv.first_mut() {
          *first = crate::exec::resolve_program(&cwd, first).to_string_lossy().into_owned();
        }
        // A profile carrying `[container]` runs in a container here too
        // (issue #421) — the same profile must not mean "on the host" in the
        // TUI and "in a container" on the CLI. The wrap comes AFTER the
        // relative-program anchoring: host paths are mirrored inside the
        // container, so the anchored absolute path is valid on both sides.
        match crate::exec::resolve_exec_container(Some(&profile), &self.exec_picker.cfg) {
          Ok(Some(container)) => {
            match crate::exec::ContainerPlan::resolve(container, &self.exec_picker.common_dir, |bin| {
              which::which(bin).is_ok()
            }) {
              // `wrap_interactive`: this overlay spawns into a real pty, so
              // the container gets `-i -t` and a REPL / debugger / prompting
              // command keeps working, exactly as it does when the same
              // profile runs on the host here.
              Ok(plan) => {
                self.exec_picker.container_seq += 1;
                let name = crate::exec::container_run_name(&cwd, std::process::id(), self.exec_picker.container_seq);
                match plan.wrap_interactive(&cwd, &argv, &name) {
                  Ok(wrapped) => {
                    argv = wrapped;
                    teardown = Some(plan.container_teardown_argv(&name));
                  }
                  Err(e) => {
                    self.status = format!("exec profile {profile:?}: {e}");
                    return None;
                  }
                }
              }
              Err(e) => {
                self.status = format!("exec profile {profile:?}: {e}");
                return None;
              }
            }
          }
          Ok(None) => {}
          Err(e) => {
            self.status = format!("exec profile {profile:?}: {e}");
            return None;
          }
        }
        Some((argv, cwd, teardown))
      }
      Err(e) => {
        self.status = format!("exec profile {profile:?}: {e}");
        None
      }
    }
  }

  /// Close the exec picker without running anything (issue #325). Returns
  /// to [`View::List`].
  pub fn close_exec_picker(&mut self) {
    if self.view == View::ExecPicker {
      self.view = View::List;
    }
  }
}
