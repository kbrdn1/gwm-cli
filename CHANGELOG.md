# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- TUI: direct pane-focus keys — `1` focuses the worktrees pane,
  `2` opens (if hidden) and focuses the status pane. Both are
  rebindable as `focus_worktrees` / `focus_status` under `[tui.keys]`;
  `Tab` keeps its toggle. (#217)
- TUI: contextual statusbar — a context chip (`worktrees` / `status` /
  `switch`) on the left, an animated loading spinner while a GitHub
  fetch is inflight, contextual hints in the middle, and the action log
  pinned right with absolute priority. (#217)
- TUI: a bottom-right `selected of visible` counter on the worktrees
  pane, lazygit-style. (#217)
- TUI: GitHub issue/PR status (`F`) is fetched off-thread, so the
  statusbar spinner animates during the `gh` shell-out instead of the
  event loop blocking. (#217)

### Changed

- TUI: the sidebar now defaults to the stacked layout (status pane under
  the worktrees table), and the split ratios are tuned per axis —
  42% / 58% stacked, 55% / 45% side-by-side. The orientation cycle
  (`V`) is unchanged. (#217)
- TUI: panes are labelled `[1] Worktrees (N)` and the help overlay is
  titled `Keybindings` with a centred context subtitle. (#217)
- TUI: modal overlays centre their titles and gain internal padding; the
  confirm-delete buttons render as flat coloured chips (` Confirm ` /
  ` Cancel `) instead of `[ Confirm ]` / `[ Cancel ]`. (#217)

### Fixed

- TUI: the sidebar Issue/PR block prompted `press R to fetch status`,
  but `R` runs the review launcher — it now resolves the live
  `fetch_github` binding (`F` by default). (#217)

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
