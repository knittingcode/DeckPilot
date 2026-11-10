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
# Everything stays in <work-dir>; the real configuration, the Keychain and the
# release keys are not touched. Windows of the test app appear on screen.
set -euo pipefail

mkdir -p "${1:?usage: update-test.sh <work-dir>}"
work="$(cd "$1" && pwd)"
from="${FROM_VERSION:-0.5.0}"
to="${TO_VERSION:-0.5.1}"
port="${PORT:-8765}"
root="$(git rev-parse --show-toplevel)"
dev() { cargo run --quiet --manifest-path "$root/Cargo.toml" -p deckpilot-dev -- "$@"; }
log() { printf '\n== %s\n' "$*"; }
fail() { echo "FAILED: $*" >&2; exit 1; }

case "$work" in "$root"/*) fail "use a work directory outside the repository" ;; esac
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
installed_version() { /usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist"; }
wait_for() { # <seconds> <description> <command...>
  local deadline=$((SECONDS + $1)) what="$2"; shift 2
  until "$@" >/dev/null 2>&1; do
    [ "$SECONDS" -lt "$deadline" ] || fail "timed out: $what"
    sleep 2
  done
}
agent_version_is() { dev validation status "$data" | grep -q "^version=$1 .*security=Unlocked"; }
agent_running() { dev validation status "$data"; }
unlock_when_running() { wait_for 120 "agent of $1 is running" agent_running; dev validation unlock "$data"; wait_for 60 "agent $1 unlocked" agent_version_is "$1"; }

install_from() {
  pkill -f "$app/Contents/MacOS/" 2>/dev/null || true
  rm -rf "$work/Applications" "$data" && mkdir -p "$work/Applications"
  cp -R "$work/out-$from/DeckPilot.app" "$app"
  dev validation seed "$data"
  "$app/Contents/MacOS/deckpilot-desktop" >> "$work/app.log" 2>&1 &
  app_pid=$!
  unlock_when_running "$from"
}

log "1. a tampered update is rejected"
manifest tampered.app.tar.gz
install_from
before=$(dev validation status "$data")
echo "before: $before"
wait_for 180 "the app fetched the tampered update" grep -q "GET /tampered.app.tar.gz" "$work/server.log"
sleep 20
[ "$(installed_version)" = "$from" ] || fail "the tampered update was installed"
agent_version_is "$from" || fail "the agent stopped after a rejected update"
echo "rejected: installed $(installed_version), agent: $(dev validation status "$data")"

log "2. the signed update is installed"
manifest DeckPilot.app.tar.gz
install_from
before=$(dev validation status "$data" | sed -E 's/ pid=[0-9]+//; s/version=[^ ]+ //')
echo "before: $(dev validation status "$data")"
wait_for 300 "the app replaced itself with $to" sh -c "[ \"\$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' '$app/Contents/Info.plist')\" = '$to' ]"
echo "installed: $(installed_version)"
# The restarted app starts the new agent; the user enters the password.
unlock_when_running "$to"
after=$(dev validation status "$data" | sed -E 's/ pid=[0-9]+//; s/version=[^ ]+ //')
echo "after: $(dev validation status "$data")"
[ "$before" = "$after" ] || fail "configuration differs after the update: $before vs $after"
head -c 16 "$data/deckpilot.db" | grep -q "SQLite format 3" && fail "the database is not encrypted"
codesign --verify --deep "$app" 2>/dev/null || echo "note: local builds are ad-hoc signed (expected)"
pgrep -fl "$app/Contents/MacOS/deckpilot-agent" | head -1
log "update test passed: $from -> $to, configuration preserved and encrypted"
