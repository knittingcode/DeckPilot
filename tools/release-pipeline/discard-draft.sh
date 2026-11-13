#!/usr/bin/env bash
# Recovery: deletes the DRAFT release of a tag so that the release workflow
# can build it again from scratch. Never touches a published release, and
# keeps the tag (delete or move a tag only by hand, and only if it was never
# published).
#
#   tools/release-pipeline/discard-draft.sh <tag>
#
# Environment: REPOSITORY, GH (see common.sh); a token that can see drafts.
set -euo pipefail
# shellcheck source=tools/release-pipeline/common.sh
source "$(dirname "$0")/common.sh"

tag="${1-}"
require_tag "$tag"
release=$(find_release "$tag")
[ -n "$release" ] || fail "there is no release for $tag"
[ "$(jq -r .draft <<< "$release")" = true ] \
  || fail "$tag is published; a published release is never deleted by this tool. Fix forward with a new version"
id=$(jq -r .id <<< "$release")
"$gh" api -X DELETE "repos/$repository/releases/$id" > /dev/null || fail "deleting the draft of $tag failed"
echo "deleted the draft release of $tag (release $id); the tag is kept"
