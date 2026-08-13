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

## the dark background

`gwm`'s default theme paints no background, so the TUI inherits the terminal's.
Every tape sets a neutral dark vhs theme (`#141414`) so the default-theme
captures read as neutral, not the blue-tinted default of a bundled theme. The
theme-gallery shots are the exception: each preset paints its own background on
purpose.

It was `#2b2b2b` until 1.8.0, a mid-grey that cost twice over. Compact paints
its section headers with the `section_bg` role, a band only a few steps off the
terminal background, and against a mid-grey the band barely read as one. On the
docs site the frames also sat *in* the page rather than on it, which is the
contrast half of the ratatui maintainer's feedback
([#544](https://github.com/kbrdn1/gwm-cli/issues/544)). Going darker fixes both
with one value, and the palette above it is unchanged.

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
- **the sidebar Agents pane at the height the full-TUI tapes use.** After Status
  and Issue/PR the sidebar has a handful of rows left, and
  `split_section_heights` serves an overflow in the order commits → working
  tree → agents, so the pane floors to zero. This is the documented triage, not
  a missing fixture: the `a` overlay shows the same session lines, and that is
  what `agents.tape` and `demo.tape` capture. A tape that specifically wants the
  sidebar pane has to buy it about three more rows than its neighbours.

## sizes

Set per tape, not shared, and every one of them was arrived at by looking at
the result. Two rules produced the current matrix
([#544](https://github.com/kbrdn1/gwm-cli/issues/544)):

**Width targets 800-1000px**, which is where a screenshot's text renders at
roughly the size of the prose around it in a README. Columns do not follow from
pixels by intuition, so measure rather than estimate: at `FontSize 15` and
`Padding 22`, **800px is 81 columns, 1000px is 103, 1160px is 121, 1500px is
159**. The way to check is a throwaway tape that runs
`echo $(tput cols) x $(tput lines)` and screenshots itself.

Four captures sit above the band because the surface does not fit inside it,
and each is a measurement rather than a preference:

| capture | width | floor it is clearing |
|:--|--:|:--|
| `side-by-side` | 1400 | `SIDEBAR_MIN_WIDTH` is 120, and `STATUS` only stops clipping around 148 |
| `keybindings`, `keymap`, `config-panel`, `palette` | 1240 | those modals clamp to a 64-column floor, which cuts their descriptions |
| `cli-list` | 1340 | `gwm list` prints 140 columns; below that the shell wraps |
| `cli-agents` | 1220 | `gwm agents` prints 126 |

**Height is cut to the content**, one blank row above the status bar. Do not
eyeball it: read the pixels. A row is 19.5px and the padding is 44px, so a band
of background rows between the last content and the status bar converts
directly into rows to remove. Two cases defeat a naive scan and need an eye
instead: `side-by-side`, whose separator paints every row, and `bordered`,
whose box edges do the same.

Changing a height changes what the TUI renders, so it takes a pass or two to
settle. It is also the reason the compact and bordered captures differ: compact
fits `9 of 9` commits in 620px where bordered needs 700 for `5 of 9`, which is
the density argument stated in the pair rather than only in prose.
