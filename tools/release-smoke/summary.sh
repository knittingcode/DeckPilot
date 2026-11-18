#!/usr/bin/env bash
# Writes the recorded checks ($SMOKE_OUT/results.tsv) to the job summary
# as a table and fails if any check failed or none was recorded.
#
#   tools/release-smoke/summary.sh <title>
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-smoke/common.sh
source "$here/common.sh"

title="${1:-Release smoke}"
results="$SMOKE_OUT/results.tsv"
summary="${GITHUB_STEP_SUMMARY:-/dev/stdout}"
{
  echo "### $title"
  echo
  if [ ! -s "$results" ]; then
    echo "No checks were recorded."
  else
    echo "| Platform | Check | Result | Evidence |"
    echo "| --- | --- | --- | --- |"
    while IFS=$'\t' read -r platform step result evidence; do
      case "$result" in
        pass) shown="pass" ;;
        fail) shown="**FAIL**" ;;
        *) shown="not covered" ;;
      esac
      evidence=${evidence//|/\\|}
      echo "| $platform | $step | $shown | $evidence |"
    done < "$results"
    echo
    echo "$(grep -c $'\tpass\t' "$results") passed, $(grep -c $'\tfail\t' "$results") failed, $(grep -c $'\tnot-covered\t' "$results") not covered."
  fi
} >> "$summary"

[ -s "$results" ] || { echo "::error::no checks were recorded"; exit 1; }
if grep -q $'\tfail\t' "$results"; then
  echo "::error::$(grep -c $'\tfail\t' "$results") checks failed"
  exit 1
fi
