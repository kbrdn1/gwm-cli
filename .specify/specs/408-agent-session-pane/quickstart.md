# Quickstart: Agent session pane

**Feature**: 408-agent-session-pane

## Exercise it for real (this machine)

```bash
cd "$(gwm path agent-session-pane)"
cargo run                      # TUI: this very worktree should show a claude
                               # indicator (this session's artefacts are live)
```

- Worktree list: `AGENT` column — glyph + freshness colour per row.
- Select a row, press `a`: detail overlay lists every matched session.
- Status pane: agent summary line for the selected worktree.

## Machine surfaces

```bash
cargo run -- list --format=json | jq '.[0].agents'   # additive field (or null)
cargo run -- statusline                              # compact indicator
```

## Exercise it hermetically (what the tests do)

Seed a fake home layout in a `TempDir` — never `$HOME`:

```text
<tmp>/claude/projects/<slug-of-worktree-path>/<uuid>.jsonl   # touch = activity
<tmp>/codex/sessions/2026/07/21/rollout-…-<uuid>.jsonl       # line 1 = session_meta JSON
<tmp>/opencode/storage/project/<sha1>.json                   # {"worktree": "...", "time": {...}}
<tmp>/vibe/logs/session/session_<ts>_<id>/meta.json          # environment.working_directory
```

then call each backend with `scan(<tmp>/…)` and `summarize(...)` with known
worktree paths. `tests/agent_sessions_tests.rs` holds these fixtures.

## Verify before push

```bash
cargo fmt && cargo clippy --all-targets --all-features -- -D warnings
cargo test
PATH="$(dirname "$(command -v cargo)"):/usr/bin:/bin" cargo test   # stripped env
```

Expected: full suite green on the stripped run too — no test in this feature
reads `$HOME` or `$PATH` (base-dir injection everywhere).
