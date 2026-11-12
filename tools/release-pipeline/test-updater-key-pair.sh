#!/usr/bin/env bash
# Tests .github/scripts/updater-key-pair.py, the signing check's key-pair
# verification, with throwaway keys made by the Tauri CLI (as the release
# key was): a matching pair passes; a wrong password, another key's public
# key, an unencrypted key and malformed input fail.
#
#   tools/release-pipeline/test-updater-key-pair.sh [<work dir>]
#
# Needs python3 and pnpm (`pnpm install` done). No real key is involved.
# ok and bad never fail, so `check && ok || bad` is an if-then-else here.
# shellcheck disable=SC2015
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
check="$root/.github/scripts/updater-key-pair.py"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/updater-key-pair.XXXXXX")}"
mkdir -p "$work"
work="$(cd "$work" && pwd)"
passed=0 failures=0
ok() {
  echo "ok   $*"
  passed=$((passed + 1))
}
bad() {
  echo "FAIL $*"
  failures=$((failures + 1))
}

generate() { # generate <name> <password>
  rm -f "$work/$1" "$work/$1.pub"
  pnpm --dir "$root" --filter @deckpilot/desktop exec tauri signer generate --ci -p "$2" -w "$work/$1" > /dev/null
}
generate one "pass word"
generate other "pass word"
generate empty ""

# run <key> <password> <public key file>
run() {
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$work/$1")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$2" \
    DECKPILOT_UPDATER_PUBKEY="$(cat "$work/$3")" python3 -I "$check" > "$work/out.log" 2>&1
}
expect() { # expect <what> <message> <run args...>
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

run one "pass word" one.pub && grep -q "^match" "$work/out.log" && ok "a matching key, password and public key" \
  || bad "matching pair: $(cat "$work/out.log")"
run empty "" empty.pub && ok "a key with an empty password" || bad "empty password: $(cat "$work/out.log")"
expect "a wrong password" "wrong password or key" one "wrong" one.pub
expect "another key's public key" "does not belong" one "pass word" other.pub
printf 'not base64!' > "$work/garbage"
expect "a malformed public key" "DECKPILOT_UPDATER_PUBKEY is not" one "pass word" garbage
expect "a malformed secret key" "TAURI_SIGNING_PRIVATE_KEY is not" garbage "pass word" one.pub
: > "$work/unset"
expect "no secret key" "TAURI_SIGNING_PRIVATE_KEY is not set" unset "" one.pub
# Nothing secret in the output.
run one "pass word" one.pub
if grep -q -F "pass word" "$work/out.log" || grep -q -F "$(cat "$work/one")" "$work/out.log"; then
  bad "the output contains key material"
else
  ok "the output contains no key material"
fi

echo
echo "$passed passed, $failures failed (work dir: $work)"
[ "$failures" -eq 0 ]
