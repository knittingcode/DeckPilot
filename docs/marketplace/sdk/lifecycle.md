# Lifecycle

What runs when, in ABI 1. Short version: a plugin has **two optional
hooks, init and shutdown, neither of which can use capabilities**, and
otherwise runs only when one of its actions is invoked.

## The host and the module

- The agent starts one sandboxed `deckpilot-plugin-host` process per
  enabled plugin **on demand**, at the first invocation, not at enable.
  Before every spawn it re-verifies the installed files against their
  integrity record and re-validates `plugin.wasm`.
- The host instantiates the module once and keeps it (one wasmtime Store)
  for the life of the process: statics and the `Plugin` built by
  `plugin!` persist between invocations of that host, until it stops.
  Do not rely on that for anything that must survive; use storage.
- Invocations of one plugin run one at a time, in order, on its *lane*
  (a queue of 8; more fail at once with `busy`). Different plugins run in
  parallel.

## Hooks

| Hook | SDK | Export | When | Can |
| --- | --- | --- | --- | --- |
| init | `Plugin::on_init(fn() -> Result<(), PluginError>)` | `dp_init` (0 ready, 1 hook failed, 2 invalid definition) | Once, when the host loads the module, before the first invocation. A host that predates `dp_init` never calls it; the SDK then runs init before the first invocation | Check static tables, prepare in-memory state. **No capabilities**: a `host_call` inside a hook gets `unavailable` locally. A failure refuses the load: the invocation fails and the next one tries a fresh host (with backoff) |
| shutdown | `Plugin::on_shutdown(fn())` | `dp_shutdown` | On a clean stop: disable, update, approve, rollback, uninstall, quarantine, crash loop, agent lock and stop. The host gets `shutdown`, runs the hook, and is killed after 1 s | Best effort, no capabilities. Not run after a crash, a timeout or a kill |

Both run under the same limits as an invocation (fuel, CPU deadline).
`plugins/examples/hello-weather` has an init hook that checks its city
table.

## An invocation

1. The agent checks state, platform, verdict and parameters
   (`invalid-parameters` before the host runs).
2. The host writes the input into guest memory (`dp_alloc`), calls
   `dp_invoke`; the SDK runs init if needed, finds the action, checks the
   parameters, calls the handler, checks the result.
3. Every capability call goes host → agent broker → executor and back,
   synchronously; the guest waits.
4. Limits (`GuestLimits::default`, `HostConfig`):

   | Limit | Value | Exceeded |
   | --- | --- | --- |
   | Wall clock | the action's `node.timeoutMs` (default 30 s, at most 10 min) | host killed; the action fails (`plugin-host-failed`, "took longer than its time limit") |
   | CPU between two host calls | 250 ms (epoch deadline) | trap; host exits |
   | Fuel | 5,000,000,000 per invocation (≈ instructions) | trap; host exits |
   | Linear memory | 64 MiB | allocation fails or trap |
   | Wasm stack | 512 KiB | trap |
   | Capability calls | 256 | SDK refuses the rest (`throttled`) |
   | Input / output | 1 MiB / 192 KiB of JSON | refused |

5. **Cancellation.** A cancelled run (for example a cancelled workflow)
   or one past its deadline raises the invocation's abort
   signal: the capability call in progress stops at once (its result
   becomes `unavailable`), later calls are refused without reaching the
   broker, and the result of the invocation is discarded; the action fails
   as `cancelled`. ABI 1 has no abort message for the guest, so the host
   gets 2 s to finish, then it is killed (not counted as a crash). Write
   handlers that make their capability calls early and return promptly
   after a failed call.
6. **Revocation during a run.** Grants are read on every call, so a revoke
   (or disable, or uninstall) makes the **next** call of the same
   invocation fail with `not-permitted`; a call already running is not
   interrupted by a revoke.

## Crashes

A panic aborts the module (`panic = "abort"` in the `guest` profile): the
host exits, the action fails (`plugin-host-failed` in keys and workflows, the IPC reason `hostFailed` for `invokePluginAction`), and the next
invocation starts a fresh host with backoff 1, 2, 4 … 60 s. Five crashes
in ten minutes disable the plugin (crash-loop breaker). Return
`PluginError`s instead of panicking; avoid `unwrap` on anything an input
can influence.

## Plugin state across events

| Event | Host | Storage | Settings, credentials | Published variables | Grants |
| --- | --- | --- | --- | --- | --- |
| Agent restart, lock/unlock | stopped | kept | kept | dropped | kept |
| Disable / enable | stopped | kept | kept | dropped | kept, inactive while disabled |
| Revoke / grant | running | kept | kept | kept | next call |
| Update, approve, rollback | stopped | kept | kept | dropped | escalations need consent |
| Host crash | restarted on demand | kept | kept | kept | kept |
| Uninstall | stopped | deleted | deleted unless `keep` | dropped | deleted |

## Not in ABI 1

Said plainly, because the design (#191, and the JavaScript declarations
in `plugins/sdk-js`) describes more:

- **No activate/deactivate with capabilities.** `onActivate` and
  `onDeactivate` of the design map to `dp_init`/`dp_shutdown`, which have
  no capabilities and no budget of their own.
- **No background work**: no timers, no intervals, no `background`
  permission in effect. A plugin runs only while one of its actions runs.
- **No `settingsChanged`**: read `Context::settings()` in each
  invocation.
- **No button state** (`state` in the manifest validates, but nothing
  pulls it) and no device access.
- **No invocation context**: a handler does not learn whether a key, a
  workflow or the app started it, nor its deadline or attempt number.
- **No progress reports** and no `sleep`. The time is a capability
  ([`Context::now`](capabilities.md#time), plugin API 1.1), not a clock
  the plugin can wait on.
