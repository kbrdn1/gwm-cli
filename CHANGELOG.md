# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- TUI sidebar gets a lazygit-style facelift (#73):
  - New `Created` row showing the branch's age in compact relative form
    (`2d`, `3w`, `1M`, …), colour-coded by freshness (green < 7d,
    yellow < 30d, dark gray otherwise).
  - Branch name is now coloured by `BranchStatus` (worst-state wins —
    dirty → red, ahead/behind → yellow, unpublished → magenta, synced
    → green, unknown → dark gray). Applies to both the sidebar
    `Branch:` row and the table `BRANCH` column.
  - Sidebar header line is now prefixed by a `●` status dot whose
    colour tracks the linked PR / issue state (open=green,
    draft=gray, merged=magenta, closed=red).
- New `[tui.open]` config section + `OpenTarget` dispatch for the `o`
  key (#73). `mode` accepts `"shell"` (default, lazygit-style
  `$SHELL` in the worktree), `"editor"` (`$EDITOR <path>`), or
  `"finder"` (pre-#73 OS file manager). `shell_cmd` / `editor_cmd`
  override the env var when set.
- New `y` keybinding: yank the selected worktree's path to the system
  clipboard (`pbcopy` / `wl-copy` / `xclip` / `xsel` / `clip`).
  Missing tool surfaces a clear status hint, no crash.

### Changed
- **Default behaviour of `o:` open changes** (#73). It used to spawn
  the OS file manager unconditionally; now it spawns `$SHELL` in the
  worktree by default. Opt back into the old behaviour with
  `[tui.open] mode = "finder"` in `.gwm.toml`.
- TUI footer / help overlay / sidebar cheat-sheet updated to reflect
  the new `o:open` and `y:yank` bindings.

## Past releases

In reverse chronological order:

- [`0.5.0`](changelogs/0.5.0.md) — 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.6.0-rc.1`](changelogs/pre-releases/0.6.0-rc.1.md) — 2026-05-20
- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md) — 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md) — 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
