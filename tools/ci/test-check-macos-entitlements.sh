#!/usr/bin/env bash
# Tests check-macos-entitlements.sh and sign-macos-plugin-host.sh on fake
# bundles whose executables are copies of /usr/bin/true signed ad hoc:
# exactly the host's entitlement on exactly Contents/MacOS/deckpilot-plugin-host
# passes; any entitlement on the app or the agent, any other or additional
# entitlement on the host, and the host's entitlement anywhere else fail.
#
#   tools/ci/test-check-macos-entitlements.sh <work dir>
#
# macOS only (codesign); elsewhere it prints a note and passes.
set -euo pipefail

if [ "$(uname -s)" != Darwin ]; then
  echo "test-check-macos-entitlements: macOS only, skipped"
  exit 0
fi

dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$dir/../.." && pwd)
check="$dir/check-macos-entitlements.sh"
host_entitlements="$root/apps/desktop/src-tauri/entitlements/plugin-host.entitlements"
work="${1:?usage: test-check-macos-entitlements.sh <work dir>}"
rm -rf "$work" && mkdir -p "$work"
failures=0

plist() {
  local body=""
  local key
  for key in "$@"; do
    body="$body<key>${key%%=*}</key><${key#*=}/>"
  done
  printf '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0"><dict>%s</dict></plist>\n' "$body"
}
plist com.apple.security.cs.allow-jit=true > "$work/allow-jit.plist"
plist com.apple.security.cs.allow-unsigned-executable-memory=true com.apple.security.cs.disable-library-validation=true > "$work/host-plus.plist"
plist com.apple.security.cs.allow-unsigned-executable-memory=false > "$work/host-false.plist"
plist com.apple.security.get-task-allow=true > "$work/get-task-allow.plist"

# sign <file> [<entitlements file>]: ad hoc, hardened runtime.
sign() {
  if [ -n "${2-}" ]; then
    codesign --force --sign - --options runtime --entitlements "$2" "$1" 2> /dev/null
  else
    codesign --force --sign - --options runtime "$1" 2> /dev/null
  fi
}

# bundle [with-host]: app and agent without entitlements, and the host with
# exactly its entitlement.
bundle() {
  app="$work/DeckPilot.app"
  rm -rf "$app" && mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  local name
  for name in deckpilot-desktop deckpilot-agent; do
    cp /usr/bin/true "$app/Contents/MacOS/$name"
    sign "$app/Contents/MacOS/$name"
  done
  if [ "${1-}" = with-host ]; then
    cp /usr/bin/true "$app/Contents/MacOS/deckpilot-plugin-host"
    sign "$app/Contents/MacOS/deckpilot-plugin-host" "$host_entitlements"
  fi
}

# expect pass|fail "<message fragment>" "<description>" [<check arguments>]
expect() {
  local want="$1" fragment="$2" what="$3"
  shift 3
  local status=0
  "$check" "$@" > "$work/out" 2>&1 || status=$?
  if [ "$want" = pass ] && [ "$status" -eq 0 ]; then
    echo "ok   $what"
  elif [ "$want" = fail ] && [ "$status" -ne 0 ] && grep -q -F -- "$fragment" "$work/out"; then
    echo "ok   $what"
  else
    echo "FAIL $what (status $status, expected $want${fragment:+ with \"$fragment\"}):"
    sed 's/^/     /' "$work/out"
    failures=$((failures + 1))
  fi
}

bundle
expect pass "" "the default release: app and agent, no entitlements, no host" "$app"

bundle with-host
expect pass "" "with DECKPILOT_RELEASE_PLUGINS=1: the host with exactly its entitlement" "$app"
expect pass "" "--plugin-host on the signed host" --plugin-host "$app/Contents/MacOS/deckpilot-plugin-host"

bundle with-host
sign "$app/Contents/MacOS/deckpilot-desktop" "$host_entitlements"
expect fail "deckpilot-desktop has entitlements" "the host's entitlement on the app is refused" "$app"

bundle with-host
sign "$app/Contents/MacOS/deckpilot-agent" "$host_entitlements"
expect fail "deckpilot-agent has entitlements" "the host's entitlement on the agent is refused" "$app"

bundle
sign "$app/Contents/MacOS/deckpilot-agent" "$work/allow-jit.plist"
expect fail "deckpilot-agent has entitlements" "any entitlement on the agent is refused" "$app"

bundle with-host
sign "$app/Contents/MacOS/deckpilot-plugin-host"
expect fail "the plugin host's entitlements are {}" "a host without its entitlement is refused" "$app"

bundle with-host
sign "$app/Contents/MacOS/deckpilot-plugin-host" "$work/allow-jit.plist"
expect fail "com.apple.security.cs.allow-jit" "allow-jit instead of the host's entitlement is refused" "$app"

bundle with-host
sign "$app/Contents/MacOS/deckpilot-plugin-host" "$work/host-plus.plist"
expect fail "disable-library-validation" "an additional entitlement on the host is refused" "$app"
expect fail "disable-library-validation" "--plugin-host refuses an additional entitlement" --plugin-host "$app/Contents/MacOS/deckpilot-plugin-host"

bundle with-host
sign "$app/Contents/MacOS/deckpilot-plugin-host" "$work/host-false.plist"
expect fail "the plugin host's entitlements are" "the host's entitlement set to false is refused" "$app"

bundle with-host
sign "$app/Contents/MacOS/deckpilot-plugin-host" "$work/get-task-allow.plist"
expect fail "get-task-allow" "get-task-allow on the host is refused" "$app"

# The exception is the one path, not the name.
bundle with-host
mkdir -p "$app/Contents/Resources/bundled"
cp "$app/Contents/MacOS/deckpilot-plugin-host" "$app/Contents/Resources/bundled/deckpilot-plugin-host"
expect fail "Resources/bundled/deckpilot-plugin-host has entitlements" "the host's entitlement elsewhere in the bundle is refused" "$app"

bundle with-host
cp "$app/Contents/MacOS/deckpilot-plugin-host" "$app/Contents/MacOS/deckpilot-plugin-host-x"
expect fail "deckpilot-plugin-host-x has entitlements" "a second host under another name is refused" "$app"

# Not only executables: any Mach-O file.
bundle
cp /usr/bin/true "$app/Contents/Resources/helper.dylib"
sign "$app/Contents/Resources/helper.dylib" "$host_entitlements"
chmod a-x "$app/Contents/Resources/helper.dylib"
expect fail "helper.dylib has entitlements" "a non-executable Mach-O file with entitlements is refused" "$app"

# The policy file itself must grant exactly the one entitlement.
bundle with-host
PLUGIN_HOST_ENTITLEMENTS="$work/host-plus.plist" expect fail "must grant exactly" "a wider entitlements file is refused" "$app"

# sign-macos-plugin-host.sh signs ad hoc with the runtime, the identifier
# and exactly the host's entitlement.
cp /usr/bin/true "$work/deckpilot-plugin-host"
if APPLE_SIGNING_IDENTITY=- "$dir/sign-macos-plugin-host.sh" "$work/deckpilot-plugin-host" > "$work/out" 2>&1 \
  && grep -q '^Identifier=io.github.knittingcode.deckpilot.plugin-host$' <<< "$(codesign --display --verbose=2 "$work/deckpilot-plugin-host" 2>&1)" \
  && grep -q 'flags=.*runtime' <<< "$(codesign --display --verbose=2 "$work/deckpilot-plugin-host" 2>&1)"; then
  echo "ok   sign-macos-plugin-host.sh signs the host as the check expects"
else
  echo "FAIL sign-macos-plugin-host.sh:"
  sed 's/^/     /' "$work/out"
  failures=$((failures + 1))
fi
if (unset APPLE_SIGNING_IDENTITY; "$dir/sign-macos-plugin-host.sh" "$work/deckpilot-plugin-host") > /dev/null 2>&1; then
  echo "FAIL sign-macos-plugin-host.sh signs without APPLE_SIGNING_IDENTITY"
  failures=$((failures + 1))
else
  echo "ok   sign-macos-plugin-host.sh needs APPLE_SIGNING_IDENTITY"
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures failure(s)"
  exit 1
fi
echo "check-macos-entitlements: all cases passed"
