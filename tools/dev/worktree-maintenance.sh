#!/usr/bin/env bash
# Audits and cleans up git worktrees of this repository (and others given
# with --repo): parallel agents each get a worktree with its own cargo
# target/, which fills the disk. Read-only unless `cleanup --apply` or
# `prune` (without --dry-run) is given. See docs/development.md,
# "Worktrees and build caches", and `--help`.
#
# Runs with bash 3.2 (macOS) and newer, on macOS and Linux.
set -euo pipefail
export LC_ALL=C

usage() {
  cat <<'EOF'
Usage: tools/dev/worktree-maintenance.sh <command> [options]

Commands:
  audit        Read-only: every worktree with its state and class.
  report       Read-only: counts per class, largest directories, and the
               space that cleanup could reclaim.
  disk-usage   Read-only: sizes of every worktree and its build directories.
  prune        `git worktree prune` (stale metadata of worktrees whose
               directory is gone). --dry-run only lists them.
  cleanup      Removes SAFE worktrees with `git worktree remove` (never
               --force), re-checking each one right before removing it.
               Dry run unless --apply is given.

Classes:
  ACTIVE     locked (an agent may be running), or a process (other than a
             macOS indexer) has a file or its working directory inside.
  REQUIRED   uncommitted changes, untracked files that are not ignored,
             commits that no remote ref holds (nor main, as a patch), an
             open pull request, a worktree nested inside, or the main
             checkout.
  UNCERTAIN  something could not be determined (gh unavailable, a stale
             lock, a git error, a missing directory).
  SAFE       none of the above: removing it loses nothing.

Options:
  --repo <path>            Repository to inspect (repeatable). Default: the
                           repository of the current directory.
  --base <ref>             Ref for patch-equivalence (`git cherry`).
                           Default: origin/HEAD, else origin/main.
  --fetch-pr-heads         First fetch GitHub's pull request heads into
                           refs/remotes/origin/pr/* (only those refs change),
                           so commits of squash-merged pull requests count
                           as preserved.
  --format table|tsv       Output of audit (default: table).
  --no-size                Skip size measurement (faster).
  --dry-run                cleanup, prune: only show what would happen
                           (the default for cleanup).
  --apply                  cleanup: really remove.
  --artifacts              cleanup: also delete build directories (target,
                           node_modules, apps/desktop/dist; ignored by git
                           and holding no tracked file) in inactive
                           worktrees (not locked, no process; never the main
                           checkout).
  --unlock-finished <name> cleanup: treat this locked worktree (directory
                           name or path) as finished. Repeat per worktree.
                           Its lock is removed only right before removal.
  --allow-active-elsewhere cleanup --apply: proceed although some worktree
                           is ACTIVE (ACTIVE ones are still never touched).
  --log <file>             cleanup log (default:
                           ~/.cache/deckpilot/worktree-maintenance.log,
                           or $WORKTREE_MAINTENANCE_LOG).
  -h, --help               This help.

Exit codes:
  0  success (also: nothing to do)
  1  error (not a git repository, git failure)
  2  usage error
  3  cleanup --apply refused: a worktree is ACTIVE
  4  cleanup: a removal failed

Environment (mainly for tests):
  WORKTREE_MAINTENANCE_GH    gh command (default: gh)
  WORKTREE_MAINTENANCE_LSOF  lsof command (default: lsof; "none" disables
                             the process check, which makes worktrees
                             UNCERTAIN unless /proc is available)
EOF
}

die() { echo "worktree-maintenance: $*" >&2; exit 1; }
usage_error() { echo "worktree-maintenance: $*" >&2; echo "Try --help." >&2; exit 2; }

GH_CMD="${WORKTREE_MAINTENANCE_GH:-gh}"
LSOF_CMD="${WORKTREE_MAINTENANCE_LSOF:-lsof}"
LOG_FILE="${WORKTREE_MAINTENANCE_LOG:-$HOME/.cache/deckpilot/worktree-maintenance.log}"

command=""
repos=""
base_opt=""
fetch_pr_heads=0
format=table
no_size=0
dry_run=""
apply=0
artifacts=0
unlock_list=""
allow_active=0

[ "$#" -gt 0 ] || { usage >&2; exit 2; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    -h | --help) usage; exit 0 ;;
    audit | report | disk-usage | prune | cleanup)
      [ -z "$command" ] || usage_error "more than one command"
      command="$1" ;;
    --repo) [ "$#" -ge 2 ] || usage_error "--repo needs a path"
      repos="$repos$2
"; shift ;;
    --base) [ "$#" -ge 2 ] || usage_error "--base needs a ref"; base_opt="$2"; shift ;;
    --fetch-pr-heads) fetch_pr_heads=1 ;;
    --format) [ "$#" -ge 2 ] || usage_error "--format needs table or tsv"
      case "$2" in table | tsv) format="$2" ;; *) usage_error "unknown format: $2" ;; esac
      shift ;;
    --no-size) no_size=1 ;;
    --dry-run) dry_run=1 ;;
    --apply) apply=1 ;;
    --artifacts) artifacts=1 ;;
    --unlock-finished) [ "$#" -ge 2 ] || usage_error "--unlock-finished needs a worktree name"
      unlock_list="$unlock_list$2
"; shift ;;
    --allow-active-elsewhere) allow_active=1 ;;
    --log) [ "$#" -ge 2 ] || usage_error "--log needs a file"; LOG_FILE="$2"; shift ;;
    *) usage_error "unknown argument: $1" ;;
  esac
  shift
done
[ -n "$command" ] || usage_error "no command given"
if [ "$apply" = 1 ] && [ "$dry_run" = 1 ]; then usage_error "--apply and --dry-run exclude each other"; fi
if [ "$command" != cleanup ]; then
  for flag_value in "$apply:--apply" "$artifacts:--artifacts" "$allow_active:--allow-active-elsewhere"; do
    [ "${flag_value%%:*}" = 0 ] || usage_error "${flag_value#*:} only applies to cleanup"
  done
  [ -z "$unlock_list" ] || usage_error "--unlock-finished only applies to cleanup"
fi
if [ -n "$dry_run" ] && [ "$command" != cleanup ] && [ "$command" != prune ]; then
  usage_error "--dry-run only applies to cleanup and prune"
fi

# Resolve the repositories (as their main checkouts) before leaving the
# current directory: the script then runs from / so that its own working
# directory never counts as a process inside a worktree.
if [ -z "$repos" ]; then
  top=$(git rev-parse --show-toplevel 2> /dev/null) || die "not inside a git repository (use --repo)"
  repos="$top
"
fi
repo_mains=""
while IFS= read -r r; do
  [ -n "$r" ] || continue
  git -C "$r" rev-parse --git-dir > /dev/null 2>&1 || die "not a git repository: $r"
  first=$(git -C "$r" worktree list --porcelain 2> /dev/null | sed -n '1s/^worktree //p') \
    || die "git worktree list failed in $r"
  [ -n "$first" ] || die "no worktrees in $r"
  case "
$repo_mains" in *"
$first
"*) continue ;; esac
  repo_mains="$repo_mains$first
"
done <<EOF
$repos
EOF
cd /

work=$(mktemp -d "${TMPDIR:-/tmp}/worktree-maintenance.XXXXXX")
trap 'rm -rf "$work"' EXIT

# Real path of a directory (resolves /var -> /private/var on macOS).
realdir() { (cd "$1" 2> /dev/null && pwd -P); }

human() {
  awk -v k="${1:-0}" 'BEGIN {
    split("K M G T", u, " "); v = k; i = 1
    while (v >= 1024 && i < 4) { v /= 1024; i++ }
    if (i == 1) printf "%dK", v; else printf "%.1f%s", v, u[i]
  }'
}

size_kb() {
  if [ "$no_size" = 1 ]; then echo 0; return; fi
  # du exits non-zero for unreadable entries, yet prints the total.
  out=$(du -sk "$1" 2> /dev/null || true)
  printf '%s\n' "$out" | awk 'NR == 1 && $1 ~ /^[0-9]+$/ { print $1; found = 1 } END { if (!found) print 0 }'
}

# --- Processes -------------------------------------------------------------
# One snapshot of open files: "<pid>\t<command>\t<path>" in $work/procs.
# PROCS_OK=0 when no method is available.
PROCS_OK=0
snapshot_processes() {
  : > "$work/procs"
  PROCS_OK=0
  if [ "$LSOF_CMD" != none ] && command -v "$LSOF_CMD" > /dev/null 2>&1; then
    # Run directly (not in a subshell of this script, which would hold the
    # script file open); lsof exits 1 when it cannot inspect some processes.
    "$LSOF_CMD" -n -P -w +c 0 -F pcn > "$work/lsof.raw" 2> /dev/null || true
    if [ -s "$work/lsof.raw" ]; then
      awk '/^p/ { pid = substr($0, 2); next }
           /^c/ { cmd = substr($0, 2); next }
           /^n\// { print pid "\t" cmd "\t" substr($0, 2) }' "$work/lsof.raw" > "$work/procs"
      PROCS_OK=1
      return
    fi
  fi
  if [ -d /proc/self ] && [ -r /proc/self/cwd ]; then
    for p in /proc/[0-9]*; do
      cwd=$(readlink "$p/cwd" 2> /dev/null) || continue
      comm=$(cat "$p/comm" 2> /dev/null) || comm="?"
      printf '%s\t%s\t%s\n' "${p#/proc/}" "$comm" "$cwd"
    done > "$work/procs"
    PROCS_OK=1
  fi
}

# Processes in a worktree, as "cmd(pid)" words: open paths are assigned to
# the deepest worktree that contains them (agent worktrees live inside the
# main checkout). Indexers (com.apple.*, Spotlight's mds/mdworker) and this
# script are ignored. Needs $work/roots: one real worktree path per line.
processes_in() {
  awk -F '\t' -v want="$1" -v self="$$" '
    NR == FNR { if ($0 != "") roots[$0] = 1; next }
    {
      pid = $1; cmd = $2; path = $3
      if (pid == self) next
      if (cmd ~ /^com\.apple/ || cmd ~ /^(mds|mds_stores|mdworker|mdworker_shared|mdsync|Spotlight)$/) next
      best = ""
      for (r in roots)
        if ((path == r || index(path, r "/") == 1) && length(r) > length(best)) best = r
      if (best == want && !((pid) in seen)) { seen[pid] = 1; out = out (out == "" ? "" : ",") cmd "(" pid ")" }
    }
    END { print out }' "$work/roots" "$work/procs"
}

# --- Pull requests ---------------------------------------------------------
# Open pull requests of a repository as "<number>\t<head branch>\t<head sha>"
# in $work/prs.<n>; GH_OK.<n> says whether gh answered.
pr_snapshot() { # <repo main> <index>
  if command -v "${GH_CMD%% *}" > /dev/null 2>&1 \
    && (cd "$1" && $GH_CMD pr list --state open --limit 1000 \
      --json number,headRefName,headRefOid \
      --jq '.[] | "\(.number)\t\(.headRefName)\t\(.headRefOid)"') > "$work/prs.$2" 2> /dev/null; then
    echo 1 > "$work/gh_ok.$2"
  else
    echo 0 > "$work/gh_ok.$2"
  fi
}

# --- Worktree listing ------------------------------------------------------
# "<path>\t<locked 0|1>\t<lock reason>\t<prunable 0|1>" per worktree of a
# repository, main checkout first.
list_worktrees() {
  git -C "$1" worktree list --porcelain | awk '
    function flush() { if (path != "") print path "\t" locked "\t" reason "\t" prunable }
    /^worktree / { flush(); path = substr($0, 10); locked = 0; reason = ""; prunable = 0; next }
    /^bare$/ { path = ""; next }
    /^locked/ { locked = 1; reason = substr($0, 8); next }
    /^prunable/ { prunable = 1; next }
    END { flush() }'
}

base_ref_for() { # <repo main>
  if [ -n "$base_opt" ]; then echo "$base_opt"; return; fi
  ref=$(git -C "$1" symbolic-ref -q refs/remotes/origin/HEAD 2> /dev/null) && { echo "$ref"; return; }
  git -C "$1" rev-parse -q --verify refs/remotes/origin/main > /dev/null 2>&1 && { echo refs/remotes/origin/main; return; }
  echo ""
}

pid_alive() { ps -p "$1" > /dev/null 2>&1; }

in_unlock_list() { # <path>
  name=$(basename "$1")
  while IFS= read -r u; do
    [ -n "$u" ] || continue
    if [ "$u" = "$name" ] || [ "$u" = "$1" ] || [ "${u%/}" = "$1" ]; then return 0; fi
  done <<EOF
$unlock_list
EOF
  return 1
}

# Build directories of a worktree that cleanup --artifacts may delete:
# allowlisted names only, real directories (no symlinks), inside the
# worktree, ignored by git and holding no tracked file. One relative path
# per line.
generated_dirs() { # <worktree path>
  wt="$1"
  {
    printf '%s\n' target plugins/runtime/target apps/desktop/dist node_modules
    git -C "$wt" status --porcelain=v1 --ignored=traditional --untracked-files=normal 2> /dev/null \
      | sed -n 's/^!! \(.*\)\/$/\1/p' \
      | awk '$0 !~ /^"/ { n = split($0, a, "/"); if (a[n] == "target" || a[n] == "node_modules" || $0 == "apps/desktop/dist") print }' \
      || true
  } | awk '!seen[$0]++' > "$work/candidates"
  while IFS= read -r rel; do
    if is_deletable_build_dir "$wt" "$rel"; then echo "$rel"; fi
  done < "$work/candidates"
  return 0
}

is_deletable_build_dir() { # <worktree> <relative dir>
  wt="$1"; rel="$2"; dir="$wt/$rel"
  case "$rel" in "" | /* | *..*) return 1 ;; esac
  name=$(basename "$rel")
  case "$name" in target | node_modules) ;; *) [ "$rel" = apps/desktop/dist ] || return 1 ;; esac
  [ -d "$dir" ] && [ ! -L "$dir" ] || return 1
  real_wt=$(realdir "$wt") && real_dir=$(realdir "$dir") || return 1
  case "$real_dir" in "$real_wt"/*) ;; *) return 1 ;; esac
  git -C "$wt" check-ignore -q "$rel/" 2> /dev/null || git -C "$wt" check-ignore -q "$rel" 2> /dev/null || return 1
  [ -z "$(git -C "$wt" ls-files -- "$rel" 2> /dev/null | head -n 1)" ] || return 1
  # Never a directory that is, or holds, a worktree.
  [ -z "$(roots_inside "$real_dir" or-equal)" ] || return 1
  return 0
}

# Worktrees (real paths from $work/roots) strictly inside a directory, or
# also the directory itself with "or-equal".
roots_inside() { # <real dir> [or-equal]
  awk -v d="$1" -v eq="${2:-}" '(index($0, d "/") == 1) || (eq != "" && $0 == d)' "$work/roots" 2> /dev/null || true
}

# --- Inspection ------------------------------------------------------------
# Inspects one worktree and sets R_* variables. Needs a process snapshot,
# $work/roots and the pull request snapshot of the repository.
inspect() { # <repo index> <repo main> <path> <locked> <lock reason> <prunable> <with sizes 0|1>
  idx="$1"; main="$2"; p="$3"; locked="$4"; R_LOCK_REASON="$5"; prunable="$6"; sizes="$7"
  R_REPO=$(basename "$main"); R_PATH="$p"; R_BRANCH="-"; R_HEAD="-"
  R_DIRTY="?"; R_UNTRACKED="?"; R_UNIQUE="?"; R_PR="?"; R_PROCS="?"
  R_LOCKED="$locked"; R_SIZE=0; R_BUILD=0; R_GEN=""; R_INACTIVE=0; R_ISMAIN=0
  active=""; required=""; uncertain=""

  real_p=$(realdir "$p" || true)
  real_main=$(realdir "$main" || true)
  if [ "$p" = "$main" ] || { [ -n "$real_p" ] && [ "$real_p" = "$real_main" ]; }; then
    R_ISMAIN=1; required="main-checkout"
  fi

  if [ "$prunable" = 1 ] || [ ! -d "$p" ]; then
    R_CLASS=UNCERTAIN; R_REASONS="directory-missing(run-prune)"
    return
  fi

  # Lock: an agent's worktree stays locked while it runs (and after).
  lock_overridden=0
  if [ "$locked" = 1 ]; then
    if in_unlock_list "$p"; then
      lock_overridden=1
    else
      lock_pid=$(printf '%s' "$R_LOCK_REASON" | sed -n 's/.*pid \([0-9][0-9]*\).*/\1/p')
      if [ -n "$lock_pid" ] && ! pid_alive "$lock_pid"; then
        uncertain="$uncertain stale-lock(pid-$lock_pid-gone)"
      else
        active="$active locked"
      fi
    fi
  fi

  # Processes.
  if [ "$PROCS_OK" = 1 ]; then
    R_PROCS=$(processes_in "${real_p:-$p}")
    [ -z "$R_PROCS" ] || active="$active processes"
    [ -n "$R_PROCS" ] || R_PROCS="-"
  else
    uncertain="$uncertain process-check-unavailable"
  fi
  if [ -z "$active" ] && [ "$PROCS_OK" = 1 ] && { [ "$locked" = 0 ] || [ "$lock_overridden" = 1 ]; }; then
    R_INACTIVE=1
  fi

  # A worktree inside this one (agents' .claude/worktrees/*) would be
  # deleted along with it.
  if [ -n "$real_p" ] && [ -n "$(roots_inside "$real_p")" ]; then
    required="$required contains-worktrees"
  fi

  # Git state.
  if R_HEAD=$(git -C "$p" rev-parse -q --verify HEAD 2> /dev/null); then
    R_BRANCH=$(git -C "$p" symbolic-ref -q --short HEAD 2> /dev/null || echo "(detached)")
  else
    R_HEAD="-"; uncertain="$uncertain no-HEAD"
  fi
  if status=$(git -C "$p" status --porcelain=v1 --untracked-files=normal 2> /dev/null); then
    R_DIRTY=$(printf '%s\n' "$status" | awk 'NF && substr($0, 1, 2) != "??"' | wc -l | tr -d ' ')
    R_UNTRACKED=$(printf '%s\n' "$status" | awk 'substr($0, 1, 2) == "??"' | wc -l | tr -d ' ')
    [ "$R_DIRTY" = 0 ] || required="$required dirty"
    [ "$R_UNTRACKED" = 0 ] || required="$required untracked"
  else
    uncertain="$uncertain git-status-failed"
  fi

  # Commits that no remote-tracking ref holds and that are not patches
  # already in the base branch (git cherry "+").
  if [ "$R_HEAD" != "-" ]; then
    if not_remote=$(git -C "$p" rev-list HEAD --not --remotes 2> /dev/null); then
      if [ -z "$not_remote" ]; then
        R_UNIQUE=0
      else
        base=$(base_ref_for "$main")
        if [ -n "$base" ]; then
          if plus=$(git -C "$p" cherry "$base" HEAD 2> /dev/null | awk '$1 == "+" { print $2 }'); then
            if [ -z "$plus" ]; then R_UNIQUE=0
            else R_UNIQUE=$(printf '%s\n' "$not_remote" | grep -c -F -x -f <(printf '%s\n' "$plus") || true)
            fi
          else
            R_UNIQUE="?"; uncertain="$uncertain git-cherry-failed"
          fi
        else
          R_UNIQUE=$(printf '%s\n' "$not_remote" | wc -l | tr -d ' ')
        fi
      fi
      [ "$R_UNIQUE" = 0 ] || [ "$R_UNIQUE" = "?" ] || required="$required unique-commits"
    else
      uncertain="$uncertain git-rev-list-failed"
    fi
  fi

  # Open pull request (by head branch or head commit).
  if [ "$(cat "$work/gh_ok.$idx" 2> /dev/null || echo 0)" = 1 ]; then
    R_PR=$(awk -F '\t' -v b="$R_BRANCH" -v h="$R_HEAD" '($2 == b || $3 == h) { printf "%s#%s", (n++ ? "," : ""), $1 }' "$work/prs.$idx")
    if [ -n "$R_PR" ]; then required="$required open-pr"; else R_PR="-"; fi
  elif [ "$R_BRANCH" = "(detached)" ] || [ "$R_BRANCH" = "-" ]; then
    R_PR="-" # no branch: no pull request can use it
  else
    uncertain="$uncertain pr-state-unknown"
  fi

  if [ "$sizes" = 1 ]; then
    R_SIZE=$(size_kb "$p")
    while IFS= read -r rel; do
      [ -n "$rel" ] || continue
      kb=$(size_kb "$p/$rel")
      R_BUILD=$((R_BUILD + kb))
      R_GEN="$R_GEN${R_GEN:+;}$rel=$kb"
    done <<EOF
$(generated_dirs "$p")
EOF
  fi

  if [ -n "$active" ]; then R_CLASS=ACTIVE
  elif [ -n "$required" ]; then R_CLASS=REQUIRED
  elif [ -n "$uncertain" ]; then R_CLASS=UNCERTAIN
  else R_CLASS=SAFE
  fi
  R_REASONS=$(echo "$active $required $uncertain" | awk '{ $1 = $1; gsub(/ /, ","); print }')
  [ "$lock_overridden" = 0 ] || R_REASONS="${R_REASONS}${R_REASONS:+,}lock-overridden"
  [ -n "$R_REASONS" ] || R_REASONS="-"
  return 0
}

# --- Snapshot of all repositories ------------------------------------------
# $work/worktrees: "<repo index>\t<repo main>\t<path>\t<locked>\t<reason>\t<prunable>"
collect() {
  : > "$work/worktrees"
  i=0
  while IFS= read -r main; do
    [ -n "$main" ] || continue
    i=$((i + 1))
    if [ "$fetch_pr_heads" = 1 ]; then
      echo "Fetching pull request heads of $(basename "$main") into refs/remotes/origin/pr/* ..." >&2
      git -C "$main" fetch --quiet --no-tags --no-write-fetch-head origin \
        '+refs/pull/*/head:refs/remotes/origin/pr/*' >&2 \
        || echo "warning: fetching pull request heads failed; continuing without them" >&2
    fi
    pr_snapshot "$main" "$i"
    list_worktrees "$main" | while IFS= read -r line; do
      printf '%s\t%s\t%s\n' "$i" "$main" "$line"
    done >> "$work/worktrees" || die "git worktree list failed in $main"
  done <<EOF
$repo_mains
EOF
  refresh_roots
  snapshot_processes
}

refresh_roots() {
  cut -f 3 "$work/worktrees" | while IFS= read -r p; do realdir "$p" || echo "$p"; done > "$work/roots"
}

# Inspects every worktree into $work/records (TSV, see header below).
TSV_HEADER="class	repo	path	branch	head	dirty	untracked	unique	pr	locked	lock_reason	processes	size_kb	build_kb	build_dirs	inactive	main	reasons"
inspect_all() { # <with sizes 0|1>
  : > "$work/records"
  while IFS='	' read -r idx main p locked reason prunable; do
    inspect "$idx" "$main" "$p" "$locked" "$reason" "$prunable" "$1"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$R_CLASS" "$R_REPO" "$R_PATH" "$R_BRANCH" "$R_HEAD" "$R_DIRTY" "$R_UNTRACKED" "$R_UNIQUE" \
      "$R_PR" "$R_LOCKED" "${R_LOCK_REASON:--}" "$R_PROCS" "$R_SIZE" "$R_BUILD" "${R_GEN:--}" \
      "$R_INACTIVE" "$R_ISMAIN" "$R_REASONS" >> "$work/records"
  done < "$work/worktrees"
  # The size of a worktree excludes the worktrees nested inside it (each
  # counts its own).
  awk -F '\t' -v OFS='\t' '
    { line[NR] = $0; path[NR] = $3; size[NR] = $13 }
    END {
      for (i = 1; i <= NR; i++) {
        parent = 0
        for (j = 1; j <= NR; j++)
          if (j != i && index(path[i], path[j] "/") == 1 && (parent == 0 || length(path[j]) > length(path[parent]))) parent = j
        if (parent) sub_kb[parent] += size[i]
      }
      for (i = 1; i <= NR; i++) {
        n = split(line[i], f, "\t")
        if (sub_kb[i] > 0) { f[13] = f[13] - sub_kb[i]; if (f[13] < 0) f[13] = 0 }
        out = f[1]; for (k = 2; k <= n; k++) out = out OFS f[k]
        print out
      }
    }' "$work/records" > "$work/records.adjusted"
  mv "$work/records.adjusted" "$work/records"
}

with_sizes() { if [ "$no_size" = 1 ]; then echo 0; else echo 1; fi; }

# --- Commands --------------------------------------------------------------
cmd_audit() {
  collect
  inspect_all "$(with_sizes)"
  if [ "$format" = tsv ]; then
    echo "$TSV_HEADER"
    cat "$work/records"
    return
  fi
  [ "$PROCS_OK" = 1 ] || echo "note: no lsof and no /proc: processes unknown" >&2
  for f in "$work"/gh_ok.*; do
    [ "$(cat "$f")" = 1 ] || { echo "note: gh unavailable or failed: open pull requests unknown" >&2; break; }
  done
  printf '%-9s  %-34s  %-34s  %-9s  %5s %5s %4s  %-8s  %-4s  %-5s  %7s  %7s\n' \
    CLASS WORKTREE BRANCH HEAD DIRTY UNTR UNIQ PR LOCK PROCS SIZE BUILD
  while IFS='	' read -r class repo p branch head dirty untr uniq pr locked _reason procs size build _gen _inactive _main _reasons; do
    name=$(basename "$p")
    [ "$_main" = 0 ] || name="$name (main)"
    lock=no; [ "$locked" = 0 ] || lock=yes
    nprocs=0; [ "$procs" = "-" ] || nprocs=$(printf '%s' "$procs" | awk -F, '{ print ($0 == "?" ? "?" : NF) }')
    printf '%-9s  %-34.34s  %-34.34s  %-9.9s  %5s %5s %4s  %-8.8s  %-4s  %-5s  %7s  %7s\n' \
      "$class" "$repo/$name" "$branch" "$head" "$dirty" "$untr" "$uniq" "$pr" "$lock" "$nprocs" \
      "$(human "$size")" "$(human "$build")"
  done < "$work/records"
  echo
  echo "Details:"
  while IFS='	' read -r class _repo p _branch _head _d _u _q _pr locked reason procs _s _b gen _i _m reasons; do
    echo "  $p"
    echo "    class $class: ${reasons}"
    [ "$locked" = 0 ] || echo "    lock: $reason"
    [ "$procs" = "-" ] || echo "    processes: $procs"
    if [ "$gen" != "-" ]; then
      printf '    build dirs:'
      printf '%s\n' "$gen" | tr ';' '\n' | while IFS='=' read -r d kb; do printf ' %s=%s' "$d" "$(human "$kb")"; done
      echo
    fi
  done < "$work/records"
}

cmd_report() {
  collect
  inspect_all 1
  echo "Worktrees per class:"
  for c in ACTIVE REQUIRED UNCERTAIN SAFE; do
    n=$(awk -F '\t' -v c="$c" '$1 == c' "$work/records" | wc -l | tr -d ' ')
    kb=$(awk -F '\t' -v c="$c" '$1 == c { s += $13 } END { print s + 0 }' "$work/records")
    printf '  %-9s %3s  %8s\n' "$c" "$n" "$(human "$kb")"
  done
  echo
  echo "Largest worktrees:"
  sort -t '	' -k13,13nr "$work/records" | awk 'NR <= 10' | while IFS='	' read -r class _r p _rest; do
    kb=$(printf '%s\n' "$_rest" | cut -f 10)
    printf '  %8s  %-9s  %s\n' "$(human "$kb")" "$class" "$p"
  done
  echo
  echo "Largest build directories:"
  awk -F '\t' '$15 != "-" { n = split($15, g, ";"); for (i = 1; i <= n; i++) { split(g[i], kv, "="); print kv[2] "\t" $1 "\t" $3 "/" kv[1] } }' "$work/records" \
    | sort -t '	' -k1,1nr | awk 'NR <= 10' | while IFS='	' read -r kb class d; do
      printf '  %8s  %-9s  %s\n' "$(human "$kb")" "$class" "$d"
    done
  echo
  safe_kb=$(awk -F '\t' '$1 == "SAFE" { s += $13 } END { print s + 0 }' "$work/records")
  art_kb=$(awk -F '\t' '$1 != "SAFE" && $16 == 1 && $17 == 0 { s += $14 } END { print s + 0 }' "$work/records")
  echo "Reclaimable:"
  printf '  %8s  cleanup --apply (SAFE worktrees)\n' "$(human "$safe_kb")"
  printf '  %8s  cleanup --apply --artifacts (build directories of other inactive worktrees)\n' "$(human "$art_kb")"
  locked_build=$(awk -F '\t' '$10 == 1 { s += $14 } END { print s + 0 }' "$work/records")
  printf '  %8s  in build directories of locked worktrees (only with --unlock-finished)\n' "$(human "$locked_build")"
}

cmd_disk_usage() {
  collect
  : > "$work/records"
  total=0
  while IFS='	' read -r _idx _main p _locked _reason prunable; do
    [ "$prunable" = 0 ] && [ -d "$p" ] || { printf '  %8s  %s (missing)\n' "-" "$p"; continue; }
    kb=$(size_kb "$p"); total=$((total + kb))
    printf '%8s  %s\n' "$(human "$kb")" "$p"
    generated_dirs "$p" | while IFS= read -r rel; do
      printf '%8s    %s\n' "$(human "$(size_kb "$p/$rel")")" "$rel"
    done
  done < "$work/worktrees"
  printf '%8s  total\n' "$(human "$total")"
}

cmd_prune() {
  while IFS= read -r main; do
    [ -n "$main" ] || continue
    if [ "$dry_run" = 1 ]; then
      echo "$(basename "$main"): would prune:"
      git -C "$main" worktree prune -n -v 2>&1 | sed 's/^/  /'
    else
      echo "$(basename "$main"): pruning stale worktree metadata:"
      git -C "$main" worktree prune -v 2>&1 | sed 's/^/  /'
    fi
  done <<EOF
$repo_mains
EOF
}

log_line() {
  mkdir -p "$(dirname "$LOG_FILE")"
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >> "$LOG_FILE"
}

# Re-inspects one worktree from fresh data (worktree list, processes, pull
# requests) and sets R_*.
recheck() { # <repo index> <repo main> <path> <with sizes>
  pr_snapshot "$2" "$1"
  line=$(list_worktrees "$2" | awk -F '\t' -v p="$3" '$1 == p') || line=""
  if [ -z "$line" ]; then R_CLASS=GONE; return; fi
  locked=$(printf '%s\n' "$line" | cut -f 2)
  reason=$(printf '%s\n' "$line" | cut -f 3)
  prunable=$(printf '%s\n' "$line" | cut -f 4)
  snapshot_processes
  inspect "$1" "$2" "$3" "$locked" "$reason" "$prunable" "$4"
}

cmd_cleanup() {
  collect
  inspect_all 0
  active=$(awk -F '\t' '$1 == "ACTIVE" { print "  " $3 " (" $18 ")" }' "$work/records")
  if [ -n "$active" ]; then
    echo "ACTIVE worktrees (never touched):"
    echo "$active"
    if [ "$allow_active" = 0 ]; then
      if [ "$apply" = 1 ]; then
        echo "Refusing cleanup --apply while worktrees are ACTIVE: agents may be running." >&2
        echo "Stop them first, or pass --allow-active-elsewhere." >&2
        exit 3
      fi
      echo "(cleanup --apply would refuse without --allow-active-elsewhere)"
    fi
    echo
  fi
  failures=0; removed=0; recovered=0
  while IFS='	' read -r class _repo p _rest; do
    [ "$class" = SAFE ] || continue
    idx=$(awk -F '\t' -v p="$p" '$3 == p { print $1; exit }' "$work/worktrees")
    main=$(awk -F '\t' -v p="$p" '$3 == p { print $2; exit }' "$work/worktrees")
    recheck "$idx" "$main" "$p" 1
    if [ "$R_CLASS" != SAFE ]; then
      echo "skip     $p: now $R_CLASS (${R_REASONS:-gone})"
      continue
    fi
    desc="path=$p branch=$R_BRANCH head=$R_HEAD size_kb=$R_SIZE"
    if [ "$apply" = 0 ]; then
      if [ "$R_LOCKED" = 1 ]; then echo "would unlock and remove  $p ($R_BRANCH, $(human "$R_SIZE"))"
      else echo "would remove  $p ($R_BRANCH, $(human "$R_SIZE"))"; fi
      recovered=$((recovered + R_SIZE)); removed=$((removed + 1))
      continue
    fi
    if [ "$R_LOCKED" = 1 ]; then
      git -C "$main" worktree unlock "$p" && log_line "unlock $desc lock=\"$R_LOCK_REASON\""
    fi
    if git -C "$main" worktree remove "$p"; then
      log_line "remove-worktree $desc"
      echo "removed  $p ($R_BRANCH, $(human "$R_SIZE"))"
      recovered=$((recovered + R_SIZE)); removed=$((removed + 1))
    else
      failures=$((failures + 1))
      log_line "remove-failed $desc"
      if [ "$R_LOCKED" = 1 ]; then git -C "$main" worktree lock --reason "$R_LOCK_REASON" "$p" || true; fi
      echo "FAILED   $p" >&2
    fi
  done < "$work/records"

  if [ "$artifacts" = 1 ]; then
    while IFS='	' read -r class _repo p _rest; do
      [ "$class" != SAFE ] || continue # removed (or would be) as a whole
      idx=$(awk -F '\t' -v p="$p" '$3 == p { print $1; exit }' "$work/worktrees")
      main=$(awk -F '\t' -v p="$p" '$3 == p { print $2; exit }' "$work/worktrees")
      recheck "$idx" "$main" "$p" 0
      [ "$R_CLASS" != GONE ] || continue
      if [ "$R_ISMAIN" = 1 ] || [ "$R_INACTIVE" != 1 ]; then continue; fi
      generated_dirs "$p" > "$work/gen"
      while IFS= read -r rel; do
        [ -n "$rel" ] || continue
        kb=$(size_kb "$p/$rel")
        if [ "$apply" = 0 ]; then
          echo "would delete build dir  $p/$rel ($(human "$kb"))"
          recovered=$((recovered + kb))
          continue
        fi
        # Last check right before deleting.
        snapshot_processes
        if [ -n "$(processes_in "$(realdir "$p")")" ] || ! is_deletable_build_dir "$p" "$rel"; then
          echo "skip     $p/$rel: no longer inactive or no longer a build directory"
          continue
        fi
        if rm -rf -- "${p:?}/$rel"; then
          log_line "remove-artifact path=$p/$rel branch=$R_BRANCH head=$R_HEAD size_kb=$kb"
          echo "deleted  $p/$rel ($(human "$kb"))"
          recovered=$((recovered + kb))
        else
          failures=$((failures + 1)); echo "FAILED   $p/$rel" >&2
        fi
      done < "$work/gen"
    done < "$work/records"
  fi

  echo
  if [ "$apply" = 0 ]; then
    echo "Dry run: $removed worktree(s) would be removed, $(human "$recovered") would be recovered."
    echo "Run again with --apply to remove them."
  else
    echo "Removed $removed worktree(s); recovered $(human "$recovered"). Log: $LOG_FILE"
  fi
  [ "$failures" = 0 ] || exit 4
}

case "$command" in
  audit) cmd_audit ;;
  report) cmd_report ;;
  disk-usage) cmd_disk_usage ;;
  prune) cmd_prune ;;
  cleanup) cmd_cleanup ;;
esac
