//! Integration tests for `.githooks/pre-commit`.
//!
//! The hook is a POSIX shell script that gates commits on two checks:
//!   1. Env-dependent test pre-validation (stripped-PATH `cargo test`).
//!   2. Local `gwm doctor` with exit-code interpretation
//!      (0 = clean, 1 = advisory warnings, 2 = errors → block).
//!
//! Tests stub `cargo` and `gwm` so gate behaviour can be exercised
//! deterministically without touching the real toolchain.

#![cfg(unix)]

mod common;

use common::init_repo;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn hook_path() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).join(".githooks/pre-commit")
}

fn install_stub(dir: &Path, name: &str, body: &str) {
  let path = dir.join(name);
  fs::write(&path, format!("#!/bin/sh\n{}\n", body)).unwrap();
  fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn stage(repo: &Path, relative_path: &str, content: &str) {
  let full = repo.join(relative_path);
  if let Some(parent) = full.parent() {
    fs::create_dir_all(parent).unwrap();
  }
  fs::write(&full, content).unwrap();
  let status = Command::new("git")
    .args(["add", "--", relative_path])
    .current_dir(repo)
    .status()
    .expect("git add ran");
  assert!(status.success(), "git add failed for {relative_path}");
}

fn run_hook(repo: &Path, stub_dir: Option<&Path>, extra_env: &[(&str, &str)]) -> Output {
  let path = match stub_dir {
    Some(p) => format!("{}:/usr/bin:/bin", p.display()),
    None => "/usr/bin:/bin".into(),
  };
  let mut cmd = Command::new("sh");
  cmd
    .arg(hook_path())
    .current_dir(repo)
    .env_clear()
    .env("PATH", path)
    .env("HOME", repo);
  for (k, v) in extra_env {
    cmd.env(k, v);
  }
  cmd.output().expect("hook ran")
}

// ─── short-circuit ─────────────────────────────────────────────────────────

#[test]
fn passes_silently_when_nothing_staged() {
  let (dir, _repo) = init_repo();
  let out = run_hook(dir.path(), None, &[]);
  assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn passes_when_only_unrelated_files_staged() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), "README.md", "edit\n");
  let out = run_hook(dir.path(), None, &[]);
  assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    !stdout.contains("gate 1") && !stdout.contains("gate 2"),
    "no gate should trigger: {stdout}"
  );
}

// ─── gate 1: env-dependent test pre-validation ────────────────────────────

#[test]
fn gate1_skipped_when_test_file_has_no_ambient_state_refs() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), "tests/sample.rs", "#[test] fn t() { assert_eq!(1, 1); }\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "cargo", "echo CARGO_INVOKED; exit 0");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(out.status.success());
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    !stdout.contains("CARGO_INVOKED"),
    "gate 1 should not invoke cargo without ambient-state refs: {stdout}"
  );
}

#[test]
fn gate1_triggers_on_assert_cmd_reference() {
  let (dir, _repo) = init_repo();
  stage(
    dir.path(),
    "tests/sample.rs",
    "use assert_cmd::Command;\n#[test] fn t() {}\n",
  );
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "cargo", "echo CARGO_INVOKED; exit 0");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(stdout.contains("gate 1"), "gate 1 message missing: {stdout}");
  assert!(stdout.contains("CARGO_INVOKED"), "cargo not invoked: {stdout}");
}

#[test]
fn gate1_triggers_on_std_env_var_reference() {
  let (dir, _repo) = init_repo();
  stage(
    dir.path(),
    "tests/sample.rs",
    "fn t() { let _ = std::env::var(\"PATH\"); }\n",
  );
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "cargo", "exit 0");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(out.status.success());
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    stdout.contains("gate 1"),
    "gate 1 should trigger on std::env::var ref: {stdout}"
  );
}

#[test]
fn gate1_blocks_commit_when_cargo_fails() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), "tests/sample.rs", "use assert_cmd::Command;\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "cargo", "exit 1");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(!out.status.success(), "hook should block on cargo failure");
  let stderr = String::from_utf8_lossy(&out.stderr);
  assert!(
    stderr.contains("blocked") || stderr.contains("FAILED"),
    "stderr should explain the block: {stderr}"
  );
}

// ─── gate 2: local gwm doctor ─────────────────────────────────────────────

#[test]
fn gate2_triggers_on_gwm_toml_change() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), ".gwm.toml", "[bootstrap]\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "gwm", "echo GWM_ARGS:$*; exit 0");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(stdout.contains("gate 2"), "gate 2 message missing: {stdout}");
  assert!(
    stdout.contains("GWM_ARGS:doctor"),
    "doctor subcommand not invoked: {stdout}"
  );
}

#[test]
fn gate2_triggers_on_src_bootstrap_change() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), "src/bootstrap.rs", "// edit\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "gwm", "exit 0");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(out.status.success());
  assert!(
    String::from_utf8_lossy(&out.stdout).contains("gate 2"),
    "gate 2 should trigger on src/bootstrap.rs"
  );
}

#[test]
fn gate2_triggers_on_src_doctor_change() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), "src/doctor.rs", "// edit\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "gwm", "exit 0");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(out.status.success());
  assert!(
    String::from_utf8_lossy(&out.stdout).contains("gate 2"),
    "gate 2 should trigger on src/doctor.rs"
  );
}

#[test]
fn gate2_triggers_on_examples_gwm_toml_example() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), "examples/gwm.toml.example", "[bootstrap]\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "gwm", "exit 0");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(out.status.success());
  assert!(
    String::from_utf8_lossy(&out.stdout).contains("gate 2"),
    "gate 2 should trigger on examples/gwm.toml.example"
  );
}

#[test]
fn gate2_triggers_on_tests_bootstrap_file() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), "tests/bootstrap_when_tests.rs", "// edit\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "gwm", "exit 0");
  install_stub(stub.path(), "cargo", "exit 0");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(out.status.success());
  assert!(
    String::from_utf8_lossy(&out.stdout).contains("gate 2"),
    "gate 2 should trigger on tests/bootstrap*.rs"
  );
}

#[test]
fn gate2_triggers_on_tests_doctor_file() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), "tests/doctor_tests.rs", "// edit\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "gwm", "exit 0");
  install_stub(stub.path(), "cargo", "exit 0");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(out.status.success());
  assert!(
    String::from_utf8_lossy(&out.stdout).contains("gate 2"),
    "gate 2 should trigger on tests/doctor*.rs"
  );
}

#[test]
fn gate2_skips_with_message_when_gwm_absent() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), ".gwm.toml", "[bootstrap]\n");
  let out = run_hook(dir.path(), None, &[]);
  assert!(out.status.success(), "hook should proceed when gwm absent");
  let stderr = String::from_utf8_lossy(&out.stderr);
  assert!(
    stderr.contains("gwm not in PATH"),
    "stderr should mention gwm absence: {stderr}"
  );
}

#[test]
fn gate2_advisory_when_doctor_exits_1() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), ".gwm.toml", "[bootstrap]\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "gwm", "exit 1");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(
    out.status.success(),
    "warnings (exit 1) should not block commit; stderr: {}",
    String::from_utf8_lossy(&out.stderr)
  );
  let stdout = String::from_utf8_lossy(&out.stdout);
  assert!(
    stdout.contains("advisory") || stdout.contains("warnings"),
    "advisory message missing: {stdout}"
  );
}

#[test]
fn gate2_blocks_when_doctor_exits_2() {
  let (dir, _repo) = init_repo();
  stage(dir.path(), ".gwm.toml", "[bootstrap]\n");
  let stub = tempfile::tempdir().unwrap();
  install_stub(stub.path(), "gwm", "exit 2");
  let out = run_hook(dir.path(), Some(stub.path()), &[]);
  assert!(!out.status.success(), "exit 2 must block commit");
  let stderr = String::from_utf8_lossy(&out.stderr);
  assert!(
    stderr.contains("ERRORS") || stderr.contains("blocked"),
    "stderr should explain the block: {stderr}"
  );
}
