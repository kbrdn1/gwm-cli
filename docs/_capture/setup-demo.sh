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
