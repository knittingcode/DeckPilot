#!/usr/bin/env bash
# Installs the Linux packages the Tauri app and the agent build against.
#
# GitHub's runners sometimes cannot reach the Ubuntu mirrors: every request
# stalls (often a broken IPv6 route) and a job hangs for an hour in apt.
# apt is therefore limited to IPv4 with short timeouts, and when the
# configured mirror fails the job tries other mirrors in turn.
set -euo pipefail

PACKAGES=(libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libudev-dev)
MIRRORS=(http://archive.ubuntu.com/ubuntu http://mirrors.edge.kernel.org/ubuntu)

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
  for file in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
    [ -f "$file" ] || continue
    sudo sed -i -E "s#https?://([a-z0-9.-]+\.)?archive\.ubuntu\.com/ubuntu/?#$1/#g; s#https?://mirrors\.edge\.kernel\.org/ubuntu/?#$1/#g" "$file"
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
