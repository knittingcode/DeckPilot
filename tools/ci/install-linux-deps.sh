#!/usr/bin/env bash
# Installs the Linux packages the Tauri app and the agent build against.
#
# GitHub's runners sometimes cannot reach Canonical's servers (the Azure
# mirror, archive.ubuntu.com and security.ubuntu.com all stall) and a job
# hangs for an hour in apt. apt is therefore limited to IPv4 with short
# timeouts, and when the configured mirrors fail the job switches every
# Ubuntu source — the runner's mirror list (/etc/apt/apt-mirrors.txt) and
# the security source — to mirrors outside Canonical's network, in turn.
set -euo pipefail

PACKAGES=(libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libudev-dev)
MIRRORS=(http://mirrors.edge.kernel.org/ubuntu http://mirrors.mit.edu/ubuntu)

printf '%s\n' \
  'Acquire::ForceIPv4 "true";' \
  'Acquire::http::Timeout "10";' \
  'Acquire::https::Timeout "10";' \
  'Acquire::Retries "2";' |
  sudo tee /etc/apt/apt.conf.d/99deckpilot-network >/dev/null

install() {
  timeout 600 sudo apt-get update -o APT::Update::Error-Mode=any &&
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

if install; then
  exit 0
fi
for mirror in "${MIRRORS[@]}"; do
  echo "::warning::installing Linux packages failed; trying $mirror"
  use_mirror "$mirror"
  if install; then
    exit 0
  fi
done
echo "::error::no Ubuntu mirror was reachable"
exit 1
