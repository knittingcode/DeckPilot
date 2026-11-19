#!/usr/bin/env bash
# A bounded pool of cargo target directories shared by worktrees: instead of
# one target/ per worktree (1-18 GB each), builds use one of N slots under
# ~/.cache/deckpilot/target-pool. A slot is held by one process at a time
# (cargo serializes builds that share a target directory, so a shared one
# would make parallel agents wait for each other). Opt-in; see
# docs/development.md, "Worktrees and build caches".
#
# The lock is a directory (macOS has no flock(1)) holding the owner's PID
# and start time; a lock whose owner is gone is taken over. Who holds it is
# decided by the shell creating a file in it with O_EXCL, not by the exit
# status of mkdir (see take_dir). A slot changing worktree is scrubbed of
# that worktree's own crates first (scrub_slot). Runs with bash 3.2 and
# newer, on macOS and Linux.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: tools/dev/target-slot.sh <command> [options]

Commands:
  run [--wait <s>] -- <command...>
        Runs the command with CARGO_TARGET_DIR set to a free slot, held for
        the command's lifetime and released afterwards. Exit status: the
        command's.
  env [--wait <s>] [--owner <pid>]
        Prints `export CARGO_TARGET_DIR=...` lines for eval, holding the slot
        for <pid> (default: the calling shell, $PPID):
          eval "$(tools/dev/target-slot.sh env --owner $$)"
        Release it with `tools/dev/target-slot.sh release --owner $$`.
  acquire [--wait <s>] [--owner <pid>]
        Prints the path of a slot held for <pid> (default: $PPID).
  release [--owner <pid>] [<slot path>]
        Releases a slot (default: $CARGO_TARGET_DIR) held by <pid> (default:
        $PPID) or by a process that is gone.
  status
        Lists the slots, their owners, sizes and idle time.
  prune [--max-age <days>] [--max-size <size>] [--dry-run]
        Empties free slots that have been idle for <days> (default 7) or
        are larger than <size> (default 30G; M or G). A slot in use is
        never touched: each one is locked while it is emptied.

Options:
  --wait <s>   Wait up to <s> seconds for a free slot (default: 0 = fail at
               once).

Environment:
  DECKPILOT_TARGET_POOL   pool directory (default ~/.cache/deckpilot/target-pool)
  DECKPILOT_TARGET_SLOTS  number of slots (default 3)
  DECKPILOT_TARGET_SLOT_MAX_AGE_DAYS  prune: idle days (default 7; 0 = off)
  DECKPILOT_TARGET_SLOT_MAX_SIZE      prune: size limit (default 30G; 0 = off)
  DECKPILOT_MIN_FREE_GB   acquire warns below this much free disk space
                          (default 25) and then also applies the size limit
  DECKPILOT_TARGET_SLOT_AUTOPRUNE     0 disables pruning on acquire

On every acquisition the other free slots that have been idle longer than
the age limit are emptied; when the disk is low, those over the size limit
too, and a warning names the commands that free more space.

Exit codes: 0 ok, 1 error, 2 usage, 3 no free slot within --wait.
A slot that the same worktree used last is preferred, so its incremental
state is reused. A slot that another worktree used last first loses the
output of this worktree's own crates (cargo would otherwise reuse the other
worktree's build of them); dependencies from crates.io and git are kept.
The worktree is that of the current directory: build in it, not elsewhere
(no --manifest-path to another worktree, no cd to one in the same shell).
EOF
}

die() { echo "target-slot: $*" >&2; exit 1; }
usage_error() { echo "target-slot: $*" >&2; echo "Try --help." >&2; exit 2; }

pool="${DECKPILOT_TARGET_POOL:-$HOME/.cache/deckpilot/target-pool}"
slots="${DECKPILOT_TARGET_SLOTS:-3}"
case "$slots" in '' | *[!0-9]* | 0) usage_error "DECKPILOT_TARGET_SLOTS must be a positive number" ;; esac
max_age_days="${DECKPILOT_TARGET_SLOT_MAX_AGE_DAYS:-7}"
max_size="${DECKPILOT_TARGET_SLOT_MAX_SIZE:-30G}"
min_free_gb="${DECKPILOT_MIN_FREE_GB:-25}"
autoprune="${DECKPILOT_TARGET_SLOT_AUTOPRUNE:-1}"

# Start time of a process, to tell a live owner from a reused PID, as ps
# prints it. On Linux this is not stable: procps adds the start in ticks to
# the boot time, which follows adjustments of the system clock, so two
# calls for one live process can print times one second apart, and a live
# owner would be taken for gone. Kept in the owner file for older copies of
# this script (worktrees on other commits share the pool).
start_of() { ps -o lstart= -p "$1" 2> /dev/null | sed 's/^ *//; s/ *$//'; }

# A start time that does not change while the process lives: on Linux the
# start in clock ticks since boot (field 22 of /proc/<pid>/stat; the command
# name in field 2 may hold spaces and parentheses, hence the cut after the
# last ")"), elsewhere the time ps prints (macOS reads it from the kernel's
# record of the start). Empty if the process is gone.
stable_start_of() {
  if [ -r "/proc/$1/stat" ]; then
    st=$(cat "/proc/$1/stat" 2> /dev/null) || return 0
    st=${st##*) }
    # shellcheck disable=SC2086 # split into fields; field 3 is now $1
    set -- $st
    [ -z "${20:-}" ] || echo "ticks:${20}"
  else
    start_of "$1"
  fi
}

# Seconds since a file was last modified (0 if it is gone).
age_of() {
  mtime=$(stat -c %Y "$1" 2> /dev/null || stat -f %m "$1" 2> /dev/null || date +%s)
  echo $(( $(date +%s) - mtime ))
}

# Writes the owner file of <lock> for <pid> atomically (rename), so that a
# reader never sees it empty or half written and takes the lock for stale.
# Line 1 is "<pid> <start as ps prints it>", the only line that older copies
# of this script read; line 2 is "start <stable start>".
write_owner() { # <lock> <pid>
  printf '%s %s\nstart %s\n' "$2" "$(start_of "$2")" "$(stable_start_of "$2")" > "$1/owner.$$"
  mv -f "$1/owner.$$" "$1/owner"
}

# Takes the lock directory <dir>: true for exactly one of the processes that
# try at the same time. mkdir(2) is atomic, but mkdir(1) is not everywhere:
# the one of uutils coreutils (Ubuntu 26.04) exits 0 when another
# process creates the directory between its own existence check and its
# mkdir(2), so two processes can both "create" it. The directory is
# therefore only where the lock lives; the lock is the file `claimed` in it,
# which the shell itself creates with O_EXCL (noclobber).
take_dir() { # <dir>
  mkdir "$1" 2> /dev/null || return 1
  (set -C && : > "$1/claimed") 2> /dev/null
}

# Removes the lock directory <dir> in one step (a rename), then its
# contents. Emptied in place, it could lose the `claimed` file of a process
# that takes the lock anew in the meantime.
drop_dir() { # <dir>
  gone="$1.gone.$$"
  rm -rf "$gone"
  if mv "$1" "$gone" 2> /dev/null; then rm -rf "$gone"; fi
}

# Whether the lock of slot <n> has an owner that is still running. A lock
# without a complete owner file is being created (or its creator died right
# after mkdir): it counts as held for 30 seconds. Sets lock_seen to whether
# the lock existed.
lock_held() { # <n>
  lock="$pool/slot-$1.lock"
  lock_seen=0
  [ -d "$lock" ] || return 1
  lock_seen=1
  opid=""; ostart=""; otag=""; ostable=""
  if [ -f "$lock/owner" ]; then
    { read -r opid ostart && read -r otag ostable; } < "$lock/owner" 2> /dev/null || true
  fi
  if [ -z "$opid" ] || [ -z "$ostart" ]; then
    [ "$(age_of "$lock")" -lt 30 ]
    return
  fi
  # An owner file of an older copy of this script has no stable start.
  if [ "$otag" = start ] && [ -n "$ostable" ]; then
    [ "$(stable_start_of "$opid")" = "$ostable" ]
    return
  fi
  [ "$(start_of "$opid")" = "$ostart" ]
}

# Tries to take slot <n> for <owner pid>; takes over a stale lock.
try_slot() { # <n> <owner>
  lock="$pool/slot-$1.lock"
  if take_dir "$lock"; then
    write_owner "$lock" "$2"
    return 0
  fi
  lock_held "$1" && return 1
  # Taking over a stale lock is serialized by a second lock, so that a
  # process that saw the old lock as stale cannot remove the new one that
  # another process has just created in its place.
  reclaim="$pool/slot-$1.reclaim"
  if ! take_dir "$reclaim"; then
    # Its holder died while reclaiming (it holds it for milliseconds).
    if [ "$(age_of "$reclaim")" -ge 30 ]; then drop_dir "$reclaim"; fi
    return 1
  fi
  taken=1
  # Only a lock that is there and stale is removed: one that its owner
  # released meanwhile may already be another process's new lock.
  if ! lock_held "$1" && [ "$lock_seen" = 1 ]; then
    drop_dir "$lock"
    echo "target-slot: took over the stale lock of slot-$1" >&2
  fi
  if take_dir "$lock"; then
    write_owner "$lock" "$2"
    taken=0
  fi
  drop_dir "$reclaim"
  return "$taken"
}

# The worktree of the current directory, for slot affinity.
worktree_key() { git rev-parse --show-toplevel 2> /dev/null || pwd -P; }

# The names of the packages whose sources live in worktree <dir>: the
# [package] name of every Cargo.toml in it (both workspaces, excluded
# crates, examples). Reading the manifests needs no network and no lock
# file; a name too many only costs a rebuild.
local_packages() { # <dir>
  {
    git -C "$1" ls-files -co --exclude-standard -- 'Cargo.toml' '*/Cargo.toml' 2> /dev/null \
      || (cd "$1" && find . \( -name target -o -name node_modules -o -name .git \) -prune -o -name Cargo.toml -print)
  } | while IFS= read -r m; do
    [ -f "$1/$m" ] || continue
    awk -v sq="'" '
      /^[[:space:]]*\[/ { inpkg = ($0 ~ /^[[:space:]]*\[package\][[:space:]]*(#.*)?$/); next }
      inpkg && /^[[:space:]]*name[[:space:]]*=/ {
        v = $0; sub(/^[^=]*=[[:space:]]*/, "", v)
        q = substr(v, 1, 1)
        if (q == "\"" || q == sq) { v = substr(v, 2); sub(q ".*", "", v); print v }
        inpkg = 0
      }' "$1/$m"
  done | sort -u
}

# Cargo names the artifacts and fingerprints of a path package after a hash
# of its path RELATIVE to the workspace root, and judges them fresh when they
# are newer than the sources. So the same crate in two worktrees maps to the
# same files in a shared target directory, and a build in worktree A can
# reuse what worktree B compiled after A's last edit: A runs B's code. When a
# slot changes worktree, this removes the output of every package whose
# sources live in the new one (fingerprints, which forces a rebuild, plus
# their artifacts, build script output, incremental state, uplifted
# binaries, examples, docs). Registry and git dependencies are keyed by
# version and source, are identical across worktrees, and are kept. If the
# packages cannot be determined, the whole slot is emptied.
#
# Cargo has two layouts of a profile directory (debug, release,
# <triple>/debug, ...), and which one a slot holds depends on the toolchain
# that built in it:
#   up to 1.99:  .fingerprint/<pkg>-<hash>, build/<pkg>-<hash> (build
#                scripts), deps/<crate>-<hash>, deps/lib<crate>-<hash>.*
#   1.100 on:    build/<pkg>/<hash>/{fingerprint,out,run}, one directory
#                per package holding all of its units
# Both have incremental/<crate>-<hash> and the uplifted outputs at the top.
own_entries() { # stdin: paths; prints those named <one of $names>-*
  while IFS= read -r f; do
    for n in $names; do
      case "${f##*/}" in "$n"-*) echo "$f" ;; esac
    done
  done
}

# The same for paths <profile>/build/<pkg>/<hash>/fingerprint: prints those
# whose <pkg> is exactly one of $names.
own_units() { # stdin: paths
  while IFS= read -r f; do
    u=${f%/*/fingerprint}
    for n in $names; do
      [ "${u##*/}" != "$n" ] || echo "$f"
    done
  done
}

# The profile directories of slot <dir>: those holding cargo's build lock
# (.cargo-lock) or an old-layout .fingerprint directory.
profile_dirs() { # <dir>
  { find "$1" -mindepth 2 -maxdepth 3 -type f -name .cargo-lock
    find "$1" -mindepth 2 -maxdepth 4 -type d -name .fingerprint
  } | while IFS= read -r m; do dirname "$m"; done | sort -u
}

scrub_slot() { # <slot dir> <worktree>
  dir="$1"
  [ -n "$(ls -A "$dir" 2> /dev/null)" ] || return 0
  names=$(local_packages "$2" | grep -E '^[A-Za-z0-9_-]+$' || true)
  if [ -z "$names" ]; then
    echo "target-slot: no cargo packages found in $2; emptying $(basename "$dir")" >&2
    find "$dir" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
    return 0
  fi
  echo "target-slot: $(basename "$dir") was last used by another worktree; removing its workspace crates" >&2
  profile_dirs "$dir" | while IFS= read -r p; do
    # A layout that is neither of the two (a later cargo) would make this
    # scrub remove nothing: empty that profile directory instead.
    if [ ! -d "$p/.fingerprint" ] \
      && [ -z "$(find "$p/build" -mindepth 3 -maxdepth 3 -type d -name fingerprint 2> /dev/null | head -n 1)" ]; then
      echo "target-slot: unknown layout of ${p#"$dir"/}; emptying it" >&2
      find "$p" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
      continue
    fi
    for e in "$p"/* "$p"/.[!.]*; do
      case "${e##*/}" in
        .fingerprint | build | deps | incremental | .cargo-lock | .cargo-build-lock | .cargo-artifact-lock | '*' | '.[!.]*') ;;
        *) rm -rf "$e" ;;
      esac
    done
    for n in $names; do
      c=$(printf '%s' "$n" | tr - _)
      # Old layout, then the new one (build/<pkg> exactly).
      rm -rf "$p/.fingerprint/$n"-* "$p/build/$n"-* \
        "$p/deps/$c"-* "$p/deps/lib$c"-* "$p/incremental/$c"-* \
        "${p:?}/build/$n"
    done
  done
  find "$dir" -mindepth 1 -maxdepth 2 -type d -name doc | while IFS= read -r d; do
    for n in $names; do
      c=$(printf '%s' "$n" | tr - _)
      rm -rf "${d:?}/$c" "${d:?}/src/$c"
    done
  done
  # A fingerprint left behind would let a stale artifact through: fail.
  left=$(
    find "$dir" -mindepth 3 -maxdepth 5 -path '*/.fingerprint/*' -prune -print | own_entries
    find "$dir" -mindepth 5 -maxdepth 6 -type d -path '*/build/*/*/fingerprint' -prune -print | own_units
  )
  [ -z "$left" ] || { echo "target-slot: could not remove: $left" >&2; return 1; }
}

# A size such as 30G or 500M, in KiB (0 = no limit).
size_kb() { # <size>
  case "$1" in
    0) echo 0 ;;
    *[0-9]G | *[0-9]g) v=${1%?}; case "$v" in '' | *[!0-9]*) return 1 ;; esac; echo $((v * 1048576)) ;;
    *[0-9]M | *[0-9]m) v=${1%?}; case "$v" in '' | *[!0-9]*) return 1 ;; esac; echo $((v * 1024)) ;;
    *[!0-9]* | '') return 1 ;;
    *) echo $(($1 * 1048576)) ;;
  esac
}

# When slot <dir> was last used: acquisitions and releases touch
# .deckpilot-slot-used; slots of older copies of this script only have
# .deckpilot-slot-user (written when the slot changed worktree).
last_use_file() { # <dir>
  for f in "$1/.deckpilot-slot-used" "$1/.deckpilot-slot-user"; do
    [ ! -e "$f" ] || { echo "$f"; return; }
  done
  echo "$1"
}

# Free space of the pool's file system, in KiB.
free_kb() { df -Pk "$pool" 2> /dev/null | awk 'NR == 2 { print $4 }'; }

# Empties the free slots that are idle for <days> or larger than <kb>
# (0 = that limit is off), except slot <skip>. Each one is locked while it
# is emptied, so a slot in use, or taken meanwhile, is kept.
prune_slots() { # <days> <kb> <dry run 0|1> <skip slot number or "">
  pn=1
  while [ "$pn" -le "$slots" ]; do
    pdir="$pool/slot-$pn"
    if [ "$pn" != "$4" ] && [ -d "$pdir" ] && [ -n "$(ls -A "$pdir" 2> /dev/null)" ]; then
      reason=""
      idle=$(age_of "$(last_use_file "$pdir")")
      if [ "$1" -gt 0 ] && [ "$idle" -ge $(($1 * 86400)) ]; then
        reason="idle for $((idle / 86400)) days"
      fi
      if [ -z "$reason" ] && [ "$2" -gt 0 ]; then
        kb=$( (du -sk "$pdir" 2> /dev/null || true) | awk 'NR == 1 { print $1 }')
        [ "${kb:-0}" -le "$2" ] || reason="$((kb / 1024)) MiB, over the limit"
      fi
      if [ -n "$reason" ]; then
        if [ "$3" = 1 ]; then
          echo "target-slot: would empty slot-$pn ($reason)" >&2
        elif lock_held "$pn"; then
          echo "target-slot: slot-$pn is in use; kept" >&2
        elif try_slot "$pn" "$$"; then
          find "$pdir" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
          drop_dir "$pool/slot-$pn.lock"
          echo "target-slot: emptied slot-$pn ($reason)" >&2
        else
          echo "target-slot: slot-$pn was taken meanwhile; kept" >&2
        fi
      fi
    fi
    pn=$((pn + 1))
  done
}

# After an acquisition: warn when the disk is low, and empty other idle
# slots (and, when low, oversized ones).
after_acquire() { # <slot number>
  low=0
  fk=$(free_kb)
  if [ -n "$fk" ] && [ "$min_free_gb" -gt 0 ] && [ "$fk" -lt $((min_free_gb * 1048576)) ]; then
    low=1
    echo "target-slot: warning: only $((fk / 1048576)) GiB free on the disk of $pool (below $min_free_gb GiB)." >&2
    echo "target-slot: free space with: tools/dev/target-slot.sh prune; tools/dev/worktree-maintenance.sh report" >&2
  fi
  [ "$autoprune" != 0 ] || return 0
  limit=0
  if [ "$low" = 1 ]; then limit=$(size_kb "$max_size" 2> /dev/null || echo 0); fi
  prune_slots "$max_age_days" "$limit" 0 "$1" || true
}

acquire() { # <owner> <wait seconds>
  [ -n "$(start_of "$1")" ] || die "owner process $1 is not running"
  mkdir -p "$pool"
  key=$(worktree_key)
  deadline=$(( $(date +%s) + $2 ))
  while :; do
    # Slots last used by this worktree first, then the rest.
    mine=""; others=""
    n=1
    while [ "$n" -le "$slots" ]; do
      if [ "$(cat "$pool/slot-$n/.deckpilot-slot-user" 2> /dev/null || true)" = "$key" ]; then
        mine="$mine $n"
      else
        others="$others $n"
      fi
      n=$((n + 1))
    done
    for n in $mine $others; do
      if try_slot "$n" "$1"; then
        mkdir -p "$pool/slot-$n"
        # The record is written only after the scrub, so an interrupted
        # scrub is redone by the next acquisition.
        if [ "$(cat "$pool/slot-$n/.deckpilot-slot-user" 2> /dev/null || true)" != "$key" ]; then
          if ! (scrub_slot "$pool/slot-$n" "$key"); then
            drop_dir "$pool/slot-$n.lock"
            die "could not clean slot-$n for $key"
          fi
          printf '%s\n' "$key" > "$pool/slot-$n/.deckpilot-slot-user"
        fi
        : > "$pool/slot-$n/.deckpilot-slot-used"
        chosen=$n
        after_acquire "$chosen"
        echo "$pool/slot-$chosen"
        return 0
      fi
    done
    if [ "$(date +%s)" -ge "$deadline" ]; then
      echo "target-slot: all $slots slots of $pool are in use" >&2
      exit 3
    fi
    sleep 1
  done
}

release() { # <owner> <slot path>
  slot="${2%/}"
  case "$slot" in "$pool"/slot-[0-9]*) ;; *) die "not a slot of $pool: $slot" ;; esac
  n="${slot##*/slot-}"
  lock="$pool/slot-$n.lock"
  [ -d "$lock" ] || return 0
  opid=""
  [ ! -f "$lock/owner" ] || read -r opid _ < "$lock/owner" || true
  if [ "$opid" = "$1" ] || ! lock_held "$n"; then
    [ ! -d "$slot" ] || : > "$slot/.deckpilot-slot-used"
    drop_dir "$lock"
  else
    die "slot-$n is held by process $opid, not $1"
  fi
}

cmd="${1:-}"
[ -n "$cmd" ] || { usage >&2; exit 2; }
shift
wait_s=0
dry_run=0
owner="$PPID"
slot_arg=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -h | --help) usage; exit 0 ;;
    --wait) [ "$#" -ge 2 ] || usage_error "--wait needs seconds"
      case "$2" in '' | *[!0-9]*) usage_error "--wait needs seconds" ;; esac
      wait_s="$2"; shift ;;
    --owner) [ "$#" -ge 2 ] || usage_error "--owner needs a pid"
      case "$2" in '' | *[!0-9]*) usage_error "--owner needs a pid" ;; esac
      owner="$2"; shift ;;
    --max-age) [ "$#" -ge 2 ] || usage_error "--max-age needs days"
      case "$2" in '' | *[!0-9]*) usage_error "--max-age needs days" ;; esac
      max_age_days="$2"; shift ;;
    --max-size) [ "$#" -ge 2 ] || usage_error "--max-size needs a size"
      max_size="$2"; shift ;;
    --dry-run) dry_run=1 ;;
    --) shift; break ;;
    -*) usage_error "unknown option: $1" ;;
    *) [ "$cmd" = release ] && [ -z "$slot_arg" ] || usage_error "unexpected argument: $1"
      slot_arg="$1" ;;
  esac
  shift
done

case "$cmd" in
  -h | --help) usage ;;
  acquire) acquire "$owner" "$wait_s" ;;
  env)
    slot=$(acquire "$owner" "$wait_s")
    printf 'export CARGO_TARGET_DIR=%q\n' "$slot"
    ;;
  release)
    [ -n "$slot_arg" ] || slot_arg="${CARGO_TARGET_DIR:-}"
    [ -n "$slot_arg" ] || usage_error "no slot given and CARGO_TARGET_DIR is not set"
    release "$owner" "$slot_arg"
    ;;
  run)
    [ "$#" -gt 0 ] || usage_error "run needs a command after --"
    slot=$(acquire "$$" "$wait_s")
    trap 'release "$$" "$slot"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    status=0
    CARGO_TARGET_DIR="$slot" "$@" || status=$?
    exit "$status"
    ;;
  status)
    n=1
    while [ "$n" -le "$slots" ]; do
      dir="$pool/slot-$n"
      state=free
      if [ -d "$pool/slot-$n.lock" ]; then
        owner_line=$(cat "$pool/slot-$n.lock/owner" 2> /dev/null || echo "?")
        if lock_held "$n"; then state="held by pid ${owner_line%% *}"; else state="stale (pid ${owner_line%% *} gone)"; fi
      fi
      size=$( (du -sh "$dir" 2> /dev/null || true) | awk 'NR == 1 { print $1 }')
      user=$(cat "$dir/.deckpilot-slot-user" 2> /dev/null || echo "-")
      idle="-"
      [ ! -d "$dir" ] || idle="$(($(age_of "$(last_use_file "$dir")") / 86400))d"
      printf 'slot-%s  %-28s  %6s  idle %-4s  last used by %s\n' "$n" "$state" "${size:-0}" "$idle" "$user"
      n=$((n + 1))
    done
    ;;
  prune)
    case "$max_age_days" in '' | *[!0-9]*) usage_error "the age limit must be a number of days" ;; esac
    kb=$(size_kb "$max_size") || usage_error "not a size: $max_size (use e.g. 30G or 500M)"
    prune_slots "$max_age_days" "$kb" "$dry_run" ""
    ;;
  *) usage_error "unknown command: $cmd" ;;
esac
