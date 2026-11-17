#!/usr/bin/env bash
# Tests the package checks without building real packages: msi-version.sh's
# mapping, and check-packages.sh on fake packages laid out as the bundler
# builds them, with stand-in binaries that carry a distribution record and
# the bundler's package-format marker, the plugin host and the bundled
# first-party plugins (stand-in modules, real manifests, pinned), signed
# with a throwaway key bound to the version:
#
# - Linux (needs dpkg-deb; skipped without it): a pre-release .deb and
#   AppImage, and what the checks must refuse in them;
# - macOS (any host; disk images through a stand-in hdiutil): the .app, its
#   updater archive and the disk image, and what the checks must refuse;
# - `check-packages.sh record` on single binaries.
#
#   tools/ci/test-check-packages.sh [<work dir>]
#
# Needs jq, python3, tar, pnpm (the Tauri CLI, `pnpm install` done) and
# cargo. Nothing leaves <work dir>.
# ok and bad never fail, so `check && ok || bad` is an if-then-else here.
# shellcheck disable=SC2015
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
override="DECKPILOT_DISTRIBUTION""_OVERRIDE"

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
sign() {
  pnpm --dir "$root" --filter @deckpilot/desktop exec tauri signer sign --app-version "$1" "$2" > /dev/null
}

# app <file> <version> <bundle token> [<channel> [<extra line>]]: a stand-in
# application. Its text carries the distribution record and the bundler's
# tokens as a real binary does: the patched token twice (the static and the
# match arm), every other once; UNK (unpatched) leaves all tokens once.
app() {
  local file="$1" version="$2" token="$3" channel="${4:-github}" extra="${5-}" name tokens=""
  for name in UNK DEB RPM APP MSI NSS; do
    if [ "$name" = "$token" ] && [ "$token" != UNK ]; then
      tokens="$tokens __TAURI_BUNDLE_TYPE_VAR_$name __TAURI_BUNDLE_TYPE_VAR_$name"
    elif [ "$name" != UNK ] || [ "$token" = UNK ]; then
      tokens="$tokens __TAURI_BUNDLE_TYPE_VAR_$name"
    fi
  done
  mkdir -p "$(dirname "$file")"
  {
    echo '#!/bin/sh'
    echo "# deckpilot-distribution-record:1;channel=$channel;format=;version=$version;end"
    echo "#$tokens"
    [ -z "$extra" ] || echo "# $extra"
  } > "$file"
  chmod 755 "$file"
}

# agent <file> <version>: a stand-in agent that reports <version> and
# prints $work/compiled-pins.json as the pins it was compiled with.
agent() {
  mkdir -p "$(dirname "$1")"
  # shellcheck disable=SC2016 # $1 belongs to the stand-in
  printf '#!/bin/sh\nif [ "$1" = --print-bundled-pins ]; then cat "%s"; else echo "deckpilot-agent %s"; fi\n' \
    "$work/compiled-pins.json" "$2" > "$1"
  chmod 755 "$1"
}

# host <file>: a stand-in plugin host.
host() {
  mkdir -p "$(dirname "$1")"
  printf '#!/bin/sh\n' > "$1"
  chmod 755 "$1"
}

# The bundled first-party plugins as plugins/first-party/build.sh stages
# them, for the release set of plugins/first-party/bundled.json: the real
# manifests and licenses, stand-in modules, and pins.json. The stand-in
# agents were "compiled" with these pins ($work/compiled-pins.json).
sha() { if command -v sha256sum > /dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
bundled_source="$work/bundled-source"
rm -rf "$bundled_source" && mkdir -p "$bundled_source"
pins='[]'
while IFS= read -r path; do
  manifest="$root/plugins/first-party/$path/manifest.json"
  pid=$(jq -r .id "$manifest")
  mkdir -p "$bundled_source/$pid"
  cp "$manifest" "$bundled_source/$pid/manifest.json"
  cp "$root/plugins/first-party/$path/LICENSE" "$bundled_source/$pid/LICENSE"
  printf '\0asm\1\0\0\0stand-in %s' "$pid" > "$bundled_source/$pid/plugin.wasm"
  files='{}'
  for file in LICENSE manifest.json plugin.wasm; do
    files=$(jq --arg f "$file" --arg s "$(sha "$bundled_source/$pid/$file")" \
      --argjson n "$(wc -c < "$bundled_source/$pid/$file" | tr -d ' ')" '. + {($f): {sha256: $s, size: $n}}' <<< "$files")
  done
  pins=$(jq --arg id "$pid" --arg v "$(jq -r .version "$manifest")" --argjson f "$files" \
    '. + [{id: $id, version: $v, class: "default", files: $f}]' <<< "$pins")
done < <(jq -r '.plugins[].path' "$root/plugins/first-party/bundled.json")
jq -S -n --argjson p "$pins" '{pinsVersion: 1, plugins: ($p | sort_by(.id))}' > "$bundled_source/pins.json"
cp "$bundled_source/pins.json" "$work/compiled-pins.json"
# bundled <dir>: the bundled plugins at <dir> (the resources' bundled/).
bundled() {
  rm -rf "$1" && mkdir -p "$(dirname "$1")"
  cp -R "$bundled_source" "$1"
}

# check <platform> <expect pass|fail> <message part> <description> <env...>
check() {
  local platform="$1" expect="$2" message="$3" what="$4"
  shift 4
  if env "$@" BUNDLE_DIR="$work/bundle" RUNNER_TEMP="$work" "$tools/check-packages.sh" "$platform" > "$work/check.log" 2>&1; then
    if [ "$expect" = pass ]; then ok "$what"; else bad "$what: passed, expected a failure"; fi
  elif [ "$expect" = fail ] && grep -q -F -- "$message" "$work/check.log"; then
    ok "$what"
  else
    bad "$what: $(tail -n 3 "$work/check.log")"
  fi
}

echo "== check-packages.sh record"
record() { env "$@" RUNNER_TEMP="$work" "$tools/check-packages.sh" record "$work/bin" "$format" > "$work/record.out" 2> "$work/record.log"; }
format=deb
app "$work/bin" 0.6.0 DEB
record PACKAGE_VERSION=0.6.0 && grep -q -x distribution_channel=github "$work/record.out" && grep -q -x package_format=deb "$work/record.out" \
  && ok "record prints the channel and the bundler's format" || bad "record: $(cat "$work/record.out" "$work/record.log")"
format=none
app "$work/bin" 0.6.0 UNK
record PACKAGE_VERSION=0.6.0 && grep -q -x package_format= "$work/record.out" \
  && ok "the unpackaged build output has no format yet" || bad "record none: $(cat "$work/record.out" "$work/record.log")"
record PACKAGE_VERSION=0.6.0 DISTRIBUTION_CHANNEL=apt && bad "a github record passes as apt" \
  || { grep -q -F "built for the distribution channel 'github', expected 'apt'" "$work/record.log" && ok "another expected channel is refused" || bad "record apt: $(cat "$work/record.log")"; }
record PACKAGE_VERSION=0.6.1 && bad "a record of another version passes" \
  || { grep -q -F "records version 0.6.0, expected 0.6.1" "$work/record.log" && ok "a record of another version is refused" || bad "record version: $(cat "$work/record.log")"; }
format=msi
record PACKAGE_VERSION=0.6.0 && bad "an unpatched binary passes as msi" \
  || { grep -q -F "is not marked as msi by the bundler" "$work/record.log" && ok "an unpatched binary is not an MSI's" || bad "record msi: $(cat "$work/record.log")"; }
format=none
app "$work/bin" 0.6.0 NSS
record PACKAGE_VERSION=0.6.0 && bad "a patched binary passes as unpackaged" \
  || { grep -q -F "carries a patched bundle type" "$work/record.log" && ok "a patched binary is not the build output" || bad "record patched: $(cat "$work/record.log")"; }
app "$work/bin" 0.6.0 UNK github "$override"
record PACKAGE_VERSION=0.6.0 && bad "a binary with the development override passes" \
  || { grep -q -F "a development override is compiled in" "$work/record.log" && ok "a development override is refused" || bad "record override: $(cat "$work/record.log")"; }
printf '#!/bin/sh\n' > "$work/bin"
record PACKAGE_VERSION=0.6.0 && bad "a binary without a record passes" \
  || { grep -q -F "carries no distribution record" "$work/record.log" && ok "a binary without a record is refused" || bad "record missing: $(cat "$work/record.log")"; }
app "$work/bin" 0.6.0 UNK
echo "# deckpilot-distribution-record:1;channel=apt;format=deb;version=0.6.0;end" >> "$work/bin"
record PACKAGE_VERSION=0.6.0 && bad "a binary with two records passes" \
  || { grep -q -F "carries several distribution records" "$work/record.log" && ok "a binary with two records is refused" || bad "record twice: $(cat "$work/record.log")"; }
# The agent does not link deckpilot-distribution yet: optional.
optional() { env "$@" RUNNER_TEMP="$work" "$tools/check-packages.sh" record "$work/bin" none optional > "$work/record.out" 2> "$work/record.log"; }
agent "$work/bin" 0.6.0
optional PACKAGE_VERSION=0.6.0 && [ ! -s "$work/record.out" ] && ok "an optional record may be absent (and prints nothing)" || bad "optional: $(cat "$work/record.out" "$work/record.log")"
app "$work/bin" 0.6.0 UNK apt
optional PACKAGE_VERSION=0.6.0 && bad "an optional record of another channel passes" \
  || { grep -q -F "built for the distribution channel 'apt'" "$work/record.log" && ok "an optional record, when present, must match" || bad "optional apt: $(cat "$work/record.log")"; }
printf '#!/bin/sh\n# %s\n' "$override" > "$work/bin"
optional PACKAGE_VERSION=0.6.0 && bad "a binary without a record but with the override passes" \
  || { grep -q -F "a development override is compiled in" "$work/record.log" && ok "the override is refused even without a record" || bad "optional override: $(cat "$work/record.log")"; }

if command -v dpkg-deb > /dev/null; then
  # linux <version> [<deb app token> [<channel>]]: a .deb and an AppImage
  # laid out as the bundler builds them, and their signatures.
  linux() {
    local version="$1" token="${2:-DEB}" channel="${3:-github}" tree="$work/deb-tree" bundle="$work/bundle"
    rm -rf "$tree" "$bundle"
    mkdir -p "$tree/DEBIAN" "$tree/usr/lib/udev/rules.d" "$tree/usr/share/applications" "$bundle/deb" "$bundle/appimage"
    printf 'Package: deck-pilot\nVersion: %s\nArchitecture: amd64\nMaintainer: test\nDescription: test package\n' "$version" > "$tree/DEBIAN/control"
    agent "$tree/usr/bin/deckpilot-agent" "$version"
    app "$tree/usr/bin/deckpilot-desktop" "$version" "$token" "$channel"
    host "$tree/usr/bin/deckpilot-plugin-host"
    bundled "$tree/usr/lib/DeckPilot/bundled"
    [ -z "${DEB_OMIT-}" ] || rm -rf "${tree:?}/$DEB_OMIT"
    : > "$tree/usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules"
    : > "$tree/usr/share/applications/DeckPilot.desktop"
    dpkg-deb --root-owner-group --build "$tree" "$bundle/deb/DeckPilot_${version}_amd64.deb" > /dev/null
    sign "$version" "$bundle/deb/DeckPilot_${version}_amd64.deb"
    appimage "$version"
  }
  # appimage <version> [<omit path>]: a self-extracting stand-in AppImage.
  appimage() {
    local version="$1" omit="${2-}" tree="$work/appimage-tree" file
    file="$work/bundle/appimage/DeckPilot_${version}_amd64.AppImage"
    rm -rf "$tree" && mkdir -p "$tree"
    printf '#!/bin/sh\n' > "$tree/AppRun"
    : > "$tree/DeckPilot.desktop"
    agent "$tree/usr/bin/deckpilot-agent" "$version"
    app "$tree/usr/bin/deckpilot-desktop" "$version" APP
    host "$tree/usr/bin/deckpilot-plugin-host"
    bundled "$tree/usr/lib/DeckPilot/bundled"
    [ -z "$omit" ] || rm -rf "${tree:?}/$omit"
    {
      # shellcheck disable=SC2016 # the stand-in's own script
      printf '#!/bin/sh\n[ "$1" = --appimage-extract ] || exit 1\nmkdir -p squashfs-root\nbase64 -d << "PAYLOAD" | tar -xzf - -C squashfs-root\n'
      tar -czf - -C "$tree" . | base64
      printf 'PAYLOAD\n'
    } > "$file"
    chmod 755 "$file"
    sign "$version" "$file"
  }

  echo "== check-packages.sh linux on a pre-release .deb and AppImage"
  linux 0.6.0-alpha.1
  check linux pass "" "a pre-release without .rpm passes" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  check linux fail "is a pre-release version; set PRERELEASE=true" "a pre-release version needs PRERELEASE=true" PACKAGE_VERSION=0.6.0-alpha.1
  check linux fail "is not a pre-release version" "PRERELEASE=true needs a pre-release version" PRERELEASE=true PACKAGE_VERSION=0.6.0
  check linux fail "expected $work/bundle/deb/DeckPilot_0.6.0-alpha.2_amd64.deb" "a .deb of another version is missing" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.2

  mkdir -p "$work/bundle/rpm"
  : > "$work/bundle/rpm/DeckPilot-0.6.0-alpha.1-1.x86_64.rpm"
  check linux fail "was built for a pre-release version" "an .rpm of a pre-release is refused" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1

  echo "== a final version still needs the .rpm"
  linux 0.6.0
  check linux fail "expected $work/bundle/rpm/DeckPilot-0.6.0-1.x86_64.rpm" "a final version without .rpm fails" PACKAGE_VERSION=0.6.0

  echo "== the signature must name the version"
  linux 0.6.0-alpha.1
  sign 0.6.0-alpha.9 "$work/bundle/deb/DeckPilot_0.6.0-alpha.1_amd64.deb"
  check linux fail "does not name version 0.6.0-alpha.1" "a signature for another version fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1

  echo "== the distribution record and package format of the .deb"
  linux 0.6.0-alpha.1 DEB apt
  check linux fail "built for the distribution channel 'apt', expected 'github'" "a .deb of another channel is refused" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  linux 0.6.0-alpha.1 UNK
  check linux fail "is not marked as deb by the bundler" "a .deb whose app was not patched is refused" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  linux 0.6.0-alpha.1 APP
  check linux fail "is not marked as deb by the bundler" "a .deb with the AppImage's app is refused" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1

  echo "== the AppImage"
  linux 0.6.0-alpha.1
  rm "$work/bundle/appimage/DeckPilot_0.6.0-alpha.1_amd64.AppImage"
  check linux fail "expected $work/bundle/appimage/DeckPilot_0.6.0-alpha.1_amd64.AppImage" "a missing AppImage fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  appimage 0.6.0-alpha.1 AppRun
  check linux fail "lacks AppRun" "an AppImage without AppRun fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  appimage 0.6.0-alpha.1 usr/bin/deckpilot-agent
  check linux fail "lacks usr/bin/deckpilot-agent" "an AppImage without the agent fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  appimage 0.6.0-alpha.1
  rm "$work/bundle/appimage/DeckPilot_0.6.0-alpha.1_amd64.AppImage.sig"
  check linux fail "has no updater signature" "an AppImage without its signature fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  appimage 0.6.0-alpha.1 usr/bin/deckpilot-plugin-host
  check linux fail "lacks usr/bin/deckpilot-plugin-host" "an AppImage without the plugin host fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  appimage 0.6.0-alpha.1 usr/lib/DeckPilot/bundled/deckpilot.notifications
  check linux fail "bundled/deckpilot.notifications is missing" "an AppImage without a pinned bundled plugin fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1

  echo "== the plugin runtime in the .deb"
  DEB_OMIT=usr/bin/deckpilot-plugin-host linux 0.6.0-alpha.1
  check linux fail "lacks usr/bin/deckpilot-plugin-host" "a .deb without the plugin host fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  DEB_OMIT=usr/lib/DeckPilot/bundled linux 0.6.0-alpha.1
  check linux fail "lacks usr/lib/DeckPilot/bundled/pins.json" "a .deb without the bundled plugins fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
  DEB_OMIT=usr/lib/DeckPilot/bundled/deckpilot.jira/plugin.wasm linux 0.6.0-alpha.1
  check linux fail "pinned plugin.wasm is missing" "a .deb with a bundled plugin's module missing fails" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
else
  echo "== Linux packages: skipped (no dpkg-deb)"
fi

echo "== check-packages.sh macos on a pre-release bundle, archive and disk image"
# A stand-in hdiutil: a "disk image" is a tar archive; attach extracts it.
cat > "$work/hdiutil" << 'EOF'
#!/bin/sh
case "$1" in
  attach) mount="" image="" ; shift
    while [ $# -gt 0 ]; do case "$1" in -mountpoint) mount="$2"; shift 2 ;; -*) shift ;; *) image="$1"; shift ;; esac; done
    tar -xf "$image" -C "$mount" ;;
  detach) ;;
  *) exit 1 ;;
esac
EOF
chmod 755 "$work/hdiutil"
# bundle_app <DeckPilot.app> <version> [<token> [<CFBundleShortVersionString>]]
bundle_app() {
  local app="$1" version="$2" token="${3:-UNK}" shown="${4:-$2}"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  python3 -I -c 'import plistlib, sys
plistlib.dump({"CFBundleIdentifier": "io.github.knittingcode.deckpilot", "CFBundleShortVersionString": sys.argv[2],
  "CFBundleVersion": sys.argv[2], "CFBundleExecutable": "deckpilot-desktop", "CFBundleIconFile": "icon.icns"}, open(sys.argv[1], "wb"))' \
    "$app/Contents/Info.plist" "$shown"
  app "$app/Contents/MacOS/deckpilot-desktop" "$version" "$token"
  agent "$app/Contents/MacOS/deckpilot-agent" "$version"
  host "$app/Contents/MacOS/deckpilot-plugin-host"
  bundled "$app/Contents/Resources/bundled"
  : > "$app/Contents/Resources/icon.icns"
}
# macos <version> [<arch>]: the bundle, its signed updater archive and the
# disk image, as the bundler leaves them in bundle/macos and bundle/dmg.
macos() {
  local version="$1" arch="${2:-aarch64}" bundle="$work/bundle"
  rm -rf "$bundle" && mkdir -p "$bundle/macos" "$bundle/dmg"
  bundle_app "$bundle/macos/DeckPilot.app" "$version"
  tar -czf "$bundle/macos/DeckPilot.app.tar.gz" -C "$bundle/macos" DeckPilot.app
  sign "$version" "$bundle/macos/DeckPilot.app.tar.gz"
  dmg "$version" "$arch"
}
# dmg <version> <arch> [no-link]: the disk image's volume, as a tar archive.
dmg() {
  local version="$1" arch="$2" volume="$work/volume"
  rm -rf "$volume" && mkdir -p "$volume"
  cp -R "$work/bundle/macos/DeckPilot.app" "$volume/"
  [ "${3-}" = no-link ] || ln -s /Applications "$volume/Applications"
  tar -cf "$work/bundle/dmg/DeckPilot_${version}_${arch}.dmg" -C "$volume" .
}
mac() { check macos "$@" HDIUTIL="$work/hdiutil" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1 MACOS_ARCH=aarch64; }
macos 0.6.0-alpha.1
mac pass "" "a complete macOS build passes"
check macos fail "DeckPilot_0.6.0-alpha.1_x64.dmg" "the disk image of the other architecture is missing" \
  HDIUTIL="$work/hdiutil" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1 MACOS_ARCH=x64
# Without MACOS_ARCH, the host's architecture (as in the Packaging job).
host=x64
if [ "$(uname -m)" = arm64 ] || [ "$(uname -m)" = aarch64 ]; then host=aarch64; fi
macos 0.6.0-alpha.1 "$host"
check macos pass "" "the disk image of the host's architecture is found without MACOS_ARCH" \
  HDIUTIL="$work/hdiutil" PRERELEASE=true PACKAGE_VERSION=0.6.0-alpha.1
macos 0.6.0-alpha.1
rm "$work/bundle/macos/DeckPilot.app.tar.gz.sig"
mac fail "has no updater signature" "an updater archive without its signature fails"
macos 0.6.0-alpha.1
bundle_app "$work/bundle/macos/DeckPilot.app" 0.6.0-alpha.1 UNK 0.6.0
mac fail "CFBundleShortVersionString is '0.6.0', expected 0.6.0-alpha.1" "a bundle of another version fails"
macos 0.6.0-alpha.1
rm "$work/bundle/macos/DeckPilot.app/Contents/MacOS/deckpilot-agent"
mac fail "lacks the executable Contents/MacOS/deckpilot-agent" "a bundle without the agent fails"
macos 0.6.0-alpha.1
rm "$work/bundle/macos/DeckPilot.app/Contents/Resources/icon.icns"
mac fail "lacks its icon" "a bundle without its icon fails"
macos 0.6.0-alpha.1
rm "$work/bundle/macos/DeckPilot.app/Contents/MacOS/deckpilot-plugin-host"
mac fail "has the plugin host at '', expected exactly Contents/MacOS/deckpilot-plugin-host" "a bundle without the plugin host fails"
macos 0.6.0-alpha.1
host "$work/bundle/macos/DeckPilot.app/Contents/Resources/deckpilot-plugin-host-aarch64-apple-darwin"
mac fail "expected exactly Contents/MacOS/deckpilot-plugin-host" "a bundle with a second plugin host fails"
macos 0.6.0-alpha.1
chmod -x "$work/bundle/macos/DeckPilot.app/Contents/MacOS/deckpilot-plugin-host"
mac fail "lacks the plugin host" "a bundle whose plugin host is not executable fails"
macos 0.6.0-alpha.1
rm -rf "$work/bundle/macos/DeckPilot.app/Contents/Resources/bundled"
mac fail "lacks the bundled plugins" "a bundle without the bundled plugins fails"
macos 0.6.0-alpha.1
printf '{"pinsVersion":1,"plugins":[]}\n' > "$work/bundle/macos/DeckPilot.app/Contents/Resources/bundled/pins.json"
rm -rf "$work/bundle/macos/DeckPilot.app/Contents/Resources/bundled/"deckpilot.*
mac fail "expected those of plugins/first-party/bundled.json" "a bundle without any bundled plugin fails"
macos 0.6.0-alpha.1
printf 'X' >> "$work/bundle/macos/DeckPilot.app/Contents/Resources/bundled/deckpilot.github/plugin.wasm"
mac fail "the bundled plugins do not match their pins" "a bundle with a changed bundled plugin fails"
macos 0.6.0-alpha.1
cp "$work/compiled-pins.json" "$work/compiled-pins.saved"
printf '{"pinsVersion":1,"plugins":[]}\n' > "$work/compiled-pins.json"
mac fail "differs from the pins compiled into" "a bundle whose agent was compiled without the pins fails"
mv "$work/compiled-pins.saved" "$work/compiled-pins.json"
macos 0.6.0-alpha.1
bundle_app "$work/bundle/macos/DeckPilot.app" 0.6.0-alpha.1 NSS
mac fail "carries a patched bundle type" "a macOS app patched as another package fails"
macos 0.6.0-alpha.1
app "$work/bundle/macos/DeckPilot.app/Contents/MacOS/deckpilot-desktop" 0.6.0-alpha.1 UNK mac_app_store
mac fail "built for the distribution channel 'mac_app_store'" "a bundle of another channel fails"
macos 0.6.0-alpha.1
: > "$work/bundle/macos/extra.txt"
tar -czf "$work/bundle/macos/DeckPilot.app.tar.gz" -C "$work/bundle/macos" DeckPilot.app extra.txt
sign 0.6.0-alpha.1 "$work/bundle/macos/DeckPilot.app.tar.gz"
mac fail "expected only DeckPilot.app" "an updater archive with more than the bundle fails"
macos 0.6.0-alpha.1
dmg 0.6.0-alpha.1 aarch64 no-link
mac fail "has no Applications link" "a disk image without the Applications link fails"
macos 0.6.0-alpha.1
rm -rf "$work/volume/DeckPilot.app/Contents/MacOS/deckpilot-desktop"
tar -cf "$work/bundle/dmg/DeckPilot_0.6.0-alpha.1_aarch64.dmg" -C "$work/volume" .
mac fail "the disk image: $work/macos-dmg/DeckPilot.app lacks the executable Contents/MacOS/deckpilot-desktop" "a disk image with a broken bundle fails"

if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed (work dir: $work)"
  exit 1
fi
echo "package checks: all checks passed"
