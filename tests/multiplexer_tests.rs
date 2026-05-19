//! Pure-logic tests for `gwm::multiplexer`. The module's command builders
//! and env-var probes are deliberately decoupled from any process spawn, so
//! these tests do not require `tmux` or `zellij` to be installed on the
//! runner — they assert against the produced argv vectors.

use gwm::multiplexer::{build_tmux_command, build_zellij_command, detect_tmux, detect_zellij, Multiplexer, SpawnMode};
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
  // With `-p` (SpawnMode::Split) we want a horizontal/vertical split in the
  // current window, not a new window. `tmux split-window -c <path>` is the
  // shape — no `-n` because tmux panes don't carry a name attribute.
  let argv = build_tmux_command("feat-12-x", Path::new("/tmp/wt/feat-12-x"), SpawnMode::Split);
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
  let argv = build_zellij_command("feat-7-foo", Path::new("/tmp/wt/feat-7-foo"), SpawnMode::Split);
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
}
