//! Terminal-multiplexer integration. Builds the argv vectors for
//! `tmux new-window/split-window`, `zellij action new-tab/new-pane` and
//! `herdr tab create / pane split` so `gwm tmux <pattern>` /
//! `gwm zellij <pattern>` / `gwm herdr <pattern>` can open a worktree in
//! one keystroke from inside an already-running multiplexer session.
//!
//! The command builders are pure functions returning `Vec<String>` so the
//! integration tests can pin the exact incantation without spawning tmux,
//! zellij or herdr on every test runner. The actual `std::process::Command`
//! spawn lives in `cli.rs`, matching the lazygit-launch pattern in
//! `tui/mod.rs::run_lazygit`.

use std::path::Path;

/// Multiplexer the user opted into via `gwm tmux …` / `gwm zellij …` /
/// `gwm herdr …`.
/// Carried through the CLI dispatch so the not-running error and the
/// argv builder share one source of truth for the binary name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexer {
  Tmux,
  Zellij,
  Herdr,
}

impl Multiplexer {
  /// Binary name as it appears on `$PATH`. Used both for the spawn and
  /// for the `<bin> session not running` error string.
  pub fn binary(self) -> &'static str {
    match self {
      Multiplexer::Tmux => "tmux",
      Multiplexer::Zellij => "zellij",
      Multiplexer::Herdr => "herdr",
    }
  }
}

/// How to open the worktree inside the multiplexer.
/// `Window` = new tmux window / zellij tab / herdr tab (the default — full screen real estate).
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

/// Build `herdr tab create --label <name> --cwd <path>` (Window) or
/// `herdr pane split --current --direction right --cwd <path>` (Split).
/// Herdr drives its own server over a socket, so both verbs are control
/// commands rather than a `new-window` equivalent; measured against
/// herdr 0.8.2 (`herdr tab create --help`, `herdr pane split --help`).
///
/// Two shapes differ from the tmux / zellij builders:
///
/// * `pane split` needs `--current` to target the caller's pane. Without
///   it herdr has no pane to split from.
/// * `--direction` has no default in herdr's parser, so it must be
///   passed. `right` is the analogue of tmux's `-h` and of the direction
///   herdr's own agent guidance reaches for on a wide pane. Making it a
///   preference is filed separately; this is the hardcoded default until
///   that lands.
///
/// `pane split` takes no `--label` (a pane is renamed after the fact with
/// `herdr pane rename`), so the worktree name is deliberately dropped in
/// `Split` mode: passing it bare would be read as the optional `[PANE_ID]`
/// positional and split someone else's pane.
///
/// Neither verb pins `--focus` / `--no-focus`: herdr focuses what it
/// creates by default, which is what `tmux new-window` and
/// `zellij action new-tab` do, and gwm has no reason to override it.
pub fn build_herdr_command(name: &str, path: &Path, mode: SpawnMode) -> Vec<String> {
  let path_str = path.display().to_string();
  match mode {
    SpawnMode::Window => vec![
      "herdr".into(),
      "tab".into(),
      "create".into(),
      "--label".into(),
      name.into(),
      "--cwd".into(),
      path_str,
    ],
    SpawnMode::Split => vec![
      "herdr".into(),
      "pane".into(),
      "split".into(),
      "--current".into(),
      "--direction".into(),
      "right".into(),
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

/// `true` when `$HERDR_ENV` is set to a non-empty value. Herdr exports it
/// as `1` to every process it starts in a managed pane, alongside
/// `HERDR_PANE_ID` / `HERDR_TAB_ID` / `HERDR_SOCKET_PATH`. The value is
/// not parsed, only its presence: `$HERDR_ENV` is the direct analogue of
/// `$TMUX`, and a herdr that one day exports something richer than `1`
/// keeps working.
pub fn detect_herdr(env: Option<String>) -> bool {
  match env {
    Some(s) => !s.is_empty(),
    None => false,
  }
}
