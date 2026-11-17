#!/usr/bin/env bash
# Tests of check-bundled-plugins.sh (docs/marketplace/module-migration.md
# §4.5): a bundle of unpacked first-party plugins whose files match the
# pins, and the pins the agent was compiled with, passes; every way of
# shipping something else fails with the message it names.
#
#   tools/release-pipeline/test-bundled-plugins.sh [<work dir>]
#
# The bundles are made here (no build): a stand-in plugin.wasm, the test
# fixture's manifest, and an agent stub that prints the pins it "was
# compiled with". Needs bash, jq and sha256sum or shasum.
# ok and bad never fail, so `check && ok || bad` is an if-then-else here.
# shellcheck disable=SC2015
set -euo pipefail
# jq on Windows ends its lines with CRLF; every value here is one line.
jq() { command jq "$@" | tr -d '\r'; }

root="$(cd "$(dirname "$0")/../.." && pwd)"
here="$root/tools/release-pipeline"
fixture="$root/plugins/first-party/test-fixtures/example-default"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/bundled-plugins-test.XXXXXX")}"
mkdir -p "$work"
work="$(cd "$work" && pwd)"
passed=0 failures=0
ok() {
  echo "ok   $*"
  passed=$((passed + 1))
}
bad() {
  echo "FAIL $*"
  failures=$((failures + 1))
}

if command -v sha256sum > /dev/null; then
  sha() { sha256sum < "$1" | cut -d' ' -f1; }
else
  sha() { shasum -a 256 < "$1" | cut -d' ' -f1; }
fi
size() { wc -c < "$1" | tr -d ' '; }

id=deckpilot.example-default

# pin <bundled dir>: writes pins.json for every plugin directory in it, as
# plugins/first-party/build.sh does.
pin() {
  local bundled="$1" plugins='[]' dir pid files file
  for dir in "$bundled"/*/; do
    [ -d "$dir" ] || continue
    pid=$(basename "$dir")
    files='{}'
    while IFS= read -r file; do
      files=$(jq --arg f "$file" --arg s "$(sha "$dir/$file")" --argjson n "$(size "$dir/$file")" \
        '. + {($f): {sha256: $s, size: $n}}' <<< "$files")
    done < <(cd "$dir" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
    plugins=$(jq --arg id "$pid" --arg v "$(jq -r .version "$dir/manifest.json")" --argjson f "$files" \
      '. + [{id: $id, version: $v, class: "default", files: $f}]' <<< "$plugins")
  done
  jq -S -n --argjson p "$plugins" '{pinsVersion: 1, plugins: $p}' > "$bundled/pins.json"
}

# fresh: a good bundle in $work/case/bundled and an agent that pins it.
fresh() {
  rm -rf "$work/case"
  local dir="$work/case/bundled/$id"
  mkdir -p "$dir/assets"
  cp "$fixture/manifest.json" "$fixture/LICENSE" "$dir/"
  cp "$fixture/assets/icon-128.png" "$dir/assets/"
  printf '\0asm\1\0\0\0stand-in module' > "$dir/plugin.wasm"
  pin "$work/case/bundled"
  agent_pins "$work/case/bundled/pins.json"
}

# agent_pins <file>: the agent stub prints <file> as its compiled pins.
agent_pins() {
  cp "$1" "$work/case/compiled.json"
  # shellcheck disable=SC2016 # $1 belongs to the stub
  printf '#!/usr/bin/env bash\n[ "$1" = --print-bundled-pins ] || exit 2\ncat "%s"\n' \
    "$work/case/compiled.json" > "$work/case/agent"
  chmod +x "$work/case/agent"
}

check() { "$here/check-bundled-plugins.sh" "$work/case/bundled" "$work/case/agent" > "$work/check.log" 2>&1; }

passes() {
  check && ok "$1" || bad "$1: $(cat "$work/check.log")"
}

# fails <name> <expected message>
fails() {
  if check; then
    bad "$1: passes"
  elif grep -q -F -- "$2" "$work/check.log"; then
    ok "$1 is refused"
  else
    bad "$1: $(cat "$work/check.log")"
  fi
}

plugin="$work/case/bundled/$id"

echo "== accepted"
fresh
passes "a bundled first-party plugin matching its pins and the agent's"
rm -rf "$work/case" && mkdir -p "$work/case/bundled"
printf '{"pinsVersion":1,"plugins":[]}\n' > "$work/case/bundled/pins.json"
agent_pins "$work/case/bundled/pins.json"
passes "an empty bundle (no plugin pinned)"
# The pins as the agent prints them may be formatted differently.
fresh
jq -c . "$work/case/bundled/pins.json" > "$work/case/compact.json"
agent_pins "$work/case/compact.json"
passes "the same pins, formatted differently"
# A bundle under a path with a backslash, like D:\a\_temp on Windows: GNU
# sha256sum escapes such a file name and prefixes its digest with "\", which
# made every digest differ from its pin.
fresh
rm -rf "$work/back\\slash" && mkdir -p "$work/back\\slash"
cp -R "$work/case/bundled" "$work/back\\slash/bundled"
if "$here/check-bundled-plugins.sh" "$work/back\\slash/bundled" "$work/case/agent" > "$work/check.log" 2>&1; then
  ok "a bundle under a path with a backslash"
else bad "a bundle under a path with a backslash: $(cat "$work/check.log")"; fi

echo "== tampered files"
fresh
printf 'X' >> "$plugin/plugin.wasm"
fails "a longer plugin.wasm" "plugin.wasm has"
fresh
sed -i.bak 's/Bundled Example/Bundled Exampl3/' "$plugin/manifest.json" && rm "$plugin/manifest.json.bak"
fails "a changed manifest of the same size" "manifest.json does not have its pinned sha256"
fresh
jq '.version = "0.1.1"' "$fixture/manifest.json" > "$plugin/manifest.json"
pin "$work/case/bundled"
jq '.plugins[0].version = "0.1.0"' "$work/case/bundled/pins.json" > "$work/case/p.json"
mv "$work/case/p.json" "$work/case/bundled/pins.json"
agent_pins "$work/case/bundled/pins.json"
fails "a manifest of another version" "manifest.json is deckpilot.example-default@0.1.1"

echo "== files that are not allowed"
fresh
cp "$plugin/plugin.wasm" "$work/case/bundled/$id-0.1.0.dpplugin"
fails "a .dpplugin next to the plugins" "contains plugin packages or signatures"
fresh
cp "$plugin/plugin.wasm" "$plugin/$id-0.1.0.dpplugin"
pin "$work/case/bundled"
agent_pins "$work/case/bundled/pins.json"
fails "a pinned .dpplugin inside a plugin" "contains plugin packages or signatures"
fresh
printf 'sig' > "$plugin/plugin.wasm.minisig"
fails "a signature" "contains plugin packages or signatures"
fresh
printf 'x' > "$plugin/README.md"
fails "an unpinned extra file" "README.md is not allowed"
fresh
printf 'x' > "$plugin/plugin.js"
pin "$work/case/bundled"
agent_pins "$work/case/bundled/pins.json"
fails "a pinned file that is not allowed" "plugin.js is not allowed"
fresh
rm "$plugin/LICENSE"
fails "a missing pinned file" "pinned LICENSE is missing"
fresh
ln -s "$fixture/LICENSE" "$plugin/NOTICE"
fails "a symbolic link" "symbolic links"
fresh
printf '{}' > "$work/case/bundled/index.json"
fails "a file at the top level (registry-like metadata)" "bundled/index.json is not listed"
fresh
mkdir -p "$work/case/bundled/deckpilot.unlisted"
fails "a plugin directory pins.json does not list" "bundled/deckpilot.unlisted is not listed"

echo "== pins"
fresh
rm "$work/case/bundled/pins.json"
fails "no pins.json" "has no pins.json"
fresh
jq '.pinsVersion = 2' "$work/case/bundled/pins.json" > "$work/case/p.json"
mv "$work/case/p.json" "$work/case/bundled/pins.json"
agent_pins "$work/case/bundled/pins.json"
fails "another pins format" "is not pins format 1"
fresh
mv "$plugin" "$work/case/bundled/acme.example-default"
pin "$work/case/bundled"
agent_pins "$work/case/bundled/pins.json"
fails "a third-party plugin" "is not a first-party plugin id"
fresh
jq '.plugins += .plugins' "$work/case/bundled/pins.json" > "$work/case/p.json"
mv "$work/case/p.json" "$work/case/bundled/pins.json"
agent_pins "$work/case/bundled/pins.json"
fails "a plugin listed twice" "lists a plugin twice"
fresh
jq '.plugins[0].class = "optional"' "$work/case/bundled/pins.json" > "$work/case/p.json"
mv "$work/case/p.json" "$work/case/bundled/pins.json"
agent_pins "$work/case/bundled/pins.json"
fails "an unknown class" "has class 'optional'"
fresh
jq '.plugins[0].files.LICENSE.sha256 = ("0" * 64)' "$work/case/bundled/pins.json" > "$work/case/p.json"
mv "$work/case/p.json" "$work/case/bundled/pins.json"
agent_pins "$work/case/bundled/pins.json"
fails "a wrong digest in pins.json" "LICENSE does not have its pinned sha256"
fresh
rm -rf "$plugin"
fails "a listed plugin that is missing" "bundled/$id is missing"

echo "== the agent's compiled pins"
fresh
printf '{"pinsVersion":1,"plugins":[]}\n' > "$work/case/empty.json"
agent_pins "$work/case/empty.json"
fails "an agent compiled without these pins" "differs from the pins compiled into"
fresh
# A bundle re-pinned after the agent was built (a swapped plugin).
printf 'X' >> "$plugin/plugin.wasm"
cp "$work/case/compiled.json" "$work/case/old.json"
pin "$work/case/bundled"
agent_pins "$work/case/old.json"
fails "files re-pinned after the agent was built" "differs from the pins compiled into"
fresh
printf '#!/usr/bin/env bash\nexit 2\n' > "$work/case/agent"
fails "an agent without --print-bundled-pins" "--print-bundled-pins failed"

echo "== an agent of another architecture (--without-running)"
check() { "$here/check-bundled-plugins.sh" "$work/case/bundled" "$work/case/agent" --without-running > "$work/check.log" 2>&1; }
# embedded <file>: an "executable" this machine cannot run, with <file>'s
# bytes inside, as include_bytes! leaves them.
embedded() {
  { printf '\317\372\355\376 not runnable here '; cat "$1"; printf ' more code'; } > "$work/case/agent"
  chmod -x "$work/case/agent"
}
fresh
embedded "$work/case/bundled/pins.json"
passes "an agent that embeds exactly pins.json, not run"
fresh
jq -c . "$work/case/bundled/pins.json" > "$work/case/compact.json"
embedded "$work/case/compact.json"
fails "an agent that embeds other bytes" "pins.json is not compiled into"
fresh
printf 'X' >> "$plugin/plugin.wasm"
cp "$work/case/bundled/pins.json" "$work/case/old.json"
pin "$work/case/bundled"
embedded "$work/case/old.json"
fails "files re-pinned after the agent was built (not run)" "pins.json is not compiled into"
fresh
embedded "$work/case/bundled/pins.json"
printf 'X' >> "$plugin/plugin.wasm"
fails "a tampered file (not run)" "plugin.wasm has"
fresh
rm "$work/case/agent"
fails "no agent (not run)" "does not exist"

echo
echo "$passed passed, $failures failed (work dir: $work)"
[ "$failures" -eq 0 ]
