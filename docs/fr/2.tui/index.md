---
title: TUI
description: L'interface ratatui, ses raccourcis clavier, la disposition de la barre latérale, les lanceurs configurables, le filtre flou et le compte à rebours de la surcouche de confirmation.
navigation:
  title: TUI
---

# TUI

![TUI gwm : table des worktrees et barre latérale de détails](../../2.tui/_assets/hero.png)

Lancer `gwm` sans argument ouvre l'interface ratatui sur le dépôt courant. De là, vous pouvez créer, supprimer, bootstrapper et naviguer entre les worktrees sans quitter le terminal.

- **[Raccourcis clavier](/fr/tui/keybindings)** : la table complète des touches, y compris la [refonte du keymap v0.10 (#290)](/fr/tui/keybindings#v010-rebind-summary) qui a réorganisé les valeurs par défaut (par ex. `O` est désormais le terminal plein écran, et non plus le menu d'ouverture ; `y` copie désormais le nom de la branche, et non plus le chemin). Le keymap est entièrement configurable via `[tui.keys]`.
- **[Barre latérale de détails](/fr/tui/sidebar)** : les quatre sous-sections encadrées du panneau de droite, l'orientation responsive, le graphe de commits à la lazygit, le bloc Issue / PR en direct (avec l'indicateur d'état CI), l'arbre de fichiers du `git status`, et le mode stashes `S`.
- **[Filtre flou](/fr/tui/filter)** : `/` ouvre la barre de filtre en ligne ; nucleo-matcher sous le capot.
- **[Compte à rebours de la surcouche de confirmation](/fr/tui/confirm-countdown)** : le compte à rebours de sécurité qui empêche les suppressions accidentelles de branches lorsque `D` est armé.
- **[Lanceurs configurables](/fr/tui/launchers)** : `[git_tui]` (`l` overlay / `L` plein écran) et `[review]` (`r` overlay / `R` plein écran), avec les placeholders `{base} {head} {path} {diff}` et l'overlay PTY embarqué.
- **[Dispatch d'ouverture](/fr/tui/open-dispatch)** : `o` ouvre un terminal dans un overlay PTY embarqué ; `O` exécute le dispatch `[tui.open]` (`shell` / `editor` / `finder`).
- **[Surcouches exec / clean](/fr/tui/keybindings#surcouche-de-sélection-exec-x)** : `x` choisit un profil `[exec.profiles]` et le lance dans une surcouche PTY ; `X` prévisualise et récupère l'espace des artefacts de build (même garde-fou git-ignore que `gwm clean --yes`, derrière un compte à rebours de confirmation).
- **[Thèmes](/fr/tui/themes)** : couleurs `[theme]` basées sur des rôles et presets intégrés (`catppuccin`, `gruvbox`, `tokyo-night`, `claude-dark`).
- **[Keymap & palette de commandes](/fr/tui/keymap-and-palette)** : remappez n'importe quel binding via `[tui.keys]` (avec support des chords), ou déclenchez une action par son nom depuis la palette `:`.
- **[Sessions d'agents](/fr/tui/agent-sessions)** : quel agent IA (Claude Code, Codex, opencode, Mistral Vibe) travaille dans quel worktree, lu depuis les artefacts sur disque de chaque outil et affiché dans la colonne `AGENT`, la barre latérale et la surcouche `a`.

`n` (nouveau worktree) et `b` (re-bootstrap) sont protégés par le [registre de confiance TOFU](/fr/configuration/trust-ledger) : un `.gwm.toml` non approuvé fait apparaître un message de refus dans la barre de statut plutôt que de lancer le bootstrap. La variante picker (`gwm switch`, alias `gwm s`) réutilise la même TUI mais désactive la création / suppression / bootstrap, puis affiche le chemin du worktree choisi sur stdout, pensé pour être `eval`-ué par le wrapper shell `gcd`.

## habillage

La passe de polish v0.8.0 a resserré le cadre de la TUI. Toutes les couleurs suivent le [`[theme]`](/fr/tui/themes) résolu :

- **Statusline** : une seule ligne. Les indications de touches sont rendues comme des puces badge en vidéo inversée (la touche peinte avec l'accent du thème, puis un libellé court) ; le message de statut (journal d'action) est épinglé à droite avec une priorité absolue. Sous contrainte de largeur, la liste des indications est tronquée avec un marqueur `…` tandis que le journal reste visible.
- **Header**, une seule ligne sans bordure : la version est une puce en vidéo inversée, le nom du dépôt est en gras, et le répertoire de travail est atténué et compressé avec un tilde. Le drapeau `picker` est sa propre puce en vidéo inversée. L'ordre d'abandon sous contrainte de largeur est chemin → nom du dépôt → puce de version (la version survit en dernier).
- **Modals** : chaque surcouche partage un même cadre, avec une bordure arrondie, un titre en gras thématisé, les couleurs du thème, et une boîte dimensionnée à son contenu plutôt qu'à un pourcentage fixe de l'écran.

### layout

`[tui] layout` ([#545](https://github.com/kbrdn1/gwm-cli/issues/545)) choisit comment les panneaux et les sections de la sidebar sont encadrés. **`"compact"` est le défaut** : aucun filet, un en-tête d'une ligne en aplat par section. Le titre garde son raccourci entre crochets et passe en majuscules, le compteur se place à droite de cette même ligne, un filet `muted` marque la frontière entre les deux panneaux, et le panneau des worktrees se dimensionne à son nombre de lignes au lieu de réserver sa part du split. Le focus se lit sur l'en-tête : le panneau actif prend l'aplat `selection_bg`, et le corps de l'autre est atténué.

La capture en haut de cette page le montre, comme toutes les autres captures de cette documentation.

`layout = "bordered"` restaure la disposition de gwm jusqu'à la 1.7, les boîtes façon lazygit :

![TUI gwm en mode bordé : filets façon lazygit autour de chaque section](../../2.tui/_assets/bordered.png) Les modales gardent leur cadre dans les deux cas. La configuration et le rôle de thème `section_bg` sont documentés sous [`.gwm.toml`](/fr/configuration/gwm-toml#layout).
