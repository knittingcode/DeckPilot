#!/usr/bin/env bash
# Checks the first-party plugins staged into the application bundle
# (docs/marketplace/module-migration.md §4.5): they are app components,
# unpacked files trusted by the digests compiled into the agent, never
# plugin packages or registry metadata.
#
#   tools/release-pipeline/check-bundled-plugins.sh <bundled dir> <agent executable>
#
# <bundled dir>: the `bundled` directory the bundler ships as a resource
# (plugins/first-party/build.sh writes it). <agent executable>: the agent
# built for the same bundle; its hidden --print-bundled-pins prints the
# pins compiled into it.
#
# Fails unless:
#   * bundled/pins.json is pins format 1, and every listed id is
#     first-party (deckpilot.<name>), listed once, with class default or
#     migration;
#   * bundled/ holds pins.json and exactly one directory per listed plugin,
#     nothing else;
#   * each plugin directory holds exactly its pinned files, regular files
#     only: manifest.json, plugin.wasm, LICENSE and icons assets/<name>.png;
#     each with its pinned size and sha256, and the manifest names the
#     pinned id and version;
#   * there is no .dpplugin, no .minisig and no symbolic link anywhere;
#   * the pins file equals the pins compiled into the agent.
#
# Needs bash, jq and sha256sum or shasum.
set -euo pipefail

bundled="${1:?usage: check-bundled-plugins.sh <bundled dir> <agent executable>}"
agent="${2:?usage: check-bundled-plugins.sh <bundled dir> <agent executable>}"
# shellcheck source=tools/release-pipeline/common.sh
source "$(dirname "$0")/common.sh"
# jq on Windows ends its lines with CRLF; every value here is one line.
jq() { command jq "$@" | tr -d '\r'; }

if command -v sha256sum > /dev/null; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi
size() { wc -c < "$1" | tr -d ' '; }

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
    [[ "$file" =~ ^(manifest\.json|plugin\.wasm|LICENSE|assets/[a-z0-9-]+\.png)$ ]] \
      || fail "$id: $file is not allowed in a bundled plugin (manifest.json, plugin.wasm, LICENSE and assets/<name>.png icons only)"
    printf '%s\n' "$pinned" | grep -x -F -- "$file" >/dev/null || fail "$id: $file is not pinned"
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
compiled=$("$agent" --print-bundled-pins) || fail "$agent --print-bundled-pins failed"
jq -e . > /dev/null 2>&1 <<< "$compiled" || fail "$agent printed no pins"
[ "$(jq -S . <<< "$compiled")" = "$(jq -S . "$pins")" ] \
  || fail "pins.json differs from the pins compiled into $agent: build the agent with DECKPILOT_BUNDLED_PINS=$pins"

echo "$count bundled plugins match their pins and the pins compiled into the agent"
