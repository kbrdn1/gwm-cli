# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/) — one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **aqua install channel** — `aqua g -i kbrdn1/gwm-cli`. gwm is registered in
  the aqua **standard registry**
  ([aquaproj/aqua-registry#57117](https://github.com/aquaproj/aqua-registry/pull/57117)),
  so no custom registry wiring is needed. aqua pulls the prebuilt binary for
  the platform from the matching GitHub Release and verifies its `sha256`
  against the published `.sha256` sidecar; Linux, macOS and Windows are
  covered on both Intel and ARM, with Windows-on-ARM falling back to the x64
  build under emulation. Requires standard registry `v4.539.0` or newer. (#380)
- **Scoop install channel for Windows** — `scoop bucket add gwm
  https://github.com/kbrdn1/scoop-gwm; scoop install gwm`. A new
  `scoop-bucket-update` job in `release.yml` renders
  `packaging/scoop/gwm.json.template` and pushes `bucket/gwm.json` to the
  `kbrdn1/scoop-gwm` bucket on every stable release (mirroring the Homebrew
  tap; pre-releases filtered out). `scoop update gwm` picks up each new
  version once the release job pushes the refreshed manifest. (#376)
- **`.deb` packages for Debian / Ubuntu** — `x86_64` and `aarch64`, built by
  `cargo-deb` and attached to every stable release (`sudo dpkg -i
  gwm-cli_<ver>-1_amd64.deb`). The package is named `gwm-cli` to avoid
  Debian's unrelated `gwm` window-manager package; the command stays `gwm`.
  (#377)
- **`.rpm` packages for Fedora / RHEL / openSUSE** — `x86_64` and `aarch64`,
  built by `cargo-generate-rpm` and attached to every stable release (`sudo
  rpm -i gwm-cli-<ver>-1.x86_64.rpm`). (#378)
- **AUR package for Arch Linux** — `yay -S gwm-cli-bin` (or `paru`). A new
  `aur-publish` job in `release.yml` renders `packaging/aur/PKGBUILD.template`
  and pushes the `gwm-cli-bin` prebuilt-binary package to the AUR on every
  stable release (via the SHA-pinned `KSXGitHub/github-actions-deploy-aur`
  action, which regenerates `.SRCINFO` and builds the package with `makepkg`
  against the real binary). Installs `gwm` + license + bash/zsh/fish
  completions; `depends` on `git` (gwm shells out to it); `provides`/`conflicts`
  `gwm-cli` and `gwm`. Pre-releases filtered out. (#379)
- **winget publishing automation** — a new `winget-publish` job in
  `release.yml` opens a manifest PR to `microsoft/winget-pkgs` (`kbrdn1.gwm`)
  for each stable release, building the manifest from the Windows `.zip` with a
  pinned, checksum-verified `komac` (run directly rather than via an action that
  installs tooling from mutable refs at runtime with the token in scope). The
  initial manifest is submitted manually; the job takes over from the next
  version. `winget install kbrdn1.gwm` becomes available once Microsoft merges
  the manifest. Pre-releases filtered out. (#381)

### Fixed

- **`.deb` / `.rpm` packages now depend on `git`** — gwm shells out to the
  `git` binary (sync, worktree rename, clean, TUI previews) beyond the vendored
  libgit2, so a minimal Debian/Fedora install without git would break those
  features. `git` is now declared in `Depends` / `Requires`. (#388)
- **The Nix flake reports the right version** — it advertised `0.3.0-rc.3`
  while the code was at `1.1.1`, so `nix profile list`, `nix flake show` and
  the store path all named a release eight versions old. The built binary was
  always correct (`gwm --version` reported `1.1.1`), so this was mislabelling
  rather than a broken build, which is why it went unnoticed. The version is
  now read out of `Cargo.toml` at eval time instead of being restated in
  `flake.nix`, making the drift impossible rather than merely fixed. (#393)

## Past releases

In reverse chronological order:

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
