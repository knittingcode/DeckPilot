#!/usr/bin/env bash
# Tests the version handling of the package checks without building real
# packages: msi-version.sh's mapping, and check-packages.sh on fake Linux
# packages of a pre-release version (a .deb with the expected layout and a
# stand-in agent, signed with a throwaway key bound to the version).
#
#   tools/ci/test-check-packages.sh [<work dir>]
#
# Needs dpkg-deb (Linux), jq, pnpm (the Tauri CLI, `pnpm install` done) and
# cargo. Nothing leaves <work dir>.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/check-packages-test.XXXXXX")}"
mkdir -p "$work"
work="$(cd "$work" && pwd)"
tools="$root/tools/ci"
failures=0
ok() { echo "ok   $*"; }
bad() {
  echo "FAIL $*"
  failures=$((failures + 1))
}

echo "== MSI versions"
for case in 0.6.0=0.6.0 0.6.0-alpha.1=0.6.0.1 0.6.0-beta.3=0.6.0.20003 0.6.0-rc.2=0.6.0.40002 \
  1.2.3-alpha.19999=1.2.3.19999 0.6.0-dev.1= 0.6.0-alpha.20000= 0.6.0-rc= v0.6.0=; do
  input="${case%%=*}" expected="${case#*=}"
  actual=$("$tools/msi-version.sh" "$input" 2> /dev/null || true)
  if [ "$actual" = "$expected" ]; then ok "$input -> ${expected:-(refused)}"; else bad "$input -> '$actual', expected '${expected:-(refused)}'"; fi
done

echo "== throwaway signing key (not the release key)"
rm -f "$work/key" "$work/key.pub"
pnpm --dir "$root" --filter @deckpilot/desktop exec tauri signer generate --ci -p "" -w "$work/key" > /dev/null
export TAURI_SIGNING_PRIVATE_KEY_PATH="$work/key" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
THROWAWAY_UPDATER_PUBKEY=$(tr -d '\r\n' < "$work/key.pub")
export THROWAWAY_UPDATER_PUBKEY
export DECKPILOT_DEV="cargo run --manifest-path $root/Cargo.toml --locked -q -p deckpilot-dev --"

# deb <version>: a .deb laid out as the bundler builds it, and its signature.
deb() {
  local version="$1" tree="$work/deb-tree" bundle="$work/bundle"
  rm -rf "$tree" "$bundle"
  mkdir -p "$tree/DEBIAN" "$tree/usr/bin" "$tree/usr/lib/udev/rules.d" "$tree/usr/share/applications" "$bundle/deb"
  printf 'Package: deck-pilot\nVersion: %s\nArchitecture: amd64\nMaintainer: test\nDescription: test package\n' "$version" > "$tree/DEBIAN/control"
  printf '#!/bin/sh\necho "deckpilot-agent %s"\n' "$version" > "$tree/usr/bin/deckpilot-agent"
  printf '#!/bin/sh\n' > "$tree/usr/bin/deckpilot-desktop"
  chmod 755 "$tree/usr/bin/deckpilot-agent" "$tree/usr/bin/deckpilot-desktop"
  : > "$tree/usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules"
  : > "$tree/usr/share/applications/DeckPilot.desktop"
  dpkg-deb --root-owner-group --build "$tree" "$bundle/deb/DeckPilot_${version}_amd64.deb" > /dev/null
  pnpm --dir "$root" --filter @deckpilot/desktop exec tauri signer sign \
    --app-version "$version" "$bundle/deb/DeckPilot_${version}_amd64.deb" > /dev/null
}

# check <expect pass|fail> <message part> <description> <env...>
check() {
  local expect="$1" message="$2" what="$3"
  shift 3
  if env "$@" BUNDLE_DIR="$work/bundle" RUNNER_TEMP="$work" "$tools/check-packages.sh" linux > "$work/check.log" 2>&1; then
    if [ "$expect" = pass ]; then ok "$what"; else bad "$what: passed, expected a failure"; fi
  elif [ "$expect" = fail ] && grep -q -F -- "$message" "$work/check.log"; then
    ok "$what"
  else
    bad "$what: $(tail -n 3 "$work/check.log")"
  fi
}

echo "== check-packages.sh on a pre-release .deb"
deb 0.6.0-alpha.1
check pass "" "a pre-release without .rpm passes" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
check fail "is a pre-release version; set PRERELEASE=true" "a pre-release version needs PRERELEASE=true" PACKAGE_VERSION=0.6.0-alpha.1
check fail "is not a pre-release version" "PRERELEASE=true needs a pre-release version" PRERELEASE=true PACKAGE_VERSION=0.6.0
check fail "expected $work/bundle/deb/DeckPilot_0.6.0-alpha.2_amd64.deb" "a .deb of another version is missing" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.2

mkdir -p "$work/bundle/rpm"
: > "$work/bundle/rpm/DeckPilot-0.6.0-alpha.1-1.x86_64.rpm"
check fail "was built for a pre-release version" "an .rpm of a pre-release is refused" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1

echo "== a final version still needs the .rpm"
deb 0.6.0
check fail "expected $work/bundle/rpm/DeckPilot-0.6.0-1.x86_64.rpm" "a final version without .rpm fails" PACKAGE_VERSION=0.6.0

echo "== the signature must name the version"
deb 0.6.0-alpha.1
pnpm --dir "$root" --filter @deckpilot/desktop exec tauri signer sign \
  --app-version 0.6.0-alpha.9 "$work/bundle/deb/DeckPilot_0.6.0-alpha.1_amd64.deb" > /dev/null
check fail "does not name version 0.6.0-alpha.1" "a signature for another version fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1

if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed (work dir: $work)"
  exit 1
fi
echo "package check versions: all checks passed"
