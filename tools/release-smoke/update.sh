#!/usr/bin/env bash
# The update path of a published release, as installed apps see it:
#
#   tools/release-smoke/update.sh <tag> <assets dir> [<previous tag> <previous assets dir>]
#
# - every release asset is listed in SHA256SUMS, and the other way round;
# - latest.json announces this version; each platform entry's URL
#   downloads (the public URL, as the updater fetches it), matches
#   SHA256SUMS, equals the .sig asset, and its signature verifies with
#   minisign against the pinned updater key (.github/updater.pub), naming
#   the file and the version in the signed comment;
# - continuity: the previous release's applications (deb, both macOS
#   archives, MSI, NSIS) embed that same updater key, so they accept this
#   release.
# Needs minisign, msitools (msiextract) and 7z.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-smoke/common.sh
source "$here/common.sh"

tag="${1:?usage: update.sh <tag> <assets dir> [<previous tag> <previous assets dir>]}"
dir="${2:?}"
require_tag "$tag"
version="${tag#v}"
prefix="https://github.com/knittingcode/DeckPilot/releases/download/"
work="${SMOKE_WORK:?}/update"
rm -rf "$work" && mkdir -p "$work/public"
pub="$work/updater.pub"
base64 -d < "$SMOKE_ROOT/.github/updater.pub" > "$pub"
key_id=$(sed -n 's/.*public key: //p' "$pub")

# Assets and SHA256SUMS agree.
assets=$(gh release view "$tag" -R "$REPOSITORY" --json assets --jq '.assets[].name' | grep -v -x SHA256SUMS | LC_ALL=C sort)
listed=$(awk '{ f = $2; sub(/^\*/, "", f); print f }' "$dir/SHA256SUMS" | tr -d '\r' | LC_ALL=C sort)
if [ "$assets" = "$listed" ]; then
  record "SHA256SUMS lists exactly the release assets" pass "$(printf '%s\n' "$assets" | wc -l | tr -d ' ') assets"
else
  record "SHA256SUMS lists exactly the release assets" fail "differences: $(diff <(echo "$assets") <(echo "$listed") | grep '^[<>]' | tr '\n' ' ')"
fi

manifest="$dir/latest.json"
announced=$(jq -r .version "$manifest")
if [ "$announced" = "$version" ]; then
  record "latest.json version" pass "version $announced, pub_date $(jq -r .pub_date "$manifest"), $(jq '.platforms | length' "$manifest") platform entries"
else
  record "latest.json version" fail "announces '$announced', expected $version"
fi
# The stable channel's URL, which installations poll.
if curl -fsSL --retry 3 -o "$work/stable.json" "${prefix%/download/}/latest/download/latest.json"; then
  if cmp -s "$work/stable.json" "$manifest"; then
    record "stable channel manifest (releases/latest/download/latest.json)" pass "identical to the $tag asset"
  else
    record "stable channel manifest (releases/latest/download/latest.json)" not-covered "the stable channel serves version $(jq -r .version "$work/stable.json"), not $tag (another release is latest)"
  fi
else
  record "stable channel manifest (releases/latest/download/latest.json)" fail "cannot be downloaded"
fi

while IFS=$'\t' read -r platform url signature; do
  step="latest.json $platform"
  name="${url##*/}"
  if [ "$url" != "$prefix$tag/$name" ]; then
    record "$step" fail "URL $url is not under $prefix$tag/"
    continue
  fi
  file="$work/public/$name"
  if [ ! -f "$file" ] && ! curl -fsSL --retry 3 -o "$file" "$url"; then
    record "$step" fail "$url does not download"
    continue
  fi
  want=$(awk -v n="$name" '{ f = $2; sub(/^\*/, "", f); if (f == n) print $1 }' "$dir/SHA256SUMS")
  got=$(sha256_of "$file")
  [ "$got" = "$want" ] || { record "$step" fail "$name downloaded from $url has sha256 $got, SHA256SUMS says $want"; continue; }
  printf '%s' "$signature" | base64 -d > "$work/$platform.minisig"
  if [ -f "$dir/$name.sig" ] && [ "$(tr -d '\r\n' < "$dir/$name.sig")" != "$(printf '%s' "$signature" | tr -d '\r\n')" ]; then
    record "$step" fail "the manifest's signature differs from the asset $name.sig"
    continue
  fi
  if out=$(minisign -V -q -p "$pub" -m "$file" -x "$work/$platform.minisig" 2>&1); then
    comment=$(sed -n 's/^trusted comment: //p' "$work/$platform.minisig")
    # The app binds a signature to the announced version only
    # (crates/deckpilot-update/src/signature.rs); the file name in the
    # comment is the bundler's, which for the macOS archives is the name
    # before the asset was renamed per architecture.
    note=""
    printf '%s' "$comment" | grep -q -F "file:$name" || note=" (the comment names the bundler's file name, not $name; the app checks only the version)"
    if printf '%s' "$comment" | grep -q -E "version:${version//./\\.}([[:space:]]|$)"; then
      record "$step" pass "$name: sha256 ok, minisign ok with key $key_id; trusted comment: $comment$note"
    else
      record "$step" fail "signature verifies but the trusted comment does not name version $version: $comment"
    fi
  else
    record "$step" fail "minisign: $out"
  fi
done < <(jq -r '.platforms | to_entries[] | [.key, .value.url, .value.signature] | @tsv' "$manifest")

# Continuity: the previous release accepts updates signed with this key.
previous="${3-}"
pdir="${4-}"
if [ -z "$previous" ]; then
  record "update continuity" not-covered "no previous release given"
  exit 0
fi
require_tag "$previous"
pversion="${previous#v}"
x="$work/previous"
mkdir -p "$x"
key=$(tr -d '\r\n' < "$SMOKE_ROOT/.github/updater.pub")
continuity() {
  local what="$1" binary="$2"
  if [ ! -f "$binary" ]; then
    record "update continuity: $previous $what" fail "the desktop binary was not found in the package"
  elif grep -q -a -F -- "$key" "$binary"; then
    record "update continuity: $previous $what" pass "$(basename "$binary") of $previous embeds the key $key_id that signed $tag"
  else
    record "update continuity: $previous $what" fail "$(basename "$binary") of $previous does not embed the key $key_id that signed $tag"
  fi
}
if [ -f "$pdir/DeckPilot_${pversion}_amd64.deb" ]; then
  dpkg-deb -x "$pdir/DeckPilot_${pversion}_amd64.deb" "$x/deb"
  continuity "deb" "$x/deb/usr/bin/deckpilot-desktop"
fi
for arch in aarch64 x64; do
  if [ -f "$pdir/DeckPilot_${arch}.app.tar.gz" ]; then
    mkdir -p "$x/mac-$arch"
    tar -xzf "$pdir/DeckPilot_${arch}.app.tar.gz" -C "$x/mac-$arch"
    continuity "macOS $arch" "$x/mac-$arch/DeckPilot.app/Contents/MacOS/deckpilot-desktop"
  fi
done
if [ -f "$pdir/DeckPilot_${pversion}_x64_en-US.msi" ]; then
  mkdir -p "$x/msi"
  (cd "$x/msi" && msiextract "$pdir/DeckPilot_${pversion}_x64_en-US.msi" > /dev/null 2>&1)
  continuity "MSI" "$(find "$x/msi" -type f -iname deckpilot-desktop.exe | head -n1)"
fi
if [ -f "$pdir/DeckPilot_${pversion}_x64-setup.exe" ]; then
  mkdir -p "$x/nsis"
  7z x -y -o"$x/nsis" "$pdir/DeckPilot_${pversion}_x64-setup.exe" > /dev/null 2>&1
  continuity "NSIS" "$(find "$x/nsis" -type f -iname deckpilot-desktop.exe | head -n1)"
fi
record "in-app update $previous -> $tag" not-covered "an automated in-app update of the installed $previous to $tag (download, install, restart) is not driven here; the deb package upgrade is checked in the Linux job"
exit 0
