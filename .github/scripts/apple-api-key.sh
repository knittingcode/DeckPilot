#!/usr/bin/env bash
# Writes the App Store Connect API private key (secret APPLE_API_KEY: the
# .p8 file's content, or its base64 encoding) to a private file named
# AuthKey_<APPLE_API_KEY_ID>.p8 and exports its path as APPLE_API_KEY_PATH
# for later steps. Nothing secret is printed.
#
# Note the naming: the Tauri bundler's APPLE_API_KEY means the key *ID*; the
# release workflow passes vars.APPLE_API_KEY_ID under that name.
set -euo pipefail

: "${APPLE_API_KEY_CONTENT:?secret APPLE_API_KEY is not configured}"
: "${APPLE_API_KEY_ID:?variable APPLE_API_KEY_ID is not configured}"

dir="${RUNNER_TEMP:?}/private_keys"
mkdir -p "$dir"
chmod 700 "$dir"
file="$dir/AuthKey_${APPLE_API_KEY_ID}.p8"
umask 077

if printf '%s' "$APPLE_API_KEY_CONTENT" | grep -q -- "-----BEGIN PRIVATE KEY-----"; then
  printf '%s\n' "$APPLE_API_KEY_CONTENT" > "$file"
elif printf '%s' "$APPLE_API_KEY_CONTENT" \
  | python3 -c 'import base64, sys; sys.stdout.buffer.write(base64.b64decode(sys.stdin.read(), validate=False))' \
    > "$file" 2>/dev/null \
  && grep -q -- "-----BEGIN PRIVATE KEY-----" "$file"; then
  :
else
  rm -f "$file"
  echo "::error::secret APPLE_API_KEY is neither a .p8 private key nor its base64 encoding"
  exit 1
fi

openssl pkey -in "$file" -noout 2>/dev/null \
  || { rm -f "$file"; echo "::error::secret APPLE_API_KEY is not a readable private key"; exit 1; }
echo "APPLE_API_KEY_PATH=$file" >> "${GITHUB_ENV:?}"
echo "App Store Connect API key written for key ID $APPLE_API_KEY_ID"
