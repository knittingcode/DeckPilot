#!/usr/bin/env bash
# Stage A: creates the draft release of a tag once, before the builds
# upload to it, and refuses anything that could replace a release.
#
#   tools/release-pipeline/create-draft.sh <tag>
#
# Environment: SHA (the gate's commit), PRERELEASE (true/false), NOTES_FILE
# (release notes), REPOSITORY, GH (see common.sh).
#
# - no release for the tag: a draft is created for exactly SHA;
# - a draft for the same commit (a re-run): it is reused, its assets are
#   replaced by the builds;
# - a draft for another commit: the tag was moved or reused; refused;
# - a published release: refused. A published release is never rebuilt,
#   replaced or given new assets: release a new version instead.
#
# Writes release_id and state (new or draft) to $GITHUB_OUTPUT (or stdout).
set -euo pipefail
# shellcheck source=tools/release-pipeline/common.sh
source "$(dirname "$0")/common.sh"

tag="${1-}"
require_tag "$tag"
: "${SHA:?}" "${PRERELEASE:?}" "${NOTES_FILE:?}"
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || fail "SHA must be a full commit SHA"
[ -s "$NOTES_FILE" ] || fail "the release notes are empty"

release=$(find_release "$tag")
if [ -z "$release" ]; then
  release=$("$gh" api -X POST "repos/$repository/releases" \
    -f tag_name="$tag" -f target_commitish="$SHA" -f name="DeckPilot $tag" \
    -F draft=true -F prerelease="$PRERELEASE" -F body=@"$NOTES_FILE") \
    || fail "the draft release of $tag could not be created"
  state=new
  echo "created the draft release of $tag for $SHA"
else
  draft=$(jq -r .draft <<< "$release")
  commit=$(jq -r .target_commitish <<< "$release")
  [ "$draft" = true ] \
    || fail "$tag is already published; a published release is never rebuilt or replaced. Release a new version instead"
  [ "$commit" = "$SHA" ] \
    || fail "the draft of $tag was built from $commit, but the tag now points to $SHA: the tag was moved or reused. Discard the draft (tools/release-pipeline/discard-draft.sh $tag) and push a new tag"
  [ "$(jq -r .prerelease <<< "$release")" = "$PRERELEASE" ] \
    || fail "the draft of $tag has the wrong pre-release flag; discard it and run again"
  state=draft
  echo "reusing the draft release of $tag (same commit $SHA); its assets are replaced"
fi
id=$(jq -r .id <<< "$release")
[[ "$id" =~ ^[0-9]+$ ]] || fail "no release ID for $tag"
{
  echo "release_id=$id"
  echo "state=$state"
} >> "${GITHUB_OUTPUT:-/dev/stdout}"
