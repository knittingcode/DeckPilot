#!/usr/bin/env bash
# The protected environment that holds the signing secrets and gates
# publication is set up as required, or nothing that needs it may run.
#
#   tools/release-pipeline/check-environment.sh [<environment>]   (default: production-release)
#
# Requires, reading only:
#   - the environment exists and administrators cannot bypass it;
#   - its deployment policy allows tags matching v* and nothing else: custom
#     branch and tag policies (not "protected branches"), exactly one
#     policy, of type tag, named v*;
#   - an authorization for every deployment, one of:
#     * reviewers: at least one required reviewer approves the run;
#     * tag authority (no required reviewer): creating a v* tag is the
#       authorization, so an active tag ruleset on refs/tags/v* must
#       restrict creation, update and deletion, and this workflow's token
#       must not be able to bypass it. Who may bypass it (the release
#       identity) is visible to administrators only and is checked by
#       tools/release-pipeline/check-tag-authority.sh (docs/releasing.md,
#       "Release authorization").
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
[ "$(jq -r '.can_admins_bypass' <<< "$environment")" = false ] \
  || fail "administrators can bypass the protection of '$name'; the release is blocked. To allow it, turn off \"Allow administrators to bypass configured protection rules\""
reviewers=$(jq '[.protection_rules[]? | select(.type == "required_reviewers") | .reviewers[]?] | length' <<< "$environment")

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

if [ "$reviewers" -ge 1 ]; then
  echo "environment '$name' requires approval by $reviewers reviewer(s) and deploys tags v* only"
  exit 0
fi
# No reviewer: only a protected v* tag authorizes a deployment.
tag_rule="a v* tag is the only authorization only if an active tag ruleset on refs/tags/v* restricts creation, update and deletion without a bypass for this workflow"
ids=$("$gh" api "repos/$repository/rulesets?targets=tag&per_page=100" --jq '.[].id' 2> /dev/null) \
  || fail "the environment '$name' requires no reviewer and the tag rulesets cannot be read; the release is blocked ($tag_rule)"
for id in $ids; do
  ruleset=$("$gh" api "repos/$repository/rulesets/$id" 2> /dev/null) || continue
  if jq -e '
      .target == "tag" and .enforcement == "active"
      and ((.conditions.ref_name.include // []) | any(. == "refs/tags/v*" or . == "~ALL"))
      and ((.conditions.ref_name.exclude // []) | length == 0)
      and ([.rules[]?.type] | contains(["creation", "update", "deletion"]))
      and .current_user_can_bypass == "never"' <<< "$ruleset" > /dev/null; then
    echo "environment '$name' deploys tags v* only, without a reviewer: creating a v* tag is the authorization (tag ruleset $(jq -r .name <<< "$ruleset"))"
    exit 0
  fi
done
fail "the environment '$name' requires no reviewer, and no tag ruleset protects the creation of v* tags; the release is blocked ($tag_rule). To allow it, add a reviewer, or set up tag authority as in docs/releasing.md"
