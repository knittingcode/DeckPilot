#!/usr/bin/env bash
# Decides which optional (not required) CI jobs a pull request needs, from
# the files it changes. Writes `<group>=true|false` to GITHUB_OUTPUT (or
# stdout) for each group below.
#
#   tools/ci/changed-paths.sh [<changed files list>]
#
# Without a list, the changed files are those between the pull request's
# merge commit and its base (HEAD^1..HEAD, checkout with fetch-depth 2).
# For a merge queue group (EVENT_NAME=merge_group) they are those between
# the group's base commit (BASE_SHA, fetched here) and its head (HEAD),
# i.e. every pull request in the group. Any other event (e.g. a push to
# main), or a diff that cannot be computed, turns every group on: when in
# doubt, run.
set -euo pipefail

groups="packaging keystore release_manifest"
# The paths of a group: an extended regular expression on whole paths.
pattern() {
  case "$1" in
    # Package checks: files that change what is bundled or how. Plain
    # source changes are covered by the Rust jobs here and by the Packaging
    # runs on main and nightly.
    packaging) echo '^apps/desktop/src-tauri/(tauri[^/]*\.json$|build\.rs$|Cargo\.toml$|capabilities/|icons/|app-icon\.svg$)|^apps/agent/(build\.rs|Cargo\.toml|Info\.plist)$|^packaging/|^tools/ci/|^\.github/workflows/|^Cargo\.(lock|toml)$|^apps/desktop/package\.json$' ;;
    # Real credential stores.
    keystore) echo '^crates/deckpilot-keystore/|^tools/ci/with-keyring\.sh$|^\.github/workflows/ci\.yml$' ;;
    # Assembly of the release's latest.json.
    release_manifest) echo '^tools/ci/|^\.github/workflows/(release|ci)\.yml$' ;;
  esac
}

out="${GITHUB_OUTPUT:-/dev/stdout}"
all() {
  echo "$1; running every optional job" >&2
  for group in $groups; do echo "$group=true" >> "$out"; done
  exit 0
}

if [ "$#" -gt 0 ]; then
  changed=$(cat "$1")
elif [ "${EVENT_NAME:-pull_request}" = merge_group ]; then
  base="${BASE_SHA:-}"
  if [ -z "$base" ] || ! git fetch --quiet --no-tags --depth=1 origin "$base" 2> /dev/null \
    || ! changed=$(git diff --name-only "$base" HEAD 2> /dev/null); then
    all "the changed files of the merge group cannot be determined"
  fi
elif [ "${EVENT_NAME:-pull_request}" != pull_request ]; then
  all "event ${EVENT_NAME}"
elif ! changed=$(git diff --name-only HEAD^1 HEAD 2>/dev/null); then
  all "the changed files cannot be determined"
fi

echo "changed files:" >&2
while IFS= read -r file; do echo "  $file"; done <<< "$changed" >&2
for group in $groups; do
  if echo "$changed" | grep -q -E "$(pattern "$group")"; then
    echo "$group=true" >> "$out"
  else
    echo "$group=false" >> "$out"
  fi
done
