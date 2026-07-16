#!/bin/sh
# render-scoop-manifest.sh — substitute placeholders in the Scoop manifest
# template and emit the rendered manifest on stdout.
#
# Used by .github/workflows/release.yml > scoop-bucket-update to refresh
# bucket/gwm.json on kbrdn1/scoop-gwm after every stable release (issue #376).
#
# Usage:
#   render-scoop-manifest.sh TAG VERSION SHA256_X86_64 TEMPLATE
#
#   TAG            v-prefixed git tag (e.g. v1.1.1)
#   VERSION        semver string without leading v (e.g. 1.1.1)
#   SHA256_X86_64  64-char hex sha256 of the Windows x86_64 zip
#   TEMPLATE       path to packaging/scoop/gwm.json.template
#
# Output: rendered manifest on stdout.
# Exit 1 on usage / validation errors, with a message on stderr.
#
# Only the `__FOO__` placeholders are substituted; the Scoop autoupdate
# variables (`$version`, `$url`) are left verbatim so `scoop update` can bump
# versions on its own between our pushes.

set -eu

if [ "$#" -ne 4 ]; then
  printf 'usage: %s TAG VERSION SHA256_X86_64 TEMPLATE\n' "$0" >&2
  printf 'render-scoop-manifest: missing arguments (got %d, expected 4)\n' "$#" >&2
  exit 1
fi

TAG="$1"
VERSION="$2"
SHA_X86_64="$3"
TEMPLATE="$4"

if [ ! -f "$TEMPLATE" ]; then
  printf 'render-scoop-manifest: template not found: %s\n' "$TEMPLATE" >&2
  exit 1
fi

# A typo in the release pipeline would otherwise ship a manifest that
# `scoop install` rejects with a confusing hash mismatch — fail loud here so
# the release run catches it on the spot.
validate_sha() {
  name="$1"
  value="$2"
  case "$value" in
    *[!0-9a-fA-F]*)
      printf 'render-scoop-manifest: %s is not hex: %s\n' "$name" "$value" >&2
      exit 1
      ;;
  esac
  len=$(printf %s "$value" | wc -c | tr -d ' ')
  if [ "$len" -ne 64 ]; then
    printf 'render-scoop-manifest: invalid sha256 %s — must be 64 hex chars, got %d: %s\n' \
      "$name" "$len" "$value" >&2
    exit 1
  fi
}

validate_sha SHA256_X86_64 "$SHA_X86_64"

# Placeholders are __FOO__ — alnum + underscores only, no sed-meta to escape.
sed \
  -e "s|__TAG__|${TAG}|g" \
  -e "s|__VERSION__|${VERSION}|g" \
  -e "s|__SHA256_X86_64__|${SHA_X86_64}|g" \
  "$TEMPLATE"
