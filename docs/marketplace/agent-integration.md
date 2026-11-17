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
  keys are compiled in at build time (`DECKPILOT_REGISTRY_ROOT_PUBKEYS`,
  [below](#trusted-root-keys)) or given at run time with
  `--plugin-registry-roots <roots.json>`. No build embeds them yet
  (production root keys await the key ceremony, U-10), so without
  `--plugin-registry-roots` nothing is fetched and the status says no
  registry is configured. With roots and neither a URL nor a directory the
  official site `https://knittingcode.github.io/deckpilot-plugins/` is used
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
  and `privateNetwork` are never passed to the broker (exact origins only);
  `openUrl` is passed per exact host, like `network`. A server the user
  entered in an origin setting (G2) is passed only while the user's consent
  to exactly it (`plugin_user_origins`) matches what the setting holds and
  its slot permission is granted ([permissions.md](sdk/permissions.md#user-entered-origins)).
  With it goes whether its pages may be opened (`openUrl:user-origin:<setting>`
  granted and still declared); that grant alone is passed as nothing.
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
  to `Session::authorize_redirect`. Response headers are filtered to the
  ABI's allowlist (`RESPONSE_HEADERS`: caching, `link` and GitLab's
  paging headers, GitHub's `x-ratelimit-*` and the `ratelimit-*` family by
  exact name; never `set-cookie` or an authentication header), after an
  injected secret was redacted from every value. `url.open`: the normalized
  https URL the broker allowed for a granted `openUrl` origin, re-checked
  (https, no userinfo, the origin's host and port), opened through the
  core launcher (`launch::plan(OpenTarget::Url)`, no shell), at most 3 per
  invocation, only in a run the user started, logged by origin only;
  agent tests record it instead (`DECKPILOT_TEST_PLUGIN_OPEN_URL`, feature
  `test-hooks`). The run context (G8) comes from the engine: a key press
  and a gesture (`RuntimeOutcome::ActionRequested` from the raw edge or
  the gesture), `runAction` (`app`), `invokePluginAction` (`test`) and a
  workflow node (`workflow`, with the run's `ExecutionTrigger` as its
  cause: `app` and `device` are `user`, then `schedule`, `event`,
  `startup` for `agentStarted`, `workSession`) travel as the core's
  `RunOrigin` in the `Invocation` to `ExecutionContext::origin`; the
  plugin's action handler turns it, with the run's `RunSummary`, into the
  ABI's `RunContext` for the executor (`engine.rs`, `worker.rs`,
  `service.rs`). The broker decides `url.open` with it and the executor
  answers `run.context` from it.
  `notify` becomes an agent notification titled with the plugin's
  name, through the agent's notifier: the status bar of open windows
  and, when the manifest's `notifications` permission declares a
  `routing` (G9), the desktop, as the plugin's settings the routing
  names say (read by the agent at every `notify`, never by the plugin);
  guests that report `NOTIFY_DELIVERY` are answered where it was shown;
  `log` goes to the agent log; `variables.publish` and
  `variables.clear` go to the plugin's own variables (below). Storage,
  credential injection and settings: see
  [Plugin data](#plugin-data-storage-credentials-settings).
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

## Trusted root keys

The registry's root role (`roots.json`: `{"keys": [...], "threshold": n}`,
1 to 4 base64-wrapped minisign public keys, the file
`deckpilot-registry keyset init` writes and `keys/roots.json` of
`knittingcode/deckpilot-plugins` holds) is the only trust anchor. It comes
from one of two places (`deckpilot_plugin_registry::roots`):

- **Embedded at build time.** `DECKPILOT_REGISTRY_ROOT_PUBKEYS`, set when
  the agent is compiled, holds the role: the JSON itself or the absolute
  path of a `roots.json`. The registry crate's `build.rs` checks it with
  the client's own rules and **fails the build** on anything malformed
  (not JSON, an unknown field, no or more than 4 keys, a key that is not a
  minisign public key, a key listed twice, a threshold of 0, above 4 or
  above the number of keys, a path that is relative or unreadable), so a
  build never carries partial roots. Unset or empty (every build so far):
  nothing is embedded. Nothing reads the variable at run time.
- **Configured at run time.** `--plugin-registry-roots <roots.json>` or
  `DECKPILOT_PLUGIN_REGISTRY_ROOTS`.

| Build | Embedded | Configured | Trusted |
| --- | --- | --- | --- |
| any | no | no | nothing: no registry, nothing fetched, "not configured" |
| any | no | yes | the configured file (development, self-hosted registries) |
| debug, or `test-hooks` | yes | yes | the configured file |
| release | yes | yes | the embedded roots; the file is ignored, with a warning in the log |
| any | yes | no | the embedded roots |

So a release build with embedded roots cannot be pointed at other roots
by its command line or environment (like the other development overrides
since #236 and #249, the override exists only in debug and `test-hooks`
builds), while a release build without them keeps today's behaviour and
still accepts `--plugin-registry-roots` for a self-hosted registry: such
a build trusts nothing by default, so the flag adds trust only where none
was built in. With roots of either kind and no registry named, the
official site is used.

Embedded roots are only the anchor. Every refresh still walks the signed
key set chain from them (`keys/1.json` signed by them, each later key set
by the previous roots and, for a root rotation, by the new roots too), so
rotating root, index or timestamp keys needs no new app build. Only the
loss or compromise of the embedded roots does: a release with new
embedded roots, whose fingerprint differs, makes the client drop the
stored chain and walk it again from `keys/1.json` (rollback floor and
revocations are kept). Switching a debug build between embedded and
configured roots does the same.

Tests: `crates/deckpilot-plugin-registry/tests/embedded_roots.rs` builds
the crate's `registry_roots` example with throwaway roots embedded (made
with the publishing tool), in the release and debug profiles: it trusts
the registry they signed and refuses another; the release build ignores a
configured `roots.json`, the debug and `test-hooks` builds use it; without
embedded roots nothing is fetched; malformed values fail the build. The
agent's `plugins::fetch::tests::embedded_roots_trust_the_registry_they_signed`
opens a registry from embedded roots and follows a root rotation.

### Enabling the public registry in release builds

After the key ceremony (`crates/deckpilot-plugin-registry/PUBLISHING.md`,
U-10) has committed `keys/roots.json` to `knittingcode/deckpilot-plugins`:

1. Create the repository variable (not a secret: these are public keys)
   `DECKPILOT_REGISTRY_ROOT_PUBKEYS` in `knittingcode/deckpilot`, with the
   exact content of that `keys/roots.json`, for example with
   `gh variable set DECKPILOT_REGISTRY_ROOT_PUBKEYS --repo knittingcode/deckpilot < roots.json`.
   An environment variable of `production-release` works too.
2. Check it locally first: `DECKPILOT_REGISTRY_ROOT_PUBKEYS="$(cat roots.json)"
   cargo build --release -p deckpilot-agent --features plugins` must
   succeed (a malformed value fails here exactly as in the release).
3. The next release's agent build (`release.yml`, "Build and stage the
   agent") passes the variable to Cargo only when it is set. The agent
   then uses `https://knittingcode.github.io/deckpilot-plugins/` with no
   flag at all. Release builds do not enable the `plugins` feature yet, so
   the roots take effect in the first release that ships plugins.

Deleting the variable makes the following releases embed nothing again.

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

## Key state of plugin actions

A plugin action whose manifest entry has `state` is one more provider of
the agent's live key states (`apps/agent/src/service/states.rs`, the
pipeline of built-in modules): `PluginActionHandler` implements
`state_policy` and `button_state`, so the scheduler, the cache, the
redraw-on-change comparison and the renderer are the existing ones. The
plugin's part runs in its sandboxed host (`dp_state`); the agent maps the
typed `KeyState` to `ButtonState` member by member and never decodes
anything of a plugin as an image
(`apps/agent/src/plugins/key_state.rs`, design:
[plugin-key-state.md](plugin-key-state.md)).

| Budget | Where |
| --- | --- |
| A key is asked at most every 5 s, only while shown on a device | `key_state::policy`, `states.rs` |
| One state call per plugin at a time; a key waits at most 6 s for its turn, then keeps what it showed | `StateBook::begin`, `TURN_WAIT` |
| No state call queues behind a running or waiting action; a key press finds at most one state call ahead of it | `Lane::submit_idle` |
| 2 s of wall clock per call (then the host is stopped, a crash for the breaker), 8 capability calls, reads only | `STATE_CALL_TIMEOUT`, the supervisor and the broker (`plugins/runtime/README.md`) |
| 3 failures in a row for a key: asked again after 5 s, doubling to 5 min, the static face with the error mark meanwhile | `StateBook::finish` |
| Not granted (`device:state`) or no state handler: the static face without a mark, asked again after 30 s | `Asked::Quiet` |
| A plugin whose actions cannot run has no state policy: its keys are static, and what was remembered about them is dropped | `PluginActionHandler::with_state`, `StateBook::forget` |

Nothing on the agent's task waits for a plugin: `button_state` runs on a
provider thread (at most 4, shared with built-in modules, the longest
waiting key first) and waits for the plugin's lane with a time limit.
State calls are not written to the plugin's failure log; a crash counts
in the supervisor as any other. The test hook `DECKPILOT_TEST_KEY_FACES`
(feature `test-hooks`) records every face sent to a device.

## IPC (protocol 10, capability `plugins`)

Types: `crates/deckpilot-ipc/src/plugins.rs`. Every request below has
`since_protocol` 10; the client refuses them locally on an older
connection, an older agent never receives them, and nothing an older client
receives changes. Errors carry `IpcError.reason` (`plugins::reason`), which
older clients ignore.

| Request (`type`) | Fields | Answer |
| --- | --- | --- |
| `getPluginRuntime` | — | `pluginRuntime { status }`: `available`, `built`, `enabled`, `executionSupported`, `platform`, `reason`, `registry { source, freshness, seq, staleReason?, expires?, offline? }`, `features?: [name]` (answered even when plugins are off) |
| `listPlugins` | — | `plugins { plugins: [PluginSummary] }` (each action with its `definition`; `health` once an invocation failed) |
| `getPlugin` | `plugin` | `plugin { plugin: PluginDetails }` (with `icon` when the installed icon checks out) |
| `listPluginsPaged` | — | `plugins`, each summary without `actions` and with `actionCount` (feature `pagedAnswers`, [below](#answers-of-any-size)) |
| `getPluginPaged` | `plugin` | `plugin` without actions (in the summary and the manifest) and icon (feature `pagedAnswers`) |
| `getPluginActions` | `plugin`, `from?` (index, default 0) | `pluginActions { plugin, version, total, actions: [{action, manifest?, tooLarge?}], next? }` (feature `pagedAnswers`) |
| `getPluginIcon` | `plugin`, `offset?` (default 0) | `pluginIcon { plugin, icon?: {mediaType, size, sha256, offset, data, next?} }` (feature `pagedAnswers`) |
| `listAvailablePlugins` | `refresh` | `availablePlugins { registry, plugins: [AvailablePlugin] }` (versions newest first, permissions, `withdrawn`, `incompatible`, `deckpilot`, `platforms`, `publishedAt`; per plugin `description`, `categories`, `publisherInfo`, `license`, `homepage`, `sourceRepository`, `deprecated`) |
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
| `subscribePluginEvents` | `migrations?: bool` (omitted when false) | `ok`; then `pluginEvent` messages on this connection, `migrationChanged` ones only with `migrations: true` |
| `listKnownPlugins` | — | `knownPlugins { plugins: [{id, name, publisher, namespace, lastVersion, installed, firstSeenAt, lastSeenAt}] }` |
| `getPluginSettings` | `plugin` | `pluginSettings { settings: {plugin, fields, values, effective, missing?, origins?} }`; `origins` lists the origin settings (G2): `{setting, label, permission, status: unset \| invalid \| declared \| slotNotGranted \| needsConsent \| granted, origin?, problem?, privateHost?, privateNetworkAllowed?, privateNetworkGranted?, pathPrefixAllowed?, methods, credentials?, grantedAt?}` |
| `setPluginSettings` | `plugin`, `values: {key: value \| null}` (merged; `null` resets to the default) | `pluginSettings` (refused with `invalidData` unless every value fits its field; an origin setting is normalized, and a changed one ends the consent to its old server) |
| `grantPluginUserOrigin` | `plugin`, `setting`, `origin` (exactly `origins[].origin`), `privateNetwork?: bool` (omitted when false) | `pluginSettings` (refused with `invalidData` for another origin than the setting holds, a declared host, or a private host without `privateNetwork`) |
| `revokePluginUserOrigin` | `plugin`, `setting` | `pluginSettings` |
| `listPluginCredentials` | `plugin` | `pluginCredentials { plugin, credentials: [{key, label, description?, hosts, schemes, setupUrl?, set, updatedAt?, granted}] }` |
| `setPluginCredential` | `plugin`, `key`, `secret` (1 B to 16 KiB, no line break or NUL) | `pluginCredentials` (never the secret) |
| `deletePluginCredential` | `plugin`, `key` | `pluginCredentials` |

Fields added for the marketplace screens, all optional and left out when
empty (older answers parse unchanged):

- `AvailablePlugin`: the registry detail's `description`, `license`,
  `homepage`, `sourceRepository`; the index entry's `categories` and
  `deprecated` reason; `publisherInfo { namespace, displayName?, login,
  verified }` from the index's publisher list. `AvailableVersion`:
  `deckpilot` (the `engines.deckpilot` range), `platforms` (empty means
  every platform) and `publishedAt` (seconds). The registry has no icons,
  action lists or changelogs, so none of those is sent for a plugin that
  is not installed.
- `PluginDetails.icon`: the active version's `icons.128` as a
  `data:image/png;base64,` URL, only if the installed file is a PNG of at
  most 256 KiB (`MAX_ICON_BYTES`) whose sha256 is the one the integrity
  record holds (`convert::icon`).
- `PluginDetails.storage { usedBytes, keys, quotaBytes, maxKeys }`: what
  the plugin's storage holds (`plugin_storage`) and the quota its current
  grants give it, computed as the broker does for every write (256 KiB
  and 1,000 keys unless a `storage` grant raises the bytes). Left out when
  plugins.db cannot be read.
- `PluginSummary.health { crashes, lastError?, lastErrorAt? }`: kept in
  memory per plugin since the agent started (`HealthLog` in
  `service.rs`), written on the lane thread when an invocation fails. A
  host crash or timeout counts as a crash; a refused host (weak sandbox,
  files modified, backing off) only sets the last error; busy, stopped and
  cancelled invocations are not recorded. Uninstalling forgets it.

Permission keys are grant keys: `network:<host>`, `notifications`,
`credentials:<ref>`, `variables:<entry>`, `storage`, `profile`,
`openUrl:<host>`, `openUrl:user-origin:<setting>`, `background`,
`device:state`, `device:icons`.
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
- `{"type": "migrationChanged", module, plugin, phase}`: a module
  migration moved (see below); only after `subscribePluginEvents` with
  `migrations: true`.

Reasons: `pluginsUnavailable`, `platformUnsupported`, `locked`,
`storeUnavailable`, `notInstalled`, `alreadyInstalled`, `unknownPlugin`,
`consentRequired`, `unknownPermission`, `registryUnavailable`, `revoked`,
`incompatible`, `invalidPackage`, `unsupportedRuntime`, `invalidState`,
`notEnabled`, `hostFailed`, `broken`, `invalidData`, `unknownMigration`,
`answerTooLarge`.

### Answers of any size

One IPC message is at most 64 KiB (`framing::MAX_MESSAGE_BYTES`). A
plugin's answer grows with its actions (each with its full definition and
manifest entry) and its icon (up to 256 KiB, base64 in the answer): 18
actions of 20 outputs, or an icon over about 48 KiB, did not fit, and the
agent, unable to write the answer, closed the connection (the app saw the
agent as gone). Since then (`crates/deckpilot-ipc/src/plugin_answers.rs`):

- An agent that lists `pagedAnswers` in `PluginRuntimeStatus.features`
  answers `listPluginsPaged` and `getPluginPaged` without actions and
  icon, pages the actions with `getPluginActions` (as many as fit, from
  index `from`; the manifest's order; `version` and `total` let a reader
  start over when the plugin changed between pages) and chunks the icon
  with `getPluginIcon` (30 KiB per chunk, checked against the integrity
  record on every request; a multiple of 3 bytes, so the chunks' base64
  joins into the file's). An action too large for a message alone comes
  without its definition and manifest entry (`tooLarge`).
- The app's backend (`commands/plugins.rs`) uses them whenever the agent
  lists the feature: `plugin_answers::list_plugins`, `plugin_details` and
  `plugin_icon` put the answers back together exactly as `listPlugins` and
  `getPlugin` have them, so the frontend, the editors and
  `plugin_actions.rs` see every action of every plugin. After a change
  that answers with details (install, enable, grant, …) it reads them
  again in full.
- An answer that does not fit (a client that does not page, such as an
  older app) is shortened by the agent before it is sent
  (`plugin_answers::fit`): the icon goes first, then the definitions of
  the largest plugins' actions (the editors do not offer those, as for an
  action whose definition does not convert), then the manifest's actions.
  What still does not fit is refused with `failed` and the reason
  `answerTooLarge`; an event that does not fit is left out. Either way
  the connection stays open.

### Module migrations

Built-in modules moving to their first-party plugins
([module-migration.md](module-migration.md) §5.5–§5.7), all additive under
protocol 10:

| Request | Fields | Answer |
| --- | --- | --- |
| `listModuleMigrations` | — | `moduleMigrations { migrations: [ModuleMigrationInfo] }` |
| `decideModuleMigration` | `module`, `approve?: [key]`, `decline?: [key]` (at least one key, none in both) | `moduleMigration { migration }`; `unknownPermission` for a key it does not wait for, `invalidState` unless it waits for consent |
| `retryModuleMigration` | `module` | `moduleMigration { migration }`; `invalidState` unless it failed or was declined |

`ModuleMigrationInfo`: `module`, `moduleName`, `plugin`, `pluginName?`,
`phase` (`pending`, `notEligible`, `installed`, `settingsCopied`,
`awaitingConsent`, `cutOver`, `failed`), `reason?`, `attempts`,
`maxAttempts`, `canRetry`, `route` (`builtin`, `plugin`,
`pluginWithdrawn`), `routeReason?`, `aliases: [{legacy, target}]`,
`consent?: [{key, permission, import?: {setting, credential, slot,
hosts, userOrigin?}, origin?: {setting, origin, privateNetwork?,
methods?, credentials?, opensPages?}, why, decision: pending | approved |
declined}]`, `updatedAt`.
Credentials are named by id; no answer carries a secret. The Tauri
commands are `list_module_migrations`, `decide_module_migration` and
`retry_module_migration`. `PluginOrigin` gains `{"type": "bundled"}` for
bundled first-party plugins (reported as `sideload` before).

The first migration is GitHub's (P9, [module-migration.md](module-migration.md)
§6): an agent with plugins enabled lists `module: "github"`, `plugin:
"deckpilot.github"`, the six `github.*` aliases and, while it waits, one
consent item `credentials:token` with `import: {setting: "credential",
credential: "github.token" (or the module's configured id), slot:
"token", hosts: ["api.github.com"]}`; `network:api.github.com` and
`openUrl:github.com` are derived and never listed. A GitHub Enterprise
or private-host configuration is `notEligible` with the reason. Once
`cutOver`, the stored `github.*` actions, keys and workflow nodes run in
the plugin under their legacy ids (`route: plugin`); an agent without
plugins, or with `route: builtin`, runs the built-in on the same data.

GitLab's and Jira's migrations (P11) carry the server the user entered
in the built-in (a self-managed GitLab, a Jira site). While they wait,
the consent items are the credential import (`credentials:token` or
`credentials:credential`, with `import.userOrigin` naming the origin
setting the slot is bound to) and `network:user-origin:<setting>` with
`origin: {setting, origin, methods, credentials, opensPages}`: approving
that item allows exactly that server for the plugin, and nothing else
does. When reaching it needs a private network there is one more item,
`network:user-origin:<setting>:private-network` (`origin.privateNetwork:
true`): not a grant, but the separate decision without which the server
is not allowed; both must be approved in one `decideModuleMigration`,
and declining it declines the server. A self-managed GitLab also lists
`network:gitlab.com` and `openUrl:gitlab.com` (not derived: the built-in
did not use them there). `origin` and `userOrigin` are additive; an
older app shows such an item by its key and the agent's `why`.

The app reaches the data requests through the Tauri commands
`list_known_plugins`, `get_plugin_settings`, `set_plugin_settings`,
`list_plugin_credentials`, `set_plugin_credential`,
`delete_plugin_credential` and `uninstall_plugin` (with `keep`), and the
functions of the same names in `apps/desktop/src/plugins/client.ts`
(`PluginClient.listKnownPlugins`, `getSettings`, `setSettings`,
`listCredentials`, `setCredential`, `deleteCredential` and
`uninstallPlugin(id, keep)`). The marketplace (builds with
`VITE_DECKPILOT_PLUGINS=1`) uses them in Installed › Configure
(`plugins/configure.tsx`: a form generated from the fields, the agent's
`invalidData` problems under their fields, one row per credential slot),
in the uninstall dialog (keep settings and/or credentials, nothing kept by
default) and for names of uninstalled plugins (plugin details, and the
editors' "not installed" reasons). A secret is typed into an uncontrolled
password input, read once on submit, cleared at once and sent; it is never
in React state, browser storage or a log, and nothing can show it again.

## Plugin data: storage, credentials, settings

All of it lives in plugins.db (tables of schema 2; schema 3 adds the
`bundled` source, `module_migrations` and `plugin_tombstones`,
[module-migration.md §5.6](module-migration.md#56-the-migration); schema 4
adds `plugin_user_origins`, the consents to servers the user entered, G2),
keyed
by plugin id. The id is
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
crash that keeps the plugin enabled. Answers of any size
(`a_plugin_larger_than_one_message_is_listed_shown_and_edited`): a plugin
of 40 actions with 20 outputs each and a 200 KiB icon, listed, shown and
edited in full through paged answers that each fit one message, and the
same agent's shortened `listPlugins`, `getPlugin` and change answers for
an app that does not page, on a connection that stays open. Plugin data (`plugin_data_is_isolated_bounded_revocable_and_secret`,
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
a route may answer late with `delayMs`, and stops waiting when the call is
interrupted; the log records what would go on the wire, injected
credentials included).
`cargo test -p deckpilot-agent --features plugins --test plugins_examples`
installs the example plugins (`plugins/examples/hello-world`,
`developer-tools`), packaged from their directories with the publishing
tool (twice, compared: deterministic), from a registry with throwaway keys,
and runs every action through the real agent and host: Hello World
enabled with no permission; Developer Tools with exactly its three grants,
its settings, the token injected as a bearer only towards
`api.github.com` (checked on the wire) and never in an answer, a log or a
file, both variables resolved, storage read back, a revoke of the
credential slot's host that makes only that slot unusable (settings and
storage keep working) and a grant that restores it, and its
`listPlugins`/`getPlugin` answers at most half of the IPC message limit
(so an app that does not page gets them whole).
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

- On macOS a package with the host (`DECKPILOT_RELEASE_PLUGINS=1`) and a
  signing identity signs the host with the hardened runtime and its one
  entitlement, `com.apple.security.cs.allow-unsigned-executable-memory`,
  which wasmtime's executable memory needs; the app and the agent keep
  none ([distribution contract](distribution.md#macos-the-plugin-hosts-entitlement)).
  Verified with ad hoc signatures (`plugins/runtime/scripts/macos-hardened-runtime.sh`
  and a local package); a Developer ID signed and notarized package with
  the host has not been built, and the published release workflow does not
  package the host yet.
- The registry trust state lives in a file, not in the encrypted database
  ([why](#trust-state-stays-a-file)).
- No production root keys and no default trust yet: builds can embed root
  keys ([how](#enabling-the-public-registry-in-release-builds)), but none
  exist until the key ceremony (U-10), so without `--plugin-registry-roots`
  nothing is fetched.
  The GitHub asset redirect hosts for packages come from the design, whose
  §20.1 lists the release-asset redirect host as unverified; no test
  reaches GitHub.
- No periodic refresh: the registry is checked at unlock and on request,
  so a long-running agent goes stale after 7 days without a browse,
  install or update. No `If-None-Match`/ETag, no package cache (offline
  installs are not possible), no proxy configuration beyond the
  environment.
- No background work; its capabilities are refused.
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
  credential slot can only name exact hosts, or a server the user entered
  and allowed (G2, `userOrigin`).
- The app checks only what it cannot send (a whole number that is not
  one, an empty or too long secret); every other rule is the agent's, and
  its message is shown under the field it names.
- The JavaScript SDK skeleton (`plugins/sdk-js`, declarations only, no
  runtime) still declares the design's synchronous `ctx.settings`; the
  ABI delivers settings through `settings.get`, which a JavaScript shim
  would call at the start of an invocation.
- Plugin variables: only `machine` scope can be published (profile and
  project values need the `profile` permission and a scope argument, design
  §6 `VariablesApi.publish`); no reading of other owners' variables
  (`variables` permission); values are not persisted across agent restarts
  (as designed), and `plugin_known` records actions, not variables, so the
  picker cannot list variables of plugins that are not installed.
- A revoke does not interrupt a capability call that is already running;
  it denies the next one.
- No crash-loop end-to-end test: the five crashes with backoff take about
  15 s. The breaker is tested in the supervisor (`crates/supervisor`); the
  agent's reaction (`lane_ended`: disable, drop values, `changed`) is not
  tested end to end.
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
