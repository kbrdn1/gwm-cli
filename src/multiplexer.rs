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

use clap::ValueEnum;
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

  /// What [`SpawnMode::Window`] actually opens here, for a status line that
  /// names the thing the user is looking at: tmux has windows, zellij and
  /// herdr have tabs.
  pub fn window_noun(self) -> &'static str {
    match self {
      Multiplexer::Tmux => "window",
      Multiplexer::Zellij | Multiplexer::Herdr => "tab",
    }
  }
}

/// Which half of the split a new pane takes (issue #589).
///
/// Two variants rather than four: `right` and `down` are the intersection
/// of what the three backends accept. tmux reaches `left` / `up` only
/// through `split-window -b`, and herdr 0.8.2 declares its `--direction`
/// as `[possible values: right, down]` outright, so a fuller compass
/// would be variants one backend could not honour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SplitDirection {
  /// Side by side. tmux `-h`, `--direction right` on zellij and herdr.
  Right,
  /// Stacked. tmux `-v`, `--direction down` on zellij and herdr.
  Down,
}

impl SplitDirection {
  /// Every variant, default first.
  pub const ALL: [SplitDirection; 2] = [SplitDirection::Right, SplitDirection::Down];

  /// The serialised spelling — equal to the `[tui] mux_pane_direction`
  /// value, to the `--direction` flag's value, and to the argument zellij
  /// and herdr take. One string, four surfaces.
  pub const fn label(self) -> &'static str {
    match self {
      SplitDirection::Right => "right",
      SplitDirection::Down => "down",
    }
  }

  /// tmux's own spelling. `-h` is a *horizontal split*, which puts the new
  /// pane to the RIGHT, and `-v` stacks it BELOW: tmux names the axis the
  /// divider runs along, not the direction the pane goes. The two
  /// vocabularies meet here and nowhere else.
  pub const fn tmux_flag(self) -> &'static str {
    match self {
      SplitDirection::Right => "-h",
      SplitDirection::Down => "-v",
    }
  }
}

/// How to open the worktree inside the multiplexer.
/// `Window`   = new tmux window / zellij tab / herdr tab (full screen real estate).
/// `Split(d)` = split the current pane towards `d` (the `-p` flag — keeps both views visible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
  Window,
  Split(SplitDirection),
}

/// Build `tmux new-window -n <name> -c <path>` (Window) or
/// `tmux split-window -h|-v -c <path>` (Split). `<name>` is the worktree's
/// short name so it shows up legibly in tmux's status bar; tmux panes
/// don't carry a name attribute, so Split intentionally omits `-n`.
///
/// The direction flag is not optional the way it reads: without it tmux
/// falls back to `-v` and stacks, which is what `--split` did up to 1.9
/// while its own `--help` promised "a horizontal split" (issue #589).
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
    SpawnMode::Split(dir) => vec![
      "tmux".into(),
      "split-window".into(),
      dir.tmux_flag().into(),
      "-c".into(),
      path_str,
    ],
  }
}

/// Build `zellij action new-tab --name <name> --cwd <path>` (Window) or
/// `zellij action new-pane --direction right|down --cwd <path>` (Split).
/// `--cwd` on `new-tab` requires zellij ≥ 0.40 — older versions surface
/// their own error, which is preferable to silently ignoring the cwd.
///
/// `--direction` on `new-pane` is optional in zellij's parser ("if no
/// direction is specified, it will try to use the biggest available
/// space"), so passing it is what makes the choice gwm's rather than the
/// layout's. It conflicts with `--floating` / `--in-place`, neither of
/// which gwm passes.
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
    SpawnMode::Split(dir) => vec![
      "zellij".into(),
      "action".into(),
      "new-pane".into(),
      "--direction".into(),
      dir.label().into(),
      "--cwd".into(),
      path_str,
    ],
  }
}

/// Build `herdr tab create --label <name> --cwd <path>` (Window) or
/// `herdr pane split --current --direction right|down --cwd <path>` (Split).
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
///   passed. It is the one flag the three backends share by name, and
///   since #589 the value comes from [`SplitDirection`] rather than from
///   the `right` this builder used to hardcode.
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
    SpawnMode::Split(dir) => vec![
      "herdr".into(),
      "pane".into(),
      "split".into(),
      "--current".into(),
      "--direction".into(),
      dir.label().into(),
      "--cwd".into(),
      path_str,
      "--focus".into(),
    ],
  }
}

/// Resolve which multiplexer the process is inside, in the order tmux,
/// zellij, herdr.
///
/// Both TUI call sites (`t`, and a `[tui.macro*]` with
/// `open_in = "mux_pane"`) need that same answer, and until #588 each wrote
/// its own if-chain: adding a third backend was two edits that could
/// disagree about the order. This is the one answer, and [`build_command`]
/// turns it into an argv.
///
/// The three env values are parameters rather than reads, the shape
/// [`detect_tmux`] already uses, so the state tests can drive every branch
/// without rewriting a process-global variable. That is not theoretical
/// here: `$TMUX` is also read by the clipboard path, so a test that unset it
/// would pull every yank test in the same binary under the env lock.
pub fn detect_multiplexer(tmux: Option<String>, zellij: Option<String>, herdr: Option<String>) -> Option<Multiplexer> {
  if detect_tmux(tmux) {
    Some(Multiplexer::Tmux)
  } else if detect_zellij(zellij) {
    Some(Multiplexer::Zellij)
  } else if detect_herdr(herdr) {
    Some(Multiplexer::Herdr)
  } else {
    None
  }
}

/// Dispatch to the right `build_*_command` for `mux`. The three call sites
/// (the CLI verb, the TUI's `t`, a `mux_pane` macro) all had this match
/// written out, and the CLI's copy is the one that knows about
/// `$HERDR_WORKSPACE_ID`.
///
/// `workspace` is forwarded unconditionally: [`build_herdr_command`] drops
/// it on a `Split`, where `--current` already resolves the workspace, and
/// the other two backends never took it. Passing it here therefore cannot
/// put a `--workspace` on a `pane split`, which is the regression this
/// signature invites and `tests/multiplexer_tests.rs` pins.
pub fn build_command(
  mux: Multiplexer,
  name: &str,
  path: &Path,
  mode: SpawnMode,
  workspace: Option<&str>,
) -> Vec<String> {
  match mux {
    Multiplexer::Tmux => build_tmux_command(name, path, mode),
    Multiplexer::Zellij => build_zellij_command(name, path, mode),
    Multiplexer::Herdr => build_herdr_command(name, path, mode, workspace),
  }
}

/// What `mode` just opened, for a status line that names the thing the
/// user is looking at rather than the thing the key is called: `t` can now
/// open a whole window or tab (#589), and "opened <name> in new pane" was
/// the only sentence the TUI had for it.
pub fn spawn_noun(mux: Multiplexer, mode: SpawnMode) -> &'static str {
  match mode {
    SpawnMode::Split(_) => "pane",
    SpawnMode::Window => mux.window_noun(),
  }
}

/// Why `mux` cannot carry a `[tui.macro*]` command in `mode`, or `None`
/// when it can.
///
/// A macro needs the new pane/tab to *run* something, and only some of the
/// six (backend, mode) pairs have a trailing-command form:
///
/// * `tmux split-window <cmd>` and `tmux new-window <cmd>` both take one.
/// * `zellij action new-pane -- <cmd>` takes one; `zellij action new-tab`
///   does not, so `mux_pane_direction = "window"` has nothing to hand a
///   zellij macro (#589).
/// * neither herdr verb takes one. Running a command in a herdr pane is
///   `herdr pane run <pane-id> <cmd>`, and the id only comes back in the
///   JSON `pane split` prints, so it is two processes and a parse, not an
///   argv (#599).
///
/// Splitting anyway would open an empty pane and silently drop the macro,
/// so the caller falls back to the PTY overlay and puts the reason in the
/// status bar.
pub const fn macro_refusal(mux: Multiplexer, mode: SpawnMode) -> Option<&'static str> {
  match (mux, mode) {
    (Multiplexer::Herdr, _) => Some("herdr panes take no command"),
    (Multiplexer::Zellij, SpawnMode::Window) => Some("zellij tabs take no command"),
    _ => None,
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
///
/// # The one case this probe gets wrong
///
/// A tmux server started from inside a herdr pane promotes the whole
/// `HERDR_*` set to its server-global environment, so every later session
/// on that server carries `HERDR_ENV=1` and a pane id it does not own
/// (herdrdev/herdr#2134, filed against 0.7.5 and closed unfixed for
/// template non-compliance, so still live in 0.8.2).
///
/// [`detect_split_command`] is immune by construction: that leak only
/// happens inside a tmux session, where `$TMUX` is set and tmux wins the
/// cascade first. `gwm herdr <pattern>` is not, and cannot be made so
/// here. The upstream issue reports every marker variable leaking
/// together, pane id and socket path included, so no local check can tell
/// a real pane from a leaked one: only asking the running server whether
/// it owns `$HERDR_PANE_ID` separates them, which is the socket round trip
/// #599 tracks. Until then the failure is loud rather than silent, since
/// herdr answers a stale pane id with a non-zero exit.
pub fn detect_herdr(env: Option<String>) -> bool {
  match env {
    Some(s) => !s.is_empty(),
    None => false,
  }
}
