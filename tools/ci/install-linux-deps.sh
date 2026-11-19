#!/usr/bin/env bash
# Installs the Linux packages the Tauri app and the agent build against.
#
# GitHub's runners sometimes cannot reach Canonical's servers (the Azure
# mirror, archive.ubuntu.com and security.ubuntu.com all stall) and a job
# hangs for an hour in apt. apt is therefore limited to IPv4 with short
# timeouts, and when the configured mirrors fail the job switches every
# Ubuntu source — the runner's mirror list (/etc/apt/apt-mirrors.txt) and
# the security source — to mirrors outside Canonical's network, in turn.
#
# Every package DeckPilot's jobs install comes from Ubuntu. The runner image
# also lists third-party sources (packages.microsoft.com and others); with
# APT::Update::Error-Mode=any one of them failing (it answers 429 Too Many
# Requests at times) failed the job, and no Ubuntu mirror helps with that.
# Those sources are therefore set aside for the rest of the job.
set -euo pipefail

PACKAGES=(libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libudev-dev)
MIRRORS=(http://mirrors.edge.kernel.org/ubuntu http://mirrors.mit.edu/ubuntu)

printf '%s\n' \
  'Acquire::ForceIPv4 "true";' \
  'Acquire::http::Timeout "10";' \
  'Acquire::https::Timeout "10";' \
  'Acquire::Retries "2";' |
  sudo tee /etc/apt/apt.conf.d/99deckpilot-network >/dev/null

# Sets aside every source file that names a server outside ubuntu.com (the
# runner's mirror list, mirror+file:, is Ubuntu's).
ubuntu_sources_only() {
  local file aside=/etc/apt/sources.list.d.deckpilot-aside
  for file in /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
    [ -f "$file" ] || continue
    if grep -Eo 'https?://[^/[:space:]]+' "$file" | grep -Eqv '://([a-z0-9.-]+\.)?ubuntu\.com$'; then
      sudo mkdir -p "$aside"
      sudo mv "$file" "$aside/"
      echo "set aside $(basename "$file") (not an Ubuntu source)"
    fi
  done
}

# A mirror can also stay reachable but crawl: one runner took 12 min 40 s
# for the 68 MB of packages (89 kB/s), where the whole step usually takes
# half a minute, and apt's timeouts only catch a connection that sends
# nothing. The packages are therefore downloaded first, within $1 seconds;
# a download that runs out of time moves on to the next mirror and keeps
# the packages it already has (/var/cache/apt/archives). Only the download
# is cut short, never dpkg, so no half-installed package is left behind.
install() {
  timeout 600 sudo apt-get update -o APT::Update::Error-Mode=any &&
    timeout "$1" sudo apt-get install -y --download-only "${PACKAGES[@]}" &&
    timeout 1200 sudo apt-get install -y "${PACKAGES[@]}"
}

use_mirror() {
  # The runner image reads its mirrors from this list (mirror+file:).
  if [ -f /etc/apt/apt-mirrors.txt ]; then
    printf '%s\tpriority:1\n' "$1" | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
  fi
  for file in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
    [ -f "$file" ] || continue
    sudo sed -i -E "s#https?://([a-z0-9.-]+\.)?(archive|security)\.ubuntu\.com/ubuntu/?#$1/#g" "$file"
  done
}

ubuntu_sources_only
if install 300; then
  exit 0
fi
last="${MIRRORS[${#MIRRORS[@]}-1]}"
for mirror in "${MIRRORS[@]}"; do
  echo "::warning::installing Linux packages failed or was too slow; trying $mirror"
  use_mirror "$mirror"
  # The last mirror gets the time the whole installation had before.
  if [ "$mirror" = "$last" ]; then download_timeout=1200; else download_timeout=300; fi
  if install "$download_timeout"; then
    exit 0
  fi
done
echo "::error::no Ubuntu mirror was reachable"
exit 1
