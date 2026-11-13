#!/usr/bin/env bash
# The protected environment that holds the signing secrets and gates
# publication is set up as required, or nothing that needs it may run.
#
#   tools/release-pipeline/check-environment.sh [<environment>]   (default: production-release)
#
# Requires, reading only:
#   - the environment exists;
#   - it has at least one required reviewer;
#   - its deployment policy allows tags matching v* and nothing else: custom
#     branch and tag policies (not "protected branches"), exactly one
#     policy, of type tag, named v*.
#
# GitHub creates a named environment, unprotected, the first time a job
# refers to it; the release workflow runs this check (environment-gate)
# before any such job, and again before publication (publish-gate), so an
# environment that is missing or unprotected stops the release before a
# secret is read.
#
# Environment: REPOSITORY, GH (see common.sh). Prints no secret or token.
set -euo pipefail
# shellcheck source=tools/release-pipeline/common.sh
source "$(dirname "$0")/common.sh"

name="${1:-production-release}"
[ -n "$repository" ] || fail "REPOSITORY (owner/name) is not set"
how="set it up as in docs/releasing.md (\"Publication gate and signing environment\"): required reviewers and the deployment tag rule v* only"
environment=$("$gh" api "repos/$repository/environments/$name" 2> /dev/null) \
  || fail "the environment '$name' does not exist or cannot be read; the release is blocked. To allow it, $how"
reviewers=$(jq '[.protection_rules[]? | select(.type == "required_reviewers") | .reviewers[]?] | length' <<< "$environment")
[ "$reviewers" -ge 1 ] \
  || fail "the environment '$name' requires no reviewer; the release is blocked. To allow it, $how"

custom=$(jq -r '.deployment_branch_policy.custom_branch_policies // false' <<< "$environment")
protected=$(jq -r '.deployment_branch_policy.protected_branches // false' <<< "$environment")
if [ "$custom" != true ] || [ "$protected" != false ]; then
  fail "the environment '$name' does not restrict deployments to selected tags; the release is blocked. To allow it, $how"
fi
policies=$("$gh" api "repos/$repository/environments/$name/deployment-branch-policies" \
  --jq '[.branch_policies[]? | "\(.type // "branch") \(.name)"] | sort | join(",")' 2> /dev/null) \
  || fail "the deployment policies of '$name' cannot be read; the release is blocked"
[ "$policies" = "tag v*" ] \
  || fail "the environment '$name' allows deployments from '${policies:-nothing}', not only tags v*; the release is blocked. To allow it, $how"
echo "environment '$name' requires approval by $reviewers reviewer(s) and deploys tags v* only"
