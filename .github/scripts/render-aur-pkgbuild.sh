#!/bin/sh
# render-aur-pkgbuild.sh — substitute placeholders in the AUR PKGBUILD template
# and emit the rendered PKGBUILD on stdout.
#
# `gwm-cli-bin` is maintained on the AUR by a third party (#430), so nothing in
# CI consumes this: it is run by hand after a stable release to produce the
# PKGBUILD handed to the packager. See CONTRIBUTING.md > Releases > AUR.
#
# Usage:
#   render-aur-pkgbuild.sh VERSION SHA256_X86_64 SHA256_ARM64 TEMPLATE
#
#   VERSION        semver string without leading v (e.g. 1.1.1)
#   SHA256_X86_64  64-char hex sha256 of the linux x86_64 tarball
#   SHA256_ARM64   64-char hex sha256 of the linux aarch64 tarball
#   TEMPLATE       path to packaging/aur/PKGBUILD.template
#
# The tag is not passed: the PKGBUILD derives its download URL from `v$pkgver`,
# so VERSION is the single source of truth for both the version and the tag.
#
# Output: rendered PKGBUILD on stdout.
# Exit 1 on usage / validation errors, with a message on stderr.

set -eu

if [ "$#" -ne 4 ]; then
  printf 'usage: %s VERSION SHA256_X86_64 SHA256_ARM64 TEMPLATE\n' "$0" >&2
  printf 'render-aur-pkgbuild: missing arguments (got %d, expected 4)\n' "$#" >&2
  exit 1
fi

VERSION="$1"
SHA_X86_64="$2"
SHA_ARM64="$3"
TEMPLATE="$4"

if [ ! -f "$TEMPLATE" ]; then
  printf 'render-aur-pkgbuild: template not found: %s\n' "$TEMPLATE" >&2
  exit 1
fi

# A typo in the release pipeline would otherwise ship a PKGBUILD that makepkg
# rejects with a confusing checksum mismatch — fail loud here so the release
# catches it on the spot.
validate_sha() {
  name="$1"
  value="$2"
  case "$value" in
    *[!0-9a-fA-F]*)
      printf 'render-aur-pkgbuild: %s is not hex: %s\n' "$name" "$value" >&2
      exit 1
      ;;
  esac
  len=$(printf %s "$value" | wc -c | tr -d ' ')
  if [ "$len" -ne 64 ]; then
    printf 'render-aur-pkgbuild: invalid sha256 %s — must be 64 hex chars, got %d: %s\n' \
      "$name" "$len" "$value" >&2
    exit 1
  fi
}

validate_sha SHA256_X86_64 "$SHA_X86_64"
validate_sha SHA256_ARM64 "$SHA_ARM64"

# Placeholders are __FOO__ — alnum + underscores only, no sed-meta to escape.
sed \
  -e "s|__VERSION__|${VERSION}|g" \
  -e "s|__SHA256_X86_64__|${SHA_X86_64}|g" \
  -e "s|__SHA256_ARM64__|${SHA_ARM64}|g" \
  "$TEMPLATE"
