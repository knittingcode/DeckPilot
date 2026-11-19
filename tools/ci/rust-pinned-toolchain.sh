#!/usr/bin/env bash
# Leaves only the Rust toolchain pinned in rust-toolchain.toml installed on
# a CI runner, and makes it the default. Run after `rustup toolchain install`
# and before Swatinem/rust-cache.
#
# rust-cache hashes the version of every installed toolchain into its cache
# key (`rustup toolchain list`), not only the one the build uses. The runner
# images preinstall the current stable toolchain, and a new stable reaches
# the runners over several days: while it rolls out, the same job on two
# runners computes two keys. A runner with the new stable then misses the
# cache that main saved on a runner without it and builds everything cold
# (Rust 1.100.0, November 2026: 6 of the 7 jobs that had it missed, against
# 5 of 78 without it), and main saves a second copy of each cache (about
# 1 GB on Windows), which evicts others from the 10 GB repository cache.
#
# Nothing in these jobs uses the preinstalled toolchain: in the repository,
# rust-toolchain.toml selects the pinned one. The pinned toolchain becomes
# the default so that commands outside the repository use it too, rather
# than no toolchain.
set -euo pipefail

# "1.99.1-x86_64-unknown-linux-gnu (overridden by '.../rust-toolchain.toml')"
pinned=$(rustup show active-toolchain | tr -d '\r' | cut -d ' ' -f 1)
case "$pinned" in
  "" | stable* | beta* | nightly*)
    echo "expected the toolchain pinned in rust-toolchain.toml, found '${pinned}'" >&2
    exit 1
    ;;
esac

rustup default "$pinned"
installed=$(rustup toolchain list --quiet | tr -d '\r' | cut -d ' ' -f 1)
for toolchain in $installed; do
  if [ "$toolchain" != "$pinned" ]; then
    echo "removing ${toolchain} (preinstalled, not pinned)"
    rustup toolchain uninstall "$toolchain"
  fi
done
echo "installed toolchains:"
rustup toolchain list
