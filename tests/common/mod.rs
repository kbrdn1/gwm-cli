//! Shared test helpers. `mod.rs` to opt out of being picked up as its own
//! integration target (cargo treats top-level `tests/*.rs` files as targets).

use git2::{Repository, Signature};
use std::path::Path;
use tempfile::TempDir;

/// Initialize a tempdir with a fresh git repo on `main` carrying one empty
/// commit. Returns the tempdir (kept alive by the caller) and the repo handle.
#[allow(dead_code)] // unused by capture_pipeline_tests; cargo compiles common per-test crate.
pub fn init_repo() -> (TempDir, Repository) {
  let dir = TempDir::new().unwrap();
  let repo = Repository::init(dir.path()).unwrap();
  repo.set_head("refs/heads/main").ok();

  let sig = Signature::now("gwm-test", "gwm@test").unwrap();
  let tree_id = {
    let mut index = repo.index().unwrap();
    index.write_tree().unwrap()
  };
  let tree = repo.find_tree(tree_id).unwrap();
  repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();

  let reopened = Repository::open(dir.path()).unwrap();
  (dir, reopened)
}

/// Canonicalize two paths and compare them. On macOS `/var/...` and
/// `/private/var/...` denote the same inode but compare unequal as strings.
#[allow(dead_code)] // used only by worktree_integration; cargo compiles common per-test crate.
pub fn paths_equal(a: &Path, b: &Path) -> bool {
  let a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
  let b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
  a == b
}

/// A directory holding nothing but a **working** `git`, for a test that hands
/// a subprocess a minimal `PATH`.
///
/// Two traps, one after the other. `/usr/bin:/bin` assumes a git in
/// `/usr/bin`, which is not a property of a POSIX system: where git comes from
/// nix or Homebrew, `/usr/bin/git` is the Xcode shim, and with no command line
/// tools installed it writes nothing, prints an install prompt on stderr and
/// **exits 0**. A caller that only checks the exit status reads that as an
/// empty answer. So the candidate is run before it is trusted, and one that
/// does not answer `git version …` is refused by name rather than symlinked
/// into the shim directory for every case to fail against.
///
/// A directory of its own rather than git's: the tools these suites stub are
/// meant to be a stub or absent, and git's neighbours on a Homebrew prefix or
/// a nix profile can include a real `cargo` or `gwm`, which would silently
/// defeat the case that wants one missing.
#[cfg(unix)]
#[allow(dead_code)] // used by the suites that drive a shell script.
pub fn git_only_bin() -> &'static Path {
  use std::sync::OnceLock;
  static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
  DIR
    .get_or_init(|| {
      let found = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v git")
        .output()
        .expect("locating git ran");
      let git = String::from_utf8_lossy(&found.stdout).trim().to_string();
      assert!(
        !git.is_empty(),
        "git must be on PATH: this suite hands a subprocess a minimal PATH and the script under \
         test opens on a git command"
      );
      let version = std::process::Command::new(&git)
        .arg("--version")
        .output()
        .expect("git --version ran");
      let reported = String::from_utf8_lossy(&version.stdout);
      assert!(
        reported.starts_with("git version"),
        "`{git}` is on PATH but does not answer `git version …` (it said {reported:?}). On macOS \
         that is the Xcode shim with no command line tools behind it: it exits 0 having done \
         nothing, so every case here would fail against an empty answer. Install the tools, or \
         run this suite with the real git first on PATH"
      );
      let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shell-suite-git-bin");
      std::fs::create_dir_all(&dir).expect("the git shim directory is creatable");
      let link = dir.join("git");
      // Recreated rather than reused: the toolchain moves between runs, and a
      // symlink to a garbage-collected nix store path resolves to nothing.
      //
      // Staged under a unique name and renamed into place rather than
      // unlinked and re-created. The directory is a fixed path, and the
      // `OnceLock` above only serialises the tests sharing ONE process:
      // `cargo-nextest` gives each test its own, so several of them reach
      // here at once and unlink-then-symlink loses that race with
      // `AlreadyExists`. `rename` is atomic and replaces the destination, so
      // concurrent callers each publish a link that resolves to the same git.
      let staging = dir.join(format!("git.{}", std::process::id()));
      let _ = std::fs::remove_file(&staging);
      std::os::unix::fs::symlink(&git, &staging).expect("git symlinks into the shim directory");
      std::fs::rename(&staging, &link).expect("the git shim link moves into place");
      dir
    })
    .as_path()
}

/// Asserts that a CI job is **blocking**: that it runs, and that it can turn
/// the workflow red.
///
/// Issue #646. GitHub Actions applies `if:` and `continue-on-error:` at the
/// job level as well as the step level, and the job wins. A guard that reads
/// only `step[...]` therefore says nothing about the job containing it: `if:
/// false` on the `test` job left all 19 tests in `release_workflow_tests`
/// green while `cargo build`, `cargo nextest run` and `cargo test --doc` had
/// stopped running, and `continue-on-error` on the `audit` job is what hid
/// RUSTSEC-2025-0068 for nine months.
///
/// Five ways to neutralise a job, plus the two assertions that keep this from
/// passing over nothing:
///
/// - the job must **exist**, and hold at least one step. An absent job parses
///   to `Value::Null`, and `null["if"]` is null, `null["steps"]` yields an
///   empty sequence, and `.all()` over an empty sequence is true. Deleting a
///   job outright would otherwise walk past every assertion below;
/// - no `if:` on the job, and none on any step, except the exact conditions
///   passed in `steps_allowed_an_if`;
/// - no `continue-on-error:` on the job, and none on any step;
/// - nothing in its `needs:` closure is narrowed with an `if:`. GitHub skips a
///   job whose dependency was skipped, so `needs: doctor` on a guarded job
///   stops it on every event that does not target `dev` while all of the above
///   stay green. `ci.yml` ships exactly such a job one screen away, which is
///   why this is checked rather than assumed. A `continue-on-error:` on a
///   dependency is deliberately not an error: it makes the dependency report
///   success, which lets the dependent run rather than skipping it.
///
/// The whole closure is walked, not just the direct dependencies: a job two
/// hops away from a conditional one is skipped exactly the same way.
///
/// `steps_allowed_an_if` carries the **value**, not a dispensation: a step
/// listed here still has to match the condition it was allowed, so widening
/// `matrix.os == 'ubuntu-latest'` into `false` is caught here and not left to
/// whichever other test happens to pin that step today.
///
/// The `if:` comparisons go through the `Value`, never `as_str()`: `if: false`
/// is a YAML boolean, so `as_str()` hands back `None` for it exactly as it
/// does for an absent key, and the canonical way to switch something off would
/// take the "no `if:` at all" arm (the defect fixed at `6bb82758`).
#[allow(dead_code)] // used by the two test binaries that parse ci.yml.
pub fn assert_job_is_blocking(workflow: &serde_yaml_ng::Value, job_name: &str, steps_allowed_an_if: &[(&str, &str)]) {
  let job = &workflow["jobs"][job_name];
  assert!(
    !job.is_null(),
    "ci.yml must define a `{job_name}` job: an absent job parses to null, and every \
     assertion below passes over null, so deleting the job would go unseen"
  );
  assert!(
    job["if"].is_null(),
    "the `{job_name}` job must carry no `if:`: a job-level condition switches every step \
     off at once while step-level guards stay green, got `if: {:?}`",
    job["if"]
  );
  assert!(
    job["continue-on-error"].is_null(),
    "the `{job_name}` job must carry no `continue-on-error:` at all. The key has to be \
     absent, not `false`, so that a later `true` is a diff against nothing rather than a \
     one-word edit. Got `continue-on-error: {:?}`",
    job["continue-on-error"]
  );

  let steps = job["steps"].as_sequence().cloned().unwrap_or_default();
  assert!(
    !steps.is_empty(),
    "the `{job_name}` job must hold at least one step: emptying `steps:` leaves a job that \
     runs and reports success while doing nothing"
  );

  // The `needs:` closure. A dependency that gets skipped skips everything
  // below it, so a job whose own `if:` and `continue-on-error:` are clean can
  // still be switched off through the job it waits on.
  let mut pending: Vec<String> = job_needs(job);
  let mut seen: Vec<String> = vec![job_name.to_string()];
  while let Some(dep) = pending.pop() {
    if seen.contains(&dep) {
      continue;
    }
    let upstream = &workflow["jobs"][dep.as_str()];
    assert!(
      !upstream.is_null(),
      "the `{job_name}` job waits on `{dep}`, which ci.yml does not define: the workflow \
       would not start at all"
    );
    assert!(
      upstream["if"].is_null(),
      "the `{job_name}` job waits on `{dep}`, which is narrowed with `if: {:?}`. GitHub \
       skips a job whose dependency was skipped, so this switches `{job_name}` off on every \
       event the condition excludes while its own `if:` and `continue-on-error:` stay clean",
      upstream["if"]
    );
    pending.extend(job_needs(upstream));
    seen.push(dep);
  }

  // A waiver names one step, so it has to land on one step. The label comes
  // from `name:`, which anyone can edit: renaming a second step to the label a
  // waiver was written for hands that step the exemption too. That is not
  // theoretical, it was found by mutation on this very helper: relabelling
  // `cargo nextest run` as `cargo test --doc` and giving it the ubuntu `if:`
  // narrows the whole suite to one runner with every test still green.
  for (name, cond) in steps_allowed_an_if {
    let hits = steps.iter().filter(|s| step_label(s) == *name).count();
    assert_eq!(
      hits, 1,
      "the `{job_name}` job allows step {name:?} the condition {cond:?}, and exactly one step \
       must answer to that label, found {hits}. Zero means the step was renamed and the waiver \
       now covers nothing; more than one means a second step inherited an exemption written \
       for its neighbour"
    );
  }

  for step in &steps {
    let label = step_label(step);
    assert!(
      step["continue-on-error"].is_null(),
      "no step of the `{job_name}` job may swallow its own failure: step {label:?} carries \
       `continue-on-error: {:?}`",
      step["continue-on-error"]
    );

    let cond = &step["if"];
    let allowed = steps_allowed_an_if
      .iter()
      .find(|(name, _)| *name == label)
      .map(|(_, cond)| *cond);
    assert!(
      cond.is_null() || (allowed.is_some() && cond.as_str() == allowed),
      "step {label:?} of the `{job_name}` job may not be conditioned away: it carries \
       `if: {cond:?}` and the only condition allowed for it is {allowed:?}"
    );
  }
}

/// The `needs:` of a job, as a list. The Actions schema allows both a bare
/// string and a sequence, and a job with no dependency parses to null, so the
/// three shapes are read here rather than at each call site.
#[allow(dead_code)] // used by the two test binaries that parse ci.yml.
fn job_needs(job: &serde_yaml_ng::Value) -> Vec<String> {
  match &job["needs"] {
    serde_yaml_ng::Value::String(one) => vec![one.clone()],
    serde_yaml_ng::Value::Sequence(many) => many
      .iter()
      .map(|v| {
        v.as_str()
          .unwrap_or_else(|| panic!("a `needs:` entry must be a job name, got {v:?}"))
          .to_string()
      })
      .collect(),
    serde_yaml_ng::Value::Null => Vec::new(),
    other => panic!("`needs:` must be a job name or a list of them, got {other:?}"),
  }
}

/// How a step is named in an assertion message, and the key a waiver in
/// `steps_allowed_an_if` is matched on. `name:` first because that is what the
/// workflow author reads, then `uses:` for the action-only steps that carry no
/// name, then the script itself.
#[allow(dead_code)] // used by the two test binaries that parse ci.yml.
fn step_label(step: &serde_yaml_ng::Value) -> &str {
  step["name"]
    .as_str()
    .or_else(|| step["uses"].as_str())
    .or_else(|| step["run"].as_str())
    .unwrap_or("<unnamed step>")
}
