#!/usr/bin/env bash
# Re-stamp the demo's seeded agent artefacts (issue #524).
#
# Session freshness is a pure function of artefact mtime: `active` under
# ACTIVE_WINDOW (300 s), `idle` past it, and gone past SCAN_WINDOW (30 days).
# A fixture built ten minutes ago therefore renders every row grey, and the
# agent captures would document something the code never shows. Owning the
# ages in one place means `setup-demo.sh` and every agent tape agree.
#
#   bash docs/_capture/refresh-agents.sh
set -euo pipefail

ROOT="${GWM_DEMO_ROOT:-$HOME/gwm-demo}"
AGENTS="${GWM_AGENTS_HOME:-$ROOT/agents-home}"
[[ -d "$AGENTS" ]] || { echo "no agent store at $AGENTS — run setup-demo.sh first" >&2; exit 1; }

# Backdate to `now - seconds`. perl's utime over `touch -d` / `touch -A`,
# whose relative-time flags differ between BSD and GNU.
age() {
  perl -e '
    my ($secs, @files) = @ARGV;
    my $t = time - $secs;
    @files = grep { -f } @files;
    die "no such artefact\n" unless @files;
    utime($t, $t, @files) == @files or die "utime: $!\n";
  ' "$@"
}

# Default: everything just wrote, so every session reads `active`.
find "$AGENTS" -name '*.jsonl' -exec touch {} +

# The two the captures show as `idle`, so the overlay documents both states.
age 10800 "$AGENTS"/.claude/projects/*feat-42*/019f6b95-*.jsonl
age 7200 "$AGENTS"/.claude/projects/*docs-71*/019f6a04-*.jsonl
