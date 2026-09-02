# `docs/_capture/`: reproducible doc captures

Every screenshot and recording under `docs/**/_assets/` is generated from a
committed [vhs](https://github.com/charmbracelet/vhs) `.tape` script here, driven
against a single deterministic demo repo. Nothing is hand-captured, so the whole
set can be regenerated after a UI change with one command.

## regenerate everything

```bash
# from anywhere in the checkout: rebuilds the demo fixture, then runs every tape
./docs/_capture/generate.sh

# reuse the existing demo fixture (faster; skips the rebuild)
GWM_KEEP_DEMO=1 ./docs/_capture/generate.sh
```

One entry point, and since [#631](https://github.com/kbrdn1/gwm-cli/issues/631)
it covers the whole set. `demo.tape` and `github-linking.tape` used to be run by
hand, which is how a run could finish on a tick over two assets it had left
stale (#575).

Requirements: `vhs`, `cargo`, a Nerd Font (`CaskaydiaCove Nerd Font Mono`, as
set in each tape's `Set FontFamily`), and `gh` for the one capture taken off the
demo repo. The run opens with a release build, so a cold tree pays a few minutes
before the first tape; a warm one pays seconds.

### the order is three constraints, not a preference

| Step | Why it cannot move |
|---|---|
| build `gwm` from this tree, its release directory first on `PATH` | the TUI header paints a `gwm X.Y.Z` chip out of `CARGO_PKG_VERSION`, so the binary vhs resolves is what the docs end up claiming. A build from an unrelated worktree renders a capture that exists, sizes correctly and shows the wrong UI: v1.10.0 came within a commit of publishing a set 175 commits stale. The directory comes from `cargo metadata`, not from `./target`, because `CARGO_TARGET_DIR` and `build.target-dir` both move it. |
| `version-stamp.tape` | asks vhs what it actually resolved, version and path, and writes both as text. The run stops when the version is not the one in `Cargo.toml`, and when the file is not the one cargo just built: a shell startup file can still put another `gwm` in front, and a stale build carrying the same version number answers the version check perfectly. |
| `github-linking.tape`, first | it opens the TUI on a real checkout of this repo, and the pane follows the selected row, which is the **main checkout** wherever the tape was launched from. So it photographs the trunk's branch, PR and uncommitted files. It runs before anything else writes under `docs/`, and only when the trunk is clean and its branch has an open PR; otherwise the script skips it and says so. During a release that means the PR is opened **before** this runs, or the capture is skipped and stays on the previous version. |
| the demo-driven tapes | the bulk of the set, against the fixture `setup-demo.sh` builds. |
| `demo.tape`, last | it creates and deletes a worktree and `rm -rf`s `.git/gwm/notes` in the fixture, so nothing else may run after it. |

The first constraint is why a release regenerates **after** the version bump is
committed, never before: v1.8.0 shipped captures advertising the previous
version, and v1.10.0 repeated it. `generate.sh` writes the version it captured
with to `captured-version.txt`, and
`docs_assets_tests::captures_were_generated_at_the_manifest_version` fails as
soon as that stops matching `Cargo.toml`. The full release sequence is in
[CONTRIBUTING.md § Releases](../../CONTRIBUTING.md#releases).

`github-linking.png` is still the one capture in the set that is not
deterministic: it photographs a live repo, so it shows the branch, history and
CI state of whatever was open at the time. The published shot was taken during
a release, which is what it is meant to document. Since it is always the trunk
in the frame, a release cut in a worktree gets a capture of `dev` rather than of
the release branch, which is a reason to cut it in the trunk.

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
  demo has no remote. `generate.sh` runs it first and points it at the checkout
  being captured through `GWM_CAPTURE_REPO`; run on its own
  (`vhs docs/_capture/github-linking.tape`) it falls back to the maintainer's
  trunk. Either way it wants a clean tree whose current branch has an open PR.

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

## everything is rendered at 2x

`Set FontSize` is 30 and every `Set Width`, `Set Height` and `Set Padding` is
twice the terminal size the capture is framed for. The pixels are a rendering
concern, not a framing one: the docs site paints a capture *wider* than its own
pixels (a 1000px `hero.png` measured 1230 CSS px in a 2560px viewport) and a
HiDPI display doubles that again, so a capture rendered at terminal scale is
upscaled before anyone reads it ([#581](https://github.com/kbrdn1/gwm-cli/issues/581)).

vhs 0.11 has no `Set Scale` (it answers `Unknown setting: Scale`), so density
comes from doubling font size and geometry together. The grid barely moves:
a column costs twice the pixels, and the residue of the division lands where it
lands.

| logical | 1x | 2x | columns 1x | columns 2x |
|:--|--:|--:|--:|--:|
| `narrow` | 800 | 1600 | 81 | 82 |
| `bootstrap`, `doctor`, `trust-ledger`, `shell-init` | 900 | 1800 | 92 | 93 |
| most of the set | 1000 | 2000 | 103 | 104 |
| `cli-agents` | 1220 | 2440 | 127 | 129 |
| `keybindings`, `keymap`, `config-panel`, `palette` | 1240 | 2480 | 130 | 131 |
| `cli-list` | 1340 | 2680 | 141 | 142 |
| `side-by-side` | 1400 | 2800 | 147 | 149 |

A gained column is only harmless while it stays on the same side of a layout
threshold, so check the two captures that exist *because* of one: `narrow` has
to stay under `SIDEBAR_MIN_WIDTH` (120) to keep showing the stacked layout, and
`side-by-side` has to stay above it. Both keep a wide margin above.

## sizes

Set per tape, not shared, and every one of them was arrived at by looking at
the result. Sizes below are the logical ones; the tapes carry twice each
number. Two rules produced the current matrix
([#544](https://github.com/kbrdn1/gwm-cli/issues/544)):

**Width targets 800-1000px**, which is where a screenshot's text renders at
roughly the size of the prose around it in a README. Columns do not follow from
pixels by intuition, so measure rather than estimate: the table above is the
current reading, and the way to take it is a throwaway tape that runs
`printf '%sx%s' $(tput cols) $(tput lines) > /tmp/cols.txt` in a `Hide` block.

Four captures sit above the band because the surface does not fit inside it,
and each is a measurement rather than a preference:

| capture | width | floor it is clearing |
|:--|--:|:--|
| `side-by-side` | 1400 | `SIDEBAR_MIN_WIDTH` is 120, and `STATUS` only stops clipping around 148 |
| `keybindings`, `keymap`, `config-panel`, `palette` | 1240 | those modals clamp to a 64-column floor, which cuts their descriptions |
| `cli-list` | 1340 | `gwm list` prints 140 columns; below that the shell wraps |
| `cli-agents` | 1220 | `gwm agents` prints 126 |

**Height is cut to the content**, one blank row above the status bar. Do not
eyeball it: read the pixels. A row is 39px at 2x and the padding is 88px, so a
band of background rows between the last content and the status bar converts
directly into rows to remove. Two cases defeat a naive scan and need an eye
instead: `side-by-side`, whose separator paints every row, and `bordered`,
whose box edges do the same.

Changing a height changes what the TUI renders, so it takes a pass or two to
settle. It is also the reason the compact and bordered captures differ: compact
fits `9 of 9` commits in 620px where bordered needs 700 for `5 of 9`, which is
the density argument stated in the pair rather than only in prose.
