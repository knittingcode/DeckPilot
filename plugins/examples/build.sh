#!/usr/bin/env bash
# Builds example plugins to wasm32-unknown-unknown with the `guest` profile,
# reproducibly, and prints the sha256 of each module.
#
#   plugins/examples/build.sh [--target-dir DIR] [--out DIR] [PACKAGE...]
#
# PACKAGE defaults to `hello-world developer-tools`. Modules are copied to
# --out (default: plugins/runtime/target/examples) as <crate>.wasm.
#
# Reproducible on one machine: the toolchain is pinned
# (rust-toolchain.toml), dependencies are locked (`--locked`, the runtime
# workspace's Cargo.lock), the guest profile strips symbols, and the
# checkout and Cargo home are remapped (--remap-path-prefix) so no
# machine-specific path ends up in the module. No flags of the calling
# environment leak in (RUSTFLAGS and wrappers are cleared). Two builds from
# clean target directories give byte-identical modules (CI: the
# `reproducible` job of .github/workflows/plugin-runtime.yml). Builds on
# different operating systems, and for developer-tools on two macOS
# machines, were not byte-identical; Ubuntu is the reference platform
# (docs/marketplace/sdk/building.md).
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
plugins=$(dirname "$here")
runtime="$plugins/runtime"
target_dir="$runtime/target"
out=""
packages=()

while [ $# -gt 0 ]; do
  case "$1" in
    --target-dir) target_dir=$2; shift 2 ;;
    --out) out=$2; shift 2 ;;
    -h | --help) sed -n '2,19p' "$0"; exit 0 ;;
    -*) echo "build.sh: unknown option $1" >&2; exit 2 ;;
    *) packages+=("$1"); shift ;;
  esac
done
[ ${#packages[@]} -gt 0 ] || packages=(hello-world developer-tools)
[ -n "$out" ] || out="$target_dir/examples"

cargo_home=${CARGO_HOME:-$HOME/.cargo}
native_plugins=$plugins
# Git Bash on Windows: the compiler sees Windows paths, the shell (and
# MSYS's argument conversion for cargo) POSIX ones.
if command -v cygpath > /dev/null; then
  native_plugins=$(cygpath -w "$plugins")
  cargo_home=$(cygpath -w "$cargo_home")
  target_dir=$(cygpath -u "$target_dir")
  out=$(cygpath -u "$out")
fi
sep=$'\x1f'
flags="--remap-path-prefix=$native_plugins=/deckpilot/plugins${sep}--remap-path-prefix=$cargo_home=/cargo"

args=()
for p in "${packages[@]}"; do args+=(--package "$p"); done

(
  cd "$runtime"
  env -u RUSTFLAGS -u RUSTC_WRAPPER -u RUSTC_WORKSPACE_WRAPPER -u CARGO_BUILD_TARGET \
    CARGO_ENCODED_RUSTFLAGS="$flags" \
    cargo build --locked "${args[@]}" \
    --target wasm32-unknown-unknown --profile guest --target-dir "$target_dir"
)

if command -v sha256sum > /dev/null; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

mkdir -p "$out"
for p in "${packages[@]}"; do
  crate=${p//-/_}
  cp "$target_dir/wasm32-unknown-unknown/guest/$crate.wasm" "$out/$crate.wasm"
  printf '%s  %s\n' "$(sha "$out/$crate.wasm")" "$crate.wasm"
done
