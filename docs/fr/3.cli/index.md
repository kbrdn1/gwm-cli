---
title: CLI
description: La face scriptable de gwm, de la référence des sous-commandes aux complétions de shell et à l'intégration tmux / zellij / herdr.
navigation:
  title: CLI
---

# CLI

`gwm <subcommand>` est la face scriptable de gwm, conçue pour être sûre dans les pipelines, les complétions de shell et les hooks de pre-commit. Chaque sous-commande se termine avec un code significatif (`0` ok, `1` avertissement, `2` échec), de sorte que vous pouvez brancher `gwm doctor` dans la CI sans parser la sortie standard.

- **[Référence](/fr/cli/reference)** : chaque sous-commande, exhaustive (`init`, `config`, `create`, `new`, `pr`, `review`, `list`, `path`, `cd`, `switch`, `bootstrap`, `sync`, `remove`, `prune`, `exec`, `clean`, `undo`, `history`, `link`, `unlink`, `open`, `status`, `labels`, `milestones`, `doctor`, `daemon`, `statusline`, `trust`, `types`, `commit-prefix`, `hooks`, `theme`, `tui keys`, `aliases`, `tmux`, `zellij`, `herdr`, `completions`, `shell-init`).
- **[Complétions de shell](/fr/cli/completions)** : générer des scripts de complétion pour zsh / bash / fish / PowerShell / elvish, plus la complétion dynamique des noms de worktree via `gwm list --format=names`.
- **[Intégration multiplexeur](/fr/cli/multiplexer)** : `gwm tmux` / `gwm zellij` / `gwm herdr` pour ouvrir un worktree dans un nouvel onglet ou panneau de la session courante.

La CLI ne lance jamais de TUI de son côté : chaque sous-commande est non interactive et écrit sur stdout/stderr. La seule surface TUI est `gwm` seul (ou `gwm switch` en mode sélecteur), couverte dans la [section TUI](/fr/tui).
