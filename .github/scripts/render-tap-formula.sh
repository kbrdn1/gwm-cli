#!/bin/sh
# render-tap-formula.sh — substitute placeholders in the Homebrew formula
# template and emit the rendered formula on stdout.
#
# Used by .github/workflows/release.yml > homebrew-tap-update to refresh
# Formula/gwm.rb on kbrdn1/homebrew-tap after every stable release.
#
# Usage:
#   render-tap-formula.sh TAG VERSION SHA256_ARM64 SHA256_X86_64 TEMPLATE
#
#   TAG            v-prefixed git tag (e.g. v0.5.0)
#   VERSION        semver string without leading v (e.g. 0.5.0)
#   SHA256_ARM64   64-char hex sha256 of the macOS aarch64 tarball
#   SHA256_X86_64  64-char hex sha256 of the macOS x86_64 tarball
#   TEMPLATE       path to packaging/homebrew/gwm.rb.template
#
# Output: rendered formula on stdout.
# Exit 1 on usage / validation errors, with a message on stderr.

set -eu

if [ "$#" -ne 5 ]; then
  printf 'usage: %s TAG VERSION SHA256_ARM64 SHA256_X86_64 TEMPLATE\n' "$0" >&2
  printf 'render-tap-formula: missing arguments (got %d, expected 5)\n' "$#" >&2
  exit 1
fi

TAG="$1"
VERSION="$2"
SHA_ARM64="$3"
SHA_X86_64="$4"
TEMPLATE="$5"

if [ ! -f "$TEMPLATE" ]; then
  printf 'render-tap-formula: template not found: %s\n' "$TEMPLATE" >&2
  exit 1
fi

# A typo in the release pipeline would otherwise ship a formula that
# `brew install` rejects with a confusing checksum mismatch — fail loud
# here so the release notes catch it on the spot.
validate_sha() {
  name="$1"
  value="$2"
  case "$value" in
    *[!0-9a-fA-F]*)
      printf 'render-tap-formula: %s is not hex: %s\n' "$name" "$value" >&2
      exit 1
      ;;
  esac
  len=$(printf %s "$value" | wc -c | tr -d ' ')
  if [ "$len" -ne 64 ]; then
    printf 'render-tap-formula: invalid sha256 %s — must be 64 hex chars, got %d: %s\n' \
      "$name" "$len" "$value" >&2
    exit 1
  fi
}

validate_sha SHA256_ARM64 "$SHA_ARM64"
validate_sha SHA256_X86_64 "$SHA_X86_64"

# Placeholders are __FOO__ — alnum + underscores only, no sed-meta to escape.
sed \
  -e "s|__TAG__|${TAG}|g" \
  -e "s|__VERSION__|${VERSION}|g" \
  -e "s|__SHA256_ARM64__|${SHA_ARM64}|g" \
  -e "s|__SHA256_X86_64__|${SHA_X86_64}|g" \
  "$TEMPLATE"
