# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **User-level global config** at `~/.config/gwm/config.toml` (XDG: `$XDG_CONFIG_HOME/gwm/config.toml`), merged **underneath** each repo's `.gwm.toml`. Set a preference once — e.g. `[theme] preset = "…"` — and it applies to every repo. The merge is a deep TOML overlay: the repo wins on conflicting scalars, tables (`[theme]`, `[worktree]`, `[tui]`, …) merge key-by-key, and arrays (`[[labels]]`, `[[bootstrap.copy]]`, …) are replaced wholesale by the repo when present. Validation runs on the merged result. With no global file, loading is identical to before (repo-only, then the built-in default). Set `GWM_NO_GLOBAL_CONFIG=1` to force strictly repo-only loading (used by CI for deterministic runs). (#190)
- Ephemeral **PR auto-detection** for a branch's pull request. When no PR is explicitly linked (`gwm link --pr`), gwm now resolves the branch's PR from GitHub (`gh pr list --head <branch> --state all`) and surfaces it marked `detected` — never written to git config, so it stays fresh and an explicit link always wins. Wired into `gwm status` (always, one worktree), the TUI sidebar (on the `F` refresh, marked " (detected)"), and a new opt-in `gwm list --detect-pr` flag that adds a `PR` column (one `gh` call per worktree; plain `gwm list` stays network-free). New `LinkSource::Detected` provenance alongside `branch-name` / `explicit`. (#181)
- `{repo_path}` and `{repo_parent}` placeholders for `.gwm.toml` paths/patterns. `{repo_path}` expands to the main repo's absolute working directory and `{repo_parent}` to its parent, so a base can be expressed relative to the repo on disk — e.g. `base = "{repo_parent}/worktrees"` puts worktrees in a sibling `worktrees/` dir, matching an editor's `../worktrees` convention (Zed's `git.worktree_directory`) without a per-project editor config. Purely additive; existing `{home}`/`{repo}` bases are unchanged. (#175)
- Colourisation of the **Working Tree** sidebar block in the TUI, with three distinct status colours so modified and created files stay visually apart: the staged (X) status column renders cyan, a worktree modification (Y) yellow, and untracked `??` entries green. Each file name takes its dominant status colour (green when untracked, yellow when modified, cyan when staged-only). The displayed text is unchanged — only colour is added — so each entry still shows the exact `git status --short` codes. (#179)

### Changed

- **TUI modal polish** — every overlay (`confirm delete`, `help`, `new worktree`, `bootstrap report`, `open`, `link`, `command palette`) now shares one frame: a rounded border with a bold themed title, colours pulled from the resolved `[theme]` instead of hard-coded values, and a box sized to its content rather than a fixed percentage of the screen (the confirm and create modals no longer dwarf their few lines). The confirm overlay gained selectable `[ Confirm ]` / `[ Cancel ]` buttons (navigate with `←` / `→` / `Tab`, `Enter` activates the focused one), with focus defaulting to **Cancel** so a stray `Enter` cancels rather than deletes; the classic `y` / `n` / `Esc` shortcuts are unchanged. A long worktree path is tilde-compressed and middle-ellipsized to one line. An animated spinner sits beside the safety-countdown progress bar as a live loader. The help overlay renders each key chord as its own coloured **badge** (the statusline chip style) with themed section headers. (#187)
- TUI statusline is now a single line. Key hints render as reverse-video badge chips (the key painted with the theme accent, followed by a short label) and the status message (action log) is pinned flush-right with absolute priority — when the terminal is too narrow, the hint list is truncated with an `…` marker while the log stays visible. Previously the footer occupied two rows and wrapped, which could push the log off-screen. No keybindings changed. (#180)

## Past releases

In reverse chronological order:

- [`0.7.0`](changelogs/0.7.0.md) — 2026-05-23
- [`0.6.0`](changelogs/0.6.0.md) — 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md) — 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

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
