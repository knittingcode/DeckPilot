#!/usr/bin/env bash
# Builds and stages the plugin runtime that every installer contains
# (docs/marketplace/module-migration.md, section 4.5; docs/releasing.md,
# "Packaging the plugins"), before the agent is built and the bundler runs:
#
#   tools/ci/stage-plugin-runtime.sh <rust target>
#
# - the bundled first-party plugins of plugins/first-party/bundled.json
#   (plugins/first-party/build.sh) in target/bundled-plugins/bundled, which
#   tauri.plugins.conf.json ships as the resource bundled/; their pins file
#   is exported as DECKPILOT_BUNDLED_PINS (to $GITHUB_ENV when set, as a
#   native absolute path), so the agent built next is compiled with them;
# - the plugin host (plugins/runtime, a separate workspace) for <rust
#   target>, release profile, staged where the Tauri configuration expects
#   it: binaries/deckpilot-plugin-host-<target>[.exe] (an external binary,
#   tauri.plugins.conf.json), or on macOS binaries/macos/deckpilot-plugin-host
#   (copied into Contents/MacOS as it is, tauri.plugins.macos.conf.json),
#   to be signed with its entitlement by tools/ci/sign-macos-plugin-host.sh.
#
# Needs cargo with the wasm32-unknown-unknown target and <rust target>, and
# jq. Prints the staged paths.
set -euo pipefail

target="${1:?usage: stage-plugin-runtime.sh <rust target>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# Native absolute path (Windows: C:/..., which build scripts can read).
native() {
  if command -v cygpath > /dev/null; then cygpath -m "$1"; else echo "$1"; fi
}

plugins/first-party/build.sh --out target/bundled-plugins
pins="$(native "$root/target/bundled-plugins/bundled/pins.json")"
[ -f "$pins" ] || { echo "::error::plugins/first-party/build.sh wrote no $pins" >&2; exit 1; }

exe=""
case "$target" in *-windows-*) exe=".exe" ;; esac
cargo build --release --locked --manifest-path plugins/runtime/Cargo.toml \
  -p deckpilot-plugin-host --bin deckpilot-plugin-host \
  --target "$target" --target-dir target/plugin-runtime
host="target/plugin-runtime/$target/release/deckpilot-plugin-host$exe"
binaries=apps/desktop/src-tauri/binaries
case "$target" in
  *-apple-darwin) staged="$binaries/macos/deckpilot-plugin-host" ;;
  *) staged="$binaries/deckpilot-plugin-host-$target$exe" ;;
esac
mkdir -p "$(dirname "$staged")"
cp "$host" "$staged"
echo "staged plugin host: $staged"
echo "bundled plugins: target/bundled-plugins/bundled ($(jq '.plugins | length' "$pins" | tr -d '\r') plugins)"

if [ -n "${GITHUB_ENV:-}" ]; then
  echo "DECKPILOT_BUNDLED_PINS=$pins" >> "$GITHUB_ENV"
fi
echo "DECKPILOT_BUNDLED_PINS=$pins"
