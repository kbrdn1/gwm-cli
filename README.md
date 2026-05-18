# gwm — git worktree manager

CLI rust avec TUI ratatui pour gérer des worktrees git multi-repo. Remplace les scripts bash type `worktree-manager.sh` par un binaire portable, avec bootstrap configurable par projet via `.gwm.toml`.

## features

- gestion worktree native via `libgit2` (pas de dépendance externe à `gwq` ou `git` CLI)
- convention de branche par défaut `<type>/#<issue>-<description>` (surchargeable)
- bootstrap hybride : config TOML (copie de fichiers, gardes regex) + hooks shell (composer, npm, etc.)
- TUI ratatui (liste, créer, supprimer, bootstrap) et sous-commandes CLI complètes (scripting friendly)
- gardes de sécurité configurables (ex : refuser de copier un `.env` qui pointe vers AWS RDS)

## install

```bash
cd ~/Projects/Perso/gwm-cli
cargo install --path .
```

le binaire `gwm` est installé dans `~/.cargo/bin`.

## usage

### TUI (interactif)

```bash
cd <un-repo-git>
gwm           # ouvre la TUI sur le repo courant
```

raccourcis :

- `↑/↓` ou `j/k` : naviguer
- `n` : nouveau worktree (formulaire)
- `d` : supprimer le worktree sélectionné
- `b` : bootstrap (rejoue les étapes du `.gwm.toml`)
- `r` : refresh
- `?` : aide
- `q` ou `Esc` : quitter

### CLI

```bash
gwm init                                    # crée .gwm.toml dans le repo courant
gwm create feat 123 "user-authentication"   # crée la worktree feat/#123-user-authentication
gwm list                                    # liste les worktrees du repo
gwm remove feat-123                         # supprime par pattern (fuzzy)
gwm bootstrap <path>                        # rejoue le bootstrap sur une worktree existante
gwm path feat-123                           # imprime le chemin résolu (utile pour cd $(gwm path …))
gwm prune                                   # nettoie les références mortes
```

## config par repo (`.gwm.toml`)

placez `.gwm.toml` à la racine du repo. exemple complet dans `examples/gwm.toml.example`.

```toml
[worktree]
# base du chemin worktree. placeholders: {repo}, {home}
base = "{home}/cc-worktree/{repo}"
# nom du dossier worktree
path_pattern = "{type}-{issue}-{desc}"
# nom de la branche
branch_pattern = "{type}/#{issue}-{desc}"

# copies de fichiers depuis le repo principal vers la nouvelle worktree
[[bootstrap.copy]]
from = ".env.testing"
to = ".env.testing"
required = true
fallback = "inline"   # si manquant, écrire le contenu de [bootstrap.fallback.env_testing]

[[bootstrap.copy]]
from = ".env"
to = ".env"
required = false
guards = ["no-aws-rds"]

# gardes regex : si une copie matche, on bloque
[[bootstrap.guard]]
name = "no-aws-rds"
deny_patterns = ["amazonaws\\.com", "\\.rds\\."]
on_match = "seed-from-example"  # ou "abort"
example_file = ".env.example"

# fallback inline pour .env.testing manquant
[bootstrap.fallback.env_testing]
content = """
APP_ENV=testing
DB_CONNECTION=sqlite
DB_DATABASE=:memory:
"""

# commandes à exécuter dans la worktree après les copies
[[bootstrap.command]]
name = "composer install"
run = "composer install --no-interaction --prefer-dist"
when = "file_exists:composer.json"

[[bootstrap.command]]
name = "direnv allow"
run = "direnv allow"
when = "file_exists:.envrc"

# vérifications : symlinks à refuser (vendor/, node_modules/ qui pointeraient ailleurs)
[[bootstrap.no_symlink]]
path = "vendor"

[[bootstrap.no_symlink]]
path = "node_modules"
```

## defaults sans `.gwm.toml`

si aucun `.gwm.toml` n'est trouvé, gwm utilise :

- branch pattern : `{type}/#{issue}-{desc}`
- path pattern : `{type}-{issue}-{desc}`
- base : `~/cc-worktree/{repo}`
- aucun bootstrap (just `git worktree add`)

## types de branche reconnus

`feat`, `fix`, `hotfix`, `docs`, `test`, `refactor`, `chore`, `perf`, `ci`, `build`

## différences avec le script bash d'origine

| feature                             | bash + gwq            | gwm                            |
| ----------------------------------- | --------------------- | ------------------------------ |
| moteur worktree                     | `gwq` (CLI externe)   | `libgit2` natif                |
| bootstrap                           | hardcodé dans le shell | déclaratif via `.gwm.toml`     |
| portabilité multi-repo              | manuelle par projet   | un seul binaire                |
| TUI                                 | menu bash linéaire    | ratatui plein écran            |
| garde anti-RDS                      | hardcodée             | regex configurables            |

## license

MIT
