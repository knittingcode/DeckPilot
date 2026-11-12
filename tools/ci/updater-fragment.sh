#!/usr/bin/env bash
# Writes the update manifest fragment of one release build: for each
# updater target key of the build, the release asset name of the package and
# its signature (the content of the bundler's .sig file).
#
#   tools/ci/updater-fragment.sh <rust target> <bundle dir> <output.json>
#
# The fragments of all builds are combined once, by assemble-latest-json.sh,
# instead of every build job merging into latest.json in the draft release
# (concurrent merges can lose entries). Target keys and asset names are those
# of tauri-action: the macOS updater archive is uploaded as
# DeckPilot_<aarch64|x64>.app.tar.gz, every other package under its own name.
set -euo pipefail

target="${1:?usage: updater-fragment.sh <rust target> <bundle dir> <output.json>}"
bundle="${2:?usage: updater-fragment.sh <rust target> <bundle dir> <output.json>}"
output="${3:?usage: updater-fragment.sh <rust target> <bundle dir> <output.json>}"
fail() {
  echo "::error::$*" >&2
  exit 1
}

entries=()
# add <key>... -- <file> <asset name>
add() {
  local keys=() file asset signature
  while [ "$1" != "--" ]; do
    keys+=("$1")
    shift
  done
  file="$2" asset="$3"
  [ -f "$file" ] || fail "$target: the updater package $file was not built"
  [ -s "$file.sig" ] || fail "$target: $file has no updater signature ($file.sig)"
  signature=$(tr -d '\r\n' < "$file.sig")
  for key in "${keys[@]}"; do
    entries+=("$(jq -n --arg key "$key" --arg asset "$asset" --arg signature "$signature" \
      '{key: $key, value: {asset: $asset, signature: $signature}}')")
  done
}

# The single file matching a pattern in a directory.
only() {
  local dir="$1" pattern="$2" found
  found=$(find "$dir" -maxdepth 1 -type f -name "$pattern" ! -name '*.sig' 2>/dev/null)
  [ -n "$found" ] || fail "$target: no $pattern in $dir"
  [ "$(echo "$found" | wc -l)" -eq 1 ] || fail "$target: several $pattern in $dir: $(echo "$found" | tr "\n" " ")"
  echo "$found"
}

case "$target" in
  aarch64-apple-darwin | x86_64-apple-darwin)
    arch=${target%%-*}
    label=$([ "$arch" = aarch64 ] && echo aarch64 || echo x64)
    add "darwin-$arch" "darwin-$arch-app" -- \
      "$(only "$bundle/macos" '*.app.tar.gz')" "DeckPilot_$label.app.tar.gz"
    ;;
  x86_64-unknown-linux-gnu)
    appimage=$(only "$bundle/appimage" '*.AppImage')
    add linux-x86_64 linux-x86_64-appimage -- "$appimage" "$(basename "$appimage")"
    deb=$(only "$bundle/deb" '*.deb')
    add linux-x86_64-deb -- "$deb" "$(basename "$deb")"
    # Test releases (pre-release versions) build no .rpm.
    if [ -d "$bundle/rpm" ]; then
      rpm=$(only "$bundle/rpm" '*.rpm')
      add linux-x86_64-rpm -- "$rpm" "$(basename "$rpm")"
    fi
    ;;
  x86_64-pc-windows-msvc)
    msi=$(only "$bundle/msi" '*.msi')
    add windows-x86_64 windows-x86_64-msi -- "$msi" "$(basename "$msi")"
    nsis=$(only "$bundle/nsis" '*-setup.exe')
    add windows-x86_64-nsis -- "$nsis" "$(basename "$nsis")"
    ;;
  *) fail "unknown release target $target" ;;
esac

mkdir -p "$(dirname "$output")"
printf '%s\n' "${entries[@]}" | jq -s '{platforms: from_entries}' > "$output"
echo "$target: $(jq -r '.platforms | to_entries | map("\(.key) = \(.value.asset)") | join(", ")' "$output")"
