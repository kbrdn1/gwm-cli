//! End-to-end coverage for `gwm config` (issue #89).

mod common;

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

use common::{init_repo, paths_equal};

#[test]
fn config_help_lists_git_config_style_actions() {
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.arg("config").arg("--help");
  cmd
    .assert()
    .success()
    .stdout(predicate::str::contains("get"))
    .stdout(predicate::str::contains("set"))
    .stdout(predicate::str::contains("unset"))
    .stdout(predicate::str::contains("list"))
    .stdout(predicate::str::contains("validate"))
    .stdout(predicate::str::contains("path"))
    .stdout(predicate::str::contains("edit"));
}

#[test]
fn config_get_reads_dot_path_values_and_defaults() {
  let (dir, _repo) = init_repo();
  fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[worktree]
base = "/tmp/gwm-worktrees"

[tui]
confirm_countdown_secs = 4

[[labels]]
name = "bug"
color = "d73a4a"
"#,
  )
  .unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "get", "worktree.base"])
    .assert()
    .success()
    .stdout("/tmp/gwm-worktrees\n");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "get", "tui.confirm_countdown_secs"])
    .assert()
    .success()
    .stdout("4\n");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "get", "labels[0].name"])
    .assert()
    .success()
    .stdout("bug\n");
}

#[test]
fn config_set_preserves_comments_and_validates_through_runtime_config() {
  let (dir, _repo) = init_repo();
  let path = dir.path().join(".gwm.toml");
  fs::write(
    &path,
    r#"
# repo-level worktree config
[worktree]
# keep this comment attached to base
base = "/tmp/old"
"#,
  )
  .unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "set", "tui.confirm_countdown_secs", "5"])
    .assert()
    .success()
    .stdout(predicate::str::contains("tui.confirm_countdown_secs = 5"));

  let edited = fs::read_to_string(&path).unwrap();
  assert!(
    edited.contains("# keep this comment attached to base"),
    "toml_edit round-trip must preserve existing comments:\n{}",
    edited
  );

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "get", "tui.confirm_countdown_secs"])
    .assert()
    .success()
    .stdout("5\n");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .arg("types")
    .assert()
    .success();
}

#[test]
fn config_set_supports_array_index_and_append_paths() {
  let (dir, _repo) = init_repo();
  fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[[labels]]
name = "bug"
"#,
  )
  .unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "set", "labels[0].color", "d73a4a"])
    .assert()
    .success();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "set", "labels[+].name", "enhancement"])
    .assert()
    .success()
    .stdout(predicate::str::contains("labels[1].name = \"enhancement\""));

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "get", "labels[0].color"])
    .assert()
    .success()
    .stdout("d73a4a\n");

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "get", "labels[1].name"])
    .assert()
    .success()
    .stdout("enhancement\n");
}

#[test]
fn config_set_accepts_git_config_style_key_value_form() {
  let (dir, _repo) = init_repo();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "set", "labels[+].name=bug"])
    .assert()
    .success()
    .stdout(predicate::str::contains("labels[0].name = \"bug\""));

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "get", "labels[0].name"])
    .assert()
    .success()
    .stdout("bug\n");
}

#[test]
fn config_unset_removes_explicit_value_so_defaults_apply() {
  let (dir, _repo) = init_repo();
  fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[tui]
confirm_countdown_secs = 5
"#,
  )
  .unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "unset", "tui.confirm_countdown_secs"])
    .assert()
    .success()
    .stdout(predicate::str::contains("unset tui.confirm_countdown_secs"));

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "get", "tui.confirm_countdown_secs"])
    .assert()
    .success()
    .stdout("3\n");
}

#[test]
fn config_list_prints_all_values_and_filters_by_prefix() {
  let (dir, _repo) = init_repo();
  fs::write(
    dir.path().join(".gwm.toml"),
    r#"
[worktree]
base = "/tmp/gwm"

[review]
tool = "lumen"
"#,
  )
  .unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "list", "--prefix", "worktree"])
    .assert()
    .success()
    .stdout(predicate::str::contains("worktree.base = \"/tmp/gwm\""))
    .stdout(predicate::str::contains("review.tool").not());
}

#[test]
fn config_path_and_validate_report_resolved_config_file() {
  let (dir, _repo) = init_repo();
  let config_path = fs::canonicalize(dir.path()).unwrap().join(".gwm.toml");
  fs::write(&config_path, "[tui]\nconfirm_countdown_secs = 2\n").unwrap();

  let output = Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "path"])
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
  let printed = String::from_utf8(output).unwrap();
  assert!(
    paths_equal(Path::new(printed.trim()), &config_path),
    "printed path {:?} should resolve to {}",
    printed.trim(),
    config_path.display()
  );

  let output = Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "validate"])
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
  let printed = String::from_utf8(output).unwrap();
  let Some(path_text) = printed.trim().strip_suffix(" is valid") else {
    panic!("unexpected validate output: {}", printed);
  };
  assert!(
    paths_equal(Path::new(path_text), &config_path),
    "validated path {:?} should resolve to {}",
    path_text,
    config_path.display()
  );
}

#[test]
fn config_validate_surfaces_parse_location() {
  let (dir, _repo) = init_repo();
  fs::write(dir.path().join(".gwm.toml"), "[tui]\nconfirm_countdown_secs = [\n").unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "validate"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("error at line"))
    .stderr(predicate::str::contains(".gwm.toml"));
}

#[test]
fn config_validate_rejects_unknown_schema_keys_with_hint() {
  let (dir, _repo) = init_repo();
  fs::write(dir.path().join(".gwm.toml"), "[review]\nfullscreem = true\n").unwrap();

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .args(["config", "validate"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("fullscreem"))
    .stderr(predicate::str::contains("did you mean 'fullscreen'"));
}

#[test]
fn config_edit_opens_editor_on_config_path() {
  let (dir, _repo) = init_repo();
  fs::write(dir.path().join(".gwm.toml"), "[tui]\nconfirm_countdown_secs = 3\n").unwrap();
  let editor = write_editor_script(dir.path());

  Command::cargo_bin("gwm")
    .unwrap()
    .current_dir(dir.path())
    .env("EDITOR", &editor)
    .args(["config", "edit"])
    .assert()
    .success();

  let marker = fs::read_to_string(dir.path().join("editor-target.txt")).unwrap();
  assert!(
    paths_equal(
      Path::new(marker.trim()),
      &fs::canonicalize(dir.path()).unwrap().join(".gwm.toml")
    ),
    "editor received {:?}",
    marker.trim()
  );
}

fn write_editor_script(root: &Path) -> std::path::PathBuf {
  #[cfg(unix)]
  {
    write_unix_editor_script(root)
  }
  #[cfg(windows)]
  {
    write_windows_editor_script(root)
  }
}

#[cfg(unix)]
fn write_unix_editor_script(root: &Path) -> std::path::PathBuf {
  let script = root.join("fake-editor.sh");
  fs::write(
    &script,
    format!(
      "#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n",
      root.join("editor-target.txt").display()
    ),
  )
  .unwrap();
  let mut perms = fs::metadata(&script).unwrap().permissions();
  use std::os::unix::fs::PermissionsExt;
  perms.set_mode(0o755);
  fs::set_permissions(&script, perms).unwrap();
  script
}

#[cfg(windows)]
fn write_windows_editor_script(root: &Path) -> std::path::PathBuf {
  let script = root.join("fake-editor.cmd");
  fs::write(
    &script,
    format!(
      "@echo off\r\necho %~1> \"{}\"\r\nexit /b 0\r\n",
      root.join("editor-target.txt").display()
    ),
  )
  .unwrap();
  script
}
