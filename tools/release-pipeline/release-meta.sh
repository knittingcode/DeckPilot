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
# notarization were configured, so both macOS builds must have them),
# ASSETS_DIR (the draft's files), DISTRIBUTION_DIR (the builds'
# distribution records: {"target", "distribution_channel",
# "package_format", "application_version"}, read from each build's binary),
# TARGETS (the packaging matrix; default docs/distribution/targets.toml).
#
# release-meta.json also lists the release's files with platform, arch,
# package format, distribution channel, update provider, SHA-256 and size
# (`artifacts`, docs/distribution/README.md section 6.2; written by
# release-artifacts.py), for the downstream distribution jobs. It is added
# after the other fields, which stay as they were; the release notes do
# not change.
#
# Fails if a build has no fragment, if a macOS build is not signed or not
# notarized although signing was configured (a failed signing or
# notarization), if a fragment claims a signature its platform cannot
# have in this pipeline, or if the files or the builds' distribution
# records do not match the packaging matrix.
set -euo pipefail

fragments="${1:?usage: release-meta.sh <fragments dir> <release-meta.json> <notes out>}"
meta="${2:?}"
notes_out="${3:?}"
: "${TAG:?}" "${VERSION:?}" "${CHANNEL:?}" "${SHA:?}" "${MANIFEST:?}" "${NOTES_FILE:?}" "${MACOS_SIGNING:?}"
: "${ASSETS_DIR:?}" "${DISTRIBUTION_DIR:?}"
here="$(cd "$(dirname "$0")" && pwd)"
targets_file="${TARGETS:-$here/../../docs/distribution/targets.toml}"
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
        if [ "$codesign" != unsigned ] || [ "$notarized" = true ]; then
          fail "$target: signing was not configured, but the build reports $codesign/$notarized"
        fi
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

case "$CHANNEL" in
  stable) prerelease=false ;;
  prerelease) prerelease=true ;;
  *) fail "unknown channel $CHANNEL" ;;
esac
artifacts=$(VERSION="$VERSION" PRERELEASE="$prerelease" MANIFEST="$MANIFEST" \
  python3 -I "$here/release-artifacts.py" "$targets_file" "$ASSETS_DIR" "$DISTRIBUTION_DIR") \
  || fail "the release's files or distribution records do not match the packaging matrix"

jq -n --arg tag "$TAG" --arg version "$VERSION" --arg channel "$CHANNEL" --arg commit "$SHA" \
  --arg manifest "$MANIFEST" --argjson builds "$all" --argjson artifacts "$artifacts" '{
    tag: $tag, version: $version, channel: $channel, commit: $commit, manifest: $manifest,
    updater_signature: "minisign",
    platforms: ($builds | map({key: .target, value: {codesign, notarized}}) | from_entries),
    artifacts: $artifacts
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
echo "release-meta.json: $(jq '.artifacts | length' "$meta") artifacts: $(jq -r '[.artifacts[] | "\(.name) (\(.distribution_channel)/\(.package_format))"] | join(", ")' "$meta")"
