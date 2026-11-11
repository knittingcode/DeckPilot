#!/usr/bin/env bash
# Installs the Linux packages the Tauri app and the agent build against.
#
# GitHub's runners use the Azure Ubuntu mirror. When it stalls, apt waits
# about 20 s per package and a job hangs for hours, so downloads get short
# timeouts and, if the mirror fails, the job switches to the main Ubuntu
# archive and tries again.
set -euo pipefail

PACKAGES=(libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libudev-dev)

printf 'Acquire::http::Timeout "15";\nAcquire::https::Timeout "15";\nAcquire::Retries "2";\n' |
  sudo tee /etc/apt/apt.conf.d/99deckpilot-timeouts >/dev/null

install() {
  timeout 900 sudo apt-get update &&
    timeout 1200 sudo apt-get install -y "${PACKAGES[@]}"
}

if install; then
  exit 0
fi
echo "::warning::the default Ubuntu mirror failed; using archive.ubuntu.com"
for file in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
  [ -f "$file" ] && sudo sed -i 's#http://azure.archive.ubuntu.com/ubuntu#http://archive.ubuntu.com/ubuntu#g' "$file"
done
install
