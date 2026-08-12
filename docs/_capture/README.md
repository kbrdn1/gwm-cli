# `docs/_capture/`: reproducible doc captures

Every screenshot and recording under `docs/**/_assets/` is generated from a
committed [vhs](https://github.com/charmbracelet/vhs) `.tape` script here, driven
against a single deterministic demo repo. Nothing is hand-captured, so the whole
set can be regenerated after a UI change with one command.

## regenerate everything

```bash
# from the repo root — rebuilds the demo fixture, then runs every tape
./docs/_capture/generate.sh

# reuse the existing demo fixture (faster; skips the rebuild)
GWM_KEEP_DEMO=1 ./docs/_capture/generate.sh
```

Requirements: `vhs`, an installed `gwm` on `PATH`, and a Nerd Font
(`CaskaydiaCove Nerd Font Mono`, as set in each tape's `Set FontFamily`).

## how it fits together

- **`setup-demo.sh`** builds `~/gwm-demo/acme-api`: a branchy trunk history (so
  the sidebar commit graph has topology) plus four worktrees in deliberately
  varied git states (clean, clean+ahead, staged, dirty+ahead). It also builds
  `~/gwm-demo/payments-svc`, an intentionally *untrusted* fixture used only for
  the TOFU trust-prompt capture. Dates are pinned so the graph is byte-stable.
  It finally seeds **agent sessions** under `~/gwm-demo/agents-home`: Claude
  Code and Codex artefacts on three of the worktrees, two of them pinned. They
  live inside the demo root and are read through gwm's own `GWM_AGENTS_HOME`
  seam, so the operator's real `~/.claude` and `~/.codex` are never written to
  and never captured. The tapes that show an agent surface export the variable;
  every other tape keeps the empty-store look.
- **`<name>.tape`**: one capture each. Still PNGs use vhs's `Screenshot`
  command (the tape's `Output …/.tmp/<name>.gif` is a throwaway vhs requires);
  animated captures `Output` their `.gif` directly.
- **`theme.tape`** is generic: `generate.sh` injects each `[theme] preset` into
  the demo config (hidden from git via `--assume-unchanged`) and moves the
  result to `theme-<preset>.png`.
- **`refresh-agents.sh`** re-stamps the seeded agent artefacts. Session
  freshness is a pure function of artefact mtime (`active` under 300 s, `idle`
  past it), so a fixture built ten minutes ago renders every agent row grey and
  the agent captures would document a state the code never shows. The script
  owns the ages in one place: everything reads `active`, except the two
  sessions the captures deliberately show as `idle`. `setup-demo.sh` calls it
  once, and `agents.tape` / `cli-agents.tape` call it again right before
  capturing.
- **`generate.sh`** orchestrates the above and drops each asset into the correct
  `docs/<section>/_assets/` directory.

## the grey background

`gwm`'s default theme paints no background, so the TUI inherits the terminal's.
Every tape sets a neutral grey vhs theme (`#2b2b2b`) so the default-theme
captures read as grey, not the blue-tinted default of a bundled theme. The
theme-gallery shots are the exception: each preset paints its own background on
purpose.

## captured off the demo

- `github-linking.tape` runs against the **real gwm-cli repo**, not the
  acme-api demo: the Issue·PR pane needs a live, `gh`-detectable PR, and the
  demo has no remote. It is not part of `generate.sh`'s demo-driven loop; run it
  directly (`vhs docs/_capture/github-linking.tape`) from a checkout whose
  current branch has an open PR, adjusting the `/206` filter to that PR.

## not covered here

- `docs/3.cli/3.multiplexer.md` (`gwm tmux --split`): vhs cannot host a tmux
  client (`open terminal failed: not a terminal`); this one needs a real
  terminal recording.
