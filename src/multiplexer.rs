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
/// Four shapes differ from the tmux / zellij builders, three of them
/// measured against a live server rather than read off the help text:
///
/// * `pane split` needs `--current` to target the caller's pane. Without
///   it herdr has no pane to split from.
/// * `--direction` has no default in herdr's parser, so it must be
///   passed. `right` is the analogue of tmux's `-h` and of the direction
///   herdr's own agent guidance reaches for on a wide pane. Making it a
///   preference is filed separately; this is the hardcoded default until
///   that lands.
/// * **`--focus` is not the default.** `tab create` and `pane split` both
///   come back `"focused": false` when the flag is omitted, where
///   `tmux new-window` and `zellij action new-tab` move the user to what
///   they create. Omitting it would make herdr the one backend that opens
///   the worktree somewhere the user cannot see.
/// * **`tab create` needs `--workspace`.** Without it herdr uses the
///   server's *focused* workspace, not the caller's: run from a pane in
///   `w2K` with `w2P` focused, the tab landed in `w2P`. `workspace` is
///   `$HERDR_WORKSPACE_ID`, which every managed pane carries. An absent or
///   empty id drops the flag rather than passing one herdr would reject,
///   so a caller outside a managed pane degrades to herdr's own choice.
///   A split needs none of this: `--current` names the pane, which already
///   resolves the workspace (measured, the pane came back in `w2K`).
///
/// `pane split` takes no `--label` (a pane is renamed after the fact with
/// `herdr pane rename`), so the worktree name is deliberately dropped in
/// `Split` mode: passing it bare would be read as the optional `[PANE_ID]`
/// positional and split someone else's pane. `workspace` is likewise
/// ignored there.
pub fn build_herdr_command(name: &str, path: &Path, mode: SpawnMode, workspace: Option<&str>) -> Vec<String> {
  let path_str = path.display().to_string();
  match mode {
    SpawnMode::Window => {
      let mut argv = vec!["herdr".into(), "tab".into(), "create".into()];
      if let Some(ws) = workspace.filter(|ws| !ws.is_empty()) {
        argv.push("--workspace".into());
        argv.push(ws.into());
      }
      argv.extend([
        "--label".into(),
        name.into(),
        "--cwd".into(),
        path_str,
        "--focus".into(),
      ]);
      argv
    }
    SpawnMode::Split => vec![
      "herdr".into(),
      "pane".into(),
      "split".into(),
      "--current".into(),
      "--direction".into(),
      "right".into(),
      "--cwd".into(),
      path_str,
      "--focus".into(),
    ],
  }
}

/// Resolve which multiplexer the process is inside/// Resolve which multiplexer the process is inside and build its `Split`
/// argv, in the order tmux, zellij, herdr.
///
/// Both TUI call sites (`t`, and a `[tui.macro*]` with
/// `open_in = "mux_pane"`) need that same answer, and until #588 each wrote
/// its own if-chain: adding a third backend was two edits that could
/// disagree about the order. The `Multiplexer` comes back with the argv
/// because the macro path has to tell herdr apart, whose panes take no
/// command.
///
/// The three env values are parameters rather than reads, the shape
/// [`detect_tmux`] already uses, so the state tests can drive every branch
/// without rewriting a process-global variable. That is not theoretical
/// here: `$TMUX` is also read by the clipboard path, so a test that unset it
/// would pull every yank test in the same binary under the env lock.
pub fn detect_split_command(
  name: &str,
  path: &Path,
  tmux: Option<String>,
  zellij: Option<String>,
  herdr: Option<String>,
) -> Option<(Multiplexer, Vec<String>)> {
  if detect_tmux(tmux) {
    Some((Multiplexer::Tmux, build_tmux_command(name, path, SpawnMode::Split)))
  } else if detect_zellij(zellij) {
    Some((Multiplexer::Zellij, build_zellij_command(name, path, SpawnMode::Split)))
  } else if detect_herdr(herdr) {
    // A split needs no workspace id: `--current` resolves it.
    Some((
      Multiplexer::Herdr,
      build_herdr_command(name, path, SpawnMode::Split, None),
    ))
  } else {
    None
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
