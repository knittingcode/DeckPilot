#!/usr/bin/env bash
# Tests of the release gate without GitHub and without real tags: a
# synthetic repository (tags on main and on a side branch, annotated and
# lightweight), the GitHub API answered from JSON fixtures by gh-stub.sh,
# and the structure of .github/workflows/release.yml.
#
#   tools/release-pipeline/test.sh [<work dir>]
#
# Needs git, jq, python3 with PyYAML and cargo (deckpilot-dev is built
# once). Nothing leaves <work dir> (default: a temporary directory).
# ok and bad never fail, so `check && ok || bad` is an if-then-else here.
# shellcheck disable=SC2015
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
here="$root/tools/release-pipeline"
work="${1:-$(mktemp -d "${TMPDIR:-/tmp}/release-pipeline-test.XXXXXX")}"
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

echo "== deckpilot-dev"
cargo build --manifest-path "$root/Cargo.toml" --locked -q -p deckpilot-dev
export DECKPILOT_DEV="$root/target/debug/deckpilot-dev"
export GH="$here/gh-stub.sh" STUB_DIR="$work/stub" REPOSITORY=example/DeckPilot
export MAIN_REF=refs/heads/main
mkdir -p "$STUB_DIR"
# Never the real repository's environment.
unset GITHUB_OUTPUT GITHUB_REPOSITORY GH_TOKEN GITHUB_TOKEN

# A synthetic repository whose sources declare one version.
repo="$work/repo"
rm -rf "$repo"
git init -q -b main "$repo"
git -C "$repo" config user.name "Release test"
git -C "$repo" config user.email "release-test@example.invalid"
git -C "$repo" config commit.gpgsign false
git -C "$repo" config tag.gpgsign false

# sources <version>: commit sources that declare <version>.
sources() {
  local v="$1"
  mkdir -p "$repo/apps/desktop/src-tauri"
  printf '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "%s"\n' "$v" > "$repo/Cargo.toml"
  printf '[[package]]\nname = "deckpilot-agent"\nversion = "%s"\n\n[[package]]\nname = "serde"\nversion = "1.0.0"\n' "$v" > "$repo/Cargo.lock"
  printf '{\n  "name": "@deckpilot/desktop",\n  "version": "%s"\n}\n' "$v" > "$repo/apps/desktop/package.json"
  printf '{\n  "productName": "DeckPilot",\n  "version": "%s"\n}\n' "$v" > "$repo/apps/desktop/src-tauri/tauri.conf.json"
  printf '# Changelog\n\n## [%s] - Unreleased\n\n- Test notes.\n' "${v%%-*}" > "$repo/CHANGELOG.md"
  git -C "$repo" add -A
  git -C "$repo" commit -q -m "version $v"
}

# A fixture of check runs for <sha>: every pinned required check, with the
# conclusion given (or success), as GitHub lists them.
# checks <sha> [<name>=<status>/<conclusion>[@<id>]...]
checks() {
  local sha="$1"
  shift
  local runs="[]" id=100 name
  while IFS= read -r name; do
    id=$((id + 1))
    runs=$(jq --arg name "$name" --arg sha "$sha" --argjson id "$id" \
      '. + [{id: $id, name: $name, head_sha: $sha, status: "completed", conclusion: "success"}]' <<< "$runs")
  done < <(grep -v -e '^#' -e '^[[:space:]]*$' "$here/required-checks.txt")
  local spec status conclusion
  for spec in "$@"; do
    name="${spec%%=*}"
    status="${spec#*=}"
    id=$((id + 1))
    case "$status" in *@*) id="${status#*@}" status="${status%@*}" ;; esac
    conclusion="${status#*/}"
    status="${status%%/*}"
    [ "$conclusion" = none ] && conclusion=null || conclusion="\"$conclusion\""
    runs=$(jq --arg name "$name" --arg sha "${CHECK_SHA:-$sha}" --argjson id "$id" --arg status "$status" \
      --argjson conclusion "$conclusion" \
      '. + [{id: $id, name: $name, head_sha: $sha, status: $status, conclusion: $conclusion}]' <<< "$runs")
  done
  jq -n --argjson runs "$runs" '{total_count: ($runs | length), check_runs: $runs}' > "$STUB_DIR/check-runs-$sha.json"
}

# gate <tag>: run the gate in the synthetic repository at <tag>.
gate() {
  local tag="$1"
  if git -C "$repo" rev-parse --verify --quiet "refs/tags/$tag^{commit}" > /dev/null; then
    git -C "$repo" checkout -q --detach "refs/tags/$tag^{commit}"
  fi
  (cd "$repo" && GITHUB_OUTPUT="$work/output" "$here/validate-tag.sh" "$tag") > "$work/gate.log" 2>&1
}

# accepts <what> <tag> <expected outputs...>
accepts() {
  local what="$1" tag="$2"
  shift 2
  : > "$work/output"
  if ! gate "$tag"; then
    bad "$what: refused: $(grep '::error::' "$work/gate.log" || tail -n 3 "$work/gate.log")"
    return
  fi
  local expected
  for expected in "$@"; do
    grep -q -x -F "$expected" "$work/output" || {
      bad "$what: output lacks $expected: $(tr '\n' ' ' < "$work/output")"
      return
    }
  done
  ok "$what"
}

# refuses <what> <tag> <expected message>
refuses() {
  : > "$work/output"
  if gate "$2"; then
    bad "$1: accepted"
  elif grep -q -F -- "$3" "$work/gate.log"; then
    if [ -s "$work/output" ]; then bad "$1: refused but wrote outputs"; else ok "$1"; fi
  else
    bad "$1: unexpected failure: $(cat "$work/gate.log")"
  fi
}

echo "== tags on main"
sources 0.6.0-alpha.1
git -C "$repo" tag -a v0.6.0-alpha.1 -m "DeckPilot 0.6.0-alpha.1"
alpha=$(git -C "$repo" rev-parse HEAD)
sources 0.6.0-beta.2
git -C "$repo" tag v0.6.0-beta.2 # lightweight
beta=$(git -C "$repo" rev-parse HEAD)
sources 0.6.0-rc.10
git -C "$repo" tag -a v0.6.0-rc.10 -m "DeckPilot 0.6.0-rc.10"
rc=$(git -C "$repo" rev-parse HEAD)
sources 0.6.0
git -C "$repo" tag -a v0.6.0 -m "DeckPilot 0.6.0"
stable=$(git -C "$repo" rev-parse HEAD)
git -C "$repo" tag -a v0.6.1 -m "version mismatch: the sources say 0.6.0"
for sha in "$alpha" "$beta" "$rc" "$stable"; do checks "$sha"; done

accepts "stable tag" v0.6.0 version=0.6.0 channel=stable prerelease=false "sha=$stable" msi_version=0.6.0
accepts "alpha tag (annotated)" v0.6.0-alpha.1 version=0.6.0-alpha.1 channel=prerelease prerelease=true "sha=$alpha"
accepts "beta tag (lightweight)" v0.6.0-beta.2 version=0.6.0-beta.2 channel=prerelease prerelease=true "sha=$beta"
accepts "rc tag" v0.6.0-rc.10 version=0.6.0-rc.10 channel=prerelease prerelease=true "sha=$rc" msi_version=0.6.0.40010
tag_object=$(git -C "$repo" rev-parse v0.6.0)
[ "$tag_object" != "$stable" ] && ok "v0.6.0 is an annotated tag object" || bad "v0.6.0 is not annotated"
accepts "annotated tag dereferenced to its commit" v0.6.0 "sha=$stable"

echo "== malformed tags"
for tag in testv0.6.0 v0.6.0-bettta.1 v0.6 v01.0.0 v1.0.0-beta.0 v1.0.0-preview.1 v1.0.0-extra \
  0.6.0 V0.6.0 v0.6.0.1 v0.06.0 v0.6.0- v0.6.0-alpha v0.6.0-alpha.01 v0.6.0-ALPHA.1 v0.6.0-alpha1 \
  v0.6.0-alpha.1.2 v0.6.0+build.1 v0.6.0-rc.1+build "v0.6.0 " " v0.6.0" $'v0.6.0\nv0.6.1' v0.5.0-1 v0.5.0-2 ""; do
  refuses "malformed tag '$(printf '%q' "$tag")'" "$tag" "is not a release tag"
done
# The Rust classification agrees.
for tag in testv0.6.0 v0.6.0-bettta.1 v1.0.0-beta.0 v0.5.0-1; do
  if "$DECKPILOT_DEV" release-tag "$tag" > /dev/null 2>&1; then
    bad "deckpilot-dev release-tag accepts $tag"
  else
    ok "deckpilot-dev release-tag refuses $tag"
  fi
done
"$DECKPILOT_DEV" release-tag v0.6.0-rc.2 | grep -q -x "channel=prerelease" \
  && ok "deckpilot-dev release-tag classifies rc as prerelease" \
  || bad "deckpilot-dev release-tag classification"

echo "== commits"
refuses "version mismatch" v0.6.1 "do not match v0.6.1"
git -C "$repo" checkout -q main
sources 256.0.0
git -C "$repo" tag v256.0.0
checks "$(git -C "$repo" rev-parse HEAD)"
refuses "a version the Windows installer cannot carry" v256.0.0 "cannot be released"
git -C "$repo" checkout -q -b side "$stable"
sources 0.7.0
git -C "$repo" tag -a v0.7.0 -m "not on main"
side=$(git -C "$repo" rev-parse HEAD)
checks "$side"
refuses "tag outside main" v0.7.0 "is not on the protected main branch"
git -C "$repo" checkout -q main
refuses "tag that does not exist" v9.9.9 "does not exist"
# The checkout is somewhere else than the tag.
git -C "$repo" checkout -q --detach "$alpha"
: > "$work/output"
if (cd "$repo" && GITHUB_OUTPUT="$work/output" "$here/validate-tag.sh" v0.6.0) > "$work/gate.log" 2>&1; then
  bad "checkout of another commit: accepted"
elif grep -q -F "not at the tagged commit" "$work/gate.log"; then
  ok "checkout of another commit"
else
  bad "checkout of another commit: $(cat "$work/gate.log")"
fi
# A modified checkout.
git -C "$repo" checkout -q --detach v0.6.0
echo "# local change" >> "$repo/Cargo.toml"
if (cd "$repo" && GITHUB_OUTPUT="$work/output" "$here/validate-tag.sh" v0.6.0) > "$work/gate.log" 2>&1; then
  bad "modified checkout: accepted"
elif grep -q -F "is modified" "$work/gate.log"; then
  ok "modified checkout"
else
  bad "modified checkout: $(cat "$work/gate.log")"
fi
git -C "$repo" checkout -q -- Cargo.toml
MAIN_REF=refs/remotes/origin/main refuses "main branch not fetched" v0.6.0 "is not available"

echo "== required checks of exactly the tagged commit"
checks "$stable" "Rust (macos-latest)=completed/failure"
refuses "a required check failed (latest run)" v0.6.0 "Rust (macos-latest): failure"
checks "$stable" "Frontend=completed/failure@1"
accepts "an earlier failure re-run successfully" v0.6.0 channel=stable
checks "$stable" "Frontend=in_progress/none"
refuses "a required check still running" v0.6.0 "Frontend: in_progress"
checks "$stable" "Storage with SQLCipher=completed/cancelled"
refuses "a required check cancelled" v0.6.0 "Storage with SQLCipher: cancelled"
checks "$stable" "Rust (windows-latest)=completed/skipped"
refuses "a required check skipped" v0.6.0 "Rust (windows-latest): skipped"
checks "$stable"
jq '.check_runs |= map(select(.name != "Updater signing keys match"))' "$STUB_DIR/check-runs-$stable.json" > "$work/c.json"
mv "$work/c.json" "$STUB_DIR/check-runs-$stable.json"
refuses "a required check missing" v0.6.0 "Updater signing keys match: no check run"
# A successful run that belongs to another commit does not count.
checks "$stable"
jq --arg other "$alpha" '.check_runs |= map(if .name == "Frontend" then .head_sha = $other else . end)' \
  "$STUB_DIR/check-runs-$stable.json" > "$work/c.json"
mv "$work/c.json" "$STUB_DIR/check-runs-$stable.json"
refuses "a check run of another commit" v0.6.0 "Frontend: no check run"
rm -f "$STUB_DIR/check-runs-$stable.json"
refuses "no check runs at all" v0.6.0 "no check run for $stable"
checks "$stable"
# The branch rules add a required check that did not run.
jq -n '[{type: "deletion"}, {type: "required_status_checks", parameters: {required_status_checks: [{context: "Frontend"}, {context: "New required check"}]}}]' \
  > "$STUB_DIR/rules.json"
refuses "a check required by the branch rules" v0.6.0 "New required check: no check run"
checks "$stable" "New required check=completed/success"
accepts "the branch rules' checks passed" v0.6.0 channel=stable
grep -q "rules/branches/main" "$STUB_DIR/calls.log" && ok "the branch rules were read" || bad "the branch rules were not read"
rm -f "$STUB_DIR/rules.json"
checks "$stable"
accepts "unreadable branch rules: the pinned list applies" v0.6.0 channel=stable
grep -q "pinned required checks" "$work/gate.log" && ok "unreadable branch rules are reported" || bad "no warning for unreadable branch rules"

echo "== release workflow structure"
if python3 "$here/check-workflow-gate.py" "$root/.github/workflows/release.yml" > "$work/structure.log" 2>&1; then
  ok "every job of release.yml depends on validate-tag"
else
  bad "release.yml: $(cat "$work/structure.log")"
fi
# The structural check notices a job without the gate.
cp "$root/.github/workflows/release.yml" "$work/ungated.yml"
printf '  sneaky:\n    runs-on: ubuntu-latest\n    if: needs.validate-tag.result == '"'success'"'\n    steps:\n      - run: echo\n' >> "$work/ungated.yml"
python3 "$here/check-workflow-gate.py" "$work/ungated.yml" > "$work/structure.log" 2>&1 \
  && bad "a job without needs: validate-tag passes the structural check" \
  || { grep -q "job sneaky does not depend on validate-tag" "$work/structure.log" && ok "a job without the gate is caught" || bad "structure: $(cat "$work/structure.log")"; }
python3 - "$root/.github/workflows/release.yml" "$work/writing-gate.yml" << 'EOF'
import sys, yaml
workflow = yaml.safe_load(open(sys.argv[1]))
workflow["jobs"]["validate-tag"]["permissions"]["contents"] = "write"
workflow["permissions"] = {"contents": "write"}
yaml.safe_dump(workflow, open(sys.argv[2], "w"))
EOF
python3 "$here/check-workflow-gate.py" "$work/writing-gate.yml" > "$work/structure.log" 2>&1 \
  && bad "write permissions pass the structural check" \
  || { grep -q "validate-tag may not have contents: write" "$work/structure.log" \
    && grep -q "workflow-level permission contents: write" "$work/structure.log" \
    && ok "write permissions of the gate or the workflow are caught" || bad "structure: $(cat "$work/structure.log")"; }

echo
echo "$passed passed, $failures failed (work dir: $work)"
[ "$failures" -eq 0 ]
