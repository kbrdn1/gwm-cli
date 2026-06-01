# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- 🎨 Add the `name` and `path` theme roles, promoting the last structural chrome the #170 audit left on hard-coded `Color::White` / `Color::Gray` ([#210](https://github.com/kbrdn1/gwm-cli/issues/210), follow-up to [#170](https://github.com/kbrdn1/gwm-cli/issues/170) / [#33](https://github.com/kbrdn1/gwm-cli/issues/33)). `name` (default `White`, bold) colours the worktree name in the table and the sidebar header plus the `Issue #N` / `PR #N` summary heads; `path` (default `Gray`) colours the table's path column — a structural mid-grey kept distinct from `muted` (`DarkGray`). Each default equals the previous literal, so users who omit `[theme]` see no change, and both roles are overridable / preset-driven like any other (all four built-in presets now define them — the claude-dark chrome maps to the palette's `--text` / `--text-dim` ([#214](https://github.com/kbrdn1/gwm-cli/issues/214)) — and `gwm theme show` dumps them). The sidebar identity-card path intentionally stays on `muted` — moving it to `path` would shift its default appearance. The genuinely non-name structural `Color::White` (help/step labels, the not-yet-fetched dot) and `Color::Reset` (unlinked marker) still carry no role. Pinned by `tests/tui_theme_audit_tests.rs` (`worktree_name_style` / `worktree_path_style` and the summary heads resolve through the new roles, with a default-preservation guard).
- 🎨 Add the `staged`, `modified`, and `untracked` theme roles for the working-tree status panel ([#211](https://github.com/kbrdn1/gwm-cli/issues/211), follow-up to [#170](https://github.com/kbrdn1/gwm-cli/issues/170) / [#33](https://github.com/kbrdn1/gwm-cli/issues/33)). `working_tree_status_line` previously borrowed the nearest semantic role — staged → `accent`, worktree-modified → `dirty`, untracked → `clean` — so recolouring a focus highlight (`accent`) silently recoloured staged files, and a branch-divergence warning (`dirty`) recoloured modified files. The three families now read their own roles (defaults `Cyan` / `Yellow` / `Green`, equal to the colours they used to borrow), decoupling the signals. All four built-in presets seed the new roles from their previous accent/dirty/clean so a preset's working-tree panel is unchanged; `gwm theme show` dumps them. Badges (PR / issue `●`) deliberately keep their existing semantic mapping. Pinned by `tests/tui_theme_audit_tests.rs`, whose status-family fixture now asserts the roles are distinct from accent/dirty/clean (so a regression to the borrow fails) plus a default-preservation guard.

### Fixed

- 🎨 Thread the resolved `[theme]` through every `draw_*` / render helper so role overrides apply everywhere ([#170](https://github.com/kbrdn1/gwm-cli/issues/170), follow-up to [#33](https://github.com/kbrdn1/gwm-cli/issues/33) / #168). Before this audit the theme reached `App.theme` but several render sites still painted hard-coded `Color::*` literals, so overriding a role (`branch`, `dirty`, `locked`, …) left them unchanged: the worktree table (marker, branch/status cells, age, selection background, column header), the sidebar identity card (branch line, badges, created/freshness, path), the working-tree status lines, the commit-graph nodes/connectors (now the `branch` role), recent-commits hash/initials, stash list, the `/` filter prompt, the header picker chip + path, the footer hint labels + status, and the issue/PR badge colours. Each literal maps to the role whose default equals it, so users who omit `[theme]` see no change. At audit time the structural `Color::White` (primary text), `Color::Gray` (table path), and `Color::Reset` (unlinked marker) carried no semantic role; the White / Gray chrome was subsequently promoted to the dedicated `name` / `path` roles in the same release (see #210 under **Added**), leaving only `Color::Reset` and a few non-name labels unthemed. The audit also closes a latent inconsistency it surfaced: a closed issue's summary-line badge used `Color::Red` while the sidebar header dot used `Color::Magenta` — both now resolve through `issue_badge_color` to the `locked` role so they always agree. Pinned by `tests/tui_theme_audit_tests.rs`, which drives each site with a synthetic per-role palette and asserts the resolved style.

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
