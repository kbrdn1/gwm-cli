# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **TUI exec / clean overlays** (issue #325). The ratatui interface gains two
  overlays over the worktree list: `x` opens an **exec picker** that lists the
  `[exec.profiles.*]` names and, on `Enter`, runs the highlighted profile's
  `command` array — with no shell — in the embedded PTY overlay rooted at the
  selected worktree; `X` opens a **clean overlay** that previews the
  reclaimable build artifacts (gated by the exact git-ignored + no-tracked-files
  safety check `gwm clean --yes` uses) and deletes them behind the same safety
  countdown as the delete confirm. Both overlays' keys are rebindable under
  `[tui.keys.modal.exec]` / `[tui.keys.modal.clean]`, and `:exec` / `:clean`
  reach them from the command palette. The exec overlay runs on the single
  selected worktree (one PTY cannot fan out, unlike the CLI `--workspace`); the
  clean safety gate (`dir_is_safe_to_clean` / `scan_worktree_safe`) is now
  shared between the CLI and the overlay so both honour the identical contract.
  This is a TUI-only surface — the frozen 1.0 machine contract (config
  sections, CLI flags) is unchanged.

- **`--workspace` fan-out for `gwm exec` / `gwm clean`** (issue #326). Both
  commands now accept the global `--workspace <root>` flag and fan out across
  the workspace's child repos, **reversing the #319 deferral** (the refusal
  is gone). Each repo's command/dir-set is resolved UPFRONT — a missing
  `--profile`, a malformed `[exec]`/`[clean]`, or an unopenable child repo
  errors before anything runs (or, for `clean --yes`, before any
  `remove_dir_all`). Repos then run **sequentially** (parallelism stays
  bounded *within* a repo, so output never interleaves across repos) under a
  `══ <repo>` header, with a `<repo>/<worktree>`-tagged rollup / report and an
  aggregated exit code (non-zero if any worktree in any repo failed; for
  clean, a delete failure in one worktree is reported but doesn't abort the
  rest). `--profile` resolves per child repo against that repo's `.gwm.toml`;
  a slug that matches nothing in a given repo contributes nothing there rather
  than aborting the fan-out; a bare child repo is tolerated. This is an
  **additive** transition (a former refusal turning into a success is not
  breaking). The `--workspace` refusal remains for commands that still don't
  implement it.

- **Bounded `--jobs` parallelism for `gwm exec`** (issue #324). `gwm exec`
  gains a `--jobs <n>` flag and an `[exec] jobs` config default (with a
  per-profile `[exec.profiles.<name>].jobs` override). Precedence: `--jobs`
  > `profile.jobs` > `[exec] jobs` > `1`. `1` (or absent) keeps the unchanged
  sequential behaviour with live, inherited output; `> 1` runs up to N
  worktrees at once, capturing each one's stdout+stderr and printing it as a
  per-worktree block in worktree order once the fan-out completes (so
  concurrent runs don't interleave). The aggregate exit code is unchanged
  (non-zero if any worktree failed). The runner uses a bounded `std::thread`
  pool — no new dependency. `jobs` is a sub-field of the already-frozen
  `[exec]` section, so the 1.0 section set is unchanged.

- **Named `[exec]` / `[clean]` config profiles** (issue #324). `.gwm.toml`
  gains two opt-in sections so common fan-out invocations can be saved per
  repo instead of retyped. `[exec.profiles.<name>]` carries a `command`
  argv **array** run via `gwm exec --profile <name>`; `[clean.profiles.<name>]`
  carries a `dirs` set reclaimed via `gwm clean --profile <name>`. Both are
  now part of the frozen 1.0 contract (`contract::CONFIG_SECTIONS` gains
  `exec` / `clean`), realizing the additive profile config anticipated by
  #319. Frozen rules: an exec profile's `command` is an argv array run with
  **no shell** (a deliberate divergence from the string-shell `command` of
  `[git_tui]` / `[review]`); a clean profile's `dirs` is a **complete** set
  that **replaces** — never adds to — the built-in `target`/`node_modules`/
  `dist`/`build`; `gwm clean` without `--profile` uses
  `[clean.profiles.default]` when present, else the built-ins; for `exec`,
  `--profile` and an inline `-- <cmd>` are mutually exclusive (exit 1) and an
  unknown profile name exits 1. The safety gate (git-ignored + no tracked
  files + skip symlinks) still applies to every directory a clean profile
  lists. Bounded `--jobs` parallelism for `exec` profiles is a follow-up
  (#324b); the inline `gwm exec -- <cmd>` and built-in `gwm clean` surfaces
  are unchanged.

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

- **Frozen 1.0 surface decision for `exec` / `clean`** (issue #319). The
  deliberate MVP cuts in `gwm exec` / `gwm clean` (#313) are confirmed as
  *deferred, additive* features for the 1.0 pledge: `exec` stays sequential by
  default (bounded `--jobs` parallelism is a future opt-in), `clean` keeps its
  built-in `target` / `node_modules` / `dist` / `build` set (the `[clean]` /
  `[exec]` profile config anticipated here now lands additively under #324,
  above), and `--workspace` fan-out across repos was refused on both commands
  pending implementation — which now lands additively under #326 (below). The
  transition is additive: a previous refusal becoming a success is not a
  breaking change. No behaviour change at the time of this decision.

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
