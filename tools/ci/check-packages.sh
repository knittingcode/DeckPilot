#!/usr/bin/env bash
# Checks the packages built by a CI package check against what a release
# must contain:
#
#   tools/ci/check-packages.sh linux
#   tools/ci/check-packages.sh windows <dir with the extracted installers>
#
# - the expected file names for the version in tauri.conf.json;
# - the package version (.deb, .rpm) and the application's version (.exe);
# - the application, the bundled agent, which reports the same version, and
#   on Linux the udev rule and the desktop entry;
# - an updater signature next to each package, made with the throwaway key
#   (THROWAWAY_UPDATER_PUBKEY, see throwaway-updater-key.sh) and naming the
#   version, as installed apps require.
#
# Windows: <dir>/nsis and <dir>/msi hold the extracted installers.
#
# Environment:
#   PRERELEASE=true   a pre-release version (e.g. 0.6.0-alpha.1): no .rpm is
#                     expected (RPM does not allow "-" in versions), and the
#                     Windows executable's version resource may hold only the
#                     numeric part (0.6.0)
#   PACKAGE_VERSION   the expected version; default: tauri.conf.json's
#   BUNDLE_DIR        default: target/release/bundle
#   DECKPILOT_DEV     how to run deckpilot-dev; default: a release build
set -euo pipefail

platform="${1:?usage: check-packages.sh linux|windows [extract-dir]}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
version="${PACKAGE_VERSION:-$(jq -r .version apps/desktop/src-tauri/tauri.conf.json)}"
bundle="${BUNDLE_DIR:-target/release/bundle}"
prerelease="${PRERELEASE:-false}"
case "$prerelease" in true | false) ;; *) echo "PRERELEASE must be true or false" >&2; exit 2 ;; esac
case "$version" in
  *-*) [ "$prerelease" = true ] || { echo "::error::$version is a pre-release version; set PRERELEASE=true" >&2; exit 1; } ;;
  *) [ "$prerelease" = false ] || { echo "::error::$version is not a pre-release version, but PRERELEASE=true" >&2; exit 1; } ;;
esac
fail() {
  echo "::error::$*"
  exit 1
}

# The file must exist; otherwise list what was built.
expect_file() {
  [ -f "$1" ] || fail "expected $1, built: $(find "$bundle" -maxdepth 2 -type f | sed "s#^$bundle/##" | tr '\n' ' ')"
}

agent_reports_version() {
  local agent="$1" reported
  reported=$("$agent" --version | tr -d '\r')
  [ "$reported" = "deckpilot-agent $version" ] \
    || fail "the bundled agent $agent reports '$reported', expected deckpilot-agent $version"
  echo "bundled agent: $reported"
}

# The updater signature of a package: present, valid for the throwaway key,
# and naming the version in its signed comment.
signed_for_version() {
  local file="$1" signature="$1.sig"
  [ -s "$signature" ] || fail "$file has no updater signature ($signature)"
  base64 -d < "$signature" | grep -q -E "^trusted comment: .*version:${version//./\\.}([[:space:]]|$)" \
    || fail "the updater signature of $file does not name version $version"
  # shellcheck disable=SC2086 # DECKPILOT_DEV is a command with arguments.
  DECKPILOT_UPDATER_PUBKEY="${THROWAWAY_UPDATER_PUBKEY:?run throwaway-updater-key.sh first}" \
    ${DECKPILOT_DEV:-cargo run --release --locked -q -p deckpilot-dev --} verify-signature "$file" "$signature"
}

case "$platform" in
  linux)
    deb="$bundle/deb/DeckPilot_${version}_amd64.deb"
    rpm="$bundle/rpm/DeckPilot-${version}-1.x86_64.rpm"
    expect_file "$deb"
    if [ "$prerelease" = true ]; then
      [ ! -e "$rpm" ] || fail "$rpm was built for a pre-release version"
    else
      expect_file "$rpm"
    fi

    contents=$(dpkg-deb --contents "$deb")
    echo "$contents"
    # dpkg-deb lists paths with or without a leading "./".
    for path in usr/bin/deckpilot-desktop usr/bin/deckpilot-agent \
      usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules 'usr/share/applications/.*\.desktop'; do
      echo "$contents" | grep -q -E " (\./)?$path\$" || fail "$deb lacks $path"
    done
    deb_version=$(dpkg-deb -f "$deb" Version)
    [ "$deb_version" = "$version" ] || fail "$deb has version $deb_version, expected $version"
    extracted="${RUNNER_TEMP:?}/deb"
    rm -rf "$extracted"
    dpkg-deb -x "$deb" "$extracted"
    agent_reports_version "$extracted/usr/bin/deckpilot-agent"
    echo "$deb: version $version, app, agent, udev rule and desktop entry"

    signed_for_version "$deb"
    if [ "$prerelease" = true ]; then
      echo "pre-release $version: no .rpm (RPM does not allow \"-\" in versions)"
    else
      files=$(rpm -qlp "$rpm")
      echo "$files"
      for path in /usr/bin/deckpilot-desktop /usr/bin/deckpilot-agent \
        /usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules; do
        echo "$files" | grep -q -x -F "$path" || fail "$rpm lacks $path"
      done
      rpm_version=$(rpm -qp --queryformat '%{VERSION}' "$rpm")
      [ "$rpm_version" = "$version" ] || fail "$rpm has version $rpm_version, expected $version"
      echo "$rpm: version $version, app, agent and udev rule"
      signed_for_version "$rpm"
    fi
    ;;
  windows)
    extracted="${2:?usage: check-packages.sh windows <extract-dir>}"
    nsis="$bundle/nsis/DeckPilot_${version}_x64-setup.exe"
    msi="$bundle/msi/DeckPilot_${version}_x64_en-US.msi"
    expect_file "$nsis"
    expect_file "$msi"
    for kind in nsis msi; do
      files=$(find "$extracted/$kind" -type f -iname '*.exe' | sed 's#.*/##' | sort -u)
      echo "$kind: $(echo "$files" | tr '\n' ' ')"
      for exe in deckpilot-desktop.exe deckpilot-agent.exe; do
        echo "$files" | grep -q -i -x -F "$exe" || fail "the $kind installer lacks $exe"
      done
      agent_reports_version "$(find "$extracted/$kind" -type f -iname deckpilot-agent.exe | head -n 1)"
      app=$(find "$extracted/$kind" -type f -iname deckpilot-desktop.exe | head -n 1)
      app_version=$(pwsh -NoProfile -Command "(Get-Item -LiteralPath '$(cygpath -w "$app")').VersionInfo.ProductVersion" | tr -d '\r')
      # A pre-release's version resource may hold only its numeric part.
      numeric="${version%%-*}"
      case "$app_version" in
        "$version" | "$version".*) ;;
        "$numeric" | "$numeric".*) [ "$prerelease" = true ] || fail "deckpilot-desktop.exe in the $kind installer has version '$app_version', expected $version" ;;
        *) fail "deckpilot-desktop.exe in the $kind installer has version '$app_version', expected $version" ;;
      esac
      echo "$kind: deckpilot-desktop.exe $app_version"
    done
    signed_for_version "$nsis"
    signed_for_version "$msi"
    ;;
  *) fail "unknown platform $platform" ;;
esac
echo "packages for $platform: checked"
