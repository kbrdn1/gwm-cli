//! Unit tests for the pure `LinkPrompt` sub-struct (issue #126).
//!
//! Exercises the two-stage link-prompt state in isolation — the prompt
//! owns `stage` / `number` / `target`, exposes target rotation, target
//! commit, and digit push/pop. The `App` orchestrator owns the
//! side-effecting `link_prompt_submit` (which shells out to
//! `github::link_issue` / `github::link_pr` against `self.repo`) and the
//! status-bar copy.
//!
//! `LinkTarget` is the cli-side enum re-exported from `tui::app` — this
//! test imports it from `gwm::tui` to match the existing public surface.

use gwm::tui::state::link_prompt::{LinkPrompt, LinkPromptStage};
use gwm::tui::LinkTarget;

#[test]
fn new_starts_in_choose_target_with_empty_buffer_and_no_target() {
  let prompt = LinkPrompt::new();
  assert_eq!(prompt.stage, LinkPromptStage::ChooseTarget);
  assert!(prompt.number.is_empty());
  assert_eq!(prompt.target, None);
}

#[test]
fn toggle_target_rotates_issue_to_pr_and_back() {
  // The two-stage prompt's ChooseTarget step lets the user flip between
  // Issue and Pr without committing. Starts with Issue as the default
  // hint so a single Enter on a fresh prompt lands on the most common
  // case.
  let mut prompt = LinkPrompt::new();
  assert_eq!(prompt.target, None);
  prompt.toggle_target();
  assert_eq!(prompt.target, Some(LinkTarget::Issue));
  prompt.toggle_target();
  assert_eq!(prompt.target, Some(LinkTarget::Pr));
  prompt.toggle_target();
  assert_eq!(prompt.target, Some(LinkTarget::Issue), "wraps back to Issue");
}

#[test]
fn commit_target_advances_to_input_number_and_stores_target() {
  // The keypress handler calls commit_target(Issue|Pr) once the user
  // picks; that's the transition from stage 1 (rotate / pick) to stage
  // 2 (type digits, then Enter to submit).
  let mut prompt = LinkPrompt::new();
  prompt.commit_target(LinkTarget::Pr);
  assert_eq!(prompt.stage, LinkPromptStage::InputNumber);
  assert_eq!(prompt.target, Some(LinkTarget::Pr));
  assert!(
    prompt.number.is_empty(),
    "buffer is cleared on commit so the user starts fresh"
  );
}

#[test]
fn push_char_only_accepts_digits_during_input_number() {
  let mut prompt = LinkPrompt::new();
  prompt.commit_target(LinkTarget::Issue);
  for c in "12a3".chars() {
    prompt.push_char(c);
  }
  assert_eq!(
    prompt.number, "123",
    "non-digit chars must be dropped during InputNumber stage"
  );
}

#[test]
fn push_char_is_noop_during_choose_target_stage() {
  // During ChooseTarget, digits aren't part of the contract — the
  // keypress handler is matching on `i`/`p`, not digits. Defence in
  // depth: even if a digit slips through somehow, it must not pollute
  // the buffer.
  let mut prompt = LinkPrompt::new();
  prompt.push_char('1');
  prompt.push_char('2');
  assert!(prompt.number.is_empty());
}

#[test]
fn pop_char_removes_last_digit_during_input_number() {
  let mut prompt = LinkPrompt::new();
  prompt.commit_target(LinkTarget::Issue);
  for c in "42".chars() {
    prompt.push_char(c);
  }
  prompt.pop_char();
  assert_eq!(prompt.number, "4");
  prompt.pop_char();
  assert!(prompt.number.is_empty());
  // No-op on empty buffer.
  prompt.pop_char();
  assert!(prompt.number.is_empty());
}

#[test]
fn pop_char_is_noop_during_choose_target_stage() {
  // Symmetric to push_char: backspace during ChooseTarget must not
  // surreptitiously empty a buffer that the next commit_target call
  // will start fresh anyway.
  let mut prompt = LinkPrompt::new();
  prompt.number.push_str("42");
  prompt.pop_char();
  assert_eq!(
    prompt.number, "42",
    "pop_char during ChooseTarget must not touch the buffer"
  );
}

#[test]
fn reset_returns_prompt_to_initial_state() {
  let mut prompt = LinkPrompt::new();
  prompt.commit_target(LinkTarget::Issue);
  prompt.push_char('4');
  prompt.push_char('2');
  prompt.reset();
  assert_eq!(prompt.stage, LinkPromptStage::ChooseTarget);
  assert!(prompt.number.is_empty());
  assert_eq!(prompt.target, None);
}
