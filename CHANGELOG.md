# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **TUI Details sidebar redesign** (#69) — the right pane is now made of four
  independent rounded-border subsections (`Worktree` / `Issue / PR` /
  `Working Tree` / `Recent Commits`) instead of one big `Details` block with
  flat `Label:` headers. The outer `Details` frame is dropped to reclaim
  vertical space. Section titles live on the block borders, so the inline
  `Basic Settings:` / `Recent commits:` / `Working tree:` content lines are
  gone. The redundant `─── Issue / PR ───` content header is removed in
  favour of the block title.

### Removed

- **`Commands:` cheat-sheet from the Details sidebar** — the 15-line
  keybindings list duplicated the `?` help overlay and pushed the
  `Issue / PR` block off-screen on common terminal sizes. Press `?` for the
  full key map. (#69)

### Docs

- **`skills/SKILL.md` refresh** — the bundled `gwm` Skill is updated to
  match the current `0.6.0-rc.1` surface: new subcommands (`doctor`,
  `switch`, `tmux`, `zellij`, `link` / `unlink` / `open` / `status`,
  `completions`, `shell-init`), composable `when` predicates, `[doctor]` /
  `[tui]` config sections, opt-in pre-commit hook at `.githooks/pre-commit`,
  updated triggers / TUI key map / troubleshooting.

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
