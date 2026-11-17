#!/usr/bin/env bash
# Verifies a signed and notarized DeckPilot.app: a valid, strict signature
# over the whole bundle; every executable inside signed by the release team
# with the hardened runtime; the plugin host at exactly
# Contents/MacOS/deckpilot-plugin-host (every release ships the plugin
# Marketplace); no entitlements on any of them, except exactly
# com.apple.security.cs.allow-unsigned-executable-memory on exactly the
# host (tools/ci/check-macos-entitlements.sh); the agent's fixed identifier
# (its Keychain access depends on it) and the host's; Gatekeeper's
# assessment; the stapled notarization ticket.
set -euo pipefail

app="${1:?usage: verify-macos-app.sh <DeckPilot.app>}"
: "${APPLE_TEAM_ID:?variable APPLE_TEAM_ID is not configured}"
agent_identifier="io.github.knittingcode.deckpilot.agent"
host_identifier="io.github.knittingcode.deckpilot.plugin-host"

codesign --verify --deep --strict --verbose=2 "$app"

found=0
while IFS= read -r -d '' file; do
  file "$file" | grep -q "Mach-O" || continue
  found=$((found + 1))
  details=$(codesign --display --verbose=2 "$file" 2>&1)
  identifier=$(echo "$details" | sed -n 's/^Identifier=//p')
  echo "$file: $identifier"
  echo "$details" | grep -q "^TeamIdentifier=$APPLE_TEAM_ID$" \
    || { echo "::error::$file is not signed by team $APPLE_TEAM_ID"; exit 1; }
  echo "$details" | grep -q "flags=.*runtime" \
    || { echo "::error::$file lacks the hardened runtime"; exit 1; }
done < <(find "$app/Contents" -type f -perm -u+x -print0)
[ "$found" -ge 3 ] || { echo "::error::expected the app, the agent and the plugin host executables, found $found"; exit 1; }
[ -f "$app/Contents/MacOS/deckpilot-plugin-host" ] \
  || { echo "::error::$app has no plugin host (Contents/MacOS/deckpilot-plugin-host)"; exit 1; }

# No entitlements anywhere, but the plugin host's one (every Mach-O file).
"$(dirname "$0")/../../tools/ci/check-macos-entitlements.sh" "$app"

agent=$(codesign --display --verbose=2 "$app/Contents/MacOS/deckpilot-agent" 2>&1)
echo "$agent" | grep -q "^Identifier=$agent_identifier$" \
  || { echo "::error::the agent's identifier is not $agent_identifier"; exit 1; }
codesign --display -r- "$app/Contents/MacOS/deckpilot-agent" 2>&1 | grep "designated"
host=$(codesign --display --verbose=2 "$app/Contents/MacOS/deckpilot-plugin-host" 2>&1)
echo "$host" | grep -q "^Identifier=$host_identifier$" \
  || { echo "::error::the plugin host's identifier is not $host_identifier"; exit 1; }
echo "$host" | grep -q "^Timestamp=" \
  || { echo "::error::the plugin host has no secure timestamp"; exit 1; }

spctl --assess --type execute --verbose=2 "$app"
xcrun stapler validate "$app"
echo "$(basename "$app"): signed, notarized and stapled"
