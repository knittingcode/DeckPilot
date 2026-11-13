#!/usr/bin/env bash
# Checks that a release has every required asset, and nothing of the other
# update channel.
#
#   tools/release-pipeline/check-assets.sh <asset list file> [--final]
#
# <asset list file>: the release's asset names, one per line. --final also
# requires release-meta.json and SHA256SUMS (written last in stage A).
#
# Environment: VERSION, PRERELEASE (true/false), MANIFEST (latest.json or
# prerelease.json).
#
# Asset names are those of tauri-action and the Tauri bundler (compare the
# v0.5.0-2 draft): installers carry the version, the macOS updater archives
# do not. Stable releases also have the .rpm (RPM refuses "-" in versions).
set -euo pipefail

list="${1:?usage: check-assets.sh <asset list file> [--final]}"
final="${2-}"
: "${VERSION:?}" "${PRERELEASE:?}" "${MANIFEST:?}"
fail() {
  echo "::error::$*" >&2
  exit 1
}

v="$VERSION"
required=(
  "DeckPilot_${v}_aarch64.dmg"
  "DeckPilot_${v}_x64.dmg"
  "DeckPilot_aarch64.app.tar.gz" "DeckPilot_aarch64.app.tar.gz.sig"
  "DeckPilot_x64.app.tar.gz" "DeckPilot_x64.app.tar.gz.sig"
  "DeckPilot_${v}_amd64.AppImage" "DeckPilot_${v}_amd64.AppImage.sig"
  "DeckPilot_${v}_amd64.deb" "DeckPilot_${v}_amd64.deb.sig"
  "DeckPilot_${v}_x64_en-US.msi" "DeckPilot_${v}_x64_en-US.msi.sig"
  "DeckPilot_${v}_x64-setup.exe" "DeckPilot_${v}_x64-setup.exe.sig"
  "$MANIFEST"
)
case "$MANIFEST" in
  latest.json) other=prerelease.json ;;
  prerelease.json) other=latest.json ;;
  *) fail "unknown manifest $MANIFEST" ;;
esac
[ "$PRERELEASE" = true ] || required+=("DeckPilot-${v}-1.x86_64.rpm" "DeckPilot-${v}-1.x86_64.rpm.sig")
[ "$final" = --final ] && required+=(release-meta.json SHA256SUMS)

missing=""
for name in "${required[@]}"; do
  grep -q -x -F -- "$name" "$list" || missing="$missing $name"
done
[ -z "$missing" ] || fail "the release lacks:$missing"
! grep -q -x -F -- "$other" "$list" \
  || fail "the release contains $other: it would be offered on the wrong update channel"
echo "all ${#required[@]} required assets are present"
