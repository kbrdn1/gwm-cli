//! End-to-end tests that invoke the compiled `gwm` binary via assert_cmd.
//! These exercise the user-visible CLI surface (subcommands, help, errors).

mod common;

use assert_cmd::Command;
use common::init_repo;
use predicates::prelude::*;
use std::path::Path;

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
    .stdout(predicate::str::contains("  tmux "))
    .stdout(predicate::str::contains("  zellij "))
    .stdout(predicate::str::contains("  doctor "))
    .stdout(predicate::str::contains("  link "))
    .stdout(predicate::str::contains("  unlink "))
    .stdout(predicate::str::contains("  open "))
    .stdout(predicate::str::contains("  status "))
    // Issue #81: declarative GitHub labels.
    .stdout(predicate::str::contains("  labels "))
    // Issue #82: declarative GitHub milestones.
    .stdout(predicate::str::contains("  milestones "))
    // Issue #95: TOFU trust ledger.
    .stdout(predicate::str::contains("  trust "))
    // Issue #86: CLI aliases (`gwm aliases list`).
    .stdout(predicate::str::contains("  aliases "));
}

// --- labels (issue #81) -------------------------------------------------

#[test]
fn labels_help_lists_list_and_push() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["labels", "--help"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("list"))
    .stdout(predicate::str::contains("push"));
}

#[test]
fn labels_list_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["labels", "list"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn labels_list_with_no_declared_labels_is_a_no_op() {
  // The no-op fast path: no `[[labels]]` in .gwm.toml ⇒ don't shell
  // out to `gh`, just print a single line and exit 0. The test
  // doesn't need `gh` on PATH to pass — that's the whole point.
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["labels", "list"])
    .assert()
    .success()
    .stdout(predicate::str::contains("0 labels declared"));
}

#[test]
fn labels_push_with_no_declared_labels_is_a_no_op() {
  // Same fast path as `list`. Push must not call `gh` when there's
  // nothing to push — that would surface `gh: not found` to users
  // who haven't yet configured `[[labels]]` and tried the command.
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["labels", "push"])
    .assert()
    .success()
    .stdout(predicate::str::contains("0 labels declared"));
}

#[test]
fn labels_push_dry_run_with_no_declared_labels_succeeds() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["labels", "push", "--dry-run"])
    .assert()
    .success()
    .stdout(predicate::str::contains("0 labels declared"));
}

#[test]
fn labels_list_surfaces_invalid_color_with_label_name() {
  // A typo in `color` must be caught at resolve time with the label
  // name in the error message — otherwise the user has to grep their
  // config to find which entry is broken.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[[labels]]
name = "bug"
color = "not-a-hex"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["labels", "list"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("bug"))
    .stderr(predicate::str::contains("not-a-hex"));
}

// --- milestones (issue #82) ---------------------------------------------

#[test]
fn milestones_help_lists_list_and_push() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["milestones", "--help"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("list"))
    .stdout(predicate::str::contains("push"));
}

#[test]
fn milestones_list_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["milestones", "list"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn milestones_list_with_no_declared_is_a_no_op() {
  // No `[[milestones]]` in .gwm.toml ⇒ don't shell out to `gh`. The
  // test passes without `gh` on PATH — that's the whole point of the
  // fast path.
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["milestones", "list"])
    .assert()
    .success()
    .stdout(predicate::str::contains("0 milestones declared"));
}

#[test]
fn milestones_push_with_no_declared_is_a_no_op() {
  // Same fast path as `list`. Push must not call `gh` when there's
  // nothing to push.
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["milestones", "push"])
    .assert()
    .success()
    .stdout(predicate::str::contains("0 milestones declared"));
}

#[test]
fn milestones_push_dry_run_with_no_declared_succeeds() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["milestones", "push", "--dry-run"])
    .assert()
    .success()
    .stdout(predicate::str::contains("0 milestones declared"));
}

#[test]
fn milestones_list_surfaces_invalid_due_on_with_title() {
  // A typo in `due_on` must be caught at resolve time with the
  // milestone title in the error message — otherwise the user has to
  // grep their config to find the offending entry.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[[milestones]]
title = "v0.7.0"
due_on = "not-a-date"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["milestones", "list"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("v0.7.0"));
}

#[test]
fn milestones_list_surfaces_invalid_state_with_title() {
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[[milestones]]
title = "v0.7.0"
state = "draft"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["milestones", "list"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("v0.7.0"))
    .stderr(predicate::str::contains("draft"));
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
fn doctor_exits_one_when_review_binary_missing() {
  // Issue #75: configuring [review] with a binary that's not on $PATH
  // must produce a Warning (exit code 1), not a Failed (exit code 2).
  // Review is opt-in — a CI pre-commit hook gated on `gwm doctor`
  // should still let the user push when only the review tool is
  // missing locally.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[review]
command = "definitely-not-on-path-review-cli {base}..{head}"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .arg("doctor")
    .assert()
    .code(1)
    .stdout(predicate::str::contains("definitely-not-on-path-review-cli"));
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
  // Outside any repo (the temp cwd of `assert_cmd` is typically not a
  // git repo on CI runners) `gwm types` must still list the built-in
  // defaults and surface the `built-in defaults` footer so users can
  // tell they're not looking at a `.gwm.toml` override.
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.current_dir(dir.path()).arg("types");
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("feat"))
    .stdout(predicate::str::contains("fix"))
    .stdout(predicate::str::contains("hotfix"))
    .stdout(predicate::str::contains("chore"))
    .stdout(predicate::str::contains("(source: built-in defaults)"));
}

#[test]
fn types_lists_configured_branch_types_from_dot_gwm_toml() {
  // When `.gwm.toml` carries a `[[branch_types]]` block, `gwm types`
  // must reflect the override verbatim and update its source footer.
  // The legacy entries that aren't in the override must NOT bleed
  // through (e.g. `hotfix` is part of the built-in defaults but is
  // intentionally absent from this repo's list).
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[[branch_types]]
name = "feat"
description = "Feature"

[[branch_types]]
name = "migration"
description = "Database migration"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.current_dir(dir.path()).arg("types");
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("feat"))
    .stdout(predicate::str::contains("migration"))
    .stdout(predicate::str::contains("Database migration"))
    .stdout(predicate::str::contains("(source: .gwm.toml)"))
    .stdout(predicate::str::contains("hotfix").not());
}

#[test]
fn types_inside_bare_repo_falls_back_to_built_in_defaults() {
  // Bare repos have no `workdir()`, so there's no place to look for a
  // `.gwm.toml`. Regression: previously `cmd_types` would propagate a
  // misleading `NotInGitRepo` error here even though `discover_repo`
  // succeeded. The fallback is identical to the "outside any repo"
  // branch — the built-in defaults with a `(source: built-in
  // defaults)` footer.
  let dir = tempfile::TempDir::new().unwrap();
  git2::Repository::init_bare(dir.path()).expect("init bare repo");
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.current_dir(dir.path()).arg("types");
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("feat"))
    .stdout(predicate::str::contains("hotfix"))
    .stdout(predicate::str::contains("(source: built-in defaults)"));
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

// Regression: in zsh, an existing alias (e.g. `gcd='git checkout'` from
// oh-my-zsh's git plugin) wins over a same-named function and refuses to
// be shadowed at definition time ("defining function based on alias").
// The init script must `unalias gcd` first so the function takes effect
// regardless of the user's prior aliases.
#[test]
fn shell_init_posix_unaliases_gcd_first() {
  // regression: oh-my-zsh's git plugin defines `gcd='git checkout'` which
  // shadowed our gcd function ("defining function based on alias").
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
  // regression: fish wildcard expansion mangled paths containing `[`, `]`,
  // or `*` when the helper used `cd $target` without `cd -- "$target"`.
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

// Issue #58: users who eval the wrapper without reading the README must
// still discover the no-arg route. Pin a one-line cheat-sheet comment in
// the script header that names the bridge to `gwm switch`. The exact
// phrase `picker via `gwm switch`` is asserted because the README, the
// CLI --help, and the wrapper now share that wording — drift in one
// surface should break the test instead of going unnoticed.
#[test]
fn shell_init_posix_header_documents_no_arg_route() {
  for shell in ["bash", "zsh"] {
    let mut cmd = Command::cargo_bin("gwm").unwrap();
    cmd.args(["shell-init", shell]);
    cmd
      .assert()
      .success()
      .stdout(predicate::str::contains("picker via `gwm switch`"));
  }
}

#[test]
fn shell_init_fish_header_documents_no_arg_route() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "fish"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("picker via `gwm switch`"));
}

#[test]
fn shell_init_powershell_header_documents_no_arg_route() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["shell-init", "powershell"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("picker via `gwm switch`"));
}

// Issue #58: a user who lands on `gwm switch --help` first (e.g. via tab
// completion) should learn that the recommended invocation is the `gcd`
// wrapper from `gwm shell-init`, not the raw `cd "$(gwm switch)"` form.
#[test]
fn switch_help_mentions_gcd_wrapper() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["switch", "--help"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("gcd").and(predicate::str::contains("shell-init")));
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

// --------------------------------------------------------------------------
// Issue #23 — `gwm tmux <pattern>` / `gwm zellij <pattern>`
// --------------------------------------------------------------------------
//
// The actual spawn (`std::process::Command::new("tmux").args(...)`) is out
// of scope here — driving it would require a live tmux/zellij server on
// every CI runner. We instead pin the user-visible contract:
//
//   1. The subcommands exist and are listed in `gwm --help`.
//   2. Outside the corresponding multiplexer (no `$TMUX` / `$ZELLIJ`),
//      the command exits non-zero with a clear stderr that names the
//      missing multiplexer — no silent no-op.
//   3. Outside a git repo, the standard `NotInGitRepo` error wins.
//   4. The argv-builder unit tests in `tests/multiplexer_tests.rs`
//      cover what gets handed to the spawn.

#[test]
fn tmux_outside_tmux_session_fails_with_clear_error() {
  // CI runners and most local shells don't have `$TMUX` set. The command
  // must refuse loudly rather than spawn `tmux` against no server, which
  // would either start a brand-new tmux server or error opaquely.
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env_remove("TMUX")
    .args(["tmux", "anything"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("tmux").and(predicate::str::contains("not")));
}

#[test]
fn zellij_outside_zellij_session_fails_with_clear_error() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env_remove("ZELLIJ")
    .args(["zellij", "anything"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("zellij").and(predicate::str::contains("not")));
}

#[test]
fn tmux_outside_git_repo_fails() {
  // `NotInGitRepo` wins over the multiplexer-not-running gate when both
  // apply — the user is told to fix the more fundamental problem first.
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env_remove("TMUX")
    .args(["tmux", "anything"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn zellij_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env_remove("ZELLIJ")
    .args(["zellij", "anything"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

// regression: PR #65 Copilot review — the not-running error carried a
// literal backslash before the env var name (`\$TMUX` instead of
// `$TMUX`). The `\\` came from a shell-escape habit; this is stderr,
// not shell source, so the dollar must render bare.
#[test]
fn tmux_outside_tmux_error_does_not_escape_dollar() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env_remove("TMUX")
    .args(["tmux", "anything"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("\\$").not())
    // And `$TMUX` itself must still appear so the hint is actionable.
    .stderr(predicate::str::contains("$TMUX"));
}

#[test]
fn zellij_outside_zellij_error_does_not_escape_dollar() {
  let (dir, _repo) = init_repo();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env_remove("ZELLIJ")
    .args(["zellij", "anything"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("\\$").not())
    .stderr(predicate::str::contains("$ZELLIJ"));
}

#[test]
fn tmux_help_mentions_split_flag() {
  // The `-p` flag (split-pane instead of new-window) is the one knob users
  // care about; it must show up in `--help` so it's discoverable without
  // reading the README.
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["tmux", "--help"]);
  cmd.assert().success().stdout(predicate::str::contains("--split"));
}

#[test]
fn zellij_help_mentions_split_flag() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["zellij", "--help"]);
  cmd.assert().success().stdout(predicate::str::contains("--split"));
}

// --- Issue/PR linking (issue #67) ----------------------------------------

#[test]
fn link_help_documents_issue_and_pr_targets() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["link", "--help"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("issue"))
    .stdout(predicate::str::contains("pr"));
}

#[test]
fn unlink_help_documents_issue_and_pr_targets() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["unlink", "--help"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("issue"))
    .stdout(predicate::str::contains("pr"));
}

#[test]
fn open_help_documents_issue_and_pr_and_print_url() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["open", "--help"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("issue"))
    .stdout(predicate::str::contains("pr"))
    .stdout(predicate::str::contains("--print-url"));
}

#[test]
fn status_help_documents_json_flag() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["status", "--help"]);
  cmd.assert().success().stdout(predicate::str::contains("--json"));
}

#[test]
fn link_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .args(["link", "issue", "42"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn link_issue_persists_and_status_reflects_it() {
  // E2E: link an issue then `gwm status --json` reads it back.
  // The repo has no remote configured so `status` runs in "local link only" mode
  // (no GitHub fetch). The JSON output should still carry the linked number.
  let (dir, repo) = init_repo();
  // Need a branch that's checked out for the CWD-resolved status to find it.
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("feat/#42-tui-search", &head, false).unwrap();
  repo.set_head("refs/heads/feat/#42-tui-search").unwrap();

  // link
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["link", "issue", "99"])
    .assert()
    .success()
    .stdout(predicate::str::contains("issue #99"));

  // status --json
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["status", "--json"])
    .assert()
    .success()
    .stdout(predicate::str::contains("\"issue\""))
    .stdout(predicate::str::contains("99"));
}

#[test]
fn unlink_issue_falls_back_to_branch_name_auto_detect() {
  let (dir, repo) = init_repo();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("feat/#42-tui-search", &head, false).unwrap();
  repo.set_head("refs/heads/feat/#42-tui-search").unwrap();

  // Override then unlink.
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["link", "issue", "99"])
    .assert()
    .success();
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["unlink", "issue"])
    .assert()
    .success();

  // After unlink the branch-name auto-detect should resurface (issue #42).
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["status", "--json"])
    .assert()
    .success()
    .stdout(predicate::str::contains("42"));
}

#[test]
fn open_print_url_emits_url_without_spawning_browser() {
  // `--print-url` is the test-friendly mode: we want to assert the URL
  // construction without actually shelling out to `open`/`xdg-open`.
  let (dir, repo) = init_repo();
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("feat/#42-tui-search", &head, false).unwrap();
  repo.set_head("refs/heads/feat/#42-tui-search").unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["open", "issue", "--print-url"])
    .assert()
    .success()
    .stdout(predicate::str::contains("https://github.com/kbrdn1/gwm-cli/issues/42"));
}

#[test]
fn open_pr_without_link_fails_clearly() {
  let (dir, repo) = init_repo();
  repo.remote("origin", "https://github.com/kbrdn1/gwm-cli.git").unwrap();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  // Branch without an issue number (no auto-detect) and no explicit PR link.
  repo.branch("random-branch", &head, false).unwrap();
  repo.set_head("refs/heads/random-branch").unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["open", "pr", "--print-url"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("no PR linked"));
}

#[test]
fn status_on_branch_with_no_link_reports_no_link() {
  let (dir, repo) = init_repo();
  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("random-branch", &head, false).unwrap();
  repo.set_head("refs/heads/random-branch").unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["status"])
    .assert()
    .success()
    .stdout(predicate::str::contains("no link"));
}

// --------------------------------------------------------------------------
// Issue #101 — E2E tests for the mutating subcommands (init / create / remove)
// --------------------------------------------------------------------------
//
// `cli_binary.rs` historically asserted only `--help` output and a handful
// of read-only error paths. The three subcommands that mutate state on
// disk (`init`, `create`, `remove`) had no end-to-end coverage, which let
// orchestration-layer regressions (e.g. issues #98 and #99) slip through
// unit-test review. The block below is the canonical CI signal for the
// CLI surface of those subcommands.
//
// Worktree-creating tests pin `[worktree].base` to a `tempfile::TempDir`
// so the test runner never writes under `~/cc-worktree/...`. The branch
// pattern is left at its default (`{type}/#{issue}-{desc}`) to exercise
// the production naming pipeline.

// --- init ---------------------------------------------------------------

#[test]
fn init_writes_gwm_toml_with_expected_sections() {
  // The default `.gwm.toml` shipped by `gwm init` is sourced from
  // `examples/gwm.toml.example`. Asserting on the exact contents would
  // couple the test to the example file character-for-character; we pin
  // the structural markers a user (and the rest of the codebase) relies
  // on: a `[worktree]` block with the documented placeholders, a
  // `[[bootstrap.copy]]` entry, and a `[[bootstrap.guard]]` entry. The
  // stdout line names the written path so users discover where it landed.
  let (dir, _repo) = init_repo();
  let cfg_path = dir.path().join(".gwm.toml");
  assert!(!cfg_path.exists(), "precondition: no .gwm.toml in fresh repo");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .arg("init")
    .assert()
    .success()
    .stdout(predicate::str::contains(".gwm.toml"));

  assert!(cfg_path.exists(), "gwm init must write .gwm.toml on disk");
  let body = std::fs::read_to_string(&cfg_path).unwrap();
  assert!(body.contains("[worktree]"), "missing [worktree] section");
  assert!(
    body.contains("base = ") && body.contains("{home}") && body.contains("{repo}"),
    "missing documented placeholders in [worktree].base"
  );
  assert!(
    body.contains("[[bootstrap.copy]]"),
    "missing [[bootstrap.copy]] template"
  );
  assert!(
    body.contains("[[bootstrap.guard]]"),
    "missing [[bootstrap.guard]] template"
  );
}

#[test]
fn init_refuses_to_overwrite_existing_gwm_toml() {
  // Idempotency contract: a second `gwm init` on a repo that already
  // carries a `.gwm.toml` must bail out instead of silently clobbering
  // user edits. The error must name the file *and* explain why so the
  // user knows what to remove if they truly want to start over. Pin
  // the exact "already exists" wording from `Config::write_default`
  // (src/config.rs) — a looser `.gwm.toml` contains-check would also
  // pass on a success run since the success path prints the same path.
  let (dir, _repo) = init_repo();
  let cfg_path = dir.path().join(".gwm.toml");
  std::fs::write(&cfg_path, "# user edits\n[worktree]\nbase = \"/custom\"\n").unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .arg("init")
    .assert()
    .failure()
    .stderr(predicate::str::contains(".gwm.toml"))
    .stderr(predicate::str::contains("already exists"));

  // Original contents must survive the failed init.
  let body = std::fs::read_to_string(&cfg_path).unwrap();
  assert!(
    body.contains("# user edits"),
    "failed gwm init must not modify the existing .gwm.toml"
  );
}

#[test]
fn init_outside_git_repo_fails() {
  // `cmd_init` calls `discover_repo` first — the standard `NotInGitRepo`
  // error wins so the user is steered to `git init` before configuring.
  let dir = tempfile::TempDir::new().unwrap();
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .arg("init")
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

// --- create -------------------------------------------------------------

/// Write a `.gwm.toml` that redirects `[worktree].base` into the test's
/// own `TempDir`. The caller already owns the `base` path (it's the
/// `TempDir` they passed in), so the test asserts on `base.join(...)`
/// directly — this helper has no return value. Bootstrap is left empty
/// by default so `gwm create` runs in its minimal shape; tests that
/// need bootstrap behaviour layer a second `.gwm.toml` write on top.
fn write_test_config(repo_root: &Path, base: &Path) {
  let body = format!(
    r#"
[worktree]
base = "{base}"
path_pattern = "{{type}}-{{issue}}-{{desc}}"
branch_pattern = "{{type}}/#{{issue}}-{{desc}}"
"#,
    base = base.display(),
  );
  std::fs::write(repo_root.join(".gwm.toml"), body).unwrap();
}

#[test]
fn create_adds_worktree_dir_and_branch_at_head() {
  // The full happy path through `cmd_create`:
  //   1. The dirname rendered from `[worktree].path_pattern` lands on disk
  //      under the configured `[worktree].base`.
  //   2. The branch rendered from `[worktree].branch_pattern` is created
  //      as a local ref pointing at the seed commit (`init_repo`'s HEAD).
  //   3. `branch.<name>.gwm-base` is recorded as the parent's short name
  //      (the launcher fallback chain depends on it — issue #75).
  let (dir, repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  write_test_config(dir.path(), base.path());
  let head_oid = repo.head().unwrap().target().unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    // Issue #95: this test writes a `.gwm.toml` and runs `gwm
    // create` non-interactively, so the TOFU prompt would block.
    // Setting GWM_ALLOW_BOOTSTRAP=1 is the documented CI bypass.
    .env("GWM_ALLOW_BOOTSTRAP", "1")
    .args(["create", "feat", "42", "tui-search"])
    .assert()
    .success()
    .stdout(predicate::str::contains("feat/#42-tui-search"))
    .stdout(predicate::str::contains("worktree created"));

  let wt_dir = base.path().join("feat-42-tui-search");
  assert!(wt_dir.exists(), "worktree dir must exist on disk");
  assert!(wt_dir.join(".git").exists(), "worktree must carry a .git pointer");

  let branch = repo
    .find_branch("feat/#42-tui-search", git2::BranchType::Local)
    .expect("branch must be created");
  let branch_oid = branch.into_reference().target().unwrap();
  assert_eq!(branch_oid, head_oid, "fresh branch must point at the main HEAD commit");

  let cfg = repo.config().unwrap();
  let recorded_base = cfg.get_string("branch.feat/#42-tui-search.gwm-base").unwrap();
  assert_eq!(
    recorded_base, "main",
    "gwm create must record the parent ref for the launcher fallback chain"
  );
}

#[test]
fn create_runs_bootstrap_by_default() {
  // A `[[bootstrap.copy]]` step lands its destination file inside the
  // freshly-created worktree iff bootstrap actually ran. The source file
  // lives in the main repo's workdir; we pin a unique marker string so a
  // false positive (e.g. a cargo lock file collision) can't pass the
  // assertion by accident.
  let (dir, _repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  let marker = "GWM_E2E_BOOTSTRAP_MARKER_v1";
  std::fs::write(dir.path().join("seed.env"), marker).unwrap();
  let body = format!(
    r#"
[worktree]
base = "{base}"
path_pattern = "{{type}}-{{issue}}-{{desc}}"
branch_pattern = "{{type}}/#{{issue}}-{{desc}}"

[[bootstrap.copy]]
from = "seed.env"
to = "seed.env"
required = true
"#,
    base = base.path().display(),
  );
  std::fs::write(dir.path().join(".gwm.toml"), body).unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    // Issue #95: bypass the TOFU prompt — the test's whole point
    // is to assert that bootstrap.copy actually runs, which is
    // gated behind the trust check.
    .env("GWM_ALLOW_BOOTSTRAP", "1")
    .args(["create", "feat", "7", "bootstrap-on"])
    .assert()
    .success();

  let copied = base.path().join("feat-7-bootstrap-on").join("seed.env");
  assert!(copied.exists(), "bootstrap copy step did not run");
  let body = std::fs::read_to_string(&copied).unwrap();
  assert!(
    body.contains(marker),
    "bootstrap copy must duplicate the source file content"
  );
}

#[test]
fn create_skips_bootstrap_with_no_bootstrap_flag() {
  // Same scaffolding as the previous test, but `--no-bootstrap` must
  // short-circuit `bootstrap::run` BEFORE the copy step. The worktree
  // directory still appears (the branch + worktree are created first),
  // but `seed.env` is absent inside it. The stdout breadcrumb that
  // `cmd_create` prints when it skips makes the intent observable.
  let (dir, _repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  std::fs::write(dir.path().join("seed.env"), "marker").unwrap();
  let body = format!(
    r#"
[worktree]
base = "{base}"
path_pattern = "{{type}}-{{issue}}-{{desc}}"
branch_pattern = "{{type}}/#{{issue}}-{{desc}}"

[[bootstrap.copy]]
from = "seed.env"
to = "seed.env"
required = true
"#,
    base = base.path().display(),
  );
  std::fs::write(dir.path().join(".gwm.toml"), body).unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["create", "feat", "8", "bootstrap-off", "--no-bootstrap"])
    .assert()
    .success()
    .stdout(predicate::str::contains("skipped bootstrap"));

  let wt_dir = base.path().join("feat-8-bootstrap-off");
  assert!(wt_dir.exists(), "worktree dir must still be created");
  assert!(
    !wt_dir.join("seed.env").exists(),
    "--no-bootstrap must prevent the copy step from running"
  );
}

#[test]
fn create_rejects_unknown_branch_type() {
  // `BranchSpec::validate` rejects branch types outside the built-in /
  // configured allow-list before any filesystem operation. We must see
  // the failure surface on stderr with the offending value so the user
  // can grep their command history without re-running.
  let (dir, _repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  write_test_config(dir.path(), base.path());

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["create", "blarg", "9", "nope"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("blarg"));

  assert!(
    !base.path().join("blarg-9-nope").exists(),
    "no worktree dir may be created when branch-type validation fails"
  );
}

#[test]
fn create_rejects_non_digit_issue() {
  // Issue must be digits-only — `abc` is rejected by `BranchSpec::new`
  // before any filesystem op. As with the unknown-branch-type test,
  // assert the no-side-effect invariant explicitly: the worktree dir
  // that *would* have been written (`<base>/feat-abc-thing`) must not
  // appear on disk. Otherwise a regression where validation runs
  // post-`worktree::add` could still satisfy a bare `.failure()` check
  // while leaving stray dirs behind.
  let (dir, _repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  write_test_config(dir.path(), base.path());

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["create", "feat", "abc", "thing"])
    .assert()
    .failure();

  assert!(
    !base.path().join("feat-abc-thing").exists(),
    "no worktree dir may be created when issue-number validation fails"
  );
}

#[test]
fn create_refuses_stale_branch_without_reuse_flag() {
  // Issue #99 E2E. A pre-existing local branch of the same name must
  // surface `BranchExists` and refuse to create the worktree dir —
  // protecting the user from silently landing on whatever commit the
  // pre-existing ref points at. The error renders the stale tip's
  // OID and the `--reuse-branch` opt-in so the message is
  // self-explanatory.
  //
  // We pin the branch at the seed commit, THEN advance main by one
  // commit so the branch tip diverges from HEAD — the textbook
  // "stale" scenario from the issue. That divergence is what makes
  // the silent reuse a foot-gun in the first place, and asserting
  // the stale OID appears verbatim in stderr proves the error message
  // is grep-able for the value the user would otherwise be silently
  // attached to.
  let (dir, repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  write_test_config(dir.path(), base.path());

  let seed = repo.head().unwrap().peel_to_commit().unwrap();
  let stale_branch = repo.branch("feat/#99-stale", &seed, false).unwrap();
  let stale_oid = stale_branch.into_reference().target().unwrap().to_string();

  let sig = git2::Signature::now("gwm-test", "gwm@test").unwrap();
  let tree_id = repo.index().unwrap().write_tree().unwrap();
  let tree = repo.find_tree(tree_id).unwrap();
  repo
    .commit(Some("HEAD"), &sig, &sig, "advance main", &tree, &[&seed])
    .unwrap();
  let new_head = repo.head().unwrap().target().unwrap().to_string();
  assert_ne!(new_head, stale_oid, "precondition: HEAD must diverge from stale branch");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_ALLOW_BOOTSTRAP", "1")
    .args(["create", "feat", "99", "stale"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("feat/#99-stale"))
    .stderr(predicate::str::contains("--reuse-branch"))
    .stderr(predicate::str::contains(stale_oid.as_str()));

  assert!(
    !base.path().join("feat-99-stale").exists(),
    "no worktree dir may be created when the branch is refused"
  );
}

#[test]
fn create_reuses_stale_branch_with_flag() {
  // Companion to `create_refuses_stale_branch_without_reuse_flag`: the
  // `--reuse-branch` opt-in restores the legacy attach-to-existing
  // behaviour, and the worktree directory does get created.
  let (dir, repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  write_test_config(dir.path(), base.path());

  let head = repo.head().unwrap().peel_to_commit().unwrap();
  repo.branch("feat/#99-stale", &head, false).unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_ALLOW_BOOTSTRAP", "1")
    .args(["create", "feat", "99", "stale", "--reuse-branch"])
    .assert()
    .success();

  assert!(
    base.path().join("feat-99-stale").exists(),
    "with --reuse-branch the worktree dir must be created against the stale branch"
  );
}

#[test]
fn create_subcommand_outside_git_repo_fails() {
  // The repo-bound contract: outside any git repo the standard
  // `NotInGitRepo` error wins, no worktree is touched. Named with the
  // `_subcommand_` infix to avoid colliding with the historical
  // `create_outside_git_repo_fails` test (line ~327) that — despite the
  // name — actually exercises `gwm list`.
  let dir = tempfile::TempDir::new().unwrap();
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["create", "feat", "1", "x"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

// --- remove -------------------------------------------------------------

#[test]
fn remove_deletes_worktree_dir_and_keeps_branch_by_default() {
  // Without `--delete-branch`, `gwm remove` is the inverse of `gwm
  // create` for the on-disk directory only: the worktree dir disappears
  // (and so does its admin entry under `.git/worktrees/`), but the
  // local branch survives so the user can re-create the worktree from
  // it later. Stdout names the dir for visual confirmation.
  let (dir, repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  write_test_config(dir.path(), base.path());

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_ALLOW_BOOTSTRAP", "1") // issue #95: TOFU bypass for the setup step
    .args(["create", "feat", "10", "remove-me"])
    .assert()
    .success();
  let wt_dir = base.path().join("feat-10-remove-me");
  assert!(wt_dir.exists());

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["remove", "remove-me"])
    .assert()
    .success()
    .stdout(predicate::str::contains("removed"));

  assert!(!wt_dir.exists(), "remove must delete the worktree directory");
  assert!(
    repo.find_branch("feat/#10-remove-me", git2::BranchType::Local).is_ok(),
    "remove without --delete-branch must keep the local branch"
  );
}

#[test]
fn remove_with_delete_branch_drops_branch() {
  // The `--delete-branch` flag drops the local branch in the same
  // command. The stdout breadcrumb names both the worktree dir and the
  // branch so two destructive operations are surfaced explicitly.
  let (dir, repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  write_test_config(dir.path(), base.path());

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_ALLOW_BOOTSTRAP", "1") // issue #95: TOFU bypass for the setup step
    .args(["create", "feat", "11", "drop-branch"])
    .assert()
    .success();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["remove", "drop-branch", "--delete-branch"])
    .assert()
    .success()
    .stdout(predicate::str::contains("branch"))
    .stdout(predicate::str::contains("deleted"));

  assert!(
    repo
      .find_branch("feat/#11-drop-branch", git2::BranchType::Local)
      .is_err(),
    "--delete-branch must remove the local branch ref"
  );
}

#[test]
fn remove_unknown_pattern_fails() {
  // Fuzzy lookup must error loudly when nothing matches — silently
  // doing nothing would mask a user typo and leave them wondering why
  // their worktree is still around.
  let (dir, _repo) = init_repo();
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["remove", "ghost"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not found"));
}

#[test]
fn remove_outside_git_repo_fails() {
  let dir = tempfile::TempDir::new().unwrap();
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["remove", "anything"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

// --- trust ledger (issue #95) -------------------------------------------

#[test]
fn trust_help_lists_list_revoke_show() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["trust", "--help"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("list"))
    .stdout(predicate::str::contains("revoke"))
    .stdout(predicate::str::contains("show"));
}

#[test]
fn trust_list_empty_prints_zero_entries() {
  // Point GWM_TRUST_LEDGER at a freshly-created tempdir so we don't
  // clobber the user's real ~/.config/gwm/trust.toml and so the test
  // is hermetic on CI runners that have an empty $HOME.
  let dir = tempfile::TempDir::new().unwrap();
  let ledger = dir.path().join("trust.toml");
  Command::cargo_bin("gwm")
    .unwrap()
    .env("GWM_TRUST_LEDGER", &ledger)
    .args(["trust", "list"])
    .assert()
    .success()
    .stdout(predicate::str::contains("0 entries in trust ledger"));
}

#[test]
fn trust_show_when_absent_says_so() {
  let dir = tempfile::TempDir::new().unwrap();
  let ledger = dir.path().join("absent.toml");
  Command::cargo_bin("gwm")
    .unwrap()
    .env("GWM_TRUST_LEDGER", &ledger)
    .args(["trust", "show"])
    .assert()
    .success()
    .stdout(predicate::str::contains("file does not exist yet"));
}

#[test]
fn trust_revoke_no_matching_origin_is_a_no_op() {
  let dir = tempfile::TempDir::new().unwrap();
  let ledger = dir.path().join("trust.toml");
  Command::cargo_bin("gwm")
    .unwrap()
    .env("GWM_TRUST_LEDGER", &ledger)
    .args(["trust", "revoke", "git@github.com:foo/bar.git"])
    .assert()
    .success()
    .stdout(predicate::str::contains("0 entries matched"));
}

#[test]
fn trust_list_show_revoke_round_trip() {
  // Seed the ledger manually (we can't easily exercise the prompt
  // path from assert_cmd), then verify list → show → revoke all
  // reflect the same entry.
  use std::fs;
  let dir = tempfile::TempDir::new().unwrap();
  let ledger = dir.path().join("trust.toml");
  fs::write(
    &ledger,
    r#"[[entries]]
origin = "git@github.com:kbrdn1/gwm-cli.git"
config_sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
trusted_at = "2026-05-22T10:00:00Z"
trusted_by = "kylian@laptop"
"#,
  )
  .unwrap();

  // list ----------------------------------------------------------
  Command::cargo_bin("gwm")
    .unwrap()
    .env("GWM_TRUST_LEDGER", &ledger)
    .args(["trust", "list"])
    .assert()
    .success()
    .stdout(predicate::str::contains("git@github.com:kbrdn1/gwm-cli.git"))
    // Short sha (first 12) is what list renders.
    .stdout(predicate::str::contains("deadbeefdead"));

  // show prints the raw toml -------------------------------------
  Command::cargo_bin("gwm")
    .unwrap()
    .env("GWM_TRUST_LEDGER", &ledger)
    .args(["trust", "show"])
    .assert()
    .success()
    .stdout(predicate::str::contains("kylian@laptop"));

  // revoke removes it --------------------------------------------
  Command::cargo_bin("gwm")
    .unwrap()
    .env("GWM_TRUST_LEDGER", &ledger)
    .args(["trust", "revoke", "git@github.com:kbrdn1/gwm-cli.git"])
    .assert()
    .success()
    .stdout(predicate::str::contains("✓ revoked 1"));

  // and list is empty again --------------------------------------
  Command::cargo_bin("gwm")
    .unwrap()
    .env("GWM_TRUST_LEDGER", &ledger)
    .args(["trust", "list"])
    .assert()
    .success()
    .stdout(predicate::str::contains("0 entries"));
}

#[test]
fn allow_bootstrap_flag_is_global_and_documented() {
  // `--allow-bootstrap` is the CI bypass; if it ever stops being a
  // global flag (e.g. accidentally scoped to one subcommand), the
  // `gwm bootstrap --allow-bootstrap` invocation in user scripts
  // silently fails. Pin it down at the help level.
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["--help"]);
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("--allow-bootstrap"))
    .stdout(predicate::str::contains("--deny-bootstrap"));
}

#[test]
fn create_without_trust_in_non_interactive_aborts_cleanly() {
  // Black-box the TOFU gate at the CLI boundary: a fresh repo with
  // a `.gwm.toml` carrying any bootstrap surface, run from a
  // non-tty (assert_cmd's piped stdin), no `--allow-bootstrap`, no
  // ledger entry → must abort with a clear message rather than
  // silently running the bootstrap commands.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"[[bootstrap.command]]
name = "echo"
run  = "echo trapped"
"#,
  )
  .unwrap();

  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_TRUST_LEDGER", &ledger)
    // No `GWM_ALLOW_BOOTSTRAP` env, no flag → must abort.
    .args(["create", "feat", "42", "trapped"])
    .assert()
    .failure()
    .stderr(
      predicate::str::contains("not in the trust ledger").or(predicate::str::contains("stdin is not interactive")),
    );
}

#[test]
fn create_with_allow_bootstrap_flag_bypasses_the_prompt() {
  // The `--allow-bootstrap` escape hatch is what makes scripted /
  // CI usage workable; this asserts the FLAG (not the env var)
  // actually short-circuits the trust gate. The env-var path is
  // covered by other tests that set GWM_ALLOW_BOOTSTRAP=1 — here
  // we exercise the clap-level wiring so a future refactor that
  // accidentally scopes the flag to one subcommand breaks loudly.
  //
  // `--no-bootstrap` is added so we don't actually shell out to
  // anything — the test is about the trust gate, not the bootstrap
  // step itself.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"[[bootstrap.command]]
name = "echo"
run  = "echo would-have-run"
"#,
  )
  .unwrap();

  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_TRUST_LEDGER", &ledger)
    // No env-var bypass — the flag must do the work on its own.
    // `--allow-bootstrap` is a clap `global = true` flag declared on
    // `Cli`, so it MUST appear before the subcommand on the argv.
    .args(["--allow-bootstrap", "create", "feat", "42", "trapped", "--no-bootstrap"])
    .assert()
    .success();

  // The ledger MUST still be empty — Allow mode bypasses without
  // recording, so the next interactive run on the same machine
  // re-prompts. That's the "don't pollute the user's ledger from CI"
  // contract.
  assert!(
    !ledger.exists()
      || std::fs::read_to_string(&ledger).unwrap().contains("entries = []")
      || std::fs::read_to_string(&ledger).unwrap().is_empty()
  );
}

#[test]
fn create_with_gwm_allow_bootstrap_env_bypasses_the_prompt() {
  // Sibling of the previous test: same fixture, but exercise the
  // `GWM_ALLOW_BOOTSTRAP=1` env-var bypass with no flag. This is
  // the CI-runner code path — scripts can't always inject extra
  // args, so the env-var path has to keep working independently.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"[[bootstrap.command]]
name = "echo"
run  = "echo would-have-run"
"#,
  )
  .unwrap();

  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_TRUST_LEDGER", &ledger)
    .env("GWM_ALLOW_BOOTSTRAP", "1")
    .args(["create", "feat", "42", "env-trapped", "--no-bootstrap"])
    .assert()
    .success();

  assert!(
    !ledger.exists()
      || std::fs::read_to_string(&ledger).unwrap().contains("entries = []")
      || std::fs::read_to_string(&ledger).unwrap().is_empty()
  );
}

#[test]
fn create_skips_trust_gate_when_bootstrap_surface_is_empty() {
  // A `.gwm.toml` that declares nothing executable (no copies, no
  // guards, no no_symlinks, no commands) carries no RCE risk —
  // prompting for trust in that case would just train the user to
  // mash `y`. Verify the gate is a silent no-op when the config has
  // only `[worktree]`.
  let (dir, _repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    format!(
      r#"[worktree]
base = "{base}"
path_pattern = "{{type}}-{{issue}}-{{desc}}"
branch_pattern = "{{type}}/#{{issue}}-{{desc}}"
"#,
      base = base.path().display(),
    ),
  )
  .unwrap();

  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");

  // No --allow-bootstrap, no env bypass, no ledger entry, no tty —
  // the gate would normally abort. With an empty bootstrap surface
  // it must short-circuit and let the create proceed.
  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_TRUST_LEDGER", &ledger)
    .args(["create", "feat", "13", "empty-surface"])
    .assert()
    .success();

  // Ledger MUST stay untouched — the skip doesn't record anything.
  assert!(
    !ledger.exists(),
    "empty-surface short-circuit must not write to the ledger"
  );
}

#[test]
fn allow_bootstrap_succeeds_even_when_ledger_is_malformed() {
  // The whole point of `--allow-bootstrap` / `GWM_ALLOW_BOOTSTRAP=1`
  // is to be the unconditional CI escape hatch. A malformed
  // trust.toml on the CI host must NOT make the bypass fail — the
  // Allow check has to short-circuit before the ledger is touched.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[[bootstrap.command]]\nname = \"x\"\nrun = \"true\"\n",
  )
  .unwrap();

  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");
  std::fs::write(&ledger, b"this is not valid toml @@@@").unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_TRUST_LEDGER", &ledger)
    .env("GWM_ALLOW_BOOTSTRAP", "1")
    .args(["create", "feat", "99", "broken-ledger", "--no-bootstrap"])
    .assert()
    .success();
}

#[test]
fn deny_bootstrap_aborts_even_when_trusted() {
  // `--deny-bootstrap` is the forensic mode: even if the ledger
  // says the config is trusted, refuse to run bootstrap. Asserts
  // the precedence resolution in `resolve_trust_mode`.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    "[[bootstrap.command]]\nname = \"x\"\nrun = \"true\"\n",
  )
  .unwrap();

  let ledger_dir = tempfile::TempDir::new().unwrap();
  let ledger = ledger_dir.path().join("trust.toml");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("GWM_TRUST_LEDGER", &ledger)
    .args(["--deny-bootstrap", "bootstrap"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("--deny-bootstrap"));
}

// --- aliases (issue #86) ------------------------------------------------

#[test]
fn aliases_help_lists_list() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["aliases", "--help"]);
  cmd.assert().success().stdout(predicate::str::contains("list"));
}

#[test]
fn aliases_list_prints_built_in_section_outside_repo() {
  // `aliases list` is read-only and must work outside a git repo —
  // built-in clap aliases are static and independent of any repo
  // config. The user fallback (`~/.config/gwm/aliases.toml`) is
  // silently empty when missing.
  let dir = tempfile::TempDir::new().unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    // Isolate the test from the developer's real `~/.config/gwm/aliases.toml`.
    .env("XDG_CONFIG_HOME", dir.path())
    .env("HOME", dir.path())
    .args(["aliases", "list"])
    .assert()
    .success()
    .stdout(predicate::str::contains("built-in:"))
    // Canonical built-in entries from `BUILT_IN_ALIASES`. Assert the full
    // formatted row (`  <name>  → <expansion>` with width-2 padding) rather
    // than a loose `contains("s")` — the latter matched incidental letters
    // in unrelated words (`aliases`, `built-ins`, …) and let the test pass
    // even when the `s → switch` row was missing or malformed.
    .stdout(predicate::str::contains("  s  → switch"))
    .stdout(predicate::str::contains("  cd → path"));
}

#[test]
fn aliases_list_prints_repo_aliases_with_source() {
  // `.gwm.toml` with `[aliases]` ⇒ `aliases list` surfaces them under
  // the `repo (.gwm.toml):` section.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[aliases]
wip = "create feat 0 wip"
ll = "list --format names"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env("XDG_CONFIG_HOME", dir.path())
    .env("HOME", dir.path())
    .args(["aliases", "list"])
    .assert()
    .success()
    .stdout(predicate::str::contains("repo (.gwm.toml)"))
    .stdout(predicate::str::contains("wip"))
    .stdout(predicate::str::contains("create feat 0 wip"))
    .stdout(predicate::str::contains("ll"))
    .stdout(predicate::str::contains("list --format names"));
}

#[test]
fn aliases_list_prints_user_aliases_with_source() {
  // `~/.config/gwm/aliases.toml` (XDG resolved via `$XDG_CONFIG_HOME`)
  // surfaces under the user section.
  let dir = tempfile::TempDir::new().unwrap();
  let user_cfg = dir.path().join("gwm");
  std::fs::create_dir_all(&user_cfg).unwrap();
  std::fs::write(
    user_cfg.join("aliases.toml"),
    r#"
[aliases]
copy = "path"
"#,
  )
  .unwrap();

  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env("XDG_CONFIG_HOME", dir.path())
    .env("HOME", dir.path())
    .args(["aliases", "list"])
    .assert()
    .success()
    .stdout(predicate::str::contains("user"))
    .stdout(predicate::str::contains("aliases.toml"))
    .stdout(predicate::str::contains("copy"))
    .stdout(predicate::str::contains("path"));
}

#[test]
fn aliases_list_repo_overrides_user_for_same_name() {
  // When repo and user both declare the same alias, the repo entry
  // takes precedence at expansion time. `aliases list` still prints
  // BOTH sections so the user can see what's being shadowed.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[aliases]
copy = "path bar"
"#,
  )
  .unwrap();
  let user_cfg = dir.path().join("gwm");
  std::fs::create_dir_all(&user_cfg).unwrap();
  std::fs::write(
    user_cfg.join("aliases.toml"),
    r#"
[aliases]
copy = "path foo"
"#,
  )
  .unwrap();

  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env("XDG_CONFIG_HOME", dir.path())
    .env("HOME", dir.path())
    .args(["aliases", "list"])
    .assert()
    .success()
    .stdout(predicate::str::contains("path bar"))
    .stdout(predicate::str::contains("path foo"));
}

#[test]
fn aliases_list_surfaces_shadow_error_with_alias_name() {
  // An invalid alias in `.gwm.toml` must fail `aliases list` with a
  // message naming the offending entry.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[aliases]
list = "create feat 0 wip"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env("XDG_CONFIG_HOME", dir.path())
    .env("HOME", dir.path())
    .args(["aliases", "list"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("list").and(predicate::str::contains("built-in")));
}

#[test]
fn alias_expansion_runs_built_in_subcommand() {
  // End-to-end expansion: declare `lst = "list --format names"`,
  // invoke `gwm lst` — must behave as `gwm list --format names`.
  // The repo has no worktrees yet so the names output is just empty.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[aliases]
lst = "list --format names"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env("XDG_CONFIG_HOME", dir.path())
    .env("HOME", dir.path())
    .arg("lst")
    .assert()
    .success();
}

#[test]
fn alias_expansion_preserves_trailing_user_args() {
  // `gwm typ` with `typ = "types"` plus a trailing `--help` must
  // surface clap's `types` help (or success), proving the trailing
  // arg made it through the expansion.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[aliases]
typ = "types"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env("XDG_CONFIG_HOME", dir.path())
    .env("HOME", dir.path())
    .args(["typ", "--help"])
    .assert()
    .success()
    .stdout(predicate::str::contains("branch types"));
}

#[test]
fn alias_does_not_shadow_built_in_subcommand_at_runtime() {
  // Even if a hostile `.gwm.toml` somehow tries to alias `types`,
  // the load gate refuses — `gwm types` always reaches the
  // built-in. This is the "defence in depth" pillar of the design.
  let (dir, _repo) = init_repo();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[aliases]
types = "list"
"#,
  )
  .unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(dir.path())
    .env("XDG_CONFIG_HOME", dir.path())
    .env("HOME", dir.path())
    .args(["types"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("types"));
}

// --- argv robustness (issue #86 — Copilot follow-up) --------------------

#[cfg(unix)]
#[test]
fn binary_tolerates_non_utf8_argv() {
  // `std::env::args()` panics if any argv entry is non-UTF-8, which is
  // a regression vs. clap's default `args_os` handling. The binary
  // must NOT abort the process on a perfectly valid OS argv just
  // because someone passed bytes that don't decode as UTF-8.
  //
  // We don't care about the exit code per se — the contract is "no
  // panic, no SIGABRT". clap may legitimately reject the unknown
  // subcommand and exit non-zero (UsageError = 2), which is fine.
  // What we forbid is the process aborting before clap even sees
  // the args.
  use std::ffi::OsString;
  use std::os::unix::ffi::OsStringExt;

  let dir = tempfile::TempDir::new().unwrap();
  let invalid_utf8: OsString = OsString::from_vec(vec![0xff, 0xfe, 0x80]);

  let output = Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("XDG_CONFIG_HOME", dir.path())
    .env("HOME", dir.path())
    .arg(&invalid_utf8)
    .output()
    .expect("binary must launch");

  // Reject SIGABRT / SIGSEGV / SIGILL — anything that indicates the
  // process died on a signal rather than exited normally. On Unix,
  // signal deaths surface as `status.code() == None` from `assert_cmd`.
  assert!(
    output.status.code().is_some(),
    "binary died on a signal (likely a panic abort) when given non-UTF-8 argv; stderr={:?}",
    String::from_utf8_lossy(&output.stderr)
  );

  // Belt-and-suspenders: the stderr should not contain the libstd panic
  // banner for invalid UTF-8 in argv.
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    !stderr.contains("invalid utf-8") && !stderr.contains("panicked at"),
    "binary panicked instead of gracefully handling non-UTF-8 argv: {}",
    stderr
  );
}
