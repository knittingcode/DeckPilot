#!/usr/bin/env bash
# Stage B, after publication: what users download is what was checked.
#
#   tools/release-pipeline/verify-published.sh <tag>
#
# Environment: VERSION, CHANNEL, MANIFEST, PRERELEASE (the tag gate's
# outputs), WORK, CURL (default curl; the tests inject a stub), REPOSITORY,
# GH (see common.sh).
#
# - the release is published, with the right pre-release flag;
# - every asset listed in SHA256SUMS downloads from its public URL and
#   matches its checksum;
# - the channel's manifest URL announces this version: for a stable release
#   that became latest, the stable channel's URL
#   (releases/latest/download/latest.json); otherwise the release's own;
# - every package URL in the manifest is one of the verified assets;
# - the stable channel's manifest never announces a pre-release.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-pipeline/common.sh
source "$here/common.sh"

tag="${1-}"
require_tag "$tag"
: "${VERSION:?}" "${CHANNEL:?}" "${MANIFEST:?}" "${PRERELEASE:?}"
work="${WORK:-$(mktemp -d "${TMPDIR:-/tmp}/verify-published.XXXXXX")}/published"
curl="${CURL:-curl}"
sha256() { if command -v sha256sum > /dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
fetch() { "$curl" -fsSL --retry 3 --retry-delay 5 -o "$2" "$1"; }
base="https://github.com/$repository/releases"

release=$(find_release "$tag")
[ -n "$release" ] || fail "there is no release for $tag"
[ "$(jq -r .draft <<< "$release")" = false ] || fail "$tag is still a draft"
[ "$(jq -r .prerelease <<< "$release")" = "$PRERELEASE" ] \
  || fail "$tag is published with prerelease=$(jq -r .prerelease <<< "$release"), expected $PRERELEASE"

rm -rf "$work" && mkdir -p "$work/assets"
fetch "$base/download/$tag/SHA256SUMS" "$work/SHA256SUMS" || fail "SHA256SUMS of $tag cannot be downloaded"
count=0
while read -r _ name; do
  name="${name#\*}"
  fetch "$base/download/$tag/$name" "$work/assets/$name" || fail "$name of $tag cannot be downloaded"
  count=$((count + 1))
done < "$work/SHA256SUMS"
(cd "$work/assets" && sha256 -c ../SHA256SUMS > ../sums.log 2>&1) \
  || fail "a published asset does not match SHA256SUMS: $(grep -v ': OK$' "$work/sums.log" | tr '\n' ' ')"
echo "all $count published assets of $tag match SHA256SUMS"

latest_tag=$("$gh" api "repos/$repository/releases/latest" --jq .tag_name 2> /dev/null || true)
if [ "$PRERELEASE" = false ] && [ "$latest_tag" = "$tag" ]; then
  manifest_url="$base/latest/download/latest.json"
else
  manifest_url="$base/download/$tag/$MANIFEST"
fi
fetch "$manifest_url" "$work/manifest.json" || fail "the manifest $manifest_url cannot be downloaded"
announced=$(jq -r .version "$work/manifest.json")
[ "$announced" = "$VERSION" ] || fail "$manifest_url announces $announced, expected $VERSION"
while read -r url; do
  name="${url##*/}"
  if [ "$url" != "$base/download/$tag/$name" ] || [ ! -f "$work/assets/$name" ]; then
    fail "the manifest points to $url, which is not a verified asset of $tag"
  fi
done < <(jq -r '.platforms[].url' "$work/manifest.json")
echo "$manifest_url announces $VERSION; every package URL is a verified asset"

# Channel isolation, as installations see it.
if fetch "$base/latest/download/latest.json" "$work/stable.json" 2> /dev/null; then
  stable=$(jq -r .version "$work/stable.json")
  [[ "$stable" != *-* ]] || fail "the stable channel announces the pre-release $stable"
  echo "the stable channel announces $stable"
else
  echo "the stable channel has no release yet"
fi
