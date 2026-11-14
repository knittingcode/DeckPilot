# Plugin runtime

Foundation of the plugin runtime from `docs/marketplace/architecture.md`
(§3 runtime, §3.8 host protocol, §5 broker, §16 limitations), built on the
spike in `experiments/marketplace-spikes/runtime/`.

A standalone Cargo workspace with its own `Cargo.lock`. It is **not** a
member of the repository's root workspace: the app, agent and release builds
do not compile it. `.github/workflows/plugin-runtime.yml` tests it on Linux,
macOS and Windows (not a required check).

```sh
rustup target add wasm32-unknown-unknown   # once, for the pinned toolchain
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

## Crates

| Crate | Responsibility |
| --- | --- |
| `crates/abi` (`deckpilot-plugin-abi`) | ABI 1. Guest exports and imports (`dp_abi_version`, `dp_alloc`, `dp_invoke`; the single import `deckpilot_v1.host_call`); host protocol frames (4-byte big-endian length + JSON) and messages; typed parameters and outcomes; closed set of capability requests; every size limit. Strict serde: unknown fields are rejected, no message carries a plugin id |
| `crates/broker` (`deckpilot-plugin-broker`) | Pure decision logic. Grants derived from approved permissions (exact `https` origins, never wildcards or IP literals; file system, process and clipboard never grantable); `Session` per channel as the only identity; grants read per call (revocation applies to the next call); redirects re-checked; SSRF address policy for the network layer; token-bucket rate limits; audit records without values |
| `crates/host` (`deckpilot-plugin-host`) | The host process. Sandboxes itself, receives the module bytes over stdin (opens no file), runs one wasmtime Store with no WASI, epoch deadline, fuel, `StoreLimits` (memory, tables, instances), bounded wasm stack, and exits non-zero after any trap. Also `deckpilot-sandbox-probe` (test only, not shipped) |
| `crates/supervisor` (`deckpilot-plugin-supervisor`) | Agent side. Spawns the host with an empty environment, fixed working directory and only stdin/stdout; handshake with sandbox check; per-invocation wall-clock timeout and kill; relays capability requests to the broker session and an executor; restart backoff 1, 2, 4 … 60 s and a crash-loop breaker (5 in 10 min); shutdown with grace period and kill |
| `guest/sdk` (`deckpilot-plugin-guest`) | Minimal Rust guest SDK for `wasm32-unknown-unknown`: `export_plugin!`, `Context::call` |
| `guest/test-guest` | Test guest: normal actions and deliberate attacks. Built from source by `crates/test-fixtures/build.rs` (no committed binaries) |

## Fixes relative to the spike

- The host **exits after any trap**, deadline, fuel exhaustion or guest ABI
  violation (`fatal` frame, exit codes 10-13); the Store is never reused.
- The epoch deadline is reset after every `host_call` **before** re-entering
  the guest (`dp_alloc`), and before every entry from the host.
- **No file access in the host at all**: the module arrives over the pipe,
  so the TOCTOU issue of path-based checks does not arise. There is no file
  capability for plugins (#191), hence no need for cap-std/`openat2`.
- The OS sandbox is applied by the host to itself (`sandbox_init` instead of
  `sandbox-exec`), plus Linux and Windows implementations.

## OS sandbox

Applied in two stages before any plugin byte is read: stage 1 while the
process is single-threaded (Linux: `no_new_privs`, `PR_SET_DUMPABLE=0`,
`RLIMIT_CORE=0`, Landlock, all inherited by later threads), then the engine
and its epoch-ticker thread are created, then stage 2 for all threads.

| Platform | Mechanism | Denied (probe tests) |
| --- | --- | --- |
| macOS | `sandbox_init` with `(version 1)(deny default)`, `RLIMIT_CORE=0` | file read, write, directory listing, TCP connect, Unix socket connect, exec |
| Linux | Landlock (best effort up to ABI 9: all file access, TCP, scopes) + seccomp-bpf allowlist with TSYNC (default `EPERM`; no `open*`, `socket`, `connect`, `execve`, `clone`, `ptrace`) | the same; Landlock alone is also tested (`EACCES`) |
| Windows | Job Object (256 MiB process memory, 1 active process, kill on close, die on unhandled exception, UI restrictions) + Low integrity token | exec, writes to the user's (Medium-integrity) files. **Not denied: file reads and network** (needs AppContainer, not implemented) |

## Verification status

| Claim | macOS 27.2 arm64 (local) | Linux aarch64 (Docker Desktop, kernel 7.0 linuxkit, local) | Linux x86_64, Windows |
| --- | --- | --- | --- |
| All tests pass (`cargo test --locked`) | verified | verified | CI workflow; not yet run |
| Host runs guests under its own OS sandbox (JIT, traps, epoch) | verified | verified | CI |
| Sandbox probes deny files, network, exec, on main and helper thread | verified | verified (seccomp `EPERM`; Landlock-only run `EACCES`) | Windows: exec and write only |
| clippy `-D warnings`, `fmt --check` | verified | verified | Windows: type-checked with clippy locally for `x86_64-pc-windows-msvc`, not run |

**NOT VERIFIED / not implemented:**

- Windows AppContainer/LPAC (file reads and network are not denied on
  Windows); a supervisor-held Job Object so that the host dies with the
  agent (today the host exits on stdin EOF, which needs the host to be
  responsive).
- Signed, hardened-runtime macOS helper (`allow-jit`), notarization,
  Pulley; the Seatbelt interface is deprecated and undocumented.
- Linux seccomp allowlist on distributions other than the Debian-based
  `rust:1` image and CI's Ubuntu; other libc versions may need more
  syscalls (they then fail with `EPERM`, not silently succeed).
- Process memory limit on macOS and Linux (the design samples RSS in the
  agent); concurrency limits and `abort` messages; the agent-side network
  executor (the broker only decides; redirects and resolved addresses must be
  checked with `authorize_redirect` and `check_connect_addr`).
- Compile time of a hostile module is bounded only by the supervisor's
  `load_timeout` (kill), not inside the host.
- wasmtime's own freedom from escape bugs (assumed, not tested; no fuzzing).

## QuickJS

Time-boxed check (outside this repository, in a scratch directory):
`rquickjs` 0.14.0 (QuickJS-NG) with `default-features = false` builds for
`wasm32-wasip1` with the pinned toolchain after `rustup target add
wasm32-wasip1`. Its build script **downloads wasi-sdk with `curl` from the
official `github.com/WebAssembly/wasi-sdk` releases, without checksum
verification**; a production build must instead point `WASI_SDK` at a
pinned, hash-verified wasi-sdk.

Result (macOS arm64, release): module 788 KB; imports exactly eight WASI
functions (`environ_get`, `environ_sizes_get`, `clock_time_get`, `fd_close`,
`fd_fdstat_get`, `fd_seek`, `fd_write`, `proc_exit`). With deny-stubs for
those eight (no WASI implementation), wasmtime 50.0.1 compiled it in 517 ms
and a fresh instance + QuickJS runtime + evaluation of a 1,000-iteration loop
took 0.25-0.5 ms, 1.3 MiB linear memory, 3.7 M fuel.

Not done: the engine is not wired into this host, which rejects every WASI
import. Next step: an engine guest built on `guest/sdk` whose eight WASI
imports are satisfied by host deny-stubs (or a libc shim at link time),
precompiled at build time. The ABI is engine-agnostic: `dp_invoke` takes an
action and typed parameters, whatever runs inside.
