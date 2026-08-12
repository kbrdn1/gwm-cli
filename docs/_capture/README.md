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

⚠️ **`demo.tape` degrades silently without the agent store.** vhs does not
check exit codes, so if `setup-demo.sh` has not run, the tape still records a
perfectly valid GIF — just one where detection finds nothing, the `AGENT`
column is absent from frame 0 and the `a` overlay is empty. Nothing fails, the
asset is simply worse. After recording, extract frame 0 and confirm the
`AGENT` column is there before committing:

```bash
ffmpeg -i docs/_capture/demo.gif -vf "select='eq(n\,0)'" -vsync 0 /tmp/f0.png
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
  the TOFU trust-prompt capture. Dates are pinned so the graph is byte-stable,
  and branch ages are backdated through `branch.<b>.gwm-created-at` (which
  `gwm create` stamps with the wall clock) so the `AGE` column reads like a
  week of work rather than showing every worktree seconds old.
- **the agents fixture** (inside `setup-demo.sh`): a fabricated agent store at
  `$GWM_DEMO_ROOT/agents-home`, wired in through `GWM_AGENTS_HOME` — gwm's own
  artefact-root seam — so captures can show the `AGENT` column and the Agents
  pane without ever writing to the real `~/.claude` or `~/.codex`. Four Claude
  Code and Codex sessions across three worktrees, two of them pinned: `feat/#42`
  carries a pinned active session next to an unpinned idle one, which is what
  makes the `a` overlay worth a screenshot. Detection and pins are seeded
  separately on purpose: the table's `AGENT` column reads the detection
  snapshot, the sidebar Agents pane reads the pins only. Note `gwm agents
  attach` refuses a session id detection has not seen, so `GWM_AGENTS_HOME`
  must be exported when the pins are written. Session freshness decays (see
  `refresh-agents.sh` below), so a tape showing an agent surface re-stamps the
  artefacts in its hidden block rather than trusting the fixture's age.
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
- **the sidebar Agents pane at the standard `Set Height 680`.** After Status
  and Issue/PR the sidebar has roughly eight rows left, and
  `split_section_heights` serves an overflow in the order commits → working
  tree → agents, so the pane floors to zero. This is the documented triage, not
  a missing fixture — the `a` overlay shows the same session lines and is what
  `demo.tape` captures. A tape that specifically wants the sidebar pane needs
  about three more rows (`Set Height 760`).
