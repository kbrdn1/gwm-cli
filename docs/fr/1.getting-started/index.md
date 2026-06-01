---
title: Démarrage
description: Installez gwm, créez votre premier worktree et configurez le helper cd en une ligne.
navigation:
  title: Démarrage
---

# Démarrage

Trois étapes pour une configuration gwm fonctionnelle :

1. **[Installation](/fr/getting-started/install)** — depuis les sources, via `cargo binstall`, Homebrew, un binaire précompilé ou un flake Nix.
2. **[Créer votre premier worktree](/fr/getting-started/first-worktree)** — `gwm create feat 42 user-auth` et ce qu'il fait.
3. **[Configurer `gcd`](/fr/getting-started/shell-init)** — un `cd` en une ligne vers n'importe quel worktree, depuis n'importe où.

Si vous avez déjà utilisé [`gwq`](https://github.com/d-kuro/gwq) ou le script bash maison `tools/worktree-manager.sh`, gwm en est la réécriture native en Rust — même workflow, configurable par dépôt, sans dépendance à une CLI externe. Les [différences conceptuelles avec le script bash](/fr/development#vs-script-bash) sont documentées dans la section développement.
