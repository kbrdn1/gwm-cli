---
title: Development
description: Building gwm from source, the test suite layout, and the contributing conventions (branches, commits, PRs).
navigation:
  title: Development
---

# Development

gwm is a small Rust crate (single binary). Build, test, and ship workflows are documented here.

- **[Testing](/development/testing)** — the 15 integration test files (433 tests as of v0.6.0), how to run a subset, and the `// regression:` sentinel-test convention.
- **[Contributing](/development/contributing)** — the Gitmoji + Conventional-Commit format, branch naming, the PR checklist, and the rules around the `CHANGELOG.md` / `changelogs/<version>.md` split.

## quick reference

```bash
cargo build              # debug build
cargo test               # 433 tests across 15 integration files + unit tests
cargo fmt && cargo clippy -- -D warnings
cargo run                # opens TUI in the current repo
cargo install --path .   # install locally
```

A Nix dev shell is pinned in [`flake.nix`](https://github.com/kbrdn1/gwm-cli/blob/main/flake.nix) — toolchain, `rust-analyzer`, `clippy`, `rustfmt`, `cargo-watch`, `cargo-edit`, and the `libgit2` build deps — without touching the host system:

```bash
nix develop
```

## vs. bash script

The full background — what changed from the original `tools/worktree-manager.sh` and why — lives in the contributing page under "[history](/development/contributing#history)".

## changelog

Released versions live under [`changelogs/<version>.md`](https://github.com/kbrdn1/gwm-cli/tree/main/changelogs); the root [`CHANGELOG.md`](https://github.com/kbrdn1/gwm-cli/blob/main/CHANGELOG.md) only holds the current `[Unreleased]` section plus an index of past releases.
