#!/usr/bin/env bash
# macOS: the plugin host under the hardened runtime, as a Developer ID
# package built with DECKPILOT_RELEASE_PLUGINS=1 signs it
# (docs/marketplace/distribution.md, "macOS: the plugin host's
# entitlement"). Signs the host and the sandbox probe ad hoc in place in the
# target directory, then:
#
#   1. hardened runtime, no entitlement: the host is killed by code signing
#      as soon as it runs a guest (SIGKILL);
#   2. hardened runtime + com.apple.security.cs.allow-jit: killed the same
#      way (wasmtime does not use MAP_JIT);
#   3. hardened runtime + apps/desktop/src-tauri/entitlements/plugin-host.entitlements:
#      the host's integration tests that start the real host or the
#      sandbox probe (process, sandbox, sandbox_user_data, parent_death,
#      sdk_e2e) pass, i.e. guests run and the host's own Seatbelt sandbox
#      still denies files, network and exec.
#
# Usage: plugins/runtime/scripts/macos-hardened-runtime.sh
# Builds the tests (debug) in the runtime's target directory (or
# CARGO_TARGET_DIR) first. A no-op with a message on other systems.
#
# Steps 1 and 2 need a Mac that enforces the hardened runtime's
# executable-memory rule. GitHub's macOS runners do not (System Integrity
# Protection is disabled there: the unentitled host runs guests), so there
# the script reports that and checks step 3 only. Set
# REQUIRE_HARDENED_RUNTIME_ENFORCEMENT=1 to make that a failure (local
# evidence on a stock Mac).
set -euo pipefail

if [ "$(uname -s)" != Darwin ]; then
  echo "macos-hardened-runtime: macOS only, skipped"
  exit 0
fi

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
entitlements="$root/apps/desktop/src-tauri/entitlements/plugin-host.entitlements"
cd "$root/plugins/runtime"

work=$(mktemp -d)
host="" probe=""
# Afterwards the binaries get a plain ad hoc signature again (no hardened
# runtime, no entitlement), as the linker left them.
cleanup() {
  local file
  for file in "$host" "$probe"; do
    [ -n "$file" ] && [ -f "$file" ] && codesign --force --sign - "$file" 2> /dev/null || true
  done
  rm -rf "$work"
}
trap cleanup EXIT

# The test executables, from cargo's build messages (built, not run).
cargo test --locked -p deckpilot-plugin-host --no-run --message-format=json > "$work/build.json"
test_exe() {
  jq -r --arg name "$1" \
    'select(.reason == "compiler-artifact" and .target.name == $name and .profile.test == true) | .executable' \
    "$work/build.json" | tail -n 1
}
bin_exe() {
  jq -r --arg name "$1" \
    'select(.reason == "compiler-artifact" and .target.name == $name and (.target.kind | index("bin")) and .profile.test == false) | .executable' \
    "$work/build.json" | tail -n 1
}
host=$(bin_exe deckpilot-plugin-host)
probe=$(bin_exe deckpilot-sandbox-probe)
[ -x "$host" ] && [ -x "$probe" ] || { echo "macos-hardened-runtime: host or probe not built" >&2; exit 1; }

cat > "$work/allow-jit.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>com.apple.security.cs.allow-jit</key><true/></dict></plist>
EOF

# Signs the host and the probe ad hoc with the hardened runtime and the
# given entitlements file (none: no entitlements).
sign() {
  local file
  for file in "$host" "$probe"; do
    if [ -n "$1" ]; then
      codesign --force --sign - --options runtime --entitlements "$1" "$file" 2> /dev/null
    else
      codesign --force --sign - --options runtime "$file" 2> /dev/null
    fi
  done
  codesign --display --verbose=2 "$host" 2>&1 | grep -E '^(CodeDirectory|Signature)' | sed 's/^/  /'
  codesign --display --entitlements - --xml "$host" 2> /dev/null | grep -o '<key>[^<]*</key>' | sed 's/^/  entitlement /' || echo "  no entitlements"
}

process=$(test_exe process)
# A guest that cannot run: the first invocation test fails because the host
# ended by SIGKILL while loading (the supervisor reports the wait status).
enforced=true
expect_killed() {
  echo "== $1: expect the host to be killed when it runs a guest"
  if [ "$enforced" = false ]; then
    echo "  skipped: this Mac does not enforce the hardened runtime (see above)"
    return
  fi
  if "$process" --exact host_starts_sandboxed_and_serves_invocations > "$work/out.txt" 2>&1; then
    if [ "$1" = "no entitlement" ] && [ "${REQUIRE_HARDENED_RUNTIME_ENFORCEMENT-}" != 1 ]; then
      # Without any entitlement a guest must die wherever the rule is
      # enforced, so this Mac does not enforce it.
      enforced=false
      echo "  NOT ENFORCED: the unentitled host ran a guest; this Mac does not enforce the"
      echo "  hardened runtime's executable-memory rule ($(csrutil status 2> /dev/null || echo 'csrutil unavailable'))."
      echo "  Steps 1 and 2 prove nothing here; step 3 still runs."
      return
    fi
    cat "$work/out.txt"
    echo "macos-hardened-runtime: the host ran a guest under the hardened runtime with $1; update the entitlement and docs/marketplace/distribution.md" >&2
    exit 1
  fi
  # wait status 9: ended by SIGKILL (code signing), not an exit code.
  if ! grep -F 'unix_wait_status(9)' "$work/out.txt" | sed 's/^/  /'; then
    cat "$work/out.txt"
    echo "macos-hardened-runtime: the host failed with $1, but not by SIGKILL" >&2
    exit 1
  fi
  echo "  killed by SIGKILL, as expected"
}

echo "== hardened runtime, no entitlement"
sign ""
expect_killed "no entitlement"

echo "== hardened runtime, com.apple.security.cs.allow-jit"
sign "$work/allow-jit.plist"
expect_killed "allow-jit"

echo "== hardened runtime, $(basename "$entitlements")"
sign "$entitlements"
for name in process sandbox sandbox_user_data parent_death sdk_e2e; do
  echo "-- $name"
  status=0
  "$(test_exe "$name")" --test-threads 4 > "$work/out.txt" 2>&1 || status=$?
  grep -E '^(test |test result)' "$work/out.txt" | sed 's/^/  /' || true
  if [ "$status" -ne 0 ]; then
    cat "$work/out.txt"
    echo "macos-hardened-runtime: $name failed under the hardened runtime with the host's entitlement" >&2
    exit 1
  fi
done
echo "macos-hardened-runtime: guests run and the sandbox holds under the hardened runtime with exactly $(basename "$entitlements")"
