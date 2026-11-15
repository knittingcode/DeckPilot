# Plugins in the agent (experimental backend)

Status: **experimental, off in every release build.** The agent's cargo
feature `plugins` (default off) adds the plugin backend; even then plugins
stay off until the agent is started with `--enable-plugins`. Release
workflows are unchanged. `apps/agent/tests/plugins_feature.rs` checks that
the default build depends on no plugin crate, and that no build of the
agent links the plugin host or wasmtime.

The executable artifact is an ABI 1 WebAssembly module
([artifact-contract.md](artifact-contract.md)).

## Architecture

```text
configuration app ──IPC protocol 10──▶ agent service task ◀── keys, workflows
                                          │  plugin requests and plugin
                                          │  actions (never run here)
                                          ▼
                                   plugin worker thread ──▶ one lane thread
                                                             per plugin host
       ┌──────────────┬──────────────┬────┴─────────┬────────────────────┐
  PluginManager   LocalRegistry   package        Broker            Supervisor ×N
  (plugins.db,    (RegistryClient verification   (grants per call, (one per running
  crash-safe      over a local    (pins, manifest, redirects, rate  plugin; spawns
  lifecycle)      directory)      plugin.wasm)    limits, audit)     the host)
                                                      ▲                 │ stdin/stdout
                                         Executor ────┘                 ▼
                             (HTTP module client,               deckpilot-plugin-host
                              notifications, log)               (OS sandbox + wasmtime,
                                                                 no WASI)
```

- **Feature gate.** `apps/agent/src/plugins/mod.rs` is always compiled; it
  answers plugin requests with *plugins not available* unless the feature
  is on and plugins are enabled. The rest of `apps/agent/src/plugins/` is
  `cfg(feature = "plugins")`. The runtime crates (`abi`, `broker`,
  `supervisor`) are path dependencies into the separate `plugins/runtime`
  workspace, which the root workspace `exclude`s; the host and wasmtime
  are never dependencies of the agent.
- **Worker.** Plugin requests go from the service task to one worker thread
  (`worker.rs`), so verification and installs never stall devices or IPC;
  invocations are prepared there and run on the plugin's lane thread
  (`lane.rs`), which owns its supervisor. The worker owns the store while
  the configuration is unlocked: unlocking opens it, locking stops every
  lane and host and closes it.
- **plugins.db key.** HKDF-SHA256 of the configuration database key, salt
  `deckpilot.plugin-store.v1`, info `deckpilot plugins.db sqlcipher key`
  (`plugins/key.rs`). No keychain item; the configuration key is never
  handed to the plugin store. When the agent switches the configuration to
  a system-managed key it re-encrypts plugins.db (`Database::rekey`). A
  password change made by the app while the agent is locked leaves
  plugins.db under the old key: the agent then reports the store as
  unavailable (`storeUnavailable`) and never touches the file.
- **Registry.** `--plugin-registry <dir>` (the publishing tool's layout:
  `site/v1/`, `releases/`) read through a `RegistrySource`; the root keys
  come separately from `--plugin-registry-roots <roots.json>` (they stand
  for the roots a release will embed). The verified state is kept in
  `<data dir>/plugin-registry/`. No network fetching yet.
- **Install.** Registry refresh (key sets, timestamp, index), revocations,
  `resolve_install` (compatibility, pins), package read from `releases/`
  (or a local file, accepted only if its bytes are what the registry
  pins), `verify_package` (sha256, size, manifest hash, permissions,
  artifact contract incl. the static `plugin.wasm` check), then the
  store's journaled install. A crash at any step is completed or undone by
  `PluginManager::recover` at the next start.
- **Grants.** Stored per permission key in plugins.db. The broker gets the
  effective grants (declared ∩ granted, nothing unless enabled) after every
  change; a revocation applies to the next capability call. Wildcard hosts
  and `privateNetwork` are never passed to the broker (exact origins only).
- **Execution.** Only in the host, through the supervisor: the active
  version's files are re-verified against their integrity record and the
  module re-validated before the host is spawned; the supervisor checks
  the host's sandbox report (the agent requires `full`, see the platform
  gate), compares the module's action ids with the
  manifest's, applies the action's `timeoutMs` (at most 10 min), restarts
  after crashes with backoff and disables the plugin after a crash loop.
- **Capability executor** (`executor.rs`). `http.fetch`: the HTTP module's
  client with `allow_private = false` (every resolved address must be
  public; the connection uses only the checked addresses), no proxy,
  15 s, 512 KiB, redirects never followed by the client: each one goes back
  to `Session::authorize_redirect`. Response headers are filtered to a safe
  list. `notify` becomes an agent notification titled with the plugin's
  name; `log` goes to the agent log. Storage, credential injection and
  settings: see [Plugin data](#plugin-data-storage-credentials-settings).
- **Platform gate.** Enabling and invoking are refused on platforms
  without a host sandbox (`platformUnsupported`): only macOS, Linux and
  Windows may run plugins; elsewhere installing and inspecting work.
- **Full sandbox only.** On every platform, every host the agent starts
  (and every restart) must report `enforcement: full` in its `ready`
  message: macOS Seatbelt; Linux seccomp with Landlock (every right up to
  ABI 9, or ABI 8, where the one missing right, `ResolveUnix`, is covered
  by seccomp denying `socket` and `connect`; see plugins/runtime/README.md);
  Windows `windows-appcontainer-job` (the AppContainer and Job Object the
  supervisor launches it in). A host reporting `partial` (Linux without
  Landlock or with an ABI below 8; Windows `windows-job-lowil`, which does
  not deny reading user files or the network) or `none` is killed before it receives the module
  (`HostConfig::require_full_sandbox`, checked in the supervisor's
  handshake so no restart can skip it). The invocation fails with
  `hostFailed` and a message naming the mechanism and level; the
  supervisor is dropped, so the refusal neither backs off nor counts
  towards a crash loop, and the next invocation checks a fresh host.
- **Actions.** Each manifest action is exposed as action type
  `<plugin id>.<action>` with `available` and `unavailableReason` (disabled,
  broken, quarantined, platform, or a declared permission that is not
  granted, e.g. revoked after enabling).
- **Actions in the engine** (design §2.3 items 1, 2, 7 and §3.7). A plugin
  id is a module id (`ModuleId` accepts exactly one `.`; built-in ids never
  have one) and `ModuleId::owner_of` takes everything before the **last**
  `.`, so `acme.hello-weather.greet` belongs to `acme.hello-weather`. After
  every change of the installed plugins (install, enable, disable, grant,
  revoke, update, rollback, uninstall, quarantine, a crash loop, lock and
  unlock) the worker publishes them and the agent registers one
  `PluginModule` (`plugins/engine.rs`) per installed plugin: enabled when
  its actions can run, otherwise disabled with the reason above, which keys,
  `runAction` and workflows report. Its state comes from plugins.db, never
  from the `modules` table, and it has no capabilities, settings, pages,
  windows, variables or migrations. Each action is a `PluginActionHandler`:
  parameters and the node contract convert one to one from the manifest
  (`cancellable` always, never `changesContext`); outputs are the declared
  ones, as text.
  - *Never on the service task.* The handler `runs_detached`: a key bound
    to it makes the runtime return `RuntimeOutcome::ActionRequested`, and
    the agent runs `Runtime::detached_action` on a thread of its own (at
    most 16 at once), then reports through `actionCompleted` and, for
    `runAction`, answers the request when it finished. Workflow nodes call
    the handler on their execution threads.
  - *Dispatch.* The handler sends the invocation to the worker, which
    checks state, verdict and grants and queues it on the plugin's **lane**
    (`plugins/lane.rs`): one thread per host that owns its supervisor, with
    a queue of 8 (more fail at once with `busy`). `invokePluginAction` uses
    the same lanes, so a slow plugin blocks neither the worker nor other
    plugins.
  - *Cancel and deadline.* A cancelled run or one past its deadline raises
    the invocation's `AbortSignal`. Protocol 1 has no `abort` frame and an
    ABI 1 guest runs to completion, so the supervisor gives the host 2 s
    (`HostConfig::abort_grace`) to answer, then kills it
    (`HostError::Aborted`, not counted as a crash). The handler waits the
    same 2 s (plus a margin), then fails the action as `cancelled`; a later
    answer is discarded. A capability call that is already running (an HTTP
    request, at most 15 s) finishes first.
  - *Containment.* Every invocation on a lane runs in `catch_unwind` (the
    broker's authorization and the executor run there); so do the
    worker's message handling, the grant updates (`sync_grants`) and the
    handler itself. A panic in the broker or executor disables the plugin;
    a host crash fails the action (`plugin-host-failed`). Neither reaches
    the service task.
  - *Older clients.* `Response::for_protocol(< 10)` drops catalog entries
    owned by a plugin module; plugin answers never reach such clients.
    Instances in profiles stay and show as unavailable.

## Plugin actions in the configuration app

Only in app builds with `VITE_DECKPILOT_PLUGINS=1`; every other build asks
the agent nothing and behaves as before.

- **Source of truth.** Each `PluginActionInfo` in `listPlugins` (and in
  every `PluginDetails`) carries `definition`: the `ActionType` exactly as
  the engine registers it (`engine::action_type`: the manifest's parameters
  and node contract), available or not. The field is additive and optional
  (agents before it, or an action that does not convert, leave it out).
- **Backend.** `src-tauri/src/plugin_actions.rs` records every answer of
  `list_plugins` (replace; a refusal clears it), of the commands that
  return a plugin's details (upsert) and of `uninstall_plugin` (remove).
  `AppState::actions()` adds the recorded types to the built-in registry:
  available ones as definition-only handlers (with no capabilities; they
  never run in the app), the others as unavailable with the reason worded
  as the engine words it (`Hello Weather (acme.hello-weather): the plugin
  is disabled`). Entries outside their plugin's namespace are ignored. So
  `create_action`, `update_action`, `save_workflow`, graph validation and
  layouts validate plugin actions as the agent will run them, and refuse
  new ones that cannot run. Until the frontend asks, nothing is recorded.
- **Frontend.** `plugins/use-plugin-modules.ts` lists the installed plugins
  when the agent starts or changes and on every `changed` plugin event;
  `lib/plugin-actions.ts` turns each into a module of the editors (id = the
  plugin id, action types = the definitions, per-type unavailable reasons),
  kept apart from the built-in modules (`AppContext.pluginModules`,
  `useActionModules`). The Action Library's type picker and the workflow
  node palette list them under the plugin's name, with the manifest
  parameters as editors; unavailable actions show the reason and cannot be
  chosen, placed or created. Instances of a disabled plugin keep their
  name and parameters and show the reason; those of an uninstalled plugin
  say that the plugin is not installed. None is ever dropped. The owner of
  a type is everything before its **last** `.`, as `ModuleId::owner_of`.

## IPC (protocol 10, capability `plugins`)

Types: `crates/deckpilot-ipc/src/plugins.rs`. Every request below has
`since_protocol` 10; the client refuses them locally on an older
connection, an older agent never receives them, and nothing an older client
receives changes. Errors carry `IpcError.reason` (`plugins::reason`), which
older clients ignore.

| Request (`type`) | Fields | Answer |
| --- | --- | --- |
| `getPluginRuntime` | — | `pluginRuntime { status }`: `available`, `built`, `enabled`, `executionSupported`, `platform`, `reason`, `registry { source, freshness, seq }` (answered even when plugins are off) |
| `listPlugins` | — | `plugins { plugins: [PluginSummary] }` (each action with its `definition`) |
| `getPlugin` | `plugin` | `plugin { plugin: PluginDetails }` |
| `listAvailablePlugins` | `refresh` | `availablePlugins { registry, plugins: [AvailablePlugin] }` (versions newest first, permissions, `withdrawn`, `incompatible`) |
| `installPlugin` | `source: {type: "registry", plugin, version?} \| {type: "file", path}`, `consent: [key]` | `plugin` (state `disabled`) |
| `enablePlugin` | `plugin`, `consent: [key]` | `plugin` (grants `consent` first; all declared permissions must be granted) |
| `disablePlugin` | `plugin` | `plugin` |
| `grantPluginPermissions` | `plugin`, `permissions: [key]` | `plugin` |
| `revokePluginPermission` | `plugin`, `permission` | `plugin` |
| `updatePlugin` | `plugin`, `version?`, `consent: [key]` | `pluginUpdate { plugin, outcome: {type: "applied" \| "awaitingConsent", version, changes} }` |
| `approvePluginUpdate` | `plugin`, `consent: [key]` (every escalated key) | `pluginUpdate` |
| `declinePluginUpdate` | `plugin` | `plugin` |
| `rollbackPlugin` | `plugin` | `plugin` |
| `uninstallPlugin` | `plugin`, `keep?: {settings?, credentials?}` | `ok` (see [retention](#uninstall-retention)) |
| `invokePluginAction` | `plugin`, `action`, `params: {name: bool \| number \| text \| list}` | `pluginActionResult { outcome: {status: "ok", message?, data} \| {status: "skipped"} \| {status: "failed", code, message?, retryable} }` (parameters that do not match the manifest: `failed`, `invalid-parameters`, before the host runs) |
| `subscribePluginEvents` | — | `ok`; then `pluginEvent` messages on this connection |
| `listKnownPlugins` | — | `knownPlugins { plugins: [{id, name, publisher, namespace, lastVersion, installed, firstSeenAt, lastSeenAt}] }` |
| `getPluginSettings` | `plugin` | `pluginSettings { settings: {plugin, fields, values, effective, missing?} }` |
| `setPluginSettings` | `plugin`, `values: {key: value \| null}` (merged; `null` resets to the default) | `pluginSettings` (refused with `invalidData` unless every value fits its field) |
| `listPluginCredentials` | `plugin` | `pluginCredentials { plugin, credentials: [{key, label, description?, hosts, schemes, setupUrl?, set, updatedAt?, granted}] }` |
| `setPluginCredential` | `plugin`, `key`, `secret` (1 B to 16 KiB, no line break or NUL) | `pluginCredentials` (never the secret) |
| `deletePluginCredential` | `plugin`, `key` | `pluginCredentials` |

Permission keys are grant keys: `network:<host>`, `notifications`,
`credentials:<ref>`, `variables:<entry>`, `storage`, `profile`,
`openUrl:<host>`, `background`, `device:state`, `device:icons`.
`PluginDetails.permissions` lists each declared one with `tier`, `status`
(`granted`, `revoked`, `notGranted`), `scope` and `justification`;
`PermissionChanges` (`added`, `broadened`, `removed`, `escalation`)
describe an update.

Events are a server message of their own,
`{"type": "pluginEvent", "event": …}`, sent only to connections that asked
with `subscribePluginEvents` (`AgentClient::subscribe_plugin_events`,
`AgentClient::next_plugin_event`), so existing clients never see them:

- `{"type": "progress", plugin, operation, stage, message?}`: operation
  `install`, `enable`, `update`, `uninstall`, `recover`, …; stage `started`,
  `fetching`, `verifying`, `installing`, `finished`, `failed`.
- `{"type": "changed", plugin, state?}`: a new state; no `state` means
  uninstalled.

Reasons: `pluginsUnavailable`, `platformUnsupported`, `locked`,
`storeUnavailable`, `notInstalled`, `alreadyInstalled`, `unknownPlugin`,
`consentRequired`, `unknownPermission`, `registryUnavailable`, `revoked`,
`incompatible`, `invalidPackage`, `unsupportedRuntime`, `invalidState`,
`notEnabled`, `hostFailed`, `broken`, `invalidData`.

The app reaches the data requests through the Tauri commands
`list_known_plugins`, `get_plugin_settings`, `set_plugin_settings`,
`list_plugin_credentials`, `set_plugin_credential`,
`delete_plugin_credential` and `uninstall_plugin` (with `keep`), and the
functions of the same names in `apps/desktop/src/plugins/client.ts`. No UI
uses them yet.

## Plugin data: storage, credentials, settings

All of it lives in plugins.db (schema 2), keyed by plugin id. The id is
always the agent's (the plugin whose host the supervisor spawned, through
the broker session); no capability request names a plugin, so a plugin
cannot reach another's rows. The executor reads and writes through a
second connection (`PluginData`), closed when the configuration locks.

| | Storage | Credentials | Settings |
| --- | --- | --- | --- |
| Table | `plugin_storage` (deleted with the plugin, foreign key) | `plugin_credentials(plugin_id, key, secret, updated_at)` | `plugin_settings(plugin_id, values_json)` |
| Declared in the manifest | nothing (always on, design §5.2); `storage` permission (`quotaKb` 257-10,240) for more | `contributes.credentials[]` slot + `credentials` permission `refs` | `contributes.settings.fields` |
| Guest ABI | `storage.get`, `storage.set`, `storage.delete`, `storage.keys`, `storage.usage` | `http.fetch` with `auth: {credential, scheme}`, `credentials.has` (true or false) | `settings.get` (object, defaults filled) |
| Set by the user | — | `setPluginCredential` | `setPluginSettings` |
| Limits | 256 KiB and 1,000 keys by default; key 1-128 characters, no control characters; value ≤ 64 KiB of JSON; bytes = key + JSON value, UTF-8 | ≤ 16 KiB, only declared slots | only declared keys; types, ranges, choices and lengths as for action parameters |
| Revocation | revoking `storage` returns the quota to 256 KiB on the next write (reading and shrinking still work); disable or uninstall denies everything | revoking `credentials:<key>` denies the next `http.fetch` with that slot and `credentials.has` | no permission; disable or uninstall denies `settings.get` |

- **Quota.** The broker puts the quota into every `storage.set` decision
  (`Allowed::StorageSet { quota }`), read from the current grants, and the
  executor checks it in the write transaction: a write that would grow
  the storage past the quota or add a key past 1,000 fails with
  `failed` and a message starting `storage-quota` (the design's error
  name); a write that does not grow it is allowed even above the quota.
- **Credentials are handles** (design §5.5). The guest names a slot; the
  broker allows the slot only for its own hosts (exact `https` origins,
  each also a network grant), its declared schemes, and never across a
  redirect to another origin. The executor then reads the secret and hands
  it to the HTTP module's client as `Auth::{Bearer, Basic, Header, Query}`
  (header and query names from the slot's `headerName` / `queryName`),
  for that request only; the client sends credentials over https only. No
  secret stored: `unavailable`, "no secret is stored". Every form of the
  secret (as is, form-encoded, base64) is replaced by `[redacted]` in the
  response headers and body and in error messages before anything reaches
  the guest. The secret is never logged (the IPC request's `Debug` shows
  `PluginSecret(<redacted>)`, the audit records `credentials:<key>`), never
  exported, and plugins never see SQLCipher keys or keychain items (there
  is no capability for them).
- **Settings** are stored as the user saved them (never the defaults, so
  a default changed by an update applies). `settings.get` and
  `getPluginSettings.effective` give each saved value that still fits its
  field, otherwise the default; a required setting with neither is listed
  in `missing` and absent for the plugin (actions still run). Fields map to
  the core's `ParameterSpec` like action parameters, so validation is the
  core's `ActionType::normalize`.
- **Parameters.** Every invocation (`invokePluginAction`, keys,
  `runAction`, workflow nodes) is checked against the action's manifest
  schema in the agent before it reaches the lane: unknown parameters,
  wrong types, values out of range, choices not offered, too long texts and
  missing required parameters fail with `invalid-parameters` and a message
  that lists every problem and names the action type; defaults are filled
  in. The manifest's camel-case kind constraints (`maxLength`, `maxItems`,
  …) are renamed to the core's snake case first (they were silently
  dropped before), and a constraint the core does not know refuses the
  action.
- **`plugin_known`** (design §2.3 item 4): every version written (install,
  update, staged update) records id, namespace, publisher, name, version
  and the actions; the table only grows. After an uninstall the agent
  registers a disabled module from it, so instances of its actions fail
  with `Hello Weather (acme.hello-weather): the plugin is not installed`.

### Uninstall retention

Default (design §5.6): uninstall deletes the plugin's files, records,
grants, **storage, settings and credentials**; its audit and its
`plugin_known` row stay. `keep: {settings: true}` and/or
`keep: {credentials: true}` keep those for a later install of the same
plugin id (storage is never kept). The choice is journaled with the
uninstall, so recovery after a crash honours it. The design's "after
asking" for credentials is the app's job (the default is to delete).

## Running it

```sh
cargo build -p deckpilot-agent --features plugins
(cd plugins/runtime && cargo build --locked -p deckpilot-plugin-host --bin deckpilot-plugin-host)
target/debug/deckpilot-agent --headless --enable-plugins \
  --plugin-host "$PWD/plugins/runtime/target/debug/deckpilot-plugin-host" \
  --plugin-registry <registry dir> --plugin-registry-roots <roots.json>
```

The host path must be absolute. A full manual test with a local registry
and the app: [local-testing.md](local-testing.md).

## Tests

`cargo test -p deckpilot-agent --features plugins --test plugins_e2e`
builds the host and the guests in `plugins/runtime`, makes a registry with
the publishing tool and throwaway keys (the sample as 1.0.0 and as 1.1.0
with an extra permission, the SDK test guest, an incompatible and a
revoked plugin) and drives a real agent process over IPC: the whole
lifecycle (inspect, install, encrypted state, consent, enable, grant,
invoke, revoke, denied, redirect refused, disable, restart, update with
escalation, rollback, uninstall, no files or rows left, configuration
database unchanged), corrupt, re-pinned, invalid-manifest and JavaScript
packages, revoked and incompatible versions, a registry with a bad
signature, a host crash, crashes in the middle of an install, the wrong
plugins.db key, and a host stand-in reporting a partial sandbox
(`deckpilot-test-plugin-host`, built only with `test-hooks`), which must be
stopped before it receives the plugin. Plugin actions in the engine: the
sample's actions as instances in a profile, unavailable while disabled, then
run by `runAction`, by key presses on a simulated MK.2 and by a workflow
node (results, outputs and the canned request checked), unavailable again
after a revoke, a disable and an uninstall; a guest panic during a key's
action (a failed action, the agent unaffected); and a workflow cancelled
while its plugin node waits for a slow server (given up after the 2 s
grace). Plugin data (`plugin_data_is_isolated_bounded_revocable_and_secret`,
the SDK test guest as two plugins, one with a 512 KiB storage grant):
storage isolation between them, the default and the granted quota, a
revoked `storage` grant on the very next write, credential slots without
and with a secret (one per plugin), injection as bearer, header and query
only to the slot's host (checked on the wire), refusals for another host,
an undeclared scheme or slot and a redirect off the origin, the echoed
secret redacted before the guest sees it, a revoked slot on the very next
call, settings with defaults and validation, uninstall with and without
`keep`, `plugin_known` after it, and no secret in the agent's log or as
clear text in any file of `plugins/`. HTTP goes to canned responses
(`DECKPILOT_TEST_PLUGIN_HTTP`, only with `test-hooks`, never in a release;
a route may answer late with `delayMs`; the log records what would go on
the wire, injected credentials included).
`.github/workflows/plugins-agent.yml` runs all of it on Linux, macOS and
Windows (not a required check).

## Not done

- No network fetching of the registry; the registry trust state lives in a
  file, not in the encrypted database.
- No variables (design §2.3 item 3) or background work; their
  capabilities are refused.
- Plugin settings live in plugins.db, not in the configuration database's
  `modules` table as the design has it (§12.4), so they are not exported
  with the configuration (design §2.3 item 9, by portability); a
  `settingsChanged` notification to a running plugin (design §3.8) does
  not exist (the plugin reads `settings.get` when it runs). No settings
  of kind `project` are offered by any UI.
- No secret-typed settings: secrets are credential slots
  (`contributes.credentials`), set with `setPluginCredential`. Request
  signing, OAuth and other uses beyond injection (design U-3) do not
  exist.
- `privateNetwork` and wildcard hosts are still never granted, so a
  credential slot can only name exact hosts.
- No UI for settings, credentials, retention or known plugins; no storage
  usage in `PluginDetails`.
- The JavaScript SDK skeleton (`plugins/sdk-js`, declarations only, no
  runtime) still declares the design's synchronous `ctx.settings`; the
  ABI delivers settings through `settings.get`, which a JavaScript shim
  would call at the start of an invocation.
- The layout editor's gesture editor and the step-workflow editor use the
  backend's action type list as before; plugin types appear there only
  after the app has read the installed plugins (see
  [the editors](#plugin-actions-in-the-configuration-app)).
- An abort does not interrupt a capability call that is already running.
- A password change made by the app while the agent is locked makes
  plugins.db unreadable (no rekey protocol between app and agent).
- Windows execution is verified only on GitHub's `windows-latest` runner
  (Windows Server 2025), through the CI workflow (run 43689909295, job
  132738173666). Desktop Windows editions (10, 11), per-user and
  `Program Files` installs, and machines with third-party security
  software are untested; on any of them a host that cannot get its
  AppContainer reports `partial` and is refused rather than run.
- The sandbox is verified locally on macOS only for this integration; Linux
  and Windows rely on the CI workflow, run 43689909295: Linux job
  132738173732 (GitHub's `ubuntu-latest`, image ubuntu-26.04, kernel
  `7.0.0-1019-azure`, Landlock ABI 8), Windows job 132738173666 (Windows
  Server 2025), macOS job 132738173797. A Linux kernel without Landlock,
  or with an ABI below 8 (before Linux 7.0), makes the host report
  `partial`, so plugins do not run there; that excludes most current LTS
  kernels.
