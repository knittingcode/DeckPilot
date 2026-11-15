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
  PluginManager   Registry        package        Broker            Supervisor ×N
  (plugins.db,    (RegistryClient verification   (grants per call, (one per running
  crash-safe      over HTTPS or   (pins, manifest, redirects, rate  plugin; spawns
  lifecycle)      a directory)    plugin.wasm)    limits, audit)     the host)
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
- **Registry.** Over HTTPS (`--plugin-registry-url <https URL of the site>`,
  [below](#registry-over-https)) or from a local directory
  (`--plugin-registry <dir>`, the publishing tool's layout: `site/v1/`,
  `releases/`), both read through a `RegistrySource` (`plugins/registry.rs`,
  `plugins/fetch.rs`) and verified by the same `RegistryClient`. The root
  keys come separately from `--plugin-registry-roots <roots.json>`; none is
  built in (production root keys are not approved, U-10), so without them
  nothing is fetched and the status says no registry is configured. With
  roots and neither a URL nor a directory the official site
  `https://knittingcode.github.io/deckpilot-plugins/` is used
  (`registry::DEFAULT_REGISTRY_URL`). The verified state is kept in
  `<data dir>/plugin-registry/` ([why not plugins.db](#trust-state-stays-a-file)).
- **Install.** Registry refresh (key sets, timestamp, index), revocations,
  `resolve_install` (compatibility, pins), the package downloaded (or read
  from `releases/`; a local file is accepted only if its bytes are what
  the registry pins, and needs no download), `verify_package` (sha256,
  size, manifest hash, permissions, artifact contract incl. the static
  `plugin.wasm` check), then the store's journaled install. A crash at any
  step is completed or undone by `PluginManager::recover` at the next
  start.
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
  name; `log` goes to the agent log; `variables.publish` and
  `variables.clear` go to the plugin's own variables (below). Storage and
  credential injection are refused (`unavailable`): neither exists in the
  agent yet.
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
  windows or migrations; its variables are below. Each action is a
  `PluginActionHandler`:
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
    the invocation's `AbortSignal`. A capability call that is running then
    stops at once: the supervisor hands every call a `CallInterrupt`
    (abort signal and deadline) through
    `CapabilityExecutor::execute_interruptible`, and the executor's fetch
    cancels the HTTP module client's `CancellationToken` (the connection is
    dropped; the canned test transport stops waiting likewise). Whatever the
    call returns after the interrupt is replaced by `unavailable` for the
    guest, and every later call of an aborted invocation is refused without
    reaching the broker or the executor (no request, publish or
    notification after an abort). Past the deadline the host is killed at
    once (`HostError::Timeout`), queued frames unread. Protocol 1 has no
    `abort` frame and an ABI 1 guest runs to completion, so after an abort
    the supervisor gives the host 2 s (`HostConfig::abort_grace`) to end
    the invocation, then kills it (`HostError::Aborted`, not counted as a
    crash). The answer of an aborted invocation is never applied (design
    §6.3: discarded): the lane turns it into `LaneError::Aborted`, and the
    handler fails the action as `cancelled` whatever arrives.
  - *Revocation during a run.* Grants are read by the broker on every call,
    and plugin requests are answered by the worker while lanes run, so a
    revoke (or disable, or uninstall) while an action runs makes the
    **next** privileged call of that same invocation fail with
    `not-permitted`; a call that is already running is not interrupted by
    a revoke.
  - *Containment.* Every invocation on a lane runs in `catch_unwind` (the
    broker's authorization and the executor run there); so do the
    worker's message handling, the grant updates (`sync_grants`) and the
    handler itself. A panic in the broker or executor disables the plugin;
    a host crash fails the action (`plugin-host-failed`). Neither reaches
    the service task.
  - *Older clients.* `Response::for_protocol(< 10)` drops catalog entries
    owned by a plugin module; plugin answers never reach such clients.
    Instances in profiles stay and show as unavailable.
- **Variables** (design §2.3 item 3, §3.7, §5.2 "own variables").
  - *Grammar* (`deckpilot-core`). `plugins` is a reserved root
    (`PLUGIN_ROOT`, in `RESERVED_ROOTS`; not in `MODULE_NAMESPACES`, which
    stays the list of built-in roots). `VariableRef::parse` accepts
    `plugins.<publisherCamel>.<nameCamel>.<path>` (1 to 3 path segments, at
    most 6 in all) whenever the two middle segments are the lower camel case
    of a valid plugin id, without consulting any list, so references parse
    on every computer, with or without the plugin.
    `VariableId::owner_namespace` (`plugins.acme.helloWeather`) and
    `VariableId::plugin_owner` (`acme.hello-weather`; the camel case is
    injective) name the owner; `module_namespace` of a plugin module is its
    `plugins.…` namespace, and `register_module` accepts only declarations
    inside it. A reference whose plugin is not installed resolves as
    `Unavailable { owner: "<plugin id>" }`. The app's `referenceRoot`
    (`apps/desktop/src/lib/variables.ts`) treats `plugins.<p>.<n>.<x>` as a
    module root (`PLUGIN_ROOT`, mirrored and checked by a Rust test).
  - *Declarations.* The active version's `contributes.variables` become the
    `PluginModule`'s variables (read-only, with their type, scope,
    requirements and sensitivity). While the plugin cannot run they are
    unavailable with its reason; after an uninstall they are gone and
    references resolve as unavailable.
  - *Publishing* (ABI 1, compatible addition): `variables.publish {id,
    value}` (`value: null` clears) and `variables.clear {id?}`; SDK
    `Context::publish_variable`, `clear_variable`, `clear_variables`. `id`
    is relative, so a plugin can only name its own namespace. The broker
    allows them for any plugin that has grants (always-on, like the log),
    10 per second; the ABI bounds values to 16 KiB. The executor accepts only
    ids the active manifest declares (`not-permitted` otherwise), of the
    declared type (`bad-request`), and only `machine` scope (`unavailable`
    for profile and project variables in this version).
  - *Values* (`plugins/variables.rs`). Kept per plugin in memory
    (`PublishedVariables`), never persisted; `PluginVariableSource`
    resolves from them, so no plugin code runs during a resolution. Every
    publish announces `sharedVariablesChanged` with the full id. Values are
    dropped, with a new generation, whenever the host is stopped for a
    lifecycle reason (`stop_host`: disable, update, approve, rollback,
    uninstall, quarantine, crash loop or panic) and when the store closes
    (lock, stop); a publish from an invocation prepared before is refused.
    A single host crash keeps them (the plugin stays enabled).
  - *Older clients.* Before protocol 10, catalog entries of plugin
    modules are dropped (as for actions), previews of `plugins.…`
    references answer `invalid` as such a client always got, and
    `AgentEvent::for_protocol` removes plugin ids from
    `sharedVariablesChanged` (the event is not sent if nothing is left).
- **Lifecycle.** Every operation that changes an installed plugin sends a
  `changed` plugin event (`watch_plugin_events` forwards it to the app)
  and republishes the catalog, so the agent re-registers the plugin's
  module, actions and variables before the next request is answered:
  install (`disabled`), enable, disable, grant, revoke, update, approve,
  decline, rollback, quarantine and release by registry verdicts, a crash
  loop or broker panic (`disabled`), uninstall (no state). Hosts: disable,
  update, approve, rollback, uninstall, quarantine and a crash loop stop
  the lane (the running invocation is aborted as above, queued ones are
  refused, the host gets `shutdown` and its `dp_shutdown` runs, then it is
  killed after 1 s); the next invocation of an enabled plugin starts a
  fresh host for the active version. A single crash restarts the host on
  demand with backoff and changes no state. Grants and revokes need no
  restart (grants per call). Locking stops every host.
  - *Guest hooks.* ABI 1 already has the lifecycle hooks the design asks
    for (§3.8 `activate`/`deactivate`): the optional exports `dp_init`
    (run when the module is loaded; a non-zero result fails the load) and
    `dp_shutdown` (run on `shutdown`). No further export was added: the
    design's other kinds (`timer`, `settingsChanged`) need background work
    and settings, which do not exist yet, and a capability call inside a
    hook is refused locally by the host (`unavailable`).

## Registry over HTTPS

`plugins/fetch.rs` (`HttpsSource`, a `RegistrySource`) fetches the
documents and packages; it decides nothing about trust. Every file goes
through `RegistryClient` exactly as from a directory: the key set chain
from the configured roots (a root rotation is accepted only as
`keys/<n+1>.json` signed by the current roots, and by the new ones too),
the timestamp's signature, seq floor (rollback) and expiry (freeze, at
most 14 days valid), the index pinned by the timestamp, each detail pinned
by the index, each package pinned by its detail. Where a file is hosted
never counts, and no key is ever taken from the network.

- **Configuration.** `--plugin-registry-url` / `DECKPILOT_PLUGIN_REGISTRY_URL`:
  the site (documents at `<url>v1/…`); `--plugin-registry-releases-url` /
  `DECKPILOT_PLUGIN_REGISTRY_RELEASES_URL`: where packages come from
  (`<url><id>--v<version>/<file>`, the tool's `releases/`) instead of the
  pinned `https://github.com/knittingcode/deckpilot-plugins/releases/download/…`;
  `--plugin-registry-roots` as above. Both URLs must be https, without
  credentials, query or fragment. A directory and a URL together are
  refused.
- **Client.** One reqwest client (rustls with ring, the HTTP module's
  stack; no new crate in the agent) and one current-thread runtime per
  registry, used only by the plugin worker: https only, the operating
  system's roots (rustls-platform-verifier), connect 10 s, 20 s without
  data, 30 s per document and 120 s per package in all. Environment
  proxies are honoured (signatures, not the transport, are the trust).
- **Redirects.** One hop at a time, at most 5, https only, and only within
  the request's origin. A package fetched from the pinned GitHub URL may
  also go to GitHub's asset hosts (`objects.githubusercontent.com`,
  `release-assets.githubusercontent.com`; design §9.3 step 5); with a
  releases URL, same origin only. Anything else is refused, not followed.
- **Sizes.** Each document stops at the client's cap (key set 16 KiB,
  timestamp 4 KiB, signatures 2 KiB, index 4 MiB, detail 256 KiB), a
  package at its pinned size (at most 5 MiB); a larger `Content-Length` is
  refused before the body is read.
- **Errors.** No answer, a TLS failure, a 5xx/408/429 or a body cut off is
  `FetchError::Unavailable`: the refresh reports `Offline` and the agent
  works from its verified state. A refused redirect, an oversized answer,
  another status or a bad path is `FetchError::Refused` (new in the
  registry crate): an error, never "offline". Like every refused document,
  it changes nothing stored.
- **Packages.** Downloaded into memory and returned only if the size and
  sha256 are the pinned ones (`verify_package` then checks everything
  again), so a partial or corrupt download is refused and nothing is
  written; the store stages only verified bytes. An update whose download
  fails leaves the installed version active and running. A tampered
  package is `invalidPackage`, an unreachable or refused one
  `registryUnavailable`.
- **Offline cache.** The crate's `FileTrustStore` (trust state, index
  cache) and the verified details in `plugin-registry/details/<sha256>.json`
  (content-addressed: re-checked against the verified index whenever read,
  pruned when an index no longer pins them). Offline, browsing lists the
  cached plugins, installed plugins run with their stored revocations,
  installs fail at the download.
- **Status.** `registry` in `getPluginRuntime` and `listAvailablePlugins`
  gains three additive fields: `staleReason` (`expired`, `indexExpired`,
  `indexUnavailable`), `expires` (the verified timestamp's expiry) and
  `offline` (why the last refresh could not reach the registry). A refused
  refresh is reported in `reason`, as before.
- **Revocations.** Every refresh that verifies a new index applies its
  revocations to the installed plugins (a `disable` quarantines the
  version and stops its host), not only the one at start and in
  `listAvailablePlugins`. A refresh is still only done at unlock and on
  request (browse with `refresh`, install, update); there is no timer.
- **Refresh from the app.** Already there: `list_available_plugins
  { refresh: true }` (Tauri) and `listAvailablePlugins(true)` (TS client).

### Trust state stays a file

The trust state and the index cache stay in `<data dir>/plugin-registry/`
(`FileTrustStore`: atomic replace with fsync) and are not moved into
plugins.db. Moving them needs a new table and migration in the plugin-store
crate, which is being changed in parallel; the state is not secret (it is
signed public data plus a seq and clock floor), and the attacker who could
reset it, local malware running as the user, is out of scope in the threat
model (§4.2) and also holds the database key. A missing state starts again
from the configured roots (trust on first use); a corrupt one is an error,
never a silent reset. Moving it into plugins.db (§12.4 `registry_state`)
remains open.

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
- **Layout and step editors.** The layout editor's library, context menu,
  inspector and gesture editor, and the step-workflow editor, take the
  plugin's state from the same modules (`pluginActionUnavailable`), not
  from the backend's type list: a saved action whose plugin cannot run
  (disabled, broken, quarantined, a permission not granted, not installed)
  cannot be dragged, assigned or added as a step, and says why; keys,
  gestures and steps that already use it keep it and show the reason. The
  gesture editor's and step editor's parameter overrides use the plugin
  types available now (`withPluginTypes`), so they appear as soon as the
  app has read the plugins, without reopening the editor.

## IPC (protocol 10, capability `plugins`)

Types: `crates/deckpilot-ipc/src/plugins.rs`. Every request below has
`since_protocol` 10; the client refuses them locally on an older
connection, an older agent never receives them, and nothing an older client
receives changes. Errors carry `IpcError.reason` (`plugins::reason`), which
older clients ignore.

| Request (`type`) | Fields | Answer |
| --- | --- | --- |
| `getPluginRuntime` | — | `pluginRuntime { status }`: `available`, `built`, `enabled`, `executionSupported`, `platform`, `reason`, `registry { source, freshness, seq, staleReason?, expires?, offline? }` (answered even when plugins are off) |
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
| `uninstallPlugin` | `plugin` | `ok` |
| `invokePluginAction` | `plugin`, `action`, `params: {name: bool \| number \| text \| list}` | `pluginActionResult { outcome: {status: "ok", message?, data} \| {status: "skipped"} \| {status: "failed", code, message?, retryable} }` |
| `subscribePluginEvents` | — | `ok`; then `pluginEvent` messages on this connection |

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
`notEnabled`, `hostFailed`, `broken`.

## Running it

```sh
cargo build -p deckpilot-agent --features plugins
(cd plugins/runtime && cargo build --locked -p deckpilot-plugin-host --bin deckpilot-plugin-host)
target/debug/deckpilot-agent --headless --enable-plugins \
  --plugin-host "$PWD/plugins/runtime/target/debug/deckpilot-plugin-host" \
  --plugin-registry <registry dir> --plugin-registry-roots <roots.json>
# or over HTTPS:
#   --plugin-registry-url https://<host>/<site>/ --plugin-registry-roots <roots.json>
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
while its plugin node waits for a slow server (the request is stopped and
the run ends at once). Plugin variables: references unavailable before the
install and while disabled, the SDK test guest's published values resolved
with their types (an undeclared id and a mistyped value refused), cleared,
dropped by a disable, unavailable after an uninstall, and invisible to a
protocol 9 client (catalog, previews, events). Cancellation: a workflow
cancelled while the guest waits 14 s for a server ends within 1.5 s and the
answer is never published. A mid-run revoke: the second request of
`two-fetches` is denied and never goes out. Lifecycle: `changed` events
for install, enable, quarantine (the test's own registry copy revokes the
version) and uninstall, with actions and variables following, and a host
crash that keeps the plugin enabled. HTTP goes to canned responses
(`DECKPILOT_TEST_PLUGIN_HTTP`, only with `test-hooks`, never in a release;
a route may answer late with `delayMs`, and stops waiting when the call is
interrupted).
`cargo test -p deckpilot-agent --features plugins --test plugins_registry_https`
does the same over HTTPS: a local TLS server (`tests/support/https.rs`,
rustls with a throwaway CA from rcgen that only the agent under test
trusts, through `DECKPILOT_TEST_REGISTRY_CA`, feature `test-hooks`) serves
a registry made with the publishing tool. Through the real agent and host:
install, enable and run over HTTPS; a tampered package, a download cut off
midway and a package redirected to another origin, each refused with
nothing installed; an update cut off (the old version keeps running), then
applied; a revocation that quarantines the installed version; a rollback
to the older site, a tampered index, a timestamp redirected to another
origin and an oversized one, all refused with the stored state kept;
offline from the cache (browsing, the installed plugin runs, installs
fail), a lost index cache (stale, `indexUnavailable`), back online; a
frozen registry (expired timestamp: nothing trusted), one that expires
while the agent runs (stale, `expired`, installs refused until re-signed);
and an agent without root keys, which fetches nothing. The unit tests in
`plugins/fetch/tests.rs` (`cargo test -p deckpilot-agent --features plugins
--lib plugins::`) cover the source with the client directly and a
controlled clock: verification, offline then stale, reachable but expired,
freeze, rollback, tampered timestamp, index and key set, key rotation over
the chain, redirects (other port, other host name, http, endless, same
origin), oversized (declared or not), cut off, 503 and 403, an untrusted
certificate, path and URL checks, and package downloads (tampered, short,
cut off, oversized, foreign URLs, redirects to allowed and other hosts).
`.github/workflows/plugins-agent.yml` runs all of it on Linux, macOS and
Windows (not a required check).

## Not done

- The registry trust state lives in a file, not in the encrypted database
  ([why](#trust-state-stays-a-file)).
- No production root keys and no default trust: the official site URL is
  known, but without `--plugin-registry-roots` nothing is fetched (U-10).
  The GitHub asset redirect hosts for packages come from the design, whose
  §20.1 lists the release-asset redirect host as unverified; no test
  reaches GitHub.
- No periodic refresh: the registry is checked at unlock and on request,
  so a long-running agent goes stale after 7 days without a browse,
  install or update. No `If-None-Match`/ETag, no package cache (offline
  installs are not possible), no proxy configuration beyond the
  environment.
- No plugin storage, credential slots, settings or background work; their
  capabilities are refused.
- Plugin variables: only `machine` scope can be published (profile and
  project values need the `profile` permission and a scope argument, design
  §6 `VariablesApi.publish`); no reading of other owners' variables
  (`variables` permission); values are not persisted across agent restarts
  (as designed) and there is no `plugin_known`, so the picker cannot list
  variables of plugins that are not installed.
- A revoke does not interrupt a capability call that is already running;
  it denies the next one.
- No crash-loop end-to-end test: the five crashes with backoff take about
  15 s. The breaker is tested in the supervisor (`crates/supervisor`); the
  agent's reaction (`lane_ended`: disable, drop values, `changed`) is not
  tested end to end.
- `plugin_known` (design §2.3 item 4) does not exist: an uninstalled
  plugin's actions are reported as "not available" without its name.
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
