# Data Model: Agent session pane

**Feature**: 408-agent-session-pane | **Date**: 2026-07-21

All types live in `src/agent_sessions.rs` unless noted. Everything is plain
data — no interior mutability, no lifetimes beyond `&Path` parameters.

## AgentKind

Closed enum of supported agents.

| Variant | Display | Compact glyph (TUI/statusline) |
|---|---|---|
| `ClaudeCode` | `claude` | `c` (nerd-font glyph chosen at implementation, themed) |
| `Codex` | `codex` | `x` |
| `Opencode` | `opencode` | `o` |
| `Vibe` | `vibe` | `v` |

- Serialized (JSON contract) as its lowercase display name — stable string,
  documented in `contracts/agents-field.md`.
- Extension = add a variant + a backend; no surface reworks (spec, Key
  Entities).

## Freshness

| Variant | Rule |
|---|---|
| `Active` | last activity ≤ 300 s ago (clock skew into the future clamps to now → `Active`) |
| `Idle` | anything older; **forced** for a Vibe session whose `end_time` is non-null |

Threshold is a module `const` (`ACTIVE_WINDOW: Duration`), documented, not
configurable (out of scope per spec).

## AgentSession

One detected session, before worktree matching.

| Field | Type | Source |
|---|---|---|
| `kind` | `AgentKind` | backend identity |
| `cwd` | `PathBuf` | recorded working directory (D2–D5 per backend) |
| `last_activity` | `SystemTime` | backend-specific: newest `.jsonl` mtime (Claude), file mtime (Codex), `time.updated` ms epoch (opencode), `messages.jsonl` mtime (Vibe) |
| `ended` | `bool` | only Vibe can set `true` (non-null `end_time`); others always `false` |
| `id` | `String` | uuid from filename (Claude/Codex), `id` field (opencode), dir-name id suffix (Vibe) |

Validation: a backend yields a session only if it recovered a non-empty `cwd`
and a plausible `last_activity`; anything else is skipped silently (FR-009).
opencode's `global.json` (`worktree: "/"`) is skipped by id.

## SessionSource (trait)

```text
kind() -> AgentKind
scan(base: &Path) -> Vec<AgentSession>   // base = the agent's artefact root
```

- `base` is the injection seam: production passes the real location derived
  from `dirs::home_dir()`; tests pass a seeded `TempDir` sub-path. No backend
  ever reads `$HOME` itself.
- Four unit-struct impls: `ClaudeCodeSource`, `CodexSource`, `OpencodeSource`,
  `VibeSource`.
- Scan bounds (D3, D5): Codex walks date dirs newest-first, 30-day window;
  Vibe takes session dirs newest-first (lexicographic on the `<ts>` prefix);
  Claude is O(#worktrees) by slug lookup — the scan step only harvests
  activity inside already-matched dirs; opencode reads the whole (small)
  project index.

## WorktreeAgents (aggregation output)

Per-worktree summary, produced by pure
`summarize(sessions: &[AgentSession], worktrees: &[(id, PathBuf)]) -> BTreeMap<id, WorktreeAgents>`.

| Field | Type | Meaning |
|---|---|---|
| `sessions` | `Vec<AgentSession>` | all sessions matched to this worktree, most recent first |
| `top` | index/copy of the most recently `Active`-leaning session | what compact surfaces display |

Matching rule (D7): canonicalise both sides when the paths exist (lexical
fallback otherwise), case-insensitive comparison on Windows/macOS,
case-sensitive on Linux — same behaviour family as `tests/common::paths_equal`
and the `statusline` canonicalizer seam.

## State transitions (TUI)

```text
AgentSnapshot (app state) ── TaskKind::AgentSessions completes ──> replaced atomically
      │                                                        (debounced like Sidebar)
      └── read-only from draw: table column, Status pane line

DetailOverlay (src/tui/state/detail_overlay.rs — generic):
  Closed ── Action::AgentSessions on a row ──> Open{title, rows}
  Open   ── dismiss key ──────────────────────> Closed (list state untouched)
  Open on session-less worktree ─────────────> Open with a single "no agent
                                               session found" row (never empty)
```

`DetailOverlay` rows are `(label, value, style-role)` triples — content-agnostic
by design so the future rich PR/Issue view reuses the same state machine.

## JSON contract delta (details in contracts/agents-field.md)

`JsonWorktree` gains `agents: Option<JsonWorktreeAgents>` —
`skip_serializing_if = None`, experimental tier, additive, SCHEMA_VERSION
stays 1. The daemon inherits it (list payloads are byte-identical by
construction); `statusline::render` reads the same struct.
