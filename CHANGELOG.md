# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Configuration panel: press `4` to open a ~90% fullscreen, scrollable
  view of the **resolved** `.gwm.toml` — the user-level global config
  (`~/.config/gwm/config.toml`) deep-merged under the repo file, exactly
  as `gwm config list` resolves it. Rows are grouped by `[section]` and
  each carries a colour-coded source column (**repo** / **user** /
  **default**) marking which layer won — provenance the CLI never
  exposed. Scrolls like the help overlay (`j`/`k`, `g`/`G`, `h`/`l`);
  `Esc` / `q` / `4` closes it. Read-first: inline editing stays a
  follow-up. Completes the `1`/`2`/`3`/`4` pane-key family; reachable by
  name via `:config-panel` and rebindable through `[tui.keys] config_panel`.
  ([#232](https://github.com/kbrdn1/gwm-cli/issues/232))
- Command Logs overlay: press `3` to open a lazygit-style, scrollable
  transcript of the external commands gwm ran — the resolved command line,
  duration, exit status, and captured output — newest-first over a ~90%
  fullscreen modal. Scrolls like the help overlay (`j`/`k`, `g`/`G`,
  `h`/`l`); `Esc` / `q` / `3` closes it. Completes the `1`/`2`/`3`
  pane-key family and gives the single-line statusbar action log a full
  scrollback. The transcript records `gh` GitHub calls, bootstrap shell
  steps, and lifecycle hooks; read-only sidebar previews are excluded to
  keep it signal-rich. Reachable by name via `:command-logs`; the `3`
  binding is rebindable through `[tui.keys] command_logs`.
  ([#226](https://github.com/kbrdn1/gwm-cli/issues/226))
- Off-thread worktree list refresh: pressing `f` / `r` now re-lists the
  worktrees on a background worker instead of blocking the event loop, so a
  large repo or slow filesystem no longer freezes the TUI mid-refresh. The
  statusbar spinner animates with a `refreshing worktrees…` label and `q` /
  `Esc` stay responsive while it runs; a failed re-list surfaces on the
  status bar instead of tearing down the session. Built on a new generic
  async-task spine (coalescing + late-result drop) that the GitHub fetch,
  bootstrap, and future panels can adopt.
  ([#231](https://github.com/kbrdn1/gwm-cli/issues/231))

### Changed

- Sidebar rendering no longer deep-clones the cached sections on every frame.
  The warm-cache draw path now renders the cached lines by reference; on a
  300-commit sidebar this is ~19% faster per frame (no perceptible change
  otherwise). ([#238](https://github.com/kbrdn1/gwm-cli/issues/238))
- The off-thread GitHub issue / PR fetch (`F`) now runs on the same shared
  async-task spine as the worktree refresh instead of its own channel — one
  off-thread mechanism, not two. Behaviour-preserving for the happy path; the
  spine's per-key generation counter is what fixes the race below.
  ([#255](https://github.com/kbrdn1/gwm-cli/issues/255))
- The fuzzy filter (`/`) now renders inside the worktrees pane title instead
  of a separate row below the table: the `/query`, cursor, and
  `(visible/total)` ratio read in the pane border, attached to the list they
  narrow. The standalone filter bar is gone, giving the table one more row.
  ([#262](https://github.com/kbrdn1/gwm-cli/issues/262))
- The command palette (`:`) input moved to the top of the modal (above the
  matches) and adopts the New Worktree modal's background-filled input style,
  so the palette reads input-then-results like the create form.
  ([#262](https://github.com/kbrdn1/gwm-cli/issues/262))

### Fixed

- GitHub status refresh no longer shows stale issue / PR data after a quick
  re-fetch. Pressing `F` (or navigating away and back) while a fetch was still
  in flight could let the *older* worker's result win the race and overwrite
  the fresh one, because the previous dedupe had no per-fetch generation. The
  fetch now rides the async-task spine, so a superseded worker's late result
  is dropped regardless of arrival order.
  ([#255](https://github.com/kbrdn1/gwm-cli/issues/255))

## Past releases

In reverse chronological order:

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
