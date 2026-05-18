# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- TUI keybinding `o` reveals the selected worktree's directory in the OS file manager (`open` on macOS, `xdg-open` on Linux, `explorer` on Windows).
- `WorktreeInfo.status` (`BranchStatus`): dirty / clean / upstream-tracked / ahead / behind, computed via libgit2.
- `STATUS` column in both the TUI table and `gwm list` CLI output, with colour-coding (`green` clean / synced, `yellow` dirty or behind, `cyan` ahead, `red` prunable, `magenta` locked, `dark_gray` unknown).

### Changed

- TUI worktree view: ratatui `List` → `Table` with dynamic column widths derived from data (name/branch capped to [18, 38], status fixed 16, path takes the rest). No more `…`-truncated names for typical branch lengths.
- `gwm list` CLI output uses the same dynamic column widths.

## [0.1.0] - 2026-05-18

### Added

- Native git worktree management via libgit2 (vendored), no `gwq` / `git` CLI dependency.
- CLI subcommands: `init`, `list`, `create`, `remove`, `path`, `bootstrap`, `prune`, `types`.
- TUI (ratatui) with views: List, Create form, Confirm delete, Bootstrap report, Help.
- Per-repo configuration via `.gwm.toml`:
  - `[worktree]` — `base`, `path_pattern`, `branch_pattern` with `{home}/{repo}/{type}/{issue}/{desc}` placeholders.
  - `[[bootstrap.copy]]` — file copies main → worktree, with optional inline fallbacks.
  - `[[bootstrap.guard]]` — regex deny patterns with `abort` or `seed-from-example` actions.
  - `[[bootstrap.no_symlink]]` — paths that must never be symlinks (e.g. `vendor`, `node_modules`).
  - `[[bootstrap.command]]` — shell commands with `when = "file_exists:..."` predicates and env injection.
- Branch convention `<type>/#<issue>-<desc>` (matches the original bash script) with override via `.gwm.toml`.
- Fuzzy worktree lookup (`gwm remove <substring>`, `gwm path <substring>`).
- 56 tests covering config, naming, bootstrap, worktree (libgit2 integration), TUI state machine, and CLI end-to-end (assert_cmd).
- Examples: `examples/gwm.toml.example` with the AWS RDS guard pattern from the original script's incident.

### Notes

This is the initial release. Behaviour and config schema may still change before `1.0`.
