---
title: Développement
description: Compiler gwm depuis les sources, l'organisation de la suite de tests et les conventions de contribution (branches, commits, PRs).
navigation:
  title: Développement
---

# Développement

gwm est une petite crate Rust (binaire unique). Les workflows de build, de test et de publication sont documentés ici.

- **[Tests](/fr/development/testing)** — les fichiers de tests d'intégration (~990 tests sur la matrice ubuntu / macos / windows à partir de la v0.8.0), la boucle TDD obligatoire red → green → refactor, comment exécuter un sous-ensemble et la convention de test-sentinelle `// regression:`.
- **[Contribuer](/fr/development/contributing)** — le format Gitmoji + Conventional Commits, le nommage des branches, la checklist de PR et les règles autour de la séparation `CHANGELOG.md` / `changelogs/<version>.md`.

## référence rapide

```bash
cargo build              # debug build
cargo test               # ~990 tests across the integration files + unit tests
cargo fmt && cargo clippy -- -D warnings
cargo run                # opens TUI in the current repo
cargo install --path .   # install locally
```

Un dev shell Nix est épinglé dans [`flake.nix`](https://github.com/kbrdn1/gwm-cli/blob/main/flake.nix) — toolchain, `rust-analyzer`, `clippy`, `rustfmt`, `cargo-watch`, `cargo-edit` et les dépendances de build de `libgit2` — sans toucher au système hôte :

```bash
nix develop
```

## vs. script bash

Le contexte complet — ce qui a changé par rapport au `tools/worktree-manager.sh` original et pourquoi — se trouve sur la page de contribution, sous « [historique](/fr/development/contributing#history) ».

## changelog

Les versions publiées vivent sous [`changelogs/<version>.md`](https://github.com/kbrdn1/gwm-cli/tree/main/changelogs) ; le [`CHANGELOG.md`](https://github.com/kbrdn1/gwm-cli/blob/main/CHANGELOG.md) racine ne contient que la section `[Unreleased]` courante plus un index des versions passées.
