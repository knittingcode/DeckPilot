#!/usr/bin/env bash
# Tests the assembly of latest.json (updater-fragment.sh and
# assemble-latest-json.sh) without a release: fake packages for all four
# release builds, signed with a throwaway key bound to the version, as the
# Tauri bundler signs them. The assembled manifest must pass the release's
# own verification (deckpilot-dev verify-release); broken inputs must fail.
#
#   tools/ci/test-latest-json.sh [<work dir>]
#
# Needs jq, pnpm (the Tauri CLI, `pnpm install` done) and cargo. Nothing is
# uploaded; everything stays in <work dir> (default: a temporary directory).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/latest-json-test.XXXXXX")}"
mkdir -p "$work"
work="$(cd "$work" && pwd)"
tools="$root/tools/ci"
version=$(jq -r .version "$root/apps/desktop/src-tauri/tauri.conf.json")
failures=0
ok() { echo "ok   $*"; }
bad() {
  echo "FAIL $*"
  failures=$((failures + 1))
}

echo "== throwaway signing key (not the release key)"
rm -f "$work/key" "$work/key.pub"
pnpm --dir "$root" --filter @deckpilot/desktop exec tauri signer generate --ci -p "" -w "$work/key" > /dev/null
export TAURI_SIGNING_PRIVATE_KEY_PATH="$work/key" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
public=$(tr -d '\r\n' < "$work/key.pub")

# package <file> [<signed version>]: a fake package with its .sig.
package() {
  mkdir -p "$(dirname "$1")"
  printf 'fake package %s\n' "$(basename "$1")" > "$1"
  pnpm --dir "$root" --filter @deckpilot/desktop exec tauri signer sign \
    --app-version "${2:-$version}" "$1" > /dev/null
}

echo "== fake bundles of the four release builds ($version)"
bundles="$work/bundles"
rm -rf "$bundles"
package "$bundles/aarch64-apple-darwin/macos/DeckPilot.app.tar.gz"
package "$bundles/x86_64-apple-darwin/macos/DeckPilot.app.tar.gz"
linux="$bundles/x86_64-unknown-linux-gnu"
package "$linux/appimage/DeckPilot_${version}_amd64.AppImage"
package "$linux/deb/DeckPilot_${version}_amd64.deb"
package "$linux/rpm/DeckPilot-${version}-1.x86_64.rpm"
windows="$bundles/x86_64-pc-windows-msvc"
package "$windows/msi/DeckPilot_${version}_x64_en-US.msi"
package "$windows/nsis/DeckPilot_${version}_x64-setup.exe"

fragments="$work/fragments"
rm -rf "$fragments"
for target in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu x86_64-pc-windows-msvc; do
  "$tools/updater-fragment.sh" "$target" "$bundles/$target" "$fragments/$target.json"
done

# The draft release as tauri-action fills it: packages under their asset
# names, each with its .sig.
release="$work/release"
rm -rf "$release" && mkdir -p "$release"
cp "$bundles/aarch64-apple-darwin/macos/DeckPilot.app.tar.gz" "$release/DeckPilot_aarch64.app.tar.gz"
cp "$bundles/aarch64-apple-darwin/macos/DeckPilot.app.tar.gz.sig" "$release/DeckPilot_aarch64.app.tar.gz.sig"
cp "$bundles/x86_64-apple-darwin/macos/DeckPilot.app.tar.gz" "$release/DeckPilot_x64.app.tar.gz"
cp "$bundles/x86_64-apple-darwin/macos/DeckPilot.app.tar.gz.sig" "$release/DeckPilot_x64.app.tar.gz.sig"
for file in "$linux"/*/* "$windows"/*/*; do cp "$file" "$release/"; done
ls "$release" > "$work/assets.txt"
printf 'Release notes of the test.\n\n' > "$work/notes.md"

export REPOSITORY=knittingcode/DeckPilot VERSION="$version" NOTES_FILE="$work/notes.md" \
  PUB_DATE=2026-01-01T00:00:00Z ASSETS_FILE="$work/assets.txt"

# assemble <tag> <fragments dir> <output>
assemble() { TAG="$1" "$tools/assemble-latest-json.sh" "$2" "$3" > "$work/assemble.log" 2>&1; }

echo "== complete release"
if assemble "v$version" "$fragments" "$release/latest.json"; then
  ok "assembled"
else
  bad "assembling a complete release: $(cat "$work/assemble.log")"
fi
# same <actual> <expected> <what>
same() {
  if [ "$1" = "$2" ]; then ok "$3"; else bad "$3: got '$1', expected '$2'"; fi
}
manifest_value() { jq -r "$1" "$release/latest.json" 2>/dev/null | tr '\n' '|' || true; }
same "$(manifest_value '.platforms | keys | join(" ")')" \
  "darwin-aarch64 darwin-aarch64-app darwin-x86_64 darwin-x86_64-app linux-x86_64 linux-x86_64-appimage linux-x86_64-deb linux-x86_64-rpm windows-x86_64 windows-x86_64-msi windows-x86_64-nsis|" \
  "every target key once"
same "$(manifest_value '.platforms["darwin-x86_64"].url')" \
  "https://github.com/knittingcode/DeckPilot/releases/download/v$version/DeckPilot_x64.app.tar.gz|" \
  "macOS URL uses the uploaded asset name"
same "$(manifest_value '.platforms["windows-x86_64"].url')" \
  "https://github.com/knittingcode/DeckPilot/releases/download/v$version/DeckPilot_${version}_x64_en-US.msi|" \
  "Windows default is the MSI"
same "$(manifest_value '.notes, .version, .pub_date')" \
  "Release notes of the test.|$version|2026-01-01T00:00:00Z|" "version, notes and date"
# The same verification as the release workflow's verify job.
if DECKPILOT_UPDATER_PUBKEY="$public" cargo run --manifest-path "$root/Cargo.toml" --locked -q -p deckpilot-dev -- \
  verify-release "$release" > "$work/verify.log" 2>&1; then
  ok "deckpilot-dev verify-release accepts it ($(grep -c '^verified' "$work/verify.log") entries)"
else
  bad "verify-release: $(cat "$work/verify.log")"
fi

# expect_failure <what> <expected message> <tag> <fragments dir>
expect_failure() {
  if assemble "$3" "$4" "$work/rejected.json"; then
    bad "$1: assembled anyway"
  elif grep -q -F "$2" "$work/assemble.log"; then
    ok "$1: rejected"
  else
    bad "$1: unexpected error: $(cat "$work/assemble.log")"
  fi
}

variant() {
  rm -rf "${work:?}/${1:?}" && cp -R "$fragments" "$work/$1"
  echo "$work/$1"
}

echo "== broken inputs"
dir=$(variant lost-build)
rm "$dir/x86_64-pc-windows-msvc.json"
expect_failure "a build's fragment is missing" "no update for: windows-x86_64 windows-x86_64-msi windows-x86_64-nsis" "v$version" "$dir"

dir=$(variant twice)
cp "$dir/aarch64-apple-darwin.json" "$dir/aarch64-apple-darwin-again.json"
expect_failure "two builds provide the same key" "several builds provide: darwin-aarch64 darwin-aarch64-app" "v$version" "$dir"

dir=$(variant no-rpm)
jq 'del(.platforms["linux-x86_64-rpm"])' "$fragments/x86_64-unknown-linux-gnu.json" > "$dir/x86_64-unknown-linux-gnu.json"
expect_failure "a final release without the .rpm" "no update for: linux-x86_64-rpm" "v$version" "$dir"
if assemble "v$version-1" "$dir" "$work/prerelease.json"; then
  ok "a test release needs no .rpm"
else
  bad "test release without .rpm: $(cat "$work/assemble.log")"
fi

grep -v -x -F "DeckPilot_${version}_amd64.deb.sig" "$work/assets.txt" > "$work/assets-without-sig.txt"
if ASSETS_FILE="$work/assets-without-sig.txt" assemble "v$version" "$fragments" "$work/rejected.json"; then
  bad "an asset missing from the release: assembled anyway"
elif grep -q -F "the release has no asset DeckPilot_${version}_amd64.deb.sig" "$work/assemble.log"; then
  ok "an asset missing from the release: rejected"
else
  bad "an asset missing from the release: $(cat "$work/assemble.log")"
fi

expect_failure "no fragments at all" "no updater fragments" "v$version" "$work/empty-dir-that-does-not-exist"

# A build whose package was not signed has no fragment.
rm "$windows/nsis/DeckPilot_${version}_x64-setup.exe.sig"
if "$tools/updater-fragment.sh" x86_64-pc-windows-msvc "$windows" "$work/unsigned.json" > "$work/fragment.log" 2>&1; then
  bad "an unsigned package: fragment written anyway"
elif grep -q "has no updater signature" "$work/fragment.log"; then
  ok "an unsigned package: no fragment"
else
  bad "an unsigned package: $(cat "$work/fragment.log")"
fi

# A package signed for another version passes assembly (the manifest is
# not signed) but not the release verification, and the app refuses it.
package "$windows/nsis/DeckPilot_${version}_x64-setup.exe" 0.0.1
cp "$windows/nsis/DeckPilot_${version}_x64-setup.exe".sig "$release/"
"$tools/updater-fragment.sh" x86_64-pc-windows-msvc "$windows" "$fragments/x86_64-pc-windows-msvc.json" > /dev/null
assemble "v$version" "$fragments" "$release/latest.json" || bad "reassembling: $(cat "$work/assemble.log")"
if DECKPILOT_UPDATER_PUBKEY="$public" cargo run --manifest-path "$root/Cargo.toml" --locked -q -p deckpilot-dev -- \
  verify-release "$release" > "$work/verify.log" 2>&1; then
  bad "a package signed for another version: verify-release accepted it"
else
  ok "a package signed for another version: verify-release rejects it"
fi

if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed (work dir: $work)"
  exit 1
fi
echo "latest.json assembly: all checks passed"
