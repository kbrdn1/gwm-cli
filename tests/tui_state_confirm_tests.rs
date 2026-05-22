//! Unit tests for the pure `ConfirmModal` sub-struct (issue #125).
//!
//! These tests exercise the timer state in isolation, without spinning up
//! a full `App` (which would pull in libgit2, the config loader, and the
//! worktree list). The same behavioural contract is also exercised at the
//! `App` level in `tui_app_tests.rs` — the two test surfaces converge
//! after the decomposition, but the unit tests below are what
//! reviewers should read to understand the modal's state machine.

use gwm::tui::state::confirm::{ConfirmKeyAction, ConfirmModal, CountdownTickOutcome};
use std::time::{Duration, Instant};

#[test]
fn classic_mode_fires_immediately_on_first_y() {
  // Total = ZERO encodes the "classic single-keystroke confirm" mode
  // (delete_branch_on_remove OFF, or confirm_countdown_secs = 0). The
  // modal must return FireNow on the very first press_y so the caller
  // executes the delete without waiting for a tick.
  let mut modal = ConfirmModal::new();
  let action = modal.press_y(Instant::now(), Duration::ZERO);
  assert_eq!(action, ConfirmKeyAction::FireNow);
  assert!(
    modal.started_at.is_none(),
    "classic mode must never arm the countdown timer"
  );
}

#[test]
fn countdown_mode_arms_on_first_y_and_disarms_on_second() {
  // total > 0 → countdown mode. Two-stage commit: first y arms, second
  // y within the window cancels.
  let mut modal = ConfirmModal::new();
  let total = Duration::from_secs(3);
  let t0 = Instant::now();
  let action = modal.press_y(t0, total);
  assert_eq!(action, ConfirmKeyAction::Armed);
  assert_eq!(modal.started_at, Some(t0));

  // Second press 1s later disarms.
  let t1 = t0 + Duration::from_secs(1);
  let action = modal.press_y(t1, total);
  assert_eq!(action, ConfirmKeyAction::Disarmed);
  assert!(modal.started_at.is_none());
}

#[test]
fn dismiss_clears_armed_countdown() {
  let mut modal = ConfirmModal::new();
  let total = Duration::from_secs(3);
  modal.press_y(Instant::now(), total);
  assert!(modal.started_at.is_some());
  modal.dismiss();
  assert!(modal.started_at.is_none(), "dismiss must reset the timer");
}

#[test]
fn tick_returns_not_armed_when_idle() {
  let mut modal = ConfirmModal::new();
  let outcome = modal.tick(Instant::now(), Duration::from_secs(3));
  assert_eq!(outcome, CountdownTickOutcome::NotArmed);
}

#[test]
fn tick_returns_pending_before_total_elapses() {
  let mut modal = ConfirmModal::new();
  let total = Duration::from_secs(3);
  let t0 = Instant::now();
  modal.press_y(t0, total);
  let outcome = modal.tick(t0 + Duration::from_millis(1500), total);
  assert_eq!(outcome, CountdownTickOutcome::Pending);
}

#[test]
fn tick_returns_ready_to_fire_at_or_after_total() {
  let mut modal = ConfirmModal::new();
  let total = Duration::from_secs(3);
  let t0 = Instant::now();
  modal.press_y(t0, total);
  let outcome = modal.tick(t0 + Duration::from_secs(3), total);
  assert_eq!(outcome, CountdownTickOutcome::ReadyToFire);
  assert!(
    modal.started_at.is_none(),
    "tick that reached the deadline must clear the timer so a re-entrant tick returns NotArmed"
  );
}

#[test]
fn tick_with_zero_total_resets_and_returns_not_armed() {
  // Defensive: if the config flips to confirm_countdown_secs = 0 while
  // the modal is armed, the next tick must treat it as classic mode
  // rather than infinite-pending.
  let mut modal = ConfirmModal::new();
  let total = Duration::from_secs(3);
  let t0 = Instant::now();
  modal.press_y(t0, total);
  assert!(modal.started_at.is_some());

  let outcome = modal.tick(t0 + Duration::from_millis(100), Duration::ZERO);
  assert_eq!(outcome, CountdownTickOutcome::NotArmed);
  assert!(modal.started_at.is_none());
}

#[test]
fn progress_is_zero_when_idle() {
  let modal = ConfirmModal::new();
  assert_eq!(modal.progress(Instant::now(), Duration::from_secs(3)), 0.0);
}

#[test]
fn progress_caps_at_one_after_total() {
  let mut modal = ConfirmModal::new();
  let total = Duration::from_secs(3);
  let t0 = Instant::now();
  modal.press_y(t0, total);
  let p = modal.progress(t0 + Duration::from_secs(10), total);
  assert!(
    (p - 1.0).abs() < f64::EPSILON,
    "progress at +10s with total=3s must cap at 1.0, got {p}"
  );
}

#[test]
fn remaining_secs_rounds_up_to_next_whole_second() {
  // The UI label reads remaining_secs as the countdown number; rounding
  // up matches the visual contract "still seeing 2s on the label means
  // there's a fractional second left, not zero".
  let mut modal = ConfirmModal::new();
  let total = Duration::from_secs(3);
  let t0 = Instant::now();
  modal.press_y(t0, total);

  // At t0 + 500ms, remaining = 2.5s → label shows 3.
  let r = modal.remaining_secs(t0 + Duration::from_millis(500), total);
  assert_eq!(r, 3);

  // At t0 + 2.5s, remaining = 0.5s → label shows 1.
  let r = modal.remaining_secs(t0 + Duration::from_millis(2500), total);
  assert_eq!(r, 1);

  // At t0 + 3s, remaining = 0 → label shows 0.
  let r = modal.remaining_secs(t0 + Duration::from_secs(3), total);
  assert_eq!(r, 0);
}
