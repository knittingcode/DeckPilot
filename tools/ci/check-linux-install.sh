#!/usr/bin/env bash
# Installs a freshly built Linux package on a clean system, with only the
# dependencies the package declares, and starts its agent:
#
#   tools/ci/check-linux-install.sh deb <package.deb>   (Ubuntu/Debian runner, sudo)
#   tools/ci/check-linux-install.sh rpm <package.rpm>   (any host with docker: Fedora container)
#   tools/ci/check-linux-install.sh appimage <file.AppImage>
#   tools/ci/check-linux-install.sh agent <agent> [<args>...]
#   tools/ci/check-linux-install.sh without-tray <agent>
#
# The agent's tray loads libayatana-appindicator3 at run time; a package
# that does not declare it installs fine and then has an agent that cannot
# start (DeckPilot 0.7.0's .deb and .rpm). So the check is not the
# installation alone but the installed agent running.
#
# `agent`: starts <agent> (with <args>) under Xvfb in a private D-Bus
# session with an isolated profile (HOME, XDG directories and
# DECKPILOT_DATA_DIR in a temporary directory), as a desktop session would,
# and fails unless it is still running after 15 s and has not reported a
# missing tray library. Used by the other commands.
#
# `appimage`: the agent of an AppImage, started as the app starts it
# (`<AppImage> --agent`, through FUSE). The AppImage bundles the tray
# library with the GLib it was built against: the system's copy cannot be
# loaded next to the AppImage's GLib (DeckPilot 0.7.0 on Ubuntu 24.04:
# "libayatana-ido3-0.4.so.0: undefined symbol: g_once_init_leave_pointer").
#
# `without-tray`: on a system without the tray library, the agent must keep
# running without a tray icon (and say so in its log).
#
# Environment: FEDORA_IMAGE (rpm; default fedora:42).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
fail() { echo "::error::$*" >&2; exit 1; }

# Running, not a zombie.
alive() {
  local state
  state=$(sed -n 's/^State:[[:space:]]*\([A-Z]\).*/\1/p' "/proc/$1/status" 2> /dev/null || true)
  [ -n "$state" ] && [ "$state" != Z ]
}

agent() {
  local agent="$1" profile log pid expect_no_tray="${EXPECT_NO_TRAY:-}"
  shift
  profile=$(mktemp -d "${TMPDIR:-/tmp}/deckpilot-install-check.XXXXXX")
  mkdir -p "$profile"/{home,data,run,xdg-data,xdg-config,xdg-cache}
  chmod 700 "$profile/run" "$profile/data"
  log="$profile/agent.log"
  # The agent's own log is in the data directory; its stderr (panics,
  # library errors) in agent.log.
  env HOME="$profile/home" XDG_DATA_HOME="$profile/xdg-data" XDG_CONFIG_HOME="$profile/xdg-config" \
    XDG_CACHE_HOME="$profile/xdg-cache" XDG_RUNTIME_DIR="$profile/run" DECKPILOT_DATA_DIR="$profile/data" \
    xvfb-run -a dbus-run-session -- "$agent" "$@" > "$log" 2>&1 &
  local wrapper=$!
  pid=""
  for _ in $(seq 1 30); do
    sleep 0.5
    pid=$(pgrep -n -x deckpilot-agent || true)
    [ -n "$pid" ] && break
  done
  sleep 15
  local status=0
  if [ -z "$pid" ] || ! alive "$pid"; then
    echo "::error::the agent did not keep running; its output:" >&2
    status=1
  elif [ -n "$expect_no_tray" ]; then
    if ! cat "$log" "$profile"/data/logs/* 2> /dev/null | grep -q 'running without a tray icon'; then
      echo "::error::the agent did not report that it runs without a tray icon:" >&2
      status=1
    fi
  elif cat "$log" "$profile"/data/logs/* 2> /dev/null | grep -q -E 'Failed to load ayatana-appindicator3|undefined symbol|running without a tray icon'; then
    echo "::error::the agent could not load its tray library:" >&2
    status=1
  fi
  sed 's/^/  | /' "$log" >&2
  for file in "$profile"/data/logs/*; do
    [ -f "$file" ] && sed 's/^/  log | /' "$file" | tail -n 20 >&2
  done
  pkill -x deckpilot-agent 2> /dev/null || true
  kill "$wrapper" 2> /dev/null || true
  wait "$wrapper" 2> /dev/null || true
  rm -rf "$profile"
  [ "$status" -eq 0 ] || exit 1
  echo "the agent ($agent $*) started and kept running for 15 s${expect_no_tray:+, without a tray icon}"
}

case "${1-}" in
  agent)
    shift
    [ "$#" -ge 1 ] || fail "usage: check-linux-install.sh agent <agent> [<args>...]"
    agent "$@"
    ;;
  appimage)
    file="${2:?usage: check-linux-install.sh appimage <file.AppImage>}"
    file="$(cd "$(dirname "$file")" && pwd)/$(basename "$file")"
    chmod +x "$file"
    # What the AppImage carries for the tray, and its GLib.
    extracted=$(mktemp -d)
    (cd "$extracted" && "$file" --appimage-extract > /dev/null)
    echo "the AppImage bundles: $(cd "$extracted/squashfs-root" && find usr/lib -maxdepth 1 \( -name 'libayatana-*' -o -name 'libdbusmenu-*' -o -name 'libglib-2.0.so*' \) | sort | tr '\n' ' ')"
    rm -rf "$extracted"
    if ldconfig -p | grep -q 'libayatana-appindicator3\.so\.1 '; then
      echo "the system has its own libayatana-appindicator3: $(ldconfig -p | grep 'libayatana-appindicator3\.so\.1 ' | head -n 1)"
    else
      echo "the system has no libayatana-appindicator3"
    fi
    agent "$file" --agent
    ;;
  without-tray)
    [ -n "${2-}" ] || fail "usage: check-linux-install.sh without-tray <agent>"
    ! ldconfig -p | grep -q -E 'lib(ayatana-)?appindicator3\.so' \
      || fail "an appindicator library is installed: this check needs a system without one"
    EXPECT_NO_TRAY=1 agent "$2"
    ;;
  deb)
    package="${2:?usage: check-linux-install.sh deb <package.deb>}"
    package="$(cd "$(dirname "$package")" && pwd)/$(basename "$package")"
    echo "Depends: $(dpkg-deb -f "$package" Depends)"
    if ldconfig -p | grep -q 'libayatana-appindicator3\.so\.1 '; then
      echo "::warning::libayatana-appindicator3 is installed already: this check does not show that the package pulls it in"
    fi
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends -o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 "$package"
    # The test's own tools (not dependencies of the package).
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends -o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 xvfb xauth dbus
    ldconfig -p | grep -q 'libayatana-appindicator3\.so\.1 ' \
      || fail "libayatana-appindicator3.so.1 is not installed after installing $package with its dependencies"
    agent /usr/bin/deckpilot-agent
    ;;
  rpm)
    package="${2:?usage: check-linux-install.sh rpm <package.rpm>}"
    dir="$(cd "$(dirname "$package")" && pwd)"
    name="$(basename "$package")"
    image="${FEDORA_IMAGE:-fedora:42}"
    docker run --rm -v "$dir:/packages:ro" -v "$root/tools/ci:/check:ro" "$image" bash -euo pipefail -c "
      rpm -qp --requires /packages/$name | sed 's/^/Requires: /'
      dnf -y -q install /packages/$name
      ldconfig -p | grep -q 'libayatana-appindicator3\.so\.1 ' \
        || { echo '::error::libayatana-appindicator3.so.1 is not installed after installing $name with its dependencies'; exit 1; }
      # The test's own tools (not dependencies of the package).
      dnf -y -q install xorg-x11-server-Xvfb xorg-x11-xauth dbus-daemon procps-ng which
      /check/check-linux-install.sh agent /usr/bin/deckpilot-agent
    "
    ;;
  *) fail "usage: check-linux-install.sh deb|rpm <package> | agent <agent> [<args>...]" ;;
esac
