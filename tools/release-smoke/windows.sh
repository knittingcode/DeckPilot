#!/usr/bin/env bash
# Checks an installed Windows DeckPilot (Git Bash):
#
#   tools/release-smoke/windows.sh layout <label> <install dir> <version>
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/release-smoke/common.sh
source "$here/common.sh"

[ "${1-}" = layout ] || { echo "usage: windows.sh layout <label> <install dir> <version>" >&2; exit 2; }
label="${2:?}"
dir="$(cygpath -u "${3:?}")"
version="${4:?}"

check_exec "$label: binaries present and executable" \
  "$dir/deckpilot-desktop.exe" "$dir/deckpilot-agent.exe" "$dir/deckpilot-plugin-host.exe"
check_version "$label: agent version" "$dir/deckpilot-agent.exe" "$version"
app_version=$(pwsh -NoProfile -Command "(Get-Item -LiteralPath '$(cygpath -w "$dir/deckpilot-desktop.exe")').VersionInfo.ProductVersion" | tr -d '\r')
case "$app_version" in
  "$version" | "$version".*) record "$label: app version resource" pass "deckpilot-desktop.exe ProductVersion $app_version" ;;
  *) record "$label: app version resource" fail "deckpilot-desktop.exe ProductVersion '$app_version', expected $version" ;;
esac
# The installation directory is the resources directory on Windows.
check_bundled "$label: bundled plugins and pins.json" "$dir/bundled" "$dir/deckpilot-agent.exe"
check_roots "$label: production registry roots embedded in the agent" "$dir/deckpilot-agent.exe"
check_updater_key "$label: updater key embedded in the app" "$dir/deckpilot-desktop.exe"
exit 0
