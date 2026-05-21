# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **TUI: configurable launchers for `l` (git_tui) and `R` (review)**
  ([#75](https://github.com/kbrdn1/gwm-cli/issues/75)). Two new
  `.gwm.toml` sections — `[git_tui]` and `[review]` — drive the
  worktree-list `l` / `R` keybindings through a shared launcher
  pipeline (placeholder expansion `{base} {head} {path} {diff}`,
  `shell-words` split, optional fullscreen suspend-and-resume).
  `[git_tui]` defaults to `lazygit -p {path}` fullscreen so existing
  repos see zero behaviour change. `[review]` accepts either a
  free-form `command = "<shell line>"` or a `tool = "<preset>"` sugar
  (`lumen`, `claude`, `codex`, `aider`, `gh`). New
  `branch.<n>.gwm-base` key, set by `gwm create`, anchors the review
  base-resolution chain (upstream → gwm-base → `[review].default_base`
  → `dev` → `main`). The `{diff}` placeholder lazily materialises a
  tempfile holding `git diff {base}..{head}` — only when the template
  references it.

### Changed

- **TUI keybind reshuffle**: `f` now refreshes the worktree list (was
  `r`, kept as alias for muscle memory); `F` refreshes the GitHub
  issue/PR status (was `R`); the freed `R` triggers the new review
  launcher. ([#75](https://github.com/kbrdn1/gwm-cli/issues/75))
- `gwm doctor` now probes the configured `[review]` and `[git_tui]`
  binaries against `$PATH`. A missing review tool surfaces as Warning
  (exit code `1`), never Failed (`2`) — review is opt-in, so a CI
  pre-commit hook keeps passing when only the local launcher is
  unavailable. ([#75](https://github.com/kbrdn1/gwm-cli/issues/75))

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
