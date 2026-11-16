# macOS: the plugin host under App Sandbox and the hardened runtime

Questions ([capability matrix](../../../docs/distribution/capability-matrix.md),
sections 2.6 and 2.7):

1. Can a sandboxed process spawn a helper, and does the helper inherit the
   sandbox?
2. Does wasmtime 50's Cranelift JIT run under the hardened runtime, and
   with which entitlement: `com.apple.security.cs.allow-jit` (which only
   covers `MAP_JIT` memory) or `com.apple.security.cs.allow-unsigned-executable-memory`?
3. Can the plugin host's self-applied Seatbelt profile
   (`sandbox_init`, deny default, as in
   `plugins/runtime/crates/host/src/sandbox/macos.rs`) be applied inside the
   App Sandbox?

Run: `tools/dev/target-slot.sh run -- experiments/distribution/macos-sandbox-plugin-host/run.sh <work dir>`.
The probe (`src/main.rs`) does what the host does at start-up: it builds a
Cranelift engine (wasmtime 50.0.1 with the host's features), compiles and
runs a small module (`add(40, 2)`), applies `sandbox_init` with the host's
profile, and runs the module again. `run.sh` signs it ad hoc in the
variants below and writes one report each to `results/`. Experiment bundle
identifiers only (`io.github.knittingcode.deckpilot.experiment.*`), no
Apple account, nothing installed.

## Result (2026-11-16, macOS 27.2 (26B123), arm64)

| Variant | Child signature | Sandboxed | JIT | `sandbox_init` inside |
| --- | --- | --- | --- | --- |
| `direct-plain` | no hardened runtime | no | ok | ok (then `/` denied, JIT still ok) |
| `direct-hardened` | hardened runtime | no | **killed** (`SIGKILL`, code signature invalid) | not reached |
| `direct-hardened-allow-jit` | hardened runtime + `allow-jit` | no | **killed** (`SIGKILL`, code signature invalid) | not reached |
| `direct-hardened-unsigned` | hardened runtime + `allow-unsigned-executable-memory` | no | ok | ok (then `/` denied, JIT still ok) |
| `sandboxed-parent-child-inherit` | `app-sandbox` + `inherit`, hardened runtime | yes, inherited | **killed** | not reached |
| `sandboxed-parent-child-inherit-allow-jit` | as above + `allow-jit` | yes, inherited | **killed** | not reached |
| `sandboxed-parent-child-inherit-unsigned` | as above + `allow-unsigned-executable-memory` | yes, inherited | ok | **failed**: `rc -1: Operation not permitted` |
| `sandboxed-parent-child-inherit-no-runtime` | `app-sandbox` + `inherit`, no hardened runtime | yes, inherited | ok | **failed**: `rc -1: Operation not permitted` |

In every sandboxed variant the parent (an `.app` with `app-sandbox`
started by LaunchServices) spawned the child with `std::process::Command`
without error, and the child reported the parent's container in
`APP_SANDBOX_CONTAINER_ID` and was denied the real `~/Library/Preferences`:
it inherited the sandbox.

Why the hardened-runtime runs die: the crash report
([excerpt](results/crash-direct-hardened-allow-jit.txt)) is
`EXC_BAD_ACCESS`, `SIGKILL (Code Signature Invalid)`, termination
namespace `CODESIGNING`, indicator `Invalid Page`, at an address in an
untagged `r-x/rwx` region. wasmtime maps its code with `mmap` and turns it
executable with `mprotect` (`runtime/code_memory.rs`); `MAP_JIT` and
`pthread_jit_write_protect_np` occur nowhere in the wasmtime 50.0.1
sources. `allow-jit` only
permits `MAP_JIT` regions, so it does not help;
`allow-unsigned-executable-memory` does.

## Conclusions

- **Spawning a helper from the App Sandbox: works**, and the helper is
  sandboxed with the parent's container (with `com.apple.security.inherit`).
- **JIT on the GitHub (Developer ID, hardened runtime) build:** the plugin
  host needs `com.apple.security.cs.allow-unsigned-executable-memory`;
  `allow-jit` is **not enough** for wasmtime 50 as configured by the host.
  (Alternatives: wasmtime's Pulley interpreter, or a wasmtime change to
  `MAP_JIT` with `pthread_jit_write_protect_np`; neither was tested.) The
  entitlement must stay on the host binary only.
- **JIT on a Mac App Store build:** works in a sandboxed helper without the
  hardened runtime (which the Mac App Store does not require), and with it
  plus `allow-unsigned-executable-memory`. Whether App Review accepts that
  entitlement was not tested.
- **The host's own Seatbelt profile inside the App Sandbox: refused**
  (`sandbox_init` returns `-1`, `EPERM`). The host as designed would report
  no `full` sandbox and plugins would fail closed. The matrix's "Blocked
  as designed" is confirmed. An XPC service with its own minimal App
  Sandbox entitlements remains the untested alternative.

Not shown: the real plugin host binary and protocol (only its engine
settings and profile were reproduced), an Apple Distribution signature, an
XPC service, and App Review.
