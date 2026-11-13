#!/usr/bin/env bash
# A stand-in for the GitHub CLI in the release pipeline tests: answers from
# files in $STUB_DIR instead of GitHub, and keeps releases there.
#
#   rules.json                 GET repos/<repo>/rules/branches/main (missing: 404)
#   check-runs-<sha>.json      GET repos/<repo>/commits/<sha>/check-runs (missing: none)
#   tags/<tag>                 the commit of a tag (GET repos/<repo>/commits/<tag>)
#   environments/<name>.json   GET repos/<repo>/environments/<name> (missing: 404)
#   environments/<name>-policies.json
#                              GET .../environments/<name>/deployment-branch-policies
#   releases/<id>.json         releases: GET (list, latest), POST, PATCH, DELETE
#   assets/<id>/<name>         their assets (`gh release download`)
#   latest                     the tag GitHub would call latest
#
# Every call is appended to $STUB_DIR/calls.log. Anything else fails, so a
# script that starts to call something new is noticed.
set -euo pipefail

dir="${STUB_DIR:?}"
mkdir -p "$dir/releases" "$dir/assets"
echo "$*" >> "$dir/calls.log"
unsupported() {
  echo "gh stub: unsupported: $*" >&2
  exit 2
}
not_found() {
  echo "gh: Not Found (HTTP 404)" >&2
  exit 1
}

# All releases, newest first, as GitHub lists them.
list() {
  local files=("$dir"/releases/*.json)
  if [ ! -e "${files[0]}" ]; then
    echo '[]'
    return
  fi
  local id release
  for release in "${files[@]}"; do
    id=$(jq -r .id "$release")
    jq --arg names "$(ls -1 "$dir/assets/$id" 2> /dev/null || true)" \
      '. + {assets: ($names | split("\n") | map(select(. != "") | {name: .}))}' "$release"
  done | jq -s 'sort_by(-.id)'
}

if [ "${1-}" = release ] && [ "${2-}" = download ]; then
  tag="$3" target=""
  shift 3
  while [ $# -gt 0 ]; do
    case "$1" in
      --dir) target="$2"; shift 2 ;;
      --repo) shift 2 ;;
      *) unsupported "release download $1" ;;
    esac
  done
  id=$(list | jq -r --arg t "$tag" '.[] | select(.tag_name == $t) | .id' | head -n 1)
  [ -n "$id" ] || not_found
  mkdir -p "$target"
  cp "$dir/assets/$id"/* "$target/" 2> /dev/null || true
  exit 0
fi

[ "${1-}" = api ] || unsupported "$*"
shift
method=GET path="" jq_filter="" fields="{}"
while [ $# -gt 0 ]; do
  case "$1" in
    --jq) jq_filter="$2"; shift 2 ;;
    --paginate) shift ;;
    -X) method="$2"; shift 2 ;;
    -f) fields=$(jq --arg k "${2%%=*}" --arg v "${2#*=}" '. + {($k): $v}' <<< "$fields"); shift 2 ;;
    -F)
      key="${2%%=*}" value="${2#*=}"
      case "$value" in
        @*) fields=$(jq --arg k "$key" --rawfile v "${value#@}" '. + {($k): $v}' <<< "$fields") ;;
        true | false | [0-9]*) fields=$(jq --arg k "$key" --argjson v "$value" '. + {($k): $v}' <<< "$fields") ;;
        *) fields=$(jq --arg k "$key" --arg v "$value" '. + {($k): $v}' <<< "$fields") ;;
      esac
      shift 2
      ;;
    -*) unsupported "option $1" ;;
    *) path="$1"; shift ;;
  esac
done

answer() {
  if [ -n "$jq_filter" ]; then jq -r "$jq_filter"; else cat; fi
}

case "$method $path" in
  "GET repos/"*/rules/branches/main)
    [ -f "$dir/rules.json" ] || not_found
    answer < "$dir/rules.json"
    ;;
  "GET repos/"*/commits/*/check-runs*)
    sha="${path#repos/*/commits/}"
    sha="${sha%%/*}"
    if [ -f "$dir/check-runs-$sha.json" ]; then
      answer < "$dir/check-runs-$sha.json"
    else
      echo '{"total_count": 0, "check_runs": []}' | answer
    fi
    ;;
  "GET repos/"*/commits/*)
    tag="${path##*/commits/}"
    [ -f "$dir/tags/$tag" ] || not_found
    jq -n --arg sha "$(cat "$dir/tags/$tag")" '{sha: $sha}' | answer
    ;;
  "GET repos/"*/environments/*/deployment-branch-policies)
    name="${path##*/environments/}"
    name="${name%/deployment-branch-policies}"
    [ -f "$dir/environments/$name-policies.json" ] || not_found
    answer < "$dir/environments/$name-policies.json"
    ;;
  "GET repos/"*/environments/*)
    name="${path##*/environments/}"
    [ -f "$dir/environments/$name.json" ] || not_found
    answer < "$dir/environments/$name.json"
    ;;
  "GET repos/"*/releases/latest)
    [ -f "$dir/latest" ] || not_found
    list | jq --arg t "$(cat "$dir/latest")" '.[] | select(.tag_name == $t)' | answer
    ;;
  "GET repos/"*/releases\?*)
    list | answer
    ;;
  "POST repos/"*/releases)
    id=$(($(cat "$dir/last-id" 2> /dev/null || echo 1000) + 1))
    echo "$id" > "$dir/last-id"
    jq --argjson id "$id" '{id: $id, draft: false, prerelease: false, body: ""} + . ' <<< "$fields" > "$dir/releases/$id.json"
    mkdir -p "$dir/assets/$id"
    answer < "$dir/releases/$id.json"
    ;;
  "PATCH repos/"*/releases/*)
    id="${path##*/}"
    [ -f "$dir/releases/$id.json" ] || not_found
    make_latest=$(jq -r '.make_latest // ""' <<< "$fields")
    jq --argjson f "$fields" '. + ($f | del(.make_latest))' "$dir/releases/$id.json" > "$dir/patched.json"
    mv "$dir/patched.json" "$dir/releases/$id.json"
    if [ "$make_latest" = true ]; then jq -r .tag_name "$dir/releases/$id.json" > "$dir/latest"; fi
    answer < "$dir/releases/$id.json"
    ;;
  "DELETE repos/"*/releases/*)
    id="${path##*/}"
    [ -f "$dir/releases/$id.json" ] || not_found
    rm -rf "$dir/releases/$id.json" "$dir/assets/$id"
    ;;
  *) unsupported "$method $path" ;;
esac
