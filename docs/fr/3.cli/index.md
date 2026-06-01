---
title: CLI
description: Chaque sous-commande gwm, avec sa synopsis, ses flags et des exemples prêts pour le shell.
navigation:
  title: CLI
---

# CLI

`gwm <subcommand>` est la face scriptable de gwm — conçue pour être sûre dans les pipelines, les complétions de shell et les hooks de pre-commit. Chaque sous-commande se termine avec un code significatif (`0` ok, `1` avertissement, `2` échec), de sorte que vous pouvez brancher `gwm doctor` dans la CI sans parser la sortie standard.

- **[Référence](/fr/cli/reference)** — chaque sous-commande, exhaustive (`init`, `config`, `create`, `new`, `pr`, `list`, `path`, `cd`, `switch`, `bootstrap`, `sync`, `remove`, `prune`, `undo`, `history`, `link`, `unlink`, `open`, `status`, `labels`, `milestones`, `doctor`, `trust`, `types`, `commit-prefix`, `hooks`, `theme`, `tui keys`, `aliases`, `tmux`, `zellij`, `completions`, `shell-init`).
- **[Complétions de shell](/fr/cli/completions)** — générer des scripts de complétion pour zsh / bash / fish / PowerShell / elvish, plus la complétion dynamique des noms de worktree via `gwm list --format=names`.
- **[Intégration multiplexeur](/fr/cli/multiplexer)** — `gwm tmux` / `gwm zellij` pour ouvrir un worktree dans un nouvel onglet ou panneau de la session courante.

La CLI ne lance jamais de TUI de son côté — chaque sous-commande est non interactive et écrit sur stdout/stderr. La seule surface TUI est `gwm` seul (ou `gwm switch` en mode sélecteur), couverte dans la [section TUI](/fr/tui).
