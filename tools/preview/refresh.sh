#!/usr/bin/env bash
# Refreshes the integration preview to the latest main and open PR heads.
#
#   tools/preview/refresh.sh [--tests]
#
# 1. Refuses while a workflow runs in the preview.
# 2. Fetches; builds preview/next from origin/main, merging the pull requests
#    of tools/preview/prs.txt (plus newly opened ones) with plain merges. A
#    pull request that is no longer open is dropped (merged ones are in main);
#    one that conflicts is excluded with the files named, never half-merged.
# 3. Re-applies the preview's own tooling commits ("preview(tooling): …").
# 4. Runs cargo fmt, clippy and the frontend type check (--tests: also the
#    Rust and frontend tests).
# 5. Backs up the preview data directory (the encrypted database as is),
#    stops only the preview session, moves preview/integration to the new
#    state (the old one stays as preview/previous) and starts it again.
# Nothing is pushed.
. "$(dirname "$0")/lib.sh"
TESTS=false; [ "${1:-}" = --tests ] && TESTS=true

if [ -s "$DATA/agent.token" ]; then
  running=$("$DEV" validation running "$DATA" 2>/dev/null || echo 0)
  if [ "${running:-0}" != 0 ]; then
    echo "a workflow is running in the preview ($running); try again when it ends" >&2; exit 1
  fi
fi
[ -z "$(git status --porcelain)" ] || { echo "uncommitted changes in the preview worktree" >&2; exit 1; }

git fetch -q --prune origin
open=$(gh pr list --state open --json number,headRefName,isDraft --jq '.[] | select(.isDraft|not) | "\(.number) \(.headRefName)"')
open_all=$(gh pr list --state open --json number,headRefName --jq '.[] | "\(.number) \(.headRefName)"')
tooling=$(git log --reverse --format=%H --grep='^preview(tooling):' origin/main..HEAD)
list=tools/preview/prs.txt
cp "$list" "$ROOT/.dev/prs.txt.new"
new="$ROOT/.dev/prs.txt.new"

# Includes that are still open, then open pull requests not mentioned yet.
plan=$(awk '$1=="include"{print $2, $3}' "$new" | while read -r n b; do
  grep -q "^$n " <<<"$open" && echo "$n $b"; done)
while read -r n b; do
  [ -z "$n" ] && continue
  grep -Eq "^(include|exclude) $n( |$)" "$new" || plan=$(printf '%s\n%s %s' "$plan" "$n" "$b")
done <<<"$open"

git switch -q -C preview/next origin/main
base=$(git rev-parse --short HEAD)
{ grep '^#' "$list"; } > "$new"
while read -r n b; do
  [ -z "$n" ] && continue
  if git merge -q --no-ff --no-edit -m "preview: merge #$n ($b)" "origin/$b" >/dev/null 2>&1; then
    echo "include $n $b" >> "$new"; echo "merged #$n"
  else
    files=$(git diff --name-only --diff-filter=U | tr '\n' ' ')
    git merge --abort
    echo "exclude $n conflict with main or earlier PRs in: $files" >> "$new"; echo "excluded #$n (conflict: $files)"
  fi
done <<<"$plan"
awk '$1=="exclude"' "$list" | while read -r _ n _rest; do
  grep -q "^$n " <<<"$open_all" && ! grep -Eq "^(include|exclude) $n " "$new" && grep "^exclude $n " "$list" >> "$new"
done
for c in $tooling; do git cherry-pick -q "$c"; done
cp "$new" "$list"
git add "$list" && git commit -q -m "preview(tooling): pull request list for $base" || true

cargo fmt --all --check
cargo clippy -q --workspace --all-targets --locked -- -D warnings
(cd apps/desktop && pnpm -s typecheck)
if $TESTS; then cargo test -q --workspace --locked; (cd apps/desktop && pnpm -s test); fi

if [ -d "$DATA" ]; then
  mkdir -p "$ROOT/.dev/backups"
  cp -Rp "$DATA" "$ROOT/.dev/backups/data-$(date +%Y%m%d-%H%M%S)"
fi
dev down || true
git branch -f preview/previous preview/integration
git switch -q preview/integration
git reset -q --hard preview/next
git branch -q -D preview/next
write_info "$base"
"$ROOT/tools/preview/start.sh"
