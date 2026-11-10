#!/usr/bin/env bash
# Verifies a signed and notarized DeckPilot.app: a valid, strict signature
# over the whole bundle; every executable inside signed by the release team
# with the hardened runtime and no entitlements; the agent's fixed identifier
# (its Keychain access depends on it); Gatekeeper's assessment; the stapled
# notarization ticket.
set -euo pipefail

app="${1:?usage: verify-macos-app.sh <DeckPilot.app>}"
: "${APPLE_TEAM_ID:?variable APPLE_TEAM_ID is not configured}"
agent_identifier="io.github.knittingcode.deckpilot.agent"

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
  entitlements=$(codesign --display --entitlements - --xml "$file" 2>/dev/null || true)
  if echo "$entitlements" | grep -q "<key>"; then
    echo "::error::$file has entitlements; DeckPilot needs none"
    exit 1
  fi
done < <(find "$app/Contents" -type f -perm -u+x -print0)
[ "$found" -ge 2 ] || { echo "::error::expected the app and the agent executables, found $found"; exit 1; }

agent=$(codesign --display --verbose=2 "$app/Contents/MacOS/deckpilot-agent" 2>&1)
echo "$agent" | grep -q "^Identifier=$agent_identifier$" \
  || { echo "::error::the agent's identifier is not $agent_identifier"; exit 1; }
codesign --display -r- "$app/Contents/MacOS/deckpilot-agent" 2>&1 | grep "designated"

spctl --assess --type execute --verbose=2 "$app"
xcrun stapler validate "$app"
echo "$(basename "$app"): signed, notarized and stapled"
