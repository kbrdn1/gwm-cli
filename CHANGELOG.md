# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Docs

- `CLAUDE.md` + `CONTRIBUTING.md` §Releases — new "Step 0: reconcile open PRs" rule. Before any RC or stable cut, run `gh pr list --state open` and reconcile every open PR (in the changeset, intentionally deferred, or closed as stale). Codifies the lesson from the v0.3.0 cut, which shipped without three queued feature PRs (#51, #52, #53) and required an immediate v0.4.0 promotion 38 minutes later. Step 0 sits explicitly at the top of both the pre-release and stable-release procedures.
- `README.md` + `gwm shell-init` output + `gwm switch --help` — document the `gcd` two-paths routing ([#58](https://github.com/kbrdn1/gwm-cli/issues/58)). The bare `gcd` (no argument) call routes to `gwm switch` (interactive picker), while `gcd <pattern>` routes to `gwm cd <pattern>` (fuzzy resolve); both branches wait on a successful exit code before performing the `cd`. The bridge was wired up in PR [#53](https://github.com/kbrdn1/gwm-cli/pull/53) but lived only in the code — fresh users reading the README, the eval'd wrapper, or `gwm switch --help` now see the no-arg route surfaced consistently across all three surfaces. New `tests/cli_binary.rs::shell_init_*_header_documents_no_arg_route` + `switch_help_mentions_gcd_wrapper` pin the docstring so future drift breaks the suite instead of going unnoticed.

## Past releases

In reverse chronological order:

- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
