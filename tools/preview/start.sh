#!/usr/bin/env bash
# Starts the integration preview (agent with a simulated MK.2 + native app)
# in the background, next to any regular development session.
. "$(dirname "$0")/lib.sh"
[ -f "$INFO" ] || write_info "$(git merge-base HEAD origin/main | cut -c1-7)"
export VITE_DECKPILOT_PREVIEW
VITE_DECKPILOT_PREVIEW=$(cat "$INFO")
if [ ! -f "$DATA/deckpilot.db" ]; then
  # Password-protected (development password): no Keychain entry is used.
  dev seed --password
fi
dev up --devices mk2
