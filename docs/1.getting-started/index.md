---
title: Getting Started
description: Install gwm, create your first worktree, and wire up the one-line cd helper.
navigation:
  title: Getting Started
---

# Getting Started

Three steps to a working gwm setup:

1. **[Install](/getting-started/install)** — from source, Homebrew, prebuilt binary, or a Nix flake.
2. **[Create your first worktree](/getting-started/first-worktree)** — `gwm create feat 42 user-auth` and what it does.
3. **[Wire up `gcd`](/getting-started/shell-init)** — one-line `cd` into any worktree from anywhere.

If you've used [`gwq`](https://github.com/d-kuro/gwq) or the in-house `tools/worktree-manager.sh` bash script before, gwm is the Rust-native rewrite — same workflow, configurable per repo, no external CLI dependencies. The conceptual [differences vs. the bash script](/development#vs-bash-script) live in the development section.
