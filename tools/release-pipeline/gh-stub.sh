#!/usr/bin/env bash
# A stand-in for the GitHub CLI in the release pipeline tests: answers
# `gh api` from JSON files in $STUB_DIR instead of GitHub.
#
#   rules.json                    GET repos/<repo>/rules/branches/main
#                                 (missing: the request fails, as without access)
#   check-runs-<sha>.json         GET repos/<repo>/commits/<sha>/check-runs
#                                 (missing: no check runs)
#
# Every call is appended to $STUB_DIR/calls.log. Anything else fails, so a
# script that starts to call something new is noticed.
set -euo pipefail

dir="${STUB_DIR:?}"
echo "$*" >> "$dir/calls.log"
[ "${1-}" = api ] || { echo "gh stub: unsupported command: $*" >&2; exit 2; }
shift
path="" jq_filter=""
while [ $# -gt 0 ]; do
  case "$1" in
    --jq) jq_filter="$2"; shift 2 ;;
    --paginate) shift ;;
    -*) echo "gh stub: unsupported option $1" >&2; exit 2 ;;
    *) path="$1"; shift ;;
  esac
done

answer() {
  if [ -n "$jq_filter" ]; then jq -r "$jq_filter" "$1"; else cat "$1"; fi
}

case "$path" in
  repos/*/rules/branches/main)
    [ -f "$dir/rules.json" ] || { echo "gh: Not Found (HTTP 404)" >&2; exit 1; }
    answer "$dir/rules.json"
    ;;
  repos/*/commits/*/check-runs*)
    sha="${path#repos/*/commits/}"
    sha="${sha%%/*}"
    file="$dir/check-runs-$sha.json"
    [ -f "$file" ] || { echo '{"total_count": 0, "check_runs": []}' > "$dir/empty.json"; file="$dir/empty.json"; }
    answer "$file"
    ;;
  *) echo "gh stub: unsupported path $path" >&2; exit 2 ;;
esac
