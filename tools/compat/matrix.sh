#!/usr/bin/env bash
# Compatibility between this commit and an earlier release, with the real
# binaries of both (see docs/compatibility.md).
#
#   tools/compat/matrix.sh [<old tag>] [<work dir>]
#
# Builds the agent and the development CLI of <old tag> (default v0.5.0-2)
# in a separate worktree, and those of this commit, then runs each agent
# against each client on throwaway data directories created with the
# development password (no credential store is touched). It also checks
# that an old agent leaves a database of this commit untouched.
#
# Prints one line per case and exits non-zero if any case fails. Nothing is
# published and nothing outside <work dir> and the old worktree is written.
set -euo pipefail

OLD_TAG=${1:-v0.5.0-2}
WORK=${2:-$(mktemp -d "${TMPDIR:-/tmp}/deckpilot-compat.XXXXXX")}
ROOT=$(git rev-parse --show-toplevel)
OLD_TREE=${OLD_TREE:-"$WORK/old-tree"}
mkdir -p "$WORK"

log() { printf '%s\n' "$*" >&2; }
failures=0
result() {
  # result <case> <ok|FAIL> <detail>
  printf '%-58s %-4s %s\n' "$1" "$2" "$3"
  [ "$2" = ok ] || failures=$((failures + 1))
}

if [ ! -f "$OLD_TREE/Cargo.toml" ]; then
  # A build cache may already have restored the old tree's target folder;
  # it is kept aside while the worktree is created.
  if [ -d "$OLD_TREE/target" ]; then
    mv "$OLD_TREE/target" "$WORK/old-target"
  fi
  rm -rf "$OLD_TREE"
  git -C "$ROOT" worktree add --detach "$OLD_TREE" "$OLD_TAG" >&2
  if [ -d "$WORK/old-target" ]; then
    mv "$WORK/old-target" "$OLD_TREE/target"
  fi
fi
log "building $OLD_TAG"
(cd "$OLD_TREE" && cargo build --locked -q -p deckpilot-agent -p deckpilot-dev)
log "building this commit"
(cd "$ROOT" && cargo build --locked -q -p deckpilot-agent -p deckpilot-dev)
OLD_BIN="$OLD_TREE/target/debug"
NEW_BIN="$ROOT/target/debug"
OLD_VERSION=$("$OLD_BIN/deckpilot-agent" --version | awk '{print $NF}')
NEW_VERSION=$("$NEW_BIN/deckpilot-agent" --version | awk '{print $NF}')

pids=()
cleanup() {
  # `${pids[@]+...}`: an empty array is "unbound" for bash 3 (macOS).
  for pid in ${pids[@]+"${pids[@]}"}; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

# start_agent <bin dir> <data dir>: runs a headless agent with a simulated
# MK.2 and waits until it accepts connections.
start_agent() {
  DECKPILOT_DATA_DIR="$2" "$1/deckpilot-agent" --headless --mock-devices mk2 \
    >"$2.agent.log" 2>&1 &
  pids+=($!)
  for _ in $(seq 1 100); do
    [ -s "$2/agent.token" ] && return 0
    sleep 0.1
  done
  log "agent in $2 did not start"
  return 1
}
stop_agents() {
  cleanup
  pids=()
}
status_of() { "$1/deckpilot-dev" validation status "$2" 2>&1 | head -1; }

old_data="$WORK/old-data"
new_data="$WORK/new-data"
rm -rf "$old_data" "$new_data"
"$OLD_BIN/deckpilot-dev" validation seed "$old_data" >/dev/null
"$NEW_BIN/deckpilot-dev" validation seed "$new_data" >/dev/null

# 1. Old agent with old client, and with the client of this commit.
start_agent "$OLD_BIN" "$old_data"
"$OLD_BIN/deckpilot-dev" validation unlock "$old_data" >/dev/null
status=$(status_of "$OLD_BIN" "$old_data")
case "$status" in
  "version=$OLD_VERSION "*) result "$OLD_VERSION client  -> $OLD_VERSION agent" ok "$status" ;;
  *) result "$OLD_VERSION client  -> $OLD_VERSION agent" FAIL "$status" ;;
esac
if (cd "$ROOT" && DECKPILOT_COMPAT_DATA_DIR="$old_data" \
  DECKPILOT_COMPAT_AGENT_VERSION="$OLD_VERSION" DECKPILOT_COMPAT_AGENT_PROTOCOL=4 \
  cargo test --locked -q -p deckpilot-ipc --test real_agent -- --nocapture \
  >"$WORK/new-client-old-agent.log" 2>&1); then
  result "$NEW_VERSION client -> $OLD_VERSION agent (negotiated)" ok \
    "$(grep -o 'protocol [0-9]*, capabilities.*' "$WORK/new-client-old-agent.log")"
else
  result "$NEW_VERSION client -> $OLD_VERSION agent (negotiated)" FAIL "see $WORK/new-client-old-agent.log"
fi
stop_agents

# 2. Agent of this commit with its own client and with the old client.
start_agent "$NEW_BIN" "$new_data"
"$NEW_BIN/deckpilot-dev" validation unlock "$new_data" >/dev/null
status=$(status_of "$NEW_BIN" "$new_data")
case "$status" in
  "version=$NEW_VERSION "*) result "$NEW_VERSION client -> $NEW_VERSION agent" ok "$status" ;;
  *) result "$NEW_VERSION client -> $NEW_VERSION agent" FAIL "$status" ;;
esac
status=$(status_of "$OLD_BIN" "$new_data")
case "$status" in
  "version=$NEW_VERSION "*) result "$OLD_VERSION client  -> $NEW_VERSION agent (protocol 4)" ok "$status" ;;
  *) result "$OLD_VERSION client  -> $NEW_VERSION agent (protocol 4)" FAIL "$status" ;;
esac
stop_agents

# 3. An old agent given a database of this commit (an old app restarting
#    "its" agent after the newer one ran): it must refuse to open it and
#    change nothing.
before=$(cd "$new_data" && shasum -a 256 deckpilot.db deckpilot.vault.json | shasum -a 256)
start_agent "$OLD_BIN" "$new_data"
unlock=$("$OLD_BIN/deckpilot-dev" validation unlock "$new_data" 2>&1 || true)
sleep 1
stop_agents
after=$(cd "$new_data" && shasum -a 256 deckpilot.db deckpilot.vault.json | shasum -a 256)
backups=$(find "$new_data" -path '*backups*' -type f | wc -l | tr -d ' ')
if [ "$before" = "$after" ] && ! grep -q "agent unlocked" <<<"$unlock"; then
  result "$OLD_VERSION agent  -> $NEW_VERSION database" ok \
    "refused, unchanged ($(tr '\n' ' ' <<<"$unlock" | cut -c1-90))"
else
  result "$OLD_VERSION agent  -> $NEW_VERSION database" FAIL \
    "changed=$([ "$before" = "$after" ] && echo no || echo yes) backups=$backups: $unlock"
fi
# The agent of this commit still opens it afterwards.
start_agent "$NEW_BIN" "$new_data"
"$NEW_BIN/deckpilot-dev" validation unlock "$new_data" >/dev/null
status=$(status_of "$NEW_BIN" "$new_data")
case "$status" in
  *"profiles=3"*) result "$NEW_VERSION agent  -> $NEW_VERSION database afterwards" ok "$status" ;;
  *) result "$NEW_VERSION agent  -> $NEW_VERSION database afterwards" FAIL "$status" ;;
esac
stop_agents

# 4. The agent of this commit upgrades the old release's database.
start_agent "$NEW_BIN" "$old_data"
"$NEW_BIN/deckpilot-dev" validation unlock "$old_data" >/dev/null
status=$(status_of "$NEW_BIN" "$old_data")
case "$status" in
  *"profiles=3"*"active_project=SHOP"*) result "$NEW_VERSION agent  -> $OLD_VERSION database (upgrade)" ok "$status" ;;
  *) result "$NEW_VERSION agent  -> $OLD_VERSION database (upgrade)" FAIL "$status" ;;
esac
stop_agents

log "work directory: $WORK"
exit "$failures"
