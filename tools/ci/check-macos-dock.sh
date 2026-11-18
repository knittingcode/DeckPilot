#!/usr/bin/env bash
# check-macos-dock.sh APP
#
# The Dock presence of the packaged DeckPilot.app (docs/background-agent.md,
# "Windows, the Dock and the three quits"), on a CI runner:
#   1. the bundled agent is a menu bar (UIElement) application from the
#      first moment LaunchServices knows it: it never registers as a
#      Foreground (Dock) application, not even briefly at startup;
#   2. a second agent stops at the instance lock and never appears at all;
#   3. the configuration app registers as a Foreground (Dock) application
#      when it is launched.
# Closing the window to the menu bar and reopening it need a person (or UI
# scripting, which hosted runners do not allow): see
# docs/release-validation.md, "macOS Dock and window lifecycle".
#
# Isolated: its own DECKPILOT_DATA_DIR and a throwaway default keychain, so
# it is only meant for CI runners. Refuses to run outside CI.
set -euo pipefail
app=${1:?usage: check-macos-dock.sh APP}
[ "${CI:-}" = true ] || { echo "error: CI only (it replaces the default keychain)" >&2; exit 2; }
[ -x "$app/Contents/MacOS/deckpilot-agent" ] && [ -x "$app/Contents/MacOS/deckpilot-desktop" ] ||
  { echo "error: $app has no deckpilot-agent and deckpilot-desktop" >&2; exit 2; }

work=$(mktemp -d "${RUNNER_TEMP:-/tmp}/dock.XXXXXX")
export DECKPILOT_DATA_DIR=$work/data
mkdir -p "$DECKPILOT_DATA_DIR"
keychain=$work/dock.keychain-db
security create-keychain -p dock "$keychain"
security unlock-keychain -p dock "$keychain"
previous=$(security default-keychain -d user | tr -d ' "')
security default-keychain -d user -s "$keychain"
pids=()
cleanup() {
  for pid in "${pids[@]+"${pids[@]}"}"; do kill "$pid" 2>/dev/null || true; done
  sleep 1
  for pid in "${pids[@]+"${pids[@]}"}"; do kill -9 "$pid" 2>/dev/null || true; done
  [ -z "$previous" ] || security default-keychain -d user -s "$previous" || true
  security delete-keychain "$keychain" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

failed=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; failed=1; }

# type PID: the LaunchServices application type of a process, or nothing.
type_of() {
  local asn
  asn=$(lsappinfo find "pid=$1" 2>/dev/null) || return 0
  [ -n "$asn" ] || return 0
  lsappinfo info "$asn" 2>/dev/null | grep -o -m1 'type="[A-Za-z]*"' | sed 's/type="\(.*\)"/\1/'
}

# watch PID SECONDS: every application type the process had, one per line.
watch_types() {
  local pid=$1 until=$((SECONDS + $2)) t
  while [ "$SECONDS" -lt "$until" ] && kill -0 "$pid" 2>/dev/null; do
    t=$(type_of "$pid")
    [ -z "$t" ] || echo "$t"
    sleep 0.05
  done | sort -u
}

# 1. The agent.
"$app/Contents/MacOS/deckpilot-agent" > "$work/agent.log" 2>&1 &
agent=$!
pids+=("$agent")
types=$(watch_types "$agent" 12)
echo "agent application types: ${types:-none}"
if ! kill -0 "$agent" 2>/dev/null; then
  bad "the agent stopped: $(tail -3 "$work/agent.log")"
elif grep -qx Foreground <<< "$types"; then
  bad "the agent registered as a Foreground (Dock) application"
elif grep -qx UIElement <<< "$types"; then
  ok "the agent is a menu bar (UIElement) application from its first moment"
else
  bad "the agent never registered with LaunchServices (types: ${types:-none})"
fi

# 2. A second agent stops at the instance lock without appearing.
"$app/Contents/MacOS/deckpilot-agent" > "$work/agent2.log" 2>&1 &
second=$!
pids+=("$second")
types=$(watch_types "$second" 30)
if kill -0 "$second" 2>/dev/null; then
  bad "a second agent is still running"
elif grep -qx Foreground <<< "$types"; then
  bad "a second agent showed a Dock icon before it stopped"
else
  ok "a second agent stopped at the instance lock (types: ${types:-none})"
fi

# 3. The configuration app belongs in the Dock when launched.
DECKPILOT_NO_AGENT_AUTOSTART=1 "$app/Contents/MacOS/deckpilot-desktop" > "$work/desktop.log" 2>&1 &
desktop=$!
pids+=("$desktop")
types=$(watch_types "$desktop" 15)
echo "app application types: ${types:-none}"
if grep -qx Foreground <<< "$types"; then
  ok "the configuration app registers as a Foreground (Dock) application"
else
  bad "the configuration app did not register as a Foreground application (types: ${types:-none}; $(tail -3 "$work/desktop.log"))"
fi

[ "$failed" -eq 0 ] || exit 1
echo "Dock presence of the packaged app: as documented"
