---
title: Integrations
description: Wire gwm with GitHub (gh) or GitLab (glab), lazygit, AI reviewers, doctor in CI, and the packaged distributions (Homebrew, Nix).
navigation:
  title: Integrations
---

# Integrations

gwm is small on purpose — it shells out to the tools you already use rather than reimplementing them. These pages cover the supported integration points.

- **[GitHub issue / PR linking](/integrations/github-linking)** — auto-link branches matching `<type>/#<N>-<slug>` to their issue, fetch live state via `gh`, surface in the TUI sidebar.
- **[GitLab (multi-forge)](/integrations/gitlab)** — point gwm at GitLab instead of GitHub: the `forge` key, the `glab` backend, and the CI-state and terminology differences.
- **[`gwm doctor`](/integrations/doctor)** — the 8 health checks, exit-code semantics (`0 / 1 / 2`), the launcher-binary probe added in v0.6, and the `[tui.keys]` keymap check added in v0.8.
- **[Homebrew & Nix](/integrations/homebrew-nix)** — the packaging surface: the Homebrew tap, the Nix flake, `cargo binstall`, and the prebuilt release archives.
- **[herdr (plugin)](/integrations/herdr)** — `herdr-plugin-gwm` drives gwm from inside the herdr multiplexer: create, switch, remove, review, exec, clean and the TUI in a pane, with herdr adopting what gwm creates.
- **[Daemon consumers](/integrations/daemon-consumers)** — the first consumers of `gwm daemon`: the bundled `gwm statusline` for shell prompts, the raw JSON-RPC one-liner, and an editor recipe (Zed / VS Code).

For CI runners that spin up worktrees via `gwm create`, set `GWM_ALLOW_BOOTSTRAP=1` (or pass `--allow-bootstrap` on the gwm invocation) so the [TOFU trust gate](/configuration/trust-ledger) bypasses the interactive prompt — required since the gate's default-deny policy aborts on non-tty stdin to prevent silent execution of attacker-controlled bootstrap lines.

The TUI-side integrations (the configurable launchers for `l` and `R`, the `[tui.open]` dispatch) live under [TUI → Configurable launchers](/tui/launchers) and [TUI → Open dispatch](/tui/open-dispatch).
