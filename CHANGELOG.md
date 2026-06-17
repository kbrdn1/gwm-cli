# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Frozen, versioned machine contracts for 1.0** (issue #317). The
  machine-readable surfaces — the `--format=json` outputs (`list`/`doctor`/
  `path`), `gwm status --json`, the daemon JSON-RPC protocol, and the
  `.gwm.toml` section set — are now pinned by contract tests
  (`tests/contract_tests.rs`) so a rename, removal, or type change of a
  stable field fails CI rather than slipping out as an accidental break. The
  guards are layered: DTO-vs-schema parity, plus DTO-side and schema-side
  field/type baselines that also catch a *coordinated* rename or a
  re-tightened `additionalProperties`. A new `gwm::contract` module is the
  single source of truth for `SCHEMA_VERSION` (the 1.0 baseline, `1`), the
  frozen daemon method/notification names, and the config section set. The
  daemon's `worktrees.changed` notification now carries
  `params.schema_version` so a long-lived `subscribe` client can detect a
  contract drift; the field is additive and ignorable. The output schemas use
  `additionalProperties: true` so an additive field validates under the same
  version (consumers ignore unknowns), while `.gwm.toml` keeps
  `deny_unknown_fields` (input rejects typos). Each `docs/schema/*.json`
  declares its `version`, and a new
  [`docs/schema/README.md`](docs/schema/README.md) documents the versioning
  policy and the stable-vs-experimental tier of every field. No behaviour
  change to any existing output.

### Docs

- **Stability & compatibility policy for 1.0** (issue #318). A new
  [Stability](docs/6.development/3.stability.md) page (EN + `docs/fr/`) is the
  explicit, published SemVer contract behind the `1.0` line: which surfaces
  are covered (CLI subcommands/flags, exit codes, the `--format=json` schemas,
  the daemon JSON-RPC protocol, the `.gwm.toml` section set), which are free
  to change in a minor/patch (TUI layout & colours, human-readable strings,
  the internal Rust API), the MSRV policy (currently 1.86, bumps ride a
  minor), and the deprecation process. It distinguishes surfaces *frozen by
  `tests/contract_tests.rs`* from those *covered by the written promise*, and
  defers the per-field stable/experimental tiers to
  [`docs/schema/README.md`](docs/schema/README.md). Linked from the README and
  `CONTRIBUTING.md`.

## Past releases

In reverse chronological order:

- [`0.9.0`](changelogs/0.9.0.md) — 2026-06-07
- [`0.8.0`](changelogs/0.8.0.md) — 2026-06-01
- [`0.7.0`](changelogs/0.7.0.md) — 2026-05-23
- [`0.6.0`](changelogs/0.6.0.md) — 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md) — 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.10.0-rc.4`](changelogs/pre-releases/0.10.0-rc.4.md) — 2026-06-17
- [`0.10.0-rc.3`](changelogs/pre-releases/0.10.0-rc.3.md) — 2026-06-17
- [`0.10.0-rc.2`](changelogs/pre-releases/0.10.0-rc.2.md) — 2026-06-16
- [`0.10.0-rc.1`](changelogs/pre-releases/0.10.0-rc.1.md) — 2026-06-10
- [`0.9.0-rc.3`](changelogs/pre-releases/0.9.0-rc.3.md) — 2026-06-07
- [`0.9.0-rc.2`](changelogs/pre-releases/0.9.0-rc.2.md) — 2026-06-06
- [`0.9.0-rc.1`](changelogs/pre-releases/0.9.0-rc.1.md) — 2026-06-02
- [`0.8.0-rc.5`](changelogs/pre-releases/0.8.0-rc.5.md) — 2026-06-01
- [`0.8.0-rc.4`](changelogs/pre-releases/0.8.0-rc.4.md) — 2026-05-29
- [`0.8.0-rc.3`](changelogs/pre-releases/0.8.0-rc.3.md) — 2026-05-29
- [`0.8.0-rc.2`](changelogs/pre-releases/0.8.0-rc.2.md) — 2026-05-23
- [`0.8.0-rc.1`](changelogs/pre-releases/0.8.0-rc.1.md) — 2026-05-23
- [`0.7.0-rc.3`](changelogs/pre-releases/0.7.0-rc.3.md) — 2026-05-23
- [`0.7.0-rc.2`](changelogs/pre-releases/0.7.0-rc.2.md) — 2026-05-23
- [`0.7.0-rc.1`](changelogs/pre-releases/0.7.0-rc.1.md) — 2026-05-22
- [`0.6.0-rc.1`](changelogs/pre-releases/0.6.0-rc.1.md) — 2026-05-20
- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md) — 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md) — 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
