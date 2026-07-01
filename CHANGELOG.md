# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- **Dropped the unsound, unmaintained `serde_yml` dependency** ([#340],
  RUSTSEC-2025-0068). The issue-form front-matter parser now uses the
  maintained `serde_yaml_ng` fork; bumped `anyhow` to 1.0.103 to clear
  RUSTSEC-2026-0190. The CI `cargo audit` job is now blocking
  (`--deny warnings`, `continue-on-error` removed) so warning-class
  advisories — unmaintained / unsound / yanked — fail the build instead
  of being silently ignored.

[#340]: https://github.com/kbrdn1/gwm-cli/issues/340

- **Hardened the `gwm daemon` unix socket** ([#341]). The socket is now
  chmod'd owner-only (`0600`), and on the world-writable `/tmp` last-resort
  fallback it is nested in a per-user owner-only `gwm-<uid>/` directory — so
  another local user can no longer connect and read the worktree list even
  on platforms that don't enforce socket-file perms for `connect(2)`. The
  common `$XDG_RUNTIME_DIR` / `$TMPDIR` paths (already private) are
  unchanged. Added DoS
  guards on the request/response path: a per-line length cap, an idle read
  timeout, and a concurrent-connection cap (all configurable on
  `ServeOptions`). Also fixed a bug where a transient `run_list` git error
  pushed a phantom-empty `worktrees.changed` to `subscribe` clients (they
  flickered "everything vanished", then self-healed next poll) — transient
  errors are now swallowed instead of streamed as an empty snapshot.

[#341]: https://github.com/kbrdn1/gwm-cli/issues/341

### Performance

- **Moved the last synchronous git subprocesses off the TUI render path**
  ([#343]). The details sidebar rebuilt its git-backed sections
  (`git_diff_stat_vs_base`, `git status --porcelain -z`, `git log`,
  `git stash list`) synchronously inside `terminal.draw()` on every
  selection / mode change, stalling `j` / `k` on a large repo or a slow
  filesystem; workspace-mode auto-refresh re-listed every repo
  synchronously too. Both now ride the `TaskRunner` (#231): the render
  path only reads the last-known payload — showing a muted `loading…`
  placeholder while a rebuild is in flight (the identity card still
  renders instantly) — and a coalesced worker rebuilds it off-thread,
  keyed to the current selection. A held `j` coalesces onto the single
  in-flight worker instead of spawning a thread per row, and the poll
  cadence tightens to 50 ms while a task is loading so the preview lands
  fast.

[#343]: https://github.com/kbrdn1/gwm-cli/issues/343

### Fixed

- **`gwm undo --bootstrap` now goes through the TOFU trust gate** ([#338]).
  Re-running a repo's `[[bootstrap.command]]` shell on undo previously
  bypassed `trust_or_prompt` entirely — `cmd_undo` called `bootstrap::run`
  directly with no trust check, so an untrusted `.gwm.toml` could run shell
  commands unprompted. Undo now mediates the bootstrap re-run through the
  same gate as `create` / `review --bootstrap` / `bootstrap`, honouring
  `--allow-bootstrap` / `GWM_ALLOW_BOOTSTRAP` / `--deny-bootstrap`.

[#338]: https://github.com/kbrdn1/gwm-cli/issues/338

- **`gwm clean --workspace` no longer panics on the empty-workspace
  invariant** ([#344]). When nothing participated and no repo validated the
  `--profile` (nor reported an error), the workspace-clean handler
  `expect`-panicked on an invariant `open_workspace_repos` already enforces;
  it now returns a `GwmError` defensively instead of unwinding.

### Changed

- **1.0.x hardening backlog** ([#344]). Froze the `gwm exec` / `gwm clean`
  flag surface (`--profile` / `--jobs` / `--yes` / the global `--workspace`)
  with a `contract_tests` canary — the subcommand-name canary
  (`help_prints_subcommands`) does not see flags. Reconciled the MSRV
  enforcement story between `Cargo.toml` and the stability doc: CI's clippy
  job catches an accidental *std-API* use above the 1.86 floor
  (`clippy::incompatible_msrv` under `-D warnings`), but **not**
  language/edition features or a dependency raising its own floor — those stay
  a local pre-bump check (`cargo msrv verify`). Documented the ungated
  `clean::scan_worktree` / `delete_reclaim` convention (feed them only a
  `scan_worktree_safe` reclaim) and their narrow TOCTOU window; justified the
  remaining `#[allow(clippy::too_many_arguments)]`; and added a release-process
  note to finalise the crate identity before tagging (the `v1.0.0` tag predated
  the `gwm` → `gwm-cli` rename, so crates.io `gwm-cli@1.0.0` isn't reachable
  from the tag).

[#344]: https://github.com/kbrdn1/gwm-cli/issues/344

## Past releases

In reverse chronological order:

- [`1.0.0`](changelogs/1.0.0.md) — 2026-06-26
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
