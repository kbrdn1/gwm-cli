---
title: Intégrations
description: Connectez gwm avec GitHub (gh) ou GitLab (glab), lazygit, les relecteurs IA, doctor en CI, et les distributions packagées (Homebrew, Nix).
navigation:
  title: Intégrations
---

# Intégrations

gwm est volontairement minimal : il délègue aux outils que vous utilisez déjà plutôt que de les réimplémenter. Ces pages couvrent les points d'intégration pris en charge.

- **[Liaison issue / PR GitHub](/fr/integrations/github-linking)** : lie automatiquement les branches au format `<type>/#<N>-<slug>` à leur issue, récupère l'état en temps réel via `gh`, et l'affiche dans la barre latérale du TUI.
- **[GitLab (multi-forge)](/fr/integrations/gitlab)** : pointer gwm vers GitLab plutôt que GitHub : la clé `forge`, le backend `glab`, et les différences d'état CI et de terminologie.
- **[`gwm doctor`](/fr/integrations/doctor)** : les 8 vérifications de santé, la sémantique des codes de sortie (`0 / 1 / 2`), la sonde du binaire de lancement ajoutée en v0.6, et la vérification du keymap `[tui.keys]` ajoutée en v0.8.
- **[Homebrew & Nix](/fr/integrations/homebrew-nix)** : la surface de packaging : le tap Homebrew, le flake Nix, `cargo binstall`, et les archives de release pré-compilées.
- **[herdr (plugin)](/fr/integrations/herdr)** : `herdr-plugin-gwm` pilote gwm depuis le multiplexeur herdr : create, switch, remove, review, exec, clean et la TUI dans un pane, herdr adoptant ce que gwm crée.
- **[Consommateurs du daemon](/fr/integrations/daemon-consumers)** : les premiers consommateurs de `gwm daemon` : le `gwm statusline` embarqué pour les prompts shell, le one-liner JSON-RPC brut, et une recette éditeur (Zed / VS Code).

Pour les runners CI qui créent des worktrees via `gwm create`, définissez `GWM_ALLOW_BOOTSTRAP=1` (ou passez `--allow-bootstrap` à l'invocation gwm) afin que la [barrière de confiance TOFU](/fr/configuration/trust-ledger) contourne l'invite interactive, ce qui est requis car la politique « refus par défaut » de la barrière avorte sur un stdin non-tty pour empêcher l'exécution silencieuse de lignes de bootstrap contrôlées par un attaquant.

Les intégrations côté TUI (les lanceurs configurables pour `l` et `R`, le dispatch `[tui.open]`) sont décrites dans [TUI → Lanceurs configurables](/fr/tui/launchers) et [TUI → Dispatch d'ouverture](/fr/tui/open-dispatch).
