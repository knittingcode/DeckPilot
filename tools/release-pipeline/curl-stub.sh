#!/usr/bin/env bash
# A stand-in for curl in the release pipeline tests: serves the public
# release URLs from the releases that gh-stub.sh keeps in $STUB_DIR, as
# GitHub would serve them to users (published releases only).
#
#   curl-stub.sh [options] -o <file> <url>
set -euo pipefail

dir="${STUB_DIR:?}"
out="" url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    --retry | --retry-delay) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
echo "curl $url" >> "$dir/calls.log"
missing() {
  echo "curl: (22) The requested URL returned error: 404" >&2
  exit 22
}

prefix="https://github.com/${REPOSITORY:?}/releases/"
rest="${url#"$prefix"}"
[ "$rest" != "$url" ] || missing
case "$rest" in
  latest/download/*) tag=$(cat "$dir/latest" 2> /dev/null) || missing; name="${rest#latest/download/}" ;;
  download/*/*) rest="${rest#download/}"; tag="${rest%%/*}"; name="${rest#*/}" ;;
  *) missing ;;
esac
id=""
for release in "$dir"/releases/*.json; do
  [ -e "$release" ] || continue
  if [ "$(jq -r .tag_name "$release")" = "$tag" ] && [ "$(jq -r .draft "$release")" = false ]; then
    id=$(jq -r .id "$release")
  fi
done
[ -n "$id" ] && [ -f "$dir/assets/$id/$name" ] || missing
cp "$dir/assets/$id/$name" "$out"
