#!/usr/bin/env bash
# Tests tools/dev/target-slot.sh in a temporary pool: concurrent
# acquisition, mutual exclusion, stale locks, release. When cargo is
# installed it also shows that two builds in different slots run at the
# same time while two builds sharing one target directory block (a tiny
# crate outside the repository, CARGO_INCREMENTAL=0). Set
# TARGET_SLOT_SKIP_CARGO=1 to skip that part.
#
#   tools/dev/tests/target-slot.test.sh
set -euo pipefail
export LC_ALL=C

script="$(cd "$(dirname "$0")/.." && pwd)/target-slot.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/target-slot-test.XXXXXX")
work=$(cd "$work" && pwd -P)
holders=""
cleanup_test() {
  for p in $holders; do kill "$p" 2> /dev/null || true; wait "$p" 2> /dev/null || true; done
  rm -rf "$work"
}
trap cleanup_test EXIT
export DECKPILOT_TARGET_POOL="$work/pool"
export DECKPILOT_TARGET_SLOTS=2
cd "$work"

failures=0
ok() { echo "ok   $*"; }
fail() { echo "FAIL $*"; failures=$((failures + 1)); }
check() { local d="$1"; shift; if "$@"; then ok "$d"; else fail "$d"; fi; }
ts() { "$BASH" "$script" "$@"; }

# A long-lived owner process.
holder() { sleep 600 & holders="$holders $!"; last_holder=$!; }

# --- acquire, release ----------------------------------------------------------
holder; h1=$last_holder
s1=$(ts acquire --owner "$h1")
check "acquire prints a slot of the pool" test "$s1" = "$work/pool/slot-1"
check "the slot directory exists" test -d "$s1"
check "the lock names its owner" grep -q "^$h1 " "$work/pool/slot-1.lock/owner"
holder; h2=$last_holder
s2=$(ts acquire --owner "$h2")
check "a second owner gets the other slot" test "$s2" = "$work/pool/slot-2"
holder; h3=$last_holder
status=0; ts acquire --owner "$h3" > /dev/null 2>&1 || status=$?
check "with every slot held, acquire --wait 0 exits 3" test "$status" = 3
status=0; ts release --owner "$h3" "$s1" > /dev/null 2>&1 || status=$?
check "a slot cannot be released by another live process" test "$status" = 1
ts status > "$work/status.txt"
check "status lists the holders" grep -q "held by pid $h1" "$work/status.txt"
ts release --owner "$h1" "$s1"
check "release frees the slot" test ! -e "$work/pool/slot-1.lock"
check "release keeps the build output" test -d "$s1"
s3=$(ts acquire --owner "$h3")
check "the freed slot is handed out again" test "$s3" = "$s1"
ts release --owner "$h2" "$s2"; ts release --owner "$h3" "$s3"

# --- stale locks ---------------------------------------------------------------
sleep 0 & dead=$!; wait "$dead" 2> /dev/null || true
mkdir -p "$work/pool/slot-1.lock" "$work/pool/slot-2.lock"
echo "$dead Thu Jan  1 00:00:00 1970" > "$work/pool/slot-1.lock/owner"
holder; h4=$last_holder
s4=$(ts acquire --owner "$h4" 2> "$work/stale.err")
check "a lock whose owner is gone is taken over" test "$s4" = "$work/pool/slot-1"
check "the takeover is reported" grep -q "stale lock of slot-1" "$work/stale.err"
# slot-2.lock has no owner file and is fresh: being created, so held.
holder; h5=$last_holder
status=0; ts acquire --owner "$h5" > /dev/null 2>&1 || status=$?
check "a fresh lock without owner file counts as held" test "$status" = 3
# A reused PID (same number, other start time) is stale too.
rm -rf "$work/pool/slot-2.lock"; mkdir "$work/pool/slot-2.lock"
echo "$h5 Thu Jan  1 00:00:00 1970" > "$work/pool/slot-2.lock/owner"
s5=$(ts acquire --owner "$h5" 2> /dev/null)
check "a lock of a reused PID is taken over" test "$s5" = "$work/pool/slot-2"
ts release --owner "$h4" "$s4"; ts release --owner "$h5" "$s5"

# --- run, concurrency ------------------------------------------------------------
# Each run marks its slot busy; a second run in the same slot at the same
# time would fail to create the mark.
cat > "$work/job.sh" << 'EOF'
#!/bin/sh
mkdir "$CARGO_TARGET_DIR/busy" || exit 9
echo "$CARGO_TARGET_DIR" > "$1"
sleep 2
rmdir "$CARGO_TARGET_DIR/busy"
EOF
chmod +x "$work/job.sh"

# Three at once, two slots, no waiting: two run in distinct slots, one is
# turned away.
pids=""
for i in 1 2 3; do ts run -- "$work/job.sh" "$work/run3.$i" 2> /dev/null & pids="$pids $!"; done
codes=""
for p in $pids; do c=0; wait "$p" || c=$?; codes="$codes $c"; done
check "of 3 concurrent runs with 2 slots, 2 succeed and 1 exits 3" \
  test "$(echo "$codes" | tr ' ' '\n' | sort | tr '\n' ' ')" = " 0 0 3 "
check "the two runs used different slots" test "$(cat "$work"/run3.* | sort -u | wc -l | tr -d ' ')" = 2

# Six at once with --wait: all succeed, never two in the same slot.
pids=""
for i in 1 2 3 4 5 6; do ts run --wait 60 -- "$work/job.sh" "$work/run6.$i" 2> /dev/null & pids="$pids $!"; done
bad=0
for p in $pids; do c=0; wait "$p" || c=$?; [ "$c" = 0 ] || bad=$((bad + 1)); done
check "6 concurrent runs with --wait all succeed, one per slot at a time" test "$bad" = 0
check "run releases its slot" test ! -e "$work/pool/slot-1.lock" -a ! -e "$work/pool/slot-2.lock"
status=0; ts run -- sh -c 'exit 7' || status=$?
check "run passes the command's exit status" test "$status" = 7

# env: an export line for eval.
holder; h6=$last_holder
line=$(ts env --owner "$h6")
check "env prints an export of CARGO_TARGET_DIR" test "$line" = "export CARGO_TARGET_DIR=$work/pool/slot-1"
CARGO_TARGET_DIR="$work/pool/slot-1" ts release --owner "$h6"
check "release defaults to CARGO_TARGET_DIR" test ! -e "$work/pool/slot-1.lock"

status=0; ts acquire --owner 0 > /dev/null 2>&1 || status=$?
check "an owner that is not running is refused" test "$status" = 1
status=0; ts bogus > /dev/null 2>&1 || status=$?
check "an unknown command exits 2" test "$status" = 2

# --- two cargo builds ----------------------------------------------------------
if [ "${TARGET_SLOT_SKIP_CARGO:-0}" != 1 ] && command -v cargo > /dev/null 2>&1; then
  export CARGO_INCREMENTAL=0
  for c in a b; do
    mkdir -p "$work/crate-$c/src"
    printf '[package]\nname = "slot-probe-%s"\nversion = "0.1.0"\nedition = "2021"\n' "$c" > "$work/crate-$c/Cargo.toml"
    echo 'fn main() {}' > "$work/crate-$c/src/main.rs"
    # A build that takes 4 seconds, long enough to overlap.
    echo 'fn main() { std::thread::sleep(std::time::Duration::from_secs(4)); }' > "$work/crate-$c/build.rs"
  done
  build_pair() { # <target dir a> <target dir b> <log prefix>; prints seconds
    local start end
    start=$(date +%s)
    (cd "$work/crate-a" && CARGO_TARGET_DIR="$1" cargo build --offline > "$work/$3.a.log" 2>&1) &
    local pa=$!
    (cd "$work/crate-b" && CARGO_TARGET_DIR="$2" cargo build --offline > "$work/$3.b.log" 2>&1) &
    local pb=$!
    wait "$pa"; wait "$pb"
    end=$(date +%s)
    echo $((end - start))
  }
  shared=$(build_pair "$work/shared-target" "$work/shared-target" shared)
  slotted=$(build_pair "$work/pool/slot-1" "$work/pool/slot-2" slotted)
  echo "     two builds sharing one target dir: ${shared}s; in two slots: ${slotted}s"
  # The build script sleeps 4 s: builds that wait for each other take at
  # least 8 s in total. Cargo's wording of the wait message varies between
  # versions ("Blocking waiting for file lock on build directory" or "on
  # artifact directory"), so the timing decides; the logs are shown.
  lock_msg='file lock on (build|artifact) directory'
  cat "$work"/shared.*.log "$work"/slotted.*.log | sed 's/^/     | /'
  check "builds sharing a target dir block each other (>= 8 s)" test "$shared" -ge 8
  check "builds in different slots overlap (< 8 s)" test "$slotted" -lt 8
  if cat "$work"/slotted.*.log | grep -E "$lock_msg" > /dev/null; then
    fail "builds in different slots waited for a target dir lock"
  else
    ok "builds in different slots never wait for a target dir lock"
  fi
else
  echo "skip cargo build overlap (cargo not installed or TARGET_SLOT_SKIP_CARGO=1)"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all checks passed (bash $BASH_VERSION)"
