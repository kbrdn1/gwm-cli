# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Extended `[[bootstrap.command]].when` predicates** — the evaluator now recognises four additional keyword atoms alongside the legacy `file_exists:`: `cmd_exists:<binary>` (resolves via `which`), `env_set:<NAME>` (variable defined), `env_eq:<NAME>=<value>` (variable equals literal), `glob_exists:<pattern>` (recursive `**` glob match under the worktree). Atoms compose with `!`, `&&`, `||` at the conventional precedence (`!` > `&&` > `||`); no parentheses. Example: `when = "file_exists:package.json && cmd_exists:bun"` picks `bun install` only when bun is on PATH. `bootstrap::evaluate_when` is now part of the public lib surface. `gwm doctor` recognises the new keywords; unknown keywords still default to true. Closes #25.

### Fixed

- `gwm doctor`: `is_writable_dir` now uses a random-suffixed `tempfile`-managed probe file (was a fixed `.gwm-doctor-write-probe` colliding under concurrent runs and leaking on SIGKILL). Closes #54.
- `gwm doctor`: `extract_binary` parses shell-quoted run-strings via `shell-words` (was `split_whitespace`, which sliced through quotes and produced false-positive "binary not on PATH" warnings on configs like `run = "\"my tool\" --flag"`). Closes #54.

### Changed

- `gwm cd <pattern>` is now exposed as a clap `visible_alias` of `gwm path` instead of a separate `Command::Cd` variant. Same UX, one routing path. The `--help` listing shows `path … [aliases: cd]`. Closes #54.

### Refactored

- `doctor::Severity` collapsed into `CheckStatus` (the two enums were structurally identical). A `pub type Severity = CheckStatus;` alias is kept so 0.3.0 library callers keep compiling. Closes #54.
- `doctor::run` now hoists `worktree::list` once and passes `&[WorktreeInfo]` into the two checks that need it (`prunable` + `orphan`), saving a libgit2 call per `gwm doctor` invocation and unifying the error-handling path. Closes #54.
- `check_when_predicates`: counter renamed from `checked` to `recognised` and incremented only after the prefix match, so the variable name accurately tracks what the `"N predicate(s) recognised"` detail reports. Closes #54.

### Tests

- `doctor_on_fresh_repo_prints_checks`: exit code is now bounded to `[0, 1]` (was unbounded, a panic / SEGV / exit-2 regression would have passed silently).
- `shell_init_{bash,zsh,fish,powershell}_emits_*`: assertions tightened to pin the actual invocation (`gwm cd "$@"` / `gwm cd $argv` / `gwm cd $Pattern`) rather than the loose `contains("gwm cd")` which would have passed even if `gwm cd` appeared only in a comment.
- `resolvable_command_binary_is_ok`: the loose `!contains("sh ")` assertion (would have passed on `[sh,other]` formatting) is replaced with structured parsing of the "not on PATH:" list.

### Docs

- README: doctor sample output updated to reflect the post-#47 `✓ N merged gwm-style branch(es) preserved` wording; a second block shows the Warning-with-hint case so users see both happy and remediation paths.
- README: test count refreshed from a stale "81" to the actual 140.
- New `changelogs/pre-releases/` directory holding per-RC notes (one file per RC, covering only the delta vs. the previous RC). Back-fills `0.2.0-rc.1`, `0.3.0-rc.1`, `0.3.0-rc.2`, `0.3.0-rc.3`. `CHANGELOG.md > Past releases` gains a `### Pre-releases` sub-index. The corresponding GitHub Release bodies are re-pointed at these files via `gh release edit --notes-file` (instead of inheriting the full `CHANGELOG.md` as before).
- `CONTRIBUTING.md > Pre-release (from dev)` describes the new per-RC notes flow; `Stable release (from main)` mentions `--notes-file` for the body.

### Dependencies

- `shell-words` `1` (new, runtime) — POSIX shell tokeniser used by the doctor's `extract_binary`.
- `tempfile` moved from `[dev-dependencies]` to `[dependencies]` — used at runtime by the doctor's `is_writable_dir` write-probe.
- `glob` `0.3` (new) — backs the `glob_exists:` predicate in `bootstrap::evaluate_when`.

## Past releases

In reverse chronological order:

- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
