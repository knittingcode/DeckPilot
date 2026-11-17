#!/usr/bin/env bash
# The entitlement policy of a signed DeckPilot.app (macOS):
#
#   tools/ci/check-macos-entitlements.sh <DeckPilot.app>
#   tools/ci/check-macos-entitlements.sh --plugin-host <binary>
#
# Every Mach-O file in the bundle has no entitlements, with one exception:
# the plugin host at exactly Contents/MacOS/deckpilot-plugin-host (in every
# release, and in local packages built with DECKPILOT_RELEASE_PLUGINS=1;
# .github/scripts/verify-macos-app.sh requires it in a release) has exactly the
# entitlements of apps/desktop/src-tauri/entitlements/plugin-host.entitlements
# (com.apple.security.cs.allow-unsigned-executable-memory, true) and nothing
# else. The application and the agent never have any. Why the host needs it:
# docs/marketplace/distribution.md, "macOS: the plugin host's entitlement".
#
# --plugin-host checks one binary against the host's entitlements (after
# tools/ci/sign-macos-plugin-host.sh, before it is bundled).
#
# Only the entitlements are checked here; .github/scripts/verify-macos-app.sh
# also checks the signature, team, hardened runtime and notarization.
# Needs codesign, plutil and jq (macOS).
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
allowed_file="${PLUGIN_HOST_ENTITLEMENTS:-$root/apps/desktop/src-tauri/entitlements/plugin-host.entitlements}"
host_path="Contents/MacOS/deckpilot-plugin-host"

fail() {
  echo "::error::$*" >&2
  exit 1
}

# The entitlements of a signed file as canonical JSON ({} for none).
entitlements_of() {
  local xml
  xml=$(codesign --display --entitlements - --xml "$1" 2> /dev/null || true)
  if [ -z "$xml" ]; then
    echo '{}'
    return
  fi
  printf '%s' "$xml" | plutil -convert json -o - - | jq -S -c .
}

allowed=$(plutil -convert json -o - "$allowed_file" | jq -S -c .)
[ "$allowed" = '{"com.apple.security.cs.allow-unsigned-executable-memory":true}' ] \
  || fail "$allowed_file must grant exactly com.apple.security.cs.allow-unsigned-executable-memory, not $allowed"

check_host() {
  local actual
  actual=$(entitlements_of "$1")
  [ "$actual" = "$allowed" ] \
    || fail "$1: the plugin host's entitlements are $actual, expected exactly $allowed"
  echo "$1: plugin host, entitlements $actual"
}

if [ "${1-}" = --plugin-host ]; then
  host="${2:?usage: check-macos-entitlements.sh --plugin-host <binary>}"
  [ -f "$host" ] || fail "$host does not exist"
  check_host "$host"
  exit 0
fi

app="${1:?usage: check-macos-entitlements.sh <DeckPilot.app> | --plugin-host <binary>}"
[ -d "$app/Contents" ] || fail "$app is not an application bundle"

checked=0
while IFS= read -r -d '' file; do
  file "$file" | grep -q "Mach-O" || continue
  checked=$((checked + 1))
  relative="${file#"$app"/}"
  if [ "$relative" = "$host_path" ]; then
    check_host "$file"
    continue
  fi
  actual=$(entitlements_of "$file")
  [ "$actual" = '{}' ] \
    || fail "$file has entitlements $actual; only $host_path may have any (exactly $allowed)"
  echo "$file: no entitlements"
done < <(find "$app/Contents" -type f -print0)
[ "$checked" -ge 1 ] || fail "$app contains no Mach-O file"
echo "$(basename "$app"): entitlements checked ($checked Mach-O files)"
