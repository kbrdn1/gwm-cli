# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- TUI keybinding `o` reveals the selected worktree's directory in the OS file manager (`open` on macOS, `xdg-open` on Linux, `explorer` on Windows).
- `WorktreeInfo.status` (`BranchStatus`): dirty / clean / upstream-tracked / ahead / behind, computed via libgit2.
- `STATUS` column in both the TUI table and `gwm list` CLI output, with colour-coding (`green` clean / synced, `yellow` dirty or behind, `cyan` ahead, `red` prunable, `magenta` locked, `dark_gray` unknown).
- CI on the `dev` integration branch: `fmt`, `clippy`, multi-OS test, and `cargo audit` now run on every push to `dev` and on every PR targeting `dev` (same matrix as `main`).
- `.github/workflows/pre-release.yml`: builds the 5 release targets (Linux x86_64 + aarch64, macOS Intel + Apple Silicon, Windows x86_64) and publishes a GitHub Release with `prerelease: true` whenever a SemVer-rc / -alpha / -beta tag is pushed (e.g. `v0.2.0-rc.1`). Also supports `workflow_dispatch` for manual reruns.
- **TUI details sidebar** (auto-shown when terminal width ≥ 120 cols, toggle with `v`): lazyssh-style panel listing the selected worktree's branch, path, head, locked / prunable / main flags, status, plus `git log --oneline -n 10`, `git status --short`, and a commands cheat-sheet.
- **TUI keybinding `l`**: suspend the TUI and launch `lazygit -p <selected-worktree-path>` fullscreen; resume the TUI when lazygit exits. Surfaces a clear error in the status bar if `lazygit` is not on `$PATH`.
- **TUI focus toggle (`Tab`)**: swap focus between the worktree list and the sidebar. `j` / `k` (and arrows) scroll the focused panel; the focused panel's border turns cyan.
- **TUI vim motions**: `gg` jumps to the first worktree, `G` to the last.
- `worktree::git_log_oneline(path, n)` and `worktree::git_status_short(path)` — thin `Command::new("git")` wrappers used by the sidebar (and exposed for tests).
- `.gwm.toml` config for this repo (Rust-flavoured): `target/` no_symlink + `cargo fetch` bootstrap step + `direnv allow` when an `.envrc` exists.

### Changed

- TUI worktree view: ratatui `List` → `Table` with dynamic column widths derived from data (name/branch capped to [18, 38], status fixed 16, path takes the rest). No more `…`-truncated names for typical branch lengths.
- `gwm list` CLI output uses the same dynamic column widths.
- TUI list `j` / `k`: now reset the sidebar scroll offset when navigating worktrees, and scroll the sidebar instead of moving selection when the sidebar has focus.

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
