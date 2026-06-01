---
title: TUI
description: L'interface ratatui — raccourcis clavier, disposition de la barre latérale, lanceurs configurables, filtre flou et compte à rebours de la surcouche de confirmation.
navigation:
  title: TUI
---

# TUI

Lancer `gwm` sans argument ouvre l'interface ratatui sur le dépôt courant. De là, vous pouvez créer, supprimer, bootstrapper et naviguer entre les worktrees sans quitter le terminal.

- **[Raccourcis clavier](/fr/tui/keybindings)** — la table complète des touches, y compris les ajouts de v0.6 (`R`, `F`, `O`, `L`, `f`, `y`). Le keymap est désormais entièrement configurable via `[tui.keys]`.
- **[Barre latérale de détails](/fr/tui/sidebar)** — les quatre sous-sections encadrées du panneau de droite, l'orientation responsive, le graphe de commits à la lazygit, le bloc Issue / PR en direct, et le mode stashes `s`.
- **[Filtre flou](/fr/tui/filter)** — `/` ouvre la barre de filtre en ligne ; nucleo-matcher sous le capot.
- **[Compte à rebours de la surcouche de confirmation](/fr/tui/confirm-countdown)** — le compte à rebours de sécurité qui empêche les suppressions accidentelles de branches lorsque `p` est armé.
- **[Lanceurs configurables](/fr/tui/launchers)** — `[git_tui]` (`l`) et `[review]` (`R`), avec les placeholders `{base} {head} {path} {diff}`.
- **[Dispatch d'ouverture](/fr/tui/open-dispatch)** — `[tui.open]` contrôle ce que fait `o` (`shell` / `editor` / `finder`).
- **[Thèmes](/fr/tui/themes)** — couleurs `[theme]` basées sur des rôles et presets intégrés (`catppuccin`, `gruvbox`, `tokyo-night`, `claude-dark`).
- **[Keymap & palette de commandes](/fr/tui/keymap-and-palette)** — remappez n'importe quel binding via `[tui.keys]` (avec support des chords), ou déclenchez une action par son nom depuis la palette `:`.

`n` (nouveau worktree) et `b` (re-bootstrap) sont protégés par le [registre de confiance TOFU](/fr/configuration/trust-ledger) — un `.gwm.toml` non approuvé fait apparaître un message de refus dans la barre de statut plutôt que de lancer le bootstrap. La variante picker (`gwm switch`, alias `gwm s`) réutilise la même TUI mais désactive la création / suppression / bootstrap, puis affiche le chemin du worktree choisi sur stdout — pensé pour être `eval`-ué par le wrapper shell `gcd`.

## habillage

La passe de polish v0.8.0 a resserré le cadre de la TUI. Toutes les couleurs suivent le [`[theme]`](/fr/tui/themes) résolu :

- **Statusline** — une seule ligne. Les indications de touches sont rendues comme des puces badge en vidéo inversée (la touche peinte avec l'accent du thème, puis un libellé court) ; le message de statut (journal d'action) est épinglé à droite avec une priorité absolue. Sous contrainte de largeur, la liste des indications est tronquée avec un marqueur `…` tandis que le journal reste visible.
- **Header** — une seule ligne sans bordure : la version est une puce en vidéo inversée, le nom du dépôt est en gras, et le répertoire de travail est atténué et compressé avec un tilde. Le drapeau `picker` est sa propre puce en vidéo inversée. L'ordre d'abandon sous contrainte de largeur est chemin → nom du dépôt → puce de version (la version survit en dernier).
- **Modals** — chaque surcouche partage un même cadre : une bordure arrondie avec un titre en gras thématisé, les couleurs du thème, et une boîte dimensionnée à son contenu plutôt qu'à un pourcentage fixe de l'écran.
