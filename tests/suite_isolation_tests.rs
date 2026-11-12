//! The suite never touches the contributor's own gwm state (issue #724).
//!
//! `cargo test` used to write the real undo journal: about fifteen `gwm
//! remove` runs in `cli_binary.rs` spawned the binary without
//! `GWM_HISTORY_FILE`, so every local run appended entries for temporary
//! repos and, at the cap, evicted real ones. The tests that created a worktree
//! without pinning `[worktree] base` left it under `~/cc-worktree`, or under
//! whatever base the contributor's global config set.
//!
//! The fix is one builder, `common::gwm()`, that points every user-level file
//! gwm reads or writes at a per-test scratch directory. This file pins what the
//! builder sets, and guards that no test spawns the binary any other way.

mod common;

use common::{gwm, gwm_std, init_repo, state_dir};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A source file with CRLF normalised, so a Windows checkout reads the same
/// lines as the other two runners.
fn read(path: &Path) -> String {
  std::fs::read_to_string(path)
    .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
    .replace("\r\n", "\n")
}

// ---------------------------------------------------------------------------
// What the builder isolates.
// ---------------------------------------------------------------------------

#[test]
fn a_removal_run_by_the_suite_lands_in_the_scratch_journal() {
  let (dir, _repo) = init_repo();
  let base = tempfile::TempDir::new().unwrap();
  std::fs::write(
    dir.path().join(".gwm.toml"),
    format!(
      "[worktree]\nbase = \"{}\"\n",
      base.path().display().to_string().replace('\\', "\\\\")
    ),
  )
  .unwrap();

  gwm()
    .current_dir(dir.path())
    .args(["create", "feat", "1", "journal-probe", "--no-bootstrap"])
    .assert()
    .success();
  gwm()
    .current_dir(dir.path())
    .args(["remove", "journal-probe"])
    .assert()
    .success();

  let journal = state_dir().join("history.toml");
  let text = std::fs::read_to_string(&journal).unwrap_or_default();
  assert!(
    text.contains("feat-1-journal-probe"),
    "the removal must be journaled in the test's scratch journal {}, which holds {text:?}. \
     Anywhere else is the contributor's own undo journal",
    journal.display()
  );
}

/// `{home}` is resolved through `dirs::home_dir()`, which reads `$HOME` on
/// unix and ignores it on Windows: there, only a pinned `[worktree] base`
/// keeps a test's worktree out of the real home.
#[cfg(unix)]
#[test]
fn a_worktree_created_without_a_base_lands_under_the_scratch_home() {
  let (dir, _repo) = init_repo();
  let repo_name = dir.path().file_name().unwrap().to_owned();

  gwm()
    .current_dir(dir.path())
    .args(["create", "feat", "2", "base-probe", "--no-bootstrap"])
    .assert()
    .success();

  let expected = state_dir()
    .join("cc-worktree")
    .join(&repo_name)
    .join("feat-2-base-probe");
  assert!(
    expected.is_dir(),
    "with no `[worktree] base` anywhere, the default `{{home}}/cc-worktree/{{repo}}` must \
     resolve under the test's scratch home, at {}",
    expected.display()
  );
}

/// The variables the consumer docs tell a third-party test suite to set, read
/// off the table under "Isolating gwm in a consumer's tests": `(name, value)`,
/// where the value is the doc's placeholder (`<file>`, `<dir>`) or literal.
fn documented_isolation_vars() -> Vec<(String, String)> {
  let path = repo_root().join("docs/5.integrations/4.daemon-consumers.md");
  let text = read(&path);
  let section = text
    .split("\n## ")
    .find(|s| s.starts_with("Isolating gwm in a consumer's tests"))
    .unwrap_or_else(|| panic!("{} must keep its isolation section", path.display()));
  section
    .lines()
    .filter_map(|line| line.strip_prefix("| `"))
    .filter_map(|rest| rest.split('`').next())
    .filter_map(|cell| cell.split_once('='))
    .map(|(name, value)| (name.to_string(), value.to_string()))
    .collect()
}

#[test]
fn the_builder_sets_every_variable_the_consumer_docs_prescribe() {
  let vars = documented_isolation_vars();
  assert!(
    vars.iter().any(|(name, _)| name == "GWM_HISTORY_FILE"),
    "the isolation table must still name `GWM_HISTORY_FILE`, or this reads nothing: {vars:?}"
  );

  let cmd = gwm_std();
  let state = state_dir();
  for (name, documented) in &vars {
    let set = cmd
      .get_envs()
      .find(|(k, _)| *k == name.as_str())
      .and_then(|(_, v)| v)
      .unwrap_or_else(|| panic!("`common::gwm()` must set `{name}`, which the consumer docs prescribe to isolate gwm"));
    if documented.starts_with('<') {
      assert!(
        Path::new(set).starts_with(&state),
        "`{name}` must point inside the test's scratch directory {}, got {}",
        state.display(),
        Path::new(set).display()
      );
    } else {
      assert_eq!(set, documented.as_str(), "`{name}` must be set to the documented value");
    }
  }
}

// ---------------------------------------------------------------------------
// Nothing spawns the binary around the builder.
// ---------------------------------------------------------------------------

/// The spellings that reach a `gwm` binary: assert_cmd's cargo lookup in all
/// its forms (method, function, macros), cargo's compile-time path to the
/// built binary, and a bare `gwm` resolved off `PATH`, which is the
/// contributor's installed one. Assembled at run time so this file does not
/// name the binary itself.
///
/// Not covered: a shell snippet that runs `gwm` off `PATH`
/// (`sh -c "gwm remove …"`), or a program name held in a variable.
fn needles() -> [String; 3] {
  [
    concat!("cargo", "_bin").to_string(),
    concat!("CARGO_BIN", "_EXE_").to_string(),
    format!("Command::new({:?})", "gwm"),
  ]
}

/// Whether `source` reaches the binary outside a comment. Whitespace is
/// dropped first, so `Command :: new ( … )` reads like its compact form.
fn names_the_binary(source: &str) -> bool {
  let needles = needles();
  source
    .lines()
    .filter(|l| !l.trim_start().starts_with("//"))
    .map(|l| l.split_whitespace().collect::<String>())
    .any(|l| needles.iter().any(|n| l.contains(n.as_str())))
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return;
  };
  for entry in entries {
    let path = entry.unwrap().path();
    if path.is_dir() {
      rs_files(&path, out);
    } else if path.extension().is_some_and(|e| e == "rs") {
      out.push(path);
    }
  }
}

#[test]
fn only_the_builder_spawns_the_binary() {
  let root = repo_root();
  let mut files = Vec::new();
  for dir in ["tests", "benches"] {
    rs_files(&root.join(dir), &mut files);
  }
  let mut naming: Vec<String> = files
    .iter()
    .filter(|f| names_the_binary(&read(f)))
    .map(|f| {
      let rel = f.strip_prefix(&root).unwrap();
      rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
    })
    .collect();
  naming.sort();
  assert_eq!(
    naming,
    ["tests/common/mod.rs"],
    "only `tests/common/mod.rs` may reach the gwm binary: spawn it with `common::gwm()`, which \
     points the journal, the trust ledger, the global config and `$HOME` at a scratch \
     directory. Anything else writes the contributor's real undo journal (#724). If \
     `tests/common/mod.rs` is missing from the list, the builder no longer spawns the binary \
     and this guard reads nothing"
  );
}

#[test]
fn the_routing_guard_can_actually_fire() {
  let [lookup, exe, path] = needles();
  for bypass in [
    format!("let c = Command::{lookup}(\"gwm\").unwrap();"),
    format!("let c = assert_cmd::{lookup}!(\"gwm\");"),
    format!("let c = {lookup}_cmd!(\"gwm\");"),
    format!("let c = std::process::Command::new(env!(\"{exe}gwm\"));"),
    format!("let c = std::process::{path};"),
    format!("let c = std::process::Command :: new ( {:?} );", "gwm"),
  ] {
    assert!(names_the_binary(&bypass), "the guard must flag {bypass:?}");
  }
  for clean in [
    "let c = common::gwm();".to_string(),
    format!("  // Command::{lookup}(\"gwm\") used to live here"),
    "let c = std::process::Command::new(\"git\");".to_string(),
  ] {
    assert!(!names_the_binary(&clean), "the guard must not flag {clean:?}");
  }
}
