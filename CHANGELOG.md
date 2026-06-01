# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `{repo_path}` and `{repo_parent}` placeholders for `.gwm.toml` paths/patterns. `{repo_path}` expands to the main repo's absolute working directory and `{repo_parent}` to its parent, so a base can be expressed relative to the repo on disk — e.g. `base = "{repo_parent}/worktrees"` puts worktrees in a sibling `worktrees/` dir, matching an editor's `../worktrees` convention (Zed's `git.worktree_directory`) without a per-project editor config. Purely additive; existing `{home}`/`{repo}` bases are unchanged. (#175)
- `claude-dark` built-in theme preset, a port of the Claude dark palette (orange accent `#D4825D` / `#C15F3C`, green/yellow/purple/red semantics). Discoverable via `gwm theme list` and `gwm theme show claude-dark` (also resolves under the alias `claude`). (#185)

### Changed

- TUI statusline is now a single line. Key hints render as reverse-video badge chips (the key painted with the theme accent, followed by a short label) and the status message (action log) is pinned flush-right with absolute priority — when the terminal is too narrow, the hint list is truncated with an `…` marker while the log stays visible. Previously the footer occupied two rows and wrapped, which could push the log off-screen. No keybindings changed. (#180)
- TUI header is now a single borderless row with a clear visual hierarchy instead of a boxed, flat cyan string. The version renders as a reverse-video badge chip (matching the #180 footer chips), the repo name is bold, and the working directory is tilde-compressed and dimmed as secondary context. The `picker` flag is now its own reverse-video chip. The layout is width-driven: under width pressure the path is truncated/dropped first, the repo name next, and the version chip survives last. Dropping the border reclaims two rows for the worktree table. `gwm --version` parity is preserved (version still sourced from `CARGO_PKG_VERSION`). (#185)
### Fixed

- The focused panel border (worktree list ↔ sidebar, toggled with `Tab`) now paints with the theme `focus` role instead of a hardcoded `Color::Cyan`, so the active "tab" honours the configured palette (e.g. the `claude-dark` preset's orange) rather than always rendering cyan. The default theme is unchanged (`focus` is still `Cyan`), so default-theme users see no difference. (#185)

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
