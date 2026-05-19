# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `[doctor].trunks` in `.gwm.toml` ([#59](https://github.com/kbrdn1/gwm-cli/issues/59)) — `gwm doctor`'s orphan-branch check now reads its trunk list from a new `[doctor]` table, default `["dev", "main"]`. Repos with non-standard trunk conventions (`master`, release trains like `release-3.x` / `release-4.x`) can opt in via config instead of seeing every merged gwm-style branch flagged as "unmerged orphan" — the silent no-op that previously affected every repo not following gwm-cli's own convention. An explicit empty list (`trunks = []`) disables the merge filter entirely. Zero-diff for repos without a `[doctor]` section, since `#[serde(default)]` on `Config::doctor` resolves to the same `["dev", "main"]` that lived in the removed `TRUNK_BRANCHES` const.

### Tests

- Sentinel / regression-only audit ([#57](https://github.com/kbrdn1/gwm-cli/issues/57)) — classified all 193 tests at v0.4.0 into three buckets (drives production logic, regression sentinel, dead weight), deleted 3 bucket-#3 tests (`bootstrap_when_tests::evaluator_is_pure_for_a_given_cwd`, `doctor_tests::prunable_check_detail_uses_singular_plural_correctly`, `cli_binary::shell_init_posix_does_not_use_paren_function_syntax`) that asserted constants against themselves or duplicated positive coverage, and annotated the 11 retained sentinels with a `// regression: <one-line>` tag so the incident target is discoverable without `git blame`. Total: **193 → 190**. Full audit table in [`claudedocs/test-audit-0.4.0.md`](claudedocs/test-audit-0.4.0.md).

### Docs

- `CLAUDE.md` + `CONTRIBUTING.md` §Releases — new "Step 0: reconcile open PRs" rule. Before any RC or stable cut, run `gh pr list --state open` and reconcile every open PR (in the changeset, intentionally deferred, or closed as stale). Codifies the lesson from the v0.3.0 cut, which shipped without three queued feature PRs (#51, #52, #53) and required an immediate v0.4.0 promotion 38 minutes later. Step 0 sits explicitly at the top of both the pre-release and stable-release procedures.
- `examples/gwm.toml.example` — documents the new `[doctor].trunks` knob with a commented-out block matching the bootstrap-step style already in the file.
- README — bumped the suite-size advert to 190 tests, added the missing `bootstrap_when_tests.rs`, `doctor_tests.rs`, and `flake_tests.rs` entries to the file tree, and pointed at the audit doc for sentinel hygiene.

## Past releases

In reverse chronological order:

- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
