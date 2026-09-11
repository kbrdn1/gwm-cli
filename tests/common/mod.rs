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
/// Five ways to neutralise a job, one invariant on the commands it runs, and
/// the two assertions that keep this from passing over nothing:
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
/// - no command the job exists to run may swallow its own failure (issue
///   #652). A `run:` block that invokes cargo must be **exactly one bare
///   cargo invocation**, which is an invariant rather than a list of forbidden
///   spellings. See `assert_run_cannot_swallow_its_failure` for why that
///   distinction is the whole point.
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

    assert_run_cannot_swallow_its_failure(workflow, job, step, job_name, label);
  }
}

/// A `run:` block that invokes cargo must be **exactly one bare cargo
/// invocation**, run by a shell that actually runs it (issue #652).
///
/// This started as a denylist, `||`, a pipe, `set +e`, `exit 0`, and review
/// walked straight through it: `if ! cargo audit --deny warnings; then echo
/// 'advisories found'; fi` leaves the `audit` job green on a red audit, which
/// is the RUSTSEC-2025-0068 scenario itself, and `cargo bench --benches --
/// --test &` leaves `bench` green without waiting. Neither carries any of the
/// four. Enumerating the ways a shell can discard an exit status does not
/// converge, because the shell is a programming language and the list is its
/// grammar.
///
/// So the shape of the command is stated positively: one line, starting with
/// `cargo `, made only of characters that carry no shell meaning. That closes
/// the spellings nobody has thought of yet, and all eight cargo commands in
/// `ci.yml` pass it unchanged.
///
/// Two things the shape alone does not cover, both found by mutating this
/// helper against itself:
///
/// - **the shell that runs it.** An earlier revision dropped the `shell:`
///   assertion, reasoning that a single command's exit status becomes the
///   step's "under every shell GitHub offers". That reasoning was an
///   enumeration in disguise, and `shell:` also takes an arbitrary command
///   line: `shell: 'true {0}'` never runs the script at all, and `shell: bash
///   -n {0}` parses it without executing. Both are green against the shape.
///   So the built-in keywords that do run the script are allowed and nothing
///   else, at step level and in `defaults.run` at job and workflow level,
///   which are the only two places `defaults` exists;
/// - **flags that compile without running.** `cargo bench --no-run --benches
///   -- --test` is one bare invocation and satisfies the shape, while being
///   issue #634 verbatim: the benches build and never run. Unlike shell
///   grammar, the set of cargo flags meaning "do not execute" is finite and
///   documented by cargo, so naming them is a bounded list rather than an
///   open-ended one, and it backs an invariant instead of standing alone.
///
/// Steps that do not invoke cargo are out of scope and stay free-form: the
/// `read the declared MSRV` step pipes `grep -m1 '^rust-version = ' Cargo.toml`
/// into `cut -d'"' -f2` and uses `exit 1` to fail loudly, both correct. The
/// step is detected by a whitespace-delimited `cargo` **token**, not by
/// `contains("cargo ")`: that spelling hangs the whole check on one literal
/// space, so `cargo\tcheck --locked || true` opted out of it entirely. A
/// prefix test is no good either, since `env VAR=x cargo …` defeats it.
/// `Cargo.toml` is not a `cargo` token, so the reader step stays out.
///
/// ## Where this stops (issue #656)
///
/// The property being reached for is "the command fails the job when the code
/// is broken". Between this file and that property sit cargo's own argument
/// parsing, the process environment and `.cargo/config.toml`, and a reader of
/// the workflow does not cross that gap. It can only make a crossing visible
/// in a diff, which is what the shape above does. Three surfaces sit on the
/// far side. Reviewing #654 found them; they are listed below as this branch
/// re-measured them against the commands the workflow actually runs, which
/// moved two of them away from where #656 put them. #656 records the decision
/// to name them here rather than chase them: each review pass opened a surface
/// the previous fix did not touch instead of a variant of it, which is a
/// domain with no last entry.
///
/// - **a filter that matches nothing.** `cargo nextest run --no-tests pass
///   zzz_no_such_test` reports `0 tests run` and exits 0, and `cargo bench
///   --benches -- --test zzz_no_such_bench` exits 0 having measured nothing:
///   `--benches` picks up the lib and bin harnesses alongside the three
///   criterion benches, the harnesses answer `0 tests` and the benches print
///   nothing at all. Each is a single bare line of allowed characters. A bare
///   filter is not enough against nextest 0.9.143, which exits 4 on a run of
///   zero tests, so it is `--no-tests pass` that does the silencing. Telling a
///   filter from the value of a flag means re-implementing cargo's argument
///   parsing on both sides of `--`, which is the mistake #634 already paid
///   for, a model written in place of the oracle;
/// - **the environment.** `CARGO_TARGET_<TRIPLE>_RUNNER: "true"` has cargo put
///   each test or bench binary through `true` instead of executing it, and
///   `env:` sits three lines from the `RUSTFLAGS` the `msrv` job already
///   overrides. Measured against the commands this workflow runs, it reaches
///   `bench` and stops there:
///   `CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER=true cargo bench --benches --
///   --test` reports all five of those binaries as run and exits 0 without
///   executing one, the harness lines gone with them. `cargo nextest run`
///   exits 4 under the same override, since nextest lists tests by running
///   each binary and `true` lists none, so the `test` job goes red rather than
///   quiet. Walking the `env:` mappings would not close the bench case either,
///   because a step can write the same variable into `$GITHUB_ENV`, which
///   GitHub documents as inherited by every later step of the job;
/// - **configuration on disk.** `target.<triple>.runner` in
///   `.cargo/config.toml` is that override in file form, and a step can write
///   it before the cargo step runs.
///
/// One vector named in the issue is deliberately not in that list. Both the
/// non-matching filter and the runner override leave `cargo test --doc` at
/// `0 passed`, exit 0, and so does a plain `cargo test --doc` on this tree:
/// the crate has no Rust doctests, every fenced block in its doc comments
/// being `text`, `toml` or `go`. Nothing is being silenced there, the step
/// has nothing to run, which is a defect of its own and not a limit of this
/// guard, filed as #659.
///
/// The last two surfaces are one shape: a step this guard leaves free-form
/// reconfigures what cargo reads, and the cargo line it does guard is
/// untouched. Closing them means constraining every step of a job rather than
/// its cargo steps, which is a different guard with a different cost.
///
/// So a green run here says the workflow carries no visible off switch on the
/// cargo commands it guards. It does not say CI is honest. The instrument that
/// would say that is observational, a canary that breaks a test on a scratch
/// branch and watches the checks turn red, and it costs a CI run every time it
/// runs. #656 holds that trade-off.
#[allow(dead_code)] // used by the two test binaries that parse ci.yml.
fn assert_run_cannot_swallow_its_failure(
  workflow: &serde_yaml_ng::Value,
  job: &serde_yaml_ng::Value,
  step: &serde_yaml_ng::Value,
  job_name: &str,
  label: &str,
) {
  let Some(script) = step["run"].as_str() else {
    return;
  };
  let lines: Vec<&str> = script.lines().collect();
  let invokes_cargo = |l: &&str| l.split_whitespace().any(|tok| tok == "cargo");
  if !lines.iter().any(invokes_cargo) {
    return;
  }

  // Whitelist, not a denylist: `shell:` accepts an arbitrary command line
  // (`shell: command [...options] {0} [...more_options]`), so anything that is
  // not a built-in keyword known to execute the script has to be refused
  // rather than inspected. `defaults.run.shell` sets the same thing for a
  // whole job or the whole workflow, and those are the only two levels it
  // exists at.
  for (shell, where_) in [
    (&step["shell"], "on this step".to_string()),
    (
      &job["defaults"]["run"]["shell"],
      format!("in `defaults.run` on the `{job_name}` job"),
    ),
    (
      &workflow["defaults"]["run"]["shell"],
      "in the workflow's `defaults.run`".to_string(),
    ),
  ] {
    assert!(
      shell.is_null() || matches!(shell.as_str(), Some("bash" | "sh" | "pwsh")),
      "step {label:?} of the `{job_name}` job runs cargo under `shell: {shell:?}` set \
       {where_}. Only the built-in `bash`, `sh` and `pwsh` keywords are allowed, because \
       `shell:` otherwise takes a whole command line: `true {{0}}` never runs the script and \
       `bash -n {{0}}` only parses it, both leaving a green step over a command that never \
       executed"
    );
  }

  assert_eq!(
    lines.len(),
    1,
    "step {label:?} of the `{job_name}` job invokes cargo, so its `run:` must be that one \
     command and nothing else. A second line is where the exit status gets discarded, by an \
     `exit 0`, by an `echo` that reports its own status, or by a `set +e` that stops errexit \
     from ever reaching it. Got {script:?}"
  );

  // Bare invocation: the characters allowed are the ones that appear in a
  // cargo command line and carry no meaning to the shell. Everything else,
  // `|`, `&`, `;`, `>`, backtick, `$`, `(`, `!`, `#`, quotes and even a tab,
  // is either a way to discard the status or a way to run something that is
  // not this command.
  let line = lines[0];
  let bare = line.starts_with("cargo ")
    && line
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || " ._:/@=+-".contains(c));
  assert!(
    bare,
    "step {label:?} of the `{job_name}` job must run cargo as a bare command: one \
     invocation, no shell operators. `if ! cargo audit …; then …; fi` and `cargo bench … &` \
     both report success over a failure while carrying no `||`, no pipe, no `set +e` and no \
     `exit 0`, which is why this is stated as an invariant and not as a list of forbidden \
     spellings. A `${{{{ … }}}}` expression is refused too, deliberately: the runner splices it \
     into the line before any shell sees it, so an attacker-controlled value becomes shell \
     source. Pass it through `env:` and read it as `$VAR`, which is GitHub's own advice, in a \
     step this guard leaves free-form. Got {line:?}"
  );

  for flag in ["--no-run", "--dry-run"] {
    assert!(
      !line.split_whitespace().any(|tok| tok == flag),
      "step {label:?} of the `{job_name}` job must not pass `{flag}`: it compiles the \
       target and never executes it, so the step reports success over work that never ran. \
       `cargo bench --no-run --benches -- --test` is issue #634 verbatim, a bench that \
       builds and is never run. Got {line:?}"
    );
  }
}

/// The matrix rows a job actually runs, after `exclude` is applied (issue
/// #653).
///
/// Reading `strategy.matrix.os` alone is what `ci_test_matrix_runs_on_windows_latest`
/// did, and an `exclude:` beside it deletes rows without touching that list:
/// `test (windows-latest)` stops existing while the guard whose entire subject
/// is the matrix keeps passing.
///
/// Anything this cannot reason about panics rather than answering vaguely. A
/// second matrix dimension makes `exclude` a filter over combinations and not
/// over names, and `include:` can add a row back after `exclude` removed it,
/// so either one silently changes what the returned list means.
#[allow(dead_code)] // used by the two test binaries that parse ci.yml.
pub fn effective_matrix_os(job: &serde_yaml_ng::Value, job_name: &str) -> Vec<String> {
  let matrix = &job["strategy"]["matrix"];
  let keys: Vec<String> = matrix
    .as_mapping()
    .map(|m| m.keys().filter_map(|k| k.as_str().map(str::to_owned)).collect())
    .unwrap_or_default();
  for key in &keys {
    assert!(
      key == "os" || key == "exclude",
      "the `{job_name}` matrix grew a `{key}` key, and this helper only knows how to apply \
       `exclude` over a single `os` dimension. A second dimension makes `exclude` a filter \
       over combinations, and `include:` adds rows back after `exclude` removed them, so \
       the list returned here would no longer mean what its callers read it as"
    );
  }

  let declared: Vec<String> = matrix["os"]
    .as_sequence()
    .cloned()
    .unwrap_or_default()
    .iter()
    .filter_map(|v| v.as_str().map(str::to_owned))
    .collect();
  let excluded: Vec<String> = matrix["exclude"]
    .as_sequence()
    .cloned()
    .unwrap_or_default()
    .iter()
    .filter_map(|e| e["os"].as_str().map(str::to_owned))
    .collect();

  // `runs-on:` is what actually decides where a row executes, and it sits one
  // line above the matrix this function reads. Pinning it to the matrix is what
  // makes the returned list mean anything: a literal `runs-on: ubuntu-latest`
  // leaves `test (windows-latest)` in the checks list, satisfying the required
  // contexts on `main`, while nothing ever compiles the
  // `[target."cfg(windows)".dependencies]` block. Worse than the `exclude:`
  // this function exists to catch, which at least deletes the row.
  let runs_on = job["runs-on"].as_str().unwrap_or_default();
  assert_eq!(
    runs_on, "${{ matrix.os }}",
    "the `{job_name}` job has a matrix, so its `runs-on:` must derive from it. A literal \
     runner makes every row execute on the same machine while the rows keep their per-OS \
     names, so the checks list still advertises the platform nobody tested on"
  );

  declared.into_iter().filter(|os| !excluded.contains(os)).collect()
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
