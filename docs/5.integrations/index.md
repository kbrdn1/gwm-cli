---
title: Integrations
description: Wire gwm with GitHub (gh), lazygit, AI reviewers, doctor in CI, and the packaged distributions (Homebrew, Nix).
navigation:
  title: Integrations
---

# Integrations

gwm is small on purpose — it shells out to the tools you already use rather than reimplementing them. These pages cover the supported integration points.

- **[GitHub issue / PR linking](/integrations/github-linking)** — auto-link branches matching `<type>/#<N>-<slug>` to their issue, fetch live state via `gh`, surface in the TUI sidebar.
- **[`gwm doctor`](/integrations/doctor)** — the 7 health checks, exit-code semantics (`0 / 1 / 2`), and the v0.6 update that now probes the configured launcher binaries.
- **[Homebrew & Nix](/integrations/homebrew-nix)** — the packaging surface: how releases flow into the Homebrew tap and the Nix flake.

The TUI-side integrations (the configurable launchers for `l` and `R`, the `[tui.open]` dispatch) live under [TUI → Configurable launchers](/tui/launchers) and [TUI → Open dispatch](/tui/open-dispatch).
