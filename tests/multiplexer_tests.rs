//! Pure-logic tests for `gwm::multiplexer`. The module's command builders
//! and env-var probes are deliberately decoupled from any process spawn, so
//! these tests do not require `tmux`, `zellij` or `herdr` to be installed on
//! the runner — they assert against the produced argv vectors.

use gwm::multiplexer::{
  build_command, build_herdr_command, build_tmux_command, build_zellij_command, detect_herdr, detect_multiplexer,
  detect_tmux, detect_zellij, macro_refusal, Multiplexer, SpawnMode, SplitDirection,
};
use std::path::Path;

// --------------------------------------------------------------------------
// tmux command builder
// --------------------------------------------------------------------------

#[test]
fn tmux_new_window_uses_new_window_subverb() {
  // `tmux new-window -n <name> -c <path>` is the canonical incantation for
  // "open this directory in a new window of the current session". The `-n`
  // labels the window (so it's discoverable in tmux's status bar) and
  // `-c` sets the new window's cwd, which is what the user expects when
  // running `gwm tmux <pattern>`.
  let argv = build_tmux_command("feat-99-auth", Path::new("/tmp/wt/feat-99-auth"), SpawnMode::Window);
  assert_eq!(argv[0], "tmux");
  assert_eq!(argv[1], "new-window");
  // `-n <name>` and `-c <path>` are both required.
  let has_n = argv.windows(2).any(|w| w[0] == "-n" && w[1] == "feat-99-auth");
  let has_c = argv.windows(2).any(|w| w[0] == "-c" && w[1] == "/tmp/wt/feat-99-auth");
  assert!(has_n, "expected `-n feat-99-auth` in argv, got: {:?}", argv);
  assert!(has_c, "expected `-c /tmp/wt/feat-99-auth` in argv, got: {:?}", argv);
}

#[test]
fn tmux_split_pane_uses_split_window_subverb() {
  // With `-p` (SpawnMode::Split) we want a split of the current window, not
  // a new window. `tmux split-window <-h|-v> -c <path>` is the shape — no
  // `-n` because tmux panes don't carry a name attribute.
  let argv = build_tmux_command(
    "feat-12-x",
    Path::new("/tmp/wt/feat-12-x"),
    SpawnMode::Split(SplitDirection::Right),
  );
  assert_eq!(argv[0], "tmux");
  assert_eq!(argv[1], "split-window");
  let has_c = argv.windows(2).any(|w| w[0] == "-c" && w[1] == "/tmp/wt/feat-12-x");
  assert!(
    has_c,
    "split-window must also set `-c` so the new pane lands in the worktree, got: {:?}",
    argv
  );
  // No `-n`: tmux split-window doesn't accept `-n`. A buggy build that
  // forwarded the window name here would error out at spawn time on
  // every invocation.
  assert!(
    !argv.iter().any(|a| a == "-n"),
    "split-window must NOT carry `-n` (tmux rejects it), got: {:?}",
    argv
  );
}

// --------------------------------------------------------------------------
// zellij command builder
// --------------------------------------------------------------------------

#[test]
fn zellij_new_tab_uses_action_new_tab() {
  // Zellij is driven by `zellij action <verb>`. The new-tab verb supports
  // both `--name` and `--cwd` since 0.40 — the latter is what makes the
  // tab open inside the worktree.
  let argv = build_zellij_command("feat-7-foo", Path::new("/tmp/wt/feat-7-foo"), SpawnMode::Window);
  assert_eq!(argv[0], "zellij");
  assert_eq!(argv[1], "action");
  assert_eq!(argv[2], "new-tab");
  let has_name = argv.windows(2).any(|w| w[0] == "--name" && w[1] == "feat-7-foo");
  let has_cwd = argv.windows(2).any(|w| w[0] == "--cwd" && w[1] == "/tmp/wt/feat-7-foo");
  assert!(has_name, "expected `--name feat-7-foo` in argv, got: {:?}", argv);
  assert!(has_cwd, "expected `--cwd /tmp/wt/feat-7-foo` in argv, got: {:?}", argv);
}

#[test]
fn zellij_split_pane_uses_action_new_pane() {
  // `-p` → split the current tab. `zellij action new-pane --cwd <path>` is
  // the shape; no `--name` because zellij panes aren't named at creation.
  let argv = build_zellij_command(
    "feat-7-foo",
    Path::new("/tmp/wt/feat-7-foo"),
    SpawnMode::Split(SplitDirection::Right),
  );
  assert_eq!(argv[0], "zellij");
  assert_eq!(argv[1], "action");
  assert_eq!(argv[2], "new-pane");
  let has_cwd = argv.windows(2).any(|w| w[0] == "--cwd" && w[1] == "/tmp/wt/feat-7-foo");
  assert!(has_cwd, "new-pane must set `--cwd`, got: {:?}", argv);
  assert!(
    !argv.iter().any(|a| a == "--name"),
    "new-pane must NOT carry `--name` (zellij rejects it on panes), got: {:?}",
    argv
  );
}

// --------------------------------------------------------------------------
// herdr command builder (#588)
// --------------------------------------------------------------------------

#[test]
fn herdr_new_tab_uses_tab_create() {
  // Herdr drives its server over a socket API: `herdr tab create` is the
  // new-tab verb, with `--cwd` for the working directory and `--label` for
  // the name shown on the tab. Measured against herdr 0.8.2
  // (`herdr tab create --help`).
  let argv = build_herdr_command(
    "feat-7-foo",
    Path::new("/tmp/wt/feat-7-foo"),
    SpawnMode::Window,
    Some("w2K"),
  );
  assert_eq!(argv[0], "herdr");
  assert_eq!(argv[1], "tab");
  assert_eq!(argv[2], "create");
  let has_label = argv.windows(2).any(|w| w[0] == "--label" && w[1] == "feat-7-foo");
  let has_cwd = argv.windows(2).any(|w| w[0] == "--cwd" && w[1] == "/tmp/wt/feat-7-foo");
  assert!(has_label, "expected `--label feat-7-foo` in argv, got: {:?}", argv);
  assert!(has_cwd, "expected `--cwd /tmp/wt/feat-7-foo` in argv, got: {:?}", argv);
}

#[test]
fn herdr_new_tab_asks_for_the_focus_tmux_and_zellij_give_for_free() {
  // Measured on herdr 0.8.2, and the opposite of what this test asserted
  // when the builder was first written: `herdr tab create --cwd <path>` with
  // no focus flag comes back `"focused": false`. `tmux new-window` and
  // `zellij action new-tab` both move the user to what they create, so
  // omitting the flag would make `gwm herdr` the one backend that opens the
  // worktree somewhere the user cannot see.
  let argv = build_herdr_command(
    "feat-7-foo",
    Path::new("/tmp/wt/feat-7-foo"),
    SpawnMode::Window,
    Some("w2K"),
  );
  assert!(
    argv.iter().any(|a| a == "--focus"),
    "tab create must focus, got: {:?}",
    argv
  );
  let argv = build_herdr_command(
    "feat-7-foo",
    Path::new("/tmp/wt/feat-7-foo"),
    SpawnMode::Split(SplitDirection::Right),
    None,
  );
  assert!(
    argv.iter().any(|a| a == "--focus"),
    "pane split must focus, got: {:?}",
    argv
  );
}

#[test]
fn herdr_new_tab_targets_the_callers_workspace() {
  // Measured on herdr 0.8.2 with two workspaces open: `herdr tab create`
  // without `--workspace` lands in the server's *focused* workspace, not the
  // caller's. Running it from a pane in `w2K` put the tab in `w2P`, i.e.
  // `gwm herdr <pattern>` would open the worktree in a different project's
  // window. `$HERDR_WORKSPACE_ID` is what the calling pane carries, so it is
  // what pins the target.
  let argv = build_herdr_command(
    "feat-7-foo",
    Path::new("/tmp/wt/feat-7-foo"),
    SpawnMode::Window,
    Some("w2K"),
  );
  let has_ws = argv.windows(2).any(|w| w[0] == "--workspace" && w[1] == "w2K");
  assert!(has_ws, "expected `--workspace w2K` in argv, got: {:?}", argv);
}

#[test]
fn herdr_new_tab_omits_the_workspace_flag_when_the_id_is_unknown() {
  // Outside a managed pane there is no `$HERDR_WORKSPACE_ID` to pass, and
  // `--workspace ""` is an argument herdr would have to reject. Falling back
  // to herdr's own choice of workspace beats failing the whole command.
  let argv = build_herdr_command("feat-7-foo", Path::new("/tmp/wt/feat-7-foo"), SpawnMode::Window, None);
  assert!(
    !argv.iter().any(|a| a == "--workspace"),
    "no workspace id means no flag, got: {:?}",
    argv
  );
  let argv = build_herdr_command(
    "feat-7-foo",
    Path::new("/tmp/wt/feat-7-foo"),
    SpawnMode::Window,
    Some(""),
  );
  assert!(
    !argv.iter().any(|a| a == "--workspace"),
    "an empty workspace id is not an id, got: {:?}",
    argv
  );
}

#[test]
fn herdr_split_pane_uses_pane_split_with_direction() {
  // `-p` → split the current pane. `herdr pane split` needs `--current` to
  // target the caller's pane, and `--direction` is not optional (its clap
  // arg has no default): omitting it makes herdr reject the call. `right`
  // is the analogue of tmux's `-h`. The pane-direction preference is filed
  // separately; until it lands, `right` is hardcoded.
  let argv = build_herdr_command(
    "feat-7-foo",
    Path::new("/tmp/wt/feat-7-foo"),
    SpawnMode::Split(SplitDirection::Right),
    None,
  );
  assert_eq!(argv[0], "herdr");
  assert_eq!(argv[1], "pane");
  assert_eq!(argv[2], "split");
  // No `--workspace` on a split: `--current` names the caller's pane, which
  // already resolves the workspace. Measured on 0.8.2, the new pane came
  // back in `w2K` while the server's focused workspace was `w2P`.
  assert!(
    !argv.iter().any(|a| a == "--workspace"),
    "`--current` already pins the workspace on a split, got: {:?}",
    argv
  );
  assert!(
    argv.iter().any(|a| a == "--current"),
    "split must target the current pane, got: {:?}",
    argv
  );
  let has_direction = argv.windows(2).any(|w| w[0] == "--direction" && w[1] == "right");
  assert!(has_direction, "expected `--direction right` in argv, got: {:?}", argv);
  let has_cwd = argv.windows(2).any(|w| w[0] == "--cwd" && w[1] == "/tmp/wt/feat-7-foo");
  assert!(has_cwd, "pane split must set `--cwd`, got: {:?}", argv);
  // `herdr pane split` takes no `--label` (panes are renamed after the fact
  // via `herdr pane rename`), so forwarding the worktree name here would be
  // parsed as the optional `[PANE_ID]` positional and split the wrong pane.
  assert!(
    !argv.iter().any(|a| a == "--label"),
    "pane split must NOT carry `--label` (herdr rejects it on panes), got: {:?}",
    argv
  );
  assert!(
    !argv.iter().any(|a| a == "feat-7-foo"),
    "the worktree name must not leak into argv as a bare positional (herdr would read it as PANE_ID), got: {:?}",
    argv
  );
}

// --------------------------------------------------------------------------
// multiplexer detection
// --------------------------------------------------------------------------

#[test]
fn detect_tmux_true_when_tmux_env_set() {
  // Inside a tmux session, `$TMUX` is set to the socket path. Any non-empty
  // value counts — gwm should not parse the socket; only the presence of
  // the variable matters for the gate.
  assert!(detect_tmux(Some("/private/tmp/tmux-501/default,12345,0".to_string())));
  assert!(detect_tmux(Some("any-nonempty-string".to_string())));
}

#[test]
fn detect_tmux_false_when_tmux_env_missing_or_empty() {
  // No env var → user is not in tmux. `gwm tmux` should refuse with a
  // clear error in this case, never silently spawn a server-less tmux
  // command.
  assert!(!detect_tmux(None));
  // Empty string is treated as "not set" — matches what shells emit for
  // `unset TMUX; echo "${TMUX-}"`.
  assert!(!detect_tmux(Some(String::new())));
}

#[test]
fn detect_zellij_true_when_zellij_env_set() {
  // Inside a zellij session, `$ZELLIJ` is set to "0" (or the session
  // socket id depending on the version). Presence is the gate, value is
  // not parsed.
  assert!(detect_zellij(Some("0".to_string())));
  assert!(detect_zellij(Some("any-nonempty-string".to_string())));
}

#[test]
fn detect_zellij_false_when_zellij_env_missing_or_empty() {
  assert!(!detect_zellij(None));
  assert!(!detect_zellij(Some(String::new())));
}

#[test]
fn detect_herdr_true_when_herdr_env_set() {
  // Inside a herdr-managed pane, `$HERDR_ENV` is `1` (alongside
  // HERDR_PANE_ID / HERDR_TAB_ID / HERDR_SOCKET_PATH). Presence is the
  // gate; the value is deliberately not parsed, so a future herdr that
  // exports something richer than `1` keeps working.
  assert!(detect_herdr(Some("1".to_string())));
  assert!(detect_herdr(Some("any-nonempty-string".to_string())));
}

#[test]
fn detect_herdr_false_when_herdr_env_missing_or_empty() {
  assert!(!detect_herdr(None));
  assert!(!detect_herdr(Some(String::new())));
}

// --------------------------------------------------------------------------
// split direction (#589)
// --------------------------------------------------------------------------
//
// Up to 1.9 a `Split` carried no direction at all: tmux fell back to `-v`
// and stacked, zellij to "the biggest available space", and herdr to the
// `right` this module hardcoded. Three backends, three answers to the same
// keystroke. `SplitDirection` is that answer, and each backend spells it
// its own way — which is the whole reason these assertions are per-backend
// rather than one shared loop.

#[test]
fn tmux_translates_the_direction_into_h_or_v() {
  // tmux names the axis the divider runs along, not where the pane goes:
  // `-h` is the HORIZONTAL split, and it puts the new pane to the RIGHT.
  // Getting this pair backwards is silent — both flags are valid, so the
  // only symptom is a pane in the wrong half.
  let right = build_tmux_command("feat-7-foo", path(), SpawnMode::Split(SplitDirection::Right));
  assert_eq!(right[2], "-h", "right is tmux's horizontal split, got: {:?}", right);
  let down = build_tmux_command("feat-7-foo", path(), SpawnMode::Split(SplitDirection::Down));
  assert_eq!(down[2], "-v", "down is tmux's vertical split, got: {:?}", down);
  // A window takes neither: `tmux new-window -h` is an error, not a hint.
  let window = build_tmux_command("feat-7-foo", path(), SpawnMode::Window);
  assert!(
    !window.iter().any(|a| a == "-h" || a == "-v"),
    "new-window must carry no split flag, got: {:?}",
    window
  );
}

#[test]
fn zellij_passes_the_direction_it_would_otherwise_guess() {
  // `zellij action new-pane` documents "if no direction is specified, it
  // will try to use the biggest available space" — a layout-dependent
  // answer, which is exactly what #589 takes away from it.
  for dir in SplitDirection::ALL {
    let argv = build_zellij_command("feat-7-foo", path(), SpawnMode::Split(dir));
    let has_dir = argv.windows(2).any(|w| w[0] == "--direction" && w[1] == dir.label());
    assert!(has_dir, "expected `--direction {}`, got: {:?}", dir.label(), argv);
  }
  // `new-tab` has no direction to take.
  let argv = build_zellij_command("feat-7-foo", path(), SpawnMode::Window);
  assert!(
    !argv.iter().any(|a| a == "--direction"),
    "new-tab must carry no direction, got: {:?}",
    argv
  );
}

#[test]
fn herdr_passes_the_direction_it_used_to_hardcode() {
  for dir in SplitDirection::ALL {
    let argv = build_herdr_command("feat-7-foo", path(), SpawnMode::Split(dir), None);
    let has_dir = argv.windows(2).any(|w| w[0] == "--direction" && w[1] == dir.label());
    assert!(has_dir, "expected `--direction {}`, got: {:?}", dir.label(), argv);
  }
}

#[test]
fn the_direction_label_is_the_spelling_every_surface_uses() {
  // One string serves the `[tui] mux_pane_direction` value, the
  // `--direction` flag, and the argument zellij and herdr take. A rename
  // here silently desyncs the config from the argv.
  assert_eq!(SplitDirection::Right.label(), "right");
  assert_eq!(SplitDirection::Down.label(), "down");
}

#[test]
fn a_herdr_split_ignores_the_workspace_id_in_either_direction() {
  // `build_command` forwards `$HERDR_WORKSPACE_ID` to every backend
  // unconditionally, which is only safe because `pane split` drops it —
  // `--current` already resolves the workspace, and herdr rejects the pair.
  // Without this assertion the dispatcher could start sending `--workspace`
  // to a split and nothing would say so until a user ran it.
  for dir in SplitDirection::ALL {
    assert_eq!(
      build_herdr_command("feat-7-foo", path(), SpawnMode::Split(dir), Some("w2K")),
      build_herdr_command("feat-7-foo", path(), SpawnMode::Split(dir), None),
      "a workspace id must make no difference to a split ({})",
      dir.label()
    );
  }
}

// --------------------------------------------------------------------------
// the cascade the TUI runs on (#588 / #589)
// --------------------------------------------------------------------------
//
// `t` and the `mux_pane` macro both have to answer "which multiplexer am I
// inside, and what is its argv". That answer used to be an if-chain written
// out twice, once per call site, which is why adding a third backend was two
// edits that could disagree. `detect_multiplexer` is the one answer and
// `build_command` the one dispatcher, and the three env values are
// parameters so these tests never touch the process environment:
// `tui_app_tests` rewrites variables under a lock precisely because `$TMUX`
// is read by the clipboard path too.

fn path() -> &'static Path {
  Path::new("/tmp/wt/feat-7-foo")
}

#[test]
fn detect_multiplexer_prefers_tmux_over_the_other_two() {
  // Order matters, and it is the reason herdr goes last: someone running gwm
  // inside a tmux session nested in a herdr pane has both variables set, and
  // #588 must not move them onto the newer backend.
  //
  // It is also what makes the TUI immune to herdrdev/herdr#2134: a tmux
  // server started from a herdr pane promotes the whole `HERDR_*` set to its
  // server-global environment, so unrelated sessions on that server claim a
  // pane they do not own. Every one of them has `$TMUX` set, so this
  // assertion is the guard, not a preference.
  let mux = detect_multiplexer(
    Some("/tmp/tmux-501/default,1,0".into()),
    Some("0".into()),
    Some("1".into()),
  )
  .expect("tmux is active, so a multiplexer must come back");
  assert_eq!(mux, Multiplexer::Tmux);
}

#[test]
fn detect_multiplexer_falls_through_to_zellij_then_herdr() {
  let mux = detect_multiplexer(None, Some("0".into()), Some("1".into()))
    .expect("zellij is active, so a multiplexer must come back");
  assert_eq!(mux, Multiplexer::Zellij, "zellij beats herdr");

  let mux = detect_multiplexer(None, None, Some("1".into())).expect("herdr is active, so a multiplexer must come back");
  assert_eq!(mux, Multiplexer::Herdr, "herdr must be reached last");
}

#[test]
fn detect_multiplexer_is_none_when_nothing_is_active() {
  // The three empty strings are not padding: a shell that ran `unset TMUX`
  // and a shell that never had it both surface as an empty value through
  // some wrappers, and neither means "inside a multiplexer".
  assert!(detect_multiplexer(None, None, None).is_none());
  assert!(detect_multiplexer(Some(String::new()), Some(String::new()), Some(String::new())).is_none());
}

#[test]
fn build_command_dispatches_to_the_matching_backend() {
  // The three call sites used to each write this match out; the CLI's copy
  // is the only one that knew about `$HERDR_WORKSPACE_ID`.
  let mode = SpawnMode::Split(SplitDirection::Down);
  assert_eq!(
    build_command(Multiplexer::Tmux, "feat-7-foo", path(), mode, Some("w2K")),
    build_tmux_command("feat-7-foo", path(), mode)
  );
  assert_eq!(
    build_command(Multiplexer::Zellij, "feat-7-foo", path(), mode, Some("w2K")),
    build_zellij_command("feat-7-foo", path(), mode)
  );
  assert_eq!(
    build_command(Multiplexer::Herdr, "feat-7-foo", path(), SpawnMode::Window, Some("w2K")),
    build_herdr_command("feat-7-foo", path(), SpawnMode::Window, Some("w2K")),
    "the workspace id must reach `tab create`"
  );
}

// --------------------------------------------------------------------------
// what a macro can actually run (#290 / #589)
// --------------------------------------------------------------------------

#[test]
fn a_macro_is_refused_by_the_backends_with_no_trailing_command_form() {
  // Running a command in a herdr pane is `herdr pane run <pane-id> <cmd>`,
  // and the id only comes back in the JSON `pane split` prints (#599), so
  // herdr is refused in both modes.
  for mode in [SpawnMode::Window, SpawnMode::Split(SplitDirection::Right)] {
    assert!(
      macro_refusal(Multiplexer::Herdr, mode).is_some(),
      "herdr carries no macro command in {:?}",
      mode
    );
  }
  // `zellij action new-tab` takes no command either, which only became
  // reachable when `mux_pane_direction = "window"` shipped (#589).
  assert!(
    macro_refusal(Multiplexer::Zellij, SpawnMode::Window).is_some(),
    "a zellij tab carries no macro command"
  );
}

#[test]
fn a_macro_runs_wherever_a_trailing_command_exists() {
  // Splitting anyway where the command cannot follow would open an empty
  // pane and drop the macro silently, so the refusals above are worth their
  // arms — but they must not spread. tmux takes a trailing command on both
  // verbs, zellij on `new-pane` behind `--`.
  for mode in [SpawnMode::Window, SpawnMode::Split(SplitDirection::Down)] {
    assert!(
      macro_refusal(Multiplexer::Tmux, mode).is_none(),
      "tmux takes a trailing command in {:?}",
      mode
    );
  }
  assert!(
    macro_refusal(Multiplexer::Zellij, SpawnMode::Split(SplitDirection::Down)).is_none(),
    "`zellij action new-pane -- <cmd>` takes one"
  );
}

// --------------------------------------------------------------------------
// Multiplexer enum: name + binary
// --------------------------------------------------------------------------

#[test]
fn multiplexer_binary_matches_verb() {
  // The `Multiplexer::binary()` helper exists so the spawn site doesn't
  // duplicate the string literal — and so the not-running error message
  // can name the right multiplexer in one line.
  assert_eq!(Multiplexer::Tmux.binary(), "tmux");
  assert_eq!(Multiplexer::Zellij.binary(), "zellij");
  assert_eq!(Multiplexer::Herdr.binary(), "herdr");
}
