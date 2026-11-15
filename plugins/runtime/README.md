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
| `crates/supervisor` (`deckpilot-plugin-supervisor`) | Agent side. Spawns the host with an empty environment, fixed working directory and only stdin/stdout; handshake with sandbox check; per-invocation wall-clock timeout and kill; an abort signal (2 s grace, then kill; not counted as a crash); relays capability requests to the broker session and an executor; restart backoff 1, 2, 4 … 60 s and a crash-loop breaker (5 in 10 min); shutdown with grace period and kill |
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
- `variables.publish {id, value}` and `variables.clear {id?}`: the
  plugin's own variables (design §2.3 item 3), by relative id (1-3
  segments, so no other owner can be named), values at most 16 KiB. Always
  allowed for a plugin with grants, rate limited (`RateLimits::variables`,
  10/s); the agent checks the id and type against the manifest. SDK:
  `Context::publish_variable`, `clear_variable`, `clear_variables`. A host
  or agent that predates them refuses the op.

The supervisor (no protocol change) hands every capability call a
`CallInterrupt` (`CapabilityExecutor::execute_interruptible`, default: the
old `execute`): when the invocation's `AbortSignal` is raised or its
deadline passes, an executor stops the running call at once and its late
result is replaced by `unavailable`; an aborted invocation's later calls
are refused without reaching the broker or the executor, and past the
deadline no queued frame is read (`crates/supervisor/tests/fake_host.rs`,
`running_calls_stop_on_abort_and_deadline`).

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
- The supervisor refuses a host whose `ready` reports `none`
  (`HostConfig::require_sandbox`, on by default) and, with
  `HostConfig::require_full_sandbox` (off by default; the agent sets it),
  anything but `full`. The host is killed before `load` (`SandboxMissing`),
  on every spawn including restarts (`crates/supervisor/tests/fake_host.rs`,
  `full_sandbox_can_be_required`).

## OS sandbox

Applied in two stages before any plugin byte is read: stage 1 while the
process is single-threaded (Linux: `no_new_privs`, `PR_SET_DUMPABLE=0`,
`RLIMIT_CORE=0`, Landlock, all inherited by later threads), then the engine
and its epoch-ticker thread are created, then stage 2 for all threads.

| Platform | Mechanism | Denied (probe tests) |
| --- | --- | --- |
| macOS | `sandbox_init` with `(version 1)(deny default)`, `RLIMIT_CORE=0` | file read, write, directory listing, TCP connect, Unix socket connect, exec |
| Linux | Landlock (best effort up to ABI 9: all file access, TCP, scopes) + seccomp-bpf allowlist with TSYNC (default `EPERM`; no `open*`, `socket`, `connect`, `execve`, `clone`, `ptrace`). Reports `full` when Landlock enforces every requested right (kernel ABI 9, Linux 7.1+), or when the kernel's ABI (probed with `LANDLOCK_CREATE_RULESET_VERSION` before restricting) is exactly 8: the only missing right is V9's `ResolveUnix`, and the allowlist denies `socket`/`connect` (and `open*`/`exec*`) outright, so it is covered (note `landlock ABI 8: ResolveUnix unavailable, covered by seccomp (no socket/connect)`; a unit test fails if the allowlist ever gains one of those calls or V9 adds another right). No Landlock, an ABI below 8 or any other gap: `partial` | the same; Landlock alone is also tested (`EACCES`) |
| Windows | **Launched by the supervisor** (`crates/supervisor/src/windows.rs`, `CreateProcessW` with `STARTUPINFOEXW`): a fresh AppContainer profile per host process with **no capabilities** (`PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`), a supervisor-held Job Object assigned at creation (`PROC_THREAD_ATTRIBUTE_JOB_LIST`: 256 MiB process memory, 1 active process, kill on close, die on unhandled exception, UI restrictions), child process creation restricted (`PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY`), only the two pipe handles inherited (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`), no console (`DETACHED_PROCESS`), environment `SystemRoot` plus the container-redirected `LOCALAPPDATA`/`TEMP`/`TMP`. The host verifies token and job and reports `windows-appcontainer-job`, `full`. Started any other way: own Job Object + Low integrity token, `windows-job-lowil`, `partial` | AppContainer: file read, write and directory listing in `%TEMP%` and the user profile, TCP connect to loopback and to the internet, exec; a system file stays readable (control). Job limits, handle list, UI restrictions and kill on close: see "Sandbox claims proven in CI". Fallback: exec and writes only. Verified on `windows-latest`, CI runs 43645511194 and 43698232584 |

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
  (kill on job close) on Windows, even a host that never reads its stdin
  (`crates/host/tests/parent_death.rs`).
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
| Host runs guests under its own OS sandbox (JIT, traps, epoch) | verified | verified (Landlock ABI 8: `full` with the `ResolveUnix` note) | Windows in the AppContainer: verified, `crates/host/tests/process.rs` and `sdk_e2e.rs` run every host through it (CI run 43645511194; hosts report `windows-appcontainer-job`, `full`) |
| Sandbox probes deny files, network, exec, on main and helper thread | verified | verified (seccomp `EPERM`; Landlock-only run `EACCES`) | Windows AppContainer: verified, `app_container_denies_user_files_network_and_exec` (files in `%TEMP%` and the profile, loopback TCP and exec denied on both threads; `System32` readable; pipes work), CI run 43645511194 |
| clippy `-D warnings`, `fmt --check` | verified | verified | Windows: `clippy --all-targets -D warnings` and `fmt --check` on `windows-latest`, CI run 43645511194 |

### Sandbox claims proven in CI

CI run 43698232584 (head `cb8d079`): `ubuntu-latest` job 132763398640,
`macos-latest` job 132763398725, `windows-latest` job 132763398723. Every
check has a control outside the sandbox that must succeed, so a denial is
the sandbox's doing; none is skipped.

| Claim | Test | Result |
| --- | --- | --- |
| Internet egress denied (not only loopback) | `sandbox.rs` `internet_egress_is_denied`: TCP to `1.1.1.1:443` and `8.8.8.8:443` on both threads. The control connects from outside the sandbox, or the test fails ("no internet access"), so a runner without outbound network cannot pass it | All three OSes. Windows AppContainer: `WSAEACCES` (10013); Linux seccomp and macOS Seatbelt: `EPERM` |
| Only the two pipe handles are inherited (Windows) | `sandbox.rs` `only_the_pipe_handles_are_inherited`: the test creates an inheritable event and an inheritable file handle and passes their values; the probe calls `SetEvent`/`WriteFile` on them. The decisive check is the parent's: its event stays unsignaled and its file empty. Control: a `std::process::Command` child (no handle list) inherits both and signals and writes through the same values | Proven. In the AppContainer strict handle checking is on (`strict_handles=0x3`), so using a value that names no handle there may end the probe with `STATUS_INVALID_HANDLE` (0xC0000008) before it reports; the test accepts that outcome or a reported denial, never a success |
| Process memory limit (256 MiB) applies to native allocations | `sandbox.rs` `process_memory_limit_stops_native_allocations`: inside, 192 MiB succeed and 320 MiB fail (`OutOfMemory`) on both threads; started directly, 320 MiB succeed | Proven (Windows) |
| One active process (job limit), on its own | `process.rs` `host_runs_alone_in_the_supervisors_job`: a second (suspended) process assigned to the host's job from outside is refused with `ERROR_NOT_ENOUGH_QUOTA` (1816); control: a fresh job without limits accepts it. This does not go through the child process policy | Proven (Windows) |
| Child process policy, on its own | `sandbox.rs` `supervisor_job_and_child_policy_apply`: `GetProcessMitigationPolicy` reports `NoChildProcessCreation` (control: not set when started directly); a raw `CreateProcessW` (nothing inherited, no console) inside fails with `ERROR_CHILD_PROCESS_BLOCKED` (367), the policy's error, not the job limit's 1816 | Proven (Windows). The older `exec` probe goes through `std::process::Command`, which fails earlier, opening `NUL` for the child's stdin; the raw probe reaches `CreateProcessW` |
| Which job applies | Host report notes: `job (supervisor): ...` when the supervisor's job was verified, `job (own, supervisor job not verified: ...)` when the host had to add its own. `process.rs` (`host_starts_sandboxed_and_serves_invocations`, `host_runs_alone_in_the_supervisors_job`) and every new AppContainer probe test require the former. From the outside, `IsProcessInJob` with the supervisor's job handle (`HostProcess::job`) is true and the job has 1 active and 1 total process; from the inside, the innermost job has exactly the supervisor's limit flags (0x2508), memory limit, all eight UI restrictions (0xff) and 1 total process | Proven (Windows) |
| UI restrictions | `sandbox.rs` `clipboard_atoms_and_desktops_are_out_of_reach`: inside, clipboard read and write fail (`OpenClipboard`: access denied) and the clipboard is unchanged when checked from outside; a global atom the test added is not found; an atom added inside is not visible outside (the job has its own atom table); `CreateDesktopW` is denied. Controls: started directly, the probe reads and replaces the clipboard, finds the atom and creates a desktop | Proven (Windows). The denials are those of the container (AppContainer and job together); the test does not tell the two layers apart |
| Host dies with its agent | `parent_death.rs`: a helper "agent" starts a host through the supervisor and is killed outright; the host must be gone within 5 s. Windows: the sandbox probe in `sleep` mode (never reads stdin, so only kill on job close can end it) and the real host. macOS and Linux: the real, idle host (stdin end-of-file) | Proven on all three OSes, gone within microseconds of the kill being reaped |
| No extra inherited descriptors (macOS, Linux) | `process.rs` `spawned_host_gets_no_environment_and_no_extra_descriptors` (existing) | Proven |

Not provable this way, and why:

- macOS and Linux have no kill-on-close: a host that is not reading
  its stdin (stuck in native code) outlives the agent until it next reads.
  `parent_death.rs` therefore tests the idle host only there; the sleeping
  probe is used on Windows alone, because elsewhere it would survive.
- The job's UI restrictions cannot be told apart from the AppContainer's
  own UI isolation for the same process without launching the host
  differently (a weakened launch path), which this change does not add.
- Memory limit, handle list, job and UI checks are Windows-only claims;
  macOS and Linux do not make them (see below).

**NOT VERIFIED / not implemented:**

- Windows AppContainer isolation is verified only on GitHub's
  `windows-latest` runner (Windows Server 2025); no
  desktop Windows edition, no per-user or `Program Files` install, and no
  machine with a third-party firewall has been tried. Loopback/network
  denial relies on the Windows Filtering Platform AppContainer rules; the
  CI job prints the firewall state next to the probe results. On macOS and
  Linux the host still exits on stdin EOF only, which needs it to be
  responsive (tested for an idle host, see above).
- Signed, hardened-runtime macOS helper (`allow-jit`), notarization,
  Pulley; the Seatbelt interface is deprecated and undocumented.
- Linux `full` on kernels with Landlock ABI 8 rests on seccomp for
  pathname UNIX sockets (`ResolveUnix` arrives with ABI 9, Linux 7.1); it is
  verified on Docker Desktop's 7.0 linuxkit kernel (aarch64, local) and on
  GitHub's `ubuntu-latest` (image ubuntu-26.04, kernel `7.0.0-1019-azure`,
  x86_64, ABI 8: plugin runtime CI run 43689909312, job 132738174558, host
  reports `full` with the `ResolveUnix` note; agent end to end run
  43689909295, job 132738173732). Kernels with ABI 9 have not been tried.
- Linux seccomp allowlist on distributions other than the Debian-based
  `rust:1` image and CI's Ubuntu; other libc versions may need more
  syscalls (they then fail with `EPERM`, not silently succeed).
- Process memory limit on macOS and Linux (the design samples RSS in the
  agent); concurrency limits and `abort` messages; the agent-side network
  executor (the broker only decides; redirects and resolved addresses must be
  checked with `authorize_redirect` and `check_connect_addr`).
- The agent-side credential store and secret injection (the broker only
  decides whether a `credential` handle may go to an origin). The sample is
  packaged as a `.dpplugin` with the module as `plugin.wasm`
  (`docs/marketplace/artifact-contract.md`).
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
