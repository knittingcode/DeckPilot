#!/usr/bin/env bash
# Stage A: records what was actually signed, from the build jobs' signing
# fragments, as release-meta.json and as a section of the release notes.
# Nothing is claimed that a build did not measure.
#
#   tools/release-pipeline/release-meta.sh <fragments dir> <release-meta.json> <notes out>
#
# Fragments (one per build target, written by release.yml):
#   {"target": "...", "codesign": "developer-id" | "authenticode" | "unsigned",
#    "notarized": true | false | null}
#
# Environment: TAG, VERSION, CHANNEL, SHA, MANIFEST, NOTES_FILE (the
# changelog notes), MACOS_SIGNING (true: Developer ID signing and
# notarization were configured, so both macOS builds must have them).
#
# Fails if a build has no fragment, if a macOS build is not signed or not
# notarized although signing was configured (a failed signing or
# notarization), or if a fragment claims a signature its platform cannot
# have in this pipeline.
set -euo pipefail

fragments="${1:?usage: release-meta.sh <fragments dir> <release-meta.json> <notes out>}"
meta="${2:?}"
notes_out="${3:?}"
: "${TAG:?}" "${VERSION:?}" "${CHANNEL:?}" "${SHA:?}" "${MANIFEST:?}" "${NOTES_FILE:?}" "${MACOS_SIGNING:?}"
fail() {
  echo "::error::$*" >&2
  exit 1
}

targets="aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-msvc x86_64-unknown-linux-gnu"
shopt -s nullglob
files=("$fragments"/*.json)
[ "${#files[@]}" -gt 0 ] || fail "no signing fragments in $fragments"
all=$(jq -s '.' "${files[@]}")
for target in $targets; do
  count=$(jq --arg t "$target" '[.[] | select(.target == $t)] | length' <<< "$all")
  [ "$count" -eq 1 ] || fail "$target: $count signing records (expected 1); did its build finish?"
  codesign=$(jq -r --arg t "$target" '.[] | select(.target == $t) | .codesign' <<< "$all")
  notarized=$(jq -r --arg t "$target" '.[] | select(.target == $t) | .notarized' <<< "$all")
  case "$target" in
    *-apple-darwin)
      if [ "$MACOS_SIGNING" = true ]; then
        [ "$codesign" = developer-id ] || fail "$target: signing failed (code signature: $codesign)"
        [ "$notarized" = true ] || fail "$target: notarization failed or was skipped"
      else
        [ "$codesign" = unsigned ] && [ "$notarized" != true ] \
          || fail "$target: signing was not configured, but the build reports $codesign/$notarized"
      fi
      ;;
    *-windows-*)
      case "$codesign" in authenticode | unsigned) ;; *) fail "$target: unknown code signature $codesign" ;; esac
      [ "$notarized" = null ] || fail "$target: Windows builds are not notarized"
      ;;
    *-linux-*)
      [ "$codesign" = unsigned ] || fail "$target: Linux packages carry no code signature in this pipeline"
      [ "$notarized" = null ] || fail "$target: Linux builds are not notarized"
      ;;
  esac
done
extra=$(jq -r --arg targets "$targets" '.[].target | select(. as $t | ($targets | split(" ") | index($t)) | not)' <<< "$all")
[ -z "$extra" ] || fail "unknown build targets: $extra"

jq -n --arg tag "$TAG" --arg version "$VERSION" --arg channel "$CHANNEL" --arg commit "$SHA" \
  --arg manifest "$MANIFEST" --argjson builds "$all" '{
    tag: $tag, version: $version, channel: $channel, commit: $commit, manifest: $manifest,
    updater_signature: "minisign",
    platforms: ($builds | map({key: .target, value: {codesign, notarized}}) | from_entries)
  }' > "$meta"

describe() {
  case "$1" in
    developer-id) echo "Developer ID" ;;
    authenticode) echo "Authenticode" ;;
    *) echo "**unsigned**" ;;
  esac
}
{
  sed -e 's/[[:space:]]*$//' "$NOTES_FILE"
  echo
  echo "## Code signing"
  echo
  echo "| Build | Code signature | Notarized |"
  echo "| ----- | -------------- | --------- |"
  for target in $targets; do
    codesign=$(jq -r --arg t "$target" '.platforms[$t].codesign' "$meta")
    notarized=$(jq -r --arg t "$target" '.platforms[$t].notarized' "$meta")
    case "$notarized" in true) n=yes ;; false) n=no ;; *) n="n/a" ;; esac
    echo "| $target | $(describe "$codesign") | $n |"
  done
  echo
  echo "Every updater package is signed with the DeckPilot updater key and verified by the app before installing. \"Unsigned\" means no platform code signature: Windows SmartScreen or Gatekeeper may warn, and Linux packages are not signed by a distribution key."
} > "$notes_out"
echo "release-meta.json: $(jq -c .platforms "$meta")"
