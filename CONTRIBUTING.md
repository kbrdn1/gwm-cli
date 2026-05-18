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

### TDD expectations

Any new feature ships with tests. The bar is:

- **unit logic** (config parsing, naming, kebab, guard regex) → tests in the matching `tests/*_tests.rs` file.
- **disk side effects** (file copy, symlink removal, command exec) → use `tempfile::TempDir`.
- **git operations** → use `tests/common::init_repo()` which gives you a fresh repo on `main` with one commit.
- **public CLI surface** → end-to-end test in `tests/cli_binary.rs` via `assert_cmd`.

A PR that adds behaviour without tests will be sent back.

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

Versioning is SemVer (`MAJOR.MINOR.PATCH`):

- `MAJOR` → breaking change
- `MINOR` → new feature
- `PATCH` → bug fix

Cut a release by:

1. Update `Cargo.toml` `version`.
2. Move `## [Unreleased]` entries into a dated section in `CHANGELOG.md`.
3. Tag: `git tag -a v0.x.y -m "v0.x.y" && git push --tags`.
4. GitHub Actions (`release.yml`) builds binaries and publishes the release.

---

By contributing, you agree your changes are licensed under the MIT License (see `LICENSE.md`).
