# Research: Agent session pane

**Feature**: 408-agent-session-pane | **Date**: 2026-07-21
**Method**: every layout below was verified against real installations on the
maintainer's machine (macOS, 2026-07-21), not taken from documentation. The
reference implementation (`chmouel/lazyworktree`, `internal/app/services/
agent_*.go`) was read for its two known limits (Windows bail-out, hardcoded
agent list), both of which this design must beat.

## D1 — Detection strategy: artefacts, not processes

**Decision**: read each agent's persisted session records under the user's home
area with `std::fs` only. No `ps`/`tasklist`, no OS API, no process scanning.

**Rationale**: lazyworktree's process scanner returns `nil` on Windows
(`agent_processes.go:55`) — a dead end for a tool shipped on Scoop and winget.
Artefact reading is identical on all three platforms and testable against a
`TempDir` with seeded fixtures (constitution I, SC-005).

**Alternatives considered**: process scan (rejected: Windows, testability);
hybrid (deferred: process-level liveness is explicitly post-demo in #408).

## D2 — Claude Code: the cwd-slug convention (spike result)

**Decision**: a session directory name is the absolute cwd with every character
outside `[A-Za-z0-9]` replaced by `-` (case preserved). Matching is **forward
only**: compute `slug(worktree_path)` and look the directory up — never try to
reverse a slug into a path.

**Evidence** (real `~/.claude/projects/` entries):

| Directory name | Original cwd |
|---|---|
| `-Users-kbrdn1-Projects-Perso-gwm-cli` | `/Users/kbrdn1/Projects/Perso/gwm-cli` |
| `-Users-kbrdn1--claude` | `/Users/kbrdn1/.claude` (note `.` → `-`, giving `--`) |
| `-Users-kbrdn1-cc-worktree-LazyCurl-feat-35-js-scripting` | case preserved, existing `-` kept |

31 project dirs inspected; zero contained `.` or `_`, confirming the
non-alphanumeric collapse.

**Consequences**: the mapping is lossy (`/a/b-c` and `/a/b/c` collide). Forward
matching makes this harmless: we slugify each *managed worktree path* (a set we
own) and index into the directory — O(#worktrees) lookups, no scan of all
project dirs. Session identity = `<uuid>.jsonl` filename; last activity = max
mtime of `*.jsonl` in the dir. Non-`.jsonl` entries (a `memory/` dir exists in
real data) are filtered out.

## D3 — Codex: rollout files, first line only

**Decision**: scan `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, parse **only
the first line** as JSON, take `payload.cwd` where `type == "session_meta"`.
Walk date directories newest-first and stop after a bounded window (30 days).

**Evidence**: real file
`sessions/2026/07/16/rollout-…-019f6b95….jsonl` first line:
`{"timestamp":…,"type":"session_meta","payload":{"session_id":…,"cwd":"/Users/kbrdn1/Projects/Perso/worktrees/gwm-cli/ci-381-winget-releaser",…}}`.
A legacy `rollout-2025-04-19-….json` (no `l`) exists in real data → filter on
the `.jsonl` extension, skip anything whose first line fails to parse (FR-009).

**Rationale for the bound**: the store accumulates for years; the `YYYY/MM/DD`
layout gives us recency-first traversal for free (lexicographic sort of dir
names descending). Last activity = file mtime; session id = the uuid in the
filename (fallback: `payload.session_id`).

## D4 — opencode: project index, skip `global`

**Decision**: read every `~/.local/share/opencode/storage/project/*.json`, take
the `worktree` field; skip the file whose `id` is `"global"` (its worktree is
`/`). Last activity = `time.updated` (epoch **milliseconds**), falling back to
`time.created`, falling back to file mtime.

**Evidence**: real files carry exactly
`{"id":"<sha1>","worktree":"/Users/kbrdn1/Projects/…","vcs":"git","time":{"created":1769872937353,"updated":1769872937354}}`
and a `global.json` with `"worktree": "/"`. `time.updated` beats mtime — it is
the agent's own record, and one real file shows `updated` months after
`created`.

**Note**: this store is per-*project*, not per-*session* — one entry per
worktree opencode ever opened. That is fine for our model: it yields one
"session" whose freshness is the project's last-touched time.

## D5 — Mistral Vibe: meta.json, and free liveness

**Decision**: scan `~/.vibe/logs/session/session_<ts>_<id>/meta.json`, take
`environment.working_directory`. A session with a **non-null `end_time` is
terminated** — classify idle regardless of mtime. Bound the scan by taking
directory names newest-first (the `<ts>` prefix `YYYYMMDD_HHMMSS` sorts
lexicographically).

**Evidence**: two real sessions;
`"environment":{"working_directory":"/Users/kbrdn1/Projects/…"}` confirmed; both
carry `end_time` set (ended). `start_time` appears **with and without** a
timezone suffix across versions (`2025-12-10T11:29:37.889998` vs
`2026-05-14T22:43:37.248053+00:00`) → do not parse those strings; use the
`messages.jsonl` mtime for freshness and `end_time`'s null-ness only as a
boolean.

**Bonus over the issue**: `end_time` gives Vibe a real ended-session signal the
other agents lack; the issue's "activity from mtimes" is thereby *improved* for
this backend at zero cost.

## D6 — Trait shape and dependency budget

**Decision**: one module `src/agent_sessions.rs` with `AgentKind`,
`AgentSession`, `Freshness`, a `SessionSource` trait
(`fn kind(&self) -> AgentKind; fn scan(&self, home: &Path) -> Vec<AgentSession>`)
and four unit-struct backends. Aggregation is a pure function
`summarize(&[AgentSession], &[worktree paths]) -> BTreeMap<path, WorktreeAgents>`.
**Zero new dependencies**: `serde`/`serde_json` (already in tree) for the two
JSON backends, `std::fs` for everything else, `dirs::home_dir()` (already in
tree) at the single production call site — every backend takes the base
directory as a parameter, so tests seed a `TempDir` and never touch `$HOME`.

**Rationale**: the trait is justified by four mutually incompatible schemes
(issue #408); the base-dir parameter is the same seam
`statusline::active_index_with` already uses for its canonicalizer — the
established test pattern in this codebase.

**Alternatives considered**: enum-dispatch instead of a trait (equivalent here;
trait chosen because the issue names it and backends stay file-local); a
configurable path table (rejected: YAGNI, out of scope per spec).

## D7 — Worktree matching: canonicalise, compare per-platform

**Decision**: canonicalise both sides (`std::fs::canonicalize` with graceful
fallback to lexical normalisation when the path no longer exists) and compare
case-insensitively on Windows/macOS, case-sensitively on Linux. Reuse the
existing comparison behaviour from `tests/common::paths_equal` / the
`statusline` canonicalizer-injection pattern rather than inventing a third.

**Note (Claude Code)**: slug matching (D2) is string-based *before*
canonicalisation, so the slug is computed from the worktree path exactly as
recorded; if the slug lookup misses, that backend simply reports nothing for
that worktree (FR-009), which is the accepted degradation.

## D8 — TUI integration: TaskRunner, new overlay state

**Decision**: detection runs off-thread as a new `TaskKind::AgentSessions`
(same coalescing/debounce contract as `TaskKind::Sidebar` from PR #351); the
render path only reads the last completed snapshot from app state. The overlay
is a new ratatui-free state machine `src/tui/state/detail_overlay.rs` with a
**generic row-list payload** (title + rows of label/value/style), so the future
rich PR/Issue view can reuse it unchanged. Keybinding: new `Action` variant,
default `a` on the worktree list, rebindable via the existing `[tui.keys]`
machinery; `tests/tui_app_tests.rs::help_overlay_documents_every_action` will
force help-overlay coverage automatically.

**Constraint check**: `a` must not collide with an existing list-view binding —
verified during implementation against `src/tui/keymap.rs` defaults; the keymap
conflict test catches a miss.

## D9 — Machine surfaces: additive field, SCHEMA_VERSION stays 1

**Decision**: add an optional `agents` object to `JsonWorktree`
(`#[serde(skip_serializing_if = "Option::is_none")]`), carrying the session
summary (per-agent kind, freshness, last-activity epoch seconds, session id).
This is **additive** under the documented `additionalProperties` rules →
`SCHEMA_VERSION` stays `1`; the daemon inherits it for free (daemon `list` is
byte-identical to `gwm list --format=json`); `gwm statusline` gains a compact
indicator read from the same field. The new field enters `docs/schema/` as
**experimental** tier first (same route the workspace `repo` field took), so its
exact shape can settle before it is frozen.

**Contract obligations**: `src/contract.rs` + `tests/contract_tests.rs` baseline
updated in the same changeset (constitution II); additive-compat test proves an
old consumer parsing a new payload still works.

## D10 — Freshness thresholds

**Decision**: `Active` = last activity within 300 s; `Idle` = older. Vibe's
non-null `end_time` forces `Idle`. Thresholds are `const` in the module,
documented in the schema docs; configurability is explicitly out of scope
(spec, Assumptions). Clock skew (mtime in the future) clamps to "now" →
`Active`, never an error.
