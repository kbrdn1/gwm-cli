# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Dependencies

- ⬆️ `git2` `0.20` → `0.21` ([#169](https://github.com/kbrdn1/gwm-cli/issues/169)). A breaking source migration, not just a version bump: git2 0.21 reworks the UTF-8 accessors `Reference::shorthand` / `Reference::name` / `Remote::url` / `Buf::as_str` to return `Result<&str, git2::Error>` (was `Option<&str>`), `Commit::summary` to `Result<Option<&str>, _>`, and `StringArray::iter` to yield `Result<Option<&str>, _>`; `Oid::zero` is deprecated in favour of the `Oid::ZERO_SHA1` constant. All call sites now collapse the new `Err` arm to `None` via `.ok()` so the observable behaviour (a non-UTF-8 / absent name is treated as missing) is preserved. Side effect: git2 0.21 drops its `url` dependency, pruning the whole `idna` / `icu_*` transitive tree from `Cargo.lock`. Closes Dependabot [#157](https://github.com/kbrdn1/gwm-cli/pull/157).

### Docs

- 📝 Sync the root `README.md` landing page to the v0.8.0 surface and fix the MSRV badge (`1.80+` → `1.82+`) ([#203](https://github.com/kbrdn1/gwm-cli/issues/203)). The "what gwm does" list now covers the config CLI + user-level global config, lifecycle hooks, CLI aliases + Gitmoji, the GitHub `gwm new` / `gwm pr` workflow with PR auto-detection, safety-daily (`--dry-run`, `gwm undo` / `gwm history`), `gwm sync`, and TUI personalisation (themes / remappable keymap / command palette / stashes). Follow-up to the #199 docs refresh — landing page only, every feature keeps its dedicated section under `docs/`.

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
