# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **TUI fuzzy filter (`/`)** — press `/` on the worktree list to open an inline filter bar at the bottom of the table. As you type, the table narrows in real time via [`nucleo-matcher`](https://docs.rs/nucleo-matcher) (same matcher as Helix / Zellij), ranking contiguous substring hits above spread-out subsequence hits. `Enter` confirms (filter sticks, navigation returns to the table); `Esc` clears the filter and restores the full list. `j` / `k` / `gg` / `G` continue to work on the filtered subset. Table title shows `worktrees (N/M)` while a filter is active. `Esc` on the plain list view clears any sticky filter before it considers quitting, so a stale filter can't accidentally exit the TUI. Closes #21.
- `gwm completions <shell>` — prints a static completion script on stdout, generated from the live clap argument tree via [`clap_complete`](https://docs.rs/clap_complete). Supported shells: `zsh`, `bash`, `fish`, `powershell`, `elvish`. Closes #18.
- `gwm list --format=names` — prints one worktree name per line (no header, no marker, no STATUS column). Suitable for backing dynamic completion of the `<pattern>` arg of `path` / `remove` / `bootstrap` (see the README "shell completions" section for a zsh wiring example).
- `gwm cd <pattern>` — fuzzy-resolve a worktree and print its on-disk path. Same semantics as `gwm path`, exposed under an explicit name for the cd flow.
- `gwm shell-init <bash|zsh|fish|powershell>` — prints a shell wrapper defining `gcd <pattern>` (the function does the actual `cd`, since the binary can't change the parent shell's directory). One-liner install: `eval "$(gwm shell-init zsh)"` in your rc file → `gcd auth` jumps to the matching worktree. The bash/zsh and PowerShell variants `unalias gcd` first so the function takes effect even if the shell already had a `gcd` alias (e.g. oh-my-zsh's `gcd=git checkout`). Closes #19.

### Docs

- `CLAUDE.md` (new, repo root) — house rules for AI-assisted contributions. Promotes **TDD as the primordial contribution rule** (red → green → refactor, mandatory failing test before production code).
- `CONTRIBUTING.md` — `TDD expectations` section rewritten as `🔴 TDD is mandatory — non-negotiable`: explicit loop, narrow exceptions, reviewer enforcement via `git log --stat tests/`.
- `CODE_OF_CONDUCT.md` — new `Engineering conduct` section anchoring the TDD rule as a contribution-conduct expectation (applies equally to human and AI-assisted PRs).

### Dependencies

- `clap_complete` `4.5` (new).
- `nucleo-matcher` `0.3` (new) — fuzzy match engine for the TUI `/` filter.

## [0.2.0] - 2026-05-18

Validated via `v0.2.0-rc.1` (pre-release published on 2026-05-18).

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
- `dependabot.yml`: `target-branch: dev` on both ecosystems so new automated PRs land on the integration branch instead of `main`.

### Changed

- TUI worktree view: ratatui `List` → `Table` with dynamic column widths derived from data (name/branch capped to [18, 38], status fixed 16, path takes the rest). No more `…`-truncated names for typical branch lengths.
- `gwm list` CLI output uses the same dynamic column widths.
- TUI list `j` / `k`: now reset the sidebar scroll offset when navigating worktrees, and scroll the sidebar instead of moving selection when the sidebar has focus.
- Sidebar content is now cached on the `App` (keyed by selected worktree's path); `git log` / `git status` only run when the selection actually changes. Sidebar scrolling clamps to the rendered content height.
- Windows `.sha256` sidecars (release + pre-release workflows) now use `Set-Content -Encoding ascii` and the conventional `<hash>  <file>` format so they verify cleanly with `sha256sum -c` across platforms.

### Dependencies

- `ratatui` `0.28` → `0.30` (and `crossterm` `0.28` → `0.29`). Renamed `Table::highlight_style` → `row_highlight_style` to track the deprecation.
- `git2` `0.19` → `0.20`.
- `toml` `0.8` → `1.1`.
- `thiserror` `1` → `2`.
- `which` `6` → `8`.
- `actions/checkout` `4` → `6`, `actions/upload-artifact` `4` → `7`, `actions/download-artifact` `4` → `8`, `softprops/action-gh-release` `2` → `3`.

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
