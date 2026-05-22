# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Declarative GitHub labels** ([#81](https://github.com/kbrdn1/gwm-cli/issues/81)). New `[[labels]]` table in `.gwm.toml` declares the desired GitHub label set (name + optional description / color), plus a new subcommand:
  - `gwm labels list` — print the resolved set and the diff against the `origin` remote (`+ create`, `~ update`, `= match`, `- extra-on-remote`).
  - `gwm labels push` — apply the diff via `gh label create --force`. `--dry-run` shows the plan without mutating the remote (it still reads remote labels via `gh label list` to compute the diff; only create / update / delete calls are skipped); `--prune` opt-in deletes labels on the remote that aren't declared in config (destructive, off by default); `--random-colors` picks a random pastel for labels with no `color` field instead of the default deterministic-hash colour.
  - Colour resolution: when `color` is omitted, gwm derives a deterministic pastel from an FNV-1a hash of the name, so the same label gets the same colour across repos. Hex normalisation accepts `#D73A4A` and round-trips to `d73a4a`.
  - Without a `[[labels]]` block in `.gwm.toml`, both subcommands are no-ops (`0 labels declared, nothing to push`) and never shell out to `gh` — safe to run in repos that haven't opted in.
  - Requires `gh` on `$PATH` (already a soft dependency of `gwm status`).
- ✨ `[[branch_types]]` block in `.gwm.toml` overrides the built-in allowed branch types. Absent or empty block ⇒ historical defaults (`feat`, `fix`, `hotfix`, `docs`, `test`, `refactor`, `chore`, `perf`, `ci`, `build`); present ⇒ only the listed types are accepted by `gwm create`, the TUI create picker and `BranchSpec::validate()`. `gwm types` prints the resolved list with a `(source: built-in defaults | .gwm.toml)` footer and aligns columns on the longest name. Invalid-type errors now enumerate the repo-local allowed list verbatim instead of leaking the hardcoded set. Entries are validated at load time: `name` must be non-empty, match `^[a-z]+$`, and be unique. (#80)
- **Declarative GitHub milestones** ([#82](https://github.com/kbrdn1/gwm-cli/issues/82)). New `[[milestones]]` table in `.gwm.toml` declares the desired GitHub milestone set (title + optional description / `due_on` / `state`), plus a new subcommand mirroring `gwm labels`:
  - `gwm milestones list` — print the resolved set and the diff against the `origin` remote (`+ create`, `~ update`, `= match`, `- extra-on-remote`).
  - `gwm milestones push` — apply the diff via `gh api repos/:owner/:repo/milestones` (POST for new entries, PATCH for updates). No native `gh milestone` subcommand exists, so we shell out to the REST API directly. `--dry-run` shows the plan without mutating the remote; `--prune` opt-in deletes milestones on the remote that aren't declared in config (destructive, off by default).
  - `due_on` accepts both `YYYY-MM-DD` (materialised as end-of-day UTC, common-sense "due Friday" semantic) and full RFC3339 (`2026-07-15T17:00:00Z`, canonicalised to UTC `Z` so non-UTC offsets don't flip-flop the diff); `state` defaults to `"open"`, opt in to `"closed"` for archived sprints.
  - Without a `[[milestones]]` block in `.gwm.toml`, both subcommands are no-ops (`0 milestones declared, nothing to push`) and never shell out to `gh` — same safe-by-default contract as labels.
  - Requires `gh` on `$PATH`.

### Tests

- ✅ E2E coverage for the mutating subcommands (#101). `tests/cli_binary.rs` now exercises `gwm init` (default body shape, idempotency refusal on existing `.gwm.toml`, repo-bound contract), `gwm create` (worktree dir + branch creation at HEAD, `branch.<name>.gwm-base` recorded for the launcher fallback chain, `[[bootstrap.copy]]` runs by default but is skipped under `--no-bootstrap`, validation rejects unknown branch types and non-digit issue numbers), and `gwm remove` (deletes the worktree dir, `--delete-branch` drops the local branch, unknown patterns fail loudly). All worktree-creating tests pin `[worktree].base` to a `tempfile::TempDir` so the CI runner never writes under `~/cc-worktree/...`.
- ✅ Characterization tests in `tests/worktree_integration.rs` for issues [#98](https://github.com/kbrdn1/gwm-cli/issues/98) and [#99](https://github.com/kbrdn1/gwm-cli/issues/99). `add_silently_attaches_to_pre_existing_stale_branch` pins the current (buggy) reuse behaviour of `worktree::add` when the target branch already exists — a fix for #99 will turn this test red and force the reviewer to confirm the contract change. `remove_prunes_admin_files_on_happy_path` pins the post-condition that `.git/worktrees/<name>` is gone after a successful `worktree::remove` — the post-condition that #98's reorder fix must preserve.

## Past releases

In reverse chronological order:

- [`0.6.0`](changelogs/0.6.0.md) — 2026-05-21
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
