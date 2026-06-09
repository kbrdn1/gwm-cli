# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The Settings panel (`4`, renamed from Configuration) is now editable: category tabs (Theme / Worktree / TUI / All) with the edit layer shown as a subtitle, a per-project ↔ global edit-layer selector (`L`), real toggles/choices (theme preset, sidebar position, open mode), numeric input (confirm countdown) and text inputs (worktree base + patterns, open shell/editor commands). Edits persist into the chosen layer's TOML and apply live; a global edit shadowed by a `.gwm.toml` override is surfaced rather than silently dropped. The read-only resolved config lives under the `All` tab.
- Scrollable modal bodies (Keybindings, Settings) now render a herdr-style scrollbar.
- Docs roadmap pages now mirror the v0.9.0 stable surface, the v0.8.0 historical cycle, and the current `[Unreleased]` dev delta in both English and French.
- TUI now has a reusable dedicated-area `LoaderWidget`; the delete-worktree modal uses it as the first real consumer, showing both in-flight and failed delete states.
- The sidebar `Working Tree` pane now renders the changed-file count in its bottom-right footer.
- Project tooling now ships a Flippad-style colored `Makefile` plus Zed tasks for the local Rust workflow (`build`, `test`, `clippy`, `doctor`, `audit`, worktree bootstrap, and related shortcuts).

### Changed

- Statusbar and modal which-key hints drop the reverse-video badge for a flat "accent bind + muted action" treatment; action buttons keep their chip style. The Keybindings and Command Logs overlays now scroll their body only, keeping the title and footer hints pinned, with a herdr-style scrollbar. Command Logs separates entries with a full-width dashed rule, uses the flat footer hint, and copies the whole transcript to the clipboard with `y`.

### Fixed

- Settings free-text fields (worktree base/patterns, open shell/editor commands) persist as TOML strings, so a value like `123` or `true` is no longer coerced to a number/bool; config writes also validate before touching disk, so a rejected edit can never overwrite a good config file — while an edit to an already-invalid file is still written (then surfaces the error) so `gwm config set` can recover a broken config rather than refusing every edit.
- Scrollable modal bodies (Keybindings, Command Logs, Settings) compute the horizontal-pan bound after reserving the scrollbar column, so the final cell of a long line stays reachable.
- Delete-worktree confirmation no longer blocks the TUI render loop while the removal runs; the async spine drives the deletion and quit waits for it like other mutating tasks.
- Create-worktree submission no longer blocks the TUI render loop while `worktree::add` and bootstrap run; the Create modal now shows a dedicated loader/failure row and the global statusline spinner stays active until completion.
- TUI quit now waits for in-flight mutating spine tasks (`sync` / `bootstrap` / delete-worktree) to finish instead of abandoning them mid-operation.

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
