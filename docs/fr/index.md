---
title: gwm — gestionnaire de worktrees git
description: CLI Rust + TUI ratatui pour gérer les worktrees git entre projets. libgit2 natif, bootstrap configurable par dépôt, binaire unique.
---

# gwm

_Cette page est la traduction française de la documentation. La version anglaise sous `/docs` fait foi en cas de divergence._

CLI Rust + TUI ratatui pour gérer les worktrees git entre projets.

- Les opérations sur les worktrees s'appuient sur `libgit2` vendored — aucune dépendance à `gwq` ; seules quelques fonctionnalités (`gwm sync`, le lanceur de diff de review, le `git status` / `git log` de la barre latérale) délèguent à votre propre `git`.
- `gwm <subcommand>` pour les scripts et les hooks ; `gwm` seul ouvre une interface ratatui.
- `.gwm.toml` par dépôt : conventions de branche / chemin, copies de fichiers, garde-fous regex, commandes de cycle de vie `[hooks.*]`, invariants no-symlink — plus une configuration globale au niveau utilisateur dans `~/.config/gwm/config.toml` fusionnée en dessous, de sorte qu'une préférence définie une fois s'applique à chaque dépôt.
- Convention de branche `<type>/#<issue>-<description>` par défaut ; redéfinissable par dépôt. Les `[aliases]` reflètent les alias `git config` ; `gwm commit-prefix` plus un hook `commit-msg` optionnel pilotent la convention Gitmoji + Conventional Commits.
- Lanceurs configurables pour les keybindings `l` (TUI git) et `R` (review).
- Personnalisation de la TUI : presets `[theme]` par rôle (`catppuccin`, `gruvbox`, `tokyo-night`, `claude-dark`), un keymap `[tui.keys]` remappable avec des chords multi-touches, une palette de commandes `:`, une sidebar responsive et une statusline sur une seule ligne.
- Filets de sécurité : `--dry-run` sur `gwm remove` / `gwm prune`, plus `gwm undo` / `gwm history` adossés à un journal d'opérations.
- Liaison GitHub issue / PR de première classe — les branches respectant la convention de nommage se lient automatiquement à leur issue ; les PR sont détectées automatiquement depuis `gh` lorsqu'elles ne sont pas liées explicitement. `[[labels]]` / `[[milestones]]` déclaratifs, `gwm new` (issue → worktree) et `gwm pr` (corps de PR templaté).
- `gwm sync` récupère l'upstream d'un worktree et fait un rebase (ou un merge) dessus, sans risque en cas de conflit.
- [Trust ledger TOFU](/fr/configuration/trust-ledger) sur `.gwm.toml` — le premier `gwm create` / `gwm bootstrap` sur un dépôt demande confirmation avant d'exécuter la moindre ligne de commande de bootstrap. `--allow-bootstrap` / `GWM_ALLOW_BOOTSTRAP=1` pour contourner en CI.
- Installation via `cargo install gwm`, `cargo binstall gwm` (archives précompilées, sans toolchain), Homebrew ou Nix.

## carte de la documentation

| Section                                            | À lire quand …                                                                |
|:---------------------------------------------------|:------------------------------------------------------------------------------|
| [Premiers pas](/fr/getting-started)                | vous voulez installer gwm et créer votre premier worktree                     |
| [TUI](/fr/tui)                                     | vous vivez dans l'interface ratatui — keymap, sidebar, lanceurs, filtre       |
| [CLI](/fr/cli)                                     | vous scriptez gwm depuis des shells, des jobs CI ou des alias `gh`            |
| [Configuration](/fr/configuration)                 | vous écrivez ou étendez `.gwm.toml` — bootstrap, garde-fous, prédicats        |
| [Intégrations](/fr/integrations)                   | vous branchez gwm avec `gh`, `lazygit`, Homebrew, Nix ou `gwm doctor` en CI   |
| [Développement](/fr/development)                   | vous contribuez — organisation des tests, conventions, dev shell              |
| [Roadmap](/fr/roadmap)                             | vous voulez savoir ce qui arrive ensuite                                      |

## le tour en 30 secondes

```bash
# install
cargo install gwm
# or: cargo binstall gwm        # prebuilt archive, no Rust toolchain
# or: brew tap kbrdn1/tap && brew install gwm

# bootstrap a per-repo config (optional but recommended)
cd /path/to/your/repo
gwm init

# create a worktree on a feature branch
gwm create feat 42 user-authentication
# → ~/cc-worktree/<repo>/feat-42-user-authentication
# → branch feat/#42-user-authentication

# open the TUI on the current repo (themes, command palette, remappable keys)
gwm

# fuzzy-jump back into an existing worktree (with `gwm shell-init` wired up)
gcd auth

# misfired a remove? bring it back
gwm undo
```

## pourquoi gwm

La version bash (`tools/worktree-manager.sh` dans certains de nos dépôts) était liée à la stack d'un seul projet et à l'historique d'incidents d'une seule équipe. `gwm` conserve les leçons — garde-fous anti-RDS, copies de `.env.testing`, hooks post-create — et les rend configurables par dépôt. Un seul binaire, le même comportement partout.

Le contexte complet vit dans [le changelog](/fr/development#changelog) et dans l'historique du tracker d'issues sur [github.com/kbrdn1/gwm-cli](https://github.com/kbrdn1/gwm-cli).
