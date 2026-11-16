#!/usr/bin/env bash
# Builds the HID probe, signs it ad hoc in four variants and runs each:
#
#   unsandboxed-direct  bare binary, hardened runtime, no entitlements (control)
#   sandbox-usb-open    .app with App Sandbox + device.usb, started by
#                       LaunchServices (`open`), so it is its own responsible
#                       process for TCC
#   sandbox-usb-direct  the same .app's binary started from this shell
#   sandbox-only-open   .app with App Sandbox only (control for device.usb)
#
#   experiments/distribution/macos-sandbox-hid/run.sh <work dir> [<results dir>]
#
# The reports go to <results dir> (default: results/ next to this script).
# Uses only ad hoc signatures and experiment bundle identifiers: no Apple
# account, no DeckPilot identifier, nothing installed. Build with
# tools/dev/target-slot.sh run -- ... to share a target directory.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=${1:?usage: run.sh <work dir> [<results dir>]}
results=${2:-$here/results}
mkdir -p "$work" "$results"
work=$(cd "$work" && pwd)

[ "$(uname -s)" = Darwin ] || { echo "macOS only" >&2; exit 1; }

CARGO_INCREMENTAL=0 cargo build --release --locked --manifest-path "$here/Cargo.toml"
target_dir=${CARGO_TARGET_DIR:-$here/target}
binary="$target_dir/release/macos-sandbox-hid"

{
  echo "date = $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "macos = $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
  echo "arch = $(uname -m)"
  echo "hidapi = 2.6.7 (default features)"
  echo "--- ioreg -p IOUSB (USB devices attached)"
  ioreg -p IOUSB
  echo "--- hidutil list (Product column, unique)"
  hidutil list 2>/dev/null | awk 'NR > 2 { print }' | sed -E 's/ +/ /g' | cut -c1-160 | sort -u | head -80
} > "$results/environment.txt"

bundle() { # <name> <entitlements or ""> -> path of the signed .app
  local name=$1 entitlements=$2 app="$work/$1.app"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS"
  sed "s/@SUFFIX@/$name/" "$here/Info.plist" > "$app/Contents/Info.plist"
  cp "$binary" "$app/Contents/MacOS/macos-sandbox-hid"
  if [ -n "$entitlements" ]; then
    codesign --force --sign - --options runtime --entitlements "$entitlements" "$app" >&2
  else
    codesign --force --sign - --options runtime "$app" >&2
  fi
  printf '%s\n' "$app"
}

signature() { # <path> -> codesign summary lines
  codesign -d --verbose=2 "$1" 2>&1 | grep -E '^(Identifier|Format|CodeDirectory|Signature)=' || true
  echo "entitlements:"
  codesign -d --entitlements - --xml "$1" 2>/dev/null | plutil -p - 2>/dev/null || echo "  (none)"
}

# Every probe run is bounded: a TCC consent prompt must never leave the
# script waiting.
bounded() { # <seconds> <command...>
  local seconds=$1; shift
  "$@" & local pid=$!
  ( sleep "$seconds"; kill "$pid" 2>/dev/null && echo "timeout after ${seconds}s" ) &
  local watchdog=$!
  local status=0
  wait "$pid" || status=$?
  kill "$watchdog" 2>/dev/null || true
  wait "$watchdog" 2>/dev/null || true
  return "$status"
}

run_open() { # <variant> <app>
  local out="$work/$1.out" err="$work/$1.err"
  rm -f "$out" "$err"
  bounded 60 open -W -n --stdout "$out" --stderr "$err" "$2" --args "$1" || echo "open: exit status $?" >> "$err"
  { signature "$2"; echo "--- report"; cat "$out" 2>/dev/null; echo "--- stderr"; cat "$err" 2>/dev/null; } > "$results/$1.txt"
}

run_direct() { # <variant> <executable>
  { signature "$2"; echo "--- report"; bounded 60 "$2" "$1" 2>&1 || echo "exit status $?"; } > "$results/$1.txt"
}

plain="$work/unsandboxed-direct"
cp "$binary" "$plain"
codesign --force --sign - --options runtime "$plain"
run_direct unsandboxed-direct "$plain"

app=$(bundle sandbox-usb "$here/entitlements/sandbox-usb.plist")
run_open sandbox-usb-open "$app"
run_direct sandbox-usb-direct "$app/Contents/MacOS/macos-sandbox-hid"

app=$(bundle sandbox-only "$here/entitlements/sandbox-only.plist")
run_open sandbox-only-open "$app"

# The reports are committed: the work and home directories become
# placeholders.
for report in "$results"/*.txt; do
  sed -i.bak -e "s#$work#<work>#g" -e "s#$HOME#~#g" "$report" && rm -f "$report.bak"
done

for report in "$results"/*.txt; do
  case $report in */environment.txt) continue ;; esac
  grep -q '^end$' "$report" || echo "warning: $report is incomplete" >&2
  echo "== $(basename "$report")"
  grep -E '^(label|app_sandbox_container_id|read_dir|input_monitoring|hid_init|hid_devices|elgato_devices|timeout|exit status)' "$report" || true
done
