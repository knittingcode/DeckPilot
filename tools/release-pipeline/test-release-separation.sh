#!/usr/bin/env bash
# Tests of check-release-separation.py (docs/marketplace/distribution.md):
# this repository's workflows pass; every fixture under
# fixtures/release-separation/reject fails with the message its first line
# names ("# expect: <message>"), alone and when added to this repository's
# workflows; every fixture under fixtures/release-separation/accept passes
# (among them a plugin host staged into the application bundle).
#
#   tools/release-pipeline/test-release-separation.sh [<work dir>]
#
# Needs bash and python3 with PyYAML; nothing else.
# ok and bad never fail, so `check && ok || bad` is an if-then-else here.
# shellcheck disable=SC2015
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
here="$root/tools/release-pipeline"
fixtures="$here/fixtures/release-separation"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/release-separation-test.XXXXXX")}"
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
separation() { python3 -I "$here/check-release-separation.py" "$1" > "$work/separation.log" 2>&1; }
# workflows <fixture> [with-repository]: a workflows directory with <fixture>.
workflows() {
  rm -rf "$work/workflows" && mkdir -p "$work/workflows"
  [ "${2-}" != with-repository ] || cp "$root"/.github/workflows/*.yml "$work/workflows/"
  cp "$1" "$work/workflows/zz-fixture.yml"
}

echo "== this repository"
separation "$root/.github/workflows" && ok "$(cat "$work/separation.log")" || bad "this repository: $(cat "$work/separation.log")"
mkdir -p "$work/empty"
separation "$work/empty" && bad "an empty directory passes" || ok "an empty directory fails closed"

echo "== rejected"
count=0
for fixture in "$fixtures"/reject/*.yml; do
  name=$(basename "$fixture" .yml)
  expected=$(sed -n '1s/^# expect: //p' "$fixture")
  [ -n "$expected" ] || {
    bad "$name: no '# expect:' line"
    continue
  }
  for mode in alone with-repository; do
    workflows "$fixture" "$mode"
    if separation "$work/workflows"; then
      bad "$name ($mode): passes"
    elif grep -q -F -- "$expected" "$work/separation.log"; then
      ok "$name ($mode) is caught"
    else
      bad "$name ($mode): $(cat "$work/separation.log")"
    fi
  done
  count=$((count + 1))
done
[ "$count" -ge 10 ] && ok "$count rejection fixtures" || bad "only $count rejection fixtures"

echo "== accepted"
for fixture in "$fixtures"/accept/*.yml; do
  name=$(basename "$fixture" .yml)
  workflows "$fixture" with-repository
  separation "$work/workflows" && ok "$name passes" || bad "$name: $(cat "$work/separation.log")"
done

echo
echo "$passed passed, $failures failed (work dir: $work)"
[ "$failures" -eq 0 ]
