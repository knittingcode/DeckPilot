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
| `guest/sdk` (`deckpilot-plugin-guest`) | Raw guest ABI bindings for `wasm32-unknown-unknown`: `export_plugin!`, `Context::call`. Plugin authors use `deckpilot-plugin-sdk` instead |
| `guest/plugin-sdk` (`deckpilot-plugin-sdk`, own semver, 0.1.0) | The Rust plugin SDK (plugin API 1.0 on ABI 1): `Plugin`/`Action`/`Param` builders and `plugin!`; typed parameters (`text`, `integer`, `number`, `boolean`, `choice`, `secret` handle) checked before the handler; `Output`/`PluginError` with shared codes; brokered `Context` (HTTP to an exact origin, credentials as handles, notifications, storage, log) with explanatory `CapabilityError`s; init/shutdown hooks; `testing::MockHost` for native tests. No `unsafe` in plugin code |
| `guest/test-guest` | Test guest: normal actions and deliberate attacks on the raw ABI |
| `guest/sdk-test-guest` | Test guest built with the SDK: lifecycle, secret handles, numbers, panic, spin, slow host calls |
| `../examples/hello-weather` | The sample plugin (see its README): a pure action and a brokered HTTPS action, `manifest.json` valid against the #191 schema (copy in `plugins/schemas/`) |

The wasm guests and the sample are built from source by
`crates/test-fixtures/build.rs` in one locked cargo invocation with the
`guest` profile and remapped source paths (no committed binaries).
`crates/host/tests/sdk_e2e.rs` runs the sample and the SDK test guest in the
real sandboxed host under the real supervisor and broker, with a stub
network executor (decisions only, no network).

## ABI 1 additions (compatible)

Each addition is optional, so original ABI 1 modules and messages are
unchanged byte for byte (tested in `crates/abi` and `crates/host/tests/lifecycle.rs`):

- Optional guest exports `dp_describe` (descriptor: SDK, API level, actions
  and parameter kinds, no plugin id), `dp_init` (0 = ready, otherwise the
  load fails) and `dp_shutdown`. They run under the invocation limits; a
  `host_call` inside them gets `unavailable` locally. A host that predates
  them never calls them, and the SDK then runs init before the first
  invocation.
- `loaded` carries the validated descriptor; `HostConfig::expect_actions`
  makes the supervisor refuse a module whose action ids differ from the
  manifest's (design §6.1).
- `http.fetch` takes an optional `auth` (credential slot key and scheme,
  never a secret); the broker allows it only for a granted `credential`
  slot, its origins and schemes, and never across a redirect to another
  origin. `HttpResponse` documents the executor's reply shape.

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
| Windows | **Launched by the supervisor** (`crates/supervisor/src/windows.rs`, `CreateProcessW` with `STARTUPINFOEXW`): a fresh AppContainer profile per host process with **no capabilities** (`PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`), a supervisor-held Job Object assigned at creation (`PROC_THREAD_ATTRIBUTE_JOB_LIST`: 256 MiB process memory, 1 active process, kill on close, die on unhandled exception, UI restrictions), child process creation restricted (`PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY`), only the two pipe handles inherited (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`), no console (`DETACHED_PROCESS`), environment `SystemRoot` plus the container-redirected `LOCALAPPDATA`/`TEMP`/`TMP`. The host verifies token and job and reports `windows-appcontainer-job`, `full`. Started any other way: own Job Object + Low integrity token, `windows-job-lowil`, `partial` | AppContainer: file read, write and directory listing in `%TEMP%` and the user profile, TCP connect to loopback, exec; a system file stays readable (control). Fallback: exec and writes only. Verified on `windows-latest`, CI run 43645511194 |

Windows design notes and trade-offs:

- AppContainer rather than a restricted token: a restricted token plus Low
  integrity still reads everything the user can read and opens network
  connections; an AppContainer without capabilities denies both by default
  (user files carry no ACE for the container or `ALL APPLICATION
  PACKAGES`; loopback is blocked for every AppContainer). Not LPAC: the
  less privileged variant would also lose the `ALL APPLICATION PACKAGES`
  read access to `System32` and `Program Files` that the host needs to load.
- Profiles: one per host process (`DeckPilot.PluginHost.<agent pid>.<n>`),
  deleted with `DeleteAppContainerProfile` after the process exits. A
  stable per-install name was rejected because processes in the same
  AppContainer can open each other (one plugin's host could reach
  another's) and its private storage would persist. A profile left by a
  crashed agent is removed when its name comes round again; otherwise it
  stays (empty) until removed.
- The container must read and execute the host binary. Under `Program
  Files` it already can; elsewhere the supervisor adds an `ALL APPLICATION
  PACKAGES` read/execute ACE **to the host executable only** (best effort,
  once per path). The working directory is `System32`.
- The two child pipe ends are inheritable for the duration of the spawn; a
  `std::process::Command` started by another agent thread in that window
  would inherit them too (only delaying end-of-file; no access to the host).
- Because the supervisor holds the job, the host dies when the agent does
  (kill on job close) on Windows.
- No console: the host is created with `DETACHED_PROCESS`. With a console
  (even `CREATE_NO_WINDOW`) Windows starts a `conhost.exe` for the new
  process, which the child process policy refuses, and the host died with
  `STATUS_DLL_INIT_FAILED` (0xC0000142) before `main`. All UI
  restrictions, the job limits and the child process policy stay as they
  are; a CI diagnosis toggling each of them showed the console was the
  only cause (CI run 43644332319).

## Verification status

| Claim | macOS 27.2 arm64 (local) | Linux aarch64 (Docker Desktop, kernel 7.0 linuxkit, local) | Linux x86_64, Windows |
| --- | --- | --- | --- |
| All tests pass (`cargo test --locked`) | verified | verified | Linux x86_64: CI run 43626042263. Windows (`windows-latest`, AppContainer launch and the Job Object/Low integrity fallback): CI run 43645511194 |
| Host runs guests under its own OS sandbox (JIT, traps, epoch) | verified | verified | Windows in the AppContainer: verified, `crates/host/tests/process.rs` and `sdk_e2e.rs` run every host through it (CI run 43645511194; hosts report `windows-appcontainer-job`, `full`) |
| Sandbox probes deny files, network, exec, on main and helper thread | verified | verified (seccomp `EPERM`; Landlock-only run `EACCES`) | Windows AppContainer: verified, `app_container_denies_user_files_network_and_exec` (files in `%TEMP%` and the profile, loopback TCP and exec denied on both threads; `System32` readable; pipes work), CI run 43645511194 |
| clippy `-D warnings`, `fmt --check` | verified | verified | Windows: `clippy --all-targets -D warnings` and `fmt --check` on `windows-latest`, CI run 43645511194 |

**NOT VERIFIED / not implemented:**

- Windows AppContainer isolation is verified only on GitHub's
  `windows-latest` runner (Windows Server 2025); no
  desktop Windows edition, no per-user or `Program Files` install, and no
  machine with a third-party firewall has been tried. Loopback/network
  denial relies on the Windows Filtering Platform AppContainer rules; the
  CI job prints the firewall state next to the probe results. The
  supervisor-held job (host dies with the agent) is not tested on its
  own. On macOS and Linux the
  host still exits on stdin EOF only, which needs it to be responsive.
- Signed, hardened-runtime macOS helper (`allow-jit`), notarization,
  Pulley; the Seatbelt interface is deprecated and undocumented.
- Linux seccomp allowlist on distributions other than the Debian-based
  `rust:1` image and CI's Ubuntu; other libc versions may need more
  syscalls (they then fail with `EPERM`, not silently succeed).
- Process memory limit on macOS and Linux (the design samples RSS in the
  agent); concurrency limits and `abort` messages; the agent-side network
  executor (the broker only decides; redirects and resolved addresses must be
  checked with `authorize_redirect` and `check_connect_addr`).
- The agent-side credential store and secret injection (the broker only
  decides whether a `credential` handle may go to an origin); packaging the
  sample as a `.dpplugin` (the #209 package format carries a JS bundle,
  `plugin.js`, not a wasm module).
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
