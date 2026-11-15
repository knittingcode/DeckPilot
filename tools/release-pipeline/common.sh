#!/usr/bin/env bash
# Shared by the release pipeline scripts (sourced, not run).
#
# Environment:
#   REPOSITORY  owner/name (default $GITHUB_REPOSITORY)
#   GH          the GitHub CLI (default gh); the tests inject gh-stub.sh

TAG_PATTERN='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$'
repository="${REPOSITORY:-${GITHUB_REPOSITORY:-}}"
gh="${GH:-gh}"

fail() {
  echo "::error::$*" >&2
  exit 1
}

# require_tag <tag>: only release tags reach the API calls below.
require_tag() {
  [[ "$1" =~ $TAG_PATTERN ]] || fail "'$1' is not a release tag"
  [ -n "$repository" ] || fail "REPOSITORY (owner/name) is not set"
}

# find_release <tag>: the release of <tag> as JSON (drafts included, which
# needs a token with write access), or nothing. Fails if there are several.
find_release() {
  local found count
  found=$("$gh" api --paginate "repos/$repository/releases?per_page=100" \
    --jq ".[] | select(.tag_name == \"$1\")") || fail "the releases of $repository could not be read"
  count=$(printf '%s' "$found" | jq -s 'length')
  [ "$count" -le 1 ] || fail "$count releases exist for $1; discard the extra drafts first (docs/releasing.md)"
  printf '%s' "$found"
}

# published_versions: the versions of all published (not draft) releases
# with a release tag, one per line.
published_versions() {
  "$gh" api --paginate "repos/$repository/releases?per_page=100" \
    --jq '.[] | select(.draft | not) | .tag_name' \
    | grep -E "$TAG_PATTERN" | sed 's/^v//' || true
}

# Release separation (docs/marketplace/distribution.md): an application
# release carries the application only. Plugin packages and the plugin
# registry's metadata have their own repository, lifecycle and keys.

# updater_asset <platform key> <version>: the one release asset the update
# manifest may name for <platform key> (the names of
# tools/ci/updater-fragment.sh); fails for a key that is not an
# application platform.
updater_asset() {
  local v="$2"
  case "$1" in
    darwin-aarch64 | darwin-aarch64-app) echo "DeckPilot_aarch64.app.tar.gz" ;;
    darwin-x86_64 | darwin-x86_64-app) echo "DeckPilot_x64.app.tar.gz" ;;
    linux-x86_64 | linux-x86_64-appimage) echo "DeckPilot_${v}_amd64.AppImage" ;;
    linux-x86_64-deb) echo "DeckPilot_${v}_amd64.deb" ;;
    linux-x86_64-rpm) echo "DeckPilot-${v}-1.x86_64.rpm" ;;
    windows-x86_64 | windows-x86_64-msi) echo "DeckPilot_${v}_x64_en-US.msi" ;;
    windows-x86_64-nsis) echo "DeckPilot_${v}_x64-setup.exe" ;;
    *) return 1 ;;
  esac
}

# foreign_asset_kind <name>: "a plugin package" or "plugin registry
# metadata" for a file of the plugin registry's repository, nothing
# otherwise. It only names the reason: check-assets.sh refuses every asset
# outside the application allowlist anyway.
foreign_asset_kind() {
  case "$1" in
    *.dpplugin | *.dpplugin.*) echo "a plugin package" ;;
    *.minisig | roots.json | timestamp.json | keys-*.json | keyset*.json | index-*.json | registry* | site-v1*)
      echo "plugin registry metadata"
      ;;
  esac
}
