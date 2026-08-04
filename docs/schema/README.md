# gwm machine contracts

gwm exposes three **machine-readable** surfaces that tooling outside this
repo depends on: editor plugins, status bars, CI scripts. As of 1.0 these
are **frozen and versioned** (issue #317): a backward-incompatible change to
a *stable* field is a conscious breaking decision, not an accident.

The freeze is enforced by `tests/contract_tests.rs`. The single source of
truth for the version and the section/method sets is
[`src/contract.rs`](../../src/contract.rs).

## The three surfaces

| Surface | Where | Documented by |
|---------|-------|---------------|
| **JSON output** | `gwm {list,doctor,path} --format=json` and `gwm status --json` | the `*.schema.json` files here |
| **Daemon JSON-RPC 2.0** | `gwm daemon` over a unix socket | [`src/daemon.rs`](../../src/daemon.rs) |
| **`.gwm.toml` config** | per-repo config file | [`docs/4.configuration`](../4.configuration/) |

The JSON output and the daemon RPC results are **byte-identical** (a daemon
`list` result is the same bytes as `gwm list --format=json`), because both
are built from the same DTOs in [`src/json_api.rs`](../../src/json_api.rs).
They therefore share one version, `SCHEMA_VERSION`.

## Versioning policy

`contract::SCHEMA_VERSION` (currently **1**) is bumped **only** on a
backward-incompatible change to a stable field:

- renaming or removing a field,
- changing its type or its documented meaning.

Adding a new optional field is **backward-compatible** and does NOT bump the
version, and consumers MUST ignore unknown fields. To keep that promise true
even for consumers that strict-validate against these files, the object
schemas use `additionalProperties: true` (the JSON Schema default): an output
carrying a field added after a consumer pulled its schema copy still
validates. A contract test pins this so the constraint can't be silently
re-tightened. (gwm's own CI still rejects an *undocumented* field via the
`serialized ⊆ properties` parity test, so the producer side stays honest.)

`.gwm.toml` is the mirror image: it is *input*, so it keeps
`deny_unknown_fields`: an unknown section is a user typo to reject, not a
forward-compatible addition to ignore.

### How a consumer detects drift

- **Daemon `subscribe` clients** (long-lived): read
  `params.schema_version` on each `worktrees.changed` notification. It
  carries `SCHEMA_VERSION`; a value you weren't built for means the contract
  moved.
- **One-shot CLI consumers**: key off `gwm --version`. The array-typed
  `list` output can't carry a top-level version field without itself
  becoming a breaking change, so the tool version is the drift signal.
- Each `*.schema.json` file also declares its `version` for offline
  validation tooling.

## Tiers: stable vs experimental

Every field/section is one of:

- **Stable**: frozen by a contract test. A rename/removal fails CI. All
  fields below not marked experimental are stable.
- **Experimental**: may change without a version bump.

### JSON output (`worktree-list.schema.json`)

Stable: `name`, `id`, `path`, `branch`, `head`, `is_main`, `is_locked`,
`is_prunable`, `status` (`is_dirty`, `has_upstream`, `ahead`, `behind`,
`unknown`), `age_seconds`, `issue`, `pr`.

| Field | Tier | Note |
|-------|------|------|
| `repo` | **experimental** | present only in `--workspace` mode; rides the young workspace feature (#36). In `properties` but never `required`. |
| `agents` | **experimental** | agent-session summary (#408): `{top, sessions[]}` of `{kind, freshness, last_activity, id}`. Additive, so it is omitted entirely (never `null`) when no session matched. In `properties` but never `required`; its shape may still change in a minor while the feature settles. |

`doctor.schema.json` (`checks[]`, `severity`, `exit_code`) and
`path.schema.json` (`name`, `path`, `branch`) are entirely **stable**.

`status.schema.json` (`gwm status --json`) is **stable**: top-level `branch`,
`issue` (`null` or `{number, source, …}`), `pr` (`null` or
`{number, source, …}`). Like the workspace `repo`, the top-level `repo`
(GitHub slug, present only with a remote) is **experimental**. The nested live
fields (`state`, `title`, `labels`, `url`, `checks_passed`, `checks_total`)
appear only when `gh` resolved the link; their names/types are stable when
present.

### Daemon JSON-RPC

Stable methods: `list`, `doctor`, `path`, `subscribe`. Stable notification:
`worktrees.changed`. Stable error codes: the JSON-RPC 2.0 standard set
(`-32700`/`-32600`/`-32601`/`-32602`/`-32603`). Adding a method is additive
(non-breaking); renaming or removing one is breaking.

### `.gwm.toml` config

The top-level section set is **stable** and pinned by
`contract::CONFIG_SECTIONS`: `worktree`, `bootstrap`, `hooks`, `doctor`,
`tui`, `theme`, `git_tui`, `review`, `labels`, `milestones`, `branch_types`,
`aliases`, `gitmoji`, `issue_template`, `pr_template`, `exec`, `clean`.
Individual keys
within a section follow the same stable-by-default rule; experimental keys
are called out in the configuration docs. `deny_unknown_fields` means an
unknown section is a hard error, so a renamed section can't pass silently.
