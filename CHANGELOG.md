# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Add `gwm config get/set/unset/list/validate/path/edit` for git-config-style `.gwm.toml` reads and comment-preserving edits.
- Add lifecycle hooks under `[hooks.*]` for create/bootstrap/remove phases, with placeholders, per-step `on_fail`, `--skip-hooks`, and legacy `[[bootstrap.command]]` compatibility as `post_create`.
- Add `[issue_template]` defaults plus `gwm new <type> <desc>` to create a GitHub issue from issue-form templates and immediately create the linked worktree.
- Add `[pr_template]` defaults plus `gwm pr [--draft] [--base <ref>] [--render]` to render per-branch-type PR bodies (with `{commits}` / `{files_changed}` placeholders) and shell out to `gh pr create`.
- Add `[tui.keys]` block in `.gwm.toml` to rebind every TUI list-view action (`down`, `up`, `top`, `bottom`, `quit`, …) with crossterm-grammar keys, including multi-key chords like `g g`. Overrides are validated at load time (unknown actions, parse errors, chord conflicts, and prefix collisions are hard errors). Adds `gwm tui keys` to print the resolved keymap with per-row source, a `gwm doctor` check for unbound `quit`, and a keymap-driven help overlay (`?`) so the documentation always matches the resolved bindings.
- Add a sidebar stashes mode (issue #34) toggled by `s` (rebindable as `toggle_sidebar_mode` in `[tui.keys]`). Cycles the Details panel between `commits` (`git log --oneline` + `git status --short`, pre-existing behaviour) and `stashes` (`git stash list`). The panel title shows the active mode; the bottom hint switches to `Enter: copy stash@{N} to status` in stashes mode. Cache is keyed by `(worktree-path, mode)` so toggling re-shells the right git command without leaking stale content between modes.
- Add a command palette overlay (issue #32) opened by `:` (rebindable as `command_palette` in `[tui.keys]`). Type to fuzzy-filter the registered actions (`:create`, `:bootstrap`, `:yank`, …); `Enter` fires, `Tab`/`Down`/`Up` cycles the highlight, `Esc` cancels. The palette and the keystroke dispatcher share the same `Action` dispatcher under the hood, so the two surfaces can never drift on which verbs are addressable or how they behave.
- Add a `[theme]` block in `.gwm.toml` (issue #33) for role-based colours (`focus`, `accent`, `branch`, `clean`, `dirty`, `main`, `locked`, `prunable`, `muted`, `selection_bg`). Built-in presets ship for `catppuccin`, `gruvbox`, `tokyo-night`; per-role overrides win over presets. Values accept named (`cyan`), indexed (`220`), or hex (`#89b4fa`). Validation runs at load (unknown preset, unknown role, bad colour value all reject). Adds `gwm theme list` to print preset names and `gwm theme show <name>` to dump the preset as a copy-pasteable `[theme]` block. The TUI threads the resolved theme through `App.theme` — full color audit of every `draw_*` site will land as a follow-up.

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
