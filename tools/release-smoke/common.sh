#!/usr/bin/env bash
# Shared by the release smoke scripts (sourced, not run); the workflow is
# .github/workflows/release-smoke.yml.
#
# Every check is recorded as one line of $SMOKE_OUT/results.tsv:
#   <platform> TAB <step> TAB pass|fail|error|not-covered TAB <evidence>
# - fail: the release failed a product check (a defect of the release);
# - error: the test itself could not work (a download, a checksum
#   mismatch, a missing tool or service on the runner);
# - not-covered: the check cannot be made on this runner, with the
#   reason; never counted as a pass.
# A failed check does not stop the job; tools/release-smoke/summary.sh
# writes the table to the job summary at the end and decides the job's
# outcome: errors always fail it, failed product checks only when
# SMOKE_STRICT=true (a manual run validating a release; on pull requests,
# which test the workflow against an already published release, they are
# reported as warnings).
#
# Environment: SMOKE_OUT (results, logs, screenshots), SMOKE_PLATFORM (the
# row label, e.g. linux, windows, macos-arm64).

: "${SMOKE_OUT:?SMOKE_OUT is not set}"
: "${SMOKE_PLATFORM:?SMOKE_PLATFORM is not set}"
mkdir -p "$SMOKE_OUT"
SMOKE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TAG_PATTERN='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'
REPOSITORY="${REPOSITORY:-knittingcode/deckpilot}"
# shellcheck disable=SC2034 # used by the scripts that source this file
APP_IDENTIFIER="io.github.knittingcode.deckpilot"
# The first-party plugins every release bundles
# (plugins/first-party/bundled.json).
BUNDLED_IDS="deckpilot.github deckpilot.gitlab deckpilot.jira deckpilot.notifications"
# The production registry's root role (public keys only).
ROOTS_URL="https://raw.githubusercontent.com/knittingcode/deckpilot-plugins/main/keys/roots.json"

record() {
  local step="$1" result="$2" evidence="${3-}"
  evidence=$(printf '%s' "$evidence" | tr '\t\r\n' '   ' | cut -c1-900)
  printf '%s\t%s\t%s\t%s\n' "$SMOKE_PLATFORM" "$step" "$result" "$evidence" >> "$SMOKE_OUT/results.tsv"
  case "$result" in
    pass) echo "PASS  $step: $evidence" ;;
    fail)
      if [ "${SMOKE_STRICT:-true}" = true ]; then echo "::error title=$step::$evidence"
      else echo "::warning title=product defect: $step::$evidence"; fi ;;
    error) echo "::error title=test error: $step::$evidence" ;;
    not-covered) echo "::notice title=$step::not covered: $evidence" ;;
    *) echo "::error::record: unknown result $result for $step" ;;
  esac
}

require_tag() {
  [[ "$1" =~ $TAG_PATTERN ]] || { echo "::error::'$1' is not a release tag" >&2; exit 2; }
}

sha256_of() {
  if command -v sha256sum > /dev/null; then sha256sum < "$1" | cut -d' ' -f1; else shasum -a 256 < "$1" | cut -d' ' -f1; fi
}

# native <path>: a path Windows programs understand (D:/a/...), elsewhere as is.
native() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s\n' "$1"; fi; }

# The production root public keys, one per line (fetched once).
root_keys() {
  local file="$SMOKE_OUT/.registry-root-role.json"
  if [ ! -s "$file" ]; then
    curl -fsSL --retry 5 --retry-all-errors --retry-delay 5 -o "$file" "$ROOTS_URL" \
      || gh api repos/knittingcode/deckpilot-plugins/contents/keys/roots.json --jq .content | base64 -d > "$file" \
      || return 1
  fi
  jq -r '.keys[]' "$file" | tr -d '\r'
}

# check_roots <step> <agent>: the agent embeds every production root key.
check_roots() {
  local step="$1" agent="$2" key missing="" count=0
  local keys
  keys=$(root_keys) || { record "$step" error "cannot fetch $ROOTS_URL"; return; }
  [ -n "$keys" ] || { record "$step" error "no keys in $ROOTS_URL"; return; }
  while IFS= read -r key; do
    [ -n "$key" ] || continue
    if grep -q -a -F -- "$key" "$agent"; then count=$((count + 1)); else missing="$missing ${key:0:24}..."; fi
  done <<< "$keys"
  if [ -z "$missing" ]; then
    local ids
    ids=$(while IFS= read -r key; do base64 -d <<< "$key" 2> /dev/null | sed -n 's/.*public key:* //p' | head -n1; done <<< "$keys" | tr '\n' ' ')
    record "$step" pass "$count/$count root keys of deckpilot-plugins keys/roots.json found in $(basename "$agent") (key ids $ids)"
  else
    record "$step" fail "missing from $agent:$missing"
  fi
}

# check_updater_key <step> <desktop binary>: the pinned updater public key
# (.github/updater.pub, as DECKPILOT_UPDATER_PUBKEY) is compiled in.
check_updater_key() {
  local step="$1" binary="$2" key
  key=$(tr -d '\r\n' < "$SMOKE_ROOT/.github/updater.pub")
  if grep -q -a -F -- "$key" "$binary"; then
    record "$step" pass "$(basename "$binary") embeds .github/updater.pub (key id $(base64 -d <<< "$key" 2> /dev/null | sed -n 's/.*public key: //p' | head -n1))"
  else
    record "$step" fail "$binary does not contain .github/updater.pub"
  fi
}

# check_bundled <step> <bundled dir> <agent> [--without-running]: the
# release's own check (tools/release-pipeline/check-bundled-plugins.sh):
# pins.json, exactly the pinned files with their digests, and the pins
# compiled into the agent; plus the expected set of plugins.
check_bundled() {
  local step="$1" dir="$2" agent="$3" mode="${4-}" out ids
  if [ ! -f "$dir/pins.json" ]; then record "$step" fail "$dir/pins.json is missing"; return; fi
  ids=$(jq -r '.plugins[] | "\(.id)@\(.version)"' "$dir/pins.json" | tr -d '\r' | LC_ALL=C sort | tr '\n' ' ')
  local got want
  got=$(jq -r '.plugins[].id' "$dir/pins.json" | tr -d '\r' | LC_ALL=C sort | tr '\n' ' ')
  want=$(printf '%s\n' "$BUNDLED_IDS" | tr ' ' '\n' | LC_ALL=C sort | tr '\n' ' ')
  if [ "$got" != "$want" ]; then record "$step" fail "bundled plugins are '$got', expected '$want'"; return; fi
  # shellcheck disable=SC2086 # $mode is empty or one flag
  if out=$("$SMOKE_ROOT/tools/release-pipeline/check-bundled-plugins.sh" "$dir" "$agent" $mode 2>&1); then
    record "$step" pass "$ids; $(printf '%s' "$out" | tail -n1)"
  else
    record "$step" fail "$(printf '%s' "$out" | tail -n3)"
  fi
}

# check_exec <step> <file>...: present and executable.
check_exec() {
  local step="$1" missing="" file
  shift
  for file in "$@"; do
    [ -f "$file" ] && [ -x "$file" ] || missing="$missing $file"
  done
  if [ -z "$missing" ]; then
    record "$step" pass "$(for f in "$@"; do printf '%s (%s bytes) ' "$f" "$(wc -c < "$f" | tr -d ' ')"; done)"
  else
    record "$step" fail "missing or not executable:$missing"
  fi
}

# check_version <step> <agent> <version>: `deckpilot-agent --version`.
check_version() {
  local step="$1" agent="$2" version="$3" reported
  reported=$("$agent" --version 2>&1 | tr -d '\r' | head -n1)
  if [ "$reported" = "deckpilot-agent $version" ]; then
    record "$step" pass "'$reported'"
  else
    record "$step" fail "'$agent --version' printed '$reported', expected 'deckpilot-agent $version'"
  fi
}

# encrypted_db <file>: a non-empty file that is not a plain SQLite database.
encrypted_db() {
  [ -s "$1" ] || return 1
  [ "$(head -c 15 "$1")" != "SQLite format 3" ]
}
