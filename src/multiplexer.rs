//! Terminal-multiplexer integration. Builds the argv vectors for
//! `tmux new-window/split-window` and `zellij action new-tab/new-pane` so
//! `gwm tmux <pattern>` / `gwm zellij <pattern>` can open a worktree in
//! one keystroke from inside an already-running multiplexer session.
//!
//! The command builders are pure functions returning `Vec<String>` so the
//! integration tests can pin the exact incantation without spawning tmux
//! or zellij on every test runner. The actual `std::process::Command`
//! spawn lives in `cli.rs`, matching the lazygit-launch pattern in
//! `tui/mod.rs::run_lazygit`.

use std::path::Path;

/// Multiplexer the user opted into via `gwm tmux …` / `gwm zellij …`.
/// Carried through the CLI dispatch so the not-running error and the
/// argv builder share one source of truth for the binary name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexer {
  Tmux,
  Zellij,
}

impl Multiplexer {
  /// Binary name as it appears on `$PATH`. Used both for the spawn and
  /// for the `<bin> session not running` error string.
  pub fn binary(self) -> &'static str {
    match self {
      Multiplexer::Tmux => "tmux",
      Multiplexer::Zellij => "zellij",
    }
  }
}

/// How to open the worktree inside the multiplexer.
/// `Window` = new tmux window / zellij tab (the default — full screen real estate).
/// `Split`  = split the current pane (the `-p` flag — keeps both views visible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
  Window,
  Split,
}

/// Build `tmux new-window -n <name> -c <path>` (Window) or
/// `tmux split-window -c <path>` (Split). `<name>` is the worktree's
/// short name so it shows up legibly in tmux's status bar; tmux panes
/// don't carry a name attribute, so Split intentionally omits `-n`.
pub fn build_tmux_command(name: &str, path: &Path, mode: SpawnMode) -> Vec<String> {
  let path_str = path.display().to_string();
  match mode {
    SpawnMode::Window => vec![
      "tmux".into(),
      "new-window".into(),
      "-n".into(),
      name.into(),
      "-c".into(),
      path_str,
    ],
    SpawnMode::Split => vec!["tmux".into(), "split-window".into(), "-c".into(), path_str],
  }
}

/// Build `zellij action new-tab --name <name> --cwd <path>` (Window) or
/// `zellij action new-pane --cwd <path>` (Split). `--cwd` on `new-tab`
/// requires zellij ≥ 0.40 — older versions surface their own error,
/// which is preferable to silently ignoring the cwd.
pub fn build_zellij_command(name: &str, path: &Path, mode: SpawnMode) -> Vec<String> {
  let path_str = path.display().to_string();
  match mode {
    SpawnMode::Window => vec![
      "zellij".into(),
      "action".into(),
      "new-tab".into(),
      "--name".into(),
      name.into(),
      "--cwd".into(),
      path_str,
    ],
    SpawnMode::Split => vec![
      "zellij".into(),
      "action".into(),
      "new-pane".into(),
      "--cwd".into(),
      path_str,
    ],
  }
}

/// `true` when `$TMUX` is set to a non-empty value — tmux exports the
/// socket path to every process spawned inside a session, so its
/// presence is the canonical "am I inside tmux?" probe.
///
/// Takes the env value as a parameter (rather than reading it directly)
/// so the unit tests can exercise both branches without mutating the
/// process environment. The CLI dispatcher calls
/// `detect_tmux(std::env::var("TMUX").ok())`.
pub fn detect_tmux(env: Option<String>) -> bool {
  match env {
    Some(s) => !s.is_empty(),
    None => false,
  }
}

/// `true` when `$ZELLIJ` is set to a non-empty value. Zellij exports the
/// variable to every command spawned inside a session, similar to tmux.
pub fn detect_zellij(env: Option<String>) -> bool {
  match env {
    Some(s) => !s.is_empty(),
    None => false,
  }
}
