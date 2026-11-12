# shellcheck shell=bash
# Shared by the integration preview scripts (preview/integration only).
set -euo pipefail
ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"
[ "$(git rev-parse --abbrev-ref HEAD)" = preview/integration ] || {
  echo "not on preview/integration; refusing" >&2; exit 1; }
[ -e "$ROOT/.git" ] && [ -f "$ROOT/.git" ] || {
  echo "the preview must run from its own worktree, not the main checkout" >&2; exit 1; }
DEV="$ROOT/target/debug/deckpilot-dev"
DATA="$ROOT/.dev/data"          # isolated: socket, lock, token and logs follow it
INFO="$ROOT/.dev/preview-info.json"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
export DECKPILOT_VITE_PORT=1430
export DECKPILOT_TAURI_CONFIG="$ROOT/apps/desktop/src-tauri/tauri.preview.conf.json"
unset DECKPILOT_UPDATER_PUBKEY   # no updater key: updates cannot be checked or installed
dev() { cargo run -q --locked -p deckpilot-dev -- "$@"; }

write_info() {
  # write_info <base sha>
  local included excluded
  included=$(awk '$1=="include"{printf "%s%s", sep, $2; sep=","}' tools/preview/prs.txt)
  excluded=$(awk '$1=="exclude"{n=$2; $1=$2=""; sub(/^ +/,""); gsub(/"/,"\\\""); printf "%s{\"pr\":%s,\"reason\":\"%s\"}", sep, n, $0; sep=","}' tools/preview/prs.txt)
  printf '{"base":"%s","included":[%s],"excluded":[%s],"built":"%s","version":"%s-preview"}\n' \
    "$1" "$included" "$excluded" "$(date '+%Y-%m-%d %H:%M')" \
    "$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)" > "$INFO"
}
