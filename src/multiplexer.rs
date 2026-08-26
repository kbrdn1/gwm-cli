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
use serde::{Deserialize, Serialize};
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

/// Which half of the split a new pane takes (issues #589 / #611).
///
/// All four compass points, which two of the three backends honour
/// directly. Measured on tmux 3.7c by reading the *new* pane's geometry
/// back through `split-window -P -F`, rather than inferring it from pane
/// order: `-h` puts it at `left=101`, `-h -b` at `left=0`, `-v` at
/// `top=26`, `-v -b` at `top=1`. zellij takes the four words on
/// `new-pane --direction`.
///
/// **herdr 0.8.2 takes only two**: `herdr pane split --help` declares
/// `--direction [possible values: right, down]`, so [`Left`] and [`Up`]
/// are refused there the way `Workspace` is refused on tmux and zellij
/// (#608). Same mechanism, opposite backend.
///
/// [`Left`]: SplitDirection::Left
/// [`Up`]: SplitDirection::Up
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
  /// Side by side, to the right. tmux `-h`, `--direction right` on zellij
  /// and herdr.
  #[default]
  Right,
  /// Stacked, below. tmux `-v`, `--direction down` on zellij and herdr.
  Down,
  /// Side by side, to the left. tmux `-h -b`, `--direction left` on
  /// zellij. Refused by herdr.
  Left,
  /// Stacked, above. tmux `-v -b`, `--direction up` on zellij. Refused by
  /// herdr.
  Up,
}

impl SplitDirection {
  /// Every variant, default first, then the two herdr cannot honour.
  pub const ALL: [SplitDirection; 4] = [
    SplitDirection::Right,
    SplitDirection::Down,
    SplitDirection::Left,
    SplitDirection::Up,
  ];

  /// The serialised spelling — equal to the `[tui] mux_pane_direction`
  /// value, to the `--direction` flag's value, and to the argument zellij
  /// and herdr take. One string, four surfaces.
  pub const fn label(self) -> &'static str {
    match self {
      SplitDirection::Right => "right",
      SplitDirection::Down => "down",
      SplitDirection::Left => "left",
      SplitDirection::Up => "up",
    }
  }

  /// `true` for the two directions herdr's parser has no value for.
  pub const fn is_herdr_capable(self) -> bool {
    matches!(self, SplitDirection::Right | SplitDirection::Down)
  }

  /// tmux's own spelling, which needs two words for half the compass.
  ///
  /// `-h` is a *horizontal split* and puts the new pane to the RIGHT; `-v`
  /// stacks it BELOW. tmux names the axis the divider runs along, not the
  /// direction the pane goes, and `-b` ("before") flips the side on
  /// whichever axis was picked. The two vocabularies meet here and nowhere
  /// else.
  pub const fn tmux_flags(self) -> &'static [&'static str] {
    match self {
      SplitDirection::Right => &["-h"],
      SplitDirection::Down => &["-v"],
      SplitDirection::Left => &["-h", "-b"],
      SplitDirection::Up => &["-v", "-b"],
    }
  }
}

/// How to open the worktree inside the multiplexer.
///
/// * `Split(d)`  = split the current pane towards `d` (the `-p` flag — keeps
///   both views visible).
/// * `Window`    = a whole screen of its own: a tmux **window**, a zellij or
///   herdr **tab**. One thing under three names, which is why the variant
///   keeps tmux's word (it is also the verb, `new-window`) while
///   [`Multiplexer::window_noun`] renders the user's.
/// * `Workspace` = herdr's level above a tab (issue #608). tmux and zellij
///   have nothing at that level, so their builders refuse rather than open
///   something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
  Window,
  Split(SplitDirection),
  Workspace,
}

/// The two refusals a target can produce (issue #608). They are the text
/// the TUI's status bar shows and the CLI's error carries, so they name the
/// backend rather than the enum: a user who set `mux_open_in = "workspace"`
/// needs to know which of their three multiplexers cannot honour it.
const TMUX_HAS_NO_WORKSPACE: &str = "tmux has no workspace level: herdr is the only backend with one";
const ZELLIJ_HAS_NO_WORKSPACE: &str = "zellij has no workspace level: herdr is the only backend with one";
/// herdr 0.8.2 declares `--direction [possible values: right, down]`, so the
/// other half of the compass is a tmux and zellij capability (issue #611).
const HERDR_SPLITS_RIGHT_OR_DOWN: &str = "herdr splits only right or down: left and up are tmux and zellij directions";

/// Build `tmux new-window -n <name> -c <path>` (Window) or
/// `tmux split-window -h|-v -c <path>` (Split). `<name>` is the worktree's
/// short name so it shows up legibly in tmux's status bar; tmux panes
/// don't carry a name attribute, so Split intentionally omits `-n`.
///
/// gwm always passes a direction flag, which tmux does not require:
/// without one it falls back to `-v` and stacks the pane, and that is what
/// `--split` did up to 1.9 while its own `--help` promised "a horizontal
/// split of the current pane" (issue #589).
pub fn build_tmux_command(name: &str, path: &Path, mode: SpawnMode) -> Result<Vec<String>, &'static str> {
  let path_str = path.display().to_string();
  Ok(match mode {
    SpawnMode::Workspace => return Err(TMUX_HAS_NO_WORKSPACE),
    SpawnMode::Window => vec![
      "tmux".into(),
      "new-window".into(),
      "-n".into(),
      name.into(),
      "-c".into(),
      path_str,
    ],
    SpawnMode::Split(dir) => {
      let mut argv: Vec<String> = vec!["tmux".into(), "split-window".into()];
      argv.extend(dir.tmux_flags().iter().map(|f| (*f).to_string()));
      argv.extend(["-c".into(), path_str]);
      argv
    }
  })
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
pub fn build_zellij_command(name: &str, path: &Path, mode: SpawnMode) -> Result<Vec<String>, &'static str> {
  let path_str = path.display().to_string();
  Ok(match mode {
    SpawnMode::Workspace => return Err(ZELLIJ_HAS_NO_WORKSPACE),
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
  })
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
///   passed. zellij spells it the same way; tmux spells the same choice
///   `-h` / `-v`. Since #589 the value comes from [`SplitDirection`]
///   rather than from the `right` this builder used to hardcode.
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
pub fn build_herdr_command(
  name: &str,
  path: &Path,
  mode: SpawnMode,
  workspace: Option<&str>,
) -> Result<Vec<String>, &'static str> {
  let path_str = path.display().to_string();
  Ok(match mode {
    // `herdr workspace create` is `tab create` minus the `--workspace` it
    // would be creating, and it is the one target the other two backends
    // cannot match (issue #608). Measured on 0.8.2: `--cwd`, `--label`,
    // `--env`, `--focus` / `--no-focus`. `--focus` is passed for the same
    // reason as on a tab: without it the workspace comes back unfocused.
    SpawnMode::Workspace => vec![
      "herdr".into(),
      "workspace".into(),
      "create".into(),
      "--label".into(),
      name.into(),
      "--cwd".into(),
      path_str,
      "--focus".into(),
    ],
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
    SpawnMode::Split(dir) if !dir.is_herdr_capable() => return Err(HERDR_SPLITS_RIGHT_OR_DOWN),
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
  })
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

/// Dispatch to the right `build_*_command` for `mux`. The CLI verb wrote
/// this match out and the two TUI call sites reached a second copy of it
/// inside `detect_split_command`; both now come here. The CLI's copy is
/// the one that knew about `$HERDR_WORKSPACE_ID`.
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
) -> Result<Vec<String>, &'static str> {
  match mux {
    Multiplexer::Tmux => build_tmux_command(name, path, mode),
    Multiplexer::Zellij => build_zellij_command(name, path, mode),
    Multiplexer::Herdr => build_herdr_command(name, path, mode, workspace),
  }
}

/// What a `[tui.macro*]` with `open_in = "mux_pane"` should spawn, or the
/// reason it has to fall back to the PTY overlay.
///
/// The three steps a macro runs on (detect, refuse, build) were an inline
/// chain in `run_macro`, which is the one place they could not be tested:
/// the function takes a `Terminal` and drives the event loop. Extracting
/// them makes the decision a value, and the fallback is the branch worth
/// pinning, since getting it wrong drops the user's command into a pane
/// nobody looks at (Codex review on PR #609).
///
/// Two refusals in one, asked in the order that produces the more useful
/// sentence: [`macro_refusal`] answers "can this backend run a command at
/// all", [`build_command`] answers "can it open this target". A macro's own
/// problem is what the status bar should name when both apply.
///
/// The env values are parameters for the same reason as in
/// [`detect_multiplexer`]: `$TMUX` is read by the clipboard path too, so a
/// test that unset it would pull every yank test in the same binary under
/// the env lock.
pub fn macro_mux_command(
  label: &str,
  path: &Path,
  mode: SpawnMode,
  tmux: Option<String>,
  zellij: Option<String>,
  herdr: Option<String>,
  workspace: Option<&str>,
) -> Result<(Multiplexer, Vec<String>), &'static str> {
  let Some(mux) = detect_multiplexer(tmux, zellij, herdr) else {
    return Err(NO_MULTIPLEXER);
  };
  if let Some(why) = macro_refusal(mux, mode) {
    return Err(why);
  }
  build_command(mux, label, path, mode, workspace).map(|argv| (mux, argv))
}

/// The one refusal that is not a backend's fault. Kept next to the others so
/// the status bar's four sentences read as one set.
const NO_MULTIPLEXER: &str = "no multiplexer";

/// What `mode` just opened, for a status line that names the thing the
/// user is looking at rather than the thing the key is called: `t` can now
/// open a whole window or tab (#589), and "opened <name> in new pane" was
/// the only sentence the TUI had for it.
pub fn spawn_noun(mux: Multiplexer, mode: SpawnMode) -> &'static str {
  match mode {
    SpawnMode::Split(_) => "pane",
    SpawnMode::Window => mux.window_noun(),
    SpawnMode::Workspace => "workspace",
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
/// * neither herdr verb takes one, `workspace create` included (#608).
///   Running a command in a herdr pane is
///   `herdr pane run <pane-id> <cmd>`, and the id only comes back in the
///   JSON `pane split` prints, so it is two processes and a parse, not an
///   argv (#599).
///
/// This answers "can this backend RUN a command", not "can it open this
/// target at all" — the builders answer the second, with an `Err`. A macro
/// asks both, and the caller checks this one first so its status names the
/// macro's problem rather than the target's.
///
/// Splitting anyway would open an empty pane and silently drop the macro,
/// so the caller falls back to the PTY overlay and puts the reason in the
/// status bar.
pub const fn macro_refusal(mux: Multiplexer, mode: SpawnMode) -> Option<&'static str> {
  match (mux, mode) {
    (Multiplexer::Herdr, SpawnMode::Workspace) => Some("herdr workspaces take no command"),
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

/// Attach a shell command line to an argv from [`build_command`], or `None`
/// when the backend has no trailing-command form in that mode.
///
/// [`macro_refusal`] answers "can this (backend, mode) run a command at
/// all"; this answers "how". They were one inline chain in `run_macro`
/// until the first half moved out in #589, and the second half stayed
/// behind because it had a single caller. `o` on the agents overlay (#591)
/// is the second, and two copies that can disagree is the defect the
/// extractions in #588 and #589 were both aimed at.
///
/// The two shapes are not interchangeable:
///
/// * **zellij** runs its trailing argv *directly*, not through a shell, so
///   a command with spaces or shell syntax has to arrive as
///   `-- <shell> <flag> <line>` or zellij looks for a binary named after
///   the whole line.
/// * **tmux** takes the command as a *single* shell-command operand and
///   hands it to the shell itself, so it is appended whole rather than
///   pre-split, which would lose everything after the first word.
///
/// **herdr gets `None`**, which a caller that asked [`macro_refusal`] first
/// can never see. It is still spelled out rather than folded into tmux's
/// arm: appending an operand `herdr pane split` ignores would open an empty
/// pane and drop the command silently, and that failure is worth being
/// unrepresentable rather than merely unreachable.
///
/// `shell` / `shell_flag` are parameters rather than env reads, the shape
/// [`detect_multiplexer`] already uses, so the argv can be pinned by a test
/// on any runner.
pub fn attach_pane_command(
  mux: Multiplexer,
  argv: &[String],
  command: &str,
  shell: &str,
  shell_flag: &str,
) -> Option<Vec<String>> {
  let mut argv = argv.to_vec();
  match mux {
    Multiplexer::Herdr => return None,
    Multiplexer::Tmux => argv.push(command.into()),
    Multiplexer::Zellij => argv.extend(["--".into(), shell.into(), shell_flag.into(), command.into()]),
  }
  Some(argv)
}
