# GitHub Labels

Reference matrix for labels used on issues and PRs in this repo. Apply with [`github-label-sync`](https://github.com/Financial-Times/github-label-sync) or manually under Settings → Labels.

## Type labels

| Name           | Color     | Description |
|:---------------|:----------|:------------|
| `feature`      | `#0E8A16` | New feature implementation |
| `fix`          | `#D73A4A` | Bug fix |
| `hotfix`       | `#FF3333` | Critical production bug fix |
| `docs`         | `#1D76DB` | Documentation only |
| `test`         | `#87CEEB` | Test additions / fixes |
| `refactor`     | `#FBCA04` | Code restructuring, no behaviour change |
| `chore`        | `#808080` | Repo maintenance |
| `perf`         | `#FFA500` | Performance improvement |
| `ci`           | `#26A69A` | CI / GitHub Actions |
| `build`        | `#BFD4F2` | Build system, Cargo manifest |

## Domain labels (gwm-specific)

| Name           | Color     | Description |
|:---------------|:----------|:------------|
| `cli`          | `#5319E7` | clap subcommands surface |
| `tui`          | `#7057FF` | ratatui views, events, rendering |
| `config`       | `#0075CA` | `.gwm.toml` parsing / schema |
| `worktree`     | `#006B75` | libgit2 worktree ops |
| `bootstrap`    | `#C5DEF5` | file copies, guards, shell hooks |
| `security`     | `#B60205` | Security issues / sensitive data leaks |
| `dependencies` | `#8B008B` | Dependency updates (dependabot lives here) |

## Status labels

| Name        | Color     | Description |
|:------------|:----------|:------------|
| `duplicate` | `#CCCCCC` | Duplicate of another issue or PR |
| `invalid`   | `#444444` | Cannot reproduce or out of scope |
| `wontfix`   | `#FFFFFF` | Valid but won't be addressed |
| `breaking`  | `#FF0000` | Non-backward-compatible change |
| `good-first-issue` | `#7057FF` | Suitable for newcomers |

## Priorities

Set via GitHub Projects priority field, not labels.

| Value      | Meaning |
|:-----------|:--------|
| `Critical` | Blocking issue, must fix immediately |
| `High`     | Important, fix within current cycle |
| `Medium`   | Standard priority |
| `Low`      | Defer to a future cycle |
| `Trivial`  | Cosmetic |

## Issue types

Used in `.github/ISSUE_TEMPLATE/*` to pre-fill the type:

- `🐛 Bug Report` → `fix`
- `✨ Feature Request` → `feature`
- `📋 Task` → `chore`
