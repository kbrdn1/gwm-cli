# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Docs

- 📝 Refresh the `docs/` tree to the full v0.8.0 surface and add a French i18n mirror under `docs/fr/` ([#199](https://github.com/kbrdn1/gwm-cli/issues/199)). Documents every v0.7.0 → v0.8.0 addition that was still missing or only listed as planned: `gwm commit-prefix`, `gwm hooks install commit-msg`, `gwm undo` / `gwm history`, `gwm theme list/show`, `gwm tui keys`, `gwm types --gitmoji`, and `--dry-run` on `gwm remove` / `gwm prune`; the role-based `[theme]` presets, the remappable `[tui.keys]` keymap with chord support, the `:` command palette, the sidebar stashes mode, and the rc.5 TUI chrome polish; the `[theme]` / `[tui.keys]` schema, the user-level global config (`~/.config/gwm/config.toml`), and the `{repo_path}` / `{repo_parent}` placeholders; the 8th `gwm doctor` keymap check, `cargo binstall` packaging, and the PR auto-detection provenance. Adds three pages (`tui/themes`, `tui/keymap-and-palette`, `configuration/global-config`), fixes typos / capitalisation / cross-links throughout, and ships a complete French translation (33 pages, English remains canonical).

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
