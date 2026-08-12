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

run() { echo "▸ $1"; vhs "$CAP/$1" >/dev/null 2>&1 || { echo "  ✗ vhs failed on $1"; return 1; }; }

# ── still + animated captures that use the default (grey) theme ────────────
for t in hero sidebar side-by-side narrow  palette keymap keybindings \
         doctor trust-ledger bootstrap \
         agents cli-list cli-agents config-panel launchers open-dispatch \
         filter countdown first-worktree shell-init; do
  [[ -f "$CAP/$t.tape" ]] && run "$t.tape"
done

# ── theme gallery: inject each preset into the demo config, then capture ───
# `--assume-unchanged` hides the temporary [theme] edit from git status so the
# trunk worktree still reads "clean" — matching every other capture.
if [[ -f "$CAP/theme.tape" ]]; then
  cp "$DEMO/.gwm.toml" "$CAP/.tmp/gwm.toml.bak"
  git -C "$DEMO" update-index --assume-unchanged .gwm.toml
  for preset in catppuccin gruvbox tokyo-night claude-dark; do
    echo "▸ theme: $preset"
    { cat "$CAP/.tmp/gwm.toml.bak"; printf '\n[theme]\npreset = "%s"\n' "$preset"; } > "$DEMO/.gwm.toml"
    vhs "$CAP/theme.tape" >/dev/null 2>&1
    mv "$CAP/.tmp/theme.png" "docs/2.tui/_assets/theme-$preset.png"
  done
  cp "$CAP/.tmp/gwm.toml.bak" "$DEMO/.gwm.toml"
  git -C "$DEMO" update-index --no-assume-unchanged .gwm.toml
fi

# ── compact layout: inject `[tui] compact`, then capture (issue #545) ──────
# Same injection dance as the theme gallery above, for the same reason: the
# mode is a config key, so the only way to capture it is to write it into the
# demo repo and hide the edit from git status while the shot is taken.
if [[ -f "$CAP/compact.tape" ]]; then
  echo "▸ compact layout"
  cp "$DEMO/.gwm.toml" "$CAP/.tmp/gwm.toml.bak"
  git -C "$DEMO" update-index --assume-unchanged .gwm.toml
  # Pin the preset too: this capture's subject is a background role, so it
  # must not inherit whatever theme the capture machine has in its global
  # config. The tape's terminal background is matched to this palette.
  { cat "$CAP/.tmp/gwm.toml.bak"; printf '\n[tui]\ncompact = true\n\n[theme]\npreset = "claude-dark"\n'; } > "$DEMO/.gwm.toml"
  # Capture the status rather than swallowing it: the restore below must
  # run whatever happens (the demo repo would keep a `[tui] compact` block
  # and an `assume-unchanged` flag otherwise), but a failed vhs still has
  # to fail the script — `|| echo` would let it print "✓ captures
  # regenerated" over a missing or stale PNG (Codex review, PR #546).
  vhs "$CAP/compact.tape" >/dev/null 2>&1; rc=$?
  cp "$CAP/.tmp/gwm.toml.bak" "$DEMO/.gwm.toml"
  git -C "$DEMO" update-index --no-assume-unchanged .gwm.toml
  [[ $rc -ne 0 ]] && { echo "  ✗ vhs failed on compact.tape"; exit $rc; }
fi

# ── shrink PNGs for the repo (lossless) ────────────────────────────────────
if command -v oxipng >/dev/null 2>&1; then
  echo "▸ optimising PNGs"
  find docs -path '*/_assets/*.png' -exec oxipng -o 2 --strip safe -q {} + || true
fi

rm -f "$CAP"/.tmp/*.gif "$CAP"/.tmp/*.png 2>/dev/null || true
echo "✓ captures regenerated"
