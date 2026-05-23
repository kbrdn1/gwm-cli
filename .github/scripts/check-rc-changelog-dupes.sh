#!/usr/bin/env bash
set -euo pipefail

tag="${1:-${GITHUB_REF_NAME:-}}"
if [ -z "$tag" ]; then
  echo "::error::usage: check-rc-changelog-dupes.sh <vX.Y.Z-rc.N>" >&2
  exit 2
fi

version="${tag#v}"
case "$version" in
  *-rc.[0-9]*) ;;
  *)
    echo "tag ${tag} is not an rc tag; skipping previous-rc changelog duplicate check"
    exit 0
    ;;
esac

base="${version%-rc.*}"
rc_number="${version##*-rc.}"
if [ "$rc_number" -eq 1 ]; then
  echo "tag ${tag} is rc.1; no previous rc changelog to compare"
  exit 0
fi

previous_rc="changelogs/pre-releases/${base}-rc.$((rc_number - 1)).md"
if [ ! -f "$previous_rc" ]; then
  echo "::error::Expected previous RC changelog ${previous_rc} to exist for tag ${tag}." >&2
  exit 1
fi

temp_file() {
  mktemp "${TMPDIR:-/tmp}/gwm-rc-dupes.XXXXXX"
}

unreleased_bullets="$(temp_file)"
previous_bullets="$(temp_file)"
unreleased_refs="$(temp_file)"
previous_refs="$(temp_file)"
duplicate_bullets="$(temp_file)"
duplicate_refs="$(temp_file)"
trap 'rm -f "$unreleased_bullets" "$previous_bullets" "$unreleased_refs" "$previous_refs" "$duplicate_bullets" "$duplicate_refs"' EXIT

awk '
  /^## \[Unreleased\]/ { in_unreleased = 1; next }
  in_unreleased && /^## / { in_unreleased = 0 }
  in_unreleased && /^[[:space:]]*-[[:space:]]+/ {
    sub(/^[[:space:]]*-[[:space:]]+/, "- ")
    sub(/[[:space:]]+$/, "")
    print
  }
' CHANGELOG.md | LC_ALL=C sort -u > "$unreleased_bullets"

awk '
  /^[[:space:]]*-[[:space:]]+/ {
    sub(/^[[:space:]]*-[[:space:]]+/, "- ")
    sub(/[[:space:]]+$/, "")
    print
  }
' "$previous_rc" | LC_ALL=C sort -u > "$previous_bullets"

grep -Eo '#[0-9]+' "$unreleased_bullets" | LC_ALL=C sort -u > "$unreleased_refs" || true
grep -Eo '#[0-9]+' "$previous_bullets" | LC_ALL=C sort -u > "$previous_refs" || true

comm -12 "$unreleased_bullets" "$previous_bullets" > "$duplicate_bullets"
comm -12 "$unreleased_refs" "$previous_refs" > "$duplicate_refs"

if [ -s "$duplicate_bullets" ] || [ -s "$duplicate_refs" ]; then
  echo "::error::CHANGELOG.md [Unreleased] repeats entries already present in ${previous_rc}." >&2
  if [ -s "$duplicate_refs" ]; then
    echo "Duplicate issue/PR references:" >&2
    sed 's/^/  /' "$duplicate_refs" >&2
  fi
  if [ -s "$duplicate_bullets" ]; then
    echo "Duplicate changelog bullets:" >&2
    sed 's/^/  /' "$duplicate_bullets" >&2
  fi
  exit 1
fi

echo "CHANGELOG.md [Unreleased] has no duplicate bullets or issue refs from ${previous_rc}"
