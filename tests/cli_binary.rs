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
    .stdout(predicate::str::contains("  tmux "))
    .stdout(predicate::str::contains("  zellij "))
    .stdout(predicate::str::contains("  doctor "))
    .stdout(predicate::str::contains("  link "))
    .stdout(predicate::str::contains("  unlink "))
    .stdout(predicate::str::contains("  open "))
    .stdout(predicate::str::contains("  status "))
    // Issue #81: declarative GitHub labels.
    .stdout(predicate::str::contains("  labels "));
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
