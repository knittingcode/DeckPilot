#!/usr/bin/env bash
# Imports the Developer ID Application certificate (secret APPLE_CERTIFICATE,
# base64 .p12, with APPLE_CERTIFICATE_PASSWORD) into a temporary keychain and
# checks that it provides exactly the configured signing identity of the
# configured team. Exports the keychain path as DECKPILOT_KEYCHAIN. Used by
# the signing check; release builds let the Tauri bundler import it.
set -euo pipefail

: "${APPLE_CERTIFICATE:?secret APPLE_CERTIFICATE is not configured}"
: "${APPLE_CERTIFICATE_PASSWORD:?secret APPLE_CERTIFICATE_PASSWORD is not configured}"
: "${APPLE_SIGNING_IDENTITY:?variable APPLE_SIGNING_IDENTITY is not configured}"
: "${APPLE_TEAM_ID:?variable APPLE_TEAM_ID is not configured}"

case "$APPLE_SIGNING_IDENTITY" in
  "Developer ID Application: "*"($APPLE_TEAM_ID)") ;;
  *)
    echo "::error::APPLE_SIGNING_IDENTITY must be a 'Developer ID Application: … ($APPLE_TEAM_ID)' identity"
    exit 1
    ;;
esac

keychain="${RUNNER_TEMP:?}/deckpilot-signing.keychain-db"
password=$(openssl rand -hex 24)
certificate="$RUNNER_TEMP/certificate.p12"
umask 077
printf '%s' "$APPLE_CERTIFICATE" \
  | python3 -c 'import base64, sys; sys.stdout.buffer.write(base64.b64decode(sys.stdin.read()))' > "$certificate"

security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 3600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$certificate" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
rm -f "$certificate"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
# Add the temporary keychain to the search list, keeping the existing ones.
keychains=("$keychain")
while IFS= read -r existing; do
  existing=$(echo "$existing" | tr -d '"' | sed 's/^ *//')
  [ -n "$existing" ] && keychains+=("$existing")
done < <(security list-keychains -d user)
security list-keychains -d user -s "${keychains[@]}"

identities=$(security find-identity -v -p codesigning "$keychain")
echo "$identities" | grep -q -F "\"$APPLE_SIGNING_IDENTITY\"" \
  || { echo "::error::the certificate does not provide the identity in APPLE_SIGNING_IDENTITY"; exit 1; }
# Expiry: signing fails after it; warn well before.
certificate_pem=$(security find-certificate -c "$APPLE_SIGNING_IDENTITY" -p "$keychain")
echo "the Developer ID certificate expires: $(echo "$certificate_pem" | openssl x509 -noout -enddate | cut -d= -f2)"
if ! echo "$certificate_pem" | openssl x509 -noout -checkend 0 >/dev/null; then
  echo "::error::the Developer ID certificate has expired; renew it in the Apple Developer account and update APPLE_CERTIFICATE"
  exit 1
elif ! echo "$certificate_pem" | openssl x509 -noout -checkend $((60 * 86400)) >/dev/null; then
  echo "::warning::the Developer ID certificate expires within 60 days; renew it and update APPLE_CERTIFICATE"
fi
echo "DECKPILOT_KEYCHAIN=$keychain" >> "${GITHUB_ENV:?}"
echo "signing identity available: $APPLE_SIGNING_IDENTITY"
