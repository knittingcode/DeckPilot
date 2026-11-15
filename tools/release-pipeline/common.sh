#!/usr/bin/env bash
# Shared by the release pipeline scripts (sourced, not run).
#
# Environment:
#   REPOSITORY  owner/name (default $GITHUB_REPOSITORY)
#   GH          the GitHub CLI (default gh); the tests inject gh-stub.sh
#
# REPOSITORY only addresses the repository (API calls, gh --repo). The
# URLs that installations check are pinned below instead.

TAG_PATTERN='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$'
repository="${REPOSITORY:-${GITHUB_REPOSITORY:-}}"
gh="${GH:-gh}"

# The download prefix of every package URL in an update manifest, and the
# public release URLs verified after publication. Pinned, never derived from
# REPOSITORY or $GITHUB_REPOSITORY: the repository is now named
# knittingcode/deckpilot, but installed builds (v0.6.0-alpha.3 and older)
# compare every manifest URL case-sensitively with exactly this prefix
# (deckpilot_update::DOWNLOAD_PREFIX) and would reject an update whose
# URLs carry the new spelling. GitHub serves both spellings. Must equal
# deckpilot_update::DOWNLOAD_PREFIX byte for byte (tested by
# crates/deckpilot-update/tests/pipeline_prefix.rs and test-publication.sh).
DOWNLOAD_PREFIX="https://github.com/knittingcode/DeckPilot/releases/download/"
# shellcheck disable=SC2034 # used by the scripts that source this file
RELEASES_URL="${DOWNLOAD_PREFIX%/download/}"

fail() {
  echo "::error::$*" >&2
  exit 1
}

# require_tag <tag>: only release tags reach the API calls below.
require_tag() {
  [[ "$1" =~ $TAG_PATTERN ]] || fail "'$1' is not a release tag"
  [ -n "$repository" ] || fail "REPOSITORY (owner/name) is not set"
}

# find_release <tag>: the release of <tag> as JSON (drafts included, which
# needs a token with write access), or nothing. Fails if there are several.
find_release() {
  local found count
  found=$("$gh" api --paginate "repos/$repository/releases?per_page=100" \
    --jq ".[] | select(.tag_name == \"$1\")") || fail "the releases of $repository could not be read"
  count=$(printf '%s' "$found" | jq -s 'length')
  [ "$count" -le 1 ] || fail "$count releases exist for $1; discard the extra drafts first (docs/releasing.md)"
  printf '%s' "$found"
}

# published_versions: the versions of all published (not draft) releases
# with a release tag, one per line.
published_versions() {
  "$gh" api --paginate "repos/$repository/releases?per_page=100" \
    --jq '.[] | select(.draft | not) | .tag_name' \
    | grep -E "$TAG_PATTERN" | sed 's/^v//' || true
}
