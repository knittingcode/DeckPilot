#!/usr/bin/env bash
# Checks the first-party plugins staged into the application bundle
# (docs/marketplace/module-migration.md §4.5): they are app components,
# unpacked files trusted by the digests compiled into the agent, never
# plugin packages or registry metadata.
#
#   tools/release-pipeline/check-bundled-plugins.sh <bundled dir> <agent executable> [--without-running]
#
# <bundled dir>: the `bundled` directory the bundler ships as a resource
# (plugins/first-party/build.sh writes it). <agent executable>: the agent
# built for the same bundle; its hidden --print-bundled-pins prints the
# pins compiled into it. With --without-running (an agent cross-compiled
# for another architecture, which this machine cannot run) the agent is
# not started: its file must contain the exact bytes of pins.json, which
# the build embeds as they are (DECKPILOT_BUNDLED_PINS, include_bytes!).
#
# Fails unless:
#   * bundled/pins.json is pins format 1, and every listed id is
#     first-party (deckpilot.<name>), listed once, with class default or
#     migration;
#   * bundled/ holds pins.json and exactly one directory per listed plugin,
#     nothing else;
#   * each plugin directory holds exactly its pinned files, regular files
#     only: manifest.json, plugin.wasm, LICENSE, PNG or JPEG images under
#     assets/ (icons and plugin API 1.8 visual assets, at most 1 MiB) and
#     translations locales/<canonical tag>.json (plugin API 1.6, at most
#     64 KiB), the paths a plugin package may contain
#     (crates/deckpilot-plugin-bundled is_allowed_file); each with its
#     pinned size and sha256, and the manifest names the pinned id and
#     version;
#   * there is no .dpplugin, no .minisig and no symbolic link anywhere;
#   * the pins file equals the pins compiled into the agent.
#
# Needs bash, jq and sha256sum or shasum (and python3 for
# --without-running).
set -euo pipefail

bundled="${1:?usage: check-bundled-plugins.sh <bundled dir> <agent executable>}"
agent="${2:?usage: check-bundled-plugins.sh <bundled dir> <agent executable>}"
run_agent=true
case "${3-}" in
  "") ;;
  --without-running) run_agent=false ;;
  *) echo "usage: check-bundled-plugins.sh <bundled dir> <agent executable> [--without-running]" >&2; exit 2 ;;
esac
# shellcheck source=tools/release-pipeline/common.sh
source "$(dirname "$0")/common.sh"
# jq on Windows ends its lines with CRLF; every value here is one line.
jq() { command jq "$@" | tr -d '\r'; }

# Digests from stdin: GNU sha256sum escapes a file name with a backslash
# (D:\a\_temp/... on Windows) and then prefixes the digest with "\".
if command -v sha256sum > /dev/null; then
  sha() { sha256sum < "$1" | cut -d' ' -f1; }
else
  sha() { shasum -a 256 < "$1" | cut -d' ' -f1; }
fi
size() { wc -c < "$1" | tr -d ' '; }

# allowed <path>: the files a bundled plugin may contain, as
# is_allowed_file in crates/deckpilot-plugin-bundled: the required files;
# a PNG or JPEG under assets/ whose path a package may contain (segments
# of letters, digits, _ . -, not starting with . or -, not ending with .,
# no Windows device name, at most 128 characters); a translation of a
# canonical locale tag directly in locales/.
allowed() {
  local file=$1 segment stem
  case "$file" in manifest.json | plugin.wasm | LICENSE) return 0 ;; esac
  [ "${#file}" -le 128 ] || return 1
  if [[ "$file" =~ ^locales/[a-z]{2,3}(-[A-Z][a-z]{3})?(-([A-Z]{2}|[0-9]{3}))?\.json$ ]]; then
    return 0
  fi
  [[ "$file" =~ ^assets/([A-Za-z0-9_][A-Za-z0-9._-]*/)*[A-Za-z0-9_][A-Za-z0-9._-]*\.(png|jpg|jpeg)$ ]] || return 1
  local IFS=/
  for segment in $file; do
    [[ "$segment" != *. ]] || return 1
    stem=$(printf '%s' "${segment%%.*}" | tr '[:upper:]' '[:lower:]')
    [[ ! "$stem" =~ ^(con|prn|aux|nul|com[0-9]|lpt[0-9])$ ]] || return 1
  done
}
# max_bytes <path>: the largest such file (empty: the package limits).
max_bytes() {
  case "$1" in
    locales/*) echo 65536 ;;
    assets/*) echo 1048576 ;;
  esac
}

[ -d "$bundled" ] || fail "$bundled is not a directory"
pins="$bundled/pins.json"
[ -f "$pins" ] || fail "$bundled has no pins.json"
jq -e '.pinsVersion == 1 and (.plugins | type == "array")' "$pins" > /dev/null 2>&1 \
  || fail "$pins is not pins format 1 (pinsVersion 1 with a plugins list)"

# Never a package, a signature or a link: bundled plugins are trusted by
# the pins compiled into the agent, not by a plugin signature.
packages=$(find "$bundled" \( -name '*.dpplugin' -o -name '*.dpplugin.*' -o -name '*.minisig' \) -print)
[ -z "$packages" ] || fail "the bundle contains plugin packages or signatures: $packages (bundled plugins are unpacked files; packages and signatures belong to the plugin registry)"
links=$(find "$bundled" -type l -print)
[ -z "$links" ] || fail "the bundle contains symbolic links: $links"
others=$(find "$bundled" ! -type f ! -type d -print)
[ -z "$others" ] || fail "the bundle contains entries that are neither files nor directories: $others"

ids=$(jq -r '.plugins[].id' "$pins")
duplicates=$(printf '%s\n' "$ids" | sort | uniq -d)
[ -z "$duplicates" ] || fail "pins.json lists a plugin twice: $duplicates"

# Top level: pins.json and the listed plugins, nothing else.
for entry in "$bundled"/* "$bundled"/.[!.]*; do
  [ -e "$entry" ] || continue
  name=$(basename "$entry")
  [ "$name" = pins.json ] && continue
  printf '%s\n' "$ids" | grep -x -F -- "$name" >/dev/null \
    || fail "bundled/$name is not listed in pins.json"
  [ -d "$entry" ] || fail "bundled/$name is not a directory"
done

count=0
while IFS=$'\t' read -r id version class; do
  [ -n "$id" ] || continue
  [[ "$id" =~ ^deckpilot\.[a-z][a-z0-9]*(-[a-z][a-z0-9]*)*$ ]] \
    || fail "$id is not a first-party plugin id (deckpilot.<name>): only DeckPilot's own plugins are bundled"
  case "$class" in default | migration) ;; *) fail "$id has class '$class' (default or migration)" ;; esac
  dir="$bundled/$id"
  [ -d "$dir" ] || fail "$id is listed in pins.json but bundled/$id is missing"
  pinned=$(jq -r --arg id "$id" '.plugins[] | select(.id == $id) | .files | keys[]' "$pins" | LC_ALL=C sort)
  present=$(cd "$dir" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
  for required in manifest.json plugin.wasm LICENSE; do
    printf '%s\n' "$pinned" | grep -x -F -- "$required" >/dev/null || fail "$id does not pin $required"
  done
  while IFS= read -r file; do
    [ -n "$file" ] || continue
    allowed "$file" \
      || fail "$id: $file is not allowed in a bundled plugin (manifest.json, plugin.wasm, LICENSE, .png/.jpg/.jpeg images under assets/ and locales/<tag>.json translations only)"
    printf '%s\n' "$pinned" | grep -x -F -- "$file" >/dev/null || fail "$id: $file is not pinned"
    limit=$(max_bytes "$file")
    [ -z "$limit" ] || [ "$(size "$dir/$file")" -le "$limit" ] \
      || fail "$id: $file has $(size "$dir/$file") bytes, at most $limit are allowed"
  done <<< "$present"
  while IFS= read -r file; do
    [ -f "$dir/$file" ] || fail "$id: pinned $file is missing"
    want_sha=$(jq -r --arg id "$id" --arg f "$file" '.plugins[] | select(.id == $id) | .files[$f].sha256' "$pins")
    want_size=$(jq -r --arg id "$id" --arg f "$file" '.plugins[] | select(.id == $id) | .files[$f].size' "$pins")
    [ "$(size "$dir/$file")" = "$want_size" ] || fail "$id: $file has $(size "$dir/$file") bytes, $want_size are pinned"
    [ "$(sha "$dir/$file")" = "$want_sha" ] || fail "$id: $file does not have its pinned sha256"
  done <<< "$pinned"
  manifest_id=$(jq -r .id "$dir/manifest.json")
  manifest_version=$(jq -r .version "$dir/manifest.json")
  [ "$manifest_id@$manifest_version" = "$id@$version" ] \
    || fail "$id: manifest.json is $manifest_id@$manifest_version, $id@$version is pinned"
  count=$((count + 1))
done < <(jq -r '.plugins[] | [.id, .version, .class] | @tsv' "$pins")

# The agent trusts the digests compiled into it, never this file: they
# must be the same.
if [ "$run_agent" = false ]; then
  [ -f "$agent" ] || fail "$agent does not exist"
  python3 -I -c 'import sys; sys.exit(0 if open(sys.argv[1], "rb").read() in open(sys.argv[2], "rb").read() else 1)' "$pins" "$agent" \
    || fail "pins.json is not compiled into $agent: build the agent with DECKPILOT_BUNDLED_PINS=$pins"
  echo "$count bundled plugins match their pins, and $agent embeds exactly this pins.json (not run)"
  exit 0
fi
compiled=$("$agent" --print-bundled-pins) || fail "$agent --print-bundled-pins failed"
jq -e . > /dev/null 2>&1 <<< "$compiled" || fail "$agent printed no pins"
[ "$(jq -S . <<< "$compiled")" = "$(jq -S . "$pins")" ] \
  || fail "pins.json differs from the pins compiled into $agent: build the agent with DECKPILOT_BUNDLED_PINS=$pins"

echo "$count bundled plugins match their pins and the pins compiled into the agent"
