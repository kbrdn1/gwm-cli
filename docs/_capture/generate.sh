#!/usr/bin/env bash
# Regenerate every gwm doc capture from the committed .tape scripts.
#
#   ./docs/_capture/generate.sh            # rebuild demo + all captures
#   GWM_KEEP_DEMO=1 ./docs/_capture/generate.sh   # reuse the existing demo
#
# Run from the repo root. Requires: vhs, gwm (installed), a Nerd Font.
# PNG stills come from `Screenshot` inside each tape; the tape's gif Output is
# a throwaway written under .tmp/. Animated captures Output their gif directly.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
CAP=docs/_capture
DEMO="${GWM_DEMO_ROOT:-$HOME/gwm-demo}/acme-api"
mkdir -p "$CAP/.tmp"
# vhs writes a Screenshot where it is told and does NOT create the parent, so
# a section whose first capture this is would fail with the error swallowed by
# `run()`.
mkdir -p docs/2.tui/_assets docs/3.cli/_assets docs/4.configuration/_assets

if [[ "${GWM_KEEP_DEMO:-}" != "1" ]]; then
  echo "▸ rebuilding demo repo"
  bash "$CAP/setup-demo.sh" >/dev/null
fi

# vhs exits 0 whether or not it wrote what the tape asked for, so a run has to
# be checked against the files on disk rather than against its status. Two
# distinct failures hide behind that zero: a tape that errors out (vhs still
# returns 0 for several of them), and the screenshot write racing process exit.
# That write is asynchronous, and without the `Sleep` every tape now carries
# after its `Screenshot`, a bare tape landed the file roughly one run in three.
# Both leave the previous asset in place, so the set stays plausible and goes
# stale one capture at a time. Anything a tape declares must therefore be newer
# than this run's start.
STARTED="$CAP/.tmp/.started"
: > "$STARTED"

# The `Screenshot`/`Output` targets a tape writes under docs/, one per line.
declares() {
  grep -E '^(Screenshot|Output) ' "$1" | awk '{print $2}' | grep '^docs/' || true
}

run() {
  echo "▸ $1"
  if ! vhs "$CAP/$1" >/dev/null 2>&1; then echo "  ✗ vhs failed on $1"; return 1; fi
  local missed=0 f
  while read -r f; do
    [[ -z "$f" ]] && continue
    if [[ ! -f "$f" || ! "$f" -nt "$STARTED" ]]; then
      echo "  ✗ $1 left $f untouched (vhs exited 0)"; missed=1
    fi
  done < <(declares "$CAP/$1")
  return $missed
}

# One retry: the race above is the common cause and it does not repeat.
#
# A retry is only safe on a tape that can run twice against the same fixture,
# and four of them mutate it. Three were already written that way: `demo` and
# `first-worktree` destroy their own state in their opening `Hide` block (the
# latter `rm -rf`s its whole sandbox), and `trust-ledger` answers `n` to the
# TOFU prompt, so its `gwm create` never happens. `bootstrap` was the exception
# and is now torn down at both ends. Any new tape that creates a worktree, a
# branch or a commit owes the same pre-clean before it can go through here.
run_checked() { run "$1" || { echo "  ↻ retrying $1"; run "$1"; }; }

# ── still + animated captures that use the default (grey) theme ────────────
FAILED=()
for t in hero sidebar side-by-side narrow  palette keymap keybindings \
         doctor trust-ledger bootstrap \
         agents cli-list cli-agents config-panel launchers open-dispatch \
         filter countdown first-worktree shell-init; do
  [[ -f "$CAP/$t.tape" ]] || continue
  run_checked "$t.tape" || FAILED+=("$t.tape")
done

# ── theme gallery: inject each preset into the demo config, then capture ───
# `--assume-unchanged` hides the temporary [theme] edit from git status so the
# trunk worktree still reads "clean" — matching every other capture.
#
# The restore runs on EXIT, not on the happy path: `set -e` plus a `mv` of a
# screenshot vhs never produced would otherwise abandon the demo repo with an
# injected [theme] block and the assume-unchanged bit still set — which is
# exactly what a concurrent run of this script produced once. Nothing here
# guards against two copies running at the same time; don't.
restore_demo_config() {
  [[ -f "$CAP/.tmp/gwm.toml.bak" ]] || return 0
  cp "$CAP/.tmp/gwm.toml.bak" "$DEMO/.gwm.toml"
  git -C "$DEMO" update-index --no-assume-unchanged .gwm.toml
}
if [[ -f "$CAP/theme.tape" ]]; then
  cp "$DEMO/.gwm.toml" "$CAP/.tmp/gwm.toml.bak"
  git -C "$DEMO" update-index --assume-unchanged .gwm.toml
  trap restore_demo_config EXIT
  for preset in catppuccin gruvbox tokyo-night claude-dark; do
    echo "▸ theme: $preset"
    { cat "$CAP/.tmp/gwm.toml.bak"; printf '\n[theme]\npreset = "%s"\n' "$preset"; } > "$DEMO/.gwm.toml"
    # `-nt "$STARTED"`, not merely `-f`: the target is moved out after every
    # preset, so a leftover from an earlier run would satisfy an existence
    # check and publish the wrong palette under this preset's name.
    if ! vhs "$CAP/theme.tape" >/dev/null 2>&1 || [[ ! "$CAP/.tmp/theme.png" -nt "$STARTED" ]]; then
      echo "  ↻ retrying theme.tape ($preset)"
      if ! vhs "$CAP/theme.tape" >/dev/null 2>&1 || [[ ! "$CAP/.tmp/theme.png" -nt "$STARTED" ]]; then
        echo "  ✗ vhs failed on theme.tape ($preset)"
        exit 1
      fi
    fi
    mv "$CAP/.tmp/theme.png" "docs/2.tui/_assets/theme-$preset.png"
  done
  restore_demo_config
  trap - EXIT
fi

# ── bordered layout: inject the opt-out, then capture (issue #545) ─────────
# Compact is the default, so every capture above already shows it. This one
# documents the opt-out. Same injection dance as the theme gallery: the mode is
# a config key, so the only way to capture it is to write it into the demo repo
# and hide the edit from git status while the shot is taken.
if [[ -f "$CAP/bordered.tape" ]]; then
  echo "▸ bordered layout"
  cp "$DEMO/.gwm.toml" "$CAP/.tmp/gwm.toml.bak"
  git -C "$DEMO" update-index --assume-unchanged .gwm.toml
  trap restore_demo_config EXIT
  # Pin the preset too: compact paints a background role, so the pair of
  # captures must not inherit whatever theme the capture machine has in its
  # global config. Both tapes' terminal background is matched to this palette.
  { cat "$CAP/.tmp/gwm.toml.bak"; printf '\n[tui]\nlayout = "bordered"\n\n[theme]\npreset = "claude-dark"\n'; } > "$DEMO/.gwm.toml"
  # Capture the status rather than swallowing it: the restore below must run
  # whatever happens (the demo repo would keep a `[tui] layout` block and an
  # `assume-unchanged` flag otherwise), but a failed vhs still has to fail the
  # script — `|| echo` would let it print "✓ captures regenerated" over a
  # missing or stale PNG (Codex review, PR #546). `run` covers the other half
  # that review could not see: vhs returning 0 without writing the PNG.
  run bordered.tape; rc=$?
  if [[ $rc -ne 0 ]]; then run bordered.tape; rc=$?; fi
  restore_demo_config
  trap - EXIT
  [[ $rc -ne 0 ]] && { echo "  ✗ vhs failed on bordered.tape"; exit $rc; }
fi

# ── shrink PNGs for the repo (lossless) ────────────────────────────────────
if command -v oxipng >/dev/null 2>&1; then
  echo "▸ optimising PNGs"
  find docs -path '*/_assets/*.png' -exec oxipng -o 2 --strip safe -q {} + || true
fi

rm -f "$CAP"/.tmp/*.gif "$CAP"/.tmp/*.png "$STARTED" 2>/dev/null || true

# The whole point of the checks above: say which tapes did not land rather than
# printing a tick over a set that is stale in places.
if (( ${#FAILED[@]} )); then
  echo "✗ ${#FAILED[@]} tape(s) did not produce their asset, twice: ${FAILED[*]}"
  exit 1
fi
echo "✓ captures regenerated"
