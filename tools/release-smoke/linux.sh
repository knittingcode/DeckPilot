#!/usr/bin/env bash
# The published Linux packages, on a GitHub-hosted Ubuntu runner:
#
#   tools/release-smoke/linux.sh deb      <assets dir> <version> [<previous assets dir> <previous version>]
#   tools/release-smoke/linux.sh appimage <assets dir> <version>
#   tools/release-smoke/linux.sh rpm      <assets dir> <version>
#
# deb: installs the previous release's .deb (if given), then this one over
#   it with apt (the package upgrade), and checks the installed files.
# appimage: extracts the AppImage (--appimage-extract) and checks its files;
#   the launch runs the AppImage file itself (session.sh).
# rpm: checks the package's digests and file list, extracts and checks its
#   files, and installs it in a Fedora container.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-smoke/common.sh
source "$here/common.sh"

kind="${1:?usage: linux.sh deb|appimage|rpm <assets dir> <version>}"
dir="${2:?}"
version="${3:?}"

# layout <step prefix> <root>: the files of an installed or extracted package.
layout() {
  local p="$1" root="$2"
  check_exec "$p: binaries present and executable" \
    "$root/usr/bin/deckpilot-desktop" "$root/usr/bin/deckpilot-agent" "$root/usr/bin/deckpilot-plugin-host"
  check_version "$p: agent version" "$root/usr/bin/deckpilot-agent" "$version"
  check_bundled "$p: bundled plugins and pins.json" "$root/usr/lib/DeckPilot/bundled" "$root/usr/bin/deckpilot-agent"
  check_roots "$p: production registry roots embedded in the agent" "$root/usr/bin/deckpilot-agent"
  check_updater_key "$p: updater key embedded in the app" "$root/usr/bin/deckpilot-desktop"
}

case "$kind" in
  deb)
    deb="$dir/DeckPilot_${version}_amd64.deb"
    [ -f "$deb" ] || { record "deb: package" error "$deb was not downloaded"; exit 0; }
    package=$(dpkg-deb -f "$deb" Package)
    if [ -n "${4-}" ] && [ -n "${5-}" ]; then
      previous="$4/DeckPilot_${5}_amd64.deb"
      if out=$(sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q "$previous" 2>&1); then
        record "deb: install previous release $5" pass "$(dpkg-query -W -f '${Package} ${Version} ${Status}' "$package")"
      else
        record "deb: install previous release $5" fail "$(printf '%s' "$out" | tail -n 3)"
      fi
    fi
    if out=$(sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q "$deb" 2>&1); then
      installed=$(dpkg-query -W -f '${Version}' "$package")
      if [ "$installed" = "$version" ]; then
        record "deb: apt-get install${4:+ (upgrade from ${5-})}" pass "$(dpkg-query -W -f '${Package} ${Version} ${Status}' "$package"); $(printf '%s' "$out" | grep -E -m1 'Unpacking|Setting up' || true)"
      else
        record "deb: apt-get install" fail "$package is at '$installed' after the install, expected $version"
      fi
    else
      record "deb: apt-get install" fail "$(printf '%s' "$out" | tail -n 3)"
      exit 0
    fi
    layout deb ""
    for path in /usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules /usr/share/applications/DeckPilot.desktop; do
      [ -f "$path" ] || missing="${missing-} $path"
    done
    if [ -z "${missing-}" ]; then
      record "deb: udev rule and desktop entry" pass "/usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules; Exec=$(sed -n 's/^Exec=//p' /usr/share/applications/DeckPilot.desktop | head -n1)"
    else
      record "deb: udev rule and desktop entry" fail "missing:$missing"
    fi
    ;;
  appimage)
    appimage="$dir/DeckPilot_${version}_amd64.AppImage"
    [ -f "$appimage" ] || { record "AppImage: file" error "$appimage was not downloaded"; exit 0; }
    chmod +x "$appimage"
    extracted="${SMOKE_WORK:?}/appimage-extracted"
    rm -rf "$extracted" && mkdir -p "$extracted"
    if (cd "$extracted" && "$appimage" --appimage-extract > /dev/null 2>&1); then
      record "AppImage: --appimage-extract" pass "extracted to squashfs-root ($(find "$extracted/squashfs-root" -type f | wc -l) files; bundled appindicator libraries: $(find "$extracted/squashfs-root" -iname '*appindicator*' -printf '%P ' | head -c 300))"
    else
      record "AppImage: --appimage-extract" fail "$appimage does not extract"
      exit 0
    fi
    layout AppImage "$extracted/squashfs-root"
    ;;
  rpm)
    rpm="$dir/DeckPilot-${version}-1.x86_64.rpm"
    [ -f "$rpm" ] || { record "rpm: package" error "$rpm was not downloaded"; exit 0; }
    if out=$(rpm -K --nosignature "$rpm" 2>&1); then
      record "rpm: header and payload digests (rpm -K)" pass "$out"
    else
      record "rpm: header and payload digests (rpm -K)" fail "$out"
    fi
    files=$(rpm -qlp "$rpm" 2> /dev/null)
    missing=""
    for path in /usr/bin/deckpilot-desktop /usr/bin/deckpilot-agent /usr/bin/deckpilot-plugin-host \
      /usr/lib/DeckPilot/bundled/pins.json /usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules; do
      printf '%s\n' "$files" | grep -q -x -F "$path" || missing="$missing $path"
    done
    if [ -z "$missing" ]; then
      record "rpm: file list (rpm -qlp)" pass "$(printf '%s\n' "$files" | wc -l) files, $(rpm -qp --queryformat '%{NAME} %{VERSION}-%{RELEASE}' "$rpm")"
    else
      record "rpm: file list (rpm -qlp)" fail "missing:$missing"
    fi
    extracted="${SMOKE_WORK:?}/rpm-extracted"
    rm -rf "$extracted" && mkdir -p "$extracted"
    # rpm2cpio exits 1 on the bundler's .rpm although it writes the whole
    # payload (tools/ci/check-packages.sh, extract_rpm); the files are
    # checked instead.
    rpm2cpio "$rpm" > "$extracted.cpio" 2> /dev/null || true
    (cd "$extracted" && cpio -idm --quiet < "$extracted.cpio") 2> /dev/null
    rm -f "$extracted.cpio"
    layout rpm "$extracted"
    # Installed for real in Fedora (dependencies from Fedora's repositories).
    image="fedora:42"
    name=$(basename "$rpm")
    if out=$(timeout 900 docker run --rm -v "$dir:/assets:ro" "$image" bash -c \
      "dnf -y -q install /assets/$name > /tmp/dnf.log 2>&1 || { tail -n 20 /tmp/dnf.log; exit 1; }
       rpm -q deck-pilot && rpm -V deck-pilot && echo verified
       deckpilot-agent --version
       test -x /usr/bin/deckpilot-desktop && test -x /usr/bin/deckpilot-plugin-host && test -f /usr/lib/DeckPilot/bundled/pins.json && echo layout-ok" 2>&1); then
      if printf '%s' "$out" | grep -q -x "deckpilot-agent $version" && printf '%s' "$out" | grep -q -x layout-ok && printf '%s' "$out" | grep -q -x verified; then
        record "rpm: dnf install in $image" pass "$(printf '%s' "$out" | tr '\n' ' ')"
      else
        record "rpm: dnf install in $image" fail "$(printf '%s' "$out" | tr '\n' ' ')"
      fi
    else
      record "rpm: dnf install in $image" fail "$(printf '%s' "$out" | tail -n 8 | tr '\n' ' ')"
    fi
    ;;
  *) echo "unknown kind $kind" >&2; exit 2 ;;
esac
exit 0
