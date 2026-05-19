# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`gwm switch` (alias `gwm s`)** — interactive picker that opens the worktree TUI in a stripped-down "pick one" mode. The fuzzy filter bar opens immediately so typing narrows the list right away; `Enter` commits the highlighted worktree and prints its path on stdout (exit `0`), `Esc` / `Ctrl-C` / `q` quits without output (exit `1`). The create / delete / bootstrap actions (`n` / `d` / `b` / `p`) are inert in picker mode; navigation (`j` / `k` / `gg` / `G`), sidebar (`v` / `Tab`), filter (`/`), `o` (open in finder), `l` (lazygit), `r` (refresh), and `?` (help) remain available. Header carries a `[picker]` tag and the help / footer adapt their key cheatsheet to match. Daily flow: `cd "$(gwm switch)"`. Closes #22.
- **TUI fuzzy filter (`/`)** — press `/` on the worktree list to open an inline filter bar at the bottom of the table. As you type, the table narrows in real time via [`nucleo-matcher`](https://docs.rs/nucleo-matcher) (same matcher as Helix / Zellij), ranking contiguous substring hits above spread-out subsequence hits. `Enter` confirms (filter sticks, navigation returns to the table); `Esc` clears the filter and restores the full list. `j` / `k` / `gg` / `G` continue to work on the filtered subset. Table title shows `worktrees (N/M)` while a filter is active. `Esc` on the plain list view clears any sticky filter before it considers quitting, so a stale filter can't accidentally exit the TUI. Closes #21.
- `gwm completions <shell>` — prints a static completion script on stdout, generated from the live clap argument tree via [`clap_complete`](https://docs.rs/clap_complete). Supported shells: `zsh`, `bash`, `fish`, `powershell`, `elvish`. Closes #18.
- `gwm list --format=names` — prints one worktree name per line (no header, no marker, no STATUS column). Suitable for backing dynamic completion of the `<pattern>` arg of `path` / `remove` / `bootstrap` (see the README "shell completions" section for a zsh wiring example).
- `gwm cd <pattern>` — fuzzy-resolve a worktree and print its on-disk path. Same semantics as `gwm path`, exposed under an explicit name for the cd flow.
- `gwm shell-init <bash|zsh|fish|powershell>` — prints a shell wrapper defining `gcd <pattern>` (the function does the actual `cd`, since the binary can't change the parent shell's directory). One-liner install: `eval "$(gwm shell-init zsh)"` in your rc file → `gcd auth` jumps to the matching worktree. The bash/zsh and PowerShell variants `unalias gcd` first so the function takes effect even if the shell already had a `gcd` alias (e.g. oh-my-zsh's `gcd=git checkout`). Closes #19.
- `gwm doctor` — diagnose the gwm setup. Aggregates 7 cheap checks (`.gwm.toml` parses, guard references resolve, `when` predicates supported, external binaries on PATH, no prunable worktrees, no orphan gwm branches, base directory writable) and reports each with `✓ / ! / ✗`. Exit code `0` (all green), `1` (any warning), `2` (any failure) — wirable into CI / pre-commit. New `src/doctor.rs` module exposing `DoctorReport` / `Check` / `Severity` / `DoctorCtx` for library users. Closes #20.

### Changed

- `gwm doctor` no longer flags gwm-style branches as orphan when they're already fully merged into one of the trunk branches (`dev`, `main`). CONTRIBUTING.md mandates preserving the source branch post-merge, so the previous behaviour produced N false-positives on every successful release. The Ok detail now reads e.g. `7 merged gwm-style branch(es) preserved per CONTRIBUTING, no unmerged orphans`. Genuine WIP branches (no worktree, no merge into a trunk) still surface as Warning. Closes #47.

### CI

- New advisory job `gwm doctor` in `.github/workflows/ci.yml` — runs `cargo build --release` then `./target/release/gwm doctor` against the repo on every push to `dev` and on every PR targeting `dev`. `continue-on-error: true` so it never blocks a merge, but a non-zero exit surfaces config / env / worktree-state regressions for human review. Closes #49.

### Docs

- `CLAUDE.md` (new, repo root) — house rules for AI-assisted contributions. Promotes **TDD as the primordial contribution rule** (red → green → refactor, mandatory failing test before production code).
- `CONTRIBUTING.md` — `TDD expectations` section rewritten as `🔴 TDD is mandatory — non-negotiable`: explicit loop, narrow exceptions, reviewer enforcement via `git log --stat tests/`.
- `CODE_OF_CONDUCT.md` — new `Engineering conduct` section anchoring the TDD rule as a contribution-conduct expectation (applies equally to human and AI-assisted PRs).
- `CHANGELOG.md` trimmed to the in-progress release only; past releases moved under `changelogs/`.
- `CLAUDE.md` — two new house rules: pre-validate environment-dependent tests with a stripped `PATH` before push (one-liner included), and run `gwm doctor` locally on PRs that touch `.gwm.toml` / bootstrap / doctor. Codifies the recipe that would have spared the 3 CI round-trips on PR #43.

### Dependencies

- `clap_complete` `4.5` (new).
- `nucleo-matcher` `0.3` (new) — fuzzy match engine for the TUI `/` filter.

## Past releases

In reverse chronological order:

- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18
