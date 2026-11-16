#!/usr/bin/env bash
# ok and bad always succeed, so `check && ok || bad` is an if-then-else here.
# shellcheck disable=SC2015
# Tests of the downstream distribution tools without a release, a token or
# a network: verify-release-input.sh against a stubbed gh serving a
# release shaped like v0.6.0-alpha.3 (with release-meta.json's artifacts
# computed by tools/release-pipeline/release-artifacts.py from
# docs/distribution/targets.toml), and generate.py's outputs for every
# channel compared with tools/distribution/tests/expected/.
#
#   tools/distribution/test.sh <work dir>
#   UPDATE_EXPECTED=1 tools/distribution/test.sh <work dir>   # rewrite expected/
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=${1:?usage: test.sh <work dir>}
rm -rf "$work" && mkdir -p "$work"
work=$(cd "$work" && pwd)
expected="$here/tests/expected"
failures=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; failures=$((failures + 1)); }

mkdir -p "$work/bin"
cp "$here/tests/gh_stub.py" "$work/bin/gh"
chmod +x "$work/bin/gh"
export PATH="$work/bin:$PATH"
export GITHUB_REPOSITORY=knittingcode/deckpilot

fixture() { python3 -I "$here/tests/fixture.py" "$@"; }
verify() { # <stub> <channel> <out> -> log in <out>.log
  GH_STUB_DIR="$1" "$here/verify-release-input.sh" v0.6.0-alpha.3 "$2" "$3" > "$3.log" 2>&1
}
generate() { # <channel> <input> <out> [args...] -> log in <out>.log
  local channel=$1 input=$2 out=$3; shift 3
  python3 -I "$here/generate.py" "$channel" --input "$input" --out "$out" "$@" > "$out.log" 2>&1
}
# compare <channel> <dir>: every generated file except logs equals expected/<channel>/.
compare() {
  local channel=$1 dir=$2
  if [ "${UPDATE_EXPECTED-}" = 1 ]; then
    rm -rf "${expected:?}/$channel" && mkdir -p "$expected/$channel"
    (cd "$dir" && find . -type f ! -name '*.log' ! -path './pool/*' | sort | while read -r f; do
      mkdir -p "$expected/$channel/$(dirname "$f")"; cp "$f" "$expected/$channel/$f"; done)
    ok "$channel: expected output rewritten"
    return
  fi
  # Packages copied into an APT pool are compared with their source below.
  if diff -r -x '*.log' -x pool "$expected/$channel" "$dir" > "$work/$channel.diff"; then
    ok "$channel: output equals tests/expected/$channel"
  else
    bad "$channel: output differs from tests/expected/$channel"; sed -n 1,40p "$work/$channel.diff"
  fi
}

echo "== the v0.6.0-alpha.3-shaped release"
stub="$work/release"
fixture build "$stub"
grep -q '"matrix_id": "github-macos-arm64"' "$stub/assets/release-meta.json" \
  && ok "release-meta.json carries artifacts[] from release-artifacts.py" \
  || bad "release-meta.json has no artifacts"

echo "== verification and generation, per channel"
for channel in homebrew winget flathub mas msstore; do
  out="$work/$channel"
  if verify "$stub" "$channel" "$out-input"; then
    ok "$channel: $(tail -1 "$out-input.log")"
  else
    bad "$channel: verification failed: $(cat "$out-input.log")"; continue
  fi
  extra=()
  [ "$channel" = winget ] && extra=(--product-code "{00000000-0000-0000-0000-000000000000}")
  if generate "$channel" "$out-input/input.json" "$out" ${extra[@]+"${extra[@]}"}; then
    compare "$channel" "$out"
  else
    bad "$channel: generation failed: $(cat "$out.log")"
  fi
done
# Only GETs and downloads reached gh.
if grep -vE '^(api -H Accept: application/vnd.github\+json repos/|release download )' "$stub/calls.log" > "$work/other-calls"; then
  bad "gh was called with something else: $(cat "$work/other-calls")"
else
  ok "gh saw only API reads and downloads ($(wc -l < "$stub/calls.log" | tr -d ' ') calls)"
fi
# Each job downloads only what it needs.
grep -q 'pattern DeckPilot_0.6.0-alpha.3_amd64.AppImage' "$stub/calls.log" \
  && bad "a job downloaded the AppImage, which no channel takes" || ok "no job downloads assets it does not take"

echo "== APT: the GitHub .deb is never republished (R7)"
if verify "$stub" apt "$work/apt-github"; then
  bad "apt accepted a release without the apt variant"
elif grep -q 'never republished to the APT repository (rule R7)' "$work/apt-github.log"; then
  ok "apt refuses a release without the apt variant"
else
  bad "apt: $(cat "$work/apt-github.log")"
fi

echo "== APT: the apt variant"
apt_stub="$work/release-apt"
fixture build "$apt_stub" --apt-deb-version 0.6.0~alpha.3
if verify "$apt_stub" apt "$work/apt-input" && generate apt "$work/apt-input/input.json" "$work/apt"; then
  compare apt "$work/apt"
  cmp -s "$apt_stub/assets/DeckPilot_0.6.0-alpha.3_amd64_apt.deb" "$work/apt/pool/main/d/deck-pilot/DeckPilot_0.6.0-alpha.3_amd64_apt.deb" \
    && ok "apt: the pool holds the verified package unchanged" || bad "apt: the pool's package differs from the release asset"
else
  bad "apt: $(cat "$work/apt-input.log" "$work/apt.log" 2>/dev/null)"
fi
# The bundler's verbatim version would sort 0.6.0-alpha.3 after 0.6.0 in dpkg.
bad_stub="$work/release-apt-verbatim"
fixture build "$bad_stub" --apt-deb-version 0.6.0-alpha.3
verify "$bad_stub" apt "$work/apt-verbatim-input" || bad "apt verbatim: $(cat "$work/apt-verbatim-input.log")"
if generate apt "$work/apt-verbatim-input/input.json" "$work/apt-verbatim"; then
  bad "apt accepted a package with the verbatim pre-release version"
elif grep -q "must carry the Debian form '0.6.0~alpha.3'" "$work/apt-verbatim.log"; then
  ok "apt refuses Version 0.6.0-alpha.3 (dpkg would order it after 0.6.0)"
else
  bad "apt verbatim: $(cat "$work/apt-verbatim.log")"
fi

echo "== verification refuses"
# <mutation> <channel> <expected message>
while IFS='|' read -r mutation channel message; do
  [ -n "$mutation" ] || continue
  case_stub="$work/case-$mutation"
  cp -R "$stub" "$case_stub"
  : > "$case_stub/calls.log"
  fixture mutate "$case_stub" "$mutation"
  if verify "$case_stub" "$channel" "$work/case-$mutation-out"; then
    bad "$mutation ($channel): accepted"
  elif grep -q -F -- "$message" "$work/case-$mutation-out.log"; then
    ok "$mutation ($channel): refused"
  else
    bad "$mutation ($channel): $(cat "$work/case-$mutation-out.log")"
  fi
done <<'EOF'
draft|homebrew|is a draft
mutable|homebrew|is not immutable
not-prerelease|winget|has prerelease=False
moved-tag|flathub|but release-meta.json records
tampered-dmg|homebrew|does not match SHA256SUMS
tampered-dmg-and-sums|homebrew|does not match release-meta.json
no-artifacts|winget|has no artifacts list
github-deb-as-apt|apt|was built for the distribution channel 'github'
msi-for-store|winget|was built for the distribution channel 'microsoft_store'
EOF
annotated="$work/case-annotated"
cp -R "$stub" "$annotated"
fixture mutate "$annotated" annotated-tag
verify "$annotated" mas "$work/case-annotated-out" \
  && ok "an annotated tag is followed to its commit" || bad "annotated tag: $(cat "$work/case-annotated-out.log")"
# The release-meta.json published with v0.6.0-alpha.3 predates artifacts[].
real="$work/case-real-alpha3"
cp -R "$stub" "$real"
fixture mutate "$real" published-alpha3-meta
if verify "$real" homebrew "$work/case-real-out"; then
  bad "the published alpha.3 release-meta.json (no artifacts) was accepted"
else
  grep -q 'predates the packaging matrix' "$work/case-real-out.log" \
    && ok "the published v0.6.0-alpha.3 metadata (no artifacts[]) is refused with a reason" \
    || bad "published alpha.3: $(cat "$work/case-real-out.log")"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)"
  exit 1
fi
echo "all distribution tool tests passed"
