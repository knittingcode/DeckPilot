#!/usr/bin/env bash
# Generates a throwaway updater signing key for the package checks, so that
# they build the updater artifacts (and their signatures) the way a release
# does. The key exists only in this job and is never the release key.
#
# Exports for later steps (GITHUB_ENV):
#   TAURI_SIGNING_PRIVATE_KEY, TAURI_SIGNING_PRIVATE_KEY_PASSWORD  the throwaway key
#   THROWAWAY_UPDATER_PUBKEY   its public key, to verify the signatures
#   THROWAWAY_UPDATER_CONFIG   a Tauri configuration file with that public key
set -euo pipefail

dir="${RUNNER_TEMP:?}/throwaway-updater"
rm -rf "$dir"
mkdir -p "$dir"
pnpm --filter @deckpilot/desktop tauri signer generate --ci -p "" -w "$dir/updater.key" >/dev/null
public=$(tr -d '\r\n' < "$dir/updater.key.pub")
jq -n --arg key "$public" '{plugins: {updater: {pubkey: $key}}}' > "$dir/updater.conf.json"
{
  echo "TAURI_SIGNING_PRIVATE_KEY=$(tr -d '\r\n' < "$dir/updater.key")"
  echo "TAURI_SIGNING_PRIVATE_KEY_PASSWORD="
  echo "THROWAWAY_UPDATER_PUBKEY=$public"
  echo "THROWAWAY_UPDATER_CONFIG=$dir/updater.conf.json"
} >> "${GITHUB_ENV:?}"
echo "throwaway updater key generated (not the release key)"
