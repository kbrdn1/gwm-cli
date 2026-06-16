---
title: Configuration
description: Le schéma .gwm.toml — conventions de worktree, pipeline de bootstrap, hooks de cycle de vie, guards regex, prédicats when, thème, keymap, templates, alias, et la configuration globale au niveau utilisateur.
navigation:
  title: Configuration
---

# Configuration

gwm lit `.gwm.toml` depuis la racine du dépôt. Sans configuration, il revient à des valeurs par défaut raisonnables (`~/cc-worktree/<repo>/<type>-<issue>-<desc>`, pas de bootstrap). Avec une configuration, il peut copier des fichiers, exécuter des hooks de cycle de vie, refuser d'hériter de secrets dangereux, configurer les launchers / keymap / thème de la TUI, et déclarer des labels GitHub, des milestones et des templates d'issue / PR. Le même schéma peut aussi vivre au niveau d'une [configuration globale utilisateur](/fr/configuration/global-config) fusionnée sous celle de chaque dépôt.

- **[schéma `.gwm.toml`](/fr/configuration/gwm-toml)** — chaque section : `[worktree]`, `[[bootstrap.copy]]`, `[[bootstrap.guard]]`, `[bootstrap.fallback.*]`, `[[bootstrap.no_symlink]]`, `[[hooks.*]]` (+ legacy `[[bootstrap.command]]`), `[theme]`, `[tui]`, `[tui.keys]`, `[tui.open]`, `[git_tui]`, `[review]`, `[gitmoji]`, `[[labels]]`, `[[milestones]]`, `[issue_template]`, `[pr_template]`, `[aliases]`, `[doctor]`.
- **[Configuration globale utilisateur](/fr/configuration/global-config)** — `~/.config/gwm/config.toml` fusionnée sous le `.gwm.toml` de chaque dépôt ; sémantique de deep-overlay et l'opt-out `GWM_NO_GLOBAL_CONFIG=1`.
- **[Pipeline de bootstrap](/fr/configuration/bootstrap)** — ordre d'exécution : hooks de cycle de vie autour des copies → guards → fallbacks → vérification no-symlink.
- **[Guards regex](/fr/configuration/guards)** — patterns de deny-list sur les fichiers copiés (l'incident d'origine « pas d'AWS RDS dans `.env` »).
- **[Prédicats `when`](/fr/configuration/when-predicates)** — `file_exists:`, `cmd_exists:`, `env_set:`, `env_eq:`, `glob_exists:`, avec composition `!`, `&&`, `||`.
- **[Trust ledger TOFU](/fr/configuration/trust-ledger)** — la barrière qui se déclenche avant le pipeline de bootstrap (issue #95). Modèle de menace, surface CLI (`gwm trust list / revoke / show`), comportement de la TUI, format du ledger, bypass CI.
- **[Presets de configuration](/fr/configuration/presets)** — `gwm init --preset <stack>` génère un `.gwm.toml` clé en main pour une stack connue (`laravel`, `node`/`nuxt`, `rust`, `go`, `python-uv`, `generic`) au lieu du template générique ; `--list-presets` et `--show`.

Exécutez `gwm init` dans un dépôt vierge pour écrire un `.gwm.toml` par défaut. Pour l'exemple annoté complet avec chaque champ commenté, voir [`examples/gwm.toml.example`](https://github.com/kbrdn1/gwm-cli/blob/main/examples/gwm.toml.example) dans le dépôt.
