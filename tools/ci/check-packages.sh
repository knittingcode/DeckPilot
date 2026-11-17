#!/usr/bin/env bash
# Checks the packages built by a CI package check against what a release
# must contain:
#
#   tools/ci/check-packages.sh linux
#   tools/ci/check-packages.sh windows <dir with the extracted installers>
#   tools/ci/check-packages.sh macos
#   tools/ci/check-packages.sh record <binary> <format> [optional]
#
# - the expected file names for the version in tauri.conf.json;
# - the package version (.deb, .rpm), the application's version (.exe) and
#   the bundle's version and identifier (macOS Info.plist);
# - the layout: the application, the bundled agent, which reports the same
#   version, on Linux the udev rule (.deb, .rpm), the desktop entry (.deb,
#   AppImage) and AppRun (AppImage), on macOS the bundle executable, icon
#   and, in the disk image, the Applications link;
# - the plugin runtime, an application component of every package
#   (docs/marketplace/module-migration.md, section 4.5): the plugin host
#   next to the agent, and the bundled first-party plugins in the
#   application's resources (bundled/), exactly the release set of
#   plugins/first-party/bundled.json, matching their pins and the pins
#   compiled into the bundled agent
#   (tools/release-pipeline/check-bundled-plugins.sh);
# - the distribution record compiled into the application (and into the
#   agent, once it links deckpilot-distribution): the channel
#   DISTRIBUTION_CHANNEL, the version, no development override
#   (docs/distribution/README.md, section 2.2), and the package format the
#   bundler patched into each package's copy of the application;
# - an updater signature next to each package, made with the throwaway key
#   (THROWAWAY_UPDATER_PUBKEY, see throwaway-updater-key.sh) and naming the
#   version, as installed apps require.
#
# Linux: .deb, AppImage and, for final versions, .rpm. Windows: <dir>/nsis
# and <dir>/msi hold the extracted installers. macOS: the .app, its updater
# archive (.app.tar.gz) and the disk image of one architecture.
#
# `record <binary> <format>` checks one binary's record and prints it as
# key=value lines (distribution_channel, package_format,
# application_version). <format> is the package the binary comes from:
# deb, rpm, appimage, msi or nsis (patched by the bundler), app (a macOS
# bundle, never patched) or none (the build output, not yet packaged).
# With `optional`, a binary without a record (the agent, until it links
# deckpilot-distribution) passes and prints nothing; the development
# override is refused either way.
#
# Environment:
#   PRERELEASE=true   a pre-release version (e.g. 0.6.0-alpha.1): no .rpm is
#                     expected (RPM does not allow "-" in versions), and the
#                     Windows executable's version resource may hold only the
#                     numeric part (0.6.0)
#   PACKAGE_VERSION   the expected version; default: tauri.conf.json's
#   BUNDLE_DIR        default: target/release/bundle
#   DISTRIBUTION_CHANNEL  the expected channel; default: github
#   MACOS_ARCH        aarch64 or x64 (the disk image's name); default: the host's
#   HDIUTIL           how to run hdiutil (macOS disk images); default: hdiutil
#   DECKPILOT_DEV     how to run deckpilot-dev; default: a release build
set -euo pipefail
# A command that ends the script under `set -e` names itself (fail() is
# not involved then, and the log would otherwise show only the exit code).
set -E
trap 'echo "::error::check-packages.sh: line $LINENO: \"$BASH_COMMAND\" failed with exit status $?"' ERR

platform="${1:?usage: check-packages.sh linux|windows [extract-dir]|macos|record <binary> <format>}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
version="${PACKAGE_VERSION:-$(jq -r .version apps/desktop/src-tauri/tauri.conf.json)}"
bundle="${BUNDLE_DIR:-target/release/bundle}"
prerelease="${PRERELEASE:-false}"
channel="${DISTRIBUTION_CHANNEL:-github}"
identifier=io.github.knittingcode.deckpilot
fail() {
  echo "::error::$*"
  exit 1
}

# The record deckpilot-distribution compiles into every binary that links it
# (RECORD in crates/deckpilot-distribution/src/lib.rs).
RECORD_PATTERN='deckpilot-distribution-record:1;channel=[a-z_]*;format=[a-z_]*;version=[0-9A-Za-z.+-]*;end'
# Only debug builds may contain the development override's variable; spelled
# in two parts so that this script never matches itself.
OVERRIDE_VARIABLE="DECKPILOT_DISTRIBUTION""_OVERRIDE"
# The bundler's per-package marker (tauri-utils platform::bundle_type): it
# replaces the first __TAURI_BUNDLE_TYPE_VAR_UNK of the application with the
# package's token; the other tokens stay in the binary as string constants.
BUNDLE_TOKENS="DEB RPM APP MSI NSS"

count() { { grep -a -o -F -- "$1" "$2" || true; } | wc -l | tr -d ' '; }

# bundle_token <format>: the bundler's token of a patched format.
bundle_token() {
  case "$1" in
    deb) echo DEB ;;
    rpm) echo RPM ;;
    appimage) echo APP ;;
    msi) echo MSI ;;
    nsis) echo NSS ;;
    *) echo "" ;;
  esac
}

# The bundle type a package's copy of the application carries: the token of
# <format> occurs more often than any other and UNK not at all, or, for the
# macOS bundle and the unpackaged build output, no token was patched in.
check_bundle_type() {
  local binary="$1" format="$2" token
  token=$(bundle_token "$format")
  local name counts="" n expected=0 unknown
  unknown=$(count "__TAURI_BUNDLE_TYPE_VAR_UNK" "$binary")
  for name in $BUNDLE_TOKENS; do
    n=$(count "__TAURI_BUNDLE_TYPE_VAR_$name" "$binary")
    counts="$counts $name=$n"
    [ "$name" = "$token" ] && expected=$n
  done
  if [ -n "$token" ]; then
    [ "$unknown" -eq 0 ] && [ "$expected" -gt 0 ] \
      || fail "$binary is not marked as $format by the bundler (UNK=$unknown$counts): the updater would not know its package format"
    for name in $BUNDLE_TOKENS; do
      [ "$name" = "$token" ] && continue
      n=$(count "__TAURI_BUNDLE_TYPE_VAR_$name" "$binary")
      [ "$expected" -gt "$n" ] \
        || fail "$binary is not marked as $format by the bundler (UNK=$unknown$counts)"
    done
  else
    local first=""
    for name in $BUNDLE_TOKENS; do
      n=$(count "__TAURI_BUNDLE_TYPE_VAR_$name" "$binary")
      [ -z "$first" ] && first=$n
      [ "$n" = "$first" ] || fail "$binary ($format) carries a patched bundle type ($counts)"
    done
  fi
}

# distribution_record <binary> <format> [optional]: the binary's compiled
# record names the expected channel and version, no format for a GitHub
# build (the bundler's marker is its format), and no development override.
# With `optional`, a binary without a record passes (the agent does not
# link deckpilot-distribution yet).
distribution_record() {
  local binary="$1" format="$2" optional="${3-}"
  [ -f "$binary" ] || fail "$binary does not exist"
  ! grep -q -a -F -- "$OVERRIDE_VARIABLE" "$binary" \
    || fail "$binary contains $OVERRIDE_VARIABLE: a development override is compiled in (a debug build?)"
  local records
  records=$(grep -a -o -E -- "$RECORD_PATTERN" "$binary" | sort -u || true)
  if [ -z "$records" ]; then
    [ "$optional" = optional ] && {
      echo "$binary: no distribution record (does not link deckpilot-distribution)"
      return 0
    }
    fail "$binary carries no distribution record (deckpilot-distribution is not linked?)"
  fi
  [ "$(wc -l <<< "$records" | tr -d ' ')" -eq 1 ] || fail "$binary carries several distribution records: $(tr '\n' ' ' <<< "$records")"
  local fields recorded_channel recorded_format recorded_version
  fields="${records#deckpilot-distribution-record:1;}"
  fields="${fields%;end}"
  recorded_channel=$(tr ';' '\n' <<< "$fields" | sed -n 's/^channel=//p')
  recorded_format=$(tr ';' '\n' <<< "$fields" | sed -n 's/^format=//p')
  recorded_version=$(tr ';' '\n' <<< "$fields" | sed -n 's/^version=//p')
  [ "$recorded_channel" = "$channel" ] \
    || fail "$binary was built for the distribution channel '$recorded_channel', expected '$channel' (DECKPILOT_DISTRIBUTION_CHANNEL)"
  [ "$recorded_version" = "$version" ] || fail "$binary records version $recorded_version, expected $version"
  if [ "$channel" = github ]; then
    # A GitHub build's format comes from the bundler, per package.
    [ -z "$recorded_format" ] || fail "$binary has the package format '$recorded_format' compiled in; a GitHub build gets it from the bundler"
  else
    [ "$recorded_format" = "$format" ] || fail "$binary records the package format '$recorded_format', expected '$format'"
  fi
  [ "$optional" = optional ] || check_bundle_type "$binary" "$format"
  echo "$binary: distribution channel $recorded_channel, version $recorded_version, package format ${format}"
}

# The file must exist; otherwise list what was built.
expect_file() {
  [ -f "$1" ] || fail "expected $1, built: $(find "$bundle" -maxdepth 2 -type f | sed "s#^$bundle/##" | tr '\n' ' ')"
}

# The plugin ids the application must bundle: those of
# plugins/first-party/bundled.json (never a test fixture), sorted.
release_plugin_ids() {
  local path
  jq -r '.plugins[].path' plugins/first-party/bundled.json | tr -d '\r' | while IFS= read -r path; do
    jq -r .id "plugins/first-party/$path/manifest.json" | tr -d '\r'
  done | LC_ALL=C sort
}

# plugin_runtime <host> <bundled dir> <agent> <what>: the plugin host is
# an executable file; bundled/ holds exactly the release set of first-party
# plugins, matching their pins and the pins compiled into <agent>.
plugin_runtime() {
  local host="$1" bundled="$2" agent="$3" what="$4" ids expected
  [ -f "$host" ] && [ -x "$host" ] || fail "$what lacks the plugin host $host"
  [ -f "$bundled/pins.json" ] || fail "$what lacks the bundled plugins ($bundled/pins.json)"
  ids=$(jq -r '.plugins[].id' "$bundled/pins.json" | tr -d '\r' | LC_ALL=C sort)
  expected=$(release_plugin_ids)
  [ "$ids" = "$expected" ] \
    || fail "$what bundles the plugins $(echo "$ids" | tr '\n' ' '), expected those of plugins/first-party/bundled.json: $(echo "$expected" | tr '\n' ' ')"
  "$root/tools/release-pipeline/check-bundled-plugins.sh" "$bundled" "$agent" \
    || fail "$what: the bundled plugins do not match their pins or the agent's"
  echo "$what: plugin host and bundled plugins $(echo "$ids" | tr '\n' ' ')"
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

# extract_rpm <package> <dir>: unpacks an .rpm's payload into <dir> and
# checks every file against the SHA-256 digest in the package header.
# rpm2cpio's exit status is not used: the Tauri bundler writes its .rpm with
# the `rpm` crate, which sets no archive size tag, and rpm2cpio (rpm 4.18,
# Ubuntu 24.04) then exits 1 although it wrote the whole payload. The
# package's own digests (rpm -K), the extraction and each file's digest are
# checked instead, which is stricter than the exit status was.
extract_rpm() {
  local rpm="$1" dir="$2" algo digest path actual
  rpm -K --nosignature "$rpm" > /dev/null || fail "$rpm: its header or payload digest does not verify"
  rpm2cpio "$rpm" > "$dir.cpio" || true
  (cd "$dir" && cpio -idm --quiet < "$dir.cpio") || fail "$rpm: its payload cannot be extracted"
  rm -f "$dir.cpio"
  algo=$(rpm -qp --queryformat '%{FILEDIGESTALGO}' "$rpm")
  [ "$algo" = 8 ] || fail "$rpm: its file digests use algorithm $algo, expected 8 (SHA-256)"
  while IFS='|' read -r digest path; do
    [ -n "$digest" ] || continue # a directory
    [ -f "$dir$path" ] || fail "$rpm: $path is missing from its payload"
    actual=$(sha256sum "$dir$path" | cut -d ' ' -f 1)
    [ "$actual" = "$digest" ] || fail "$rpm: $path does not match the digest in its header"
  done < <(rpm -qp --queryformat '[%{FILEDIGESTS}|%{FILENAMES}\n]' "$rpm")
}

# fresh_dir <name>: an empty directory for extracted files.
fresh_dir() {
  local dir="${RUNNER_TEMP:?}/$1"
  rm -rf "$dir"
  mkdir -p "$dir"
  echo "$dir"
}

# plist_value <Info.plist> <key>
plist_value() {
  python3 -I -c 'import plistlib, sys; print(plistlib.load(open(sys.argv[1], "rb")).get(sys.argv[2], ""))' "$1" "$2"
}

# macos_app <DeckPilot.app> <what>: the layout, version and identifier of a
# bundle, its bundled agent and its distribution record.
macos_app() {
  local app="$1" what="$2" plist="$1/Contents/Info.plist" executable
  [ -d "$app" ] || fail "$what: $app does not exist"
  [ -f "$plist" ] || fail "$what: $app has no Contents/Info.plist"
  [ "$(plist_value "$plist" CFBundleIdentifier)" = "$identifier" ] \
    || fail "$what: CFBundleIdentifier is '$(plist_value "$plist" CFBundleIdentifier)', expected $identifier"
  [ "$(plist_value "$plist" CFBundleShortVersionString)" = "$version" ] \
    || fail "$what: CFBundleShortVersionString is '$(plist_value "$plist" CFBundleShortVersionString)', expected $version"
  executable=$(plist_value "$plist" CFBundleExecutable)
  [ "$executable" = deckpilot-desktop ] || fail "$what: CFBundleExecutable is '$executable', expected deckpilot-desktop"
  for path in Contents/MacOS/deckpilot-desktop Contents/MacOS/deckpilot-agent; do
    [ -f "$app/$path" ] && [ -x "$app/$path" ] || fail "$what: $app lacks the executable $path"
  done
  local icon
  icon=$(plist_value "$plist" CFBundleIconFile)
  [ -n "$icon" ] && [ -f "$app/Contents/Resources/$icon" ] || fail "$what: $app lacks its icon (CFBundleIconFile '$icon')"
  # The plugin host is in Contents/MacOS, and only there (targets.toml:
  # plugin_host_install = "contents_macos_own_entitlement"); the bundled
  # plugins in Contents/Resources/bundled.
  local hosts
  hosts=$(cd "$app" && find Contents -name 'deckpilot-plugin-host*' | sort | tr '\n' ' ')
  [ "$hosts" = "Contents/MacOS/deckpilot-plugin-host " ] \
    || fail "$what: $app has the plugin host at '$hosts', expected exactly Contents/MacOS/deckpilot-plugin-host"
  agent_reports_version "$app/Contents/MacOS/deckpilot-agent"
  plugin_runtime "$app/Contents/MacOS/deckpilot-plugin-host" "$app/Contents/Resources/bundled" \
    "$app/Contents/MacOS/deckpilot-agent" "$what"
  distribution_record "$app/Contents/MacOS/deckpilot-desktop" app
  distribution_record "$app/Contents/MacOS/deckpilot-agent" none optional
  echo "$what: $identifier $version, app, agent and icon"
}

case "$platform" in
  record)
    binary="${2:?usage: check-packages.sh record <binary> <format> [optional]}"
    format="${3:?usage: check-packages.sh record <binary> <format> [optional]}"
    case "$format" in deb | rpm | appimage | msi | nsis | app | none) ;; *) fail "unknown package format $format" ;; esac
    if [ "${4-}" = optional ] && [ -f "$binary" ] && ! grep -q -a -E -- "$RECORD_PATTERN" "$binary"; then
      # Prints nothing: there is no record to report.
      distribution_record "$binary" "$format" optional >&2
      exit 0
    fi
    distribution_record "$binary" "$format" "${4-}" >&2
    echo "distribution_channel=$channel"
    echo "package_format=$([ "$channel" = github ] && echo "${format/none/}" || echo "$format")"
    echo "application_version=$version"
    exit 0
    ;;
esac

case "$prerelease" in true | false) ;; *) echo "PRERELEASE must be true or false" >&2; exit 2 ;; esac
case "$version" in
  *-*) [ "$prerelease" = true ] || { echo "::error::$version is a pre-release version; set PRERELEASE=true" >&2; exit 1; } ;;
  *) [ "$prerelease" = false ] || { echo "::error::$version is not a pre-release version, but PRERELEASE=true" >&2; exit 1; } ;;
esac

case "$platform" in
  linux)
    deb="$bundle/deb/DeckPilot_${version}_amd64.deb"
    rpm="$bundle/rpm/DeckPilot-${version}-1.x86_64.rpm"
    appimage="$bundle/appimage/DeckPilot_${version}_amd64.AppImage"
    expect_file "$deb"
    expect_file "$appimage"
    if [ "$prerelease" = true ]; then
      [ ! -e "$rpm" ] || fail "$rpm was built for a pre-release version"
    else
      expect_file "$rpm"
    fi

    contents=$(dpkg-deb --contents "$deb")
    echo "$contents"
    # dpkg-deb lists paths with or without a leading "./".
    for path in usr/bin/deckpilot-desktop usr/bin/deckpilot-agent usr/bin/deckpilot-plugin-host \
      usr/lib/DeckPilot/bundled/pins.json \
      usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules 'usr/share/applications/.*\.desktop'; do
      echo "$contents" | grep -q -E " (\./)?$path\$" || fail "$deb lacks $path"
    done
    deb_version=$(dpkg-deb -f "$deb" Version)
    [ "$deb_version" = "$version" ] || fail "$deb has version $deb_version, expected $version"
    extracted=$(fresh_dir deb)
    dpkg-deb -x "$deb" "$extracted"
    agent_reports_version "$extracted/usr/bin/deckpilot-agent"
    distribution_record "$extracted/usr/bin/deckpilot-desktop" deb
    distribution_record "$extracted/usr/bin/deckpilot-agent" none optional
    plugin_runtime "$extracted/usr/bin/deckpilot-plugin-host" "$extracted/usr/lib/DeckPilot/bundled" \
      "$extracted/usr/bin/deckpilot-agent" "$deb"
    echo "$deb: version $version, app, agent, plugin host, bundled plugins, udev rule and desktop entry"
    signed_for_version "$deb"

    # The AppImage extracts itself (no FUSE needed): AppRun, the desktop
    # entry, the application and the agent.
    extracted=$(fresh_dir appimage)
    cp "$appimage" "$extracted/DeckPilot.AppImage"
    chmod +x "$extracted/DeckPilot.AppImage"
    (cd "$extracted" && ./DeckPilot.AppImage --appimage-extract > /dev/null) \
      || fail "$appimage does not extract (--appimage-extract)"
    squashfs="$extracted/squashfs-root"
    for path in AppRun usr/bin/deckpilot-desktop usr/bin/deckpilot-agent usr/bin/deckpilot-plugin-host \
      usr/lib/DeckPilot/bundled/pins.json; do
      [ -e "$squashfs/$path" ] || fail "$appimage lacks $path"
    done
    [ -n "$(find "$squashfs" -maxdepth 1 -name '*.desktop' -print -quit)" ] || fail "$appimage lacks its desktop entry"
    agent_reports_version "$squashfs/usr/bin/deckpilot-agent"
    distribution_record "$squashfs/usr/bin/deckpilot-desktop" appimage
    distribution_record "$squashfs/usr/bin/deckpilot-agent" none optional
    plugin_runtime "$squashfs/usr/bin/deckpilot-plugin-host" "$squashfs/usr/lib/DeckPilot/bundled" \
      "$squashfs/usr/bin/deckpilot-agent" "$appimage"
    echo "$appimage: version $version, AppRun, app, agent, plugin host, bundled plugins and desktop entry"
    signed_for_version "$appimage"

    if [ "$prerelease" = true ]; then
      echo "pre-release $version: no .rpm (RPM does not allow \"-\" in versions)"
    else
      files=$(rpm -qlp "$rpm")
      echo "$files"
      for path in /usr/bin/deckpilot-desktop /usr/bin/deckpilot-agent /usr/bin/deckpilot-plugin-host \
        /usr/lib/DeckPilot/bundled/pins.json /usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules; do
        echo "$files" | grep -q -x -F "$path" || fail "$rpm lacks $path"
      done
      rpm_version=$(rpm -qp --queryformat '%{VERSION}' "$rpm")
      [ "$rpm_version" = "$version" ] || fail "$rpm has version $rpm_version, expected $version"
      extracted=$(fresh_dir rpm)
      extract_rpm "$rpm" "$extracted"
      distribution_record "$extracted/usr/bin/deckpilot-desktop" rpm
      plugin_runtime "$extracted/usr/bin/deckpilot-plugin-host" "$extracted/usr/lib/DeckPilot/bundled" \
        "$extracted/usr/bin/deckpilot-agent" "$rpm"
      echo "$rpm: version $version, app, agent, plugin host, bundled plugins and udev rule"
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
      for exe in deckpilot-desktop.exe deckpilot-agent.exe deckpilot-plugin-host.exe; do
        echo "$files" | grep -q -i -x -F "$exe" || fail "the $kind installer lacks $exe"
      done
      agent=$(find "$extracted/$kind" -type f -iname deckpilot-agent.exe | head -n 1)
      agent_reports_version "$agent"
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
      distribution_record "$app" "$kind"
      distribution_record "$agent" none optional
      # Next to the agent: the host, and bundled/ (the installation
      # directory is the resources directory on Windows).
      plugin_runtime "$(dirname "$agent")/deckpilot-plugin-host.exe" "$(dirname "$agent")/bundled" "$agent" "the $kind installer"
    done
    signed_for_version "$nsis"
    signed_for_version "$msi"
    ;;
  macos)
    # (No `case` inside $(...): macOS's bash 3.2 cannot parse it.)
    arch="${MACOS_ARCH-}"
    if [ -z "$arch" ]; then
      arch=x64
      if [ "$(uname -m)" = arm64 ] || [ "$(uname -m)" = aarch64 ]; then arch=aarch64; fi
    fi
    case "$arch" in aarch64 | x64) ;; *) fail "MACOS_ARCH must be aarch64 or x64, not $arch" ;; esac
    app="$bundle/macos/DeckPilot.app"
    archive="$bundle/macos/DeckPilot.app.tar.gz"
    dmg="$bundle/dmg/DeckPilot_${version}_${arch}.dmg"
    [ -d "$app" ] || fail "expected $app, built: $(find "$bundle" -maxdepth 2 | sed "s#^$bundle/##" | tr '\n' ' ')"
    expect_file "$archive"
    expect_file "$dmg"
    macos_app "$app" "the application bundle"

    # The updater installs the archived bundle: the same layout, and nothing
    # beside it.
    extracted=$(fresh_dir macos-updater)
    tar -xzf "$archive" -C "$extracted"
    top=$(cd "$extracted" && find . -mindepth 1 -maxdepth 1 | sed 's#^\./##' | sort | tr '\n' ' ')
    [ "$top" = "DeckPilot.app " ] || fail "$archive holds '$top', expected only DeckPilot.app"
    macos_app "$extracted/DeckPilot.app" "the updater archive"
    signed_for_version "$archive"

    # The disk image: the bundle and the link to /Applications to drag it to.
    mount=$(fresh_dir macos-dmg)
    ${HDIUTIL:-hdiutil} attach -nobrowse -readonly -mountpoint "$mount" "$dmg" > /dev/null \
      || fail "$dmg does not mount"
    detach() { ${HDIUTIL:-hdiutil} detach "$mount" > /dev/null 2>&1 || true; }
    trap detach EXIT
    [ -L "$mount/Applications" ] && [ "$(readlink "$mount/Applications")" = /Applications ] \
      || fail "$dmg has no Applications link"
    macos_app "$mount/DeckPilot.app" "the disk image"
    detach
    trap - EXIT
    echo "$dmg: DeckPilot.app and the Applications link"
    ;;
  *) fail "unknown platform $platform" ;;
esac
echo "packages for $platform: checked"
