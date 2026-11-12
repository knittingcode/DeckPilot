#!/usr/bin/env bash
# Prints the Windows Installer (WiX) version for a DeckPilot version. WiX only
# accepts numeric versions, so pre-releases map to a fourth number that keeps
# them in order before the final release:
#
#   0.6.0          -> 0.6.0
#   0.6.0-alpha.N  -> 0.6.0.N
#   0.6.0-beta.N   -> 0.6.0.(20000+N)
#   0.6.0-rc.N     -> 0.6.0.(40000+N)
#
# The same mapping as the release workflow's tag gate; anything else fails.
set -euo pipefail

version="${1:?usage: msi-version.sh <version>}"
if [[ "$version" =~ ^([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
  echo "$version"
elif [[ "$version" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-(alpha|beta|rc)\.([0-9]+)$ ]]; then
  base="${BASH_REMATCH[1]}" kind="${BASH_REMATCH[2]}" n="${BASH_REMATCH[3]}"
  case "$kind" in
    alpha) offset=0 ;;
    beta) offset=20000 ;;
    rc) offset=40000 ;;
  esac
  build=$((offset + 10#$n))
  # WiX allows at most 65535 in each field.
  if [ "$((10#$n))" -ge 20000 ] || [ "$build" -gt 65535 ]; then
    echo "$version: pre-release number too large for an MSI version" >&2
    exit 1
  fi
  echo "$base.$build"
else
  echo "$version: not a version the MSI can represent (X.Y.Z or X.Y.Z-alpha|beta|rc.N)" >&2
  exit 1
fi
