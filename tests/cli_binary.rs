//! End-to-end tests that invoke the compiled `gwm` binary via assert_cmd.
//! These exercise the user-visible CLI surface (subcommands, help, errors).

mod common;

use assert_cmd::Command;
use common::init_repo;
use predicates::prelude::*;

#[test]
fn help_prints_subcommands() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.arg("--help");
  // Match the subcommand-listing column exactly: clap aligns subcommand
  // names with two leading spaces and at least two trailing spaces before
  // the description. A loose `contains("cd")` would also match prose like
  // "to cd into it" in another subcommand's description.
  // `cd` is now a visible alias of `path` (clap renders it as
  // `path  ...  [aliases: cd]`), so we assert the alias marker
  // rather than a separate `  cd ` row.
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("  init "))
    .stdout(predicate::str::contains("  list "))
    .stdout(predicate::str::contains("  create "))
    .stdout(predicate::str::contains("  path "))
    .stdout(predicate::str::contains("[aliases: cd]"))
    .stdout(predicate::str::contains("  bootstrap "))
    .stdout(predicate::str::contains("  prune "))
    .stdout(predicate::str::contains("  completions "))
    .stdout(predicate::str::contains("  shell-init "))
    .stdout(predicate::str::contains("  switch "))
    .stdout(predicate::str::contains("  doctor "));
}

#[test]
fn switch_alias_s_resolves_to_switch() {
  // Issue #22: `gwm s` is the daily-driver alias for `gwm switch`. Resolving
  // `gwm s --help` to the switch help text proves the alias is wired in.
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["s", "--help"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("Open an interactive picker"));
}

#[test]
fn top_level_help_advertises_switch_alias() {
  // The visible alias must show up in the top-level help summary so users
  // discover `gwm s` without reading the subcommand-specific page.
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.arg("--help");
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("switch").and(predicate::str::contains("[aliases: s]")));
}

#[test]
fn switch_outside_git_repo_fails() {
  // Same contract as the other repo-bound subcommands: bail out with a
  // clear error rather than dropping the user into an empty TUI.
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .arg("switch")
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn doctor_on_fresh_repo_prints_checks() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  // Exit code is intentionally not asserted to be 0: in environments
  // without `lazygit` on PATH (most CI runners) or a pre-existing
  // `~/cc-worktree/` parent, the report legitimately surfaces Warning
  // entries and exits 1. But we still bound the contract — anything
  // other than 0 (all green) or 1 (warning) on a vanilla fresh repo
  // means the doctor is over-flagging (or panicking, which would also
  // produce a non-0/1 code) and we want the test to fail loudly.
  // The 0/1/2 mapping is unit-tested in `tests/doctor_tests.rs`.
  cmd
    .current_dir(dir.path())
    .arg("doctor")
    .assert()
    .code(predicate::in_iter([0_i32, 1]))
    .stdout(predicate::str::contains("✓"))
    .stdout(predicate::str::contains(".gwm.toml"))
    .stdout(predicate::str::contains("base directory writable"));
}

#[test]
fn doctor_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .arg("doctor")
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn doctor_exits_two_on_invalid_config() {
  let (dir, _repo) = init_repo();
  std::fs::write(dir.path().join(".gwm.toml"), "broken = [unterminated").unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .arg("doctor")
    .assert()
    .code(2)
    .stdout(predicate::str::contains("✗"));
}

#[test]
fn version_flag() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.arg("--version");
  cmd.assert().success().stdout(predicate::str::contains("gwm"));
}

#[test]
fn types_lists_branch_types() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.arg("types");
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("feat"))
    .stdout(predicate::str::contains("fix"))
    .stdout(predicate::str::contains("hotfix"))
    .stdout(predicate::str::contains("chore"));
}

#[test]
fn create_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.current_dir(dir.path()).arg("list");
  cmd
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn completions_zsh_emits_compdef_header() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "zsh"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("#compdef gwm"))
    .stdout(predicate::str::contains("_gwm"));
}

#[test]
fn completions_bash_emits_complete_directive() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "bash"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("_gwm"))
    .stdout(predicate::str::contains("complete "));
}

#[test]
fn completions_fish_emits_complete_directive() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "fish"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("complete -c gwm"));
}

#[test]
fn completions_powershell_emits_register_block() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "powershell"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("Register-ArgumentCompleter"))
    .stdout(predicate::str::contains("gwm"));
}

#[test]
fn completions_rejects_unknown_shell() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["completions", "tcsh"]);
  cmd.assert().failure();
}

#[test]
fn list_format_names_emits_one_name_per_line() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["list", "--format=names"])
    .assert()
    .success()
    // No table header, no leading "*" marker, no STATUS column.
    .stdout(predicate::str::contains("NAME").not())
    .stdout(predicate::str::contains("STATUS").not())
    // Main worktree is excluded so the output mirrors what `find_fuzzy`
    // accepts (path/remove/bootstrap skip the main workdir). A fresh repo
    // therefore prints nothing.
    .stdout(predicate::str::is_empty());
}

#[test]
fn cd_unknown_pattern_fails_with_not_found() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["cd", "nope"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not found"));
}

#[test]
fn cd_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["cd", "anything"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

// Use `function gcd { ... }` (zsh-style, also valid in bash) rather than
// `gcd() { ... }`. The latter triggers `zsh: defining function based on
// alias 'gcd'` at parse time when an alias of the same name already
// exists — even if a preceding `unalias` would remove it, since zsh
// parses the whole eval'd block before running any of it.
#[test]
fn shell_init_bash_emits_gcd_function() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "bash"]);
  // Pin the invocation site: a regression that mentions `gwm cd` only in
  // a comment but doesn't actually call it from the function body would
  // pass a loose `contains("gwm cd")`. `gwm cd "$@"` (POSIX-style
  // argument forwarding) is what makes `gcd auth` translate into
  // `gwm cd auth`.
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("function gcd"))
    .stdout(predicate::str::contains("gwm cd \"$@\""));
}

#[test]
fn shell_init_zsh_emits_gcd_function() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "zsh"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("function gcd"))
    .stdout(predicate::str::contains("gwm cd \"$@\""));
}

#[test]
fn shell_init_posix_does_not_use_paren_function_syntax() {
  for shell in ["bash", "zsh"] {
    let mut cmd = Command::cargo_bin("gwm").unwrap();
    cmd.args(["shell-init", shell]);
    // `gcd()` is the form that explodes under an existing alias.
    cmd.assert().success().stdout(predicate::str::contains("gcd()").not());
  }
}

// Regression: in zsh, an existing alias (e.g. `gcd='git checkout'` from
// oh-my-zsh's git plugin) wins over a same-named function and refuses to
// be shadowed at definition time ("defining function based on alias").
// The init script must `unalias gcd` first so the function takes effect
// regardless of the user's prior aliases.
#[test]
fn shell_init_posix_unaliases_gcd_first() {
  for shell in ["bash", "zsh"] {
    let mut cmd = Command::cargo_bin("gwm").unwrap();
    cmd.args(["shell-init", shell]);
    cmd.assert().success().stdout(predicate::str::contains("unalias gcd"));
  }
}

#[test]
fn shell_init_powershell_unaliases_gcd_first() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "powershell"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("Remove-Alias"))
    .stdout(predicate::str::contains("gcd"));
}

#[test]
fn shell_init_fish_emits_function_block() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "fish"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("function gcd"))
    .stdout(predicate::str::contains("gwm cd $argv"))
    .stdout(predicate::str::contains("end"));
}

// Regression: fish performs wildcard expansion on unquoted variables, so
// `cd $target` would mangle paths containing `[`, `]`, or `*`. The
// emitted helper must use `cd -- "$target"` to disable both option
// parsing and glob expansion.
#[test]
fn shell_init_fish_quotes_target_with_double_dash() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "fish"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("cd -- \"$target\""));
}

#[test]
fn shell_init_powershell_emits_function_block() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "powershell"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("function gcd"))
    .stdout(predicate::str::contains("gwm cd $Pattern"))
    .stdout(predicate::str::contains("Set-Location"));
}

// Issue #22: with no argument, `gcd` should drop into `gwm switch` (the
// interactive picker) rather than print a usage error. The wrapper must
// still bail before cd'ing if the picker is cancelled (non-zero exit).
#[test]
fn shell_init_posix_no_arg_invokes_switch() {
  for shell in ["bash", "zsh"] {
    let mut cmd = Command::cargo_bin("gwm").unwrap();
    cmd.args(["shell-init", shell]);
    cmd
      .assert()
      .success()
      .stdout(predicate::str::contains("gwm switch"))
      .stdout(predicate::str::contains("\"$#\" -eq 0"));
  }
}

#[test]
fn shell_init_fish_no_arg_invokes_switch() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "fish"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("gwm switch"))
    .stdout(predicate::str::contains("count $argv) -eq 0"));
}

#[test]
fn shell_init_powershell_no_arg_invokes_switch() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "powershell"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("gwm switch"))
    .stdout(predicate::str::contains("IsNullOrEmpty"));
}

#[test]
fn shell_init_rejects_unknown_shell() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "tcsh"]);
  cmd.assert().failure();
}

#[test]
fn list_format_table_is_default() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .arg("list")
    .assert()
    .success()
    .stdout(predicate::str::contains("NAME"))
    .stdout(predicate::str::contains("STATUS"));
}
