//! Safety countdown for the destructive-action confirm modal (issue #30,
//! extracted from `tui::app::App` per #125 / #102).
//!
//! Pure state — no I/O, no `App` dependency. The `App` orchestrator owns
//! the side effects (status messages, the `worktree::remove` call, the
//! view transition back to `View::List`); this module owns the timer and
//! returns enums describing what the orchestrator should do next.

use std::time::{Duration, Instant};

/// Outcome of pressing `y` / Enter on the confirm overlay. The event loop
/// matches on this to decide whether to fire the delete immediately or
/// wait for the countdown tick.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ConfirmKeyAction {
  /// Classic modal (delete_branch OFF, or countdown_secs = 0). The caller
  /// must invoke the destructive action right away.
  FireNow,
  /// Countdown just got armed by this keystroke.
  Armed,
  /// Countdown was armed and got disarmed by this second keystroke.
  Disarmed,
}

/// State of the safety countdown after a tick. Returned by
/// [`ConfirmModal::tick`] so the event loop can branch without reaching
/// into the modal's internals.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum CountdownTickOutcome {
  /// No countdown was running (modal closed, or classic confirm modal).
  NotArmed,
  /// Countdown is still running — the loop should keep drawing the bar.
  Pending,
  /// Countdown has elapsed; the caller must execute the destructive
  /// action and clear the modal. The modal has already reset its own
  /// timer state so a re-entrant tick returns `NotArmed`.
  ReadyToFire,
}

/// Safety countdown state for the confirm modal. `Default` opens the
/// modal in "classic / not-armed" state.
#[derive(Debug, Default)]
pub struct ConfirmModal {
  /// Anchor for the safety countdown (issue #30). When `Some`, the modal
  /// renders a progress bar and [`ConfirmModal::tick`] decrements it
  /// before returning `ReadyToFire`. `None` = modal closed, classic mode,
  /// or armed-but-just-disarmed by a second `y` press.
  pub started_at: Option<Instant>,
}

impl ConfirmModal {
  pub fn new() -> Self {
    Self::default()
  }

  /// Reset to the freshly-opened state (timer cleared, classic mode).
  /// Called by the orchestrator when the modal opens or dismisses.
  pub fn reset(&mut self) {
    self.started_at = None;
  }

  /// Handle a `y` / Enter press. `total` is the configured countdown
  /// duration; `Duration::ZERO` encodes "classic single-keystroke
  /// confirm" (delete_branch_on_remove OFF, or confirm_countdown_secs = 0).
  pub fn press_y(&mut self, now: Instant, total: Duration) -> ConfirmKeyAction {
    if total.is_zero() {
      // Defensive: if `started_at` was set by a prior code path that
      // armed before checking `total` (or by the config flipping to
      // `confirm_countdown_secs = 0` mid-modal), clear it so the
      // invariant "classic mode is never armed" holds. Copilot review
      // on PR #131 flagged this — without the clear, a future caller
      // could leak armed state across a press_y that returns FireNow.
      self.started_at = None;
      return ConfirmKeyAction::FireNow;
    }
    if self.started_at.is_some() {
      self.started_at = None;
      ConfirmKeyAction::Disarmed
    } else {
      self.started_at = Some(now);
      ConfirmKeyAction::Armed
    }
  }

  /// Handle the dismissal keys (`n` / `Esc`). Always clears the timer.
  pub fn dismiss(&mut self) {
    self.started_at = None;
  }

  /// Tick the countdown forward. Called from the event loop on every
  /// poll-timeout iteration (every 200ms). Returns `ReadyToFire` exactly
  /// once when the timer crosses `total`; the modal's `started_at` is
  /// cleared at that point.
  pub fn tick(&mut self, now: Instant, total: Duration) -> CountdownTickOutcome {
    let Some(started) = self.started_at else {
      return CountdownTickOutcome::NotArmed;
    };
    if total.is_zero() {
      // Defensive: if config changed mid-modal to 0s, treat as classic.
      self.started_at = None;
      return CountdownTickOutcome::NotArmed;
    }
    if now.saturating_duration_since(started) < total {
      CountdownTickOutcome::Pending
    } else {
      self.started_at = None;
      CountdownTickOutcome::ReadyToFire
    }
  }

  /// Countdown progress in `[0.0, 1.0]`. `0.0` when not armed, `1.0` once
  /// elapsed. Used by the UI to draw the gauge.
  pub fn progress(&self, now: Instant, total: Duration) -> f64 {
    let Some(started) = self.started_at else {
      return 0.0;
    };
    if total.is_zero() {
      return 0.0;
    }
    let elapsed = now.saturating_duration_since(started).as_secs_f64();
    (elapsed / total.as_secs_f64()).min(1.0)
  }

  /// Seconds remaining (rounded up to the next whole second) for the UI
  /// label. `0` when not armed or when the countdown has elapsed.
  pub fn remaining_secs(&self, now: Instant, total: Duration) -> u64 {
    let Some(started) = self.started_at else {
      return 0;
    };
    if total.is_zero() {
      return 0;
    }
    let remaining = total.saturating_sub(now.saturating_duration_since(started));
    if remaining.is_zero() {
      return 0;
    }
    let extra = if remaining.subsec_nanos() > 0 { 1 } else { 0 };
    remaining.as_secs() + extra
  }
}
