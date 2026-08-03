# Security Policy

## Supported versions

`gwm` follows [Semantic Versioning](https://semver.org/). Security fixes land
on the latest minor line; older lines are not backported.

| Version | Supported          |
| ------- | ------------------ |
| 1.6.x   | :white_check_mark: |
| < 1.6   | :x:                |

Every version up to and including 1.5.0 carries
[GHSA-fffq-vg6f-gxqm](https://github.com/kbrdn1/gwm-cli/security/advisories/GHSA-fffq-vg6f-gxqm)
(high): a branch name could inject a command into a lifecycle hook. There is no
backport, so upgrading to 1.6.0 is the fix.

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues,
pull requests, or discussions.**

Instead, use GitHub's private vulnerability reporting:

1. Go to the [**Security** tab](https://github.com/kbrdn1/gwm-cli/security)
   of this repository.
2. Click **Report a vulnerability** to open a private advisory
   ([direct link](https://github.com/kbrdn1/gwm-cli/security/advisories/new)).
3. Describe the issue with enough detail to reproduce it: affected version,
   platform, steps, and impact.

If you cannot use private reporting, email **onepiecekylian@gmail.com** with
the same details and `SECURITY` in the subject line.

## What to expect

This is a solo-maintained project, so responses are best-effort:

- **Acknowledgement** within 7 days of your report.
- **Assessment** of severity and affected versions, shared with you as it
  progresses.
- **Fix and disclosure** coordinated with you — a patched release and, where
  warranted, a [RustSec](https://rustsec.org/) advisory. Please give the
  maintainer a reasonable window to ship a fix before any public disclosure.

## Scope

`gwm` is a local-first CLI/TUI for managing git worktrees. The most relevant
threat surfaces are:

- The bootstrap step (file copies, command hooks, `.env` guards) — see the
  trust gate and `gwm doctor`.
- The `gwm daemon` JSON-RPC unix socket and its consumers.
- `gwm review`, which fetches untrusted PR refs into a worktree (bootstrap is
  opt-in behind `--bootstrap` for this reason).

Dependency advisories are gated in CI via `cargo audit --deny warnings`.
