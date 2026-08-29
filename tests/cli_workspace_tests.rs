//! End-to-end tests for workspace mode on the CLI (issue #36): the
//! `--workspace <dir>` global flag and the merged `gwm list` table.

use assert_cmd::Command;
use git2::{Repository, Signature};
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Init a git repo at `path` (created if missing) on `main` with one commit.
fn init_repo_at(path: &Path) {
  fs::create_dir_all(path).unwrap();
  let repo = Repository::init(path).unwrap();
  repo.set_head("refs/heads/main").ok();
  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  let tree_id = {
    let mut index = repo.index().unwrap();
    index.write_tree().unwrap()
  };
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
}

/// A workspace root with two child repos plus a non-repo dir to ignore.
fn workspace_root() -> TempDir {
  let root = TempDir::new().unwrap();
  init_repo_at(&root.path().join("alpha"));
  init_repo_at(&root.path().join("beta"));
  fs::create_dir_all(root.path().join("notes")).unwrap();
  root
}

#[test]
fn list_workspace_prints_repo_column_and_every_repo() {
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .args(["list", "--workspace"])
    .arg(root.path())
    .assert()
    .success()
    // The merged table leads with a REPO column ahead of NAME/BRANCH/STATUS.
    .stdout(predicate::str::contains("REPO"))
    .stdout(predicate::str::contains("NAME"))
    .stdout(predicate::str::contains("BRANCH"))
    .stdout(predicate::str::contains("STATUS"))
    // Both child repos appear; the non-repo `notes/` dir does not.
    .stdout(predicate::str::contains("alpha"))
    .stdout(predicate::str::contains("beta"))
    .stdout(predicate::str::contains("notes").not());
}

#[test]
fn workspace_flag_is_global_before_the_subcommand() {
  // `gwm --workspace <root> list` parses identically to `gwm list
  // --workspace <root>` because the flag is `global = true`.
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .arg("--workspace")
    .arg(root.path())
    .arg("list")
    .assert()
    .success()
    .stdout(predicate::str::contains("alpha"))
    .stdout(predicate::str::contains("beta"));
}

#[test]
fn list_workspace_missing_root_fails() {
  let root = TempDir::new().unwrap();
  let missing = root.path().join("nope");
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd.args(["list", "--workspace"]).arg(&missing).assert().failure();
}

#[test]
fn list_workspace_empty_root_reports_no_repos() {
  let root = TempDir::new().unwrap();
  fs::create_dir_all(root.path().join("plain")).unwrap();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .args(["list", "--workspace"])
    .arg(root.path())
    .assert()
    .failure()
    .stderr(predicate::str::contains("no git repos"));
}

/// Minimal `.gwm.toml` pinning the worktree base into `base` with no
/// bootstrap commands, so `create --no-bootstrap` never hits the trust prompt.
fn write_min_config(repo_root: &Path, base: &Path) {
  let body = format!(
    "[worktree]\nbase = \"{}\"\npath_pattern = \"{{type}}-{{issue}}-{{desc}}\"\nbranch_pattern = \"{{type}}/#{{issue}}-{{desc}}\"\n",
    base.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
  );
  fs::write(repo_root.join(".gwm.toml"), body).unwrap();
}

#[test]
fn create_in_workspace_without_repo_flag_fails() {
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .arg("--workspace")
    .arg(root.path())
    .args(["create", "feat", "42", "demo", "--no-bootstrap"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("--repo"));
}

#[test]
fn create_in_workspace_targets_the_named_repo() {
  let root = workspace_root();
  let base = TempDir::new().unwrap();
  write_min_config(&root.path().join("alpha"), base.path());

  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .arg("--workspace")
    .arg(root.path())
    .args(["create", "feat", "42", "demo", "--repo", "alpha", "--no-bootstrap"])
    .assert()
    .success()
    .stdout(predicate::str::contains("feat/#42-demo"));

  // The branch lands in alpha, not beta.
  let alpha = Repository::open(root.path().join("alpha")).unwrap();
  assert!(
    alpha.find_branch("feat/#42-demo", git2::BranchType::Local).is_ok(),
    "branch must be created in the alpha repo"
  );
  let beta = Repository::open(root.path().join("beta")).unwrap();
  assert!(
    beta.find_branch("feat/#42-demo", git2::BranchType::Local).is_err(),
    "branch must NOT leak into the beta repo"
  );
}

/// Fake `gh` answering `gh issue view <n> …` with one canned issue, written
/// to a file the script cats back so an arbitrary title survives `cmd.exe`.
/// Mirrors the helper in `cli_binary.rs`; duplicated rather than shared
/// because `tests/common` is compiled into every integration target and this
/// one is only needed by two of them.
fn write_issue_gh(root: &Path, issue_json: &str) -> std::path::PathBuf {
  let payload = root.join("issue.json");
  fs::write(&payload, issue_json).unwrap();
  #[cfg(unix)]
  {
    let script = root.join("gh");
    fs::write(
      &script,
      format!(
        "#!/bin/sh\nif [ \"$1\" = \"issue\" ] && [ \"$2\" = \"view\" ]; then\n  cat \"{}\"\nfi\n",
        payload.display()
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
  {
    let script = root.join("gh.cmd");
    fs::write(
      &script,
      format!(
        "@echo off\r\nif \"%~1\"==\"issue\" if \"%~2\"==\"view\" type \"{}\"\r\n",
        payload.display()
      ),
    )
    .unwrap();
    script
  }
}

fn prepend_path(dir: &Path) -> String {
  let old = std::env::var_os("PATH").unwrap_or_default();
  let mut paths = vec![dir.to_path_buf()];
  paths.extend(std::env::split_paths(&old));
  std::env::join_paths(paths).unwrap().to_string_lossy().into_owned()
}

#[test]
fn create_from_issue_in_workspace_targets_the_named_repo() {
  // `--issue <N>` (issue #617) reads the existing-worktree link and resolves
  // the forge from the repo `cmd_create` discovered, so both have to be the
  // child `--repo` named rather than whatever the current directory is. The
  // origin lives on alpha alone: if the derivation looked anywhere else it
  // would have no slug to query.
  let root = workspace_root();
  let base = TempDir::new().unwrap();
  let alpha_root = root.path().join("alpha");
  write_min_config(&alpha_root, base.path());
  fs::write(
    alpha_root.join(".gwm.toml"),
    format!(
      "{}\n[issue_template.by_type]\nfeat = {{ title_prefix = \"[Feature]: \", labels = [\"feature\"] }}\n",
      fs::read_to_string(alpha_root.join(".gwm.toml")).unwrap()
    ),
  )
  .unwrap();
  Repository::open(&alpha_root)
    .unwrap()
    .remote("origin", "https://github.com/kbrdn1/gwm-cli.git")
    .unwrap();

  let fake_bin = TempDir::new().unwrap();
  let fake_gh = write_issue_gh(
    fake_bin.path(),
    r#"{"number":594,"title":"[Feature]: modal layout","state":"OPEN","url":"https://github.com/kbrdn1/gwm-cli/issues/594","labels":[{"name":"feature"}],"updatedAt":"2026-08-29T00:00:00Z"}"#,
  );

  Command::cargo_bin("gwm")
    .unwrap()
    .arg("--workspace")
    .arg(root.path())
    .env("GWM_GH", &fake_gh)
    .env("PATH", prepend_path(fake_bin.path()))
    .args(["create", "--issue", "594", "--repo", "alpha", "--no-bootstrap"])
    .assert()
    .success()
    .stdout(predicate::str::contains("feat/#594-modal-layout"));

  let alpha = Repository::open(&alpha_root).unwrap();
  assert!(
    alpha
      .find_branch("feat/#594-modal-layout", git2::BranchType::Local)
      .is_ok(),
    "the derived branch must land in the alpha repo"
  );
  let beta = Repository::open(root.path().join("beta")).unwrap();
  assert!(
    beta
      .find_branch("feat/#594-modal-layout", git2::BranchType::Local)
      .is_err(),
    "the derived branch must NOT leak into the beta repo"
  );
}

#[test]
fn create_in_workspace_with_unknown_repo_lists_available() {
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .arg("--workspace")
    .arg(root.path())
    .args(["create", "feat", "42", "demo", "--repo", "ghost", "--no-bootstrap"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("ghost"))
    .stderr(predicate::str::contains("alpha"));
}

#[test]
fn workspace_flag_rejected_on_unsupported_subcommand() {
  // `--workspace` is global, so clap accepts it on `remove`, but `run` only
  // implements it for `list`/`create`/TUI. It must be rejected rather than
  // silently acting on the current repo — a wrong-target footgun (#303 P2).
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .arg("--workspace")
    .arg(root.path())
    .args(["remove", "foo"])
    .assert()
    .failure()
    .stderr(predicate::str::contains("only supported with"));
}

#[test]
fn bare_gwm_in_workspace_root_declines_autodetect_without_a_tty() {
  // assert_cmd pipes stdin (not a tty), so the auto-detect prompt must decline
  // silently and fall through to single-repo discovery — which then fails with
  // NotInGitRepo because the workspace root is not itself a repo. This proves
  // bare `gwm` never blocks on an unanswerable prompt in a pipe / CI.
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .current_dir(root.path())
    .assert()
    .failure()
    .stderr(predicate::str::contains("not inside a git repository"));
}

#[test]
fn list_workspace_names_format_lists_worktrees_per_repo() {
  // `--format names` in workspace mode qualifies each worktree with its repo
  // (`<repo>/<name>`) so a shell-completion candidate is unambiguous.
  let root = workspace_root();
  let mut cmd = Command::cargo_bin("gwm").unwrap();
  cmd
    .args(["list", "--workspace"])
    .arg(root.path())
    .args(["--format", "names"])
    .assert()
    .success()
    .stdout(predicate::str::contains("alpha/alpha"))
    .stdout(predicate::str::contains("beta/beta"));
}

#[test]
fn workspace_json_reads_each_note_from_its_own_repo(/* issue #515 */) {
  // A note lives in the `.git` of the repo that owns the row, so the
  // workspace listing is the only surface that has to open a handle PER ROW
  // (the same per-row open the agent pins need). Two repos with a note each,
  // both on a branch called `main`: read through one shared handle and both
  // rows would carry the same text, which is the bug this pins.
  let root = workspace_root();
  for (repo, body) in [("alpha", "alpha's own note\n"), ("beta", "beta's own note\n")] {
    let notes = root.path().join(repo).join(".git/gwm/notes");
    fs::create_dir_all(&notes).unwrap();
    fs::write(notes.join("main.md"), body).unwrap();
  }

  let out = Command::cargo_bin("gwm")
    .unwrap()
    .args(["list", "--format=json", "--workspace"])
    .arg(root.path())
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
  let rows: serde_json::Value = serde_json::from_slice(&out).unwrap();
  let note_of = |name: &str| {
    rows
      .as_array()
      .unwrap()
      .iter()
      .find(|r| r["repo"] == name)
      .unwrap_or_else(|| panic!("no row for {name} in {rows}"))["note"]
      .as_str()
      .map(str::to_string)
  };

  assert_eq!(note_of("alpha").as_deref(), Some("alpha's own note\n"));
  assert_eq!(note_of("beta").as_deref(), Some("beta's own note\n"));
}

/// A containerised `[exec.profiles.ci]` whose `runtime` is `echo`, so the
/// "container" run prints the argv gwm built instead of needing a daemon.
/// `runtime` is explicit, which the resolver honours without a `PATH` probe.
#[cfg(unix)]
fn container_profile_toml() -> &'static str {
  "[exec.profiles.ci]\ncommand = [\"marker\"]\n\n[exec.profiles.ci.container]\nimage = \"the-image\"\nruntime = \"echo\"\n"
}

#[cfg(unix)]
#[test]
fn exec_workspace_mounts_each_repos_own_gitdir() {
  // Issue #421 through the whole CLI path, workspace included: every repo
  // resolves its own plan, so a worktree of `beta` must be handed `beta`'s
  // gitdir. Crossing them would be worse than the bug this feature fixes —
  // git would answer, against the wrong repository.
  let root = workspace_root();
  let worktrees = TempDir::new().unwrap();
  for name in ["alpha", "beta"] {
    let repo_path = root.path().join(name);
    fs::write(repo_path.join(".gwm.toml"), container_profile_toml()).unwrap();
    let repo = Repository::open(&repo_path).unwrap();
    repo
      .worktree(&format!("{name}-wt"), &worktrees.path().join(name), None)
      .unwrap();
  }

  let mut cmd = Command::cargo_bin("gwm").unwrap();
  let out = cmd
    .args(["exec", "--workspace"])
    .arg(root.path())
    .args(["--profile", "ci"])
    .assert()
    .success();
  let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();

  // `echo` printed the argv, one line per worktree, under its repo header.
  for name in ["alpha", "beta"] {
    let other = if name == "alpha" { "beta" } else { "alpha" };
    let line = stdout
      .lines()
      .find(|l| l.starts_with("run --rm") && l.contains(&format!("/{name}/.git")))
      .unwrap_or_else(|| panic!("no run line carrying {name}'s gitdir in:\n{stdout}"));
    assert!(
      !line.contains(&format!("/{other}/.git")),
      "{name}'s run must not mount {other}'s gitdir: {line}"
    );
    assert!(
      line.ends_with("the-image marker"),
      "the image and command close it: {line}"
    );
  }
  // And the header names the run so a containerised fan-out is never silent.
  assert!(
    stdout.contains("[echo the-image]"),
    "the per-worktree header announces the container: {stdout}"
  );
}

#[cfg(unix)]
#[test]
fn exec_refuses_an_unmountable_worktree_before_running_any_other() {
  // Resolution is upfront by contract (#326): a worktree whose path cannot be
  // expressed as a `-v source:destination` mount must fail the whole fan-out,
  // not after an earlier worktree already ran. The marker file is the proof —
  // if the first worktree ran, it exists.
  let root = workspace_root();
  let worktrees = TempDir::new().unwrap();
  let repo_path = root.path().join("alpha");
  fs::write(repo_path.join(".gwm.toml"), container_profile_toml()).unwrap();
  let repo = Repository::open(&repo_path).unwrap();
  // `aaa` sorts before `zz:z`, so it is the one that would run first.
  repo.worktree("aaa", &worktrees.path().join("aaa"), None).unwrap();
  repo.worktree("zzz", &worktrees.path().join("zz:z"), None).unwrap();

  let mut cmd = Command::cargo_bin("gwm").unwrap();
  let out = cmd
    .current_dir(&repo_path)
    .args(["exec", "--profile", "ci"])
    .assert()
    .failure();
  let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
  assert!(
    stderr.contains("zz:z") || stderr.contains(':'),
    "the error names the path it cannot mount:\n{stderr}"
  );
  let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(
    !stdout.contains("run --rm"),
    "no worktree ran before the refusal:\n{stdout}"
  );
}

#[cfg(unix)]
#[test]
fn exec_workspace_refuses_an_unmountable_worktree_before_any_repo_runs() {
  // Upfront resolution is a WORKSPACE-wide contract (#326), not a per-repo
  // one: a worktree of the last repo that cannot be expressed as a container
  // mount must surface before the first repo has run its command.
  let root = workspace_root();
  let worktrees = TempDir::new().unwrap();
  for name in ["alpha", "beta"] {
    let repo_path = root.path().join(name);
    fs::write(repo_path.join(".gwm.toml"), container_profile_toml()).unwrap();
    let repo = Repository::open(&repo_path).unwrap();
    // `beta` (second in discovery order) is the one holding the bad path.
    let dir = if name == "beta" {
      worktrees.path().join("od:d")
    } else {
      worktrees.path().join(name)
    };
    repo.worktree(&format!("{name}-wt"), &dir, None).unwrap();
  }

  let mut cmd = Command::cargo_bin("gwm").unwrap();
  let out = cmd
    .args(["exec", "--workspace"])
    .arg(root.path())
    .args(["--profile", "ci"])
    .assert()
    .failure();
  let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(
    !stdout.contains("run --rm"),
    "no repo ran before the refusal:\n{stdout}"
  );
  let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
  assert!(stderr.contains("od:d"), "the error names the path:\n{stderr}");
}
