#!/usr/bin/env bash
# The required check "Updater signing keys match", without any secret:
#
# 1. The updater public key pinned in .github/updater.pub is a well-formed
#    Ed25519 minisign key, and the repository variable
#    DECKPILOT_UPDATER_PUBKEY (compiled into release builds) is that key.
#    The variable must be set unless DECKPILOT_UPDATER_PUBKEY_OPTIONAL is
#    true (pull requests from forks may not see repository variables).
# 2. updater-key-pair.py, which the release workflow runs against the real
#    signing key in the protected environment, accepts and refuses the
#    committed throwaway fixtures (.github/scripts/fixtures, test only).
#
#   .github/scripts/updater-key-pair-fixtures.sh
#
# Needs bash and python3 (standard library) only: no dependency
# installation, no build, nothing secret.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
check="$root/.github/scripts/updater-key-pair.py"
fixtures="$root/.github/scripts/fixtures"
pinned="$root/.github/updater.pub"
work="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/updater-key-pair.XXXXXX")"
trap 'rm -rf "$work"' EXIT
passed=0 failures=0
ok() {
  echo "ok   $*"
  passed=$((passed + 1))
}
bad() {
  echo "FAIL $*"
  failures=$((failures + 1))
}

# 1. The pinned public key and the repository variable.
require=--require-variable
[ "${DECKPILOT_UPDATER_PUBKEY_OPTIONAL:-false}" = true ] && require=--public-only
if python3 -I "$check" --pinned "$pinned" --public-only "$require" > "$work/out.log" 2>&1; then
  ok "$(cat "$work/out.log")"
  [ -n "${DECKPILOT_UPDATER_PUBKEY:-}" ] || echo "::notice::DECKPILOT_UPDATER_PUBKEY is not visible here; only the pinned key was checked"
else
  bad "the pinned public key: $(cat "$work/out.log")"
fi

# 2. The key-pair verification with throwaway keys.
# run <key file> <password> <public key file> [options...]
run() {
  local key="$1" password="$2" public="$3"
  shift 3
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$key")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$password" \
    DECKPILOT_UPDATER_PUBKEY="$(cat "$public")" python3 -I "$check" "$@" > "$work/out.log" 2>&1
}
accepts() { # accepts <what> <run args...>
  local what="$1"
  shift
  if run "$@" && grep -q "^match" "$work/out.log"; then
    ok "$what"
  else
    bad "$what: $(cat "$work/out.log")"
  fi
}
refuses() { # refuses <what> <message> <run args...>
  local what="$1" message="$2"
  shift 2
  if run "$@"; then
    bad "$what: accepted"
  elif grep -q -F -- "$message" "$work/out.log"; then
    ok "$what"
  else
    bad "$what: $(cat "$work/out.log")"
  fi
}

one="$fixtures/throwaway-updater.key"
other="$fixtures/other-updater.key"
password="throwaway fixture password"
: > "$work/empty"
printf 'not base64!' > "$work/garbage"

accepts "a matching key, password and public key" "$one" "$password" "$one.pub"
accepts "a key with an empty password" "$other" "" "$other.pub"
accepts "a key that matches the pinned file" "$one" "$password" "$one.pub" --pinned "$one.pub" --require-variable
refuses "a wrong password" "wrong password or key" "$one" "wrong" "$one.pub"
refuses "another key's public key" "does not belong" "$one" "$password" "$other.pub"
refuses "a throwaway key against the pinned release key" "does not belong to $pinned" "$one" "$password" "$work/empty" --pinned "$pinned"
refuses "a variable other than the pinned key" "is not the public key pinned in" "$one" "$password" "$one.pub" --pinned "$pinned"
refuses "no variable where it is required" "DECKPILOT_UPDATER_PUBKEY is not set" "$one" "$password" "$work/empty" --pinned "$one.pub" --require-variable
refuses "a malformed public key" "DECKPILOT_UPDATER_PUBKEY is not" "$one" "$password" "$work/garbage"
refuses "a malformed secret key" "TAURI_SIGNING_PRIVATE_KEY is not" "$work/garbage" "$password" "$one.pub"
refuses "no secret key" "TAURI_SIGNING_PRIVATE_KEY is not set" "$work/empty" "" "$one.pub"
# Nothing secret in the output.
run "$one" "$password" "$one.pub" || true
if grep -q -F "$password" "$work/out.log" || grep -q -F "$(cat "$one")" "$work/out.log"; then
  bad "the output contains key material"
else
  ok "the output contains no key material"
fi

echo
echo "$passed passed, $failures failed"
[ "$failures" -eq 0 ]
