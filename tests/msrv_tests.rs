//! Guards for the declared MSRV (issue #491).
//!
//! Two failure modes, two guards:
//!
//! 1. **The declared floor drifts below the dependency graph.** `Cargo.toml`
//!    said `1.86` while the graph required far more, and nothing said a word:
//!    the clippy job only catches *std-API* regressions, never a dependency
//!    raising its own floor. Reading the graph through `cargo metadata` is not
//!    enough either, since it only reports crates that declare a
//!    `rust-version` at all (metadata said `1.88`, a build said `1.95`). Only
//!    compiling at the floor settles it, which needs that toolchain actually
//!    installed, so it is guarded by the `msrv` CI job; what *is* checked here
//!    is that the job exists and derives its toolchain from `Cargo.toml`
//!    instead of hardcoding a version that would itself drift.
//! 2. **The prose drifts from the manifest.** Ten places advertise the MSRV to
//!    users; the bump has to reach all of them.
//!
//! Same shape as `flake_tests.rs`: pin the *mechanism*, and treat `Cargo.toml`
//! as the single source of truth every other claim mirrors.

use std::fs;
use std::path::PathBuf;

fn repo_file(rel: &str) -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Reads a repo file with line endings normalised to `\n`.
///
/// The normalisation is not cosmetic. Windows runners check out with
/// `core.autocrlf=true`, so every file arrives CRLF, and `job_block`'s
/// `find("\n  msrv:\n")` matched nothing there while passing on Linux and
/// macOS: the guard failed on `test (windows-latest)` only.
fn read(rel: &str) -> String {
  fs::read_to_string(repo_file(rel))
    .unwrap_or_else(|err| panic!("{rel} must exist at the repo root; read error: {err}"))
    .replace("\r\n", "\n")
}

/// The declared floor, read from the left margin of `Cargo.toml` so a
/// `rust-version` mentioned inside the comment block above it cannot match.
fn declared_msrv() -> String {
  read("Cargo.toml")
    .lines()
    .find_map(|line| line.strip_prefix("rust-version = \""))
    .and_then(|rest| rest.split('"').next())
    .expect("Cargo.toml must declare `rust-version = \"X.Y\"` at the left margin")
    .to_string()
}

/// Every *live* MSRV claim, as a pattern with `{msrv}` standing in for the
/// declared floor. Historical statements are deliberately absent: the v0.10.0
/// row in `ROADMAP.md`, the v0.9.0 / #35 paragraphs in `docs/7.roadmap.md`,
/// the `changelogs/*.md` records and the `set_var`-became-unsafe comments in
/// `trust_tests.rs` / `history_tests.rs` all say `1.86` about the past and
/// must keep saying it. That is why this is an allowlist of anchored patterns
/// and not a repo-wide sweep for the old number.
const LIVE_CLAIMS: &[(&str, &str)] = &[
  ("README.md", "badge/rust-{msrv}%2B-orange"),
  ("CONTRIBUTING.md", "stable channel, {msrv}+"),
  ("ROADMAP.md", "MSRV is **{msrv}**"),
  ("docs/1.getting-started/1.install.md", "MSRV is **{msrv}**"),
  ("docs/fr/1.getting-started/1.install.md", "le MSRV est **{msrv}**"),
  ("docs/6.development/3.stability.md", "(currently **{msrv}**)"),
  ("docs/fr/6.development/3.stability.md", "(actuellement **{msrv}**)"),
  ("docs/7.roadmap.md", "The project MSRV is **{msrv}**."),
  ("docs/fr/7.roadmap.md", "Le MSRV du projet est **{msrv}**."),
  ("skills/SKILL.md", "MSRV {msrv})"),
  ("skills/SKILL.md", "({msrv}+ — the crate MSRV)"),
];

#[test]
fn every_live_msrv_claim_matches_cargo_toml() {
  let msrv = declared_msrv();
  for (file, pattern) in LIVE_CLAIMS {
    let expected = pattern.replace("{msrv}", &msrv);
    assert!(
      read(file).contains(&expected),
      "{file} must advertise the declared MSRV: expected to find {expected:?} \
       (Cargo.toml declares `rust-version = \"{msrv}\"`). An MSRV bump has to \
       reach every user-facing claim, not just the manifest (#491)"
    );
  }
}

/// The `msrv` job, parsed. Reading the YAML rather than slicing it out of the
/// text is what makes the assertions below say what they mean: a text search
/// for `Cargo.toml` and `rust-version` is satisfied by a *leftover* read step
/// whose output nothing consumes, so swapping the action input for a literal
/// `toolchain: 1.95` while keeping that step would have kept the old guard
/// green, reintroducing exactly the drift it exists to block. The value of the
/// input is now compared to the step output it must reference.
fn msrv_job() -> serde_yaml_ng::Value {
  let workflow: serde_yaml_ng::Value =
    serde_yaml_ng::from_str(&read(".github/workflows/ci.yml")).expect("ci.yml must be valid YAML");
  workflow["jobs"]["msrv"].clone()
}

fn steps(job: &serde_yaml_ng::Value) -> Vec<serde_yaml_ng::Value> {
  job["steps"].as_sequence().cloned().unwrap_or_default()
}

#[test]
fn ci_runs_the_msrv_check_on_every_supported_platform() {
  let job = msrv_job();
  let matrix = job["strategy"]["matrix"]["os"]
    .as_sequence()
    .cloned()
    .unwrap_or_default()
    .iter()
    .filter_map(|v| v.as_str().map(str::to_owned))
    .collect::<Vec<_>>();
  for os in ["ubuntu-latest", "macos-latest", "windows-latest"] {
    assert!(
      matrix.iter().any(|m| m == os),
      "the msrv job must run on {os}, like the `test` job (matrix is {matrix:?}). \
       A single-platform run compiles neither the \
       `[target.\"cfg(windows)\".dependencies]` block nor the `#[cfg(windows)]` \
       code, so a Windows-only dependency raising its own floor stays green on \
       Linux while `cargo install` breaks for those users"
    );
  }
}

#[test]
fn ci_feeds_the_msrv_job_the_toolchain_declared_in_cargo_toml() {
  let job = msrv_job();
  let steps = steps(&job);

  let reader = steps
    .iter()
    .find(|s| s["id"].as_str() == Some("msrv"))
    .expect("the msrv job needs a step with `id: msrv` that reads the declared floor");
  let script = reader["run"].as_str().unwrap_or_default();
  assert!(
    script.contains("rust-version") && script.contains("Cargo.toml"),
    "the `id: msrv` step must read `rust-version` out of Cargo.toml, got: {script:?}"
  );
  assert!(
    reader["shell"].as_str() == Some("bash"),
    "the reader step must force `shell: bash`: windows-latest defaults to \
     PowerShell, which has no grep or cut"
  );

  let toolchain = steps
    .iter()
    .find(|s| {
      s["uses"]
        .as_str()
        .is_some_and(|u| u.starts_with("dtolnay/rust-toolchain@"))
    })
    .expect("the msrv job must install a toolchain via dtolnay/rust-toolchain");
  assert_eq!(
    toolchain["uses"].as_str(),
    Some("dtolnay/rust-toolchain@master"),
    "pin the action at @master and pass the version as an input; a \
     `rust-toolchain@<version>` ref is a literal that drifts from the manifest"
  );
  assert_eq!(
    toolchain["with"]["toolchain"].as_str(),
    Some("${{ steps.msrv.outputs.version }}"),
    "the installed toolchain must be the value read from Cargo.toml, not a \
     literal. A literal here is the drift this whole job exists to catch (#491)"
  );
}

#[test]
fn ci_checks_the_msrv_locked_and_without_default_features() {
  let commands = steps(&msrv_job())
    .iter()
    .filter_map(|s| s["run"].as_str().map(str::to_owned))
    .collect::<Vec<_>>()
    .join("\n");

  let checks: Vec<&str> = commands
    .lines()
    .map(str::trim)
    .filter(|l| l.starts_with("cargo check"))
    .collect();
  assert!(
    !checks.is_empty(),
    "the msrv job must run `cargo check` at the declared floor"
  );
  assert!(
    checks.iter().all(|c| c.contains("--locked")),
    "every msrv `cargo check` must pass `--locked`: cargo's `rust-version` gate \
     fires at *resolve* time against the committed lockfile, which is what turns \
     a dependency declaring a higher floor into a red job. Got: {checks:?}"
  );
  assert!(
    checks.iter().any(|c| c.contains("--no-default-features")),
    "the msrv job must also check `--no-default-features`. `daemon` is default-on, \
     so the default-features check never compiles the \
     `#[cfg(not(all(any(unix, windows), feature = \"daemon\")))]` arms of \
     `cmd_daemon` / `cmd_statusline`, and that build is documented as supported. \
     Got: {checks:?}"
  );
}
