#!/usr/bin/env bash
# Checks that no bundled first-party plugin ships with the id and version
# of a released one but other files, and records a release's pins.
#
#   tools/release-pipeline/check-released-pins.sh <pins.json> [--platform P] [--released FILE]
#   tools/release-pipeline/check-released-pins.sh --record <release> <pins.json> [--platform P] [--released FILE] [--source TEXT]
#
# <pins.json>: the pins plugins/first-party/build.sh wrote (bundled/pins.json).
# FILE: the pins of the published releases, default
# plugins/first-party/released-pins.json (or $DECKPILOT_RELEASED_PINS).
# P: macos, linux or windows, default this machine's: build.sh is
# reproducible on one platform, so a release's pins are recorded per
# platform and compared only with a build on the same one.
#
# Why: the agent upgrades an installed bundled plugin only to a higher
# version, and refuses (Broken) an installed copy whose files differ from
# the pins compiled into it. A plugin rebuilt with other files under a
# version an earlier release shipped breaks every installation that
# upgrades from that release. A changed plugin gets a new version
# (plugins/first-party/README.md, "Versions").
#
# The check fails when the released pins are malformed, or when a plugin
# of <pins.json> has the id and version of a plugin a release recorded for
# P, with other files (another file set, size or sha256). A plugin of a
# version no release recorded passes, and so does the same plugin with
# the same files.
#
# --record <release> adds the pins of <release> (X.Y.Z) for P to FILE,
# after the same check; recording the same pins again changes nothing,
# other pins for a release and platform already recorded are refused.
#
# Needs bash and jq.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
# jq on Windows ends its lines with CRLF; every value here is one line.
jq() { command jq "$@" | tr -d '\r'; }
fail() {
  echo "::error::$*" >&2
  exit 1
}
usage() {
  echo "usage: check-released-pins.sh [--record <release>] <pins.json> [--platform macos|linux|windows] [--released FILE] [--source TEXT]" >&2
  exit 2
}

released="${DECKPILOT_RELEASED_PINS:-$root/plugins/first-party/released-pins.json}"
platform=""
record=""
source_text=""
pins=""
while [ $# -gt 0 ]; do
  case "$1" in
    --record) [ $# -ge 2 ] || usage; record=$2; shift 2 ;;
    --platform) [ $# -ge 2 ] || usage; platform=$2; shift 2 ;;
    --released) [ $# -ge 2 ] || usage; released=$2; shift 2 ;;
    --source) [ $# -ge 2 ] || usage; source_text=$2; shift 2 ;;
    -*) usage ;;
    *) [ -z "$pins" ] || usage; pins=$1; shift ;;
  esac
done
[ -n "$pins" ] || usage
if [ -z "$platform" ]; then
  case "$(uname -s)" in
    Darwin) platform=macos ;;
    Linux) platform=linux ;;
    MINGW* | MSYS* | CYGWIN*) platform=windows ;;
    *) fail "unknown platform $(uname -s): pass --platform macos, linux or windows" ;;
  esac
fi
case "$platform" in macos | linux | windows) ;; *) fail "platform '$platform' (macos, linux or windows)" ;; esac

[ -f "$pins" ] || fail "$pins does not exist"
jq -e '.pinsVersion == 1 and (.plugins | type == "array")' "$pins" > /dev/null 2>&1 \
  || fail "$pins is not pins format 1 (pinsVersion 1 with a plugins list)"
[ -f "$released" ] || fail "$released does not exist (the pins of the published releases)"

# The released pins: format 1, every entry a release X.Y.Z, a platform and
# the id, version and files of each plugin, one entry per release and
# platform, one plugin per id in each.
jq -e '
  def file: type == "object" and (keys == ["sha256", "size"])
    and (.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and (.size | type == "number" and . >= 0);
  def plugin: type == "object" and (keys == ["files", "id", "version"])
    and (.id | type == "string" and test("^deckpilot\\.[a-z][a-z0-9]*(-[a-z][a-z0-9]*)*$"))
    and (.version | type == "string" and test("^[0-9]+\\.[0-9]+\\.[0-9]+$"))
    and (.files | type == "object" and length > 0 and all(.[]; file));
  .releasedPinsVersion == 1 and (.releases | type == "array")
  and all(.releases[];
    type == "object"
    and (.release | type == "string" and test("^[0-9]+\\.[0-9]+\\.[0-9]+$"))
    and (.platform | IN("macos", "linux", "windows"))
    and ((.source // "") | type == "string")
    and (.plugins | type == "array" and all(.[]; plugin)
         and ((map(.id) | unique | length) == length)))
  and ((.releases | map([.release, .platform]) | unique | length) == (.releases | length))
' "$released" > /dev/null 2>&1 \
  || fail "$released is not released pins format 1 (releasedPinsVersion 1; releases of {release X.Y.Z, platform macos|linux|windows, source, plugins [{id, version, files {path: {sha256, size}}}]}, one per release and platform)"

# A release and platform already recorded: the same pins or nothing.
if [ -n "$record" ]; then
  [[ "$record" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "--record $record: a release is X.Y.Z"
  entry=$(jq -c --arg release "$record" --arg platform "$platform" --arg source "$source_text" '
    {release: $release, platform: $platform, source: $source,
     plugins: [.plugins[] | {id, version, files}] | sort_by(.id)}' "$pins")
  existing=$(jq -c --arg release "$record" --arg platform "$platform" \
    '.releases[] | select(.release == $release and .platform == $platform)' "$released")
  if [ -n "$existing" ]; then
    [ "$(jq -c '.plugins' <<< "$existing")" = "$(jq -c '.plugins' <<< "$entry")" ] \
      || fail "$released already records other pins for DeckPilot $record ($platform)"
    echo "$released already records these pins for DeckPilot $record ($platform)"
    exit 0
  fi
fi

# One line per plugin built with the id and version of a released one but
# other files: "<id> <version>: released in DeckPilot <release> (<platform>) with other files: <paths>".
conflicts=$(jq -r --slurpfile built "$pins" --arg platform "$platform" '
  [.releases[] | select(.platform == $platform) | . as $r | .plugins[] | {release: $r.release, id, version, files}] as $shipped
  | $built[0].plugins[] as $p
  | $shipped[]
  | select(.id == $p.id and .version == $p.version and .files != $p.files)
  | . as $q
  | ([($q.files + $p.files) | keys[] | select($q.files[.] != $p.files[.])] | join(", ")) as $paths
  | "\($p.id) \($p.version): released in DeckPilot \($q.release) (\($platform)) with other files: \($paths)"
' "$released")
if [ -n "$conflicts" ]; then
  while IFS= read -r line; do echo "::error::$line" >&2; done <<< "$conflicts"
  fail "a bundled plugin has the id and version of a released one but other files: an installation upgraded from that release keeps its copy, which no longer matches the pins, and shows the plugin as broken. Give each changed plugin a new version (manifest.json, Cargo.toml and plugins/runtime/Cargo.lock; plugins/first-party/README.md, \"Versions\")"
fi

count=$(jq --arg platform "$platform" '[.releases[] | select(.platform == $platform)] | length' "$released")
if [ -z "$record" ]; then
  if [ "$count" -eq 0 ]; then
    echo "no release recorded pins for $platform in $released: nothing to compare"
  else
    echo "no bundled plugin reuses a released version with other files ($count releases recorded for $platform)"
  fi
  exit 0
fi

tmp="$released.tmp.$$"
jq --argjson entry "$entry" '.releases += [$entry]' "$released" > "$tmp"
mv "$tmp" "$released"
echo "recorded the pins of $(jq '.plugins | length' <<< "$entry") bundled plugins for DeckPilot $record ($platform) in $released"
