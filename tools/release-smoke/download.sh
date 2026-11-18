#!/usr/bin/env bash
# Downloads release assets of <tag> and checks each against the release's
# SHA256SUMS:
#
#   tools/release-smoke/download.sh <tag> <dir> <glob>...
#
# Every downloaded file must be listed in SHA256SUMS and match it. GH_TOKEN
# is the workflow's read-only token.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-smoke/common.sh
source "$here/common.sh"

tag="${1:?usage: download.sh <tag> <dir> <glob>...}"
dir="${2:?usage: download.sh <tag> <dir> <glob>...}"
shift 2
require_tag "$tag"
mkdir -p "$dir"
step="download $tag"

args=(-p SHA256SUMS)
for glob in "$@"; do args+=(-p "$glob"); done
if ! out=$(gh release download "$tag" -R "$REPOSITORY" -D "$dir" --clobber "${args[@]}" 2>&1); then
  record "$step" error "gh release download failed: $out"
  exit 0
fi

checked=0 bad=""
while IFS= read -r -d '' file; do
  name=$(basename "$file")
  [ "$name" = SHA256SUMS ] && continue
  want=$(awk -v n="$name" '{ f = $2; sub(/^\*/, "", f); if (f == n) print $1 }' "$dir/SHA256SUMS" | tr -d '\r')
  if [ -z "$want" ]; then bad="$bad $name(not in SHA256SUMS)"; continue; fi
  got=$(sha256_of "$file")
  if [ "$got" = "$want" ]; then checked=$((checked + 1)); else bad="$bad $name(sha256 $got != $want)"; fi
done < <(find "$dir" -maxdepth 1 -type f -print0)

names=$(cd "$dir" && find . -maxdepth 1 -type f ! -name SHA256SUMS | sed 's#^\./##' | LC_ALL=C sort | tr '\n' ' ')
if [ -n "$bad" ]; then
  record "$step + SHA256SUMS" error "$bad"
elif [ "$checked" -eq 0 ]; then
  record "$step + SHA256SUMS" error "nothing matched $*"
else
  record "$step + SHA256SUMS" pass "$checked files match SHA256SUMS: $names"
fi
