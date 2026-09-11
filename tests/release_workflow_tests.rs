use std::fs;
#[cfg(unix)]
use std::{path::Path, process::Command};

mod common;
use common::{assert_job_is_blocking, effective_matrix_os};

#[cfg(unix)]
const CHECK_RC_DUPES: &str = ".github/scripts/check-rc-changelog-dupes.sh";

#[test]
fn stable_release_workflow_skips_prerelease_tags() {
  let workflow = fs::read_to_string(".github/workflows/release.yml").unwrap();

  for suffix in ["-rc.", "-alpha.", "-beta."] {
    let guard = format!("!contains(github.event.inputs.tag || github.ref_name, '{suffix}')");
    assert!(
      workflow.contains(&guard),
      "release.yml must guard stable release jobs against {suffix} tags"
    );
  }
}

#[test]
fn stable_release_publish_uses_github_cli_with_workflow_token() {
  let workflow = fs::read_to_string(".github/workflows/release.yml").unwrap();
  let publish_step = workflow
    .split("      - name: publish release")
    .nth(1)
    .and_then(|tail| tail.split("\n  homebrew-tap-update:").next())
    .expect("release.yml must contain a publish release step before homebrew-tap-update");

  assert!(
    !workflow.contains("uses: softprops/action-gh-release"),
    "release.yml must not use softprops/action-gh-release for the stable GitHub Release publish step"
  );
  assert!(
    publish_step.contains("GH_TOKEN: ${{ github.token }}"),
    "release.yml must pass the workflow token to gh via GH_TOKEN in the publish release step"
  );
  assert!(
    publish_step.contains("gh release create \"$TAG\""),
    "release.yml must create the stable GitHub Release with gh release create"
  );
  assert!(
    publish_step.contains("--notes-file \"${{ steps.changelog.outputs.path }}\""),
    "stable release notes must still come from changelogs/<version>.md"
  );
  assert!(
    publish_step.contains("gh release upload \"$TAG\"") && publish_step.contains("--clobber"),
    "release.yml must upload artifacts with gh release upload --clobber so recovery reruns can replace assets"
  );
}

/// Every `actions/checkout` in `release.yml`, paired with its `with:` block.
fn release_workflow_checkout_steps() -> Vec<(String, serde_yaml_ng::Value)> {
  workflow_checkout_steps(".github/workflows/release.yml")
}

/// Every `actions/checkout` in the given workflow, paired with its `with:`
/// block.
///
/// Parsing the YAML rather than grepping the text keeps the invariants below
/// honest: a step that spells its inputs differently, or a job that grows a
/// second checkout, is still seen.
fn workflow_checkout_steps(path: &str) -> Vec<(String, serde_yaml_ng::Value)> {
  let workflow: serde_yaml_ng::Value = serde_yaml_ng::from_str(&fs::read_to_string(path).unwrap()).unwrap();

  let mut steps = Vec::new();
  for (job_name, job) in workflow["jobs"].as_mapping().expect("workflow must define jobs") {
    let job_name = job_name.as_str().unwrap_or_default().to_string();
    let Some(job_steps) = job["steps"].as_sequence() else {
      continue;
    };
    for step in job_steps {
      let uses = step["uses"].as_str().unwrap_or_default();
      if uses.starts_with("actions/checkout@") {
        steps.push((job_name.clone(), step["with"].clone()));
      }
    }
  }
  steps
}

/// A checkout that is only there to read the tree (sources, packaging
/// templates, render scripts, `changelogs/`) has no use for the auto-injected
/// token `actions/checkout` writes into `.git/config`. Leaving it there hands a
/// credential to every later step in the job, including the ones that render
/// templates from release data.
///
/// The discriminator is the explicit `token:` input: the two checkouts that
/// genuinely push (the Homebrew tap and the Scoop bucket) pass a scoped PAT and
/// rely on it being persisted. Everything else must opt out.
#[test]
fn release_workflow_checkouts_without_a_token_do_not_persist_credentials() {
  let mut audited = 0;

  for (job, with) in release_workflow_checkout_steps() {
    if !with["token"].is_null() {
      continue;
    }
    audited += 1;
    assert_eq!(
      with["persist-credentials"].as_bool(),
      Some(false),
      "the checkout in job `{job}` does not push, so it must set `persist-credentials: false`"
    );
  }

  assert!(
    audited >= 4,
    "expected at least 4 credential-free checkouts in release.yml, found {audited} — the parser is \
     probably no longer seeing the steps"
  );
}

/// Every workflow in the directory, so a file added later is audited by
/// construction rather than by remembering to extend a hand-written list. The
/// three sweeps below all enumerate from here: naming files individually is
/// how a new workflow silently escapes an invariant that was supposed to be
/// repo-wide.
fn workflow_paths() -> Vec<String> {
  let mut paths: Vec<String> = fs::read_dir(".github/workflows")
    .expect("the workflows directory must exist")
    .map(|e| e.unwrap().path())
    .filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml"))
    .map(|p| p.to_str().unwrap().to_string())
    .collect();
  paths.sort();
  paths
}

/// #433, the follow-up to #429/#432: the sibling workflows carry the same
/// shape, and none of their checkouts pushes — `ci.yml` is entirely read-only,
/// `pre-release.yml` publishes through `gh` with an env token, never
/// `git push`, and `docs-sync.yml` only calls an API. No checkout outside
/// release.yml has any business passing `token:`, so the rule is stricter
/// there: every checkout opts out, no exceptions.
///
/// The set is discovered, not listed: release.yml is the single exception
/// (it owns the audited token split above), everything else is swept.
#[test]
fn sibling_workflow_checkouts_do_not_persist_credentials() {
  let mut swept = 0;
  let mut audited = 0;

  for path in workflow_paths() {
    // By file name, not by path: `read_dir` joins with the platform separator,
    // so a full-path comparison against `.github/workflows/release.yml` misses
    // on Windows, and the one workflow that is *supposed* to carry a token
    // gets swept with the rest. Caught by `test (windows-latest)`, green on
    // the other two runners.
    //
    // And by the whole name, not a suffix: `pre-release.yml` ends with
    // `release.yml`, so `ends_with` would drop a workflow that must be
    // audited. The `swept` floor below is what would catch that.
    // Fully qualified: `Path` is imported under `#[cfg(unix)]` in this file,
    // next to the `Command` the shell-script tests need, and this test is not
    // conditional. Reaching for the short name would have failed to compile on
    // the very runner the line above exists for.
    let is_release = std::path::Path::new(&path).file_name().and_then(|f| f.to_str()) == Some("release.yml");
    if is_release {
      continue;
    }
    swept += 1;

    for (job, with) in workflow_checkout_steps(&path) {
      assert!(
        with["token"].is_null(),
        "the checkout in `{path}` job `{job}` passes an explicit token, but nothing in this \
         workflow pushes — drop it or move the job behind release.yml's audited split"
      );
      audited += 1;
      assert_eq!(
        with["persist-credentials"].as_bool(),
        Some(false),
        "the checkout in `{path}` job `{job}` does not push, so it must set \
         `persist-credentials: false`"
      );
    }
  }

  // A glob that matches nothing passes vacuously, and so does one that stops
  // seeing the steps inside the files it matched. Both floors are the counts
  // at the time of writing, minus release.yml.
  assert!(
    swept >= 3,
    "expected at least 3 workflows besides release.yml, found {swept} — the directory listing is \
     probably no longer seeing them"
  );
  assert!(
    audited >= 8,
    "expected at least 8 credential-free checkouts outside release.yml, found {audited} — the \
     parser is probably no longer seeing the steps"
  );
}

/// The mirror of the invariant above: the two checkouts that push must keep the
/// credential they were handed. A blanket `persist-credentials: false` sweep
/// across the file would break `git push` in both publish jobs, and it would
/// break it at tag time, on the one run nobody gets to retry cheaply.
#[test]
fn release_workflow_publishing_checkouts_keep_their_token() {
  let pushing: Vec<_> = release_workflow_checkout_steps()
    .into_iter()
    .filter(|(_, with)| !with["token"].is_null())
    .collect();

  assert_eq!(
    pushing.len(),
    2,
    "expected exactly the tap and bucket checkouts to carry a token, found {}",
    pushing.len()
  );

  for (job, with) in pushing {
    assert_ne!(
      with["persist-credentials"].as_bool(),
      Some(false),
      "job `{job}` pushes with its token, so it must not disable credential persistence"
    );
  }
}

/// The AUR publish automation was removed in #430: `gwm-cli-bin` is maintained
/// on the AUR by a third party, so the job never had push rights on it. Being
/// advisory, it failed silently on every stable tag while the release run
/// reported success, which is the worst of both worlds: the docs read as
/// automated and nobody sees the failure.
///
/// `AUR_SSH_PRIVATE_KEY` is pinned alongside the job because the secret was
/// malformed to begin with (`invalid format` at the v1.2.0 tag). Resurrecting
/// a reference to it by copy-paste would fail the same way, quietly.
///
/// If co-maintenance of the package is ever granted, deleting this test is the
/// correct first step of the change that brings the job back, not a workaround
/// for it. The template, render script and their tests were kept intact for
/// exactly that.
#[test]
fn release_workflow_carries_no_aur_publish_automation() {
  let workflow = fs::read_to_string(".github/workflows/release.yml").unwrap();

  for needle in [
    "aur-publish",
    "AUR_SSH_PRIVATE_KEY",
    "github-actions-deploy-aur",
    "gwm-cli-bin",
  ] {
    assert!(
      !workflow.contains(needle),
      "release.yml must not reference `{needle}`: the AUR package is maintained by a third party \
       (#430) and is refreshed by hand, see CONTRIBUTING.md > Releases > AUR"
    );
  }
}

/// The winget publish automation was removed in #448: `WINGET_TOKEN` was never
/// provisioned, so the guard step turned every stable release run into a red
/// "publish kbrdn1.gwm to winget" job, and the channel is blocked upstream
/// anyway (the initial manifest PR microsoft/winget-pkgs#403295 sits on
/// Needs-CLA, and `komac update` can only update a package that already
/// exists). winget joins the AUR, Nixpkgs and aqua as a channel fed by hand:
/// the maintainer runs a pinned `komac update ... --submit` after a stable
/// release, see CONTRIBUTING.md > Releases > winget.
///
/// If the channel is unblocked and manual submissions prove routine, deleting
/// this test is the correct first step of the change that brings the job back.
#[test]
fn release_workflow_carries_no_winget_publish_automation() {
  let workflow = fs::read_to_string(".github/workflows/release.yml").unwrap();

  // Broad on purpose: `winget`/`WINGET` also catches the `winget-releaser`
  // action the removed wiring test explicitly banned (it would resurrect a
  // mutable-ref binary with a classic PAT in scope), not just the old job's
  // own identifiers.
  for needle in ["winget", "WINGET", "komac"] {
    assert!(
      !workflow.contains(needle),
      "release.yml must not reference `{needle}`: winget submissions are made by hand (#448), \
       see CONTRIBUTING.md > Releases > winget"
    );
  }
}

#[test]
fn prerelease_workflow_does_not_match_stable_tags() {
  let workflow = fs::read_to_string(".github/workflows/pre-release.yml").unwrap();

  assert!(
    workflow.contains("\"v*.*.*-rc.*\""),
    "pre-release.yml must trigger on rc tags"
  );
  assert!(
    workflow.contains("\"v*.*.*-alpha.*\""),
    "pre-release.yml must trigger on alpha tags"
  );
  assert!(
    workflow.contains("\"v*.*.*-beta.*\""),
    "pre-release.yml must trigger on beta tags"
  );
  assert!(
    !workflow.contains("\n      - \"v*.*.*\""),
    "pre-release.yml must not trigger on stable tags"
  );
}

const DOCS_SYNC: &str = ".github/workflows/docs-sync.yml";

fn docs_sync_triggers() -> serde_yaml_ng::Value {
  let workflow: serde_yaml_ng::Value = serde_yaml_ng::from_str(&fs::read_to_string(DOCS_SYNC).unwrap()).unwrap();
  workflow["on"].clone()
}

/// The published docs must only ever show what was *delivered*. `main` is the
/// delivered state — it is reached exclusively through a `dev` → `main` PR —
/// so the sync fires on pushes to `main` and on nothing else.
///
/// A `dev` trigger would publish pages describing unreleased behaviour. A tag
/// trigger would be worse: GitHub's `v*.*.*` glob also matches `v0.8.0-rc.4`,
/// the exact trap `release.yml` has to guard against by hand, so the docs of
/// every release candidate would go live as if they were stable.
#[test]
fn docs_sync_fires_on_pushes_to_main_only() {
  let on = docs_sync_triggers();
  let push = &on["push"];

  let branches: Vec<&str> = push["branches"]
    .as_sequence()
    .expect("docs-sync.yml must restrict its push trigger to a branch list")
    .iter()
    .filter_map(|b| b.as_str())
    .collect();
  assert_eq!(
    branches,
    ["main"],
    "docs-sync.yml must fire on `main` alone: any other branch publishes undelivered docs"
  );

  assert!(
    push["tags"].is_null(),
    "docs-sync.yml must not trigger on tags: the `v*.*.*` glob matches pre-release tags too"
  );
  assert!(
    on["pull_request"].is_null(),
    "docs-sync.yml must not trigger on pull requests: it publishes, it does not check"
  );
}

/// The paths filter is the one half of the contract this repo cannot see: the
/// conversion script lives in `kbrdn1/kbrdn-docs` and reads a fixed set of
/// roots out of this repo. A root dropped here does not break anything loudly
/// — it just stops waking the sync, and the site quietly serves stale pages.
///
/// So the list is pinned literally. If the site starts reading another root,
/// this test is where that gets recorded.
///
/// `Cargo.toml` is in the set and is the reason this test exists rather than
/// being obvious: it is not documentation, but `sync-gwm-docs.mjs` reads the
/// `[package]` version out of it for the release badge in the site header.
/// A release moves `changelogs/` in the same push, so the omission only shows
/// when a version bump travels alone, and the failure is a stale badge rather
/// than an error.
#[test]
fn docs_sync_watches_every_root_the_site_reads() {
  let on = docs_sync_triggers();
  let paths: Vec<&str> = on["push"]["paths"]
    .as_sequence()
    .expect("docs-sync.yml must filter its push trigger by path")
    .iter()
    .filter_map(|p| p.as_str())
    .collect();

  for root in ["docs/**", "changelogs/**", "Cargo.toml"] {
    assert!(
      paths.contains(&root),
      "docs-sync.yml must watch `{root}`: the site's sync script reads it, so a change there \
       has to wake the sync (paths = {paths:?})"
    );
  }
}

/// One `ci.yml` job by name, parsed. The text-slicing predecessor of this
/// helper cut the `test` job out between the literal `  test:` and
/// `\n  hook-smoke:` markers, so inserting any job between the two silently
/// emptied what the assertions ran against, the same failure mode the `msrv`
/// tests already parse the YAML to avoid.
fn ci_workflow() -> serde_yaml_ng::Value {
  serde_yaml_ng::from_str(&fs::read_to_string(".github/workflows/ci.yml").unwrap()).expect("ci.yml must be valid YAML")
}

fn ci_job(name: &str) -> serde_yaml_ng::Value {
  let workflow = ci_workflow();
  let job = workflow["jobs"][name].clone();
  assert!(!job.is_null(), "ci.yml must define a `{name}` job");
  job
}

/// The `run:` scripts of a job's steps, in order.
fn run_steps(job: &serde_yaml_ng::Value) -> Vec<String> {
  job["steps"]
    .as_sequence()
    .cloned()
    .unwrap_or_default()
    .iter()
    .filter_map(|s| s["run"].as_str().map(str::to_owned))
    .collect()
}

/// Read through `effective_matrix_os` and not `strategy.matrix.os` directly
/// (issue #653): an `exclude:` beside that list deletes rows without touching
/// it, so `test (windows-latest)` stops existing while this guard, whose whole
/// subject is the matrix, keeps passing.
#[test]
fn ci_test_matrix_runs_on_windows_latest() {
  let job = ci_job("test");
  let matrix = effective_matrix_os(&job, "test");

  for os in ["ubuntu-latest", "macos-latest", "windows-latest"] {
    assert!(
      matrix.iter().any(|m| m == os),
      "ci.yml test matrix must include {os} (matrix is {matrix:?})"
    );
  }

  let runs = run_steps(&job);
  assert!(
    runs.iter().any(|r| r.contains("cargo build")),
    "windows-latest must run the same cargo build step as the other test matrix rows, got {runs:?}"
  );
  assert!(
    runs.iter().any(|r| r.contains("cargo nextest run")),
    "windows-latest must run the same test step as the other test matrix rows, got {runs:?}"
  );
}

/// `cargo-nextest` has no `cargo install` step on purpose: building it from
/// source on every runner, windows-latest most of all, costs minutes against
/// the seconds a prebuilt binary takes (issue #634). The swap is time-neutral
/// on wall-clock, so it is bought for process-per-test isolation and cannot
/// afford to pay minutes for the tool. The install action is pinned here
/// rather than left to whoever next edits the job.
#[test]
fn ci_installs_nextest_from_a_prebuilt_binary() {
  let job = ci_job("test");
  let uses: Vec<String> = job["steps"]
    .as_sequence()
    .cloned()
    .unwrap_or_default()
    .iter()
    .filter_map(|s| s["uses"].as_str().map(str::to_owned))
    .collect();
  assert!(
    uses.iter().any(|u| u.starts_with("taiki-e/install-action")),
    "the test job must install cargo-nextest from a prebuilt binary, got {uses:?}"
  );
  assert!(
    !run_steps(&job)
      .iter()
      .any(|r| r.contains("cargo install cargo-nextest")),
    "cargo-nextest must arrive prebuilt: building it from source on every runner \
     costs minutes, and the swap has no wall-clock gain to spend them from"
  );
}

/// Issue #634: `benches/sidebar_cache_hit.rs` panicked for 1086 commits and
/// nobody noticed, because no job ran the benches. `cargo bench` also aborts
/// at the first failure, so it masked the third bench on top. This pins the
/// job that would have caught it.
///
/// Two properties, both load-bearing:
///
/// - it must actually RUN them (`--test` is criterion's run-once-measure-
///   nothing mode), not just `--no-run` them: a bench that builds and then
///   panics is exactly the case that went unseen;
/// - it must be able to go red: no `continue-on-error`, and no pipe on the
///   command, which would report the pipe's exit code instead of the runner's.
#[test]
fn ci_runs_the_benches_and_can_fail_on_one() {
  let job = ci_job("bench");
  let runs = run_steps(&job);
  let bench_run = runs
    .iter()
    .find(|r| r.contains("cargo bench"))
    .unwrap_or_else(|| panic!("the bench job must run `cargo bench`, got {runs:?}"));

  // `--benches` and not `--bench <name>`: the second builds and runs one
  // target, which is the partial coverage #634 is about. `cargo bench` alone
  // would also do, but naming the flag keeps a later `--bench sidebar_cache_hit`
  // from passing a guard whose whole subject is a bench nobody ran.
  assert!(
    bench_run.contains("--benches"),
    "the bench job must run every bench target (`--benches`), not one by name: the third \
     bench went unrun for 1086 commits and that is what this job exists to catch, got {bench_run:?}"
  );
  assert!(
    bench_run.contains("-- --test"),
    "the bench job must pass criterion's `--test` so each bench actually runs \
     once (and the job stays a compile-and-run guard, not a timing gate), got {bench_run:?}"
  );
  assert!(
    !bench_run.contains('|'),
    "piping the bench command reports the pipe's exit code, not the bench runner's, \
     the panic this job exists to catch would be swallowed, got {bench_run:?}"
  );
  // A dead bench is what #634 is about, so the job has to be able to go red,
  // and it has to run at all: the `doctor` job in this same file is narrowed
  // with an `if:`, and doing that here would keep every assertion above green
  // while the benches quietly stopped running on pull requests.
  assert_job_is_blocking(&ci_workflow(), "bench", &[]);
}

#[test]
fn prerelease_workflow_checks_unreleased_against_previous_rc_before_publish() {
  let workflow = fs::read_to_string(".github/workflows/pre-release.yml").unwrap();
  let check_pos = workflow
    .find("check unreleased changelog against previous rc")
    .expect("pre-release.yml must run the duplicate changelog guard");
  let publish_pos = workflow
    .find("publish pre-release")
    .expect("pre-release.yml must still publish the pre-release");

  assert!(
    check_pos < publish_pos,
    "duplicate changelog guard must run before publishing the pre-release"
  );
  assert!(
    workflow.contains("./.github/scripts/check-rc-changelog-dupes.sh \"${{ steps.tag.outputs.name }}\""),
    "pre-release.yml must call the duplicate changelog guard with the resolved tag"
  );
}

#[cfg(unix)]
#[test]
fn rc_changelog_dupe_check_fails_on_repeated_bullet() {
  let tmp = tempfile::tempdir().unwrap();
  write_release_files(
    tmp.path(),
    r#"
# Changelog

## [Unreleased]

### Fixed

- Release workflow publishes with the workflow token. (#146)
- Fresh post-rc delta. (#147)

## Past releases
"#,
    r#"
# [0.7.0-rc.2] - 2026-05-23

### Fixed

- Release workflow publishes with the workflow token. (#146)
"#,
  );

  let output = run_dupe_check(tmp.path(), "v0.7.0-rc.3");

  assert!(!output.status.success(), "duplicate bullet must fail the check");
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("#146"),
    "failure should name the duplicated issue ref: {stderr}"
  );
  assert!(
    stderr.contains("Release workflow publishes with the workflow token"),
    "failure should name the duplicated changelog bullet: {stderr}"
  );
}

#[cfg(unix)]
#[test]
fn rc_changelog_dupe_check_fails_on_repeated_issue_ref() {
  let tmp = tempfile::tempdir().unwrap();
  write_release_files(
    tmp.path(),
    r#"
# Changelog

## [Unreleased]

### Changed

- Tighten release workflow token handling. (#146)

## Past releases
"#,
    r#"
# [0.7.0-rc.2] - 2026-05-23

### Fixed

- Release workflow publishes with the workflow token. (#146)
"#,
  );

  let output = run_dupe_check(tmp.path(), "v0.7.0-rc.3");

  assert!(!output.status.success(), "repeated issue ref must fail the check");
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("#146"),
    "failure should name the duplicated issue ref: {stderr}"
  );
}

#[cfg(unix)]
#[test]
fn rc_changelog_dupe_check_allows_new_post_rc_delta() {
  let tmp = tempfile::tempdir().unwrap();
  write_release_files(
    tmp.path(),
    r#"
# Changelog

## [Unreleased]

### Fixed

- Fresh post-rc delta. (#147)

## Past releases
"#,
    r#"
# [0.7.0-rc.2] - 2026-05-23

### Fixed

- Release workflow publishes with the workflow token. (#146)
"#,
  );

  let output = run_dupe_check(tmp.path(), "v0.7.0-rc.3");

  assert!(
    output.status.success(),
    "new post-rc deltas must pass: {}",
    String::from_utf8_lossy(&output.stderr)
  );
}

#[cfg(unix)]
#[test]
fn rc_changelog_dupe_check_skips_first_rc_without_previous_notes() {
  let tmp = tempfile::tempdir().unwrap();
  fs::create_dir_all(tmp.path().join("changelogs/pre-releases")).unwrap();
  fs::write(
    tmp.path().join("CHANGELOG.md"),
    r#"
# Changelog

## [Unreleased]

### Fixed

- First rc entry. (#147)

## Past releases
"#,
  )
  .unwrap();

  let output = run_dupe_check(tmp.path(), "v0.7.0-rc.1");

  assert!(
    output.status.success(),
    "rc.1 has no previous rc to compare: {}",
    String::from_utf8_lossy(&output.stderr)
  );
}

#[cfg(unix)]
fn write_release_files(root: &Path, changelog: &str, previous_rc: &str) {
  fs::create_dir_all(root.join("changelogs/pre-releases")).unwrap();
  fs::write(root.join("CHANGELOG.md"), changelog).unwrap();
  fs::write(root.join("changelogs/pre-releases/0.7.0-rc.2.md"), previous_rc).unwrap();
}

#[cfg(unix)]
fn run_dupe_check(root: &Path, tag: &str) -> std::process::Output {
  let script = std::env::current_dir().unwrap().join(CHECK_RC_DUPES);
  let test_script = root.join(CHECK_RC_DUPES);
  fs::create_dir_all(test_script.parent().unwrap()).unwrap();
  fs::copy(script, &test_script).unwrap();

  Command::new("bash")
    .arg(CHECK_RC_DUPES)
    .arg(tag)
    .current_dir(root)
    .output()
    .unwrap()
}

/// `cargo nextest run` cannot run doctests: they have no test binary for it
/// to schedule. #634 took `cargo test` out of this job, so unless something
/// runs them explicitly the repo silently stops compiling every `///` example
/// it has, which is the same shape of rot as the bench that panicked for 1086
/// commits with nothing to report it.
///
/// This pins the step rather than the property, on purpose, and the detour is
/// worth recording. The first version of this guard scanned `src/` and decided
/// for itself which fences rustdoc would compile, so that CI would not have to
/// pay for a doctest run. Two review passes probed it against the toolchain
/// and it was wrong ten ways: indented blocks carrying no fence at all, fences
/// nested in a blockquote (`src/forge.rs` already writes that style), `~~~`,
/// `/** */` blocks, `#[doc = "..."]` attributes, `{.rust}`, and the tag rules
/// themselves, which turn out to be order-sensitive on 1.96.1
/// (```` ```no_run,text ```` is a doctest, ```` ```text,no_run ```` is not).
///
/// Every one of those was found by asking rustdoc. Which is the point: the
/// oracle was there the whole time, it costs about 35 seconds, and pinning it
/// to ubuntu keeps it off a critical path that windows holds for four minutes
/// more. An emulation that has to track rustdoc's release notes to stay
/// correct is a worse guard than the thing it emulates, however cheap it runs.
#[test]
fn ci_runs_doctests_since_nextest_cannot() {
  let job = ci_job("test");
  let runs = run_steps(&job);
  assert!(
    runs.iter().any(|r| r.contains("cargo test --doc")),
    "the test job must run `cargo test --doc`: nextest cannot, and nothing else in the repo \
     does, so without it every doctest under src/ is compiled and run by nobody. Got {runs:?}"
  );

  // Present is not the same as running. `run_steps` flattens the steps and
  // reports their scripts whatever their `if:`, so `if: false` would leave
  // the assertion above green over a step that never executes, and
  // `continue-on-error` would leave it green over one that never fails. That
  // is not hypothetical here: `continue-on-error` on the `audit` job is what
  // hid RUSTSEC-2025-0068 for nine months, and the bench job carries the same
  // pair of assertions for the same reason.
  let step = job["steps"]
    .as_sequence()
    .cloned()
    .unwrap_or_default()
    .into_iter()
    .find(|s| s["run"].as_str().is_some_and(|r| r.contains("cargo test --doc")))
    .expect("the `cargo test --doc` step was found in the scripts, so it must be in the steps");
  assert!(
    step["continue-on-error"].is_null(),
    "the doctest step must be able to fail the job: a doctest that runs and is not allowed to \
     go red is a doctest nobody runs"
  );
  // One `if:` is legitimate, and only one: doctests behave identically on the
  // three runners, so this pays for them once on the row with the slack.
  // Anything else is the step being switched off by another name.
  //
  // Matched on the VALUE, not through `as_str()`. `if: false` is a YAML
  // boolean, so `as_str()` hands back `None` for it exactly as it does for an
  // absent key: the first version of this check used `match … .as_str()` and
  // the canonical way to switch a step off took its "no `if:` at all" arm.
  let cond = &step["if"];
  assert!(
    cond.is_null() || cond.as_str() == Some("matrix.os == 'ubuntu-latest'"),
    "the doctest step may only be narrowed to the ubuntu matrix row, got `if: {cond:?}`"
  );
}

/// Issue #646. The `test` job is the one carrying `cargo build`, `cargo
/// nextest run` and `cargo test --doc`, so switching it off takes the whole
/// suite with it. `if: false` on this job was mutated into `ci.yml` and all 19
/// tests in this binary stayed green, because every guard here reads
/// `step[...]` and GitHub Actions resolves `if:` at the job level too.
///
/// The doctest step keeps its one legitimate `if:`, because doctests behave
/// identically on the three runners and are paid for once on the row with the
/// slack. The helper pins that condition by value rather than waiving the
/// check for the step.
#[test]
fn ci_test_job_cannot_be_switched_off_or_made_advisory() {
  assert_job_is_blocking(
    &ci_workflow(),
    "test",
    &[("cargo test --doc", "matrix.os == 'ubuntu-latest'")],
  );
}

/// Issue #646. `cargo audit` had no test naming it at all: `grep -rn '"audit"'
/// tests/*.rs` returned nothing, while `tests/release_workflow_tests.rs` cited
/// the job twice to justify guards placed elsewhere. Three simultaneous
/// neutralisations, `continue-on-error: true` on the job and `cargo audit
/// --deny warnings` rewritten to `cargo audit || true`, left 23 tests green
/// across the two binaries that parse `ci.yml`.
///
/// Two properties on top of the job being blocking:
///
/// - `--deny warnings`, without which warning-class advisories (unmaintained /
///   unsound / yanked) exit 0. That is half of how RUSTSEC-2025-0068
///   (`serde_yml`, unsound + unmaintained) slipped past for nine months;
/// - no pipe on the command. A shell pipeline reports the exit status of its
///   last element, so `cargo audit --deny warnings || true` and `cargo audit |
///   tee log` both report success over a failing audit. Testing for `|` covers
///   `||` as well.
///
/// Accepted advisories belong in `audit.toml` (`[advisories] ignore = […]`)
/// with a rationale, which is a conscious decision in a reviewed diff (#340),
/// not a job that cannot fail.
#[test]
fn ci_audits_dependencies_and_can_fail_on_an_advisory() {
  let job = ci_job("audit");
  assert_job_is_blocking(&ci_workflow(), "audit", &[]);

  let step = job["steps"]
    .as_sequence()
    .cloned()
    .unwrap_or_default()
    .into_iter()
    .find(|s| s["name"].as_str() == Some("cargo audit"))
    .expect("the audit job needs a step named `cargo audit` that runs the audit");
  let run = step["run"]
    .as_str()
    .expect("the `cargo audit` step must carry a `run:` script")
    .to_string();

  assert!(
    run.contains("cargo audit"),
    "the `cargo audit` step must actually run cargo-audit, got {run:?}"
  );
  assert!(
    run.contains("--deny warnings"),
    "cargo audit must run with `--deny warnings`: plain `cargo audit` exits 0 on \
     unmaintained, unsound and yanked advisories, which is how RUSTSEC-2025-0068 went \
     unseen for nine months. Got {run:?}"
  );
  assert!(
    !run.contains('|'),
    "the audit command must carry no pipe and no `||`: a shell pipeline reports its last \
     element's exit status, so `cargo audit --deny warnings || true` succeeds over a \
     failing audit exactly as the `continue-on-error` this guard also forbids. Got {run:?}"
  );
}

/// Issue #653. Every guard in this file and in `msrv_tests.rs` reasons about
/// jobs, and the workflow's own `on:` block switches all eight off at once,
/// one level above every one of them. Narrowing `branches:` to `[main]` stops
/// the whole of CI on a pull request targeting `dev`, which is where every
/// feature branch lands, and leaves all 26 tests green.
///
/// `paths:` is the same hole in a subtler shape, and the reason it is asserted
/// absent rather than merely checked: `paths: ['src/**']` would skip CI on a
/// pull request touching only `tests/`, which is exactly the shape of this
/// very change. It is `needs:` one level up.
///
/// The key is read as the string `"on"`. YAML 1.1 would resolve a bare `on`
/// to the boolean `true`, which is the classic trap for anything parsing
/// Actions workflows, but serde_yaml_ng implements YAML 1.2, where it stays a
/// string. Checked against this file rather than assumed, and stated here so
/// nobody "fixes" it into `workflow[true]`.
#[test]
fn ci_fires_on_main_and_dev_with_nothing_filtered_out() {
  let on = &ci_workflow()["on"];
  assert!(
    !on.is_null(),
    "ci.yml must declare an `on:` block: without one the workflow never runs and every \
     job-level guard in this file passes over a workflow nobody triggers"
  );

  for event in ["push", "pull_request"] {
    let branches: Vec<String> = on[event]["branches"]
      .as_sequence()
      .cloned()
      .unwrap_or_default()
      .iter()
      .filter_map(|v| v.as_str().map(str::to_owned))
      .collect();
    // Membership is not enough: GitHub reads these as patterns, and a later
    // `!dev` excludes what an earlier `dev` included. `[main, dev, '!dev']`
    // passes every membership test above while no pull request to `dev` runs
    // any CI at all.
    for pattern in &branches {
      assert!(
        !pattern.starts_with('!'),
        "ci.yml must not carry a negative branch pattern on `{event}`: `{pattern}` excludes \
         what an earlier entry includes, so the list still reads as if it covered that \
         branch while nothing fires on it. Got {branches:?}"
      );
    }

    for branch in ["main", "dev"] {
      assert!(
        branches.iter().any(|b| b == branch),
        "ci.yml must fire on `{event}` for `{branch}` (branches are {branches:?}). `dev` is \
         where every feature branch lands and `main` is what releases are cut from, so \
         dropping either stops all eight jobs on that path while every job-level guard \
         stays green"
      );
    }
    // `types:` belongs beside the path filters and is the sharpest of the
    // three, because it has defaults (`opened`, `synchronize`, `reopened`):
    // `types: [labeled]` stops CI on every ordinary pull request while the
    // event and its branches still read exactly as they do now.
    //
    // Each carries its own reason. A shared message would describe a path
    // filter while firing on `types:`, which is an assertion lying about what
    // it checks.
    for (filter, why) in [
      (
        "paths",
        "a path filter skips the whole workflow on a change it does not match, so a pull \
         request touching only `tests/` would run no CI at all",
      ),
      (
        "paths-ignore",
        "an ignore filter skips the whole workflow on a change it does match, which is the \
         same hole written the other way round",
      ),
      (
        "types",
        "restricting the activity types replaces the defaults (`opened`, `synchronize`, \
         `reopened`), so something like `types: [labeled]` runs no CI on an ordinary pull \
         request while the branches above still read as they do now",
      ),
    ] {
      assert!(
        on[event][filter].is_null(),
        "ci.yml must not filter `{event}` by `{filter}`: {why}. Got `{filter}: {:?}`",
        on[event][filter]
      );
    }
  }

  assert!(
    on.as_mapping().is_some_and(|m| m.contains_key("workflow_dispatch")),
    "ci.yml must keep `workflow_dispatch:`: it is the only way to re-run the suite without \
     pushing a commit"
  );
}

/// Issue #655. `assert_job_is_blocking` had four callers for eight jobs, and
/// the four were the ones #646's own audit happened to name. `fmt` and
/// `clippy` are two of the seven contexts `main` requires and neither was
/// among them: `grep -rn '"fmt"\|"clippy"' tests/` returned nothing for the
/// jobs. `hook-smoke` is a required context too and was equally unnamed.
///
/// So the caller list is replaced by a sweep. A list is what produced this
/// issue: #646 guarded what it had looked at, #652 and #653 widened the
/// helper without widening its callers, and a ninth job added tomorrow would
/// arrive unguarded exactly the same way. Walking `jobs:` instead means a new
/// job is covered the moment it exists, and switching one off is a diff
/// against this test rather than against nothing.
///
/// `doctor` is the one exemption, and it is pinned rather than waived. It is
/// advisory by design: `continue-on-error: true` on the `gwm doctor` step and
/// an `if:` restricting the job to `dev`, both deliberate (the report wants
/// eyes, not a blocked merge, and `lazygit` is absent on the runner so a
/// Warning is its floor). What is asserted here is that it is *still* that
/// job. Should it ever lose either property it stops being advisory, the
/// assertion below fails, and it has to move into the guarded set rather than
/// sit in an exemption written for a job it no longer is.
///
/// The three per-job callers elsewhere in this file and in `msrv_tests` stay:
/// each carries a rationale and properties this sweep cannot express (the
/// `test` job's one legitimate doctest `if:`, `audit`'s `--deny warnings`).
/// They overlap with the sweep on purpose. Redundant coverage costs a
/// millisecond; a gap costs nine months, which is what RUSTSEC-2025-0068 did.
#[test]
fn ci_every_job_is_blocking_except_the_advisory_doctor() {
  let workflow = ci_workflow();
  let jobs: Vec<String> = workflow["jobs"]
    .as_mapping()
    .expect("ci.yml must define a `jobs:` mapping")
    .keys()
    .filter_map(|k| k.as_str().map(str::to_owned))
    .collect();

  // A sweep guards the jobs it finds and says nothing about the ones that
  // stopped existing. An emptied `jobs:` leaves it iterating over nothing
  // while reporting success, and a count-based floor does not close that
  // either: deleting one job while adding another satisfies any count. So the
  // eight are named.
  //
  // This is an enumeration, and deliberately so, because it is a **bounded**
  // one. It covers the jobs `ci.yml` ships today; the sweep below covers the
  // ones it does not, which is the exact inverse of the caller list this test
  // replaces, a list that could only ever cover what someone remembered to add
  // to it. Membership is a floor and never an equality: a ninth job has to be
  // a green test that the sweep then guards, not a red one.
  for expected in [
    "fmt",
    "clippy",
    "msrv",
    "test",
    "bench",
    "hook-smoke",
    "audit",
    "doctor",
  ] {
    assert!(
      jobs.iter().any(|j| j == expected),
      "ci.yml must still define the `{expected}` job. Deleting or renaming it is invisible to \
       the sweep below, which guards whatever jobs it finds, and a job going quiet is not \
       always a blocked merge either: only five of the eight are required contexts on `main`, \
       so `msrv`, `bench` and `doctor` can vanish with nothing on GitHub's side objecting. \
       Got {jobs:?}"
    );
  }

  for job_name in &jobs {
    if job_name == "doctor" {
      let job = &workflow["jobs"]["doctor"];
      assert!(
        !job["if"].is_null(),
        "the `doctor` job is exempt from the blocking guard because it is advisory, and it \
         has lost the `if:` that restricts it to `dev`. It is no longer the job this \
         exemption was written for: guard it like the rest, or restore the condition"
      );
      let advisory = job["steps"]
        .as_sequence()
        .cloned()
        .unwrap_or_default()
        .iter()
        .any(|s| s["continue-on-error"].as_bool() == Some(true));
      assert!(
        advisory,
        "the `doctor` job is exempt from the blocking guard because it is advisory, and no \
         step of it carries `continue-on-error: true` any more. A job that can turn the \
         workflow red does not belong in an exemption for one that cannot"
      );
      continue;
    }
    assert_job_is_blocking(&workflow, job_name, steps_allowed_an_if(job_name));
  }
}

/// The `if:` conditions the sweep above allows, by job. Everything not listed
/// gets `&[]`, which is the helper refusing every step-level `if:`.
///
/// One entry today: doctests behave identically on the three runners and are
/// paid for once, on the row with the slack against windows. The waiver
/// carries the condition by value, so widening it to `false` is caught here
/// and not left to whichever other test happens to pin that step.
fn steps_allowed_an_if(job_name: &str) -> &'static [(&'static str, &'static str)] {
  match job_name {
    "test" => &[("cargo test --doc", "matrix.os == 'ubuntu-latest'")],
    _ => &[],
  }
}

/// Issue #655. Being blocking is not enough for `fmt`: `cargo fmt --all`
/// without `--check` **rewrites the tree and exits 0**. It is one bare cargo
/// invocation on one line under a built-in shell, so it satisfies every
/// assertion in `assert_job_is_blocking` including the bare-command
/// invariant, and it is not `--no-run` or `--dry-run` either. The job goes
/// green forever while formatting drift lands, on a runner whose working tree
/// is thrown away a minute later.
///
/// That is the same class as `cargo bench --no-run` (#634) and `cargo audit`
/// without `--deny warnings` (#340): a flag, not a shell operator, is what
/// separates a command that enforces from one that reports. The shape of the
/// command cannot see it, so it is asserted here, where `CLAUDE.md` states
/// it: "CI enforces `cargo fmt --check`".
#[test]
fn ci_fmt_job_checks_formatting_rather_than_rewriting_it() {
  let runs = run_steps(&ci_job("fmt"));
  let fmt = runs
    .iter()
    .find(|r| r.contains("cargo fmt"))
    .unwrap_or_else(|| panic!("the fmt job must run `cargo fmt`, got {runs:?}"));

  assert!(
    fmt.contains("--check"),
    "the fmt job must run `cargo fmt` with `--check`: without it cargo rewrites the tree in \
     place and exits 0, so the job reports success over formatting it silently fixed on a \
     runner and threw away. Got {fmt:?}"
  );
  assert!(
    fmt.contains("--all"),
    "the fmt job must check every crate in the workspace (`--all`): a single-package check \
     leaves the rest unformatted while the job name still reads `rustfmt`. Got {fmt:?}"
  );
}

/// Issue #655. The same hole one job over. `cargo clippy --all-targets` with
/// `-D warnings` dropped exits 0 on every lint it finds, and `cargo clippy -D
/// warnings` without `--all-targets` never lints `tests/`, `benches/` or
/// `examples/` at all, which in this repo is 110 test binaries and three
/// benches, the larger half of the code. Both are one bare cargo invocation
/// and both pass `assert_job_is_blocking` unchanged.
///
/// Neither flag is incidental: `CLAUDE.md` states the command as `cargo
/// clippy --all-targets -- -D warnings` and the house rule that an
/// `#[allow(...)]` needs a comment only means anything while the lint would
/// otherwise have failed the build.
///
/// The workflow-level `RUSTFLAGS: -D warnings` is not a second line of
/// defence to lean on here. It is set once at the top of `ci.yml` for every
/// job, so it is one edit away from being gone for all eight of them, and the
/// `msrv` job already overrides it to `""` at job level, which is precedent
/// that it does get overridden. The command has to carry its own denial.
#[test]
fn ci_clippy_job_denies_warnings_across_all_targets() {
  let runs = run_steps(&ci_job("clippy"));
  let clippy = runs
    .iter()
    .find(|r| r.contains("cargo clippy"))
    .unwrap_or_else(|| panic!("the clippy job must run `cargo clippy`, got {runs:?}"));

  assert!(
    clippy.contains("-D warnings"),
    "the clippy job must pass `-D warnings`: clippy exits 0 on a lint it only warns about, \
     so dropping the denial leaves the job green over every lint in the tree. Got {clippy:?}"
  );
  assert!(
    clippy.contains("--all-targets"),
    "the clippy job must lint every target (`--all-targets`): the default leaves `tests/`, \
     `benches/` and `examples/` unlinted, which here is 110 test binaries and three benches \
     the job would report clean without having read. Got {clippy:?}"
  );
}
