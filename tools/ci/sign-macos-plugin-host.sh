#!/usr/bin/env bash
# Signs the staged plugin host for a macOS package (the release workflow,
# the Packaging workflow ad hoc, DECKPILOT_RELEASE_PLUGINS=1 locally),
# before the Tauri bundler runs:
#
#   APPLE_SIGNING_IDENTITY=<identity> tools/ci/sign-macos-plugin-host.sh <binary>
#
# with the hardened runtime, a fixed identifier
# (io.github.knittingcode.deckpilot.plugin-host) and exactly
# apps/desktop/src-tauri/entitlements/plugin-host.entitlements
# (com.apple.security.cs.allow-unsigned-executable-memory). The bundler
# signs every external binary with the one set of entitlements of the
# whole app (none), so the host is not an external binary on macOS: it is
# copied into Contents/MacOS as signed here (tauri.plugins.macos.conf.json,
# bundle.macOS.files), and the bundler then signs the app around it and
# notarizes the whole. Why the host needs the entitlement:
# docs/marketplace/distribution.md, "macOS: the plugin host's entitlement".
#
# APPLE_SIGNING_IDENTITY is the identity the bundler signs the app with:
# "Developer ID Application: Name (TEAMID)" (secure timestamp), or "-" for
# an ad hoc signature (local tests). It must be in the keychain search list
# (or in SIGN_KEYCHAIN); this script never imports a certificate. Then
# tools/ci/check-macos-entitlements.sh --plugin-host checks the result.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
entitlements="$root/apps/desktop/src-tauri/entitlements/plugin-host.entitlements"
identifier="io.github.knittingcode.deckpilot.plugin-host"

binary="${1:?usage: APPLE_SIGNING_IDENTITY=<identity> sign-macos-plugin-host.sh <binary>}"
identity="${APPLE_SIGNING_IDENTITY:?APPLE_SIGNING_IDENTITY is not set}"
[ -f "$binary" ] || { echo "::error::$binary does not exist" >&2; exit 1; }

args=(--force --sign "$identity" --options runtime --identifier "$identifier" --entitlements "$entitlements")
[ "$identity" = "-" ] || args+=(--timestamp)
[ -z "${SIGN_KEYCHAIN-}" ] || args+=(--keychain "$SIGN_KEYCHAIN")
codesign "${args[@]}" "$binary"

details=$(codesign --display --verbose=2 "$binary" 2>&1)
grep -q "^Identifier=$identifier$" <<< "$details" \
  || { echo "::error::$binary: identifier is not $identifier" >&2; exit 1; }
grep -q "flags=.*runtime" <<< "$details" \
  || { echo "::error::$binary lacks the hardened runtime" >&2; exit 1; }
"$root/tools/ci/check-macos-entitlements.sh" --plugin-host "$binary"
