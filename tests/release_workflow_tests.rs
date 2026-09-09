use std::fs;
#[cfg(unix)]
use std::{path::Path, process::Command};

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
fn ci_job(name: &str) -> serde_yaml_ng::Value {
  let workflow: serde_yaml_ng::Value =
    serde_yaml_ng::from_str(&fs::read_to_string(".github/workflows/ci.yml").unwrap())
      .expect("ci.yml must be valid YAML");
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

#[test]
fn ci_test_matrix_runs_on_windows_latest() {
  let job = ci_job("test");
  let matrix: Vec<String> = job["strategy"]["matrix"]["os"]
    .as_sequence()
    .cloned()
    .unwrap_or_default()
    .iter()
    .filter_map(|v| v.as_str().map(str::to_owned))
    .collect();

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
  assert!(
    job["continue-on-error"].is_null(),
    "the bench job must be able to fail the workflow: a dead bench is what #634 is about"
  );
  let steps = job["steps"].as_sequence().cloned().unwrap_or_default();
  assert!(
    steps.iter().all(|s| s["continue-on-error"].is_null()),
    "no step of the bench job may swallow its own failure"
  );
  // The `doctor` job in this same file is narrowed with an `if:`. Doing that
  // here would keep all the assertions above green while the benches quietly
  // stop running on pull requests, which is the state #634 exists to close.
  assert!(
    job["if"].is_null(),
    "the bench job must not be narrowed with an `if:`: it has to run on every event this \
     workflow runs on, or a dead bench goes unseen again"
  );
  assert!(
    steps.iter().all(|s| s["if"].is_null()),
    "no step of the bench job may be conditioned away"
  );
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

/// The doc-comment "documents" of one source file: each contiguous run of
/// `///` or `//!` lines, and each `/** */` or `/*! */` block, as the lines
/// rustdoc would hand to its markdown parser.
///
/// Per document, not per file, because rustdoc parses each one as its own
/// markdown: an unbalanced fence in one cannot invert opener and closer in
/// the next.
fn doc_documents(text: &str) -> Vec<(usize, Vec<String>)> {
  let lines: Vec<&str> = text.lines().collect();
  let mut docs: Vec<(usize, Vec<String>)> = Vec::new();
  let mut i = 0;
  while i < lines.len() {
    let t = lines[i].trim_start();
    if let Some(rest) = t.strip_prefix("///").or_else(|| t.strip_prefix("//!")) {
      let start = i + 1;
      let mut body = vec![rest.to_string()];
      i += 1;
      while i < lines.len() {
        let t = lines[i].trim_start();
        let Some(rest) = t.strip_prefix("///").or_else(|| t.strip_prefix("//!")) else {
          break;
        };
        body.push(rest.to_string());
        i += 1;
      }
      docs.push((start, body));
    } else if t.starts_with("/**") || t.starts_with("/*!") {
      let start = i + 1;
      let mut body = Vec::new();
      let mut first = t[3..].to_string();
      let mut closed = false;
      if let Some(cut) = first.find("*/") {
        first.truncate(cut);
        closed = true;
      }
      body.push(first);
      i += 1;
      while !closed && i < lines.len() {
        let mut l = lines[i].to_string();
        if let Some(cut) = l.find("*/") {
          l.truncate(cut);
          closed = true;
        }
        // rustdoc strips the decorative leading `*` of the `/** */` style.
        let stripped = l.trim_start();
        body.push(match stripped.strip_prefix('*') {
          Some(r) => r.to_string(),
          None => l,
        });
        i += 1;
      }
      docs.push((start, body));
    } else {
      i += 1;
    }
  }
  docs
}

/// Every fence opener inside a doc comment under `src/`, as
/// `(file, line, info string)`.
///
/// Openers only: the closing fence of an open block is not one, which is why
/// this tracks state instead of grepping. Both fence characters are handled
/// (rustdoc's markdown accepts `~~~` as readily as backticks) and a closer
/// has to match the opener's character and be at least as long, so a four
/// backtick block quoting a three backtick one closes where it should.
fn doc_comment_fence_openers() -> Vec<(String, usize, String)> {
  let mut stack = vec![std::path::PathBuf::from("src")];
  let mut files = Vec::new();
  while let Some(d) = stack.pop() {
    for entry in fs::read_dir(&d).unwrap_or_else(|e| panic!("cannot list {}: {e}", d.display())) {
      let path = entry.unwrap().path();
      if path.is_dir() {
        stack.push(path);
      } else if path.extension().is_some_and(|e| e == "rs") {
        files.push(path);
      }
    }
  }
  files.sort();

  fn fence(line: &str) -> Option<(char, usize, String)> {
    let t = line.trim_start();
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = t.chars().take_while(|x| *x == c).count();
    (len >= 3).then(|| (c, len, t[len..].trim().to_string()))
  }

  let mut openers = Vec::new();
  for file in files {
    // CRLF normalised: a Windows checkout stores these with `\r\n`, and the
    // info string would carry a trailing `\r` into the classifier.
    let text = fs::read_to_string(&file)
      .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()))
      .replace("\r\n", "\n");
    for (start, body) in doc_documents(&text) {
      let mut open: Option<(char, usize)> = None;
      for (offset, line) in body.iter().enumerate() {
        let Some((c, len, info)) = fence(line) else {
          continue;
        };
        match open {
          Some((oc, olen)) if oc == c && len >= olen && info.is_empty() => open = None,
          Some(_) => {}
          None => {
            open = Some((c, len));
            openers.push((file.display().to_string(), start + offset, info));
          }
        }
      }
    }
  }
  openers
}

/// Does rustdoc compile this fence's body as a doctest?
///
/// Its rule is `rust &= !seen_other_tags || seen_rust_tags`, starting from
/// `rust = true`: an empty info string is Rust, and **any** recognised Rust
/// token keeps it Rust no matter what else sits beside it. Writing that as
/// "every token must be recognised" is the mistake this function was fixed
/// out of, and it is not academic. Probed against the local toolchain, all
/// three of ```` ```rust,zzz_unknown ````, ```` ```edition2024 ```` and
/// ```` ```compile_fail,E0308 ```` compile and run, and an `all`-based
/// filter calls all three non-Rust.
///
/// `edition` and `ignore-` are prefix matches in rustdoc, not literals, so
/// they are prefixes here too: an `edition2027` list entry would otherwise
/// have to be added the year it appears, which is exactly the maintenance a
/// derived guard exists to avoid.
fn is_rust_doctest_fence(info: &str) -> bool {
  const RUST_TOKENS: [&str; 7] = [
    "rust",
    "ignore",
    "should_panic",
    "no_run",
    "compile_fail",
    "test_harness",
    "standalone_crate",
  ];
  let info = info.trim();
  if info.is_empty() {
    return true;
  }
  info
    .split([',', ' ', '\t'])
    .map(str::trim)
    .filter(|t| !t.is_empty())
    .any(|t| RUST_TOKENS.contains(&t) || t.starts_with("edition") || t.starts_with("ignore-"))
}

/// `cargo nextest run` cannot run doctests: they have no test binary for it
/// to schedule. #634 took `cargo test` out of the `test` job, and no other
/// workflow runs `cargo test --doc`, so from that commit on the repo's
/// doctest coverage stopped being a property of CI. It is a property of
/// `src/`: no doc comment carries a Rust fence, therefore nothing is lost.
/// The first one anyone writes is silently never compiled and never run,
/// which is the same shape of rot as the bench that panicked for 1086
/// commits with nothing to report it.
///
/// The swap shipped with that stated in a comment, and the comment had the
/// enumeration wrong: it said every fence was ```text or ```go and missed
/// four ```toml ones. So this derives the fact rather than asserting a list.
///
/// If it fails, either tag the new fence with a non-Rust language, or add a
/// `cargo test --doc` step to `ci.yml` next to the nextest one and this goes
/// green on its own: it reads that file's parsed `run:` scripts.
#[test]
fn no_doctest_under_src_while_ci_cannot_run_doctests() {
  // Read from the parsed `run:` scripts, never from the file text. The job
  // comment next to the nextest step *names* `cargo test --doc` as the thing
  // to add if a doctest ever appears, so a substring search over `ci.yml`
  // matches that sentence and the guard never fires. It was written that way
  // first, and it stayed green with a bare ``` fence planted under `src/`.
  let workflow: serde_yaml_ng::Value =
    serde_yaml_ng::from_str(&fs::read_to_string(".github/workflows/ci.yml").unwrap())
      .expect("ci.yml must be valid YAML");
  let runs_doctests = ["test", "bench", "msrv"]
    .iter()
    .filter_map(|name| {
      let job = workflow["jobs"][*name].clone();
      (!job.is_null()).then(|| run_steps(&job))
    })
    .flatten()
    .any(|r| r.contains("cargo test --doc"));

  let openers = doc_comment_fence_openers();
  // Not decoration: an empty scan (a moved `src/`, a broken walker) would
  // make every filter below vacuously true and this guard green on nothing.
  assert!(
    !openers.is_empty(),
    "the scan found no fenced block at all under src/, which is a broken scan, not a clean tree"
  );

  let eligible: Vec<String> = openers
    .into_iter()
    .filter(|(_, _, info)| is_rust_doctest_fence(info))
    .map(|(f, line, info)| format!("{f}:{line} (fence info {info:?})"))
    .collect();

  assert!(
    runs_doctests || eligible.is_empty(),
    "these doc-comment fences are compiled as Rust doctests, and nothing in CI runs them \
     since `cargo nextest run` replaced `cargo test`: {eligible:?}. Either tag them with a \
     non-Rust language, or add a `cargo test --doc` step to ci.yml."
  );
}

/// The classifier, pinned against what the toolchain actually does.
///
/// Every case here was run through `cargo test --doc` on a scratch crate
/// before being written down; the three in the first group are the ones an
/// `all`-based filter got wrong.
#[test]
fn rust_doctest_fence_classification_matches_rustdoc() {
  for info in [
    "",
    "rust",
    "rust,zzz_unknown",
    "edition2024",
    "edition2021",
    "compile_fail,E0308",
    "should_panic",
    "no_run",
    "ignore",
    "ignore-windows",
  ] {
    assert!(
      is_rust_doctest_fence(info),
      "rustdoc runs ```{info} as a doctest, the guard must see it"
    );
  }
  for info in ["text", "toml", "go", "json", "bash", "console", "text,toml"] {
    assert!(
      !is_rust_doctest_fence(info),
      "rustdoc does not run ```{info}, flagging it would make the guard cry wolf"
    );
  }
}
