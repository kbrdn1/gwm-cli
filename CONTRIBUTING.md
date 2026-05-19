# Contributing to gwm

Thanks for your interest in `gwm` — a Rust CLI / TUI for managing git worktrees across projects. This file describes the conventions used here. They mirror the ones used in [`fiches-pedagogiques-api-rest`](https://github.com/FlippadTeam/fiches-pedagogiques-api-rest/blob/dev/CONTRIBUTING.md) so the muscle memory is the same.

## Table of contents

- [About this repository](#about-this-repository)
- [Project layout](#project-layout)
- [Development](#development)
- [Testing](#testing)
- [Branches](#branches)
- [Commits](#commits)
- [Labels](#labels)
- [Pull Requests](#pull-requests)
- [Merge strategy](#merge-strategy)
- [Releases](#releases)

## About this repository

`gwm` is a single-binary Rust crate (`bin` + reusable `lib`):

- **bin** `gwm` — entry point: dispatches to subcommands (CLI) or opens the TUI.
- **lib** `gwm` — modules (`config`, `naming`, `worktree`, `bootstrap`, `cli`, `tui`, `error`) exposed publicly so integration tests in `tests/` can drive them directly.

It uses [`git2`](https://docs.rs/git2) (vendored libgit2) for worktree operations and [`ratatui`](https://docs.rs/ratatui) for the TUI.

## Project layout

```
gwm-cli/
├── Cargo.toml
├── CHANGELOG.md
├── CONTRIBUTING.md
├── LICENSE.md
├── README.md
├── examples/
│   └── gwm.toml.example
├── src/
│   ├── lib.rs            # public re-exports
│   ├── main.rs           # bin entry point
│   ├── error.rs
│   ├── config.rs         # .gwm.toml parsing
│   ├── naming.rs         # branch / path conventions
│   ├── worktree.rs       # libgit2 worktree ops
│   ├── bootstrap.rs      # copies / guards / shell hooks
│   ├── cli.rs            # clap subcommands
│   └── tui/
│       ├── mod.rs        # event loop
│       ├── app.rs        # state
│       └── ui.rs         # rendering
└── tests/
    ├── common/           # shared helpers (init_repo, paths_equal)
    ├── config_tests.rs
    ├── naming_tests.rs
    ├── bootstrap_tests.rs
    ├── worktree_integration.rs
    ├── tui_app_tests.rs
    └── cli_binary.rs     # assert_cmd end-to-end
```

All tests live under `tests/` — no inline `#[cfg(test)] mod tests` blocks inside `src/`.

## Development

### Prerequisites

- Rust toolchain (stable channel, 1.80+ — verified on 1.89).
- A C compiler (libgit2 is vendored and built from source on first `cargo build`).

### Build & run

```bash
git clone https://github.com/kbrdn1/gwm-cli.git
cd gwm-cli

cargo build              # builds bin + lib
cargo run -- list        # smoke test the CLI
cargo run                # opens the TUI in the current repo
cargo install --path .   # install gwm into ~/.cargo/bin
```

### Code style

- **Indentation**: 2 spaces (matches `fiches-pedagogiques` convention).
- **Formatter**: `cargo fmt` (project uses `rustfmt` defaults except indent).
- **Linter**: `cargo clippy -- -D warnings`.
- Run `cargo fmt && cargo clippy` before opening a PR.

## Testing

```bash
cargo test                              # run everything
cargo test --test config_tests          # one file
cargo test --test worktree_integration  # libgit2 integration
cargo test -- --nocapture               # see println from tests
```

### 🔴 TDD is mandatory — non-negotiable

**Test-Driven Development is the primary contribution rule of this repo.** No production code lands without a failing test that pinned the behaviour down first. This is not a guideline, it is a hard merge requirement. PRs that add or change behaviour without tests are sent back, full stop.

The loop is **red → green → refactor**:

1. **Red** — write a failing test capturing the new behaviour (or the bug you are fixing). Run it. It MUST fail for the right reason (assertion mismatch, not a compile error in unrelated code).
2. **Green** — write the minimum production code that turns the test green. No speculative abstractions.
3. **Refactor** — clean up under green tests. Re-run the suite after each refactor step.

Where the test lives:

- **unit logic** (config parsing, naming, kebab, guard regex) → tests in the matching `tests/*_tests.rs` file.
- **disk side effects** (file copy, symlink removal, command exec) → use `tempfile::TempDir`.
- **git operations** → use `tests/common::init_repo()` which gives you a fresh repo on `main` with one commit.
- **public CLI surface** → end-to-end test in `tests/cli_binary.rs` via `assert_cmd`.
- **bootstrap stages** (copy, guard, no-symlink, command) → `tests/bootstrap_tests.rs`.
- **TUI state transitions** → ratatui-free state-machine tests in `tests/tui_app_tests.rs`.

#### Exceptions (must be argued in the PR description)

The bar to skip a test is "observably untestable from the public surface":

- Pure formatting / typo fixes in incidental strings (not asserted anywhere).
- Dependency bumps with no behaviour change (CI green is the test).
- Comment-only changes.

Everything else needs a test. "I tested it manually" is not an exception — codify it as an integration test.

#### Enforcement

- Reviewers run `git log --stat <branch>..HEAD -- tests/`. If the touched module has no companion test diff and the change isn't one of the exceptions above, the PR is blocked.
- The `## Tests` checklist in the PR template is binding. Do not tick `cargo test` unless it actually ran green locally.
- `tests/cli_binary.rs::help_prints_subcommands` is the canary — update it whenever a new CLI subcommand is added.

## Branches

Main branches:

- `main` — what ships. Direct commits allowed for trivial maintenance (typos, docs, dep bumps). Anything user-visible goes through a PR.
- Feature branches: `<type>/#<issue-number>-<short-description>`.

Examples: `feat/#12-tui-search`, `fix/#45-locked-worktree-detection`, `docs/#3-update-readme`.

`gwm` itself uses this exact convention via `gwm create feat 12 tui-search`.

## Commits

Format: `<emoji> <type>(<scope>)<!>: <subject>` (Gitmoji + Conventional Commits).

### Types

| Type       | When                                                |
|:-----------|:----------------------------------------------------|
| `feat`     | new feature                                         |
| `fix`      | bug fix                                             |
| `hotfix`   | critical production bug fix                         |
| `refactor` | code restructuring, no behaviour change             |
| `docs`     | documentation only                                  |
| `test`     | adding / fixing tests                               |
| `perf`     | performance improvement                             |
| `chore`    | repo maintenance (deps, config, scripts)            |
| `ci`       | CI / GitHub Actions changes                         |
| `build`    | build system, Cargo manifest                        |

### Emojis (Gitmoji)

| Emoji | Type       |
|:------|:-----------|
| ✨    | feat       |
| 🐛    | fix        |
| 🚑️   | hotfix     |
| 📝    | docs       |
| ♻️    | refactor   |
| ⚡    | perf       |
| ✅    | test       |
| 🔧    | chore      |
| 🏗️    | build      |
| 👷    | ci         |
| 🔥    | chore (remove) |
| ⬆️    | chore (bump deps) |
| 🔒    | security   |

### Scopes (optional, used in this repo)

`config`, `naming`, `worktree`, `bootstrap`, `cli`, `tui`, `tests`, `docs`, `ci`, `structure`.

### Examples

- `✨ feat(tui): add fuzzy search on worktree list`
- `🐛 fix(worktree): handle is_prunable error gracefully`
- `🔧 chore(deps): bump ratatui to 0.29`
- `♻️ refactor(bootstrap): extract guard-matching into pure fn`
- `✅ test(naming): cover unicode descriptions`

### Breaking changes

Suffix the type with `!` and add a `BREAKING CHANGE:` footer:

```
✨ feat(config)!: replace `[[bootstrap.copy]]` with `[[steps]]`

BREAKING CHANGE: configs using the old keys must migrate to the new schema.
```

## Labels

See [`.github/LABELS.md`](.github/LABELS.md) for the full matrix. Quick reference:

- **type**: `feature`, `fix`, `hotfix`, `docs`, `test`, `refactor`, `chore`, `perf`, `ci`, `build`
- **status**: `duplicate`, `invalid`, `wontfix`
- **domain**: `cli`, `tui`, `config`, `worktree`, `bootstrap`, `security`, `dependencies`

## Pull Requests

Before opening a PR:

- [ ] `cargo fmt`
- [ ] `cargo clippy -- -D warnings`
- [ ] `cargo test` (all green)
- [ ] CHANGELOG.md updated under `## [Unreleased]`
- [ ] If the public CLI changed: README usage section updated
- [ ] If the config schema changed: `examples/gwm.toml.example` and the README section updated

Use the PR template (`.github/PULL_REQUEST_TEMPLATE.md`).

## Merge strategy

- **Never squash**. Use a regular merge commit so the atomic commit history (with its `feat` / `fix` / `refactor` labels) is preserved on `main`.
- **Never delete the source branch** after merge. Keeps traceability and lets us cherry-pick / revert.

```bash
gh pr merge <num> --merge   # NOT --squash, NOT --delete-branch
```

## Releases

Versioning is SemVer (`MAJOR.MINOR.PATCH`), with `-rc.N` / `-alpha.N` / `-beta.N` suffixes for pre-releases cut from `dev`.

- `MAJOR` → breaking change
- `MINOR` → new feature
- `PATCH` → bug fix
- `-rc.N` / `-alpha.N` / `-beta.N` → release candidate / alpha / beta cut from `dev` before promotion to `main`

### Step 0 — Reconcile open PRs (applies to every tag)

Before any RC or stable cut, run:

```bash
gh pr list --state open
```

Every open PR must be in exactly one of these buckets:

- **In the changeset** — merged into the source branch (`dev` for RCs / stables, `main` for hotfixes) before tagging.
- **Intentionally deferred** — won't make this release, will land in a later one. Note why in the release notes if it was a known candidate.
- **Closed as stale** — superseded, obsolete, or duplicate. Close with a one-line comment pointing at the supersession.

Skipping this step caused the v0.3.0 cut to ship without three queued feature PRs (#51, #52, #53). Recovery required an immediate v0.4.0 promotion 38 minutes later. **Two minutes upfront beats a follow-up release.**

### Pre-release (from `dev`)

When `dev` is ready to be exercised by early adopters before promotion:

1. **Step 0 first** — see above.
2. Stay on `dev` (do not merge to `main` yet).
3. Write per-RC notes in a new file `changelogs/pre-releases/<version>-rc.N.md` — heading `# [<version>-rc.N] - YYYY-MM-DD`, body describing only the **delta** against the previous RC (or against the previous stable, for `rc.1`). One file per RC, not a running log. (See [`changelogs/pre-releases/0.3.0-rc.2.md`](changelogs/pre-releases/0.3.0-rc.2.md) for the expected layout.)
4. Add the entry to `CHANGELOG.md`'s `## Past releases > ### Pre-releases` index.
5. Tag: `git tag -a v0.x.y-rc.N -m "v0.x.y-rc.N" && git push --tags`.
6. GitHub Actions (`pre-release.yml`) builds binaries and publishes a **prerelease** (5 targets — Linux x86_64 + aarch64, macOS Intel + Apple Silicon, Windows x86_64). The release body is populated from the per-RC file via `--notes-file changelogs/pre-releases/<version>-rc.N.md` (run `gh release edit <tag> --notes-file <path>` after the workflow if you need to refresh it).
7. Iterate: subsequent candidates are `v0.x.y-rc.2`, `v0.x.y-rc.3`, …

### Stable release (from `main`)

Once the rc is validated and promoted to `main`:

1. **Step 0 first** — see above.
2. Update `Cargo.toml` `version`.
3. Move the `## [Unreleased]` section out of `CHANGELOG.md` into a new file `changelogs/<version>.md` (e.g. `changelogs/0.3.0.md`), rename its heading to `# [<version>] - YYYY-MM-DD`, and add a one-line entry at the bottom of `CHANGELOG.md`'s `## Past releases` index pointing to the new file. `CHANGELOG.md` at the root then only carries the next `## [Unreleased]` section. (See [`changelogs/0.2.0.md`](changelogs/0.2.0.md) for the expected layout.)
4. Merge `dev` → `main` (regular merge, never squash; see [Merge strategy](#merge-strategy)).
5. Tag: `git tag -a v0.x.y -m "v0.x.y" && git push --tags`.
6. GitHub Actions (`release.yml`) builds binaries and publishes the stable release. The release body is populated from `changelogs/<version>.md` via `--notes-file` (run `gh release edit v0.x.y --notes-file changelogs/<version>.md` after the workflow if needed).

Triggering matrix:

| Tag pattern              | Workflow         | `prerelease` flag |
|:-------------------------|:-----------------|:------------------|
| `v0.x.y`                 | `release.yml`    | `false`           |
| `v0.x.y-rc.N`            | `pre-release.yml`| `true`            |
| `v0.x.y-alpha.N`         | `pre-release.yml`| `true`            |
| `v0.x.y-beta.N`          | `pre-release.yml`| `true`            |

---

By contributing, you agree your changes are licensed under the MIT License (see `LICENSE.md`).
