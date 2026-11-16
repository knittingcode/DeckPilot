#!/usr/bin/env bash
# Builds the plugin host probe and runs it signed (ad hoc) in these variants:
#
#   direct-plain                no hardened runtime, no entitlements (control)
#   direct-hardened             hardened runtime, no entitlements
#   direct-hardened-allow-jit   hardened runtime + cs.allow-jit
#   direct-hardened-unsigned    hardened runtime + cs.allow-unsigned-executable-memory
#   sandboxed-parent-*          a sandboxed .app started by LaunchServices
#                               (`open`) spawns the child from Contents/MacOS:
#     ...-child-inherit               child: app-sandbox + inherit, hardened runtime
#     ...-child-inherit-allow-jit     child: app-sandbox + inherit + allow-jit, hardened runtime
#     ...-child-inherit-unsigned      child: app-sandbox + inherit +
#                                     allow-unsigned-executable-memory, hardened runtime
#     ...-child-inherit-no-runtime    child: app-sandbox + inherit, no hardened runtime
#                                     (the Mac App Store does not require it)
#
#   experiments/distribution/macos-sandbox-plugin-host/run.sh <work dir> [<results dir>]
#
# Ad hoc signatures and experiment bundle identifiers only: no Apple account,
# no DeckPilot identifier, nothing installed.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=${1:?usage: run.sh <work dir> [<results dir>]}
results=${2:-$here/results}
mkdir -p "$work" "$results"
work=$(cd "$work" && pwd)
[ "$(uname -s)" = Darwin ] || { echo "macOS only" >&2; exit 1; }

CARGO_INCREMENTAL=0 cargo build --release --locked --manifest-path "$here/Cargo.toml"
binary="${CARGO_TARGET_DIR:-$here/target}/release/macos-sandbox-plugin-host"
ents="$here/entitlements"

{
  echo "date = $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "macos = $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
  echo "arch = $(uname -m)"
  echo "wasmtime = 50.0.1 (cranelift, runtime, std)"
} > "$results/environment.txt"

# A TCC prompt or a hung child must never leave the script waiting.
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

sign() { # <path> <runtime: yes|no> [<entitlements>]
  local options=()
  [ "$2" = yes ] && options+=(--options runtime)
  [ -n "${3-}" ] && options+=(--entitlements "$3")
  codesign --force --sign - ${options[@]+"${options[@]}"} "$1" 2>/dev/null
}

signature() { # <path>
  codesign -d --verbose=2 "$1" 2>&1 | grep -E '^(Identifier|CodeDirectory|Signature)=' || true
  echo "entitlements:"
  codesign -d --entitlements - --xml "$1" 2>/dev/null | plutil -p - 2>/dev/null || echo "  (none)"
}

direct() { # <variant> <runtime> [<entitlements>]
  local exe="$work/$1"
  cp "$binary" "$exe"
  sign "$exe" "$2" "${3-}"
  {
    signature "$exe"
    echo "--- report"
    bounded 60 "$exe" child "$1" 2>&1 || echo "exit status $?"
  } > "$results/$1.txt"
}

sandboxed() { # <variant> <child runtime> <child entitlements>
  local app="$work/$1.app" out="$work/$1.out" err="$work/$1.err"
  rm -rf "$app" "$out" "$err"
  mkdir -p "$app/Contents/MacOS"
  sed "s/@SUFFIX@/$1/" "$here/Info.plist" > "$app/Contents/Info.plist"
  cp "$binary" "$app/Contents/MacOS/macos-sandbox-plugin-host"
  cp "$binary" "$app/Contents/MacOS/plugin-host-child"
  # Inside out: the helper first, then the bundle (its main executable).
  sign "$app/Contents/MacOS/plugin-host-child" "$2" "$3"
  sign "$app" yes "$ents/sandbox-parent.plist"
  bounded 90 open -W -n --stdout "$out" --stderr "$err" "$app" \
    --args parent "$app/Contents/MacOS/plugin-host-child" "$1" || echo "open: exit status $?" >> "$err"
  {
    echo "== parent"; signature "$app"
    echo "== child"; signature "$app/Contents/MacOS/plugin-host-child"
    echo "--- report"; cat "$out" 2>/dev/null
    echo "--- stderr"; cat "$err" 2>/dev/null
  } > "$results/$1.txt"
}

direct direct-plain no
direct direct-hardened yes
direct direct-hardened-allow-jit yes "$ents/allow-jit.plist"
direct direct-hardened-unsigned yes "$ents/allow-unsigned-executable-memory.plist"
sandboxed sandboxed-parent-child-inherit yes "$ents/sandbox-child.plist"
sandboxed sandboxed-parent-child-inherit-allow-jit yes "$ents/sandbox-child-allow-jit.plist"
sandboxed sandboxed-parent-child-inherit-unsigned yes "$ents/sandbox-child-allow-unsigned-executable-memory.plist"
sandboxed sandboxed-parent-child-inherit-no-runtime no "$ents/sandbox-child.plist"

# Why a SIGKILLed child died: the fields of its newest macOS crash report.
crash_excerpt() { # <process name> <output>
  local report
  # shellcheck disable=SC2012 # crash report names have no special characters
  report=$(ls -t "$HOME"/Library/Logs/DiagnosticReports/"$1"-*.ips 2>/dev/null | head -1) || true
  [ -n "$report" ] || return 0
  python3 -I -c '
import json, sys
head, body = open(sys.argv[1]).read().split("\n", 1)
h, d = json.loads(head), json.loads(body)
print("source = macOS crash report of", sys.argv[2], "(fields excerpted)")
print("os_version =", h.get("os_version"))
for key in ("type", "signal"):
    print(f"exception.{key} =", d["exception"][key])
for key in ("namespace", "indicator"):
    print(f"termination.{key} =", d["termination"][key])
print("vmRegionInfo:")
print(d.get("vmRegionInfo", ""))
' "$report" "$1" > "$2"
}
sleep 5 # ReportCrash writes the report shortly after the process died
crash_excerpt direct-hardened-allow-jit "$results/crash-direct-hardened-allow-jit.txt"

# The reports are committed: the work and home directories become
# placeholders.
for report in "$results"/*.txt; do
  sed -i.bak -e "s#$work#<work>#g" -e "s#$HOME#~#g" "$report" && rm -f "$report.bak"
done

for report in "$results"/*.txt; do
  case $report in */environment.txt|*/crash-*.txt) continue ;; esac
  echo "== $(basename "$report" .txt)"
  grep -E '(app_sandbox_container_id|read_dir|spawn|engine|jit_|sandbox_init|timeout|exit status)' "$report" || true
done
