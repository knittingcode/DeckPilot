#!/usr/bin/env bash
# The published macOS disk image and updater archive of one architecture:
#
#   tools/release-smoke/macos.sh <assets dir> <version> <aarch64|x64>
#
# Mounts the disk image; Gatekeeper (spctl) on the image and the app,
# codesign --verify --deep --strict, the stapled tickets, the release's own
# signature check (.github/scripts/verify-macos-app.sh), the plugin host's
# signature; copies the app to $SMOKE_WORK/Applications (the "install") and
# checks its contents; the updater archive must hold the same signed app.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-smoke/common.sh
source "$here/common.sh"

dir="${1:?usage: macos.sh <assets dir> <version> <aarch64|x64>}"
version="${2:?}"
arch="${3:?}"
case "$arch" in aarch64) machine=arm64 ;; x64) machine=x86_64 ;; *) echo "arch must be aarch64 or x64" >&2; exit 2 ;; esac
dmg="$dir/DeckPilot_${version}_${arch}.dmg"
archive="$dir/DeckPilot_${arch}.app.tar.gz"
applications="${SMOKE_WORK:?}/Applications"
installed="$applications/DeckPilot.app"

if [ "$(uname -m)" != "$machine" ]; then
  record "runner architecture" error "this runner is $(uname -m), the $arch build needs $machine"
fi
[ -f "$dmg" ] || { record "dmg: file" error "$dmg was not downloaded"; exit 0; }

# checked <step> <evidence: all|last> <command...>: records the command's
# outcome. Gatekeeper and stapler ask Apple's ticket service: a request
# that times out is retried (3 attempts), and if it still cannot reach the
# service it is a test error, not a failed check.
checked() {
  local step="$1" show="$2" out="" status=0 attempt evidence
  shift 2
  for attempt in 1 2 3; do
    out=$("$@" 2>&1) && status=0 || status=$?
    if [ "$status" -eq 0 ] || ! grep -q -E 'NSURLErrorDomain|timed out|network connection' <<< "$out"; then break; fi
    [ "$attempt" -lt 3 ] && sleep 20
  done
  if [ "$show" = last ]; then evidence=$(printf '%s' "$out" | tail -n1); else evidence=$(printf '%s' "$out" | tr '\n' ' '); fi
  if [ "$status" -eq 0 ]; then
    record "$step" pass "$evidence"
  elif grep -q -E 'NSURLErrorDomain|timed out|network connection' <<< "$out"; then
    record "$step" error "Apple's service could not be reached (3 attempts): $(printf '%s' "$out" | tr '\n' ' ')"
  else
    record "$step" fail "$(printf '%s' "$out" | tr '\n' ' ')"
  fi
}

# The disk image as downloaded.
checked "dmg: Gatekeeper (spctl open)" all spctl --assess --type open --context context:primary-signature -vv "$dmg"
checked "dmg: stapled ticket" last xcrun stapler validate "$dmg"

mount="$SMOKE_WORK/dmg-mount"
mkdir -p "$mount"
if ! out=$(hdiutil attach -nobrowse -readonly -mountpoint "$mount" "$dmg" 2>&1); then
  record "dmg: mount" fail "$out"
  exit 0
fi
detach() { hdiutil detach "$mount" > /dev/null 2>&1 || hdiutil detach -force "$mount" > /dev/null 2>&1 || true; }
trap detach EXIT
app="$mount/DeckPilot.app"
if [ -d "$app" ] && [ -L "$mount/Applications" ]; then
  record "dmg: mount" pass "DeckPilot.app and the Applications link -> $(readlink "$mount/Applications")"
else
  record "dmg: mount" fail "contents: $(find "$mount" -mindepth 1 -maxdepth 1 | tr '\n' ' ')"
fi

if out=$(codesign --verify --deep --strict --verbose=2 "$app" 2>&1); then
  record "app: codesign --verify --deep --strict" pass "$(printf '%s' "$out" | tr '\n' ' ')"
else
  record "app: codesign --verify --deep --strict" fail "$(printf '%s' "$out" | tr '\n' ' ')"
fi
checked "app: Gatekeeper (spctl -a -t execute)" all spctl -a -vvv -t execute "$app"
checked "app: stapled ticket" last xcrun stapler validate "$app"
team=$(codesign --display --verbose=2 "$app/Contents/MacOS/deckpilot-desktop" 2>&1 | sed -n 's/^TeamIdentifier=//p')
authority=$(codesign --display --verbose=2 "$app/Contents/MacOS/deckpilot-desktop" 2>&1 | sed -n 's/^Authority=//p' | head -n1)
# The release's own check, with the team of the app's signature: every
# executable signed by that one team, hardened runtime, entitlements, the
# agent's and the host's identifiers, Gatekeeper and the ticket.
if [ -n "$team" ]; then
  checked "app: verify-macos-app.sh (release check, team $team, $authority)" last \
    env APPLE_TEAM_ID="$team" "$SMOKE_ROOT/.github/scripts/verify-macos-app.sh" "$app"
else
  record "app: verify-macos-app.sh (release check)" fail "the app has no TeamIdentifier"
fi
host="$app/Contents/MacOS/deckpilot-plugin-host"
if [ -f "$host" ]; then
  details=$(codesign --display --verbose=2 "$host" 2>&1)
  entitlements=$(codesign --display --entitlements - --xml "$host" 2> /dev/null | plutil -convert json -o - - 2> /dev/null || true)
  if codesign --verify --strict "$host" 2> /dev/null && printf '%s' "$details" | grep -q '^Identifier=io.github.knittingcode.deckpilot.plugin-host$' \
    && printf '%s' "$details" | grep -q "^TeamIdentifier=$team$" && printf '%s' "$details" | grep -q 'flags=.*runtime'; then
    record "plugin host: signature" pass "$(printf '%s' "$details" | grep -E '^(Identifier|TeamIdentifier|Timestamp|CodeDirectory)' | tr '\n' ' ') entitlements: $entitlements"
  else
    record "plugin host: signature" fail "$(printf '%s' "$details" | tr '\n' ' ')"
  fi
else
  record "plugin host: signature" fail "$host is missing"
fi

# Installed by copying, as a user drags it to Applications.
rm -rf "$applications" && mkdir -p "$applications"
if ditto "$app" "$installed" && codesign --verify --deep --strict "$installed" 2> /dev/null; then
  record "app: copied to a temporary Applications folder" pass "$installed (signature still valid)"
else
  record "app: copied to a temporary Applications folder" fail "the copy at $installed does not verify"
fi
detach
trap - EXIT

# Contents of the installed copy.
macos="$installed/Contents/MacOS"
check_exec "app: binaries present and executable" "$macos/deckpilot-desktop" "$macos/deckpilot-agent" "$macos/deckpilot-plugin-host"
archs=""
for binary in deckpilot-desktop deckpilot-agent deckpilot-plugin-host; do
  archs="$archs $binary=$(lipo -archs "$macos/$binary" 2> /dev/null)"
done
if [ "$(printf '%s' "$archs" | grep -o "=$machine" | wc -l | tr -d ' ')" = 3 ]; then
  record "app: architecture" pass "$archs"
else
  record "app: architecture" fail "$archs (expected $machine)"
fi
plist="$installed/Contents/Info.plist"
bundle_version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$plist" 2> /dev/null)
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist" 2> /dev/null)
if [ "$bundle_version" = "$version" ] && [ "$bundle_id" = "$APP_IDENTIFIER" ]; then
  record "app: Info.plist" pass "$bundle_id $bundle_version"
else
  record "app: Info.plist" fail "'$bundle_id' '$bundle_version'"
fi
check_version "app: agent version" "$macos/deckpilot-agent" "$version"
check_bundled "app: bundled plugins and pins.json" "$installed/Contents/Resources/bundled" "$macos/deckpilot-agent"
check_roots "app: production registry roots embedded in the agent" "$macos/deckpilot-agent"
check_updater_key "app: updater key embedded in the app" "$macos/deckpilot-desktop"

# The updater archive: the same signed app.
if [ -f "$archive" ]; then
  extracted="$SMOKE_WORK/updater-archive"
  rm -rf "$extracted" && mkdir -p "$extracted"
  tar -xzf "$archive" -C "$extracted"
  top=$(cd "$extracted" && find . -mindepth 1 -maxdepth 1 | sed 's#^\./##' | tr '\n' ' ')
  a=$(codesign --display --verbose=3 "$extracted/DeckPilot.app" 2>&1 | sed -n 's/^CDHash=//p')
  b=$(codesign --display --verbose=3 "$installed" 2>&1 | sed -n 's/^CDHash=//p')
  if [ "$top" = "DeckPilot.app " ] && codesign --verify --deep --strict "$extracted/DeckPilot.app" 2> /dev/null && [ -n "$a" ] && [ "$a" = "$b" ]; then
    record "updater archive: same signed app as the disk image" pass "$(basename "$archive"): CDHash $a"
  else
    record "updater archive: same signed app as the disk image" fail "contents '$top', CDHash '$a' vs disk image '$b'"
  fi
else
  record "updater archive: same signed app as the disk image" error "$archive was not downloaded"
fi
exit 0
