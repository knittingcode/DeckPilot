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
# On Linux, ps can print the start time of one live process one second
# apart in two calls. A live owner stays the owner whatever ps prints now:
# the start on the first line of the owner file is set off by one second
# (any other time does as well), the stable one on the second is kept.
holder; h7=$last_holder
s7=$(ts acquire --owner "$h7")
holder; h8=$last_holder
s8=$(ts acquire --owner "$h8")
{ echo "$h7 Thu Jan  1 00:00:01 1970"; sed -n 2p "$s7.lock/owner"; } > "$work/owner.new"
mv "$work/owner.new" "$s7.lock/owner"
holder; h9=$last_holder
status=0; ts acquire --owner "$h9" > /dev/null 2> "$work/jitter.err" || status=$?
check "a live owner is not taken for gone when ps prints another start time" test "$status" = 3
[ "$status" = 3 ] || cat "$work/jitter.err"
ts release --owner "$h7" "$s7"; ts release --owner "$h8" "$s8"

# --- run, concurrency ------------------------------------------------------------
# Each run marks its slot busy; a second run in the same slot at the same
# time fails to create the mark and exits 9. The mark is a file that the
# shell creates with O_EXCL (noclobber), which no implementation of mkdir or
# touch can get wrong. With a second argument the job holds its slot until
# that file appears (at most 60 s), otherwise for 2 s.
cat > "$work/job.sh" << 'EOF'
#!/bin/sh
(set -C && : > "$CARGO_TARGET_DIR/busy") 2> /dev/null \
  || { echo "job: another run is using $CARGO_TARGET_DIR" >&2; exit 9; }
echo "$CARGO_TARGET_DIR" > "$1"
if [ -n "${2:-}" ]; then
  n=0
  while [ ! -e "$2" ] && [ "$n" -lt 600 ]; do sleep 0.1; n=$((n + 1)); done
else
  sleep 2
fi
rm "$CARGO_TARGET_DIR/busy"
EOF
chmod +x "$work/job.sh"

concurrent_runs() { # <label>
  local i p c n pids codes bad
  # Three at once, two slots, no waiting: two run in distinct slots, one is
  # turned away. The two hold their slots until every run has either
  # started its job or ended, so the third cannot get a slot that a quick
  # first run has already given back, however the three are scheduled.
  rm -f "$work"/run3.* "$work"/err3.* "$work"/exit3.* "$work/go3"
  pids=""
  for i in 1 2 3; do
    (
      c=0
      ts run -- "$work/job.sh" "$work/run3.$i" "$work/go3" 2> "$work/err3.$i" || c=$?
      echo "$c" > "$work/exit3.$i"
    ) &
    pids="$pids $!"
  done
  n=0
  while [ "$(find "$work" -maxdepth 1 \( -name 'run3.*' -o -name 'exit3.*' \) | wc -l)" -lt 3 ] && [ "$n" -lt 600 ]; do
    sleep 0.1
    n=$((n + 1))
  done
  : > "$work/go3"
  for p in $pids; do wait "$p" || true; done
  codes=$(cat "$work"/exit3.* | sort | tr '\n' ' ')
  check "$1of 3 concurrent runs with 2 slots, 2 succeed and 1 exits 3" test "$codes" = "0 0 3 "
  [ "$codes" = "0 0 3 " ] || { echo "     exit codes: $codes"; cat "$work"/err3.*; }
  check "$1the two runs used different slots" test "$(cat "$work"/run3.* | sort -u | wc -l | tr -d ' ')" = 2

  # Six at once with --wait: all succeed, never two in the same slot.
  pids=""
  codes=""
  for i in 1 2 3 4 5 6; do ts run --wait 60 -- "$work/job.sh" "$work/run6.$i" 2> "$work/err6.$i" & pids="$pids $!"; done
  bad=0
  for p in $pids; do c=0; wait "$p" || c=$?; codes="$codes $c"; [ "$c" = 0 ] || bad=$((bad + 1)); done
  check "${1}6 concurrent runs with --wait all succeed, one per slot at a time" test "$bad" = 0
  [ "$bad" = 0 ] || { echo "     exit codes:$codes"; cat "$work"/err6.*; }
  check "$1run releases its slot" test ! -e "$work/pool/slot-1.lock" -a ! -e "$work/pool/slot-2.lock"
}
concurrent_runs ""

# The same with a mkdir that does not fail when the directory exists. That
# is what mkdir of uutils coreutils (Ubuntu 26.04) does when two
# of them race for one path: both exit 0. Here it happens every time, so a
# lock that trusts mkdir's exit status puts all three runs in slot-1.
mkdir "$work/shim"
printf '#!/bin/sh\nexec %s -p "$@"\n' "$(command -v mkdir)" > "$work/shim/mkdir"
chmod +x "$work/shim/mkdir"
check "the stand-in mkdir succeeds on an existing directory" env PATH="$work/shim:$PATH" mkdir "$work/shim"
real_path="$PATH"
PATH="$work/shim:$PATH"
concurrent_runs "with a mkdir that is not exclusive, "
PATH="$real_path"
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

  # --- two worktrees, one slot ---------------------------------------------------
  # Cargo hashes a path package by its path relative to the workspace root
  # and judges its output fresh by mtime, so without a scrub a worktree that
  # gets a slot another one built in after its last edit runs that other
  # worktree's code. Two copies of a workspace: a member with a build script
  # (rerun-if-changed) and an excluded path crate, like plugins/runtime.
  # One is a git repository, one is not.
  mkws() { # <dir> <tag>
    mkdir -p "$1/core/src" "$1/app/src" "$1/extra/helper/src"
    printf '[workspace]\nmembers = ["core", "app"]\nexclude = ["extra"]\nresolver = "2"\n' > "$1/Cargo.toml"
    printf '[package]\nname = "probe-core"\nversion = "0.1.0"\nedition = "2021"\n' > "$1/core/Cargo.toml"
    cat > "$1/core/build.rs" << 'EOF'
fn main() {
    println!("cargo:rerun-if-changed=who.txt");
    let who = std::fs::read_to_string("who.txt").unwrap();
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("who.txt"), who).unwrap();
}
EOF
    echo 'pub fn who() -> &'"'"'static str { include_str!(concat!(env!("OUT_DIR"), "/who.txt")).trim_ascii() }' > "$1/core/src/lib.rs"
    printf '[package]\nname = "probe-helper"\nversion = "0.1.0"\nedition = "2021"\n' > "$1/extra/helper/Cargo.toml"
    printf '[package]\nname = "probe-app"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\nprobe-core = { path = "../core" }\nprobe-helper = { path = "../extra/helper" }\n' > "$1/app/Cargo.toml"
    echo 'fn main() { println!("{}/{}", probe_core::who(), probe_helper::who()); }' > "$1/app/src/main.rs"
    settag "$1" "$2"
  }
  settag() { # <dir> <tag>: the code that a build of <dir> must run
    echo "$2" > "$1/core/who.txt"
    echo 'pub fn who() -> &'"'"'static str { "'"$2"'" }' > "$1/extra/helper/src/lib.rs"
  }
  wt_run() { # <dir> <log>: prints what the build of <dir> ran
    (cd "$1" && DECKPILOT_TARGET_POOL="$work/wt-pool" DECKPILOT_TARGET_SLOTS=1 \
      "$BASH" "$script" run -- cargo run --offline -q -p probe-app 2> "$2") || echo "build failed"
  }
  mkws "$work/wt-a" A1
  (cd "$work/wt-a" && git init -q .)
  mkws "$work/wt-b" B1
  got=$(wt_run "$work/wt-a" "$work/wt.1.log")
  check "worktree A runs its code ($got)" test "$got" = A1/A1
  # Which layout this cargo writes (see scrub_slot): the scrub handles both,
  # the toolchain decides which one is tested here.
  prof="$work/wt-pool/slot-1/debug"
  if [ -d "$prof/.fingerprint" ]; then
    echo "     cargo $(cargo -V | cut -d' ' -f2) writes the old layout (.fingerprint, deps)"
  else
    echo "     cargo $(cargo -V | cut -d' ' -f2) writes the new layout (build/<package>/<hash>)"
  fi
  got=$(wt_run "$work/wt-b" "$work/wt.2.log")
  check "worktree B, next in the same slot, runs its own code ($got)" test "$got" = B1/B1
  # A third-party crate in the slot (stand-ins in both layouts): kept across
  # worktrees.
  fake_old="$prof/.fingerprint/thirdparty-0123456789abcdef $prof/deps/libthirdparty-0123456789abcdef.rlib"
  fake_new="$prof/build/thirdparty/0123456789abcdef/fingerprint $prof/build/thirdparty/0123456789abcdef/out/libthirdparty.rlib"
  mkdir -p "$prof/.fingerprint/thirdparty-0123456789abcdef" "$prof/deps" \
    "$prof/build/thirdparty/0123456789abcdef/fingerprint" "$prof/build/thirdparty/0123456789abcdef/out"
  : > "$prof/deps/libthirdparty-0123456789abcdef.rlib"
  : > "$prof/build/thirdparty/0123456789abcdef/out/libthirdparty.rlib"
  sleep 1
  settag "$work/wt-a" A2
  got=$(wt_run "$work/wt-a" "$work/wt.3.log")
  check "worktree A after an edit runs its code ($got)" test "$got" = A2/A2
  # A profile directory in a layout of neither kind (a later cargo).
  odd="$work/wt-pool/slot-1/odd-triple/debug"
  mkdir -p "$odd/units/probe-core"
  : > "$odd/.cargo-lock"
  : > "$odd/units/probe-core/output"
  got=$(wt_run "$work/wt-b" "$work/wt.4.log")
  check "worktree B after A's newer build runs its own code ($got)" test "$got" = B1/B1
  check "the scrub empties a profile directory in an unknown layout" test ! -e "$odd/units"
  kept=1
  # shellcheck disable=SC2086 # word splitting of the path lists is intended
  for f in $fake_old $fake_new; do [ -e "$f" ] || { kept=0; echo "     removed: $f"; }; done
  check "the scrub keeps dependencies that are not the worktree's own" test "$kept" = 1
  got=$(wt_run "$work/wt-b" "$work/wt.5.log")
  check "worktree B again in its slot runs its code ($got)" test "$got" = B1/B1
  check "the same worktree again in its slot rebuilds nothing" \
    test -z "$(grep -E 'Compiling|scrub|removing' "$work/wt.5.log" || true)"
  check "a slot changing worktree reports the scrub" grep -q "last used by another worktree" "$work/wt.4.log"
else
  echo "skip cargo build overlap (cargo not installed or TARGET_SLOT_SKIP_CARGO=1)"
fi

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all checks passed (bash $BASH_VERSION)"
