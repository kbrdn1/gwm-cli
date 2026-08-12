#!/usr/bin/env bash
# Rebuild the deterministic demo repo used for every gwm doc capture.
#
# One fixture, many captures (issue #206): a single `acme-api` repo with a
# branchy trunk history (so the sidebar commit graph has topology to draw) and
# four worktrees in deliberately varied git states (clean / dirty / staged /
# ahead) so the hero + sidebar shots look like a real day of work.
#
# Idempotent: wipes and rebuilds ~/gwm-demo from scratch. Dates are pinned so
# the commit graph and ordering are byte-stable across runs (relative "x ago"
# times still drift — that is cosmetic and fine for docs).
set -euo pipefail

ROOT="${GWM_DEMO_ROOT:-$HOME/gwm-demo}"
REPO="$ROOT/acme-api"

export GIT_AUTHOR_NAME="Robin Vale"
export GIT_AUTHOR_EMAIL="robin@acme.dev"
export GIT_COMMITTER_NAME="Robin Vale"
export GIT_COMMITTER_EMAIL="robin@acme.dev"

# pinned clock, bumped per commit so ordering is stable
_T=1719000000
_stamp() { _T=$((_T + 3600)); export GIT_AUTHOR_DATE="$_T +0000" GIT_COMMITTER_DATE="$_T +0000"; }
commit() { _stamp; git commit -q "$@"; }

rm -rf "$ROOT"
mkdir -p "$REPO"
cd "$REPO"

git init -q -b main
git config commit.gpgsign false

# ── trunk history: a small branchy graph so `○ ◎ │ ╮ ╭ ╯` has something to render
mkdir -p src
printf 'fn main() { println!("acme-api up"); }\n' > src/main.rs
printf '# acme-api\n\nA tiny REST service.\n' > README.md
printf 'PORT=3000\nDATABASE_URL=postgres://localhost/acme\n' > .env.example
printf '/target\n.env\n' > .gitignore
git add -A && commit -m "🎉 init acme-api"

printf 'PORT=3000\nDATABASE_URL=postgres://localhost/acme\nLOG_LEVEL=info\n' > .env.example
git add -A && commit -m "🔧 chore: add LOG_LEVEL to env example"

# a merged feature branch → gives the graph a real fork/join
git switch -q -c feat/router
mkdir -p src/routes
printf 'pub fn health() -> &'"'"'static str { "ok" }\n' > src/routes/health.rs
git add -A && commit -m "✨ feat(router): mount /health"
git switch -q main
printf 'axum = "0.7"\n' > Cargo.deps
git add -A && commit -m "📦 build: pin axum 0.7"
_stamp; git merge -q --no-ff feat/router -m "🔀 merge: router foundation"
git branch -q -d feat/router

printf 'tokio = { version = "1", features = ["full"] }\n' >> Cargo.deps
git add -A && commit -m "📦 build: add tokio runtime"

# the .gwm.toml the worktrees bootstrap from
cat > .gwm.toml <<'TOML'
# gwm — worktree bootstrap for acme-api
[worktree]
base           = "{repo_parent}/worktrees/{repo}"
path_pattern   = "{type}-{issue}-{desc}"
branch_pattern = "{type}/#{issue}-{desc}"

# copy the untracked local env file into every fresh worktree
[[bootstrap.copy]]
from = ".env.example"
to   = ".env"
required = true

# keep each worktree's build cache local (never symlink it from main)
[[bootstrap.no_symlink]]
path = "target"

# post-create hooks — run once the worktree exists
[[bootstrap.command]]
name = "install dependencies"
run  = "echo '  42 packages installed'"

[[bootstrap.command]]
name = "seed local database"
run  = "echo '  schema applied · 12 fixtures loaded'"
TOML
git add -A && commit -m "🔧 chore: add gwm worktree config"

# ── four worktrees, four git states ───────────────────────────────────────
mk() { gwm create --allow-bootstrap "$@" >/dev/null 2>&1; }
wt() { echo "$ROOT/worktrees/acme-api/$1"; }

mk feat 42 payment-webhooks
mk fix  57 rate-limit-headers
mk chore 63 bump-axum
mk docs 71 openapi-examples

# feat/#42 → dirty + ahead by 2 (active work in progress)
cd "$(wt feat-42-payment-webhooks)"
mkdir -p src/routes
printf 'pub async fn stripe(_: String) -> u16 { 200 }\n' > src/routes/webhooks.rs
git add -A && commit -m "✨ feat(webhooks): stripe endpoint skeleton"
printf 'pub async fn verify_sig() -> bool { true }\n' >> src/routes/webhooks.rs
git add -A && commit -m "🔒 feat(webhooks): signature verification"
# leave it dirty: one modified tracked file + one untracked
printf '// TODO: idempotency keys\n' >> src/routes/webhooks.rs
printf 'STRIPE_SECRET=sk_test_xxx\n' >> .env

# fix/#57 → clean + ahead by 1
cd "$(wt fix-57-rate-limit-headers)"
printf 'pub const RETRY_AFTER: &str = "Retry-After";\n' > src/routes/ratelimit.rs
git add -A && commit -m "🐛 fix(http): emit Retry-After on 429"

# chore/#63 → staged change, not committed
cd "$(wt chore-63-bump-axum)"
printf 'axum = "0.8"\n' > Cargo.deps
git add Cargo.deps

# docs/#71 → clean, no local commits (fresh branch)
cd "$(wt docs-71-openapi-examples)" >/dev/null

# ── backdate the branch ages ──────────────────────────────────────────────
# The AGE column reads `branch.<b>.gwm-created-at`, which `gwm create` stamps
# with the wall clock — so a capture taken right after this script shows five
# worktrees all seconds old, which reads as staged rather than as a week of
# work. Offsets are relative to now, so the spread stays stable whenever the
# recording happens (issue #523; this is the "relative times drift" wart the
# header comment flags, closed for the four fixture worktrees).
now=$(date +%s)
age() { git -C "$REPO" config "branch.$1.gwm-created-at" "$((now - $2))"; }
age 'feat/#42-payment-webhooks'   $((9 * 86400))
age 'fix/#57-rate-limit-headers'  $((4 * 86400))
age 'chore/#63-bump-axum'         $((2 * 86400))
age 'docs/#71-openapi-examples'   $((5 * 3600))

# ── fabricated agent sessions (issue #523) ────────────────────────────────
# The repo's pitch is "shows which AI agent is working where", so the demo
# fixture has to carry agent artefacts. `GWM_AGENTS_HOME` is gwm's own
# artefact-root seam, so they live INSIDE $ROOT: the real ~/.claude and
# ~/.codex are never written to, and the `rm -rf "$ROOT"` above cleans them.
AGENTS_HOME="$ROOT/agents-home"
export GWM_AGENTS_HOME="$AGENTS_HOME"
CLAUDE_SID=2f8c1a94-7d0e-4b52-9c31-6ae08f5db417
CODEX_SID=0199f2b6-4c7a-7331-8d15-2be9c04af8e1

# Claude Code: one `.jsonl` per session under the slugged worktree path
# (every non-alphanumeric byte becomes `-`); the first user message is the
# session name gwm displays.
claude_slug=$(printf '%s' "$(wt feat-42-payment-webhooks)" | sed 's/[^A-Za-z0-9]/-/g')
mkdir -p "$AGENTS_HOME/.claude/projects/$claude_slug"
printf '%s\n' '{"type":"user","message":{"content":"Add idempotency keys to the Stripe webhook handler"}}' \
  > "$AGENTS_HOME/.claude/projects/$claude_slug/$CLAUDE_SID.jsonl"

# Codex: a rollout under today's day-dir, worktree carried by the
# `session_meta` first line; the thread name comes from session_index.jsonl.
codex_day="$AGENTS_HOME/.codex/sessions/$(date -u +%Y/%m/%d)"
mkdir -p "$codex_day"
printf '{"type":"session_meta","payload":{"id":"%s","cwd":"%s"}}\n' \
  "$CODEX_SID" "$(wt fix-57-rate-limit-headers)" > "$codex_day/rollout-$CODEX_SID.jsonl"
printf '{"id":"%s","thread_name":"Emit Retry-After on every 429"}\n' \
  "$CODEX_SID" > "$AGENTS_HOME/.codex/session_index.jsonl"

# Pin both. The table's AGENT column reads detection, the sidebar Agents
# pane reads pins only — the demo shows both, so both are needed.
#
# Non-fatal on purpose. `gwm agents attach` refuses an id detection has not
# seen, so it is the one step here that can fail for an environmental reason,
# and under `set -e` that would abort the script before the untrusted
# `payments-svc` fixture below — costing `trust-ledger.tape` its subject over
# a missing pin. Warn loudly instead: the pane degrades, the rest survives.
cd "$REPO"
pin() {
  gwm agents attach "$1" "$2" >/dev/null \
    || echo "warning: could not pin $2 to $1 — the sidebar Agents pane will be empty" >&2
}
pin feat-42 "$CLAUDE_SID"
pin fix-57 "$CODEX_SID"

# ── untrusted fixture for the TOFU trust-ledger capture ────────────────────
# A second repo we deliberately never add to the trust ledger, with a juicy
# bootstrap surface so the first-run "Trust this .gwm.toml?" prompt has content.
UNTRUSTED="$ROOT/payments-svc"
mkdir -p "$UNTRUSTED/src"
cd "$UNTRUSTED"
git init -q -b main
git config commit.gpgsign false
printf 'fn main() { println!("payments-svc"); }\n' > src/main.rs
printf 'STRIPE_KEY=sk_test_xxx\nPORT=8080\n' > .env.example
cat > .gwm.toml <<'TOML'
[worktree]
base           = "{repo_parent}/worktrees/{repo}"
branch_pattern = "{type}/#{issue}-{desc}"

[[bootstrap.copy]]
from = ".env.example"
to   = ".env"
required = true

[[bootstrap.command]]
name = "install dependencies"
run  = "npm ci"

[[bootstrap.command]]
name = "run database migrations"
run  = "./scripts/migrate.sh"
TOML
git add -A && commit -m "🎉 init payments-svc"

echo "demo rebuilt at $REPO"
echo "untrusted fixture at $UNTRUSTED"
gwm list 2>/dev/null || true
