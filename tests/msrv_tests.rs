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

fn read(rel: &str) -> String {
  fs::read_to_string(repo_file(rel))
    .unwrap_or_else(|err| panic!("{rel} must exist at the repo root; read error: {err}"))
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

/// The *executable* lines of the `ci.yml` block belonging to `job`, i.e. from
/// its 2-space-indented key up to the next one, with YAML comments stripped.
///
/// Both halves are load-bearing. Without the slicing, a `--locked` or a
/// `Cargo.toml` from some *other* job satisfies the assertions below. Without
/// dropping comments, the job's own prose does: the first draft of this guard
/// stayed green with `--locked` deleted from the `cargo check` line, because
/// the comment right above it explains that `--locked` is load-bearing. A
/// guard that matches its own documentation never fires.
fn job_block(yaml: &str, job: &str) -> String {
  let start = yaml
    .find(&format!("\n  {job}:\n"))
    .unwrap_or_else(|| panic!("ci.yml must declare a `{job}:` job (2-space indent)"))
    + 1;
  let rest = &yaml[start..];
  let end = rest
    .match_indices("\n  ")
    .find(|(i, _)| {
      let line = rest[i + 3..].lines().next().unwrap_or("");
      // Next job key: `  <name>:` and nothing after the colon.
      !line.starts_with(char::is_whitespace)
        && line.ends_with(':')
        && line
          .trim_end_matches(':')
          .chars()
          .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    })
    .map(|(i, _)| i + 1)
    .unwrap_or(rest.len());
  rest[..end]
    .lines()
    .filter(|line| !line.trim_start().starts_with('#'))
    .collect::<Vec<_>>()
    .join("\n")
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

#[test]
fn ci_verifies_the_declared_msrv_and_derives_it_from_cargo_toml() {
  let block = job_block(&read(".github/workflows/ci.yml"), "msrv");
  assert!(
    !block.contains("rust-toolchain@1."),
    "the msrv job must not pin a toolchain literal (`dtolnay/rust-toolchain@1.88`); \
     read `rust-version` out of Cargo.toml and feed it to \
     `dtolnay/rust-toolchain@master`, otherwise the job and the manifest drift \
     apart exactly the way the manifest and the graph did (#491)"
  );
  assert!(
    block.contains("Cargo.toml") && block.contains("rust-version"),
    "the msrv job must derive its toolchain from Cargo.toml's `rust-version`"
  );
  assert!(
    block.contains("--locked"),
    "the msrv job must pass `--locked`: cargo's `rust-version` gate fires at \
     *resolve* time against the committed lockfile, which is what turns a \
     dependency raising its floor into a red job"
  );
}
