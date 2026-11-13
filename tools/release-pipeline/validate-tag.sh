#!/usr/bin/env bash
# The release gate: decides whether a pushed tag may build a release.
# Every job of the release workflow depends on it (docs/releasing.md).
#
#   tools/release-pipeline/validate-tag.sh <tag>
#
# Run in a checkout of the tag with origin/main fetched. Changes nothing:
#
#   1. the tag is exactly vX.Y.Z, vX.Y.Z-alpha.N, vX.Y.Z-beta.N or
#      vX.Y.Z-rc.N (TAG_PATTERN below; no normalization);
#   2. the commit it points to (annotated tags dereferenced) is the checked
#      out commit, the tree is unmodified, and the commit is reachable from
#      the protected main branch;
#   3. every declared version matches the tag (deckpilot-dev check-versions);
#   4. every required check passed on exactly this commit: for each check
#      name, the latest check run of this SHA must have concluded "success";
#   5. alpha, beta and rc are pre-releases, a plain version is stable.
#
# Writes version, channel, sha, prerelease (true/false), msi_version (the
# numeric Windows Installer version) and manifest (latest.json or
# prerelease.json) to $GITHUB_OUTPUT (or stdout). Exits non-zero, with an ::error::, on the first failure.
#
# Environment:
#   REPOSITORY     owner/name (default $GITHUB_REPOSITORY)
#   MAIN_REF       the protected branch's ref (default refs/remotes/origin/main)
#   GH             the GitHub CLI (default gh); the tests inject a stub
#   DECKPILOT_DEV  how to run deckpilot-dev (default: cargo run --locked -q -p deckpilot-dev --)
#   REQUIRED_CHECKS_FILE  pinned required checks (default: required-checks.txt here)
#   RELEASE_REF    the ref the workflow runs on (github.ref); if set, it must be
#                  refs/tags/<tag>
set -euo pipefail

TAG_PATTERN='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$'

here="$(cd "$(dirname "$0")" && pwd)"
tag="${1-}"
repository="${REPOSITORY:-${GITHUB_REPOSITORY:-}}"
main_ref="${MAIN_REF:-refs/remotes/origin/main}"
gh="${GH:-gh}"
required_file="${REQUIRED_CHECKS_FILE:-$here/required-checks.txt}"
read -r -a deckpilot_dev <<< "${DECKPILOT_DEV:-cargo run --locked -q -p deckpilot-dev --}"

fail() {
  echo "::error::$*" >&2
  exit 1
}

# 1. The tag itself.
[[ "$tag" =~ $TAG_PATTERN ]] \
  || fail "'$tag' is not a release tag: use vX.Y.Z, vX.Y.Z-alpha.N, vX.Y.Z-beta.N or vX.Y.Z-rc.N (no leading zeros, N >= 1)"
# A run started by hand must run on the tag itself, not on a branch with
# the tag as a parameter.
if [ -n "${RELEASE_REF:-}" ] && [ "$RELEASE_REF" != "refs/tags/$tag" ]; then
  fail "the workflow runs on $RELEASE_REF; run it on the tag: gh workflow run release.yml --ref $tag"
fi
version="${tag#v}"
if [[ "$tag" == *-* ]]; then
  channel=prerelease prerelease=true
else
  channel=stable prerelease=false
fi
[ -n "$repository" ] || fail "REPOSITORY (owner/name) is not set"

# 2. The commit: the tag's target, checked out unmodified, on main.
sha=$(git rev-parse --verify --quiet "refs/tags/$tag^{commit}") \
  || fail "tag $tag does not exist in this checkout"
head=$(git rev-parse HEAD)
[ "$head" = "$sha" ] || fail "the checkout is at $head, not at the tagged commit $sha"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "the checkout of $tag is modified"
git rev-parse --verify --quiet "$main_ref^{commit}" > /dev/null \
  || fail "$main_ref is not available; fetch the main branch first"
git merge-base --is-ancestor "$sha" "$main_ref" \
  || fail "the commit $sha of $tag is not on the protected main branch ($main_ref); tag a commit of main"
echo "tag $tag points to $sha, which is on $main_ref"

# 3. Versions, read only.
"${deckpilot_dev[@]}" check-versions --tag "$tag" --root "$(git rev-parse --show-toplevel)" \
  || fail "the versions of the sources do not match $tag"
# The Rust classification must agree, and the version must fit an MSI
# (the Windows installer's numeric product version).
classified=$("${deckpilot_dev[@]}" release-tag "$tag") || fail "$tag cannot be released"
grep -q -x "channel=$channel" <<< "$classified" || fail "deckpilot-dev classifies $tag differently: $classified"
msi_version=$(sed -n 's/^msi_version=//p' <<< "$classified")
manifest=$(sed -n 's/^manifest=//p' <<< "$classified")
[ -n "$msi_version" ] || fail "no MSI version for $tag"

# 4. Required checks of exactly this commit: the pinned list, plus whatever
# the branch rules of main require (read only; if they cannot be read, the
# pinned list alone applies).
required=$(grep -v -e '^#' -e '^[[:space:]]*$' "$required_file")
if ruleset=$("$gh" api "repos/$repository/rules/branches/main" \
  --jq '.[] | select(.type == "required_status_checks") | .parameters.required_status_checks[].context' 2> /dev/null); then
  required=$(printf '%s\n%s\n' "$required" "$ruleset")
else
  echo "::warning::the branch rules of main could not be read; using the pinned required checks"
fi
required=$(printf '%s\n' "$required" | grep -v '^$' | sort -u)

# name, status, conclusion, head SHA and ID of every check run of the commit.
runs=$("$gh" api --paginate "repos/$repository/commits/$sha/check-runs?per_page=100" \
  --jq '.check_runs[] | [.name, .status, (.conclusion // "none"), .head_sha, (.id | tostring)] | @tsv') \
  || fail "the check runs of $sha could not be read"

problems=""
while IFS= read -r name; do
  # The latest run (highest ID) of this name for this very SHA.
  latest=$(printf '%s\n' "$runs" | awk -F '\t' -v name="$name" -v sha="$sha" \
    '$1 == name && $4 == sha && ($5 + 0) > best { best = $5 + 0; line = $0 } END { print line }')
  if [ -z "$latest" ]; then
    problems="$problems\n  $name: no check run for $sha"
    continue
  fi
  status=$(printf '%s' "$latest" | cut -f 2)
  conclusion=$(printf '%s' "$latest" | cut -f 3)
  if [ "$status" != completed ]; then
    problems="$problems\n  $name: $status"
  elif [ "$conclusion" != success ]; then
    problems="$problems\n  $name: $conclusion"
  else
    echo "required check passed: $name"
  fi
done <<< "$required"
[ -z "$problems" ] || fail "required checks have not passed on $sha:$(printf '%b' "$problems" | tr '\n' ';')"

# 6. Outputs.
out="${GITHUB_OUTPUT:-/dev/stdout}"
{
  echo "version=$version"
  echo "channel=$channel"
  echo "sha=$sha"
  echo "prerelease=$prerelease"
  echo "msi_version=$msi_version"
  echo "manifest=$manifest"
} >> "$out"
echo "$tag: version $version, $channel channel, commit $sha"
