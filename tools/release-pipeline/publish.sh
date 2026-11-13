#!/usr/bin/env bash
# Stage B: publishes the draft release of a tag, only after everything has
# been checked again on the draft's actual assets, then verifies the
# published release from its public URLs (verify-published.sh).
#
#   tools/release-pipeline/publish.sh <tag>
#
# Environment:
#   SHA, VERSION, CHANNEL, MANIFEST, PRERELEASE  outputs of the tag gate
#   WORK            scratch directory (default: a temporary directory)
#   VERIFY_RELEASE  signature verification of a release directory
#                   (default: cargo run --locked -q -p deckpilot-dev -- verify-release)
#   REPOSITORY, GH  see common.sh; CURL see verify-published.sh
#
# Before publishing (fail closed, the release stays a draft):
#   - exactly one release exists for the tag and the tag still points to SHA;
#   - every required asset, nothing of the other channel (check-assets.sh);
#   - SHA256SUMS covers exactly the assets and every checksum matches;
#   - release-meta.json names this tag, version, channel, manifest and SHA;
#   - every updater signature verifies (deckpilot-dev verify-release);
#   - the release notes are not empty.
# Then one update publishes it: a pre-release for alpha, beta and rc; a
# stable release, marked latest only if it is newer than every published
# stable release.
#
# Idempotent: for an already published release of the same commit nothing
# is changed and only the verification runs again, so a failed run can be
# re-run.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-pipeline/common.sh
source "$here/common.sh"

tag="${1-}"
require_tag "$tag"
: "${SHA:?}" "${VERSION:?}" "${CHANNEL:?}" "${MANIFEST:?}" "${PRERELEASE:?}"
[ "$tag" = "v$VERSION" ] || fail "$tag is not the tag of version $VERSION"
work="${WORK:-$(mktemp -d "${TMPDIR:-/tmp}/publish.XXXXXX")}"
read -r -a verify_release <<< "${VERIFY_RELEASE:-cargo run --locked -q -p deckpilot-dev -- verify-release}"
sha256() { if command -v sha256sum > /dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }

release=$(find_release "$tag")
[ -n "$release" ] || fail "there is no release for $tag; stage A did not create one"
id=$(jq -r .id <<< "$release")
tag_commit=$("$gh" api "repos/$repository/commits/$tag" --jq .sha) || fail "the tag $tag cannot be resolved"
[ "$tag_commit" = "$SHA" ] || fail "the tag $tag now points to $tag_commit, not to $SHA that was built and checked"

if [ "$(jq -r .draft <<< "$release")" = true ]; then
  [ "$(jq -r .target_commitish <<< "$release")" = "$SHA" ] || fail "the draft of $tag was not built from $SHA"
  [ -n "$(jq -r '.body // "" | gsub("\\s"; "")' <<< "$release")" ] || fail "the draft of $tag has no release notes"

  assets="$work/assets"
  rm -rf "$assets" && mkdir -p "$assets"
  "$gh" release download "$tag" --repo "$repository" --dir "$assets" > /dev/null \
    || fail "the assets of the draft could not be downloaded"
  (cd "$assets" && ls -1) > "$work/assets.txt"
  VERSION="$VERSION" PRERELEASE="$PRERELEASE" MANIFEST="$MANIFEST" \
    "$here/check-assets.sh" "$work/assets.txt" --final

  # SHA256SUMS: exactly the other assets, each matching.
  awk '{ sub(/^\*/, "", $2); print $2 }' "$assets/SHA256SUMS" | sort > "$work/summed.txt"
  grep -v -x -F SHA256SUMS "$work/assets.txt" | sort > "$work/present.txt"
  diff -u "$work/summed.txt" "$work/present.txt" > "$work/sums.diff" \
    || fail "SHA256SUMS does not cover exactly the assets of the release: $(grep '^[-+][^-+]' "$work/sums.diff" | tr '\n' ' ')"
  (cd "$assets" && sha256 -c SHA256SUMS > "$work/sums.log" 2>&1) \
    || fail "an asset does not match SHA256SUMS: $(grep -v ': OK$' "$work/sums.log" | tr '\n' ' ')"

  meta="$assets/release-meta.json"
  for field in tag:"$tag" version:"$VERSION" channel:"$CHANNEL" manifest:"$MANIFEST" commit:"$SHA"; do
    key="${field%%:*}" expected="${field#*:}"
    [ "$(jq -r --arg k "$key" '.[$k]' "$meta")" = "$expected" ] \
      || fail "release-meta.json has $key $(jq -r --arg k "$key" '.[$k]' "$meta"), expected $expected"
  done

  "${verify_release[@]}" "$assets" > "$work/verify.log" 2>&1 \
    || fail "the updater signatures do not verify: $(tail -n 3 "$work/verify.log" | tr '\n' ' ')"

  # Latest: a stable release newer than every published stable release.
  latest=false
  if [ "$PRERELEASE" = false ]; then
    newest=$( (published_versions | grep -v -- - || true; echo "$VERSION") | sort -V | tail -n 1)
    if [ "$newest" = "$VERSION" ] && ! published_versions | grep -q -x -F "$VERSION"; then
      latest=true
    fi
  fi
  "$gh" api -X PATCH "repos/$repository/releases/$id" \
    -F draft=false -F prerelease="$PRERELEASE" -f make_latest="$latest" > /dev/null \
    || fail "publishing $tag failed; it is still a draft. Re-run the publish job"
  echo "published $tag ($CHANNEL$([ "$latest" = true ] && echo ", latest"))"
else
  [ "$(jq -r .target_commitish <<< "$release")" = "$SHA" ] || fail "the published $tag was not built from $SHA"
  echo "$tag is already published; verifying only"
fi

"$here/verify-published.sh" "$tag"
