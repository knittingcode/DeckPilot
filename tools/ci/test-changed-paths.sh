#!/usr/bin/env bash
# Tests changed-paths.sh: which optional jobs a pull request's files turn on.
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/changed-paths.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/changed-paths-test.XXXXXX")
trap 'rm -rf "$work"' EXIT
failures=0

# expect "<expected outputs, sorted>" <changed file>...
expect() {
  local expected="$1"
  shift
  printf '%s\n' "$@" > "$work/changed"
  local actual
  : > "$work/out"
  GITHUB_OUTPUT="$work/out" "$script" "$work/changed" 2> /dev/null
  actual=$(sort "$work/out" | tr '\n' ' ')
  if [ "$actual" = "$expected " ]; then
    echo "ok   $*"
  else
    echo "FAIL $*: got '$actual', expected '$expected'"
    failures=$((failures + 1))
  fi
}

none="keystore=false packaging=false release_manifest=false"
expect "$none" apps/desktop/src/App.tsx
expect "$none" crates/deckpilot-core/src/lib.rs docs/development.md
expect "$none" apps/agent/src/main.rs
expect "$none" apps/desktop/src-tauri/src/updates.rs
expect "keystore=false packaging=true release_manifest=false" apps/agent/build.rs
expect "keystore=false packaging=true release_manifest=false" apps/agent/Info.plist
expect "keystore=false packaging=true release_manifest=false" apps/desktop/src-tauri/tauri.release.conf.json
expect "keystore=false packaging=true release_manifest=false" apps/desktop/src-tauri/Cargo.toml
expect "keystore=false packaging=true release_manifest=false" apps/desktop/src-tauri/icons/icon.ico
expect "keystore=false packaging=true release_manifest=false" apps/desktop/src-tauri/capabilities/default.json
expect "keystore=false packaging=true release_manifest=false" apps/desktop/package.json
expect "keystore=false packaging=true release_manifest=false" apps/desktop/src-tauri/tauri.conf.json
expect "keystore=false packaging=true release_manifest=false" Cargo.lock
expect "keystore=false packaging=true release_manifest=false" packaging/linux/70-deckpilot-streamdeck.rules
expect "keystore=true packaging=false release_manifest=false" crates/deckpilot-keystore/src/lib.rs
expect "keystore=false packaging=true release_manifest=true" tools/ci/check-packages.sh
expect "keystore=false packaging=true release_manifest=true" .github/workflows/release.yml
expect "keystore=true packaging=true release_manifest=true" .github/workflows/ci.yml
expect "keystore=false packaging=true release_manifest=false" .github/workflows/compatibility.yml
# Look-alikes do not count.
expect "$none" crates/deckpilot-keystore-extra/x.rs tools/cix/a.sh xCargo.lock apps/desktop/src-tauri/src/tauri.rs

# Pushes run everything.
: > "$work/push"
GITHUB_OUTPUT="$work/push" EVENT_NAME=push "$script" 2> /dev/null
actual=$(sort "$work/push" | tr '\n' ' ')
if [ "$actual" = "keystore=true packaging=true release_manifest=true " ]; then
  echo "ok   push to main runs every optional job"
else
  echo "FAIL push: got '$actual'"
  failures=$((failures + 1))
fi

[ "$failures" -eq 0 ] || { echo "$failures case(s) failed"; exit 1; }
echo "changed paths: all cases passed"
