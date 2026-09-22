use std::fs;
#[cfg(unix)]
use std::{path::Path, process::Command};

mod common;
use common::{assert_job_is_blocking, effective_matrix_os, job_needs, step_label, string_keys, workflow_step};

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

/// The stable publish step, as written. `edit` runs when the release already
/// exists (a recovery rerun), `create` on a fresh tag, which is to say on the
/// actual release; `upload` attaches every artifact the build produced.
const PUBLISH_RELEASE_SCRIPT: &str = r##"set -euo pipefail

if gh release view "$TAG" --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then
  gh release edit "$TAG" \
    --repo "$GITHUB_REPOSITORY" \
    --title "$TAG" \
    --notes-file "${{ steps.changelog.outputs.path }}" \
    --draft=false \
    --prerelease=false
else
  gh release create "$TAG" \
    --repo "$GITHUB_REPOSITORY" \
    --title "$TAG" \
    --notes-file "${{ steps.changelog.outputs.path }}" \
    --verify-tag \
    --draft=false \
    --prerelease=false
fi

gh release upload "$TAG" \
  --repo "$GITHUB_REPOSITORY" \
  --clobber \
  dist/*.tar.gz \
  dist/*.tar.gz.sha256 \
  dist/*.zip \
  dist/*.zip.sha256 \
  dist/*.deb \
  dist/*.deb.sha256 \
  dist/*.rpm \
  dist/*.rpm.sha256
"##;

/// The `resolve changelog path` step of each release workflow, whose output
/// the publish step reads. Pinning `--notes-file "${{ steps.changelog.outputs.path }}"`
/// alone pins the reference and not what it resolves to: with
/// `CHANGELOG_PATH="CHANGELOG.md"` here, the empty index goes out as the notes
/// and every other guard stays green, which is the v0.6.0 incident itself.
/// The dash in the error message is spelled `\u{2014}` so this file carries
/// none while still matching the workflow byte for byte.
const RESOLVE_STABLE_CHANGELOG: &str = concat!(
  r##"TAG="${{ github.ref_name }}"
VERSION="${TAG#v}"
CHANGELOG_PATH="changelogs/${VERSION}.md"
if [ ! -f "${CHANGELOG_PATH}" ]; then
  echo "::error::Expected ${CHANGELOG_PATH} to exist for tag ${TAG} "##,
  "\u{2014}",
  r##" release notes would otherwise fall back to the empty CHANGELOG.md index."
  exit 1
fi
echo "path=${CHANGELOG_PATH}" >> "$GITHUB_OUTPUT"
"##
);

const RESOLVE_RC_CHANGELOG: &str = concat!(
  r##"TAG="${{ steps.tag.outputs.name }}"
VERSION="${TAG#v}"
CHANGELOG_PATH="changelogs/pre-releases/${VERSION}.md"
if [ ! -f "${CHANGELOG_PATH}" ]; then
  echo "::error::Expected ${CHANGELOG_PATH} to exist for tag ${TAG} "##,
  "\u{2014}",
  r##" release notes would otherwise fall back to the empty CHANGELOG.md index."
  exit 1
fi
echo "path=${CHANGELOG_PATH}" >> "$GITHUB_OUTPUT"
"##
);

/// The `if:` both stable jobs carry, on one line. `release.yml` writes it as a
/// block scalar across three.
const STABLE_TAGS_ONLY: &str = "!contains(github.event.inputs.tag || github.ref_name, '-rc.') && \
                                !contains(github.event.inputs.tag || github.ref_name, '-alpha.') && \
                                !contains(github.event.inputs.tag || github.ref_name, '-beta.')";

/// The stable publish job, every key but `if:` and every step in order (issue
/// #665). The `run:` of the two steps pinned above and the publish step's
/// `env:` are filled in from what the test already pins, so each is written
/// once. An action is named without its `@ref`, see `assert_job_as_written`.
const STABLE_RELEASE_JOB: &str = r##"
name: github release
needs: [build]
runs-on: ubuntu-latest
steps:
  - uses: actions/checkout
    with:
      persist-credentials: false
  - name: download all artifacts
    uses: actions/download-artifact
    with:
      path: dist
      merge-multiple: true
  - name: resolve changelog path
    id: changelog
    shell: bash
  - name: publish release
    shell: bash
"##;

/// The pre-release publish job, same convention: the resolver's `run:` and the
/// publish step's `with:` come from the test, and `resolve tag` is written out
/// here because nothing else pins it, although `tag_name` and `name` read its
/// output.
const PRE_RELEASE_JOB: &str = r##"
name: github pre-release
needs: [build]
runs-on: ubuntu-latest
steps:
  - name: resolve tag
    id: tag
    shell: bash
    run: |
      if [ -n "${{ inputs.tag }}" ]; then
        echo "name=${{ inputs.tag }}" >> "$GITHUB_OUTPUT"
      else
        echo "name=${{ github.ref_name }}" >> "$GITHUB_OUTPUT"
      fi
  - uses: actions/checkout
    with:
      ref: ${{ steps.tag.outputs.name }}
      persist-credentials: false
  - name: download all artifacts
    uses: actions/download-artifact
    with:
      path: dist
      merge-multiple: true
  - name: resolve changelog path
    id: changelog
    shell: bash
  - name: check unreleased changelog against previous rc
    shell: bash
    run: ./.github/scripts/check-rc-changelog-dupes.sh "${{ steps.tag.outputs.name }}"
  - name: publish pre-release
    uses: softprops/action-gh-release
"##;

/// Issue #665. The steps pinned by name leave the rest of the publish job
/// free-form, and review measured two ways to empty the notes with every one
/// of those pins green: `gh release edit "$TAG" --notes ""` added after the
/// publish, and the notes file truncated by a step inserted between the
/// resolver and the publish. A line appended to `$GITHUB_ENV` by any earlier
/// step is the same shape, since it sets the environment of every step after
/// it. So the whole job is pinned: its keys, then the list of steps in order,
/// then each step by value.
///
/// The keys, because a step is not the only thing that reaches a `run:`. A
/// `container:` with an `env:` runs every step through `docker exec` inside it,
/// `SHELLOPTS: noexec` included, and `defaults:` changes where and how they
/// run. Review measured the first with every step pinned and the suite green.
/// The one key compared elsewhere is `if:`, which `assert_publish_job_blocks`
/// reads with its whitespace collapsed, since the stable job writes it as a
/// block scalar.
///
/// The labels are compared first so that an added, removed or reordered step
/// reads as a list in the message, not as two unrelated steps compared at the
/// same index. Ordered, because a step's output is unset for the steps above
/// it, and a set would accept the resolver moved below its reader.
///
/// The `@ref` of an action is left free: Dependabot bumps `github-actions` on
/// this repo, and pinning the ref would turn each of its pull requests red for
/// a change it exists to make. The action's name is pinned, and so is
/// everything passed to it.
///
/// What this leaves out is what the actions and the scripts do inside. The
/// step is pinned, not the file it calls: `check-rc-changelog-dupes.sh` runs
/// between the resolver and the publish, and its own tests pin what it
/// detects, not what else it does, so a line added to it can still truncate
/// the rc notes. And the rest of the workflow: the jobs after the publish
/// hold a read-only token since #669
/// (`release_workflow_grants_write_only_to_build_and_publish`), but the PATs
/// they push with are scoped to the tap and the bucket only by what
/// CONTRIBUTING.md says to create, and the other workflows of this repository
/// are read by nothing here. A reader of one job cannot close that, the
/// ceiling #656 names.
///
/// `ci.yml`'s `flake` job is pinned the same way (#672); `why` carries what
/// each job stands to lose.
fn assert_job_as_written(path: &str, job: &str, expected: &serde_yaml_ng::Value, why: &str) {
  let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
  let workflow: serde_yaml_ng::Value =
    serde_yaml_ng::from_str(&text).unwrap_or_else(|e| panic!("{path} must be valid YAML: {e}"));
  let outside_steps = |job: &serde_yaml_ng::Value| {
    let mut keys = job.as_mapping().cloned().unwrap_or_default();
    keys.remove("steps");
    keys.remove("if");
    keys
  };
  assert_eq!(
    outside_steps(&workflow["jobs"][job]),
    outside_steps(expected),
    "{path} job `{job}` changed outside its steps. Every key of the job is pinned but `if:`, \
     which its blocking check compares: a `container:` carries its `env:` into every `run:` \
     step, `SHELLOPTS: noexec` included, and `defaults:` changes where and how they run. {why}"
  );
  let actual: Vec<serde_yaml_ng::Value> = workflow["jobs"][job]["steps"]
    .as_sequence()
    .cloned()
    .unwrap_or_default()
    .into_iter()
    .map(|mut step| {
      if let Some(action) = step["uses"]
        .as_str()
        .and_then(|u| u.split_once('@'))
        .map(|(action, _)| action.to_owned())
      {
        step["uses"] = action.into();
      }
      step
    })
    .collect();
  let expected = expected["steps"]
    .as_sequence()
    .expect("the expected job must list its steps");
  let labels = |steps: &[serde_yaml_ng::Value]| steps.iter().map(|s| step_label(s).to_owned()).collect::<Vec<_>>();
  assert_eq!(
    labels(&actual),
    labels(expected),
    "{path} job `{job}` must run exactly these steps, in this order. {why}"
  );
  for (step, want) in actual.iter().zip(expected) {
    assert_eq!(
      step,
      want,
      "{path} job `{job}`: the step {:?} changed. Every step of this job is pinned by value, the \
       action's `@ref` aside. {why}",
      step_label(want)
    );
  }
}

/// What a change anywhere in a publish job stands to lose (issue #665).
const PUBLISH_JOB_WHY: &str = "Issue #665: every step of a publish job runs with the workflow's \
  write token, ahead of the publish or after it, so a step added or changed anywhere in it can \
  empty the release notes with every other guard green: `gh release edit` with empty notes after \
  the publish, the notes file truncated before it, or a line appended to `$GITHUB_ENV`. If the \
  change is intended, update the expected job in the same diff";

/// A `run:` step pinned by value (issue #647): it runs, it can fail its job, it
/// runs under bash, and its script is exactly `script`, as written. `if: false`
/// is a YAML boolean and `continue-on-error: false` is not an absent key, so
/// both are compared to null rather than read through `as_str()`. `shell:`
/// takes a whole command line, and `true {0}` never runs the script.
///
/// The script is compared as the YAML parser hands it over, indentation of the
/// block already removed, with nothing normalised on top. The two versions of
/// this guard before it joined continuation lines themselves, and review found
/// two places where that join and bash disagree: `dist/*.rpm \ ` with a trailing
/// space ends the command in bash and was joined here, and `"$TAG"\` followed
/// by `--notes-file` glues the two into one word in bash and was split here:
/// that word becomes the value of `--title`, and the notes path after it a
/// positional argument, uploaded as an asset instead of read as the notes. Each is a model
/// of bash written in place of bash, the mistake #634 already paid for. As
/// written, the only cost is that reformatting the step reformats the pin.
fn assert_run_step(step: &serde_yaml_ng::Value, label: &str, env: &serde_yaml_ng::Value, script: &str, why: &str) {
  // The environment is part of what a script does, so it is pinned with it,
  // for every step pinned here rather than wherever someone remembers to:
  // `SHELLOPTS: noexec` has bash parse the script, run nothing and exit 0,
  // leaving `path=` unwritten. Review found the resolver steps unpinned while
  // a comment said every step was, which is why this is a parameter and not a
  // call site.
  assert_eq!(
    step["env"], *env,
    "the {label} step's `env:` changed. It is pinned by value (issue #647): a variable there \
     changes what the script does without touching it"
  );
  assert!(
    step["if"].is_null() && step["continue-on-error"].is_null(),
    "the {label} step must run and be able to fail its job, got `if: {:?}` and \
     `continue-on-error: {:?}`",
    step["if"],
    step["continue-on-error"]
  );
  assert_eq!(
    step["shell"].as_str(),
    Some("bash"),
    "the {label} step must run under bash, got `shell: {:?}`",
    step["shell"]
  );
  assert_eq!(step["run"].as_str(), Some(script), "{why}");
}

/// Issue #647. A step that runs and fails its job says nothing about the job:
/// GitHub applies `if:` and `continue-on-error:` at the job level too, and the
/// job wins (#646). `continue-on-error: true` on the job turns a failed
/// `gh release create` into a green run over a half-published release, and
/// `if: false` on it publishes nothing at all.
///
/// So the job holding the publish step, and every job it waits on, since a
/// skipped dependency skips everything below it, must carry exactly
/// `condition` as its `if:` and no `continue-on-error:`. `assert_job_is_blocking`
/// is not reused because it refuses any job-level `if:`, and the stable jobs
/// carry one on purpose. The condition is compared by value, with whitespace
/// collapsed because the workflow writes it as a block scalar.
///
/// The publish job's own `needs:` is pinned to `needs`, because the walk reads
/// its route from that key: emptied, it walks the publish job alone and passes,
/// while the publish runs beside the build instead of after it and uploads
/// whatever `dist/` holds at that moment.
///
/// `continue-on-error:` is refused on the dependencies too, which is where
/// this parts from `assert_job_is_blocking` on purpose. On a CI dependency it
/// makes the dependency report success so the dependent runs; here the
/// dependent is the publish, and `build` reporting success over a failed
/// build is a release going out without its artifacts.
fn assert_publish_job_blocks(path: &str, job_name: &str, condition: Option<&str>, needs: &[&str]) {
  let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
  let workflow: serde_yaml_ng::Value =
    serde_yaml_ng::from_str(&text).unwrap_or_else(|e| panic!("{path} must be valid YAML: {e}"));
  // The environment reaches a `run:` step from three `env:` levels, and
  // `SHELLOPTS: noexec` at any of them has bash run nothing and exit 0. The
  // step level is pinned through the `env` argument of `assert_run_step`, the
  // job and the workflow levels here, and the `env:` of a job `container:`, a
  // fourth, by `assert_job_as_written`. An action's inputs are not reachable
  // this way: the runner writes `INPUT_<NAME>` for every declared input, the
  // empty string when it has no default, over whatever `env:` set (verified
  // in actions/runner, `ActionManifestManager.cs` and `Handler.cs`).
  //
  // The other steps of the publish job, a step writing to `$GITHUB_ENV`
  // among them, and the job's own keys are pinned by `assert_job_as_written`
  // (issue #665), which also says what stays out of reach.
  let workflow_env: serde_yaml_ng::Value = serde_yaml_ng::from_str("CARGO_TERM_COLOR: always").unwrap();
  assert_eq!(
    workflow["env"], workflow_env,
    "{path} must set nothing in its workflow-level `env:` but `CARGO_TERM_COLOR`: that environment \
     reaches the steps that resolve and publish the notes, and `SHELLOPTS: noexec` there runs \
     nothing and exits 0"
  );
  assert!(
    workflow["jobs"][job_name]["env"].is_null(),
    "{path} job `{job_name}` must carry no `env:`: it reaches the steps that resolve and publish the \
     notes, and `SHELLOPTS: noexec` there runs nothing and exits 0. Got `env: {:?}`",
    workflow["jobs"][job_name]["env"]
  );
  assert_eq!(
    job_needs(&workflow["jobs"][job_name]),
    needs,
    "{path} job `{job_name}` must wait on exactly {needs:?}: the publish has to run after the build \
     it uploads, and the checks below walk the jobs this key names"
  );
  let condition = condition.map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "));
  let mut pending = vec![job_name.to_string()];
  let mut seen: Vec<String> = Vec::new();
  while let Some(name) = pending.pop() {
    if seen.contains(&name) {
      continue;
    }
    let job = &workflow["jobs"][name.as_str()];
    assert!(
      !job.is_null(),
      "{path} must define a `{name}` job: the publish step runs in it or waits on it"
    );
    let actual = job["if"]
      .as_str()
      .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "));
    let runs = match &condition {
      None => job["if"].is_null(),
      Some(c) => actual.as_deref() == Some(c.as_str()),
    };
    assert!(
      runs,
      "{path} job `{name}` must carry `if: {condition:?}` and nothing else, because the publish \
       step runs in it or after it. Got `if: {:?}`",
      job["if"]
    );
    assert!(
      job["continue-on-error"].is_null(),
      "{path} job `{name}` must carry no `continue-on-error:`: a failed publish would end in a green \
       run over a half-published release. Got `continue-on-error: {:?}`",
      job["continue-on-error"]
    );
    pending.extend(job_needs(job));
    seen.push(name);
  }
}

/// Issue #647. The stable publish step is pinned **by value**, script and
/// environment, because every substring check it had was satisfied by
/// something other than what it named.
///
/// `--notes-file` was asserted once over a script holding two exclusive
/// branches, so dropping it from `create`, the branch a real release takes,
/// left the test green and would have published an empty body in place of
/// `changelogs/<version>.md` (gh 2.100.0 sends no body without `--notes`,
/// `--notes-file` or `--generate-notes`, and has no prompt in CI): release
/// notes missing, which is what this test exists to prevent since v0.6.0. `--verify-tag`, `--draft=false`, `--prerelease=false` and the
/// `.tar.gz` / `.zip` uploads were asserted by nothing. And a flag check reads
/// presence, which an addition defeats: `gh` keeps the last value of a
/// repeated flag (measured on gh 2.100.0, `--limit 5 --limit 1` lists one
/// release), so `--draft=false --draft=true` publishes a draft with the
/// substring still there. #655 hit the same shape on `cargo clippy`.
///
/// Comparing the script as written closes all three without listing them.
/// Changing what it says is a change to the release, and belongs in the same
/// diff as this constant.
///
/// The step is only half of the path. `--notes-file` names the output of
/// `resolve changelog path`, which is pinned the same way and must come
/// before the publish step, since an output is unset for the steps above the
/// one that writes it. The `release` job around both, with the `build` job it
/// waits on, must run on stable tags and fail the run when it fails (see
/// `assert_publish_job_blocks`).
#[test]
fn stable_release_publish_uses_github_cli_with_workflow_token() {
  let workflow = fs::read_to_string(".github/workflows/release.yml").unwrap();
  assert!(
    !workflow.contains("uses: softprops/action-gh-release"),
    "release.yml must not use softprops/action-gh-release for the stable GitHub Release publish step"
  );

  let (publish_at, step) = workflow_step(".github/workflows/release.yml", "release", "publish release");
  // `gh` authenticates with the workflow token and publishes the tag that
  // triggered the run, and nothing else sits in its environment.
  let env: serde_yaml_ng::Value =
    serde_yaml_ng::from_str("GH_TOKEN: ${{ github.token }}\nTAG: ${{ github.ref_name }}").unwrap();
  assert_run_step(
    &step,
    "publish release",
    &env,
    PUBLISH_RELEASE_SCRIPT,
    "release.yml's publish release script changed. It is pinned by value (issue #647): the notes \
     must come from `changelogs/<version>.md` in BOTH the `create` and the `edit` branch, `create` \
     must verify the tag and publish neither a draft nor a pre-release, and every artifact kind \
     must be uploaded. If the change is intended, update `PUBLISH_RELEASE_SCRIPT` in the same diff",
  );

  let (resolve_at, resolve) = workflow_step(".github/workflows/release.yml", "release", "resolve changelog path");
  assert!(
    resolve_at < publish_at,
    "the resolve changelog path step must run before publish release: `--notes-file` reads its \
     output, which is empty for every step above it, and `gh release create` then publishes with \
     an empty body"
  );
  assert_eq!(
    resolve["id"].as_str(),
    Some("changelog"),
    "the resolve changelog path step must keep `id: changelog`: `--notes-file` reads \
     `steps.changelog.outputs.path`, which is empty under any other id"
  );
  assert_run_step(
    &resolve,
    "resolve changelog path",
    &serde_yaml_ng::Value::Null,
    RESOLVE_STABLE_CHANGELOG,
    "release.yml's changelog path changed. It is pinned by value (issue #647): the notes must be \
     `changelogs/<version>.md` and the job must fail when that file is missing, never fall back \
     to the `CHANGELOG.md` index",
  );

  let mut job: serde_yaml_ng::Value = serde_yaml_ng::from_str(STABLE_RELEASE_JOB).unwrap();
  job["steps"][2]["run"] = RESOLVE_STABLE_CHANGELOG.into();
  job["steps"][3]["env"] = env;
  job["steps"][3]["run"] = PUBLISH_RELEASE_SCRIPT.into();
  assert_job_as_written(".github/workflows/release.yml", "release", &job, PUBLISH_JOB_WHY);

  assert_publish_job_blocks(
    ".github/workflows/release.yml",
    "release",
    Some(STABLE_TAGS_ONLY),
    &["build"],
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

/// Issue #673. The workflow guards collect mapping keys through
/// `string_keys`, which refuses a key the parser does not read as a string.
/// serde_yaml_ng reads `true:` and `false:` as booleans; GitHub reads them as
/// the job, or the matrix dimension, named `true` or `false`. A guard that
/// skipped them would pass over exactly that job.
#[test]
#[should_panic(expected = "every key must be a string")]
fn string_keys_refuses_a_key_the_parser_reads_as_a_boolean() {
  let jobs: serde_yaml_ng::Value = serde_yaml_ng::from_str(
    "build: {}
true: {}
",
  )
  .unwrap();
  string_keys(jobs.as_mapping().unwrap(), "jobs");
}

#[test]
fn string_keys_returns_every_key_in_order() {
  let jobs: serde_yaml_ng::Value = serde_yaml_ng::from_str(
    "build: {}
release: {}
",
  )
  .unwrap();
  assert_eq!(string_keys(jobs.as_mapping().unwrap(), "jobs"), ["build", "release"]);
}

/// The jobs of `release.yml` allowed to inherit the workflow's `contents:
/// write`: `release` publishes, and `build` runs before it (see
/// `release_workflow_grants_write_only_to_build_and_publish`).
const INHERITS_THE_WRITE_TOKEN: [&str; 2] = ["build", "release"];

/// Issue #669. `release.yml` grants `contents: write` at the workflow level,
/// and every job inherits it unless it says otherwise. `homebrew-tap-update`
/// and `scoop-bucket-update` run after the publish with that token and a
/// `GH_TOKEN: ${{ github.token }}` in one of their steps, so one line added to
/// either, `gh release edit "$TAG" --notes ""`, empties the notes #665 pins on
/// the publish job, with every guard green. Neither needs to write here: its
/// `GITHUB_TOKEN` checks out this repository and downloads a published
/// sidecar, and its push goes to the tap or the bucket through the PAT of its
/// second checkout, which `permissions:` does not govern.
///
/// A job-level `permissions:` replaces the workflow-level one for that job,
/// and every scope it does not name is `none` (workflow syntax,
/// `jobs.<job_id>.permissions`). So every job carries exactly `contents:
/// read`, compared by value, unless it is one of the two named jobs allowed
/// to inherit the write token. By value, because a presence check is
/// satisfied by `write-all`. A sweep, because a job added later with no
/// `permissions:` inherits the write token by default, which is how these
/// two got it: it arrives red here, not unguarded.
///
/// `build` inherits on purpose and is out of this issue's scope. It finishes
/// before the publish starts, so it cannot undo the notes of the tag being
/// released: the publish job writes them after it, from
/// `changelogs/<version>.md`, in both its branches. It can still edit the
/// notes of any earlier release, which nothing rewrites; restricting it is
/// the same one-line change, left out of this issue. `release` keeps its
/// access, and `assert_job_as_written` pins the absence of a job-level
/// `permissions:` there.
///
/// What no reader of this file can check is whether `contents: read` is
/// enough at run time. Both jobs are `continue-on-error: true`, so a missing
/// permission would leave the release green and the tap and the bucket
/// silently stale: the next stable tag is the check.
#[test]
fn release_workflow_grants_write_only_to_build_and_publish() {
  let path = ".github/workflows/release.yml";
  let workflow: serde_yaml_ng::Value =
    serde_yaml_ng::from_str(&fs::read_to_string(path).unwrap()).unwrap_or_else(|e| panic!("{path}: {e}"));
  let write: serde_yaml_ng::Value = serde_yaml_ng::from_str("contents: write").unwrap();
  assert_eq!(
    workflow["permissions"], write,
    "{path} must grant `contents: write` at the workflow level and nothing else: the publish \
     job inherits it, and the jobs below are checked against it"
  );

  // Through `string_keys`: a `true:` job, skipped, would keep the write
  // token with this test green.
  let jobs = string_keys(
    workflow["jobs"]
      .as_mapping()
      .expect("release.yml must define a `jobs:` mapping"),
    &format!("{path} `jobs:`"),
  );
  // The loop below catches a rename, since it holds every job but `build`
  // and `release` to `contents: read` under whatever name. What a sweep does
  // not see is a job that stopped existing, down to a `jobs:` mapping it
  // reads nothing in. So the four are named, as a floor and never an
  // equality; the cost is that a harmless rename goes red too.
  for expected in ["build", "release", "homebrew-tap-update", "scoop-bucket-update"] {
    assert!(
      jobs.iter().any(|j| j == expected),
      "{path} must still define the `{expected}` job, got {jobs:?}"
    );
  }

  let read: serde_yaml_ng::Value = serde_yaml_ng::from_str("contents: read").unwrap();
  for job in jobs.iter().filter(|j| !INHERITS_THE_WRITE_TOKEN.contains(&j.as_str())) {
    assert_eq!(
      workflow["jobs"][job.as_str()]["permissions"],
      read,
      "{path} job `{job}` must carry `permissions: contents: read` (issue #669). Without it the \
       job inherits the workflow's `contents: write`, and one step added to it can edit the \
       release notes after the publish. Only {INHERITS_THE_WRITE_TOKEN:?} may inherit it. \
       Got `permissions: {:?}`",
      workflow["jobs"][job.as_str()]["permissions"]
    );
  }
}

/// Issue #677. `ci.yml` declared no `permissions:` at all, so its jobs took
/// whatever the repository setting handed them:
/// `default_workflow_permissions` reads `read` today, which is a setting no
/// pull request shows and one switch away from `write`. What that would hand a
/// write token to is every build script and proc macro of the dependency
/// graph, `cargo install cargo-audit`, and five third-party actions, one of
/// which, `cachix/install-nix-action`, copies the token into
/// `/etc/nix/nix.conf` (`install-nix.sh` at `v31`, lines 50-52, installed at
/// line 93). The same reasoning closed #669 one workflow over.
///
/// Read, so nothing in `ci.yml` needs more: every job does the same two
/// things, check out and build, and the file names no token of its own. Not
/// that no step holds one, which would be false: `actions/checkout` defaults
/// `token:` to `${{ github.token }}` and the nix action falls back to
/// `GITHUB_TOKEN`, and scoping exactly that default is what the grant is for.
/// What the file naming one would mean is a step doing something with it, the
/// one edit that makes `contents: read` the wrong grant, so that is asserted
/// rather than asserted *about*: a premise left in prose falsifies before the
/// guard does, which this repo has recorded once already (#648). The search
/// reads the whole parsed workflow, and proves on synthetic shapes that it
/// sees the surfaces `ci.yml` does not carry today.
///
/// Job level is read as well as workflow level, because a job-level
/// `permissions:` replaces the workflow's wholesale rather than narrowing it.
/// So a job either leaves it out or restates the same `contents: read`, and
/// anything else is refused without ranking scopes: `contents: write` and a
/// second scope are escalations, and a `{}` that cannot even check out is a
/// job that fails at run time, neither of which belongs here unannounced.
#[test]
fn ci_workflow_grants_a_read_only_token() {
  let path = ".github/workflows/ci.yml";
  let workflow = ci_workflow();
  let read: serde_yaml_ng::Value = serde_yaml_ng::from_str("contents: read").unwrap();
  assert_eq!(
    workflow["permissions"], read,
    "{path} must grant exactly `contents: read` at the workflow level (issue #677). With no \
     `permissions:` at all its jobs inherit the repository default, which is a setting no diff \
     in this repo records: flip it to write and every build script, every `cargo install` and \
     the nix action that writes the token to `/etc/nix/nix.conf` get a token that can push here"
  );

  // Through `string_keys` (issue #673): GitHub reads `true:` as the job named
  // `true` and runs it, while the YAML parser here reads a boolean, so a
  // `filter_map` over string keys is what would skip it, carrying whatever
  // `permissions:` it declares past this loop.
  let jobs = string_keys(
    workflow["jobs"]
      .as_mapping()
      .expect("ci.yml must define a `jobs:` mapping"),
    &format!("{path} `jobs:`"),
  );
  for job in &jobs {
    let declared = &workflow["jobs"][job.as_str()]["permissions"];
    assert!(
      declared.is_null() || *declared == read,
      "{path} job `{job}` must leave `permissions:` out or restate the workflow's \
       `contents: read`, and nothing else (issue #677). A job-level block replaces the \
       workflow's wholesale: `contents: write` or a second scope is the escalation the \
       workflow-level grant exists to prevent, and a `{{}}` cannot check out at all. Either \
       way it is a conscious change that belongs in its own diff, with this test updated. \
       Got `permissions: {declared:?}`"
    );
  }
  assert!(
    jobs.len() >= 9,
    "expected at least the 9 jobs `ci.yml` ships, found {}. The mapping is probably no longer \
     being read, and the loop above would then pass over nothing",
    jobs.len()
  );

  // The premise of the grant, enforced. The **whole** parsed workflow is
  // searched, not a list of the keys a token was thought to arrive through:
  // review measured the first version of this, three keys inside `steps:`, and
  // found `run:` alone carrying it, a `with:` dropped from the list staying
  // green, and every job-level surface unread, `container:`, `services:` and a
  // reusable call's `secrets:` among them. Enumerating where a secret can be
  // written is the shape #652 and #655 already paid for.
  //
  // And the words, not their spellings. `GITHUB_TOKEN`, `github.token`,
  // `github['token']`, `secrets.PAT`, `secrets: inherit` and
  // `toJSON(secrets)` are six ways to write two things, and Actions keeps
  // adding syntax for them: a list of the forms thought of is the four review
  // passes #672 spent learning that text does not bound a language. So the
  // haystack is refused the words `token` and `secret` in any casing, which is
  // a superset of every form. The cost is a legitimate step whose name happens
  // to carry either word going red, and that cost is the point: it is a line a
  // reader should look at twice.
  //
  // Comments are gone by the time the parser is done, so the comment above the
  // grant in `ci.yml` can name what it forbids without matching itself.
  //
  // The ceiling, named because it is one: this reads `ci.yml`, so a token an
  // action consumes inside its own definition is invisible here.
  // `cachix/install-nix-action@v31` is exactly that shape, `GITHUB_TOKEN:
  // ${{ github.token }}` in its own `action.yml`, and a local
  // `uses: ./.github/actions/…` would be too. Scoping the token is what
  // answers that, which is the grant above, not a wider search.
  let flagged = |value: &serde_yaml_ng::Value| -> Vec<&'static str> {
    let text = serde_yaml_ng::to_string(value)
      .expect("a workflow must serialise")
      .to_lowercase();
    ["token", "secret"].into_iter().filter(|w| text.contains(w)).collect()
  };
  assert!(
    flagged(&workflow).is_empty(),
    "{path} reads {:?}, so it is no longer the token-free workflow `contents: read` was granted \
     for (issue #677). Decide what scope that step needs and say it in `permissions:`, rather \
     than leaving the grant to mean something it no longer does",
    flagged(&workflow)
  );

  // What the sweep above is searching, stated rather than assumed: a
  // serialisation that stopped carrying the steps would hold no token either.
  let serialised = serde_yaml_ng::to_string(&workflow).expect("ci.yml must serialise");
  for inside in ["cargo fmt --all -- --check", "cargo nextest run", "install-nix-action"] {
    assert!(
      serialised.contains(inside),
      "the searched serialisation of {path} does not contain `{inside}`, so it is not carrying \
       the steps and the token search above is looking at a shell of the workflow"
    );
  }

  // And that it sees each shape it exists for. These are synthetic, because
  // `ci.yml` carries none of them: the guard is about what a later edit could
  // add, so the proof cannot come from today's file.
  for (shape, yaml) in [
    (
      "a step's `env:`",
      "jobs:\n  probe:\n    steps:\n      - env:\n          GH_TOKEN: x\n",
    ),
    (
      "a step's `with:`",
      "jobs:\n  probe:\n    steps:\n      - with:\n          token: ${{ github.token }}\n",
    ),
    (
      "a step's `run:`",
      "jobs:\n  probe:\n    steps:\n      - run: echo \"$GITHUB_TOKEN\"\n",
    ),
    (
      "a job's `container.env`",
      "jobs:\n  probe:\n    container:\n      image: x\n      env:\n        GITHUB_TOKEN: y\n",
    ),
    (
      "a service's `credentials`",
      "jobs:\n  probe:\n    services:\n      s:\n        credentials:\n          password: ${{ secrets.PAT }}\n",
    ),
    (
      "a reusable call's `secrets: inherit`",
      "jobs:\n  probe:\n    uses: ./.github/workflows/other.yml\n    secrets: inherit\n",
    ),
    ("the workflow's own `env:`", "env:\n  GH_TOKEN: x\njobs: {}\n"),
    (
      "an index expression",
      "jobs:\n  probe:\n    steps:\n      - run: echo ${{ github['token'] }}\n",
    ),
    (
      "a whole context serialised",
      "jobs:\n  probe:\n    steps:\n      - run: echo ${{ toJSON(secrets) }}\n",
    ),
  ] {
    let probe: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).expect("the probe must parse");
    assert!(
      !flagged(&probe).is_empty(),
      "the token search no longer sees {shape}, so `ci.yml` could grow one with this test green"
    );
  }
}

/// Issue #677, the half a single file cannot state: a workflow added later
/// with no `permissions:` inherits the repository default the same way
/// `ci.yml` did, and nothing here would say so. So every workflow declares
/// one, whatever it is: `{}` for `docs-sync.yml`, `contents: read` for
/// `ci.yml`, `contents: write` for the two that publish.
///
/// All four are also compared **by value**, each by its own test:
/// `ci_workflow_grants_a_read_only_token`,
/// `release_workflow_grants_write_only_to_build_and_publish`,
/// `docs_sync_workflow_grants_nothing` and
/// `pre_release_workflow_grants_write_only_to_build_and_publish`. The last two
/// arrived with #681, after review measured that `write-all` on either of those
/// files kept this suite green; the sentence above used to say so, and is
/// reproduced here because it is the reason a fifth workflow needs a by-value
/// test of its own and not just this one.
///
/// This test is deliberately weaker than the by-value ones: it reads that the
/// key exists, not what it says, because what a workflow needs is its own
/// business. What it refuses is the silence. Declaring it on every job instead of at the top is
/// explicit too, and passes: GitHub allows either, and neither leaves a job
/// taking the repository default.
#[test]
fn every_workflow_declares_its_permissions() {
  let mut swept = 0;
  for path in workflow_paths() {
    let workflow: serde_yaml_ng::Value =
      serde_yaml_ng::from_str(&fs::read_to_string(&path).unwrap()).unwrap_or_else(|e| panic!("{path}: {e}"));
    let jobs = string_keys(
      workflow["jobs"]
        .as_mapping()
        .unwrap_or_else(|| panic!("`{path}` must define a `jobs:` mapping")),
      &format!("`{path}` `jobs:`"),
    );
    let every_job_declares = !jobs.is_empty()
      && jobs
        .iter()
        .all(|job| !workflow["jobs"][job.as_str()]["permissions"].is_null());
    assert!(
      !workflow["permissions"].is_null() || every_job_declares,
      "`{path}` declares `permissions:` neither at the workflow level nor on every one of its \
       jobs (issue #677), so a job of it takes whatever `default_workflow_permissions` says, a \
       repository setting no diff in this repo records. Declare what the workflow needs, \
       `permissions: {{}}` if that is nothing"
    );
    swept += 1;
  }
  assert!(
    swept >= 4,
    "expected at least the 4 workflows this repo ships, found {swept}. The directory listing is \
     probably no longer seeing them, and the loop above would then pass over nothing"
  );
}

/// Any workflow by path, parsed. `ci_workflow()` is the `ci.yml`-shaped
/// sibling of this one, kept because most of this file only ever reads that
/// file.
fn workflow_at(path: &str) -> serde_yaml_ng::Value {
  serde_yaml_ng::from_str(&fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}")))
    .unwrap_or_else(|e| panic!("{path} must be valid YAML: {e}"))
}

/// Issue #681. `docs-sync.yml` grants `permissions: {}`, the strongest
/// statement in this repo and the one an edit widens most easily: it is
/// triggered by a push to `main`, and `write-all` there would hand every scope
/// to a workflow whose only job dispatches an API call with a PAT that is not
/// the workflow token.
///
/// Compared as a `Value`, never through `as_str()`: `{}` is an empty mapping
/// and `write-all` is a string, so a reader that asks for a string reads `None`
/// for both and cannot tell the grant from its absence (the #669 lesson, and
/// `string_keys` below for the key side of the same trap).
#[test]
fn docs_sync_workflow_grants_nothing() {
  let path = DOCS_SYNC;
  let workflow = workflow_at(path);
  let nothing: serde_yaml_ng::Value = serde_yaml_ng::from_str("{}").unwrap();
  assert_eq!(
    workflow["permissions"], nothing,
    "{path} must grant `permissions: {{}}` at the workflow level and nothing else (issue #681). \
     It checks nothing out and authenticates its one API call with `secrets.DOCS_SITE_TOKEN`, so \
     the workflow token needs no scope at all. Until this test the grant was read as present and \
     never as what it said, which left `write-all` here passing the whole suite"
  );

  // Through `string_keys` (issue #673): GitHub runs a job keyed `true:`, the
  // parser here reads a boolean, and a `filter_map` over string keys is what
  // would skip it, carrying whatever it declares past this loop.
  let jobs = string_keys(
    workflow["jobs"]
      .as_mapping()
      .unwrap_or_else(|| panic!("{path} must define a `jobs:` mapping")),
    &format!("{path} `jobs:`"),
  );
  for job in &jobs {
    let declared = &workflow["jobs"][job.as_str()]["permissions"];
    assert!(
      declared.is_null() || *declared == nothing,
      "{path} job `{job}` must leave `permissions:` out or restate the workflow's `{{}}`, and \
       nothing else (issue #681). A job-level block replaces the workflow's wholesale, so any \
       scope written here is a grant the workflow level says it does not need. Got \
       `permissions: {declared:?}`"
    );
  }
  assert!(
    !jobs.is_empty(),
    "expected the `notify` job {path} ships, found none. The mapping is probably no longer being \
     read, and the loop above would then pass over nothing"
  );
}

/// Issue #681, the other half. `pre-release.yml` publishes a GitHub Release
/// from an rc/alpha/beta tag, so it grants `contents: write` at the workflow
/// level and both its jobs inherit it. That grant was declared and compared by
/// nothing, which is what #669 closed for `release.yml` and never reached
/// here: `write-all` passed, and so did a third job added after the publish
/// with the write token in hand.
///
/// Held to the same split as `release.yml`: only the jobs in
/// `INHERITS_THE_WRITE_TOKEN` may take the workflow grant, and any other job
/// must carry `contents: read`. There is no such job today, which is precisely
/// why the rule is written now rather than when one appears.
#[test]
fn pre_release_workflow_grants_write_only_to_build_and_publish() {
  let path = ".github/workflows/pre-release.yml";
  let workflow = workflow_at(path);
  let write: serde_yaml_ng::Value = serde_yaml_ng::from_str("contents: write").unwrap();
  assert_eq!(
    workflow["permissions"], write,
    "{path} must grant `contents: write` at the workflow level and nothing else (issue #681): \
     the publish job inherits it to create the pre-release, and nothing here needs a second \
     scope. `write-all` passed this file until this test existed"
  );

  let jobs = string_keys(
    workflow["jobs"]
      .as_mapping()
      .unwrap_or_else(|| panic!("{path} must define a `jobs:` mapping")),
    &format!("{path} `jobs:`"),
  );
  // Named as a floor and never an equality, the #669 reasoning: the loop below
  // holds every job but these two to `contents: read` under whatever name, but
  // a sweep cannot see a job that stopped existing, down to a `jobs:` mapping
  // it reads nothing in. The cost is that a harmless rename goes red too.
  for expected in INHERITS_THE_WRITE_TOKEN {
    assert!(
      jobs.iter().any(|j| j == expected),
      "{path} must still define the `{expected}` job, got {jobs:?}"
    );
  }

  let read: serde_yaml_ng::Value = serde_yaml_ng::from_str("contents: read").unwrap();
  for job in jobs.iter().filter(|j| !INHERITS_THE_WRITE_TOKEN.contains(&j.as_str())) {
    assert_eq!(
      workflow["jobs"][job.as_str()]["permissions"],
      read,
      "{path} job `{job}` must carry `permissions: contents: read` (issue #681). Without it the \
       job inherits the workflow's `contents: write`, and one step added to it can edit the \
       pre-release notes #647 pins after the publish. Only {INHERITS_THE_WRITE_TOKEN:?} may \
       inherit it. Got `permissions: {:?}`",
      workflow["jobs"][job.as_str()]["permissions"]
    );
  }
}

/// Every workflow in the directory, so a file added later is audited by
/// construction rather than by remembering to extend a hand-written list. The
/// sweeps below all enumerate from here: naming files individually is
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
/// `pre-release.yml` publishes through `softprops/action-gh-release`, never
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

/// Issue #647. `pre-release.yml` publishes through `softprops/action-gh-release`,
/// and its `body_path`, the input that makes an rc's notes come from
/// `changelogs/pre-releases/<version>.md`, was covered by no test at all: the
/// guard against the v0.6.0-rc.1 incident (an rc published with the empty
/// `CHANGELOG.md` index as its body) only ever looked at `release.yml`.
///
/// The whole `with:` mapping is pinned by value rather than `body_path` alone.
/// The action takes inputs that change the body without touching that key,
/// `generate_release_notes` and `append_body` among them, and an added key is
/// exactly what a single-key check cannot see. The action is matched on its
/// name and not its version, because dependabot bumps the version.
///
/// `body_path` names a step output, so the step computing it is pinned too,
/// and so is the job, both the same way as on the stable side: a reference
/// pinned to an output that resolves to the index, or a job switched off
/// above a step that looks fine, is the incident again with every key intact.
#[test]
fn pre_release_publish_takes_its_notes_from_the_per_rc_changelog() {
  let (publish_at, step) = workflow_step(".github/workflows/pre-release.yml", "release", "publish pre-release");
  assert!(
    step["uses"]
      .as_str()
      .is_some_and(|u| u.starts_with("softprops/action-gh-release@")),
    "the publish pre-release step must use softprops/action-gh-release, got `uses: {:?}`",
    step["uses"]
  );
  assert!(
    step["if"].is_null() && step["continue-on-error"].is_null(),
    "the publish pre-release step must run and be able to fail the job, got `if: {:?}` and \
     `continue-on-error: {:?}`",
    step["if"],
    step["continue-on-error"]
  );
  let with: serde_yaml_ng::Value = serde_yaml_ng::from_str(
    "tag_name: ${{ steps.tag.outputs.name }}\n\
     name: ${{ steps.tag.outputs.name }}\n\
     body_path: ${{ steps.changelog.outputs.path }}\n\
     files: |\n  dist/*.tar.gz\n  dist/*.tar.gz.sha256\n  dist/*.zip\n  dist/*.zip.sha256\n\
     draft: false\n\
     prerelease: true\n",
  )
  .unwrap();
  assert_eq!(
    step["with"], with,
    "pre-release.yml's publish inputs changed. They are pinned by value (issue #647): the body must \
     come from `changelogs/pre-releases/<version>.md` through `body_path`, with no input that \
     generates or appends notes, and the release must be a published pre-release. If the change \
     is intended, update this test in the same diff"
  );

  let (resolve_at, resolve) = workflow_step(".github/workflows/pre-release.yml", "release", "resolve changelog path");
  assert!(
    resolve_at < publish_at,
    "the resolve changelog path step must run before publish pre-release: `body_path` reads its \
     output, which is empty for every step above it, and softprops then falls back to the unset \
     `body` input and publishes with no notes"
  );
  assert_eq!(
    resolve["id"].as_str(),
    Some("changelog"),
    "the resolve changelog path step must keep `id: changelog`: `body_path` reads \
     `steps.changelog.outputs.path`, which is empty under any other id"
  );
  assert_run_step(
    &resolve,
    "resolve changelog path",
    &serde_yaml_ng::Value::Null,
    RESOLVE_RC_CHANGELOG,
    "pre-release.yml's changelog path changed. It is pinned by value (issue #647): an rc's notes \
     must be `changelogs/pre-releases/<version>.md` and the job must fail when that file is \
     missing, never fall back to the `CHANGELOG.md` index",
  );

  let mut job: serde_yaml_ng::Value = serde_yaml_ng::from_str(PRE_RELEASE_JOB).unwrap();
  job["steps"][3]["run"] = RESOLVE_RC_CHANGELOG.into();
  job["steps"][5]["with"] = with;
  assert_job_as_written(".github/workflows/pre-release.yml", "release", &job, PUBLISH_JOB_WHY);

  assert_publish_job_blocks(".github/workflows/pre-release.yml", "release", None, &["build"]);
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
  assert_job_is_blocking(&ci_workflow(), "bench");
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

/// Issue #659. The crate has no doctests, and says so in its manifest rather
/// than keeping a CI step with nothing to run.
///
/// #634 moved the suite onto nextest, which cannot run doctests, and added a
/// `cargo test --doc` step next to it, pinned by a guard that asserted the step
/// was present and unconditioned. That guard never asserted the step ran
/// anything, and it ran nothing: every fenced block in the doc comments is
/// `text`, `toml` or `go`, and `cargo test --doc -- --list` answers `0 tests`.
/// A step that cannot fail, held in place by a test that could not notice.
///
/// Writing doctests to give the step a subject was the other way out, and it
/// does not fit this lib. `src/lib.rs` is `#![doc(hidden)]`, an internal test
/// seam with no SemVer guarantee that tells readers not to build on it, so an
/// example on it documents an API nobody is meant to call, and behaviour is
/// tested under `tests/` already. A guard failing on a zero count is out of
/// reach too: #652 makes every cargo step one bare invocation, so there is no
/// shell to count with, and re-running cargo from inside the suite would cost
/// roughly 35 seconds on each of the three runners to guard a subject that
/// does not exist.
///
/// Both halves are pinned because neither one holds the decision alone.
/// `doctest = false` only turns doctests off for a plain `cargo test`: an
/// explicit `cargo test --doc` still compiles and runs them (measured on cargo
/// 1.97.0, a scratch crate with the key set and a failing doctest exits 101).
/// So restoring the step alone brings the empty step back, and dropping the
/// key alone has `cargo test` run doctests locally that no CI job runs.
/// Anyone who wants doctests flips the key, writes them and restores the step,
/// and this test says so when they touch either half.
///
/// What this cannot see is a ```` ```rust ```` fence written under the
/// declaration, which nothing compiles. Deciding from the source which fences
/// rustdoc would run is the scanner #634 wrote and threw away after it was
/// found wrong ten ways, so the manifest is what carries that rule, not a
/// model of rustdoc.
#[test]
fn doctests_are_declared_off_rather_than_run_empty() {
  let manifest: toml::Value =
    toml::from_str(&fs::read_to_string("Cargo.toml").unwrap()).expect("Cargo.toml must parse");
  // By value: `doctest = true` must fail like an absent key does, and a
  // lookup that stops at "is there a key" would pass it. A string such as
  // `"false"` never reaches this line, cargo refuses the manifest first.
  let doctest = manifest.get("lib").and_then(|lib| lib.get("doctest"));
  assert_eq!(
    doctest,
    Some(&toml::Value::Boolean(false)),
    "Cargo.toml must declare `doctest = false` under `[lib]`: the lib carries no doctests \
     (issue #659), and a plain `cargo test` would otherwise run an empty doctest phase. If \
     doctests are wanted now, write them, drop this key and restore a `cargo test --doc` step \
     in ci.yml together, then update this test"
  );

  // Every job, not just `test`: the step was there, and moving it to another
  // job is the same empty step under a different name. Detected by token so
  // `cargo test --all --doc` and `cargo  test --doc` are caught along with
  // the spelling it used to have.
  let workflow = ci_workflow();
  for (job_name, job) in workflow["jobs"].as_mapping().expect("ci.yml must define `jobs:`") {
    for run in run_steps(job) {
      let tokens: Vec<&str> = run.split_whitespace().collect();
      assert!(
        !(tokens.contains(&"cargo") && tokens.contains(&"--doc")),
        "ci.yml job {job_name:?} runs doctests ({run:?}) while Cargo.toml declares \
         `doctest = false` and the lib carries none, so the step has nothing to run and cannot \
         fail (issue #659). Restore it together with real doctests and the key, not alone"
      );
    }
  }
}

/// Issue #646. The `test` job is the one carrying `cargo build` and `cargo
/// nextest run`, so switching it off takes the whole suite with it. `if:
/// false` on this job was mutated into `ci.yml` and all 19 tests in this
/// binary stayed green, because every guard here reads `step[...]` and GitHub
/// Actions resolves `if:` at the job level too.
///
/// No step of it is allowed an `if:`. The doctest step narrowed to the ubuntu
/// row was the one exception, until #659 removed the step.
#[test]
fn ci_test_job_cannot_be_switched_off_or_made_advisory() {
  assert_job_is_blocking(&ci_workflow(), "test");
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
  assert_job_is_blocking(&ci_workflow(), "audit");

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
/// jobs, and the workflow's own `on:` block switches all nine off at once,
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
         dropping either stops all nine jobs on that path while every job-level guard \
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

/// Issue #672. The `flake` job is the oracle for the flake's version: the text
/// guard in `flake_tests.rs` reads bindings by name, and four review passes of
/// #648 each found a Nix form it misses. `assert_job_is_blocking` reads what
/// can skip a job, and the shape of a `run:` only when it invokes cargo, so
/// review measured this job neutralised with it green: `exit 1` turned into
/// `exit 0`, or `shell: 'true {0}'`, which never runs the script. So the job
/// is pinned whole, its keys and each step by value, and so are the two
/// workflow-level keys that reach its step: `env:`, where `SHELLOPTS: noexec`
/// has bash parse the script and exit 0, and `defaults:`, which can set its
/// shell.
///
/// What this leaves out is what nix and the install action do inside, the
/// ceiling `assert_job_as_written` names for the publish jobs.
#[test]
fn ci_evaluates_the_flake_version_against_cargo_toml() {
  let workflow = ci_workflow();
  assert_job_is_blocking(&workflow, "flake");

  let mut job: serde_yaml_ng::Value = serde_yaml_ng::from_str(FLAKE_JOB).unwrap();
  job["steps"][2]["run"] = FLAKE_VERSION_CHECK.into();
  assert_job_as_written(".github/workflows/ci.yml", "flake", &job, FLAKE_JOB_WHY);

  let env: serde_yaml_ng::Value =
    serde_yaml_ng::from_str("CARGO_TERM_COLOR: always\nRUSTFLAGS: -D warnings\nGWM_NO_GLOBAL_CONFIG: \"1\"").unwrap();
  assert_eq!(
    workflow["env"], env,
    "ci.yml's workflow-level `env:` changed. It reaches the flake job's step, and \
     `SHELLOPTS: noexec` there has bash parse the comparison and exit 0 without running it \
     (#672). If the change is intended, update the `env` this test expects in the same diff"
  );
  assert!(
    workflow["defaults"].is_null(),
    "ci.yml must carry no workflow-level `defaults:`: a `run.shell` there sets the shell of \
     the flake job's step, and `true {{0}}` never runs it. Got {:?}",
    workflow["defaults"]
  );
}

const FLAKE_JOB_WHY: &str = "Issue #672: this job is what fails when the flake's version drifts \
  from Cargo.toml, and `exit 1` turned into `exit 0`, the comparison dropped, or a shell that \
  never runs the script each leave it green over a drifted flake. If the change is intended, \
  update `FLAKE_JOB` and `FLAKE_VERSION_CHECK` in the same diff";

/// The `flake` job of `ci.yml`, its script aside (`FLAKE_VERSION_CHECK`).
const FLAKE_JOB: &str = r#"
name: flake version (nix eval)
runs-on: ubuntu-latest
steps:
  - uses: actions/checkout
    with:
      persist-credentials: false
  - uses: cachix/install-nix-action
  - name: the flake's package is Cargo.toml's version
"#;

/// Both versions are read by nix: `Cargo.toml`'s through `builtins.fromTOML`,
/// `--impure` because it is the checkout's file and not the store's, and the
/// packages' by evaluating them. `name` is compared too, since a `name =` next
/// to `pname` changes what `nix profile list` shows without touching
/// `version`.
///
/// Every system `packages` exposes is evaluated, not one (#675): evaluation is
/// system-independent, so a runner can read them all, and a system the pinned
/// nixpkgs no longer serves fails the `nix eval` outright rather than waiting
/// for a user on that platform to find out.
///
/// The key set is compared by value, which is what `all` cannot say: `all`
/// over the systems that remain is green on the systems that left, so a
/// shortened `eachSystem` list passes it with every version correct
/// (measured). `flake_tests.rs` pins the list too, but as text, so it stays
/// green on a list shortened while the names live on in a binding or a
/// comment; here the set comes out of the evaluation itself. An emptied
/// `packages` does not reach jq at all, the eval failing on the missing
/// attribute.
const FLAKE_VERSION_CHECK: &str = r#"want=$(nix eval --raw --impure --expr '(builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version')
got=$(nix eval --json .#packages --apply 'builtins.mapAttrs (_: ps: "${ps.gwm.name} ${ps.gwm.version}")')
echo "Cargo.toml: $want, flake: $got"
if ! printf '%s' "$got" | jq -e --arg v "$want" '(keys == ["aarch64-darwin", "aarch64-linux", "x86_64-linux"]) and all(.[]; . == "gwm-\($v) \($v)")' > /dev/null; then
  echo "::error file=flake.nix::the flake builds $got while Cargo.toml is at $want (#393, #672, #675)"
  exit 1
fi
"#;

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
/// The four per-job callers stay: `bench`, `test` and `audit` elsewhere in
/// this file, `msrv` in `msrv_tests`. Each carries a rationale the sweep
/// cannot hold, and two carry a property it cannot express either, `audit`'s
/// `--deny warnings` and `bench`'s `--benches -- --test`. The sweep allows no
/// step an `if:` anywhere: the doctest step's ubuntu condition was the one
/// waiver it carried, and #659 removed that step. They overlap
/// with the sweep on purpose. Redundant coverage costs a millisecond; a gap
/// costs nine months, which is what RUSTSEC-2025-0068 did.
#[test]
fn ci_every_job_is_blocking_except_the_advisory_doctor() {
  let workflow = ci_workflow();
  // Through `string_keys` (issue #673): a job keyed `true:` or `false:`,
  // skipped, could be switched off with this sweep green.
  let jobs = string_keys(
    workflow["jobs"]
      .as_mapping()
      .expect("ci.yml must define a `jobs:` mapping"),
    "ci.yml `jobs:`",
  );

  // A sweep guards the jobs it finds and says nothing about the ones that
  // stopped existing. An emptied `jobs:` leaves it iterating over nothing
  // while reporting success, and a count-based floor does not close that
  // either: deleting one job while adding another satisfies any count. So the
  // nine are named.
  //
  // This is an enumeration, and deliberately so, because it is a **bounded**
  // one. It covers the jobs `ci.yml` ships today; the sweep below covers the
  // ones it does not, which is the exact inverse of the caller list this test
  // replaces, a list that could only ever cover what someone remembered to add
  // to it. Membership is a floor and never an equality: a tenth job has to be
  // a green test that the sweep then guards, not a red one.
  for expected in [
    "fmt",
    "clippy",
    "msrv",
    "test",
    "bench",
    "hook-smoke",
    "audit",
    "flake",
    "doctor",
  ] {
    assert!(
      jobs.iter().any(|j| j == expected),
      "ci.yml must still define the `{expected}` job. Deleting or renaming it is invisible to \
       the sweep below, which guards whatever jobs it finds, and a job going quiet is not \
       always a blocked merge either: only five of the nine are required contexts on `main`, \
       so `msrv`, `bench`, `flake` and `doctor` can vanish with nothing on GitHub's side objecting. \
       Got {jobs:?}"
    );
  }

  for job_name in &jobs {
    if job_name == "doctor" {
      assert_doctor_is_still_the_advisory_job(&workflow["jobs"]["doctor"]);
      continue;
    }
    assert_job_is_blocking(&workflow, job_name);
  }
}

/// What the sweep's one exemption is exempt *as*: the advisory job, its `if:`
/// restricting it to `dev` and its report step marked `continue-on-error:
/// true`. Both are deliberate, the report wants eyes rather than a blocked
/// merge and `lazygit` is absent on the runner so a Warning is its floor. Lose
/// either and it is no longer the job the exemption was written for, so it
/// goes red here and has to join the guarded set instead.
///
/// The marker is pinned to the step that carries it, never asserted
/// existentially over the job's steps. "some step of `doctor` is
/// `continue-on-error`" is satisfied by any of them, so moving the marker off
/// `gwm doctor` and onto `cargo build` leaves an existential assertion green
/// while `gwm doctor` itself becomes able to fail the job, which is the exact
/// drift the exemption claims to catch. Exactly one step must answer to the
/// label, so a second step renamed `gwm doctor` cannot inherit the marker
/// written for its neighbour either.
///
/// "The job cannot turn the workflow red" is deliberately *not* the property
/// asserted, because it is not true and never was: `doctor`'s checkout, its
/// toolchain install and its `cargo build` all fail hard, and should. Only the
/// report is advisory.
///
/// `DOCTOR_CONDITION` is the condition spelled on one line: `ci.yml` writes it
/// as a block scalar across two.
const DOCTOR_CONDITION: &str = "(github.event_name == 'push' && github.ref == 'refs/heads/dev') || \
                                (github.event_name == 'pull_request' && github.base_ref == 'dev')";

fn assert_doctor_is_still_the_advisory_job(job: &serde_yaml_ng::Value) {
  // By value, not by presence. `!job["if"].is_null()` is satisfied by any
  // condition at all, `if: always()` included, while the message below claims
  // the restriction to `dev` is what it checks. That is the same overstatement
  // the `continue-on-error` assertion made before it was pinned to its step.
  //
  // Whitespace is normalised first because the condition is a YAML block
  // scalar: reflowing it across lines is a formatting change and must not be a
  // red test, whereas changing what it admits must be.
  let condition = job["if"]
    .as_str()
    .unwrap_or_default()
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
  assert_eq!(
    condition, DOCTOR_CONDITION,
    "the `doctor` job is exempt from the blocking guard because it is advisory, and its \
     `if:` no longer restricts it to `dev`. `main` is meant to be stable and the doctor \
     exists to catch in-development regressions, so widening this runs an advisory job on \
     every release path; narrowing it stops the only thing that exercises `gwm doctor` at \
     all. Either way it is no longer the job this exemption was written for: guard it like \
     the rest, or restore the condition"
  );

  let steps = job["steps"].as_sequence().cloned().unwrap_or_default();
  let reports: Vec<&serde_yaml_ng::Value> = steps
    .iter()
    .filter(|s| s["name"].as_str() == Some("gwm doctor"))
    .collect();
  assert_eq!(
    reports.len(),
    1,
    "the `doctor` job must hold exactly one step named `gwm doctor`, found {}. Zero means \
     the step this exemption is written around was renamed or removed and the exemption now \
     covers nothing; more than one means a second step answers to the label and inherits \
     the advisory marker written for its neighbour",
    reports.len()
  );
  assert_eq!(
    reports[0]["continue-on-error"].as_bool(),
    Some(true),
    "the `gwm doctor` step must carry `continue-on-error: true`: that one step being \
     advisory is the whole reason this job sits outside the blocking guard. Asserting it of \
     the step rather than of the job as a whole is deliberate, since `some step is \
     continue-on-error` stays green when the marker moves onto `cargo build` and `gwm \
     doctor` quietly becomes able to fail the job. Got `continue-on-error: {:?}`",
    reports[0]["continue-on-error"]
  );
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

  // The two assertions above name the flags and why they matter, which is the
  // diagnostic half. They do not close the command, because `contains` reads a
  // line where a later flag overrides an earlier one: `cargo fmt --all --
  // --check --config=disable_all_formatting=true` keeps both substrings, is
  // one bare invocation, and exits 0 over a file rustfmt would otherwise
  // reject. So the command is pinned by value, the same statement already made
  // about the `RUSTFLAGS` that reaches `clippy`.
  assert_eq!(
    fmt, EXPECTED_FMT,
    "the fmt job must run exactly `{EXPECTED_FMT}`. Appending to it is enough to undo it, \
     since rustfmt takes `--config` on the command line and the last setting wins, so \
     neither `--check` nor `--all` surviving in the line says the line still checks \
     anything. Changing what CI formats is a conscious decision in a reviewed diff"
  );
}

/// The formatting command, pinned. `CLAUDE.md`: "CI enforces `cargo fmt
/// --check`".
const EXPECTED_FMT: &str = "cargo fmt --all -- --check";

/// The lint command, pinned. `CLAUDE.md`: "`cargo clippy --all-targets -- -D
/// warnings` must pass". `--all-features` on top, because a lint behind a
/// non-default feature is still a lint.
const EXPECTED_CLIPPY: &str = "cargo clippy --all-targets --all-features -- -D warnings";

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
/// job, so it is one edit away from being gone for all nine of them, and the
/// `msrv` job already overrides it to `""` at job level, which is precedent
/// that it does get overridden. The command has to carry its own denial.
///
/// It is also not inert, which is the other half of the same fact and the
/// hole the paragraph above left open. The lint level clippy runs at is the
/// command *and* `RUSTFLAGS`, and `RUSTFLAGS` wins: `env: RUSTFLAGS:
/// "--cap-lints=allow"` on this job makes `cargo clippy --all-targets
/// --all-features -- -D warnings` exit 0 on every lint in the tree, denial
/// intact, one bare invocation, job green. So the flags that reach the job are
/// pinned rather than left to the command alone, at the two levels `env:`
/// exists above a step and on the steps themselves.
///
/// Pinned by value, not screened for weakening spellings. `--cap-lints=allow`,
/// `-A warnings`, `--force-warn`, a `-D warnings` cancelled by an earlier
/// `--cap-lints`: rustc's flags are not a fixed set and enumerating the ones
/// that weaken is the denylist #652 already walked through. The exact value is
/// the only statement that also closes the ones nobody has thought of.
///
/// `CARGO_ENCODED_RUSTFLAGS` is refused outright rather than pinned, because
/// cargo reads it *instead of* `RUSTFLAGS` when it is set: a job carrying it
/// would leave the pin above describing a variable nothing reads.
#[test]
fn ci_clippy_job_denies_warnings_across_all_targets() {
  let workflow = ci_workflow();
  let job = ci_job("clippy");
  let runs = run_steps(&job);
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

  // Same reason as the fmt job: `contains` reads a line whose last flag wins.
  // `cargo clippy --all-targets --all-features -- -D warnings --cap-lints=allow`
  // keeps both substrings, is one bare invocation, touches no `env:`, and exits
  // 0 on every lint in the tree. Pinning `RUSTFLAGS` below while leaving the
  // command open would refuse the neutralisation in the variable and hand it
  // over on the line beside it.
  assert_eq!(
    clippy, EXPECTED_CLIPPY,
    "the clippy job must run exactly `{EXPECTED_CLIPPY}`. `-D warnings` surviving in the \
     line does not mean the line denies anything: `--cap-lints=allow` appended after it \
     caps every lint in the tree and the job exits 0. Changing what CI lints is a conscious \
     decision in a reviewed diff"
  );

  assert_eq!(
    workflow["env"]["RUSTFLAGS"].as_str(),
    Some("-D warnings"),
    "the workflow-wide `RUSTFLAGS` must stay exactly `-D warnings`, because it is half of \
     the lint level `cargo clippy` runs at and the half that wins: `--cap-lints=allow` here \
     exits the job 0 on every lint in the tree while the `-D warnings` on the command sits \
     there untouched. Changing what clippy is allowed to ignore is a conscious decision in \
     a reviewed diff, not a one-word edit to a shared `env:` block. Got {:?}",
    workflow["env"]["RUSTFLAGS"]
  );

  // Below the workflow, `env:` exists at exactly two levels, and either one
  // shadows the value pinned above for this job alone. `msrv` overriding
  // `RUSTFLAGS` to `""` two jobs away is the precedent that this does happen.
  let mut envs: Vec<(&serde_yaml_ng::Value, String)> = vec![(&job["env"], "the `clippy` job".to_string())];
  let steps = job["steps"].as_sequence().cloned().unwrap_or_default();
  for step in &steps {
    envs.push((&step["env"], format!("step {:?} of the `clippy` job", step_label(step))));
  }
  for (env, where_) in &envs {
    assert!(
      env["RUSTFLAGS"].is_null(),
      "`RUSTFLAGS` must not be set on {where_}: it shadows the workflow-wide `-D warnings` \
       for this job alone, and it decides the lint level over the command's own denial. Got \
       `RUSTFLAGS: {:?}`",
      env["RUSTFLAGS"]
    );
    assert!(
      env["CARGO_ENCODED_RUSTFLAGS"].is_null(),
      "`CARGO_ENCODED_RUSTFLAGS` must not be set on {where_}: cargo reads it *instead of* \
       `RUSTFLAGS`, so it silently replaces the value pinned above rather than adding to it. \
       Got `CARGO_ENCODED_RUSTFLAGS: {:?}`",
      env["CARGO_ENCODED_RUSTFLAGS"]
    );
  }
  assert!(
    workflow["env"]["CARGO_ENCODED_RUSTFLAGS"].is_null(),
    "`CARGO_ENCODED_RUSTFLAGS` must not be set workflow-wide either, for the same reason: \
     cargo reads it instead of `RUSTFLAGS`, so the pin above would describe a variable \
     nothing reads. Got {:?}",
    workflow["env"]["CARGO_ENCODED_RUSTFLAGS"]
  );
}
