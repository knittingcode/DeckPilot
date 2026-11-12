#!/usr/bin/env bash
# Tests of the two-stage publication without GitHub: releases are kept by
# gh-stub.sh and served to "users" by curl-stub.sh, in <work dir>. Covers
# draft creation and tag reuse, the signing record (failed signing and
# notarization), required assets and channel isolation, the publication
# gate, publication as pre-release or (latest) stable, everything that
# must keep a draft unpublished, idempotent re-runs and recovery.
#
#   tools/release-pipeline/test-publication.sh [<work dir>]
#
# Needs bash and jq; no git, cargo or network (signature verification is stubbed
# here; it is tested by tools/ci/test-latest-json.sh and deckpilot-dev).
# ok and bad never fail, so `check && ok || bad` is an if-then-else here.
# shellcheck disable=SC2015
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/release-publication-test.XXXXXX")}"
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
unset GITHUB_OUTPUT GITHUB_REPOSITORY GH_TOKEN GITHUB_TOKEN
export GH="$here/gh-stub.sh" CURL="$here/curl-stub.sh" REPOSITORY=example/DeckPilot
export VERIFY_RELEASE=true
base="https://github.com/$REPOSITORY/releases"
sha_a=$(printf 'a%.0s' {1..40})
sha_b=$(printf 'b%.0s' {1..40})
sha256() { if command -v sha256sum > /dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }

# fresh: an empty GitHub.
fresh() {
  export STUB_DIR="$work/stub"
  rm -rf "$STUB_DIR" && mkdir -p "$STUB_DIR/tags" "$STUB_DIR/environments"
}

# run <log> <command...>: run quietly, keeping the output.
run() {
  local log="$1"
  shift
  "$@" > "$work/$log" 2>&1
}

# expect_failure <what> <expected message> <command...>
expect_failure() {
  local what="$1" message="$2"
  shift 2
  if run fail.log "$@"; then
    bad "$what: succeeded"
  elif grep -q -F -- "$message" "$work/fail.log"; then
    ok "$what"
  else
    bad "$what: unexpected: $(tail -n 3 "$work/fail.log")"
  fi
}

# release <tag>: the stub's release JSON of <tag>.
release() { jq -s --arg t "$1" '.[] | select(.tag_name == $t)' "$STUB_DIR"/releases/*.json; }

# env_for <version>: the tag gate's outputs for <version>.
env_for() {
  export VERSION="$1" SHA="${2:-$sha_a}"
  if [[ "$VERSION" == *-* ]]; then
    export CHANNEL=prerelease PRERELEASE=true MANIFEST=prerelease.json
  else
    export CHANNEL=stable PRERELEASE=false MANIFEST=latest.json
  fi
}

# signing <dir> [<target>=<codesign>/<notarized>...]: signing records of
# the four builds, macOS signed and notarized unless overridden.
signing() {
  local dir="$1"
  shift
  rm -rf "$dir" && mkdir -p "$dir"
  local spec target codesign notarized
  for spec in aarch64-apple-darwin=developer-id/true x86_64-apple-darwin=developer-id/true \
    x86_64-pc-windows-msvc=unsigned/null x86_64-unknown-linux-gnu=unsigned/null "$@"; do
    target="${spec%%=*}" codesign="${spec#*=}"
    notarized="${codesign#*/}" codesign="${codesign%/*}"
    if [ "$codesign" = missing ]; then
      rm -f "$dir/$target.json"
      continue
    fi
    jq -n --arg t "$target" --arg c "$codesign" --argjson n "$notarized" \
      '{target: $t, codesign: $c, notarized: $n}' > "$dir/$target.json"
  done
}

# draft <tag>: stage A as the workflow runs it, against the stub: the draft,
# its assets, the manifest, the signing record and SHA256SUMS.
draft() {
  local tag="$1" v="${1#v}"
  echo "$SHA" > "$STUB_DIR/tags/$tag"
  printf '### Added\n\n- Things.\n' > "$work/notes.md"
  NOTES_FILE="$work/notes.md" "$here/create-draft.sh" "$tag" > "$work/draft.out"
  local id
  id=$(sed -n 's/^release_id=//p' "$work/draft.out")
  local assets="$STUB_DIR/assets/$id"
  local names=(
    "DeckPilot_${v}_aarch64.dmg" "DeckPilot_${v}_x64.dmg"
    DeckPilot_aarch64.app.tar.gz DeckPilot_aarch64.app.tar.gz.sig
    DeckPilot_x64.app.tar.gz DeckPilot_x64.app.tar.gz.sig
    "DeckPilot_${v}_amd64.AppImage" "DeckPilot_${v}_amd64.AppImage.sig"
    "DeckPilot_${v}_amd64.deb" "DeckPilot_${v}_amd64.deb.sig"
    "DeckPilot_${v}_x64_en-US.msi" "DeckPilot_${v}_x64_en-US.msi.sig"
    "DeckPilot_${v}_x64-setup.exe" "DeckPilot_${v}_x64-setup.exe.sig"
  )
  [ "$PRERELEASE" = true ] || names+=("DeckPilot-${v}-1.x86_64.rpm" "DeckPilot-${v}-1.x86_64.rpm.sig")
  local name
  for name in "${names[@]}"; do printf 'content of %s\n' "$name" > "$assets/$name"; done
  jq -n --arg v "$v" --arg base "$base/download/$tag/" '{version: $v, notes: "n", platforms: {
      "darwin-aarch64": {signature: "s", url: ($base + "DeckPilot_aarch64.app.tar.gz")},
      "windows-x86_64": {signature: "s", url: ($base + "DeckPilot_" + $v + "_x64_en-US.msi")}}}' \
    > "$assets/$MANIFEST"
  signing "$work/signing"
  TAG="$tag" NOTES_FILE="$work/notes.md" MACOS_SIGNING=true \
    "$here/release-meta.sh" "$work/signing" "$assets/release-meta.json" "$work/body.md" > /dev/null
  "$GH" api -X PATCH "repos/$REPOSITORY/releases/$id" -F body=@"$work/body.md" > /dev/null
  (cd "$assets" && sha256 -- * > "$work/SHA256SUMS" && mv "$work/SHA256SUMS" SHA256SUMS)
  echo "$id"
}

publish() { WORK="$work/publish" "$here/publish.sh" "$1"; }

echo "== draft creation and tag reuse"
fresh
env_for 0.6.0-alpha.1
printf 'notes\n' > "$work/notes.md"
run create.log env NOTES_FILE="$work/notes.md" "$here/create-draft.sh" v0.6.0-alpha.1 \
  && grep -q "^state=new" "$work/create.log" && ok "a new tag gets a draft" || bad "new draft: $(cat "$work/create.log")"
[ "$(release v0.6.0-alpha.1 | jq -r '[.draft, .prerelease, .target_commitish] | join(" ")')" = "true true $sha_a" ] \
  && ok "the draft is a pre-release of the tagged commit" || bad "draft fields: $(release v0.6.0-alpha.1)"
run create.log env NOTES_FILE="$work/notes.md" "$here/create-draft.sh" v0.6.0-alpha.1 \
  && grep -q "^state=draft" "$work/create.log" && ok "a re-run reuses the draft of the same commit" || bad "re-run: $(cat "$work/create.log")"
expect_failure "a moved tag (draft of another commit)" "the tag was moved or reused" \
  env SHA="$sha_b" NOTES_FILE="$work/notes.md" "$here/create-draft.sh" v0.6.0-alpha.1
"$GH" api -X POST "repos/$REPOSITORY/releases" -f tag_name=v0.6.0-alpha.1 -F draft=true > /dev/null
expect_failure "two drafts for one tag" "2 releases exist" env NOTES_FILE="$work/notes.md" "$here/create-draft.sh" v0.6.0-alpha.1
fresh
"$GH" api -X POST "repos/$REPOSITORY/releases" -f tag_name=v0.6.0-alpha.1 -f target_commitish="$sha_a" -F draft=false > /dev/null
expect_failure "a published release is never rebuilt" "already published" \
  env NOTES_FILE="$work/notes.md" "$here/create-draft.sh" v0.6.0-alpha.1
expect_failure "a malformed tag" "is not a release tag" env NOTES_FILE="$work/notes.md" "$here/create-draft.sh" v0.6.0-1
: > "$work/empty.md"
expect_failure "empty release notes" "release notes are empty" env NOTES_FILE="$work/empty.md" "$here/create-draft.sh" v0.6.0-alpha.2

echo "== signing record"
env_for 0.6.0
meta() { TAG=v0.6.0 NOTES_FILE="$work/notes.md" "$here/release-meta.sh" "$work/signing" "$work/meta.json" "$work/body.md"; }
signing "$work/signing"
if MACOS_SIGNING=true run meta.log meta; then
  ok "signed and notarized macOS builds are recorded"
  grep -q -F "| x86_64-pc-windows-msvc | **unsigned** | n/a |" "$work/body.md" \
    && ok "unsigned Windows installers are recorded as unsigned in the notes" || bad "notes: $(cat "$work/body.md")"
  [ "$(jq -r '.platforms["x86_64-unknown-linux-gnu"].codesign, .commit, .manifest' "$work/meta.json" | tr '\n' ' ')" = "unsigned $sha_a latest.json " ] \
    && ok "release-meta.json records commit, manifest and signatures" || bad "meta: $(cat "$work/meta.json")"
else
  bad "signing record: $(cat "$work/meta.log")"
fi
signing "$work/signing" aarch64-apple-darwin=unsigned/false
MACOS_SIGNING=true expect_failure "failed signing" "aarch64-apple-darwin: signing failed" meta
signing "$work/signing" x86_64-apple-darwin=developer-id/false
MACOS_SIGNING=true expect_failure "failed notarization" "x86_64-apple-darwin: notarization failed" meta
signing "$work/signing" x86_64-pc-windows-msvc=missing/null
MACOS_SIGNING=true expect_failure "a build without a signing record" "x86_64-pc-windows-msvc: 0 signing records" meta
signing "$work/signing" x86_64-unknown-linux-gnu=authenticode/null
MACOS_SIGNING=true expect_failure "a signature that cannot exist is not claimed" "Linux packages carry no code signature" meta
signing "$work/signing" aarch64-apple-darwin=unsigned/false x86_64-apple-darwin=unsigned/false
MACOS_SIGNING=false run meta.log meta && grep -q -F "| aarch64-apple-darwin | **unsigned** | no |" "$work/body.md" \
  && ok "deliberately unsigned macOS builds are recorded as unsigned" || bad "unsigned macOS: $(cat "$work/meta.log")"
signing "$work/signing"
MACOS_SIGNING=false expect_failure "signed builds without configured signing" "signing was not configured" meta

echo "== required assets and channel isolation"
assets_of() { (cd "$STUB_DIR/assets/$1" && ls -1) > "$work/assets.txt"; }
fresh
env_for 0.6.0
id=$(draft v0.6.0)
assets_of "$id"
run assets.log "$here/check-assets.sh" "$work/assets.txt" --final && ok "a complete stable release" || bad "complete: $(cat "$work/assets.log")"
grep -v -x "DeckPilot-0.6.0-1.x86_64.rpm" "$work/assets.txt" > "$work/a.txt"
expect_failure "a stable release without the .rpm" "lacks: DeckPilot-0.6.0-1.x86_64.rpm" "$here/check-assets.sh" "$work/a.txt"
grep -v -x "DeckPilot_0.6.0_x64-setup.exe.sig" "$work/assets.txt" > "$work/a.txt"
expect_failure "a package without its updater signature" "lacks: DeckPilot_0.6.0_x64-setup.exe.sig" "$here/check-assets.sh" "$work/a.txt"
grep -v -x "latest.json" "$work/assets.txt" > "$work/a.txt"
expect_failure "a release without its manifest (incomplete manifest)" "lacks: latest.json" "$here/check-assets.sh" "$work/a.txt"
grep -v -x "SHA256SUMS" "$work/assets.txt" > "$work/a.txt"
expect_failure "a draft without SHA256SUMS" "lacks: SHA256SUMS" "$here/check-assets.sh" "$work/a.txt" --final
{ cat "$work/assets.txt"; echo prerelease.json; } > "$work/a.txt"
expect_failure "a stable release with the pre-release manifest" "contains prerelease.json" "$here/check-assets.sh" "$work/a.txt"
env_for 0.6.0-beta.1
fresh
id=$(draft v0.6.0-beta.1)
assets_of "$id"
run assets.log "$here/check-assets.sh" "$work/assets.txt" --final && ok "a pre-release needs no .rpm" || bad "pre-release: $(cat "$work/assets.log")"
{ cat "$work/assets.txt"; echo latest.json; } > "$work/a.txt"
expect_failure "a pre-release with latest.json" "contains latest.json" "$here/check-assets.sh" "$work/a.txt"

echo "== publication gate"
fresh
expect_failure "no production-release environment" "does not exist" "$here/check-environment.sh"
echo '{"name": "release", "protection_rules": [{"type": "required_reviewers", "reviewers": [{"type": "User"}]}]}' > "$STUB_DIR/environments/release.json"
expect_failure "a protected environment of another name does not count" "'production-release' does not exist" "$here/check-environment.sh"
echo '{"name": "production-release", "protection_rules": []}' > "$STUB_DIR/environments/production-release.json"
expect_failure "an environment without reviewers" "requires no reviewer" "$here/check-environment.sh" production-release
echo '{"name": "production-release", "protection_rules": [{"type": "wait_timer", "wait_timer": 5}]}' > "$STUB_DIR/environments/production-release.json"
expect_failure "an environment with only a wait timer" "requires no reviewer" "$here/check-environment.sh" production-release
echo '{"name": "production-release", "protection_rules": [{"type": "required_reviewers", "reviewers": [{"type": "User", "reviewer": {"login": "maintainer"}}]}]}' \
  > "$STUB_DIR/environments/production-release.json"
run env.log "$here/check-environment.sh" production-release && ok "an environment with a required reviewer" || bad "environment: $(cat "$work/env.log")"

echo "== publication"
# expect_draft <what> <message> <tag>: publication refused, still a draft.
expect_draft() {
  expect_failure "$1" "$2" publish "$3"
  [ "$(release "$3" | jq -r .draft)" = true ] && ok "$1: still a draft" || bad "$1: published anyway"
}
fresh
env_for 0.6.0-alpha.1
draft v0.6.0-alpha.1 > /dev/null
if run publish.log publish v0.6.0-alpha.1; then
  ok "a pre-release is published"
  [ "$(release v0.6.0-alpha.1 | jq -r '[.draft, .prerelease] | join(" ")')" = "false true" ] \
    && ok "as a GitHub pre-release" || bad "flags: $(release v0.6.0-alpha.1)"
  [ ! -f "$STUB_DIR/latest" ] && ok "a pre-release never becomes latest" || bad "latest: $(cat "$STUB_DIR/latest")"
  grep -q -F "curl $base/download/v0.6.0-alpha.1/prerelease.json" "$STUB_DIR/calls.log" \
    && ok "its prerelease.json is verified from the public URL" || bad "manifest URL not verified"
  grep -q "the stable channel has no release yet" "$work/publish.log" && ok "the stable channel stays empty" || bad "stable channel: $(cat "$work/publish.log")"
else
  bad "pre-release publication: $(cat "$work/publish.log")"
fi
patches=$(grep -c -- "-X PATCH repos/$REPOSITORY/releases/[0-9]* -F draft=false" "$STUB_DIR/calls.log" || true)
[ "$patches" -eq 1 ] && ok "published with one update" || bad "$patches publishing updates"
run publish.log publish v0.6.0-alpha.1 && grep -q "already published; verifying only" "$work/publish.log" \
  && ok "re-running stage B changes nothing and verifies again" || bad "re-run: $(cat "$work/publish.log")"
patches=$(grep -c -- "-F draft=false" "$STUB_DIR/calls.log" || true)
[ "$patches" -eq 1 ] && ok "the re-run did not update the release" || bad "the re-run updated the release"

# Stable after the pre-release: latest, and the stable channel serves it.
env_for 0.6.0 "$sha_b"
draft v0.6.0 > /dev/null
if run publish.log publish v0.6.0; then
  [ "$(cat "$STUB_DIR/latest" 2> /dev/null)" = v0.6.0 ] && ok "the newest stable release becomes latest" || bad "latest: $(cat "$STUB_DIR/latest" 2> /dev/null)"
  grep -q -F "curl $base/latest/download/latest.json" "$STUB_DIR/calls.log" \
    && ok "the stable channel's URL announces it" || bad "stable URL not verified"
else
  bad "stable publication: $(cat "$work/publish.log")"
fi
# A newer stable exists: a fix of an older line is not latest.
fresh
env_for 0.7.0 "$sha_a"
draft v0.7.0 > /dev/null
run publish.log publish v0.7.0 || bad "0.7.0: $(cat "$work/publish.log")"
env_for 0.6.1 "$sha_b"
draft v0.6.1 > /dev/null
if run publish.log publish v0.6.1; then
  [ "$(cat "$STUB_DIR/latest")" = v0.7.0 ] && ok "an older stable release does not become latest" || bad "latest moved to $(cat "$STUB_DIR/latest")"
  [ "$(release v0.6.1 | jq -r .prerelease)" = false ] && ok "and is still a stable release" || bad "0.6.1 flags"
else
  bad "0.6.1: $(cat "$work/publish.log")"
fi

echo "== a draft that must not be published"
fresh
env_for 0.6.0-rc.1
id=$(draft v0.6.0-rc.1)
echo tampered >> "$STUB_DIR/assets/$id/DeckPilot_0.6.0-rc.1_amd64.deb"
expect_draft "a changed asset" "does not match SHA256SUMS" v0.6.0-rc.1
fresh
id=$(draft v0.6.0-rc.1)
echo extra > "$STUB_DIR/assets/$id/unexpected.bin"
expect_draft "an asset outside SHA256SUMS" "does not cover exactly the assets" v0.6.0-rc.1
fresh
id=$(draft v0.6.0-rc.1)
rm "$STUB_DIR/assets/$id/DeckPilot_0.6.0-rc.1_x64.dmg"
expect_draft "a missing artifact" "lacks: DeckPilot_0.6.0-rc.1_x64.dmg" v0.6.0-rc.1
fresh
id=$(draft v0.6.0-rc.1)
rm "$STUB_DIR/assets/$id/prerelease.json"
expect_draft "a missing manifest" "lacks: prerelease.json" v0.6.0-rc.1
fresh
id=$(draft v0.6.0-rc.1)
jq '.commit = "'"$sha_b"'"' "$STUB_DIR/assets/$id/release-meta.json" > "$work/m.json"
mv "$work/m.json" "$STUB_DIR/assets/$id/release-meta.json"
(cd "$STUB_DIR/assets/$id" && rm SHA256SUMS && sha256 -- * > ../S && mv ../S SHA256SUMS)
expect_draft "a signing record of another commit" "release-meta.json has commit" v0.6.0-rc.1
fresh
draft v0.6.0-rc.1 > /dev/null
VERIFY_RELEASE=false expect_draft "a failed updater signature" "updater signatures do not verify" v0.6.0-rc.1
fresh
draft v0.6.0-rc.1 > /dev/null
echo "$sha_b" > "$STUB_DIR/tags/v0.6.0-rc.1"
expect_draft "a tag moved after stage A" "now points to" v0.6.0-rc.1
fresh
id=$(draft v0.6.0-rc.1)
"$GH" api -X PATCH "repos/$REPOSITORY/releases/$id" -f body=" " > /dev/null
expect_draft "no release notes" "has no release notes" v0.6.0-rc.1
fresh
expect_failure "no release at all" "there is no release" publish v0.6.0-rc.1

echo "== recovery"
fresh
env_for 0.6.0-rc.2
id=$(draft v0.6.0-rc.2)
# Publication succeeds, but an asset is not downloadable afterwards (e.g.
# removed by hand): verification fails, the release stays published, and
# stage B is re-run once it is fixed.
mv "$STUB_DIR/assets/$id/DeckPilot_0.6.0-rc.2_x64.dmg" "$work/held.dmg"
"$GH" api -X PATCH "repos/$REPOSITORY/releases/$id" -F draft=false -F prerelease=true -f make_latest=false > /dev/null
expect_failure "verification after publication finds a missing download" "cannot be downloaded" publish v0.6.0-rc.2
mv "$work/held.dmg" "$STUB_DIR/assets/$id/DeckPilot_0.6.0-rc.2_x64.dmg"
run publish.log publish v0.6.0-rc.2 && ok "re-running stage B after the fix verifies it" || bad "recovery: $(cat "$work/publish.log")"
# Failed stage A: the draft is discarded and built again; published
# releases are never deleted.
env_for 0.6.0-rc.3
draft v0.6.0-rc.3 > /dev/null
run discard.log "$here/discard-draft.sh" v0.6.0-rc.3 && [ -z "$(release v0.6.0-rc.3)" ] \
  && ok "a failed draft is discarded" || bad "discard: $(cat "$work/discard.log")"
draft v0.6.0-rc.3 > /dev/null && ok "and built again" || bad "rebuilding the draft"
expect_failure "a published release is never discarded" "is published" "$here/discard-draft.sh" v0.6.0-rc.2
[ "$(release v0.6.0-rc.2 | jq -r .draft)" = false ] && ok "the published release is untouched" || bad "published release changed"

echo
echo "$passed passed, $failures failed (work dir: $work)"
[ "$failures" -eq 0 ]
