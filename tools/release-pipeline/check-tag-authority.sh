#!/usr/bin/env bash
# Is creating a v* tag a safe single authorization for a release? Run by the
# owner, read only, with an administrator's token (only administrators see
# who may bypass a ruleset):
#
#   tools/release-pipeline/check-tag-authority.sh <release identity>
#
#   <release identity>  who alone may create v* tags:
#                       admin-role        the repository administrator role
#                                         (DeckPilot's model; see
#                                         docs/releasing.md, "Release
#                                         authorization")
#                       deploy-key:<id>   a deploy key (the stricter
#                                         alternative)
#
# Passes only if:
#   - an active tag ruleset on refs/tags/v* (no exclusions) restricts
#     creation, update and deletion;
#   - its only bypass actor is the release identity, and nothing else may
#     bypass it;
#   - a deploy key named as the release identity exists and can write;
#   - production-release deploys tags v* only and administrators cannot
#     bypass it.
# Then the required reviewers of production-release can be removed; the
# release workflow's gate (check-environment.sh) then requires the same tag
# ruleset on every run. Also lists everyone who can push to the
# repository: anyone with write access can still run workflows and push
# branches, but no longer create release tags.
#
# Environment: REPOSITORY (default: the origin remote), GH (default: gh).
# Prints no secret or token.
set -euo pipefail
# shellcheck source=tools/release-pipeline/common.sh
source "$(dirname "$0")/common.sh"

identity="${1-}"
case "$identity" in
  deploy-key:[0-9]*) expected=$(jq -n --argjson id "${identity#deploy-key:}" '[{actor_type: "DeployKey", actor_id: $id}]') ;;
  admin-role) expected='[{"actor_type": "RepositoryRole", "actor_id": 5}]' ;;
  *) fail "usage: check-tag-authority.sh deploy-key:<id> | admin-role" ;;
esac
if [ -z "$repository" ]; then
  repository=$(git remote get-url origin | sed -E 's#(git@github.com:|https://github.com/)##; s#\.git$##')
fi
problems=0
problem() {
  echo "::error::$*"
  problems=$((problems + 1))
}

echo "== tag rulesets of $repository"
ids=$("$gh" api "repos/$repository/rulesets?targets=tag&per_page=100" --jq '.[].id') \
  || fail "the rulesets of $repository cannot be read"
authority=""
for id in $ids; do
  ruleset=$("$gh" api "repos/$repository/rulesets/$id")
  jq -e 'has("bypass_actors")' <<< "$ruleset" > /dev/null \
    || fail "this token cannot see who may bypass rulesets; use an administrator's token"
  echo "ruleset $id '$(jq -r .name <<< "$ruleset")': $(jq -r '.enforcement' <<< "$ruleset"), refs $(jq -c '.conditions.ref_name' <<< "$ruleset"), rules $(jq -c '[.rules[].type]' <<< "$ruleset"), bypass $(jq -c '[.bypass_actors[] | {actor_type, actor_id, bypass_mode}]' <<< "$ruleset")"
  if jq -e '.target == "tag" and .enforcement == "active"
      and ((.conditions.ref_name.include // []) | any(. == "refs/tags/v*" or . == "~ALL"))
      and ((.conditions.ref_name.exclude // []) | length == 0)
      and ([.rules[]?.type] | contains(["creation", "update", "deletion"]))' <<< "$ruleset" > /dev/null; then
    authority="$ruleset"
  fi
done
if [ -z "$authority" ]; then
  problem "no active tag ruleset on refs/tags/v* restricts creation, update and deletion"
else
  actors=$(jq -c '[.bypass_actors[] | {actor_type, actor_id: (.actor_id // null)}] | sort' <<< "$authority")
  [ "$actors" = "$(jq -c 'sort' <<< "$expected")" ] \
    || problem "the tag ruleset may be bypassed by $actors, not only by $identity"
  jq -e '[.bypass_actors[] | select(.bypass_mode != "always")] | length == 0' <<< "$authority" > /dev/null \
    || problem "the release identity's bypass mode is not 'always'"
  own=$(jq -r .current_user_can_bypass <<< "$authority")
  echo "this token (the account running the check) can bypass it: $own"
  if [ "${identity%%:*}" = deploy-key ] && [ "$own" != never ]; then
    problem "the account running this check can still create v* tags ($own)"
  fi
fi

echo "== the release identity"
if [ "${identity%%:*}" = deploy-key ]; then
  key=$("$gh" api "repos/$repository/keys/${identity#deploy-key:}" 2> /dev/null) \
    || problem "deploy key ${identity#deploy-key:} does not exist"
  if [ -n "${key:-}" ]; then
    echo "deploy key ${identity#deploy-key:} '$(jq -r .title <<< "$key")', read_only=$(jq -r .read_only <<< "$key")"
    [ "$(jq -r .read_only <<< "$key")" = false ] || problem "the deploy key is read-only; it cannot push tags"
  fi
else
  echo "the administrator role: anyone acting with an administrator's account (including automation that uses it) can create release tags"
fi

echo "== who can push"
"$gh" api "repos/$repository/collaborators?per_page=100" --jq '.[] | "collaborator \(.login): \(.role_name)"'
"$gh" api "repos/$repository/keys?per_page=100" --jq '.[] | "deploy key \(.id) \(.title): \(if .read_only then "read-only" else "write" end)"'

echo "== production-release"
environment=$("$gh" api "repos/$repository/environments/production-release") \
  || fail "the environment production-release cannot be read"
[ "$(jq -r .can_admins_bypass <<< "$environment")" = false ] || problem "administrators can bypass production-release"
policies=$("$gh" api "repos/$repository/environments/production-release/deployment-branch-policies" \
  --jq '[.branch_policies[] | "\(.type) \(.name)"] | sort | join(",")')
[ "$policies" = "tag v*" ] || problem "production-release deploys '$policies', not only tags v*"
echo "required reviewers: $(jq -c '[.protection_rules[] | select(.type == "required_reviewers") | .reviewers[].reviewer.login]' <<< "$environment")"

if [ "$problems" -gt 0 ]; then
  echo "$problems problem(s): keep the required reviewers of production-release"
  exit 1
fi
echo "only $identity can create v* tags: the required reviewers of production-release may be removed (docs/releasing.md, \"Release authorization\")"
