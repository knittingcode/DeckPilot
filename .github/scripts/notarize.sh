#!/usr/bin/env bash
# Submits a file (.dmg, or a .zip of signed code) for notarization, waits for
# Apple's result and fails unless it is "Accepted"; on rejection the
# notarization log is printed. Needs APPLE_API_KEY_PATH (see
# apple-api-key.sh), APPLE_API_KEY_ID and APPLE_API_ISSUER.
set -euo pipefail

file="${1:?usage: notarize.sh <file>}"
: "${APPLE_API_KEY_PATH:?run apple-api-key.sh first}"
: "${APPLE_API_KEY_ID:?variable APPLE_API_KEY_ID is not configured}"
: "${APPLE_API_ISSUER:?variable APPLE_API_ISSUER is not configured}"
auth=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER")

result=$(xcrun notarytool submit "$file" "${auth[@]}" --wait --timeout 2h --output-format json)
status=$(printf '%s' "$result" | python3 -c 'import json, sys; print(json.load(sys.stdin).get("status", ""))')
id=$(printf '%s' "$result" | python3 -c 'import json, sys; print(json.load(sys.stdin).get("id", ""))')
echo "notarization of $(basename "$file"): $status (submission $id)"
if [ "$status" != "Accepted" ]; then
  [ -n "$id" ] && xcrun notarytool log "$id" "${auth[@]}" || true
  echo "::error::notarization of $(basename "$file") was not accepted: $status"
  exit 1
fi
