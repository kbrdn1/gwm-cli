# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Multi-forge support: a `Forge` trait and a GitLab (`glab`) backend**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). Issue / PR lookups
  now go through a `Forge` abstraction with two implementations: the existing
  GitHub one (`gh`) and a new GitLab one (`glab`). Worktrees, bootstrap,
  branch naming and the `branch.<name>.gwm-*` link storage are unchanged and
  forge-neutral — only the network layer knows which forge is in play.
  - New `forge = "github" | "gitlab"` key in `.gwm.toml`. Omitted, the forge
    is inferred from the `origin` host; a **self-hosted** instance lives on
    an arbitrary domain and cannot be detected from the URL, so the explicit
    key is the supported way in and always wins over inference.
  - A host gwm does not recognise is **not** assumed to be GitHub. Only the
    vendors' own domains resolve without configuration — `github.com`,
    `ghe.com`, `gitlab.com`; a `gitlab.*` hostname is chosen by whoever owns
    the domain and states nothing. Anything else needs the `forge` key, and
    gwm reports that rather than guessing. Guessing would have sent an
    authenticated `gh` call — and a `$GH_ENTERPRISE_TOKEN` — to whatever host
    a cloned repo's `origin` happened to name.
  - `$GWM_GLAB` overrides the `glab` binary, mirroring `$GWM_GH`.
  - `gwm doctor` probes the forge CLI, but only when `forge` is set
    explicitly, so repos that never opt in gain no new warning.
  - GitLab specifics absorbed at the parse boundary: `iid` (not `id`) as the
    user-visible number, `opened`/`locked` states, nested subgroup paths,
    `#RRGGBB` label colours, `due_date` vs `due_on`, and `state_event` for
    milestone transitions.

### Changed

- **Repo-slug extraction is host-agnostic**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `origin` URLs are
  now parsed into host + path for any host, in both scp-like (`git@host:path`)
  and scheme-ful (`ssh://`, `https://`) forms. Pre-#419 a non-`github.com`
  remote was rejected outright, which made a GitLab remote unusable before
  the backend got a say. Issue / PR URLs are built from the parsed host
  instead of a hardcoded `https://github.com/`, so self-hosted instances get
  links to their own server.
- **An unrecognised CI state is reported as unknown, never as green**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `CheckOutcome`
  gained an explicit `Unknown` variant, rendered as its own row in the CI
  checks overlay. It aggregates as non-green, so a forge state this build
  does not know can no longer paint a passing CI that is not passing.

### Fixed

- **A blocking `manual` GitLab pipeline no longer reads as green**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). It was mapped to
  passing by analogy with GitHub's `SKIPPED`, but a pipeline reports `manual`
  while it waits on a *blocking* manual job — suspended, and possibly barring
  the merge.
- **Generated URLs keep the remote's scheme and web port**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `http://host:8080/…`
  was collapsed to the bare host and rebuilt as `https://host/…`, so `gwm open`
  produced dead links for self-hosted instances. An `ssh://host:2222` port is
  still dropped — that one addresses sshd, not the web UI.
- **`$GITLAB_HOST` is pinned on every `glab` call**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `glab` otherwise
  resolves the instance from the process working directory, which in workspace
  mode is the workspace root rather than the row's repo — a same-named project
  on another instance could be read and its iid persisted locally.
- **Issue / MR bodies are redacted from Command Logs**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `glab` has no
  `--body-file`, so the rendered body rides in `--description`; the transcript
  now stores its length instead of its contents.
- **A milestone `due_on` carrying a time is refused on GitLab**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). GitLab's `due_date`
  is date-only, so such a value could never converge and was rewritten on every
  push. It now fails with the cause named.
- **`$GH_HOST` is pinned for a GitHub Enterprise origin**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). Host-agnostic slug
  parsing made non-github.com hosts reachable for the first time, and without
  the pin `gh` silently targeted github.com and could read a same-named repo on
  another tenant.
- **A guessed origin never overrides the forge CLI's own configuration**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). An SSH remote carries
  no web scheme or port, so `https://<ssh-host>` is a guess: good enough to
  build a link, not good enough to force through `$GITLAB_HOST` / `$GH_HOST`
  over a `glab` / `gh` setup that may name a different web hostname. Same for
  the no-origin creation fallback, which briefly forced gitlab.com.
- **Clearing a label or milestone field in `.gwm.toml` clears it upstream**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). Dropping a
  `description` or `due_on` produced an update that omitted the field
  entirely, so the remote value survived and the same update replayed on every
  push. The declared set is the desired state, so absent optionals are now sent
  empty on the GitLab update path.
- **A malformed `.gwm.toml` no longer silently picks the wrong forge**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). The forge lookups
  added here swallowed config errors and fell back to host inference, dropping
  a `forge = "gitlab"` a self-hosted instance depends on. Single-repo paths now
  surface the error; a workspace row with a broken config skips detection
  instead of guessing.

- **The forge CLI runs inside the repo, not gwm's working directory**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `gh` / `glab`
  resolve the instance from their cwd when nothing pins it; gwm's cwd is the
  workspace root, not the row's repo. This is the root fix for the
  wrong-tenant hazard and covers SSH remotes, where no host can honestly be
  pinned. `$GH_HOST` is now pinned for github.com too, since an ambient
  `GH_HOST` would otherwise retarget a github.com repo.
- **The forge CLI host pin carries the port**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `https://ghe.example:8443/…`
  pinned only `ghe.example`, sending `gh` to port 443 — guaranteed wrong, and
  possibly a different instance listening there.
- **IPv6 remotes parse correctly**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `git@[::1]:group/repo.git`
  was split on the first colon, yielding host `git@[` and a nonsense path. The
  scp-form and port splits are now bracket-aware.
- **Inherited repository selectors are stripped from the forge CLI**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `$GH_REPO`,
  `$GITLAB_REPO`, `$REMOTE_ALIAS` and `$GIT_REMOTE_URL_VAR` each override which
  project the CLI acts on and are inherited from gwm's own environment, so an
  exported one silently retargeted every call. Host variables are deliberately
  left alone — gwm does not always know the host.
- **Both forges' alternate SSH endpoints map back to the API host**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)).
  `ssh://git@ssh.github.com:443/…` and `altssh.gitlab.com` exist for networks
  that block port 22; the API and web UI stay on the canonical domain, so
  pinning the SSH host broke every call and produced dead links.
- **PR / MR auto-detection ignores a fork sharing the branch name**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `--head` /
  `--source-branch` match the branch *name* only, so a fork's PR could win and
  be persisted as this branch's detected PR. Filtered on GitHub's
  `isCrossRepository` and on GitLab's `source_project_id` vs `project_id`.
- **An SSH origin lets `glab` resolve the project itself**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). No host can honestly
  be pinned from an SSH remote, but passing `--repo <slug>` anyway made glab
  resolve it against its default host — defeating the working directory gwm
  sets. The flag is dropped in that case, and the REST paths use `glab api`'s
  `:fullpath` placeholder.
- **Issue / MR links come from the forge when the local URL would be a guess**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). `https://<ssh-host>/…`
  is wrong whenever the SSH hostname is not the web one, or the web UI runs on
  HTTP or a non-standard port. `gwm open` now asks for the server's `web_url`;
  the TUI reuses an already-cached status so the render thread issues no
  request, and both fall back to the constructed URL offline.
- **Hook `{owner}` / `{repo}` placeholders split a nested namespace correctly**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). A GitLab slug can be
  `group/sub/proj`; splitting on the first separator gave `owner=group`,
  `repo=sub/proj`. The namespace is everything before the last one.
- **The GitHub host is pinned whenever the slug is known**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)), including github.com
  and including SSH origins. `gh` takes no hostname from `--repo owner/repo`,
  bakes the slug into `gh api repos/<slug>`, and does not fall back to the
  working directory the way `glab` does — so an ambient `GH_HOST` retargeted
  every call, and pinning nothing on an enterprise host meant silently
  querying github.com.
- **`gwm doctor` honours `$GWM_GH` / `$GWM_GLAB`**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). The forge-CLI probe
  looked for the bare `gh` / `glab` name, warning about a working setup that
  points at an alternative binary.
- **Ancestor group labels stay out of the project label diff**
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)). GitLab returns them
  from the project endpoint by default, so `gwm labels push --prune` proposed
  deleting labels the project does not own.

### Docs

- New [GitLab (multi-forge)](docs/5.integrations/5.gitlab.md) integration
  page covering forge selection, nested groups, the pipeline-to-CI-state
  mapping, and the deferred TUI terminology sweep
  ([#419](https://github.com/kbrdn1/gwm-cli/issues/419)).

## Past releases

In reverse chronological order:

- [`1.4.0`](changelogs/1.4.0.md) — 2026-07-25
- [`1.3.0`](changelogs/1.3.0.md) — 2026-07-24
- [`1.2.0`](changelogs/1.2.0.md) — 2026-07-21
- [`1.1.1`](changelogs/1.1.1.md) — 2026-07-16
- [`1.1.0`](changelogs/1.1.0.md) — 2026-07-15
- [`1.0.3`](changelogs/1.0.3.md) — 2026-07-09
- [`1.0.2`](changelogs/1.0.2.md) — 2026-07-06
- [`1.0.1`](changelogs/1.0.1.md) — 2026-07-01
- [`1.0.0`](changelogs/1.0.0.md) — 2026-06-26
- [`0.9.0`](changelogs/0.9.0.md) — 2026-06-07
- [`0.8.0`](changelogs/0.8.0.md) — 2026-06-01
- [`0.7.0`](changelogs/0.7.0.md) — 2026-05-23
- [`0.6.0`](changelogs/0.6.0.md) — 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md) — 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md) — 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md) — 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md) — 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md) — 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.10.0-rc.4`](changelogs/pre-releases/0.10.0-rc.4.md) — 2026-06-17
- [`0.10.0-rc.3`](changelogs/pre-releases/0.10.0-rc.3.md) — 2026-06-17
- [`0.10.0-rc.2`](changelogs/pre-releases/0.10.0-rc.2.md) — 2026-06-16
- [`0.10.0-rc.1`](changelogs/pre-releases/0.10.0-rc.1.md) — 2026-06-10
- [`0.9.0-rc.3`](changelogs/pre-releases/0.9.0-rc.3.md) — 2026-06-07
- [`0.9.0-rc.2`](changelogs/pre-releases/0.9.0-rc.2.md) — 2026-06-06
- [`0.9.0-rc.1`](changelogs/pre-releases/0.9.0-rc.1.md) — 2026-06-02
- [`0.8.0-rc.5`](changelogs/pre-releases/0.8.0-rc.5.md) — 2026-06-01
- [`0.8.0-rc.4`](changelogs/pre-releases/0.8.0-rc.4.md) — 2026-05-29
- [`0.8.0-rc.3`](changelogs/pre-releases/0.8.0-rc.3.md) — 2026-05-29
- [`0.8.0-rc.2`](changelogs/pre-releases/0.8.0-rc.2.md) — 2026-05-23
- [`0.8.0-rc.1`](changelogs/pre-releases/0.8.0-rc.1.md) — 2026-05-23
- [`0.7.0-rc.3`](changelogs/pre-releases/0.7.0-rc.3.md) — 2026-05-23
- [`0.7.0-rc.2`](changelogs/pre-releases/0.7.0-rc.2.md) — 2026-05-23
- [`0.7.0-rc.1`](changelogs/pre-releases/0.7.0-rc.1.md) — 2026-05-22
- [`0.6.0-rc.1`](changelogs/pre-releases/0.6.0-rc.1.md) — 2026-05-20
- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md) — 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md) — 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md) — 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md) — 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md) — 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md) — 2026-05-18
