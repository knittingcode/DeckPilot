#!/usr/bin/env bash
# Builds the first-party plugins the application bundles and stages them as
# app components, with the pins the agent is compiled with
# (docs/marketplace/module-migration.md §4.4, §4.5).
#
#   plugins/first-party/build.sh --out DIR [--test-fixtures] [--target-dir DIR]
#
# Writes DIR/bundled/<id>/manifest.json, plugin.wasm, LICENSE and the icons
# the manifest names, and DIR/bundled/pins.json: for every plugin its id,
# version, class and the size and sha256 of every file. The release build
# compiles the agent with DECKPILOT_BUNDLED_PINS=DIR/bundled/pins.json and
# ships DIR/bundled as a Tauri resource (tauri.plugins.conf.json); the
# agent trusts the files only if they match the pins compiled into it.
#
# The plugins are those of plugins/first-party/bundled.json.
# --test-fixtures adds plugins/first-party/test-fixtures/bundled.json (the
# test plugin deckpilot.example-default), for tests and dry runs only:
# a release bundle never contains a test fixture.
#
# Reproducible on one platform, like plugins/examples/build.sh: the pinned
# toolchain (rust-toolchain.toml), the locked plugins/runtime workspace,
# the `guest` profile, remapped source and Cargo paths, and no flags from
# the calling environment. No package (.dpplugin) and no signature is
# made: bundled plugins are trusted by the pins, never by a key.
#
# Needs cargo (with the wasm32-unknown-unknown target) and jq.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
plugins=$(dirname "$here")
runtime="$plugins/runtime"
target_dir="$runtime/target"
out=""
fixtures=false

while [ $# -gt 0 ]; do
  case "$1" in
    --out) out=$2; shift 2 ;;
    --target-dir) target_dir=$2; shift 2 ;;
    --test-fixtures) fixtures=true; shift ;;
    -h | --help) sed -n '2,27p' "$0"; exit 0 ;;
    *) echo "build.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$out" ] || { echo "build.sh: --out DIR is required" >&2; exit 2; }
# Cargo runs in plugins/runtime: a relative target directory is the caller's.
mkdir -p "$target_dir"
target_dir=$(cd "$target_dir" && pwd)
fail() { echo "build.sh: $*" >&2; exit 1; }
command -v jq > /dev/null || fail "jq is required"
# jq on Windows ends its lines with CRLF; every value here is one line.
jq() { command jq "$@" | tr -d '\r'; }

if command -v sha256sum > /dev/null; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi
size() { wc -c < "$1" | tr -d ' '; }

# The plugin list: "<directory relative to plugins/first-party> <class>".
lists=("$here/bundled.json")
[ "$fixtures" = false ] || lists+=("$here/test-fixtures/bundled.json")
entries=()
for list in "${lists[@]}"; do
  base=$(dirname "$list")
  while IFS=$'\t' read -r path class; do
    [ -n "$path" ] || continue
    case "$class" in default | migration) ;; *) fail "$list: $path has class '$class' (default or migration)" ;; esac
    rel=${base#"$here"}
    rel=${rel#/}
    entries+=("${rel:+$rel/}$path"$'\t'"$class")
  done < <(jq -r '.plugins[] | [.path, .class] | @tsv' "$list")
done

bundled="$out/bundled"
rm -rf "$bundled"
mkdir -p "$bundled"

crates=()
for entry in ${entries[@]+"${entries[@]}"}; do
  dir="$here/${entry%%$'\t'*}"
  [ -f "$dir/Cargo.toml" ] || fail "$dir has no Cargo.toml"
  crates+=("$(sed -n 's/^name = "\(.*\)"$/\1/p' "$dir/Cargo.toml" | head -n 1)")
done

if [ ${#crates[@]} -gt 0 ]; then
  cargo_home=${CARGO_HOME:-$HOME/.cargo}
  native_plugins=$plugins
  if command -v cygpath > /dev/null; then
    native_plugins=$(cygpath -w "$plugins")
    cargo_home=$(cygpath -w "$cargo_home")
    target_dir=$(cygpath -u "$target_dir")
  fi
  sep=$'\x1f'
  flags="--remap-path-prefix=$native_plugins=/deckpilot/plugins${sep}--remap-path-prefix=$cargo_home=/cargo"
  args=()
  for crate in "${crates[@]}"; do args+=(--package "$crate"); done
  (
    cd "$runtime"
    env -u RUSTFLAGS -u RUSTC_WRAPPER -u RUSTC_WORKSPACE_WRAPPER -u CARGO_BUILD_TARGET \
      CARGO_ENCODED_RUSTFLAGS="$flags" \
      cargo build --locked "${args[@]}" \
      --target wasm32-unknown-unknown --profile guest --target-dir "$target_dir"
  )
fi

pins='[]'
index=0
for entry in ${entries[@]+"${entries[@]}"}; do
  dir="$here/${entry%%$'\t'*}"
  class="${entry##*$'\t'}"
  crate="${crates[$index]}"
  index=$((index + 1))
  manifest="$dir/manifest.json"
  id=$(jq -r .id "$manifest")
  version=$(jq -r .version "$manifest")
  case "$id" in deckpilot.*) ;; *) fail "$manifest: $id is not a first-party id (deckpilot.*)" ;; esac
  [ "$(jq 'has("assets")' "$manifest")" = false ] \
    || fail "$manifest: a bundled plugin has no assets but its icons"
  target="$bundled/$id"
  [ ! -e "$target" ] || fail "$id is listed twice"
  mkdir -p "$target"
  cp "$manifest" "$target/manifest.json"
  cp "$dir/LICENSE" "$target/LICENSE"
  cp "$target_dir/wasm32-unknown-unknown/guest/${crate//-/_}.wasm" "$target/plugin.wasm"
  while IFS= read -r icon; do
    case "$icon" in assets/*.png) ;; *) fail "$manifest: icon $icon is not assets/<name>.png" ;; esac
    mkdir -p "$target/assets"
    cp "$dir/$icon" "$target/$icon"
  done < <(jq -r '.icons // {} | .[]' "$manifest")

  files='{}'
  while IFS= read -r file; do
    files=$(jq --arg f "$file" --arg s "$(sha "$target/$file")" --argjson n "$(size "$target/$file")" \
      '. + {($f): {sha256: $s, size: $n}}' <<< "$files")
  done < <(cd "$target" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
  pins=$(jq --arg id "$id" --arg v "$version" --arg c "$class" --argjson f "$files" \
    '. + [{id: $id, version: $v, class: $c, files: $f}]' <<< "$pins")
  printf '%s  %s %s (%s)\n' "$(sha "$target/plugin.wasm")" "$id" "$version" "$class"
done

jq -S -n --argjson p "$pins" '{pinsVersion: 1, plugins: ($p | sort_by(.id))}' > "$bundled/pins.json"
echo "wrote $bundled/pins.json ($(jq '.plugins | length' "$bundled/pins.json") plugins)"
