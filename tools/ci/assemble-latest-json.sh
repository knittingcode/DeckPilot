#!/usr/bin/env bash
# Assembles the release's latest.json once, from the fragments of all build
# jobs (updater-fragment.sh), so that no entry can be lost to concurrent
# updates of the draft release.
#
#   tools/ci/assemble-latest-json.sh <fragments dir> <output.json>
#
# Environment:
#   REPOSITORY  owner/name, for the download URLs
#   TAG         the release tag, e.g. v0.6.0 (pre-release tags contain "-")
#   VERSION     the version announced, e.g. 0.6.0
#   NOTES_FILE  file with the release notes
#   PUB_DATE    optional; RFC 3339, default now
#   ASSETS_FILE optional; the draft release's asset names, one per line:
#               every package and its .sig must be among them
#
# Fails if two fragments name the same target key, if a required platform
# or package type is missing, or if an asset is not in the release.
set -euo pipefail

fragments="${1:?usage: assemble-latest-json.sh <fragments dir> <output.json>}"
output="${2:?usage: assemble-latest-json.sh <fragments dir> <output.json>}"
: "${REPOSITORY:?}" "${TAG:?}" "${VERSION:?}" "${NOTES_FILE:?}"
pub_date="${PUB_DATE:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"
fail() {
  echo "::error::$*" >&2
  exit 1
}

shopt -s nullglob
files=("$fragments"/*.json)
[ "${#files[@]}" -gt 0 ] || fail "no updater fragments in $fragments"

# Every key exactly once.
duplicates=$(jq -r '.platforms | keys[]' "${files[@]}" | sort | uniq -d)
[ -z "$duplicates" ] || fail "several builds provide: $(echo "$duplicates" | tr "\n" " " | sed "s/ $//")"

# Every platform and package type that installations update from. Final
# releases also build the .rpm; test releases (pre-release tags) do not.
required="darwin-aarch64 darwin-aarch64-app darwin-x86_64 darwin-x86_64-app
  linux-x86_64 linux-x86_64-appimage linux-x86_64-deb
  windows-x86_64 windows-x86_64-msi windows-x86_64-nsis"
case "$TAG" in *-*) ;; *) required="$required linux-x86_64-rpm" ;; esac
listed=$(jq -r '.platforms | keys[]' "${files[@]}")
missing=""
for key in $required; do
  echo "$listed" | grep -q -x -F "$key" || missing="$missing $key"
done
[ -z "$missing" ] || fail "no update for:$missing"

if [ -n "${ASSETS_FILE:-}" ]; then
  for asset in $(jq -r '.platforms[].asset' "${files[@]}" | sort -u); do
    for name in "$asset" "$asset.sig"; do
      grep -q -x -F "$name" "$ASSETS_FILE" || fail "the release has no asset $name"
    done
  done
fi

jq -s \
  --arg version "$VERSION" \
  --rawfile notes "$NOTES_FILE" \
  --arg pub_date "$pub_date" \
  --arg base "https://github.com/$REPOSITORY/releases/download/$TAG/" \
  '{
    version: $version,
    notes: ($notes | sub("\\s+$"; "")),
    pub_date: $pub_date,
    platforms: (map(.platforms) | add
      | with_entries(.value = {signature: .value.signature, url: ($base + .value.asset)}))
  }' "${files[@]}" > "$output"
echo "latest.json for $VERSION: $(jq -r '.platforms | keys | join(" ")' "$output")"
