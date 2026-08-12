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

`demo.tape` is **not** in `generate.sh`'s loop — it is the one long-form
recording and is run on its own, from the repo root, after the fixture exists:

```bash
bash docs/_capture/setup-demo.sh && vhs docs/_capture/demo.tape
```

### maintainer-only for now

Every tape hardcodes `/Users/kbrdn1/gwm-demo/...` in its `cd`, so the set only
regenerates on the maintainer's machine as it stands. `setup-demo.sh` and
`generate.sh` both honour `GWM_DEMO_ROOT`; the tapes do not. Making them
portable means threading the variable through 17 `Type "cd …"` lines and is
tracked separately — until then, treat regeneration as a maintainer task.

## how it fits together

- **`setup-demo.sh`** builds `~/gwm-demo/acme-api`: a branchy trunk history (so
  the sidebar commit graph has topology) plus four worktrees in deliberately
  varied git states (clean, clean+ahead, staged, dirty+ahead). It also builds
  `~/gwm-demo/payments-svc`, an intentionally *untrusted* fixture used only for
  the TOFU trust-prompt capture. Dates are pinned so the graph is byte-stable.
- **`<name>.tape`**: one capture each. Still PNGs use vhs's `Screenshot`
  command (the tape's `Output …/.tmp/<name>.gif` is a throwaway vhs requires);
  animated captures `Output` their `.gif` directly.
- **`theme.tape`** is generic: `generate.sh` injects each `[theme] preset` into
  the demo config (hidden from git via `--assume-unchanged`) and moves the
  result to `theme-<preset>.png`.
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
