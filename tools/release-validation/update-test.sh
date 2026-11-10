#!/usr/bin/env bash
# Reproducible update test between two locally built, signed versions (macOS).
#
#   tools/release-validation/update-test.sh <work-dir>
#
# Builds FROM_VERSION and TO_VERSION (default 0.5.0 -> 0.5.1) from the current
# commit in temporary worktrees, signs their updater artifacts with a
# throwaway key (never the release key), and compiles them with a loopback
# update endpoint (DECKPILOT_UPDATE_ENDPOINT; only http://127.0.0.1 is
# accepted). Then, with an isolated, password-protected configuration:
#
#   1. failure: a tampered update is served; it must be rejected and the
#      installed version and its agent must keep running;
#   2. success: the real update is served; the app must download it, verify
#      its signature, stop the agent, replace itself, restart, start the new
#      agent and open the same encrypted configuration (profiles, active
#      profile and project).
#
# KEY_MODE=system (default: password) tests the default protection instead:
# the agent of the old version creates the configuration with a random key in
# the Keychain (service "DeckPilot Development"), a restart of app and agent
# must open it from the Keychain, and after the update the new agent must
# open it from the Keychain without any prompt. Keychain access depends on a
# stable code identity, so this needs both builds signed with the same
# Developer ID: provide APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD and
# APPLE_SIGNING_IDENTITY (the builds are signed, never notarized).
# TEST_KEYCHAIN, if set, becomes the default keychain while the apps run.
#
# Everything stays in <work-dir>; the real configuration, production Keychain
# items and the release keys are not touched. Windows of the test app appear
# on screen.
set -euo pipefail

mkdir -p "${1:?usage: update-test.sh <work-dir>}"
work="$(cd "$1" && pwd)"
from="${FROM_VERSION:-0.5.0}"
to="${TO_VERSION:-0.5.1}"
port="${PORT:-8765}"
key_mode="${KEY_MODE:-password}"
signed=""
[ -n "${APPLE_CERTIFICATE:-}" ] && signed=1
# Never notarize test builds: no Apple submission.
unset APPLE_ID APPLE_PASSWORD APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
root="$(git rev-parse --show-toplevel)"
dev() { cargo run --quiet --manifest-path "$root/Cargo.toml" -p deckpilot-dev -- "$@"; }
log() { printf '\n== %s\n' "$*"; }
fail() { echo "FAILED: $*" >&2; exit 1; }

case "$work" in "$root"/*) fail "use a work directory outside the repository" ;; esac
[ "$key_mode" = system ] && [ -z "$signed" ] \
  && fail "KEY_MODE=system needs Developer ID signed builds (APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD, APPLE_SIGNING_IDENTITY)"
mkdir -p "$work"
target="$(rustc -vV | sed -n 's/^host: //p')"
case "$target" in
  aarch64-apple-darwin) platform=darwin-aarch64 ;;
  x86_64-apple-darwin) platform=darwin-x86_64 ;;
  *) fail "this script runs on macOS" ;;
esac

log "throwaway updater key"
rm -f "$work/updater.key" "$work/updater.key.pub"
pnpm --dir "$root" --filter @deckpilot/desktop exec tauri signer generate --ci -p "" -w "$work/updater.key" >/dev/null
TAURI_SIGNING_PRIVATE_KEY="$(cat "$work/updater.key")"
DECKPILOT_UPDATER_PUBKEY="$(cat "$work/updater.key.pub")"
export TAURI_SIGNING_PRIVATE_KEY DECKPILOT_UPDATER_PUBKEY
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
export DECKPILOT_UPDATE_ENDPOINT="http://127.0.0.1:$port/latest.json"

build() {
  local version="$1" tree="$work/src-$1"
  log "build $version"
  git -C "$root" worktree remove --force "$tree" 2>/dev/null || true
  git -C "$root" worktree add --detach "$tree" HEAD >/dev/null
  # The first `version` in Cargo.toml is [workspace.package]'s.
  perl -0pi -e "s/^version = \"[^\"]+\"/version = \"$version\"/m" "$tree/Cargo.toml"
  perl -0pi -e "s/\"version\": \"[^\"]+\"/\"version\": \"$version\"/" "$tree/apps/desktop/package.json" "$tree/apps/desktop/src-tauri/tauri.conf.json"
  mkdir -p "$work/target"
  ln -s "$work/target" "$tree/target"
  (
    cd "$tree"
    pnpm install --offline --frozen-lockfile >/dev/null
    cargo update --workspace --offline --quiet
    cargo run --quiet -p deckpilot-dev -- package -- \
      --config '{"plugins":{"updater":{"dangerousInsecureTransportProtocol":true}}}'
  ) > "$work/build-$version.log" 2>&1 || { tail -30 "$work/build-$version.log"; fail "build $version"; }
  rm -rf "$work/out-$version" && mkdir -p "$work/out-$version"
  cp -R "$work/target/release/bundle/macos/DeckPilot.app" "$work/out-$version/"
  cp "$work/target/release/bundle/macos/DeckPilot.app.tar.gz" "$work/target/release/bundle/macos/DeckPilot.app.tar.gz.sig" "$work/out-$version/"
  git -C "$root" worktree remove --force "$tree"
}

[ -d "$work/out-$from" ] && [ -z "${REBUILD:-}" ] || build "$from"
[ -d "$work/out-$to" ] && [ -z "${REBUILD:-}" ] || build "$to"

if [ -n "$signed" ]; then
  log "both builds carry the same code identity"
  for version in "$from" "$to"; do
    for binary in deckpilot-agent deckpilot-desktop; do
      codesign --verify --strict "$work/out-$version/DeckPilot.app/Contents/MacOS/$binary"
      details=$(codesign --display --verbose=2 "$work/out-$version/DeckPilot.app/Contents/MacOS/$binary" 2>&1)
      echo "$version $binary: $(echo "$details" | sed -n 's/^Identifier=//p') team=$(echo "$details" | sed -n 's/^TeamIdentifier=//p')"
    done
  done
  for binary in deckpilot-agent deckpilot-desktop; do
    a=$(codesign --display -r- "$work/out-$from/DeckPilot.app/Contents/MacOS/$binary" 2>&1 | grep designated)
    b=$(codesign --display -r- "$work/out-$to/DeckPilot.app/Contents/MacOS/$binary" 2>&1 | grep designated)
    [ "$a" = "$b" ] || fail "$binary has different designated requirements: $a / $b"
    echo "$binary: $a"
  done
fi

serve="$work/serve"
manifest() {
  local artifact="$1"
  python3 - "$to" "$platform" "http://127.0.0.1:$port/$artifact" "$work/out-$to/DeckPilot.app.tar.gz.sig" > "$serve/latest.json" <<'PY'
import json, sys
version, platform, url, sig = sys.argv[1:5]
print(json.dumps({"version": version, "notes": "Update test", "pub_date": "2026-01-01T00:00:00Z",
                  "platforms": {platform: {"signature": open(sig).read().strip(), "url": url}}}))
PY
}
rm -rf "$serve" && mkdir -p "$serve"
cp "$work/out-$to/DeckPilot.app.tar.gz" "$serve/DeckPilot.app.tar.gz"
cp "$work/out-$to/DeckPilot.app.tar.gz" "$serve/tampered.app.tar.gz"
printf 'tampered' >> "$serve/tampered.app.tar.gz"
python3 -m http.server --bind 127.0.0.1 "$port" --directory "$serve" > "$work/server.log" 2>&1 &
server=$!
app_pid=""
cleanup() {
  [ -n "$app_pid" ] && kill "$app_pid" 2>/dev/null || true
  pkill -f "$work/Applications/DeckPilot.app/Contents/MacOS/" 2>/dev/null || true
  kill "$server" 2>/dev/null || true
}
trap cleanup EXIT

app="$work/Applications/DeckPilot.app"
data="$work/data"
export DECKPILOT_DATA_DIR="$data"
if [ -n "${TEST_KEYCHAIN:-}" ]; then
  security default-keychain -d user -s "$TEST_KEYCHAIN"
fi
installed_version() { /usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist"; }
wait_for() { # <seconds> <description> <command...>
  local deadline=$((SECONDS + $1)) what="$2"; shift 2
  until "$@" >/dev/null 2>&1; do
    [ "$SECONDS" -lt "$deadline" ] || fail "timed out: $what"
    sleep 2
  done
}
status() { dev validation status "$data"; }
agent_version_is() { status | grep -q "^version=$1 .*security=Unlocked"; }
agent_running() { status; }
comparable() { status | sed -E 's/ pid=[0-9]+//; s/version=[^ ]+ //'; }
stop_apps() {
  pkill -f "$app/Contents/MacOS/deckpilot-desktop" 2>/dev/null || true
  pkill -f "$app/Contents/MacOS/deckpilot-agent" 2>/dev/null || true
  local deadline=$((SECONDS + 20))
  while pgrep -f "$app/Contents/MacOS/" >/dev/null && [ "$SECONDS" -lt "$deadline" ]; do sleep 1; done
}
forget_keys() { # Development keys created by this test, nothing else.
  [ -f "$data/deckpilot.vault.json" ] || return 0
  local id
  id=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("protection",{}).get("keyId",""))' "$data/deckpilot.vault.json")
  [ -n "$id" ] && security delete-generic-password -s "DeckPilot Development" -a "database-key-$id" >/dev/null 2>&1 || true
}
start_app() {
  "$app/Contents/MacOS/deckpilot-desktop" >> "$work/app.log" 2>&1 &
  app_pid=$!
}
# The configuration opens: with a system key by itself (from the Keychain),
# with a password when the user enters it (here: `validation unlock`).
opened() { # <version>
  wait_for 120 "agent of $1 is running" agent_running
  if [ "$key_mode" = password ]; then dev validation unlock "$data"; fi
  wait_for 60 "agent $1 opened the configuration" agent_version_is "$1"
  status | grep -q "waiting_for_key_store=false" || fail "the agent is waiting for the Keychain"
}

install_from() {
  stop_apps
  forget_keys
  rm -rf "$work/Applications" "$data" && mkdir -p "$work/Applications"
  cp -R "$work/out-$from/DeckPilot.app" "$app"
  if [ "$key_mode" = password ]; then
    dev validation seed "$data"
    start_app
    opened "$from"
  else
    # First start: the agent creates the configuration with a system key.
    start_app
    opened "$from"
    status | grep -q "protection=Some(SystemManaged)" || fail "not protected by a system key"
    dev validation populate "$data"
    # A restart of app and agent opens it from the Keychain by itself.
    stop_apps
    start_app
    opened "$from"
    echo "restart: opened from the Keychain without a password"
  fi
}

trap 'cleanup; forget_keys' EXIT

log "1. a tampered update is rejected ($key_mode)"
manifest tampered.app.tar.gz
install_from
echo "before: $(status)"
wait_for 180 "the app fetched the tampered update" grep -q "GET /tampered.app.tar.gz" "$work/server.log"
sleep 20
[ "$(installed_version)" = "$from" ] || fail "the tampered update was installed"
agent_version_is "$from" || fail "the agent stopped after a rejected update"
echo "rejected: installed $(installed_version), agent: $(status)"

log "2. the signed update is installed ($key_mode)"
manifest DeckPilot.app.tar.gz
: > "$work/server.log"
install_from
before=$(comparable)
contents_before=$(dev validation inventory "$data")
echo "before: $(status)"
echo "contents: $contents_before"
wait_for 300 "the app replaced itself with $to" sh -c "[ \"\$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' '$app/Contents/Info.plist')\" = '$to' ]"
echo "installed: $(installed_version)"
# The restarted app starts the new agent, which opens the configuration:
# from the Keychain (system key) or with the user's password.
opened "$to"
after=$(comparable)
contents_after=$(dev validation inventory "$data")
echo "after: $(status)"
echo "contents: $contents_after"
[ "$before" = "$after" ] || fail "the state differs after the update: $before vs $after"
[ "$contents_before" = "$contents_after" ] || fail "the contents differ after the update"
head -c 16 "$data/deckpilot.db" | grep -q "SQLite format 3" && fail "the database is not encrypted"
pgrep -fl "$app/Contents/MacOS/deckpilot-agent" | head -1
log "update test passed ($key_mode): $from -> $to, configuration preserved and encrypted"
