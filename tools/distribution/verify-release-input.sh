#!/usr/bin/env bash
# The common verification step of the downstream distribution jobs
# (docs/distribution/README.md, section 6.3): checks a published release
# and writes <out dir>/input.json. See verify_release_input.py.
#
#   tools/distribution/verify-release-input.sh <tag> <channel> <out dir>
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
exec python3 -I "$here/verify_release_input.py" "$@"
