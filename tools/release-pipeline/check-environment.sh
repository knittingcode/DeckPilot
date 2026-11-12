#!/usr/bin/env bash
# Stage B's precondition: the deployment environment that gates publication
# exists and requires a reviewer. Without it, publication fails closed —
# GitHub would otherwise create the environment, unprotected, the first
# time a job names it.
#
#   tools/release-pipeline/check-environment.sh [<environment>]   (default: release)
#
# Environment: REPOSITORY, GH (see common.sh). Reads only.
set -euo pipefail
# shellcheck source=tools/release-pipeline/common.sh
source "$(dirname "$0")/common.sh"

name="${1:-release}"
[ -n "$repository" ] || fail "REPOSITORY (owner/name) is not set"
how="create it in Settings > Environments with at least one required reviewer (docs/releasing.md, \"Publication gate\")"
environment=$("$gh" api "repos/$repository/environments/$name" 2> /dev/null) \
  || fail "the environment '$name' does not exist or cannot be read; publication is blocked. To allow it, $how"
reviewers=$(jq '[.protection_rules[]? | select(.type == "required_reviewers") | .reviewers[]?] | length' <<< "$environment")
[ "$reviewers" -ge 1 ] \
  || fail "the environment '$name' requires no reviewer; publication is blocked. To allow it, $how"
echo "environment '$name' requires approval by $reviewers reviewer(s)"
