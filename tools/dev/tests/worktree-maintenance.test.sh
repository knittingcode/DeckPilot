#!/usr/bin/env bash
# Tests tools/dev/worktree-maintenance.sh on a throwaway repository with a
# bare "origin" and one worktree per case. Nothing outside a temporary
# directory is touched. Runs with bash 3.2 and newer.
#
#   tools/dev/tests/worktree-maintenance.test.sh
set -euo pipefail
export LC_ALL=C

script="$(cd "$(dirname "$0")/.." && pwd)/worktree-maintenance.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/wtm-test.XXXXXX")
work=$(cd "$work" && pwd -P)
sleeper=""
cleanup_test() {
  if [ -n "$sleeper" ]; then kill "$sleeper" 2> /dev/null || true; wait "$sleeper" 2> /dev/null || true; fi
  rm -rf "$work"
}
trap cleanup_test EXIT

# Isolated from the user's git configuration (signing, hooks, templates).
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid

failures=0
ok() { echo "ok   $*"; }
fail() { echo "FAIL $*"; failures=$((failures + 1)); }
check() { # <description> <command...>
  local d="$1"; shift
  if "$@"; then ok "$d"; else fail "$d"; fi
}

g() { git -C "$repo" "$@"; }
commit_file() { # <dir> <file> <content>
  echo "$3" > "$1/$2"
  git -C "$1" add "$2"
  git -C "$1" commit -q -m "change $2"
}
has_ref() { g rev-parse -q --verify "$1" > /dev/null; }

# --- Fixture ---------------------------------------------------------------
origin="$work/origin.git"
repo="$work/repo"
git init -q --bare -b main "$origin"
git init -q -b main "$repo"
printf 'target/\nnode_modules/\ncache/\nnest/\n' > "$repo/.gitignore"
mkdir -p "$repo/keep/target"
echo tracked > "$repo/keep/target/file"
git -C "$repo" add .gitignore && git -C "$repo" add -f keep/target/file
git -C "$repo" commit -q -m initial
commit_file "$repo" README "hello"
g remote add origin "$origin"
g push -q origin main
g remote set-head origin main 2> /dev/null || g symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/main

add_wt() { # <name> <branch>
  g worktree add -q -b "$2" "$work/$1" main
}

# clean, pushed (merged) branch with build output: SAFE
add_wt wt-merged feat/merged
commit_file "$work/wt-merged" merged.txt merged
git -C "$work/wt-merged" push -q origin feat/merged
mkdir -p "$work/wt-merged/target/debug" && echo bin > "$work/wt-merged/target/debug/app"

# commit patch-equivalent to one on main (cherry-picked), never pushed: SAFE
add_wt wt-equiv feat/equiv
commit_file "$work/wt-equiv" equiv.txt equiv
g cherry-pick "$(git -C "$work/wt-equiv" rev-parse HEAD)" > /dev/null
g push -q origin main

# uncommitted change, plus build output and a non-allowlisted ignored dir:
# REQUIRED, but inactive
add_wt wt-dirty feat/dirty
echo changed >> "$work/wt-dirty/README"
mkdir -p "$work/wt-dirty/target/debug" "$work/wt-dirty/node_modules/pkg" "$work/wt-dirty/cache"
echo bin > "$work/wt-dirty/target/debug/app"
echo js > "$work/wt-dirty/node_modules/pkg/index.js"
echo keep > "$work/wt-dirty/cache/data"

# untracked, not ignored file: REQUIRED
add_wt wt-untracked feat/untracked
echo new > "$work/wt-untracked/notes.txt"

# local commit on no remote: REQUIRED
add_wt wt-unique feat/unique
commit_file "$work/wt-unique" unique.txt unique

# commit only on GitHub's pull request ref (squash-merged PR, branch
# deleted): REQUIRED until --fetch-pr-heads, then SAFE
add_wt wt-prref feat/prref
commit_file "$work/wt-prref" prref.txt prref
git -C "$work/wt-prref" push -q origin HEAD:refs/pull/7/head

# pushed branch with an open pull request (fake gh): REQUIRED
add_wt wt-openpr feat/openpr
commit_file "$work/wt-openpr" openpr.txt openpr
git -C "$work/wt-openpr" push -q origin feat/openpr

# locked by a live process: ACTIVE (with build output that must survive)
add_wt wt-locked feat/locked
git -C "$work/wt-locked" push -q origin feat/locked
mkdir -p "$work/wt-locked/target" && echo bin > "$work/wt-locked/target/app"
g worktree lock --reason "claude agent wt-locked (pid $$ start now)" "$work/wt-locked"

# locked by a process that is gone: UNCERTAIN
add_wt wt-stale feat/stale
git -C "$work/wt-stale" push -q origin feat/stale
sleep 0 &
dead_pid=$!
wait "$dead_pid" 2> /dev/null || true
g worktree lock --reason "claude agent wt-stale (pid $dead_pid start then)" "$work/wt-stale"

# a process with its working directory inside: ACTIVE
add_wt wt-proc feat/proc
git -C "$work/wt-proc" push -q origin feat/proc
(cd "$work/wt-proc" && exec sleep 600) &
sleeper=$!

# a clean worktree holding another worktree (in an ignored directory, like
# .claude/worktrees): REQUIRED, removing it would delete the inner one
add_wt wt-outer feat/outer
git -C "$work/wt-outer" push -q origin feat/outer
g worktree add -q -b feat/inner "$work/wt-outer/nest/inner" main
echo changed >> "$work/wt-outer/nest/inner/README"

# the main checkout has build output too (never deleted)
mkdir -p "$repo/target" && echo bin > "$repo/target/app"

# Fake gh: lists the open pull request of feat/openpr.
fakebin="$work/bin"
mkdir -p "$fakebin"
printf '42\tfeat/openpr\t%s\n' "$(git -C "$work/wt-openpr" rev-parse HEAD)" > "$work/prs"
cat > "$fakebin/gh" << EOF
#!/bin/sh
cat "$work/prs"
EOF
chmod +x "$fakebin/gh"
export WORKTREE_MAINTENANCE_GH="$fakebin/gh"
log="$work/maintenance.log"

wtm() { "$BASH" "$script" "$@" --repo "$repo" --log "$log"; }
cd "$work"

# Wait until the process check sees the sleeper (its cwd is set after fork).
for _ in 1 2 3 4 5 6 7 8 9 10; do
  wtm audit --format tsv --no-size | grep -q "^ACTIVE	[^	]*	$work/wt-proc	" && break
  sleep 0.5
done

class_of() { # <tsv file> <path>
  awk -F '\t' -v p="$2" '$3 == p { print $1 }' "$1"
}
expect_class() { # <tsv file> <path> <class>
  local got
  got=$(class_of "$1" "$2")
  if [ "$got" = "$3" ]; then ok "$(basename "$2") is $3"; else fail "$(basename "$2") is '$got', expected $3"; fi
}

# --- Classification --------------------------------------------------------
wtm audit --format tsv > "$work/a1.tsv"
expect_class "$work/a1.tsv" "$repo" REQUIRED
expect_class "$work/a1.tsv" "$work/wt-merged" SAFE
expect_class "$work/a1.tsv" "$work/wt-equiv" SAFE
expect_class "$work/a1.tsv" "$work/wt-dirty" REQUIRED
expect_class "$work/a1.tsv" "$work/wt-untracked" REQUIRED
expect_class "$work/a1.tsv" "$work/wt-unique" REQUIRED
expect_class "$work/a1.tsv" "$work/wt-prref" REQUIRED
expect_class "$work/a1.tsv" "$work/wt-openpr" REQUIRED
expect_class "$work/a1.tsv" "$work/wt-locked" ACTIVE
expect_class "$work/a1.tsv" "$work/wt-stale" UNCERTAIN
expect_class "$work/a1.tsv" "$work/wt-proc" ACTIVE
expect_class "$work/a1.tsv" "$work/wt-outer" REQUIRED
expect_class "$work/a1.tsv" "$work/wt-outer/nest/inner" REQUIRED
check "a worktree holding another one says so" sh -c "grep '	$work/wt-outer	' '$work/a1.tsv' | grep -q contains-worktrees"
check "the open pull request is reported" grep -q "	#42	" "$work/a1.tsv"
check "the main checkout is marked" grep -q "main-checkout" "$work/a1.tsv"
check "the sleeper is listed as a process" grep -q "sleep(" "$work/a1.tsv"
check "build dirs are measured" grep -q "target=" "$work/a1.tsv"
check "the table output works" wtm audit --no-size > "$work/table.txt"
check "the table names every worktree" test "$(grep -c -E '^(ACTIVE|REQUIRED|UNCERTAIN|SAFE) ' "$work/table.txt")" = 13

# Without gh, nothing is SAFE: the pull request state is unknown.
WORKTREE_MAINTENANCE_GH="$work/no-such-gh" wtm audit --format tsv --no-size > "$work/nogh.tsv"
expect_class "$work/nogh.tsv" "$work/wt-merged" UNCERTAIN
expect_class "$work/nogh.tsv" "$work/wt-dirty" REQUIRED

# The PR ref only counts after --fetch-pr-heads, which only adds origin/pr/*.
refs_before=$(g for-each-ref --format='%(refname)' | grep -v '^refs/remotes/origin/pr/' | sort)
wtm audit --format tsv --no-size --fetch-pr-heads > "$work/a2.tsv" 2> /dev/null
refs_after=$(g for-each-ref --format='%(refname)' | grep -v '^refs/remotes/origin/pr/' | sort)
expect_class "$work/a2.tsv" "$work/wt-prref" SAFE
check "--fetch-pr-heads adds refs/remotes/origin/pr/7" has_ref refs/remotes/origin/pr/7
check "--fetch-pr-heads changes no other ref" test "$refs_before" = "$refs_after"

# --- report, disk-usage ------------------------------------------------------
wtm report > "$work/report.txt"
check "report counts classes" grep -q -E '^  SAFE +3 ' "$work/report.txt"
check "report shows reclaimable space" grep -q "Reclaimable" "$work/report.txt"
wtm disk-usage > "$work/du.txt"
check "disk-usage lists build dirs" grep -q "    target" "$work/du.txt"

# --- cleanup -----------------------------------------------------------------
snapshot() { (cd "$work" && find . -path ./origin.git -prune -o -print | sort) | grep -v -E '^\./(a[0-9]|nogh|report|du|table|out|maintenance)' ; }
before=$(snapshot)
wtm cleanup > "$work/out-dry.txt"
check "dry run is the default and deletes nothing" test "$before" = "$(snapshot)"
check "dry run lists the SAFE worktrees" grep -q "would remove  $work/wt-merged" "$work/out-dry.txt"
check "dry run writes no log" test ! -e "$log"

status=0
wtm cleanup --apply > "$work/out-refused.txt" 2>&1 || status=$?
check "--apply is refused while worktrees are ACTIVE (exit 3)" test "$status" = 3
check "the refused run deletes nothing" test "$before" = "$(snapshot)"

status=0
wtm cleanup --apply --allow-active-elsewhere > "$work/out-apply.txt" 2>&1 || status=$?
check "cleanup --apply succeeds" test "$status" = 0
for gone in wt-merged wt-equiv wt-prref; do
  check "$gone was removed" test ! -e "$work/$gone"
done
for kept in wt-dirty wt-untracked wt-unique wt-openpr wt-locked wt-stale wt-proc wt-outer wt-outer/nest/inner; do
  check "$kept was kept" test -d "$work/$kept"
done
check "the main checkout was kept" test -f "$repo/README"
check "branches are never deleted" has_ref refs/heads/feat/merged
check "build dirs are kept without --artifacts" test -d "$work/wt-dirty/target"
check "the log has one entry per removal" test "$(grep -c ' remove-worktree ' "$log")" = 3
merged_head=$(g rev-parse feat/merged)
check "the log names path, branch, head and size" \
  grep -q -E " remove-worktree path=$work/wt-merged branch=feat/merged head=$merged_head size_kb=[0-9]+$" "$log"
check "the locked worktree is still locked" grep -q "claude agent wt-locked" "$repo/.git/worktrees/wt-locked/locked"

status=0
wtm cleanup --apply --allow-active-elsewhere > "$work/out-again.txt" 2>&1 || status=$?
check "a second run succeeds" test "$status" = 0
check "a second run removes nothing" grep -q "Removed 0 worktree" "$work/out-again.txt"
check "a second run logs nothing" test "$(wc -l < "$log" | tr -d ' ')" = 3

# --- cleanup --artifacts ------------------------------------------------------
wtm cleanup --artifacts > "$work/out-art-dry.txt"
check "artifacts dry run deletes nothing" test -d "$work/wt-dirty/target"
check "artifacts dry run lists target" grep -q "would delete build dir  $work/wt-dirty/target" "$work/out-art-dry.txt"
wtm cleanup --apply --allow-active-elsewhere --artifacts > "$work/out-art.txt" 2>&1
check "target of an inactive REQUIRED worktree is deleted" test ! -e "$work/wt-dirty/target"
check "node_modules of an inactive REQUIRED worktree is deleted" test ! -e "$work/wt-dirty/node_modules"
check "an ignored dir off the allowlist is kept" test -f "$work/wt-dirty/cache/data"
check "the uncommitted change is kept" grep -q changed "$work/wt-dirty/README"
check "a tracked dir named target is kept" test -f "$work/wt-dirty/keep/target/file"
check "target of a locked worktree is kept" test -f "$work/wt-locked/target/app"
check "target of the main checkout is kept" test -f "$repo/target/app"
check "artifact deletions are logged" grep -q " remove-artifact path=$work/wt-dirty/target " "$log"

# --- --unlock-finished --------------------------------------------------------
wtm cleanup --unlock-finished wt-stale > "$work/out-unlock-dry.txt"
check "--unlock-finished dry run keeps the lock" test -e "$repo/.git/worktrees/wt-stale/locked"
check "--unlock-finished dry run says it would unlock" grep -q "would unlock and remove  $work/wt-stale" "$work/out-unlock-dry.txt"
wtm cleanup --apply --allow-active-elsewhere --unlock-finished wt-stale > "$work/out-unlock.txt" 2>&1
check "--unlock-finished removes that worktree" test ! -e "$work/wt-stale"
check "--unlock-finished leaves other locks alone" test -d "$work/wt-locked"
check "the unlock is logged" grep -q " unlock path=$work/wt-stale " "$log"

# --- prune --------------------------------------------------------------------
rm -rf "$work/wt-unique"
expect_class <(wtm audit --format tsv --no-size) "$work/wt-unique" UNCERTAIN
wtm prune --dry-run > "$work/out-prune-dry.txt"
check "prune --dry-run lists the stale entry" grep -q "wt-unique" "$work/out-prune-dry.txt"
check "prune --dry-run keeps the metadata" test -d "$repo/.git/worktrees/wt-unique"
wtm prune > /dev/null
check "prune removes the stale metadata" test ! -e "$repo/.git/worktrees/wt-unique"
check "prune keeps the branch" has_ref refs/heads/feat/unique

# --- usage --------------------------------------------------------------------
status=0; "$BASH" "$script" > /dev/null 2>&1 || status=$?
check "no command exits 2" test "$status" = 2
status=0; "$BASH" "$script" cleanup --apply --dry-run > /dev/null 2>&1 || status=$?
check "--apply with --dry-run exits 2" test "$status" = 2
check "--help works" "$BASH" "$script" --help > /dev/null

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all checks passed (bash $BASH_VERSION)"
