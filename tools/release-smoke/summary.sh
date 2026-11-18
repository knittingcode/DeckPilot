#!/usr/bin/env bash
# Writes the recorded checks to the job summary and decides the job's
# outcome:
#
#   tools/release-smoke/summary.sh <title> [<results.tsv>...]
#
# Default input: $SMOKE_OUT/results.tsv. The product checks that failed
# (defects of the release) are listed first, then the test errors, then
# the whole table.
#
# Outcome: no checks recorded, or any test error (result "error": a
# download, a checksum mismatch, a missing tool), fails the job. A failed
# product check fails it only with SMOKE_STRICT=true (manual runs, which
# validate a release); otherwise (pull requests, which test the workflow
# against an already published release) it is a warning.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-smoke/common.sh
source "$here/common.sh"

title="${1:-Release smoke}"
shift || true
[ "$#" -gt 0 ] || set -- "$SMOKE_OUT/results.tsv"
results="$SMOKE_OUT/.summary-input.tsv"
cat "$@" > "$results" 2> /dev/null || true
strict="${SMOKE_STRICT:-true}"
summary="${GITHUB_STEP_SUMMARY:-/dev/stdout}"
fails=$(awk -F '\t' '$3 == "fail"' "$results" | wc -l | tr -d ' ')
errors=$(awk -F '\t' '$3 == "error"' "$results" | wc -l | tr -d ' ')
passes=$(awk -F '\t' '$3 == "pass"' "$results" | wc -l | tr -d ' ')
uncovered=$(awk -F '\t' '$3 == "not-covered"' "$results" | wc -l | tr -d ' ')
cell() { local v="${1//|/\\|}"; printf '%s' "$v"; }
{
  echo "### $title"
  echo
  if [ ! -s "$results" ]; then
    echo "No checks were recorded."
  else
    if [ "$fails" -gt 0 ]; then
      if [ "$strict" = true ]; then
        echo "> [!CAUTION]"
        echo "> **$fails product check(s) failed: the release has defects.**"
      else
        echo "> [!WARNING]"
        echo "> **$fails product check(s) failed: defects of the published release.** Reported, not failing this pull request run (manual runs fail on them)."
      fi
      echo
      echo "| Platform | Check | Evidence |"
      echo "| --- | --- | --- |"
      awk -F '\t' '$3 == "fail"' "$results" | while IFS=$'\t' read -r platform step _ evidence; do
        echo "| $(cell "$platform") | **$(cell "$step")** | $(cell "$evidence") |"
      done
      echo
    fi
    if [ "$errors" -gt 0 ]; then
      echo "> [!CAUTION]"
      echo "> **$errors test error(s)**: the test could not run as intended."
      echo
      echo "| Platform | Check | Evidence |"
      echo "| --- | --- | --- |"
      awk -F '\t' '$3 == "error"' "$results" | while IFS=$'\t' read -r platform step _ evidence; do
        echo "| $(cell "$platform") | **$(cell "$step")** | $(cell "$evidence") |"
      done
      echo
    fi
    echo "$passes passed, $fails failed, $errors test errors, $uncovered not covered."
    echo
    echo "<details><summary>All checks</summary>"
    echo
    echo "| Platform | Check | Result | Evidence |"
    echo "| --- | --- | --- | --- |"
    while IFS=$'\t' read -r platform step result evidence; do
      case "$result" in
        pass) shown="pass" ;;
        fail) shown="**FAIL**" ;;
        error) shown="**TEST ERROR**" ;;
        *) shown="not covered" ;;
      esac
      echo "| $(cell "$platform") | $(cell "$step") | $shown | $(cell "$evidence") |"
    done < "$results"
    echo
    echo "</details>"
  fi
  echo
} >> "$summary"

[ -s "$results" ] || { echo "::error::no checks were recorded"; exit 1; }
if [ "$errors" -gt 0 ]; then
  echo "::error::$errors test errors"
  exit 1
fi
if [ "$fails" -gt 0 ]; then
  if [ "$strict" = true ]; then
    echo "::error::$fails product checks failed"
    exit 1
  fi
  echo "::warning::$fails product checks failed (defects of the published release; not failing a pull request run)"
fi
exit 0
