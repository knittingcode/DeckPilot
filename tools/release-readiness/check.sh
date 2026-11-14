#!/usr/bin/env bash
# Release readiness: runs every automated check that can run on this
# computer and prints a report (docs/release-readiness.md describes the
# procedure and the manual part).
#
#   tools/release-readiness/check.sh [--with-compat] [--tag vX.Y.Z]
#
# Each line is one check with a status:
#   verified  the check ran here and passed
#   FAILED    the check ran and failed
#   skipped   not run here (option not given, or needs another platform);
#             it is NOT verified
# Exits non-zero if any check failed. It never publishes anything, never
# creates tags and never reads signing keys.
set -uo pipefail

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"
WITH_COMPAT=false
TAG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --with-compat) WITH_COMPAT=true ;;
    --tag) TAG=$2; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done

LOGS=$(mktemp -d "${TMPDIR:-/tmp}/deckpilot-readiness.XXXXXX")
failures=0
report=()
run() {
  # run <name> <command...>
  local name=$1 log
  shift
  log="$LOGS/$(echo "$name" | tr ' /' '__').log"
  if "$@" >"$log" 2>&1; then
    report+=("verified  $name")
  else
    report+=("FAILED    $name (log: $log)")
    failures=$((failures + 1))
  fi
}
skip() { report+=("skipped   $1 — $2"); }

version_args=()
[ -n "$TAG" ] && version_args=(--tag "$TAG")
run "versions are consistent" cargo run --locked -q -p deckpilot-dev -- check-versions ${version_args[@]+"${version_args[@]}"}
run "release notes exist" cargo run --locked -q -p deckpilot-dev -- release-notes
run "Rust formatting" cargo fmt --all --check
run "Rust lints" cargo clippy --workspace --all-targets --locked -- -D warnings
run "Rust tests (incl. SQLCipher, migrations, IPC, updater, modules, redaction)" \
  cargo test --workspace --locked
run "upgrade from a real 0.5.0 database" \
  cargo test --locked -p deckpilot-storage --test upgrade_from_0_5_0
run "IPC peers of earlier protocols" cargo test --locked -p deckpilot-ipc --test peers
run "frontend format" pnpm format:check
run "frontend lint" pnpm lint
run "frontend types" pnpm typecheck
run "frontend tests" pnpm test
run "hardware procedure (simulated MK.2)" \
  cargo run --locked -q -p deckpilot-dev -- hardware-test --mock --force --dir "$LOGS/hardware"
if $WITH_COMPAT; then
  run "compatibility with v0.5.0-2 (real binaries)" tools/compat/matrix.sh v0.5.0-2 "$LOGS/compat"
else
  skip "compatibility with v0.5.0-2 (real binaries)" "pass --with-compat (builds the old tag)"
fi
case "$(uname -s)" in
  Darwin)
    skip "Linux packaging" "CI job 'Package check (Linux .deb)'"
    skip "Windows packaging" "CI job 'Package check (Windows installers)'"
    ;;
  Linux)
    skip "Windows packaging" "CI job 'Package check (Windows installers)'"
    ;;
  *) skip "Linux packaging" "CI job" ;;
esac
skip "macOS signing and notarization" "Release workflow on a test tag, or Apple signing check workflow on a release tag"
skip "signed update and Keychain continuity" "update-continuity workflow; tools/release-validation/update-test.sh"
skip "manual platform checks" "see docs/release-readiness.md"

printf '%s\n' "${report[@]}"
echo "logs: $LOGS"
exit "$failures"
