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

### Security

- 🔒 **Guard `deny_patterns` compile at load time, never fail-open silently** ([#96](https://github.com/kbrdn1/gwm-cli/issues/96)). Closes a refusal-mechanism bypass on `[[bootstrap.guard]]`: `bootstrap.rs::guard_match` wrapped `Regex::new(pat)` in `if let Ok(re) = …`, silently dropping every invalid pattern. A guard whose only deny pattern failed to compile collapsed into "no patterns at all" — a `.env` containing the substring the user expected to be denied passed through with a green "guard passed" message. A refusal mechanism that silently refuses to refuse is strictly worse than no mechanism, because the user reads the report and trusts a file that never went through the rule.
  - **Load-time validation** in a new `Config::validate_bootstrap_guards()` helper iterates every `[[bootstrap.guard]].deny_patterns` entry and runs `Regex::new`, surfacing a `GwmError::Config` that quotes the guard name, the offending pattern (`{:?}`-formatted so control chars and regex metasyntax stay visible), and the underlying `regex::Error` message. Bootstrap never reaches `run` for a config with a broken pattern.
  - **Runtime defence-in-depth** in `bootstrap::guard_match` for `Config` values that bypass the loader (test fixtures, programmatic constructors, future APIs): a `Regex::new` failure at evaluation time now surfaces as a `StepStatus::Failed` step naming the guard, the offending pattern, and the issue reference — the copy is refused, instead of silently dropping the pattern as the original fail-open did. Closes the residual gap Copilot flagged on PR #116.
  - No CLI surface or `.gwm.toml` schema change for valid configurations.
- 🔒 **TOFU trust ledger on `.gwm.toml` + `--allow-bootstrap` / `--deny-bootstrap` flags** ([#95](https://github.com/kbrdn1/gwm-cli/issues/95)). Closes the arbitrary-RCE primitive against `gwm create` / `gwm bootstrap` on hostile clones: until this lands, running `gwm` against a freshly cloned (or PR-fork) repo was equivalent to `curl … | sh` against the repo author, with no trust boundary between "I cloned this remote" and "this remote can execute commands as me".
  - **Ledger** at `~/.config/gwm/trust.toml` (overridable via `$GWM_TRUST_LEDGER`) records `(origin URL, sha256 of .gwm.toml, trusted_at, trusted_by)` tuples. First run on a repo with a `.gwm.toml` prints a summary of the bootstrap surface (copies, guards, no-symlinks, command lines) and prompts `Trust this .gwm.toml? [y/N/show]`. Subsequent runs against the same `(origin, hash)` pass silently. Any byte change to `.gwm.toml` re-prompts (whitespace included — `rm -rf /tmp/` and `rm -rf /tmp /` differ by one byte).
  - **New subcommand** `gwm trust` with three actions: `list` (audit the ledger), `revoke <origin>` (drop entries for an origin URL — verbatim match against the recorded form, SSH and HTTPS are distinct), `show` (print the active ledger path and contents).
  - **Bypass flags**: `--allow-bootstrap` (global, also `GWM_ALLOW_BOOTSTRAP=1`) skips the prompt without recording — for CI runners and scripted workflows where there is no human to answer. `--deny-bootstrap` refuses to run bootstrap even if the ledger says trusted — for forensic inspection of an unfamiliar repo.
  - **Non-interactive safety**: when stdin is not a tty and no `--allow-bootstrap` is in effect, `gwm` aborts with a clear message rather than hanging on a read that will never see input. The default-deny prevents a CI pipeline from silently running attacker code because the prompt got swallowed.
  - **No schema change to `.gwm.toml`** — purely an additive layer above the existing bootstrap pipeline. Existing repos get a single one-shot prompt after upgrade.
- 🔒 **`bootstrap` refuses to copy through symlinks at the destination** ([#93](https://github.com/kbrdn1/gwm-cli/issues/93)). Three changes that together close a write-anywhere primitive triggered by `gwm bootstrap` on an attacker-controlled checkout:
  - **Reorder**: `run_no_symlinks` now runs **before** `run_copies` in `bootstrap::run`. The previous ordering let a target declared in both `[[bootstrap.no_symlink]]` and `[[bootstrap.copy]]` be touched by the copy pass first — either silently skipping through a live symlink or writing through a dangling one before the no-symlink pass got a chance to strip it.
  - **Defence in depth in `run_copies`**: a new `symlink_metadata` check at the top of the loop refuses any step whose destination is a symlink (broken or live), regardless of whether the user declared `[[bootstrap.no_symlink]]` for it. Closes two failure modes the reorder alone doesn't: symlinks not declared in `[[no_symlink]]` (planted by manual migration, editor plugins, or an attacker), and the macroscopic TOCTOU window between the no-symlink pass and the copy loop.
  - **`O_NOFOLLOW`-based write primitives** (`bootstrap::copy_no_follow` / `bootstrap::write_no_follow`) replace every `fs::copy` / `fs::write` site under `run_copies`, `resolve_missing` (inline fallback), and `handle_guard_match` (seed-from-example). The destination file is opened with `O_NOFOLLOW | O_CREAT | O_EXCL` on unix, so even if a symlink materialises in the microseconds between the stat and the open, the syscall returns `ELOOP` (symlink) or `EEXIST` (other entry) instead of writing through. Source permissions are preserved on unix to match the prior `fs::copy` behaviour.
  - Observable contract change: a copy step whose destination is a symlink now reports `Failed` with a message naming the path and referencing #93. Pre-fix, the live-symlink case was a silent `Skipped` and the broken-symlink case was a `Failed` from `fs::copy` errno. No CLI surface or `.gwm.toml` schema change.
- 🔒 **`bootstrap` rejects path traversal in copy / guard / fallback fields** ([#94](https://github.com/kbrdn1/gwm-cli/issues/94)). Closes the write-anywhere / read-anywhere primitive flagged on `step.to`, `Guard.example_file`, and `FallbackContent.target` — values like `"../../etc/passwd"` or `"/etc/shadow"` previously slipped through `Path::join` and landed outside the worktree (or read outside the main repo). Two layers:
  - **Load-time validation** (`Config::load_for_repo` → `validate_bootstrap_paths`): rejects empty strings, absolute paths, and `..` traversal segments in the three fields, surfacing the violation with the exact TOML key (e.g. `bootstrap.copy[0].to`) so the user can fix the config without grepping. `bootstrap.copy[].from` is intentionally not validated — it's joined onto `ctx.main_repo` (the repo root that ships the config), same trust boundary as the file itself.
  - **Runtime defence-in-depth** (`bootstrap::ensure_within`): canonicalize-then-prefix-check on every resolved path in `run_copies` (against `ctx.worktree`) and on `example_file` in `handle_guard_match` (against `ctx.main_repo`). The deepest existing ancestor is canonicalized so the check catches `..` traversal, absolute paths, AND symlinks in intermediate components — closes a third attack vector the load-time validator alone misses (an attacker who can plant a symlink at a `dst` parent component redirects the resolved path even when the TOML string is benign).
  - Behavior on rejection: the step reports `Failed` with a detail that names the offending path and references issue #94. No CLI surface or `.gwm.toml` schema change for benign configurations.

### Fixed

- 🐛 **`R: review` no longer silently no-ops on push-tracked PR branches** ([#117](https://github.com/kbrdn1/gwm-cli/issues/117)). `resolve_review_base` now ignores `branch.<n>.merge` when it points at the branch itself. After the canonical `gwm create <type> <#> <slug> && git push -u origin <branch>` flow, git records `merge = refs/heads/<same-branch>` — the local branch tracks its own remote-side copy. The previous priority-1 chain step returned that name verbatim, making `git diff <branch>..<branch>` empty so the launcher's default `skip_when_no_changes = true` silently swallowed the `R` keystroke. The fix falls through to the next chain step (`branch.<n>.gwm-base` → `[review].default_base` → `dev` / `main`) when the upstream is self-referential, so the launcher hands `git diff` a ref that actually diverges from HEAD. The self-equality compare runs after `read_branch_merge` strips the `refs/heads/` prefix, so both the canonical refspec form and the bare short-name form are caught.

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
