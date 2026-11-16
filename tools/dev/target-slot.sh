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
        Lists the slots, their owners and sizes.

Options:
  --wait <s>   Wait up to <s> seconds for a free slot (default: 0 = fail at
               once).

Environment:
  DECKPILOT_TARGET_POOL   pool directory (default ~/.cache/deckpilot/target-pool)
  DECKPILOT_TARGET_SLOTS  number of slots (default 3)

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

# Start time of a process, to tell a live owner from a reused PID.
start_of() { ps -o lstart= -p "$1" 2> /dev/null | sed 's/^ *//; s/ *$//'; }

# Seconds since a file was last modified (0 if it is gone).
age_of() {
  mtime=$(stat -c %Y "$1" 2> /dev/null || stat -f %m "$1" 2> /dev/null || date +%s)
  echo $(( $(date +%s) - mtime ))
}

# Writes the owner file of <lock> for <pid> atomically (rename), so that a
# reader never sees it empty or half written and takes the lock for stale.
write_owner() { # <lock> <pid>
  printf '%s %s\n' "$2" "$(start_of "$2")" > "$1/owner.$$"
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
  opid=""; ostart=""
  [ ! -f "$lock/owner" ] || read -r opid ostart < "$lock/owner" || true
  if [ -z "$opid" ] || [ -z "$ostart" ]; then
    [ "$(age_of "$lock")" -lt 30 ]
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
own_entries() { # stdin: paths; prints those named <one of $names>-*
  while IFS= read -r f; do
    for n in $names; do
      case "${f##*/}" in "$n"-*) echo "$f" ;; esac
    done
  done
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
  # Profile directories (debug, release, <triple>/debug, ...) are those
  # holding a .fingerprint directory.
  find "$dir" -mindepth 2 -maxdepth 4 -type d -name .fingerprint | while IFS= read -r fp; do
    p=$(dirname "$fp")
    for e in "$p"/* "$p"/.[!.]*; do
      case "${e##*/}" in .fingerprint | build | deps | incremental | .cargo-lock | '*' | '.[!.]*') ;; *) rm -rf "$e" ;; esac
    done
    for n in $names; do
      c=$(printf '%s' "$n" | tr - _)
      rm -rf "$p/.fingerprint/$n"-* "$p/build/$n"-* \
        "$p/deps/$c"-* "$p/deps/lib$c"-* "$p/incremental/$c"-*
    done
  done
  find "$dir" -mindepth 1 -maxdepth 2 -type d -name doc | while IFS= read -r d; do
    for n in $names; do
      c=$(printf '%s' "$n" | tr - _)
      rm -rf "${d:?}/$c" "${d:?}/src/$c"
    done
  done
  # A fingerprint left behind would let a stale artifact through: fail.
  left=$(find "$dir" -mindepth 3 -maxdepth 5 -path '*/.fingerprint/*' -prune -print | own_entries)
  [ -z "$left" ] || { echo "target-slot: could not remove: $left" >&2; return 1; }
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
        echo "$pool/slot-$n"
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
    drop_dir "$lock"
  else
    die "slot-$n is held by process $opid, not $1"
  fi
}

cmd="${1:-}"
[ -n "$cmd" ] || { usage >&2; exit 2; }
shift
wait_s=0
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
      printf 'slot-%s  %-28s  %6s  last used by %s\n' "$n" "$state" "${size:-0}" "$user"
      n=$((n + 1))
    done
    ;;
  *) usage_error "unknown command: $cmd" ;;
esac
