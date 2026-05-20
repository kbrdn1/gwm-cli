# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Issue / PR linking** ([#67](https://github.com/kbrdn1/gwm-cli/issues/67)) — link the current worktree to a GitHub issue and / or pull request, open them in the browser, and surface their status in the TUI details panel.
  - New CLI subcommands: `gwm link {issue|pr} <N>`, `gwm unlink {issue|pr}`, `gwm open {issue|pr} [--print-url]`, `gwm status [--json]`.
  - Storage: `git config branch.<name>.gwm-issue` and `branch.<name>.gwm-pr` (local, per-branch, survives worktree moves).
  - Auto-detection: branches following `<type>/#<N>-<slug>` derive the issue automatically; explicit `gwm link issue <N>` overrides.
  - TUI key bindings: `O` open menu (issue / pr), `L` link prompt, `R` refresh GitHub status.
  - Right-panel status: live issue state (open / closed) and PR state (open / draft / closed / merged) with CI check rollup (`checks N/M`). Fetched via `gh issue view` / `gh pr view`.

### Dependencies

- Add `serde_json = "1"` for parsing the `gh` CLI JSON output.

## Past releases

In reverse chronological order:

- [`0.5.0`](changelogs/0.5.0.md) — 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md) — 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md) — 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
