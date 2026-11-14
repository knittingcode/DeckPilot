# Plugin runtime spikes: results

Feasibility spikes for the runtime proposed in
`docs/marketplace/architecture.md` (PR #191, sections 3 and 5): a wasm
engine in wasmtime, inside one OS-sandboxed host process per plugin, with a
permission broker in the agent.

This is a standalone Cargo project (its own `[workspace]` and `Cargo.lock`).
It is not part of the DeckPilot build and is not production code.

```sh
rustup target add wasm32-unknown-unknown   # once, for the pinned toolchain
cargo test                                  # 42 tests on macOS, 37 on Linux, 35 on Windows
cargo run --release --bin ipc-bench         # the measurements below
```

## Summary

| Spike | Result |
| --- | --- |
| 1. Wasm isolation | **Demonstrated for a Rust guest**, not for QuickJS. No WASI is linked; a module importing any WASI function does not instantiate. Epoch deadline and fuel stop an infinite loop; `StoreLimits` stops a memory bomb at 64 MiB; five kinds of guest trap leave the host running |
| 2. Host IPC | **Demonstrated.** Length-prefixed JSON over stdin/stdout, 9 µs median round trip; the supervisor survives a crashed (SIGABRT) and a hung child and kills the hung one on timeout |
| 3. Permission broker | **Demonstrated.** Deny by default; per-plugin grants; `http.fetch` origin allowlist and `fs.read` below a granted directory, with traversal, absolute paths and symlink escapes denied; plugin identity taken from the channel, not the request |
| 4. OS sandbox | **Demonstrated on macOS only** (`sandbox-exec`, deny-default profile): the host process itself cannot read the user's files, write outside its scratch directory, connect over TCP or to a Unix socket, or exec. Linux and Windows not implemented |
| 5. Cross-platform | Tests pass on macOS (arm64). macOS-only code is behind `cfg(target_os = "macos")`; Windows type-checks locally (`cargo check --tests --target x86_64-pc-windows-msvc`); **Linux and Windows test runs not verified**. `.github/workflows/marketplace-spikes.yml` runs the suite on all three platforms |

## Environment

macOS 27.2 (26B123), Apple Silicon (arm64), Rust 1.99.1 (repository
toolchain pin), release build, Cranelift backend. All numbers are from one
machine and one run each; treat them as orders of magnitude.

Dependencies added (this project only):

| Crate | Version | Use |
| --- | --- | --- |
| `wasmtime` | 50.0.1 (pinned `=`; latest stable at the time; 51.0.0 is rc) | features `cranelift`, `runtime`, `std`, `wat` only (no WASI, no component model, no cache) |
| `serde` | 1.0.229 | frames |
| `serde_json` | 1.0.154 | frames |
| `url` | 2.5.8 | origin parsing in the broker |
| `tempfile` (dev) | 3.27.0 | test fixtures |

144 crates in `Cargo.lock` in total (mostly wasmtime and Cranelift).

## Layout

| Path | Role |
| --- | --- |
| `guest/` | Test guest (Rust, `wasm32-unknown-unknown`, 16 KB). Stand-in for the QuickJS engine module. Exports deliberate misbehaviour (`spin`, `memory_bomb`, `trap_*`), ambient-authority probes and a `relay` that forwards a request through the single import `deckpilot.host_call` |
| `build.rs` | Builds the guest with a separate target directory and embeds it |
| `src/engine.rs` | wasmtime host: engine config, `StoreLimits`, epoch ticker, the `host_call` import (bounds-checked) |
| `src/protocol.rs` | Frames: 4-byte big-endian length + JSON; size limits from the design (2 MiB / 256 KiB) |
| `src/bin/plugin-host.rs` | The plugin host process: fresh Store per invocation; `host_call` becomes a `call` frame; test hooks `crash`, `hang`, `probe` |
| `src/supervisor.rs` | Stand-in for the agent: spawns the host with an empty environment and `/` as working directory, routes `call` frames to the broker, timeouts, kill |
| `src/broker.rs` | `authorize(plugin, op, args) -> Decision`, the only decision point; audit log without values |
| `src/sandbox.rs` | macOS Seatbelt profile and launcher |
| `tests/` | `isolation.rs` (11), `ipc.rs` (11), `broker.rs` (15, of which 2 Unix-only), `sandbox_macos.rs` (5, macOS-only) |

## Spike 1: wasm isolation

What the tests show (`tests/isolation.rs`):

- **Exactly one import.** The guest module imports only
  `deckpilot.host_call`. Rust std on `wasm32-unknown-unknown` compiles file,
  network, environment, clock and process calls to stubs: inside the guest,
  `fs::read`, `fs::write`, `read_dir`, `env::args`, `TcpStream::connect` and
  `Command::output` return an error; `env::vars` and `SystemTime::now`
  panic (a trap). None reaches the host.
- **No ambient authority unless linked.** Seven WAT modules that import WASI
  `fd_write`, `path_open`, `sock_accept`, `environ_get`, `clock_time_get`,
  `random_get` or `proc_exit` all fail at instantiation with "unknown
  import". The authority a guest has is exactly the set of functions the
  host puts in the `Linker`.
- **Infinite loop.** Epoch interruption (10 ms ticker thread, deadline
  25 ticks = the design's 250 ms) traps `spin` with `Trap::Interrupt` after
  300 ms (one to two ticks of slack). Fuel (50 M units) traps it with
  `Trap::OutOfFuel` after about 2 ms. The deadline is reset when the guest
  returns from `host_call`, so time spent in the broker does not count
  (test: a 400 ms host call does not trap). That reset had to come *before*
  the host calls back into the guest (`guest_alloc`), otherwise the guest
  trapped on re-entry; a real host must handle this too.
- **Memory bomb.** With `StoreLimits` at 64 MiB, a guest allocating 1 MiB
  chunks traps (allocator failure becomes `abort` = `unreachable`) at 63 MiB
  of linear memory; raw `memory.grow` returns -1 at the cap; a module whose
  declared initial memory exceeds the cap does not instantiate.
- **Traps do not crash the host.** `panic!`, `unreachable`, out-of-bounds
  load (`MemoryOutOfBounds`), unbounded recursion (also `MemoryOutOfBounds`:
  the Rust shadow stack in linear memory overflows before the native wasm
  stack) and the memory bomb each return an error to the host; a fresh Store
  in the same process then works.
- **Hostile import arguments.** A WAT guest passing an out-of-range pointer or
  a 2 GiB length to `host_call` gets an error; the host handler is never
  called.

Measurements:

| | Value |
| --- | --- |
| Guest compile (Cranelift, 16 KB module) | 16-18 ms |
| Fresh Store + instantiate + one call, in process | 11 µs median, 17 µs p99 |
| guest → `host_call` → guest, in process | 0.2 µs per call |
| Epoch deadline 250 ms, observed stop | 300-305 ms |
| Memory bomb stopped (64 MiB) | 9-11 ms |
| Host process RSS after ready / after 3,000 invocations and a bomb | 12.8 MiB / 12.8 MiB (peak during the bomb not measured) |
| `plugin-host` release binary (unstripped, with Cranelift) | 13 MB |

**Crash isolation versus security isolation.** What this spike proves is
that *our* guest, in wasmtime 50.0.1 with this configuration, cannot reach
host resources except through `host_call`, and that its failures are
contained (crash isolation within the process). Wasm is also a security
boundary only to the extent wasmtime itself is free of escape bugs; that is
not something a spike can prove. Infinite loops and memory bombs are
resource isolation; traps are crash isolation; the absence of imports is the
security property. A process crash outside wasm (a bug in the host's own
native code, or a wasmtime bug) is handled only by spike 2.

## Spike 2: host ↔ plugin IPC

What the tests show (`tests/ipc.rs`):

- `ready` handshake; 1,000 ping round trips; guest invocations through the
  process; guest traps and an epoch-interrupted infinite loop are reported as
  errors and the host keeps serving.
- The host is started with an empty environment (probe: 0 variables).
- **Crash:** the host calls `abort()`; the supervisor's reader sees EOF,
  reports `Closed(signal 6)`, a send to the dead child fails with an error
  (no SIGPIPE crash), and a respawn works.
- **Hang:** the host blocks in native code; `recv` with a 500 ms timeout
  returns `Timeout`, the child is killed (wait returns at once, non-success),
  and a respawn works. An invocation deadline does the same.
- Malformed frames: the host exits with code 3 on a garbage frame; the codec
  rejects oversized length prefixes before allocating, truncated frames,
  invalid JSON and unknown message types (e.g. `getDatabaseKey`).

Measurements (release, 1,000 iterations each):

| Round trip | p50 | p95 | p99 |
| --- | --- | --- | --- |
| ping/pong | 9 µs | 13-20 µs | 16-64 µs |
| `invoke add()` (new Store per invocation) | 26-28 µs | 32-46 µs | 42-99 µs |
| `invoke relay` → `fs.read` allowed (guest → host → broker → file → back) | 81-83 µs | 127-130 µs | 180-190 µs |
| `invoke relay` → `http.fetch` denied | 44-46 µs | 53-55 µs | 63-73 µs |
| capability call inside one invocation (1,000 `fs.read`s) | 44-49 µs mean | | |
| spawn `plugin-host` until `ready`, unsandboxed (20) | 17-24 ms | 18-26 ms | first spawn 200-340 ms |
| spawn until `ready` under `sandbox-exec` (20) | 25-36 ms | 28-50 ms | 28-50 ms |

The first spawn of a freshly built binary is an outlier (200-340 ms; likely
the system's first-launch scan). Start-up is dominated by compiling the
guest; the design's plan to precompile at build time removes most of it.

**IPC is not a sandbox.** The unsandboxed control test in spike 4 runs the
same host process and reads the user's files, lists the home directory,
connects to a local TCP port and a local Unix socket, and executes
`/bin/echo`. A separate process gives crash isolation and per-plugin
kill/limits only.

## Spike 3: permission enforcement

`Broker::authorize(plugin, op, args)` is the only decision point; the
supervisor passes the plugin id bound to the child process. Denials proven
by `tests/broker.rs`:

- Unknown plugin, unknown op (`process.exec`, `keystore.getDatabaseKey`),
  empty op, missing or mistyped arguments; a grant of one kind does not
  imply another.
- `fs.read`: allowed for regular files below the granted directory
  (canonicalized at grant time, so a symlinked grant path also works).
  Denied: `../secret.txt`, `sub/../../secret.txt`, `../data-evil/...`
  (sibling with a common prefix), any `..` at all (conservative), absolute
  paths, `C:\...` and UNC forms, empty path, NUL, directories, missing
  files; on Unix, symlinks inside the granted directory that point to a file
  outside, a directory outside, a relative `../` target or `/etc/passwd`
  (a symlink that stays inside is allowed).
- `http.fetch`: allowed only for the exact origin `https://api.example.com`
  (host case and an explicit `:443` normalize). Denied: `http:`, another
  port, the parent domain, a subdomain, `api.example.com.evil.net`,
  `https://api.example.com@evil.net/` (userinfo trick; the host is
  `evil.net`), any userinfo, the allowed URL inside a query string, `ftp:`,
  `file:`, an IP literal, unparsable input.
- Revocation applies to the next call; the audit log stores op and target
  (host or relative path), never URL query, body or file content.
- End to end (guest → `host_call` → plugin host → `call` frame → broker →
  `reply`): allowed and denied requests behave as above; a process spawned
  for plugin `b` that writes `"plugin":"a"` into its request is still `b`;
  a malformed guest request never reaches the broker; 200 calls in one
  invocation are each audited.

Not covered: `http.fetch` performs no request (the decision is what is
tested; redirects, DNS rebinding and private ranges after resolution are
not); grant storage and consent; rate limits and quotas.

Production note: `fs.read` here checks with `canonicalize` and then opens by
path, which leaves a time-of-check/time-of-use window if anything else can
change the directory. A real implementation should open relative to a
directory handle that cannot be escaped (`cap-std`, or `openat2` with
`RESOLVE_BENEATH` on Linux). The design does not make the file system
grantable in API 1 (§5.2); it is included only because the spike brief asked
for it.

## Spike 4: OS sandbox (macOS)

`SandboxProfile::macos_deny_default` produces a `(deny default)` Seatbelt
profile that allows: exec of the host binary only, reads of the host binary
and system libraries (`/usr/lib`, `/System/Library`, cryptexes, the dyld
cache, `/dev/null`, `/dev/urandom`), metadata of `/`, `/private`, `/var`,
read of the root directory listing, read/write of one scratch directory,
`sysctl-read`. No network, no `mach-lookup`, no IPC. The supervisor
launches the host through `/usr/bin/sandbox-exec -p <profile>`.

`tests/sandbox_macos.rs`, with probes executed by the host's own native code
(i.e. modelling a plugin that has escaped wasm):

| Probe | Unsandboxed (control) | Sandboxed |
| --- | --- | --- |
| read a file in another temp dir | ok | `EPERM` |
| read `/etc/passwd`, `~/.zshrc` | (not run) | `EPERM` |
| list `$HOME`, `/Users` | ok | `EPERM` |
| write outside scratch, write in `$HOME` | ok | `EPERM` (file not created) |
| TCP connect to a local listener, to `1.1.1.1:443` | ok | `EPERM` |
| connect to a Unix socket (stand-in for the agent endpoint) | ok | `EPERM` |
| exec `/bin/echo` | ok | `EPERM` |
| write and read inside scratch | | ok |
| run the wasm guest (Cranelift JIT), epoch interrupt | | ok |
| broker-mediated `fs.read` of the same denied file | | ok (the supervisor reads it) |

Findings:

- With a strictly path-scoped read allowlist the host aborts at start-up
  before `main` (no message, SIGABRT). Bisecting showed the missing rule is
  `file-read-data` on `/` itself (directory listing of the root). Who reads
  it (dyld or the Rust runtime) was not determined.
- Cranelift-generated code runs under this profile for an unsigned,
  non-hardened binary. **Not tested:** a signed binary with the hardened
  runtime, where JIT needs `com.apple.security.cs.allow-jit` (the design's
  spike S2), and notarization.
- Sandbox start-up costs about 8-12 ms extra per spawn.

Not proven here:

- `sandbox-exec` and SBPL are deprecated and undocumented. The design's v1
  plan is the host calling `sandbox_init` on itself before reading any
  plugin byte; that path, and the App Sandbox helper fallback, are not
  tested. The profile is a sketch, not a reviewed policy (for example,
  `sysctl-read` and the system library reads are broad, and `mach-lookup`
  needs for a real host were not explored).
- **Windows (not verified):** AppContainer or LPAC with no capabilities,
  started with `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` and an
  explicit `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, inside a Job Object
  (`JOB_OBJECT_LIMIT_PROCESS_MEMORY`, `ACTIVE_PROCESS = 1`,
  `KILL_ON_JOB_CLOSE`, UI restrictions). Crates: `windows` or `windows-sys`
  (`Win32_Security_Isolation`, `Win32_System_JobObjects`,
  `Win32_System_Threading`).
- **Linux (not verified):** `PR_SET_NO_NEW_PRIVS`, Landlock (best available
  ABI; ABI 4+ also restricts TCP bind/connect) and a seccomp-bpf allowlist
  denying `open*`, `socket`, `connect`, `execve`, `ptrace`, plus rlimits and
  `PR_SET_DUMPABLE=0`. Crates: `landlock`, `seccompiler` (or `libseccomp`),
  `rustix` or `libc`. wasmtime's own syscalls (mmap/mprotect, signals,
  threads for the epoch ticker) must be on the allowlist.

## Spike 5: cross-platform

- Platform-specific code: `src/sandbox.rs` (`macos_deny_default` and the
  `sandbox-exec` launcher, `cfg(target_os = "macos")`; on other platforms
  `SandboxProfile::command` panics and `SandboxProfile::SUPPORTED` is
  false), `tests/sandbox_macos.rs` (whole file `cfg(target_os = "macos")`),
  two symlink tests in `tests/broker.rs` (`cfg(unix)`), and the
  `connect_unix` probe in the host (`cfg(unix)`). Everything else uses std
  only.
- Verified locally: `cargo test` on macOS arm64 (42 passed), `cargo clippy
  --all-targets` with no warnings, `cargo check --tests --bins --target
  x86_64-pc-windows-msvc` succeeds.
- Not verified: running the tests on Linux or Windows. A Linux
  `cargo check` from macOS fails only because wasmtime's build script needs
  a Linux C cross compiler. `.github/workflows/marketplace-spikes.yml` runs the suite on
  `ubuntu-24.04`, `macos-15` and `windows-2025`.

## What is NOT proven

- **QuickJS-in-wasm was not built.** Building rquickjs or QuickJS-NG for
  `wasm32-wasip1` needs a WASI C sysroot (wasi-sdk), which is not installed
  here; Javy's CLI is a prebuilt third-party binary and its output imports
  WASI. Wasm isolation is proven for our 16 KB Rust guest only. Engine size,
  compile time, memory baseline and per-call cost for QuickJS are unknown;
  the 11 µs instantiate and 12.8 MiB RSS figures will grow. Follow-up: build
  QuickJS-NG with wasi-sdk (or `javy-plugin-api` 7.1.0 from source), link only
  clock and random (or nothing), and re-run `tests/isolation.rs` and
  `ipc-bench` against it.
- The OS sandbox is shown **only on macOS**, only via the deprecated
  `sandbox-exec`, only for an unsigned binary.
- wasmtime's freedom from escape bugs (the actual security boundary of layer
  5) is assumed, not tested; no fuzzing.
- Channel: stdin/stdout pipes, not the design's inherited
  socketpair/handle; the agent token, endpoint DACL and `getDatabaseKey`
  exposure are not exercised beyond the Unix-socket probe.
- Process-level memory limit (Job Object or RSS sampling), restart backoff,
  crash counting, concurrency limits, `abort` messages: not implemented.
- The design requires the host process to exit after a trap; the spike
  host discards the Store and keeps the process (tests rely on it to show
  the process survives).
- Networking in the broker, consent, grant storage, credentials as handles:
  not implemented.

## Recommendation

Proceed with the design's layering. The parts that carry the most risk
behaved as hoped on macOS: wasmtime's import model gives a clean "no
authority unless linked" boundary, epoch and `StoreLimits` stop runaway
guests without killing the process, per-plugin processes are cheap to
supervise (~13 MiB, ~20 ms to start before precompilation), and a
broker round trip over framed JSON costs tens of microseconds, well below
anything a key press would notice.

Before committing:

1. **Build QuickJS-NG for wasm (S1 proper)** and repeat spike 1 and the
   measurements with it; this is the largest unknown.
2. **Self-applied `sandbox_init` on a signed, hardened, notarized helper**
   with Cranelift (`allow-jit`) versus Pulley (S2/S3). Keep the bisected
   profile and these probe tests as the acceptance suite.
3. **Linux Landlock + seccomp and Windows AppContainer + Job Object**
   prototypes, each passing the same probe table as `sandbox_macos.rs`. The
   probe tests are written so that only the launcher differs per platform.
4. Reset the epoch deadline before any host-initiated re-entry into the
   guest, discard the Store (and, per the design, the process) after any
   trap, and open files relative to a directory handle if a file-system
   capability is ever added.
