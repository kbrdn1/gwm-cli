# Contract Delta: `agents` field on the worktree list row

**Feature**: 408-agent-session-pane | **Status**: additive, experimental tier
**SCHEMA_VERSION**: stays `1` (additive change under the documented
`additionalProperties` rules — same route as the workspace `repo` field)

## Where it appears

- `gwm list --format=json` → each worktree row
- daemon `list` result and `worktrees.changed` notification (byte-identical to
  the CLI payload by construction — no separate daemon work)
- consumed by `gwm statusline` for its compact indicator

## Shape

```jsonc
{
  "name": "feat-408-agent-session-pane",
  // …existing row fields, all unchanged…
  "agents": {                     // OPTIONAL — omitted when no session matched
    "top": {                      // most recently active session (compact surfaces)
      "kind": "claude",           // "claude" | "codex" | "opencode" | "vibe"
      "freshness": "active",      // "active" | "idle"
      "last_activity": 1784480000, // epoch seconds, UTC
      "id": "a7820111-8232-4857-9a4f-bb4f514024d9"
    },
    "sessions": [                 // full set, most recent first (overlay parity)
      { "kind": "claude", "freshness": "active", "last_activity": 1784480000, "id": "…" },
      { "kind": "codex",  "freshness": "idle",   "last_activity": 1784470000, "id": "…" }
    ]
  }
}
```

## Rules

1. **Additive only.** No existing field changes name, type, or meaning. An old
   consumer that ignores unknown fields keeps working — pinned by an
   additive-compat test in `tests/contract_tests.rs`.
2. **Omitted, not null/empty.** A worktree with no matched session has **no**
   `agents` key (`skip_serializing_if`), so payload size is unchanged for
   non-users (SC-003, SC-004).
3. **Experimental tier.** The field enters `docs/schema/` as *experimental*:
   its shape may still change in a minor while it settles. Promotion to the
   stable tier is a later, deliberate act. Consumers are told via the tiers
   table in `docs/schema/README.md`.
4. **Stable strings.** `kind` and `freshness` values are lowercase, frozen at
   the moment the field is promoted to stable; until then they follow the
   experimental rules.
5. **No CLI change.** No new subcommand or flag; exit codes untouched.

## Enforcement (constitution II)

- `src/contract.rs`: field registered with the other schema knowledge in the
  same changeset.
- `tests/contract_tests.rs`: baseline updated + additive-compat case (parse a
  new payload with a struct that lacks `agents` → success).
- `docs/schema/`: field + tier documented, EN and FR.
