//! Display-impl contracts for `GwmError`. These pin the user-facing
//! prefix on each variant so cross-subcommand error messages stay
//! coherent — a user who typed `gwm tmux` should never see the word
//! "bootstrap" in the failure message.

use gwm::error::GwmError;

#[test]
fn command_failed_display_does_not_mention_bootstrap() {
  // regression: PR #65 Copilot review — the variant was originally
  // labelled `"bootstrap command failed: {0}"`, which printed
  // `"bootstrap command failed: tmux exited with status Some(1)"`
  // when `gwm tmux` failed to spawn. The shared variant must stay
  // operation-agnostic; callers prepend their own context into the
  // data string.
  let e = GwmError::CommandFailed("tmux exited with status Some(1)".into());
  let rendered = e.to_string();
  assert!(
    !rendered.contains("bootstrap"),
    "shared CommandFailed variant must not leak a `bootstrap` prefix into other subcommands' errors; got: {}",
    rendered
  );
  // The inner string must still surface so the caller's context (here:
  // `tmux exited …`) reaches the user.
  assert!(
    rendered.contains("tmux exited with status Some(1)"),
    "inner detail must round-trip through Display, got: {}",
    rendered
  );
}

#[test]
fn command_failed_display_keeps_command_word_for_grep_compatibility() {
  // The variant is still about command failures; keep the word
  // `command` (or `spawn`) in the prefix so users / scripts grep'ing
  // for failure patterns still match.
  let e = GwmError::CommandFailed("noop".into());
  let rendered = e.to_string().to_lowercase();
  assert!(
    rendered.contains("command") || rendered.contains("spawn") || rendered.contains("failed"),
    "Display impl should still convey command/spawn/failure semantics, got: {}",
    rendered
  );
}
