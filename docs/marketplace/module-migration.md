# Moving built-in modules to plugins

Status: **proposed** (design record; no code changes). It applies to the
experimental plugin marketplace. Nothing here changes a release build until
the PRs in [§9](#9-pr-sequence) land, and several steps wait on owner
decisions ([§10](#10-blockers-and-owner-decisions)).

**Owner decision of 2026-11-20.** Git, Docker, Workspace, Projects,
Workflows, GitHub, GitLab and Jira are extracted into first-party plugins,
and B4 of 2026-11-15 is superseded. The decision record is
[§11](#11-owner-decisions-of-2026-11-20). The written security reviews for
the new host capabilities are in
[§7, plugin API 1.9 reviews](#plugin-api-19-security-reviews-2026-11-20).
Where this document says that one of those modules "stays built-in" or
"stays core" (§1, §3.3, §5.7, G12, G13, G15), §11 governs.

The evidence, module by module, with `file:line` citations, is in
[module-matrix.md](module-matrix.md). This document makes the decisions.
Line numbers are from `origin/main` at `21fc2de2`. The capability baseline
also includes the open pull requests #231 (plugin variables, cancellation)
and #232 (storage, credentials as handles, settings). The release separation
rules come from #225 (`docs/marketplace/distribution.md`,
`tools/release-pipeline/check-assets.sh`,
`tools/release-pipeline/check-release-separation.py`). The plugin design is
`docs/marketplace/architecture.md` on #191, especially §2.3 and §5.

## 1. Summary

| Question | Answer |
| --- | --- |
| What can move now? | Nothing, without new host capabilities. The closest are `github`, `gitlab` and `jira`: HTTPS and JSON with a token. With #232 they lack only `openUrl`, a few response headers and user-entered origins |
| What stays in the core? | `general`, `projects`, `work-session`, and the launcher half of `system` (`open-app`, `open-url`, `open-file`, `open-directory`). Also the HTTP transport, which moves out of `modules/http` into a core crate, because the plugin broker already uses it |
| What stays built-in but optional? | `system.run-command`, `system.process-status`, `git`, `docker`, `workspace`, `http`. Each needs process execution, the file system, the Docker daemon or free-form network authority, which cannot be granted securely or would add no isolation ([§7](#7-sdk-gaps)) |
| Default bundled plugin set | `deckpilot.notifications`, once `notify` carries severity (G9) and a plugin can read its own run's summary (G8). Nothing in `system` or `general` qualifies |
| Identity of first-party plugins | The reserved publisher `deckpilot`: `deckpilot.github`, with actions `deckpilot.github.pull-requests`. The names `core`, `project` and `resource` under it are reserved forever |
| Compatibility | A static **alias table** in the core maps each legacy action type (`github.pull-requests`) to its plugin action type. Routing happens when the action registry is built. Stored profiles are **never rewritten**, so an older app still finds its built-in ids |
| Migration | Idempotent and journaled per module in `plugins.db`. It never writes `deckpilot.db` and never deletes `modules` or `credentials` rows. The built-in keeps running until the migration has finished and the plugin is healthy, and it takes over again whenever the plugin breaks |
| Bundled trust | Pin the sha256 of every bundled plugin in the agent binary at build time. The digests are trusted as much as the agent itself, need no new key, and do not wait for U-10 or U-12 |
| First module | `github`. Opt-in, read-only, a fixed default origin, one bearer token, and every legacy behaviour reproducible after G1 and G6. `http` is not a good first module ([§6](#6-the-first-module)) |
| Installer size | Moving the optional modules out saves about 1 MB per installer when plugins are enabled. The plugin host adds about 12 MB uncompressed. Size is not a reason to migrate ([§8](#8-installer-size)) |

## 2. Why, and what "plugin" means here

A plugin is an ABI 1 wasm module run by the sandboxed, out-of-process
`deckpilot-plugin-host` (`docs/marketplace/artifact-contract.md`). The host
has no WASI, no clock, no file system, no process and no clipboard access
(`plugins/runtime/crates/abi/src/capability.rs:1-7`). It reaches the
outside world only through `host_call`. Each call is authorized by the
broker against the stored consent and executed by the agent
(`apps/agent/src/plugins/executor.rs`).

A built-in module is compiled-in Rust. Its capabilities "protect against
mistakes … it is not a sandbox against hostile code"
([modules.md](../modules.md), "Security boundary"). Moving a module into a
plugin therefore buys three things: isolation of its parsing and logic from
the agent, a release cycle independent of the application, and the user's
right to remove it. It buys nothing when the module's job is the dangerous
operation itself, such as running `git` or `docker`. Then the sandbox would
only hold a formatter, and the host would hold all the authority.

## 3. The core boundary

These stay in the core, and the core never depends on a plugin: device
management, the agent lifecycle, profiles and layouts, the workflow engine,
action execution, the plugin runtime and broker, the Marketplace client,
the encrypted configuration and updates.

The dependency graph is one-directional:

```text
deckpilot-core  ◀── deckpilot-modules (built-ins) ◀── apps/agent, apps/desktop
      ▲                                                   │
      └──────── alias table (strings only) ───────────────┘
plugins (wasm) ── host_call ──▶ broker ──▶ executor (agent) ──▶ core services
```

The alias table ([§5.2](#52-action-type-aliases)) lives in
`deckpilot-core` as data. It names plugin action types as strings and never
links plugin code. When a target is not installed, the legacy id resolves
as unavailable, exactly as for any absent module. One current edge has to
be cut: the agent's plugin executor imports
`deckpilot_module_http::{client, policy}`
(`apps/agent/src/plugins/executor.rs:110-111`), and so do `github`, `gitlab`
and `jira`. The transport moves to a core crate (`crates/deckpilot-net`)
before `http` or its dependants change ([§9](#9-pr-sequence), P2).
Similarly, `workspace` depends on the `system` crate for path helpers
(`modules/workspace/src/actions.rs:13-14`). Those helpers move to the core
next to `launch` if `workspace` ever moves.

### 3.1 `general`, action by action

| Action | Decision | Why |
| --- | --- | --- |
| `general.wait` | core | The workflow engine's delay and the reference cancellable node (`modules/general/src/lib.rs:173-256`). A guest has no clock; a plugin would need a host sleep (G10) to do what the core does in 40 lines |
| `general.log` | core | The diagnostic action and the probe action of the agent's tests (`apps/agent/tests/agent.rs:133`). It reads the profile and project for its placeholders (`lib.rs:143-160`), which plugins cannot (G8). It is also the target of the schema 3 rewrite of `deckpilot.core.log` (`crates/deckpilot-storage/src/legacy.rs:146`) |

Both stay. The crate is 113 KB as an rlib with no dependencies, so moving
it saves nothing and adds an IPC hop to every log line.

### 3.2 `system`, action by action

| Action | Decision | Why |
| --- | --- | --- |
| `system.open-app` | core | Launching bound applications is the product's base function. It needs machine resource bindings, `canonicalize` and the OS launcher (`crates/deckpilot-core/src/launch.rs:83-118`), which is already core code |
| `system.open-url` | core | Arbitrary user-entered http, https and mailto URLs (`modules/system/src/support.rs:209-223`). A plugin `openUrl` grant is limited to listed hosts (architecture §5.2), so the plugin equivalent would be an "any host" grant, a broad permission with no isolation gained |
| `system.open-file` | core | As `open-app`, plus a path inside a bound location |
| `system.open-directory` | core | As `open-file` |
| `system.run-command` | built-in, optional | Arbitrary process execution with a cleared environment, off until `allowCommands` (`modules/system/src/lib.rs:92-112`, `src/actions.rs:510-518`). It cannot be offered to a sandbox: the capability *is* code execution (G13) |
| `system.process-status` | built-in, optional | Process names and PIDs through `sysinfo` (`src/actions.rs:657-670`). It could become a plugin with a narrow inspection capability (G11), but the gain is small. It is the only user of `sysinfo` (828 KB rlib) |

`system` stays one module. Splitting it would change the owner of two
action types for no gain: `run-command` is already off until the user
allows it on this computer, and `process-status` only reads process names.

### 3.3 `projects` and `work-session`

Both are core. `projects` returns `ContextEffect`s
(`crates/deckpilot-core/src/execution.rs:385-402`), and `ProjectContext` is
never grantable (`plugins/runtime/crates/broker/src/permissions.rs:31`). The
core's profile model, label templates, step conditions, IPC requests and
three storage tables depend on it ([modules.md](../modules.md), "Remaining
dependencies of the core on projects"). `work-session` is a view of state
the agent owns, in core schema 9 tables. Both declare the only module
variables and the only `ALIASES` (`crates/deckpilot-core/src/variables/registry.rs:53,309-350`).
So **no variable needs to migrate**, and the variable alias design in
[§5.4](#54-variables) is a rule for later modules, not work for now.

*Superseded for `projects` (2026-11-20, [§11](#11-owner-decisions-of-2026-11-20)).*
The user-facing part of `projects` moves to `deckpilot.projects`: its
actions, its variables, the management view, the status item, the
dashboard card, cycling and the `wrapAround` setting. The project data
model and its storage stay in the core as a neutral *project context*
primitive, because the profile model, the step conditions, the FKs of
schemas 8 and 9, portability and IPC need it. The plugin changes the active
project only through the reviewed `projectContext` operations
([X6](#x6-project-operations-projectcontext)). `work-session` stays core,
unchanged. The `projects.current.*` variables now migrate as §5.4
describes.

## 4. The default system plugin set

### 4.1 What ships bundled

| Plugin | From | Condition | Permissions |
| --- | --- | --- | --- |
| `deckpilot.notifications` | `notifications.show`, `notifications.workflow-summary` | G8 (run summary of the invocation's own workflow) and G9 (`notify` with severity and critical) | `notifications` (low tier) |

`system` and `general` contribute nothing ([§3](#3-the-core-boundary)). The
set is deliberately small.

*As built (P10):* `deckpilot.notifications` (`plugins/first-party/notifications`)
is bundled with class `default` and is the only member of the set. Its one
permission is `notifications` with a routing (G9), low tier, so seeding
enables it at the first start; see [§9](#9-pr-sequence) P10. A bundled plugin must not need a permission the
user has not seen, and only `notifications` fits that today. Optional
integration plugins (`deckpilot.github` and later ones) can use the same
mechanism as **migration payloads** ([§4.3](#43-bundled-classes)), but they
are not part of the default set.

### 4.2 Requirements

- **Offline install from the application bundle.** The plugins ship inside
  the installer as app resources and are installed by the agent from there,
  with no network access and no registry.
- **The normal lifecycle.** A bundled plugin is an ordinary installed plugin
  in `plugins.db` (states, versions, grants, audit, journal;
  `crates/deckpilot-plugin-store/src/db.rs:28-100`) with a new install source
  `bundled`. Disable, uninstall, rollback, quarantine and the Activity view
  work as for any plugin.
- **Available on first launch.** The agent seeds bundled plugins right after
  the plugin store opens, before the module registry is built for the first
  time.
- **Independent updates.** Once the registry is live (U-10, U-12), a
  registry version newer than the bundled one updates it through the
  normal update path, including escalation consent (architecture §13.2). A
  newer app may carry a newer bundled version, which updates an installed
  *bundled-source* copy only. A bundled version never replaces a newer
  registry install and never downgrades.
- **No silent broad permissions.** Seeding grants only low-tier permissions,
  recorded with `source = 'derived'` and `decided_by = 'bundled:<app version>'`
  (`plugin_grants.source` already allows `derived`, `db.rs:57-70`). Seeding
  never grants a medium- or high-tier permission. A bundled plugin that
  declares one is installed **disabled** until the user consents in the
  normal dialog.
- **Explicit choices stick.** Uninstalling or disabling a bundled plugin
  writes a tombstone. Seeding checks the tombstone first and never
  reinstalls or re-enables. Only the user can undo it: enabling a disabled
  plugin again, or **Restore** under *Removed DeckPilot plugins* in
  Settings › Plugins › Installed for an uninstalled one
  (`restoreBundledPlugin`, protocol 15), which clears the tombstone and
  installs the pinned copy from the application bundle exactly as seeding
  does (the same pin and seal checks, low-tier grants only, the rest
  waiting for consent), with the settings and credentials kept at the
  uninstall. A refused check changes nothing: the tombstone stays. A broken
  bundled copy gets **Reinstall** there, the same request: the broken copy
  is replaced by the bundle's, with its settings, credentials, grants and
  revocations, and no tombstone is written. The module's routing goes from
  `PluginWithdrawn` back to `Plugin` with the restore.

### 4.3 Bundled classes

| Class | Seeded on a fresh install | Seeded on an upgrade | Example |
| --- | --- | --- | --- |
| `default` | yes | yes, unless tombstoned | `deckpilot.notifications` |
| `migration` | no | only when the user's data uses the legacy module ([§5.6](#56-the-migration)) and it is not tombstoned | `deckpilot.github` |

The `migration` class makes upgrades work offline and without registry
keys. A fresh install that wants GitHub actions gets `deckpilot.github` from
the Marketplace. Until the registry is live, the built-in stays the
fresh-install path ([§5.8](#58-fresh-installs)).

### 4.4 Who signs bundled plugins

The registry roles (root, index and timestamp keys) are not approved yet
(U-10, U-12). The options:

| Option | How | For | Against |
| --- | --- | --- | --- |
| **A. App code signature** | Trust files because they sit inside the signed `.app` or MSI. On macOS, check the bundle seal (`SecStaticCode`) at seeding | No new key; the OS already verifies the bundle | Only macOS and signed Windows installs have an enforceable seal. AppImage, `.deb`, `.rpm` and per-user NSIS installs have none the agent can check. Platform-specific code in a security path |
| **B. Digests pinned in the agent** (recommended) | The release build writes `bundled/pins.json` (id, version, class, sha256 and size of `manifest.json` and `plugin.wasm`). The agent embeds the expected digests at compile time (`include_bytes!` of the pins file, or a generated constant), and verifies the files against them before seeding and at every load (`verify_installed`) | Same trust as the agent binary, which is already covered by the updater signature and the OS signature where one exists. Works offline on every platform. No key ceremony. Independent of U-10/U-12 | A bundled plugin changes only with an app release, which is intended: it is an app component. Anyone who can replace the agent binary can replace the pins, but they could replace the agent anyway |
| C. A third minisign key, "bundled root" | Sign bundled plugins in the app's release workflow with a separate key | Uniform with registry verification | One more key ceremony and secret. A plugin signature produced in this repository is what `check-release-separation.py` forbids (`PLUGIN_SIGNING`, `REGISTRY_SECRET`). It adds nothing over B, because the key would sit in the same environment as the app's signing |
| D. Registry keys | Ship registry-signed `.dpplugin` packages | One trust path for every plugin | Blocked on U-10/U-12. It would make an app release contain independently distributed plugin packages, which the release separation forbids |

**Recommendation: B, plus A where the platform offers it** (defence in depth
on macOS). After U-10/U-12, registry-published versions of `deckpilot.*`
plugins are verified by the registry roots as usual. The bundled copy stays
trusted by its pin. Two trust roots for one plugin id are acceptable because
each covers different bytes: the pinned bundled version, and registry
versions newer than it.

The agent must also refuse any `deckpilot.*` id from a source other than
`bundled` or the registry with the verified `deckpilot` publisher: no
sideload, no dev install. Otherwise a sideloaded "first-party" plugin could
capture alias routing, and with it the user's legacy actions and credential
import ([§5.2](#52-action-type-aliases)).

### 4.5 Bundled plugins and the release separation

`docs/marketplace/distribution.md` (#225) forbids two things in an
application release: plugin packages (`*.dpplugin`) and registry metadata
as release assets, and plugin signing in this repository. Bundled default
plugins are **app components**, like the plugin host. The difference must
be precise and checkable:

| | Independently distributed plugin | Bundled plugin (app component) |
| --- | --- | --- |
| Form | `<id>-<version>.dpplugin`, a signed package | Unpacked files `bundled/<id>/manifest.json`, `plugin.wasm`, `LICENSE`, inside the installer's resources. Never a `.dpplugin`, never an archive of its own |
| Where | GitHub Releases of `knittingcode/deckpilot-plugins` | Inside the `.app`, MSI, NSIS, AppImage, `.deb`, `.rpm`; never a release asset |
| Built by | the registry's CI from a pinned commit | this repository's release build, from source in this repository (`plugins/first-party/<name>`), with the guest profile |
| Trusted by | registry signatures | digests pinned in the agent (option B) |
| Updated by | the registry | an app update, or a newer registry version (§4.2) |

The release checks must allow exactly this and nothing more:

- `check-assets.sh`: **unchanged.** Bundled plugins are never release assets.
- `check-release-separation.py`: unchanged rules. The bundle step stages
  files under `bundled/`, which match none of the registry patterns
  (`\.dpplugin\b`, `site/v1`, `roots.json`, `timestamp.json`, `index/`,
  `keys/`, `.minisig`). Add an `accept` fixture for a Tauri `resources`
  entry of `bundled/**` next to the existing `plugin-host-app-component.yml`,
  and a `reject` fixture for a `.dpplugin` staged into the bundle.
- A new check, `tools/release-pipeline/check-bundled-plugins.sh` (stage A):
  every directory in `bundled/` is listed in `bundled/pins.json`; every
  listed id starts with `deckpilot.`; digests match the files; the pins
  file matches the digests compiled into the agent (the agent prints them
  with a hidden `--print-bundled-pins`); there is no `.dpplugin`, no
  `.minisig` and no file outside the allowed names.

The pins file is deliberately not called `index.json` and the directory is
not `index/`, so no registry pattern can match it by accident. The accept
fixture pins this.

**As built (P5).** `plugins/first-party/build.sh` writes the layout and the
pins; `crates/deckpilot-plugin-bundled` holds the pins format, the pins
compiled into the agent (`DECKPILOT_BUNDLED_PINS` at build time) and the
verification; `apps/agent/src/plugins/bundled.rs` seeds. Two refinements
of the above: a bundled plugin also carries the icons its manifest names
(`assets/<name>.png`, pinned like every other file), because the manifest
contract requires `icons.128`; and `check-release-separation.py` gained a
rule (stricter, nothing relaxed) that refuses any step staging a
`.dpplugin` into the bundle (`src-tauri/` or `bundled/`). The host and
`bundled/` are packaged with `tauri.plugins.conf.json` on top of
`tauri.release.conf.json`: by the release workflow and the *Packaging*
workflow's package checks for every installer
([releasing.md](../releasing.md#packaging-the-plugins)), and locally by
`deckpilot-dev package` with `DECKPILOT_RELEASE_PLUGINS=1`.
`tools/ci/check-packages.sh` requires the host and exactly the release set
of `bundled/` in every package. See
[`plugins/first-party/README.md`](../../plugins/first-party/README.md).

## 5. Compatibility and migration

### 5.1 Identity

- First-party plugins use the reserved publisher **`deckpilot`**:
  `deckpilot.<module>` for the plugin, `deckpilot.<module>.<action>` for its
  action types, and `plugins.deckpilot.<moduleCamel>.*` for its variables.
  All are valid today: `ModuleId::new` accepts a publisher of 2–24 and a
  name of 2–32 characters (`crates/deckpilot-core/src/module.rs:69-97`), and
  `owner_of` splits at the last dot (`:127-130`).
- **Reserved names.** `deckpilot.core`, `deckpilot.project` and
  `deckpilot.resource` are never assigned. Before schema 3,
  `deckpilot.core.log`, `deckpilot.project.cycle`, `deckpilot.project.select`
  and `deckpilot.resource.resolve` were stored action types
  (`crates/deckpilot-storage/src/legacy.rs:146-177`). An old export that
  still contains them must never resolve to a plugin. The registry refuses
  the names, the agent refuses to install them, and a core test pins the
  list.
- The publisher `deckpilot` is reserved in the registry for the owner
  (`knittingcode`), and the agent enforces [§4.4](#44-who-signs-bundled-plugins)'s
  source rule.
- Action names keep the legacy name: `github.pull-requests` becomes
  `deckpilot.github.pull-requests`. Parameter keys and output keys stay
  identical. This is a **contract**, checked by a test
  ([§5.3](#53-parameters-outputs-and-node-specs)).

Alternatives considered: giving the plugin the legacy namespace (impossible,
because plugin ids always contain a dot and built-in ids never do), and
rewriting stored ids (rejected, because it breaks downgrades, see
[§5.9](#59-rollback-and-downgrade)).

### 5.2 Action type aliases

A new core table, with the same shape as the variable `ALIASES`
(`crates/deckpilot-core/src/variables/registry.rs:309-350`):

```rust
/// A legacy built-in action type and the plugin action type that replaces it.
pub struct ActionAlias {
    pub legacy: &'static str,        // "github.pull-requests"
    pub target: &'static str,        // "deckpilot.github.pull-requests"
    pub module: &'static str,        // "github": the legacy owner
    pub plugin: &'static str,        // "deckpilot.github"
    pub since: &'static str,         // the app version that added it
    pub params: &'static [ParamRule],// usually empty (§5.3)
}
pub const ACTION_ALIASES: &[ActionAlias] = &[ /* empty until a cutover */ ];
```

**Routing.** The agent passes the core a `LegacyRouting` (module id →
`Builtin` | `Plugin`), computed from the migration records
([§5.6](#56-the-migration)). The core never reads `plugins.db`.
`ModuleRegistry::action_registry` (`crates/deckpilot-core/src/module.rs:849`)
then registers, for each alias:

| Routing | Built-in compiled in | Target registered | Result for `legacy` |
| --- | --- | --- | --- |
| `Builtin` | yes | any | the built-in handler (today's behaviour) |
| `Plugin` | any | enabled and available | an `AliasHandler` that delegates to the target's handler, with the target module's capabilities |
| `Plugin` | yes | broken, quarantined, incompatible or missing because of a system fault | the built-in handler, with a warning in the Activity view (fallback) |
| `Plugin` | any | disabled or uninstalled **by the user** | unavailable: "GitHub (`deckpilot.github`) is disabled". The user's choice is respected; there is no fallback |
| `Plugin` | no | not registered | unavailable, naming the plugin, with "install from Marketplace" |

`ActionRegistry::dispatch`, `definition`, `check_parameters` and
`decorate_face` (`crates/deckpilot-core/src/registry.rs:204,315`) look up the
legacy id as today and find the `AliasHandler`. Nothing in the engine
changes. `ModuleRegistry::availability` (`module.rs:823-841`) consults the
alias table before `owner_of`, so `github.pull-requests` reports the
plugin's availability once the `github` module is gone. The capability
check of `dispatch` uses the target's capabilities, and the effect check
applies unchanged.

**Catalog and editors.** The catalog offers only the canonical
`deckpilot.github.*` types for new instances. Legacy types are hidden like
`deprecated` ones (`crates/deckpilot-core/src/action.rs:351`) but remain
editable. Their definition is the target's, under the legacy id. Clients
below protocol 10 never see plugin types (`Response::for_protocol`, #191
§2.3 item 7). For them the agent presents alias-routed legacy types as
ordinary built-in types, so an older app can still edit and run them.

**Why not rewrite.** Stored documents keep `github.pull-requests`. An older
app, which has the built-in, keeps working on the same database. A newer
app routes the same id to the plugin. A future major version may offer an
explicit, user-started "update action types" command with a backup. It is
never automatic.

### 5.3 Parameters, outputs and node specs

Overrides (`BindingTarget::Action.overrides`, `NodeKind::Action.overrides`
and `mappings`) are keyed by parameter key, and downstream nodes read
outputs by key ([module-matrix.md](module-matrix.md#where-action-type-ids-are-stored)).
So a replacement plugin **must** declare:

- the same parameter keys with compatible kinds and constraints (same
  choices; ranges at least as wide);
- the same output keys, kinds and `sensitive` flags;
- a node spec at least as permissive: the same `effects`, `retrySafe` and
  `idempotent`, a timeout at least as long, and `cancellable` wherever the
  built-in is. The agent makes every plugin action cancellable (it can
  always stop the plugin), so a plugin may replace an action that cannot be
  interrupted, such as `github.open`; never the other way.

`deckpilot-modules/tests/contracts.rs` gains a test that loads each aliased
plugin's manifest (from `plugins/first-party/<name>/manifest.json`) and
compares it with the built-in's `ActionType` while the built-in is still
compiled in.

`ParamRule` exists for the one unavoidable difference: parameter kinds that
plugins cannot have (`credential`, `resource`; `crates/deckpilot-plugin-manifest/src/model.rs:405-460`).
The only one the migratable modules use is the optional `credential`
override (`modules/github/src/actions.rs:89-97`, and the same in gitlab and
jira). The rules:

- `DropIfDefault("credential")`: an instance whose `credential` is empty or
  equals the module's configured credential is translated by dropping it.
  The plugin uses its slot.
- Any other value makes the module **not eligible** for migration on this
  computer ([§5.6](#56-the-migration)) until G5 (a `credentialSlot`
  parameter kind) exists. The built-in keeps running, and the migration
  note names the action.

The translation runs on the parameters *after* overrides and mappings are
applied, so it covers instances, bindings and workflow nodes alike. While
the plugin serves the legacy type, a value it cannot be given is also
refused when an instance, a step or a node is **saved**
(`ActionHandler::validate` of the `AliasHandler`, reported by
`ActionRegistry::normalize` as `invalid parameters`, the error of any
built-in's invalid parameter), whatever the plugin's capabilities.

### 5.4 Variables

No migratable module declares variables. The rule for later ones: a module
variable `x.y` that moves becomes `plugins.deckpilot.<name>.y`. A static
entry in `ALIASES` maps it, with the plugin as owner, so references resolve
without running plugin code and show as `Unavailable { owner }` while the
plugin is absent (#231's grammar). The `MODULE_NAMESPACES` entry stays, so
old references keep parsing.

### 5.5 Settings, credentials and enabled state

| Legacy | Plugin | Rule |
| --- | --- | --- |
| `modules` row `config` (`crates/deckpilot-storage/src/legacy.rs:67-71`) | `plugin_settings(plugin_id, values_json)` in `plugins.db` (#232) | Copy keys one to one, keeping each key's portability. Write only if the plugin has no saved settings yet. **Never** modify or delete the `modules` row |
| A setting holding a credential id (`github.credential`, default `github.token`) | `plugin_credentials(deckpilot.github, token)` | Copy the secret only if the slot is empty, and only after the user confirms (high tier, below). **Never** delete or change the `credentials` row: `http.request` or another workflow may name the same credential, and an older app needs it |
| Origin settings (`apiUrl`, `siteUrl`) | a `network` grant for that exact origin | Eligible only if the origin is the plugin's declared default, or G2 exists. `allowPrivateHost = true` is never migrated automatically (G3, high tier). *As built (P11):* for a plugin with a user-origin slot, the server the user entered is copied into the plugin's setting and **asked for**, exactly it; its private network is a separate item ([§6](#6-the-first-module), "the GitLab and Jira cut-overs") |
| `modules.enabled` | plugin state `enabled` or `disabled` | Copied once |

**Consent.** The built-in could use the stored token. The plugin gets a
grant no wider than that: one exact origin, one slot, the declared
schemes. Grants for low- and medium-tier permissions that the built-in
demonstrably used on this computer (its configured origin for GET, `openUrl`
for its web host) are recorded as `derived` with `decided_by =
'migration:<module>'`, and the user is told in a post-migration notice with
a one-click revoke. **High-tier grants, which are credentials and private
network, are never derived.** Until the user confirms them in a one-time
dialog ("GitHub actions now run as a plugin. Allow it to use your stored
token `github.token` for api.github.com?"), the module stays on the
built-in. That keeps every capability working and grants nothing silently.

**Exports and imports.** Exports keep writing legacy ids in profiles, as
stored. An imported old payload with `modules.github` settings is applied
to `deckpilot.github` through the module alias (`ActionAlias.module →
plugin`) when the plugin is installed, after portability checks. Otherwise
it is reported as an absent module (today's behaviour,
`crates/deckpilot-portability/src/import.rs:317-364`). Plugin secrets and
grants are never exported, as for every plugin.

### 5.6 The migration

One record per module in `plugins.db` (schema 3, [§9](#9-pr-sequence) P3):

```sql
CREATE TABLE module_migrations (
    module_id     TEXT PRIMARY KEY,      -- 'github'
    plugin_id     TEXT NOT NULL,         -- 'deckpilot.github'
    phase         TEXT NOT NULL CHECK (phase IN ('pending', 'not_eligible',
                      'installed', 'settings_copied', 'awaiting_consent',
                      'cut_over', 'failed')),
    reason        TEXT,                  -- why not eligible or failed; no values
    attempts      INTEGER NOT NULL DEFAULT 0,
    source_digest TEXT,                  -- sha256 of the modules row and credential ids, never secrets
    app_version   TEXT NOT NULL,
    updated_at    INTEGER NOT NULL
) STRICT;
CREATE TABLE plugin_tombstones (
    plugin_id  TEXT PRIMARY KEY,
    decision   TEXT NOT NULL CHECK (decision IN ('uninstalled', 'disabled')),
    decided_at INTEGER NOT NULL
) STRICT;
-- and plugins.source gains 'bundled' (a table rebuild: CHECK constraints
-- cannot be altered in SQLite; db.rs:36)
```

**Steps**, run by the agent after the plugin store opens and before the
module registry is built. Each step checks its target first, so a rerun is
harmless:

1. **Eligibility.** The module is registered as a built-in alias target in
   `ACTION_ALIASES`. The user's data uses it: it is enabled, and there is an
   instance of its types or a saved `modules` row. No instance fails a
   `ParamRule`. Its origins and private-network settings are migratable
   (§5.5). Otherwise the phase is `not_eligible` with a reason, and the
   built-in stays.
2. **Install** the `migration`-class bundled package, or keep an installed
   registry version of the same id if it is not older than the bundled one.
   This is one plugin-store transaction (journaled, crash-safe,
   `crates/deckpilot-plugin-store/src/recover.rs`). Skipped if a tombstone
   exists: the phase becomes `not_eligible` ("removed by you").
3. **Settings.** Copy them if `plugin_settings` has no row, and record
   `source_digest`.
4. **Grants and credentials.** Record derived grants. If a high-tier grant
   or a credential copy is needed, the phase is `awaiting_consent` and the
   app shows the dialog. Then copy the secret into the slot if it is empty.
5. **Cut over.** The phase becomes `cut_over`. The routing for the module
   becomes `Plugin`, and the registry is rebuilt.

**Re-runs and drift.** At every start, a `cut_over` module whose
`source_digest` changed, because the user went back to an older app and
edited the built-in's settings, is reconciled. If the plugin's settings are
unchanged since the copy, the settings are copied again. Otherwise the
plugin's settings are kept and a migration note says so. Nothing is ever
merged silently.

**Failure and recovery.** Any failed step sets `failed` with a reason and
increments `attempts`. The routing stays `Builtin`, so the user's keys keep
working. The step is retried at the next start, up to 3 times, then only
when the user presses Retry in Settings › Plugins. A plugin store that
cannot be opened (wrong schema, damaged, locked;
`apps/agent/src/plugins/service.rs:294-302`) means `Builtin` for every
module. The agent never fails to start because of a migration.

**Offline.** Every step is local: bundled files, `deckpilot.db`,
`plugins.db`. The registry is consulted only for an optional newer version.

**Never delete.** No step deletes or rewrites a `modules` row, a
`credentials` row, a profile document or a built-in's tables. The legacy
data is the downgrade path.

**As built (P6).** The engine is `apps/agent/src/plugins/migration.rs`.
The agent's service task reads what it needs from `deckpilot.db` (read
only, and nothing at all while `ACTION_ALIASES` is empty) whenever it
loads the modules and hands it to the plugin worker, which owns
`plugins.db`; the first snapshot after the plugin store opened runs the
steps. The worker computes the `LegacyRouting` from the journal after
every change, and `load_modules` sets it on the module registry before the
action registry is built. Details beyond the text above:

- Per-module knowledge the alias table does not carry (which settings name
  the credentials, the origin and the private-network switch, and the
  plugin's slot for each credential) is a static `MigrationPlan` next to
  the engine, one per alias block (GitHub's since P9, §6).
- Derived grants: the low tier, and `network` for the built-in's (default)
  origin and `openUrl` for its web hosts when the built-in has the
  capability; recorded with `source = 'derived'`, `decided_by =
  'migration:<module>'` through `PluginManager::derive_grants`, which
  refuses the high tier and never overwrites a stored decision (a
  revocation stays). Anything else waits in `awaiting_consent`, and so
  does every credential import: a secret is copied only when the slot's
  grant was made by the migration's consent (`decided_by =
  'user:migration:<module>'`), never because of a grant made elsewhere.
- `source_digest` is the sha256 of the copied settings, the credential
  ids and the built-in's eligibility settings (origin, private network).
  At a start, a changed digest copies the settings again when the
  plugin's still match the recorded digest and keeps them (with a note in
  `reason`) otherwise; a source that is no longer eligible goes back to
  `not_eligible`, so the built-in serves again.
- The health check before the cut-over: the plugin is enabled, its files
  match the pins, it declares every alias target with a contract that can
  replace the built-in's (`alias::incompatibilities`), and its required
  settings are set. A failed check disables the plugin again (the agent's
  own disable, no tombstone) and records `failed`.
- Routing: `Plugin` after the cut-over, `PluginWithdrawn` while the
  plugin has a tombstone, and `Builtin` for a bundled copy this agent does
  not pin (a build without the plugin) and in every other phase.
- The tests use a fixture legacy module and alias (feature `test-hooks`,
  never in a release build) and the fixture plugin
  `plugins/first-party/test-fixtures/legacy-fixture`.

**As built (P7).** The app drives the consent and Retry through protocol
10 requests of the `plugins` capability
([agent-integration.md](agent-integration.md#module-migrations)):

- `listModuleMigrations` lists every journaled migration: phase, reason,
  attempts, the route of its legacy action types (`builtin`, `plugin`,
  `pluginWithdrawn` with the reason), its aliases, and the consent items:
  every permission it needs that is not granted, and every credential
  import, by the stored credential's id, the built-in's setting, the
  plugin's slot and the slot's hosts. Never a value.
- `decideModuleMigration` approves and declines items one by one. An
  approval is a grant decided by `user:migration:<module>`, the only
  decision that imports a credential. A decline is a refusal decided the
  same way: the module goes to `not_eligible` ("you declined …") and
  stays there at every start, with the built-in serving, until the user
  presses Retry. Low- and medium-tier grants stay derived as before;
  credentials and the private network are never granted without an
  approval.
- `retryModuleMigration` runs a `failed` migration again (attempts reset)
  and, for one the user declined, forgets those refusals
  (`PluginManager::forget_declines`, which never grants anything) so the
  dialog asks again.
- `pluginEvent` `migrationChanged { module, plugin, phase }` reaches only
  connections that subscribed with `subscribePluginEvents { migrations:
  true }`, so an older app never receives an event it cannot read.
- Bundled plugins report the origin `bundled`.
- The app (`VITE_DECKPILOT_PLUGINS=1`) shows the notice and the consent
  dialog in Settings › Plugins (`apps/desktop/src/plugins/migration*.tsx`).
  Its editors (`lib/plugin-actions.ts`) show a legacy type of a module
  routed to the plugin under the built-in module with the plugin's
  definition, deprecated, so the picker offers the plugin's type only; a
  module routed to its built-in is unchanged; a withdrawn plugin makes the
  legacy type unavailable with the agent's reason. The app's own save
  validation still uses the built-in's definition, which the contract
  (§5.3) keeps compatible.
- `DECKPILOT_TEST_MIGRATION_DECISIONS` stays for tests that decide before
  an agent starts.

### 5.7 Keeping optional modules: bundled legacy implementations

Until a module's replacement exists and a cut-over has succeeded on a given
computer, its built-in stays compiled in as the **legacy implementation**,
and it takes over whenever the plugin breaks (§5.2). It is removed from the
build only when all of the following hold:

1. the replacement has been the default route for two minor releases;
2. its contract test (§5.3) has passed in every release since;
3. the only remaining reasons for `not_eligible` are ones the user can
   resolve in the UI (for example, choosing a credential slot after G5).

After removal, `ACTION_ALIASES` keeps the entry forever, so the stored ids
keep resolving.

*Waived for the extraction set (2026-11-20, [§11](#11-owner-decisions-of-2026-11-20) D2).*
For `github`, `gitlab`, `jira`, `git`, `docker`, `workspace` and `projects`,
criterion 1 ("two minor releases as the default route") does not apply.
Two conditions replace it: the plugin covers every user (GitHub Enterprise
included, WP-B0), and the "core without optional plugins" check (WP-T1)
passes. Criterion 2 (the contract test) still applies. Criterion 3 is
replaced by accepting a known cost: a user whose migration is
`awaiting_consent`, `failed` or `not_eligible` sees recoverable Unavailable
keys and has no built-in fallback.

### 5.8 Fresh installs

A fresh install has no `deckpilot.db` before the first start, so no
`app_launches` and no profiles exist. It seeds the `default` class only.
During the transition, optional built-ins stay registered and enabled as
today, because there is no Marketplace to get their replacements from until
the registry is live (U-10, U-12). From the release in which the registry is
live, a fresh install registers optional legacy built-ins as disabled and
hides them from the catalog. The user installs the plugin replacements from
the Marketplace. So "a fresh install gets only the default set" is a release
gate, not a code switch.

### 5.9 Rollback and downgrade

| Database | What the migration changes | An older app |
| --- | --- | --- |
| `deckpilot.db` (schema 9) | nothing: no schema change, no rewritten document, no deleted row | opens it as before. Legacy ids find its built-ins; `modules` and `credentials` rows are as they were at migration time |
| `plugins.db` | schema 2 → 3 (`module_migrations`, `plugin_tombstones`, `source = 'bundled'`) | refuses schema 3 with `UnsupportedSchema` (`crates/deckpilot-plugin-store/src/db.rs:163`). The agent reports the plugin store as unavailable and runs without plugins (`apps/agent/src/plugins/service.rs:294-302`). Release builds include the `plugins` feature from the first release after v0.6.0 |

Turning plugins off on the same data (`--disable-plugins` or
`DECKPILOT_ENABLE_PLUGINS=0`, see [§5.10](#510-plugins-on-by-default))
behaves like an older app: `plugins/` is not opened, and the legacy ids
run their built-ins again.

So a **downgrade is safe**, with three documented losses:

1. Settings changed in the plugin after the migration are not seen by the
   older app.
2. Credentials entered only into a plugin slot are not in `credentials`.
3. Action instances created after the upgrade with canonical
   `deckpilot.*` types show as unavailable on the older app. They are kept,
   as for any absent module.

Upgrading again is idempotent (§5.6 "Re-runs and drift"). A test must pin
that a schema 2 build refuses schema 3 without writing, and that the agent
then starts with plugins disabled. The rule from
[data-model.md](../data-model.md) holds: there is no in-place downgrade of
either database. This design avoids needing one for `deckpilot.db`.

### 5.10 Plugins on by default

In a build with the agent's `plugins` cargo feature, plugins are **on**
without any argument (`apps/agent/src/plugins/mod.rs`,
`ENABLED_BY_DEFAULT`). The configuration app and the login item start the
agent without plugin arguments (`apps/desktop/src-tauri/src/agent.rs`,
`apps/agent/src/autostart.rs`), so the bundled plugins, their pins and the
migrations of this document take effect on the first start of such a
build, with the consent rules of §5.5 (B3: derived grants for low and
medium tiers; credentials and private networks wait for the user).

| Switch | Plugins |
| --- | --- |
| none (the app, the login item) | on with the feature, off without it |
| `--disable-plugins`, or `DECKPILOT_ENABLE_PLUGINS=0` | off: plugin requests are refused with `pluginsUnavailable`, `plugins/` is not opened, the legacy ids run their built-ins |
| `--enable-plugins` (kept for compatibility), or `DECKPILOT_ENABLE_PLUGINS=1` | on with the feature; nothing without it |

The command line wins over the variable, and of two switches the last.
A build without the feature is unchanged: it never answers plugin
requests, whatever the switches.

An update from 0.6.0 (an encrypted `deckpilot.db`, no `plugins/`) starts
like any other: locked, `getPluginRuntime` says plugins are on and every
other plugin request waits for the key (`locked`), creating nothing;
after the unlock the agent creates `plugins.db` with the key derived from
the configuration key, seeds the bundled plugins and runs their
migrations. `deckpilot.db` is not written, byte for byte, so 0.6.0 still
opens the same data. Tests: `plugins_notifications.rs`
(`a_0_6_0_data_dir_starts_with_plugins_on_without_a_switch`, from the
0.6.0 fixture of `crates/deckpilot-storage/tests/fixtures/v0.6.0`) and
`plugins_e2e.rs` (`plugins_are_on_unless_turned_off`, every switch and
value).

## 6. The first module

**Recommendation: `github`.**

| Criterion | `github` | `http` | `notifications` |
| --- | --- | --- | --- |
| Brokered HTTP suffices | yes for api.github.com with #232's credential handles | **no**: free-form and templated URLs, no origin at install (`modules/http/src/actions.rs:128-139,378`), private hosts by design, any credential per action, 10 MiB and 300 s against 512 KiB and 15 s | not network |
| New host work | G1 `openUrl` (designed in §5.2 of #191), G6 `link` and `x-ratelimit-*` headers | G2, G3, G4, G5, G6, and the transport refactor | G8, G9 |
| Blast radius | opt-in module, read-only actions, one token | many workflows; the transport is shared with the broker | used at the end of many workflows |
| Exercises | aliases, settings copy, credential import with consent, eligibility (GHE stays built-in), fallback | — | the bundled default path |

`http` is the obvious candidate on paper and the wrong one in practice. The
brokered fetch was built for a plugin that *declares* its hosts. `http`
exists to call hosts the user types per action, including localhost. A
plugin `http` would need an "any origin the user enters" grant. The broker
then becomes a pass-through, and the agent keeps the same authority. Its
client also *is* the broker's transport.

`github` keeps all its behaviour after two small, already designed host
features. Users with GitHub Enterprise, a private host or a non-default
credential override stay `not_eligible` and keep the built-in until G2, G3
or G5 lands, which demonstrates the eligibility path. GitHub pagination must
fit 512 KiB per response. With `per_page` of at most 50 a search page is far
below it, so G4 is not on the critical path.

*As built (P8):* `plugins/first-party/github` (`deckpilot.github`, class
`migration` in `bundled.json`) reproduces all six actions; no variables on
either side. `contracts.rs` (`first_party`) checks the manifest against the
built-in, and `apps/agent/tests/plugins_github.rs` runs every action through
the agent and the host against the built-in on the same canned answers,
with identical results. What differs, all on failure paths, limits or the
editor (save-time checks, two messages, `retryAfterSeconds`, transport
error codes, the `Date`-based time limit, the agent's rates for plugins),
is listed in [`plugins/first-party/github/README.md`](../../plugins/first-party/github/README.md).
Lists use pages of 50 (two for more than 50 pull requests, ten for the
built-in's 500 checks).

*As built (P9), the cut-over.* The first behaviour change, and only in
builds with the `plugins` feature, where plugins are on unless turned off
([§5.10](#510-plugins-on-by-default)); release builds include the
feature from the first release after v0.6.0 (v0.6.0 and older do not).

- `ACTION_ALIASES` (`crates/deckpilot-core/src/alias.rs`) has one block:
  `github.<action>` → `deckpilot.github.<action>` for `current-user`,
  `repository`, `pull-requests`, `review-requests`, `checks` and `open`,
  module `github`, plugin `deckpilot.github`, since `0.6.0`, with
  `ParamRule::DropIfDefault("credential")` on every action but `open`
  (which has no `credential`). `contracts.rs` (`first_party`) checks
  that the shipped block is exactly the one it validates.
- The `MigrationPlan` of `github` (`apps/agent/src/plugins/migration.rs`,
  `GITHUB`): the credential setting `credential` (a credential id,
  default `github.token`) goes to the plugin's slot `token`; the origin
  setting `apiUrl`, eligible only as `https://api.github.com`; the
  private-network switch `allowPrivateHost`, never migrated; `openUrl`
  derived for `github.com`. A per-action `credential` other than empty or
  the module's own makes the module `not_eligible` (G5).
- Consent: `network:api.github.com` and `openUrl:github.com` are derived
  (the built-in demonstrably used them); the token import
  (`credentials:token`, high tier) waits for the user's approval in
  Settings › Plugins. Until then, after a decline (until Retry) and on
  every computer that is not eligible, the built-in serves.
- `bundled.json` lists `deckpilot.github` as class `migration`, so every
  package (the release's installers, and `DECKPILOT_RELEASE_PLUGINS=1`
  locally) builds it, ships it and compiles its pins into the agent; CI's
  bundle dry run and `tools/ci/check-packages.sh` check the release set is
  exactly that.
- The plugin is built with #257's SDK: it exports `dp_features`, reads the
  network kind and reports the built-in's transport codes; P8's difference
  4 is gone. The remaining differences (save-time semantic checks, which
  stay run-time only: repository, ref and URL format; two messages and
  the network message wording; `retryAfterSeconds`; the `Date`-based time
  limit; the agent's limits for plugins) are listed in the plugin's
  README.
- `apps/agent/tests/plugins_cutover.rs` runs the upgrade end to end from
  an alpha.3 configuration (a key bound to `github.pull-requests`, a
  workflow node running `github.checks`, the token in `github.token`):
  seeded and pinned, settings copied, the built-in serving until the
  approval through IPC, then the key and the node in the plugin with the
  imported token (in the request headers, nowhere in answers or logs);
  `deckpilot.db` byte-identical; a build without plugins on the same data
  runs the built-in; a GitHub Enterprise configuration stays
  `not_eligible`; disabling the plugin makes the legacy actions
  unavailable, with no fallback; uninstalling and upgrading again does
  not reinstall it; a decline keeps the built-in until Retry. The
  built-in's requests go to a stand-in on 127.0.0.1
  (`DECKPILOT_TEST_BUILTIN_GITHUB_API`, feature `test-hooks`, never in a
  release build), its stored settings untouched.
- Found by that test and fixed: a built-in network action (`github`,
  `http`, `gitlab`, `jira`) bound to a key, or run with `runAction`, runs
  on the agent's service task, where `deckpilot-net` blocked on a runtime
  inside the agent's and panicked, ending the agent. A request from a
  runtime's thread now runs on a thread of its own. The service task still
  waits for it, as for every built-in action run from a key.
- Follow-up: the desktop end-to-end suite (`marketplace_gui`,
  `.github/workflows/desktop-e2e.yml`) does not show the migration
  notice and consent dialog yet. It needs the GitHub plugin built and
  staged with its pins and an alpha.3 data directory before the app
  starts; the agent side of that dialog is covered by
  `plugins_cutover.rs` and P7's `plugins_migration.rs`.

*As built (P11, the GitLab plugin):* `plugins/first-party/gitlab`
(`deckpilot.gitlab`, class `migration` in `bundled.json`) reproduces all
six actions of the built-in `gitlab` module; no variables on either side.
It is built and pinned, and **not routed**: its alias block and migration
plan come with its cut-over.

- The API URL is a user-entered origin (G2): `gitlab.com` is the declared
  host and the default, and a self-managed GitLab (with its path, possibly
  on a private network) is reached, sent the token and opened only after
  the user allowed exactly that server. The token is the slot `token`,
  sent in `PRIVATE-TOKEN`; pages open on the same server
  (`openUrl.userOrigins`, §7 G1).
- `contracts.rs` (`first_party`) checks the manifest against the built-in
  (actions, settings, and that the permissions are exactly these three);
  `apps/agent/tests/plugins_gitlab.rs` runs every action through the agent
  and the host against the built-in on the same canned answers, on
  GitLab.com and on an entered GitLab under a path, with identical
  results.
- Lists use pages of 50 (two for more than 50 merge requests, ten for the
  built-in's 500 jobs or uncounted merge requests), and pages of 10 when
  an answer is over the 512 KiB a plugin gets. The differences, all on
  failure paths, limits or the editor, are listed in
  [`plugins/first-party/gitlab/README.md`](../../plugins/first-party/gitlab/README.md).
- `plugins/first-party/support` holds what the plugins for a user-entered
  server share (URLs without the `url` crate, the `Date`-based time limit).

*As built (P11, the Jira plugin):* `plugins/first-party/jira`
(`deckpilot.jira`, class `migration`) reproduces all four actions of the
built-in `jira` module; no variables on either side. Built and pinned,
**not routed** until its cut-over.

- Jira has no fixed host, so the plugin declares none: the site is its one
  user-entered origin (`siteUrl`, with its path, possibly on a private
  network), the credential slot `credential` is bound to it and has no
  host of its own, and pages open on it. Until the user allowed the site
  nothing is sent and every reading fails with `not-permitted`, saying
  where to allow it.
- `deployment` chooses REST API 3 with Basic (Cloud, `email:api-token`) or
  REST API 2 with Bearer (Data Center); the searches are the built-in's
  fixed templates.
- `contracts.rs` (`first_party`) checks the manifest;
  `apps/agent/tests/plugins_jira.rs` runs every action against the
  built-in on Cloud and on a Data Center under a path, with identical
  results **and identical requests** (the plugin's canned answers are
  exactly what the built-in asked for).
- One difference is new: the built-in checks that a Cloud credential is
  `email:api-token` before it sends anything; a plugin never sees its
  secret, so Jira's 401 says so instead. All differences:
  [`plugins/first-party/jira/README.md`](../../plugins/first-party/jira/README.md).

*As built (P11), the GitLab and Jira cut-overs.* In builds with the
`plugins` feature only (plugins on unless turned off, §5.10); release
builds include the feature from the first release after v0.6.0 (v0.6.0
and older do not).

- `ACTION_ALIASES` has two more blocks, `gitlab.<action>` →
  `deckpilot.gitlab.<action>` (six) and `jira.<action>` →
  `deckpilot.jira.<action>` (four), with
  `ParamRule::DropIfDefault("credential")` on every action but `open`.
  `contracts.rs` checks that the shipped blocks are exactly the ones it
  validates.
- The plans (`apps/agent/src/plugins/migration.rs`, `GITLAB`, `JIRA`)
  name, beside the credential setting and the plugin's slot, the origin
  setting with `entered: true`: the plugin takes a server the user entered
  (`OriginPlan`). GitHub's stays `entered: false`.
- **A server the user entered in the built-in is carried, and asked
  for.** The value of `apiUrl` or `siteUrl` is copied into the plugin's
  setting of the same key, normalized as the agent stores what a user
  saves there. It allows nothing: the migration then waits
  (`awaiting_consent`) for the user to allow exactly that server, the
  per-origin consent of G2, as a consent item of its own
  (`network:user-origin:<setting>`, with the server in the item). The
  approved policy derives low- and medium-tier permissions the built-in
  used; G2 defines the consent to an entered server as never derived, and
  the slot is high tier (it may be on a private network). Where the two
  could be read differently the engine asks.
  - The consent is recorded only by the decision that names the server
    (`decided_by = user:migration:<module>`): approving the credential, or
    a slot grant made elsewhere, allows no server.
  - **The private network is an item of its own**
    (`network:user-origin:<setting>:private-network`, no grant by itself):
    asked when the server's host is private or the built-in's
    `allowPrivateHost` was on for it, and the server is allowed only with
    both, in one decision. Declining it declines the server; the built-in
    stays until Retry.
  - A server a plugin can never reach (plain `http`, this computer, a
    user name in the address) keeps the built-in (`not_eligible`), and so
    does `allowPrivateHost` for the default origin, which has no
    per-server consent.
- Derived: on GitLab.com, `network:gitlab.com` and `openUrl:gitlab.com`
  (the built-in used them); for both modules
  `openUrl:user-origin:<setting>`, which opens nothing before the server
  is allowed. Not derived, so asked: the credential import and the origin
  slot (always), the server (when one was entered), and, for a
  self-managed GitLab, GitLab.com's two grants, which the built-in did
  not use there.
- After the cut-over, another server saved by an older app is copied
  again and the module goes back to `awaiting_consent` (the built-in
  serves) until the user allowed that one too. The entered origin is
  part of the copied settings, not of the eligibility digest, so it is a
  change like any other setting's.
- The app's consent dialog shows the server, what is sent there and that
  its pages may be opened, and the private network as a separate item
  (`MigrationConsentItem.origin`, [agent-integration.md](agent-integration.md#module-migrations)).
- `apps/agent/tests/plugins_cutover_servers.rs` runs the upgrades end to
  end from alpha.3 configurations: a self-managed GitLab under a path and
  a Jira Cloud site (the built-in serves until the server's own item is
  approved; then the key and the workflow node run in the plugin at
  exactly that server with the imported credential; `deckpilot.db`
  byte-identical; a build without plugins runs the built-in; disable and
  uninstall withdraw the legacy actions and an upgrade does not bring the
  plugin back); GitLab.com; a GitLab on a private network (declined,
  restarted, Retry, allowed); servers a plugin cannot reach; and a
  computer that does not use the module. The built-ins' requests go to
  stand-ins on 127.0.0.1 (`DECKPILOT_TEST_BUILTIN_GITLAB_API`,
  `DECKPILOT_TEST_BUILTIN_JIRA_SITE`, feature `test-hooks`).
- Follow-ups: GitHub Enterprise through the same mechanism (a user-origin
  slot in `deckpilot.github` and `entered: true` in its plan; its web
  host is not derivable from the API URL on GitHub Enterprise Cloud, so
  `open` needs its own decision); the desktop end-to-end suite does not
  drive these dialogs yet.

`notifications` follows as the first `default`-class bundled plugin. It is
the one that proves seeding, tombstones and first launch.

*As built (P10):* `plugins/first-party/notifications`
(`deckpilot.notifications`, class `default`) reproduces both actions; the
alias block `notifications.*` → `deckpilot.notifications.*` and its
(empty) migration plan are shipped. The migration engine takes a
`default`-class plugin as the built-in's replacement whether or not the
data uses the built-in: a fresh install cuts over at its first start, with
nothing to migrate; an upgrade copies the module's three settings first.
Both are low tier only, so no consent is asked. A built-in the user had
turned off turns the seeded plugin off once (the enabled state is copied
once, §5.5), and the module stays `not_eligible` on its built-in.
`contracts.rs` checks the manifest; `apps/agent/tests/plugins_notifications.rs`
compares 14 scenarios with the built-in through the real agent and host,
and covers the first launch, tombstones across restarts and an app update,
and an alpha.3 upgrade with an older build on the same data. The
differences are listed in
[`plugins/first-party/notifications/README.md`](../../plugins/first-party/notifications/README.md).

## 7. SDK gaps

Each gap names the modules that need it, a security design sketch, and
whether it can be offered. Operation names follow ABI 1's closed
`CapabilityRequest` set. Every addition is optional and compatible, like
#231's and #232's.

### G1 `openUrl` (github, gitlab, jira)

Declared in the manifest (`crates/deckpilot-plugin-manifest/src/model.rs:240-244`)
and in #191 §5.2, but no operation exists. Sketch: the op `url.open {url}`
is allowed only for `https`, only for an exact granted origin (from the
manifest or G2), and only within an invocation started by a key press or the
app, never by a timer or a workflow trigger. The rate is 1 per 2 s. The agent
opens the URL through the core launcher (`launch::plan(OpenTarget::Url)`),
which never accepts `file:` or custom schemes, and audits the origin. Risk:
low to medium (phishing pages on granted origins only). **Offer.**

*Status (P4):* implemented as the op `url.open` (`Context::open_url`):
exact `openUrl` origins from the manifest (wildcards are not granted), the
broker's strict URL parser (https only, no userinfo, no IP literal or
local name, at most 2,048 bytes), 1 per 2 s plus 20 per 10 minutes per
plugin and 3 per invocation, audited and logged by origin only, opened
through the core launcher. Since G8, only in a run the user started (a
key press, a gesture, the app, or a workflow the user started): the
broker denies it as `not-permitted` in a run a schedule, a lifecycle
event, the agent's start or a work session started, and the consent text
says so.

*Status (P11):* pages of a server the user entered. `openUrl.hosts` are
fixed names, so `gitlab.open` on a self-managed GitLab and every
`jira.open` (Jira has no fixed host) had no grant to open with. The
`openUrl` permission now takes `userOrigins`, the settings of G2 slots: a
declared medium-tier grant per setting (`openUrl:user-origin:<setting>`)
that opens nothing by itself, and lets `url.open` open `https` pages of
exactly the server the user entered there and allowed (G2's per-origin
consent, whose dialog says so). By server, not by the slot's path prefix (a
GitLab's pages are beside its `/api/v4`, not under it); requests are
unchanged. A private server only when it is the one the user allowed on a
private network. No schema change in `plugins.db`: the grant is an
ordinary row, and the broker's record of the consented server carries the
flag ([permissions.md](sdk/permissions.md#pages-of-a-server-the-user-entered)).

### G2 User-entered origins (github GHE, gitlab self-managed, jira, http)

Sketch: the manifest declares `network.userOrigins: { setting: "apiUrl",
methods: [...] }`, and optionally a credential slot `hostsFrom: "apiUrl"`.
When the user saves the setting, the agent parses one exact origin (no
wildcard, no IP literal unless G3, no userinfo) and shows consent: "can send
data and your token to `ghe.example.com`". It stores a grant with `source =
'user'`. The broker is unchanged, because it still sees exact origins. A
changed setting revokes the old origin. Risk: medium; a token goes only where
the user typed. **Offer.** For `http`'s per-action URLs, the same mechanism
per action instance would be needed ("consent on save of each URL"). That
is possible, but it gives the plugin the authority the built-in has.
**Defer.**

*Status:* implemented ([permissions.md](sdk/permissions.md#user-entered-origins)).
The slot is `network.userOrigins: [{ setting, methods?, pathPrefix?,
privateNetwork? }]`, a declared grant of its own
(`network:user-origin:<setting>`, medium tier, high with
`privateNetwork`), and a credential slot binds to it with `userOrigin`
(the design's `hostsFrom`). Saving the setting normalizes it; the user
then allows exactly that origin (`grantPluginUserOrigin`), recorded per
plugin and setting in plugins.db schema 4 (`plugin_user_origins`) with
its path prefix, methods, credential slots and, as a separate explicit
consent, the private network. The broker matches scheme, host, port and
the path prefix exactly, on every redirect too; a private host (an IP
literal of a private range or a local name) is refused without that
consent, and the executor connects to private ranges (never loopback or
link-local) only for it. A changed setting, a revocation and an
uninstall end the consent. The `http` per-action variant stays deferred.

### G3 Private network (http, GHE or GitLab on a LAN, Jira Data Center)

`GrantScope::PrivateNetwork` exists (`crates/deckpilot-plugin-store/src/grants.rs:111`),
but the agent drops it (`apps/agent/src/plugins/convert.rs:334`) and the
executor hard-codes `allow_private: false` (`executor.rs:130-136`). Sketch:
a high-tier checkbox, plus a machine-specific `privateHosts` setting that
lists exact hosts. `NetworkPolicy { allow_private: true, private_hosts }`
covers those hosts only. Plain `http` is allowed for them, and credentials
over plain `http` only to loopback, as the built-in does
(`modules/http/src/client.rs:189`). It is never derived by a migration.
Risk: high (requests against local admin interfaces). **Offer, high tier.**

### G4 Per-plugin limits (http, and large GitLab or Jira pages)

Today: a 15 s fetch, 512 KiB responses and 32 KiB request bodies. Sketch:
the manifest declares `limits: { fetchTimeoutSeconds ≤ 120,
responseKb ≤ 4096 }`, shown in consent. Bodies larger than the 1 MiB
host-call reply (`abi/src/limits.rs:26`) need a chunked read (`http.read
{handle, offset}`) instead of a bigger frame. Risk: resource use only.
**Offer.**

### G5 Credential selection and import (http, overrides in github, gitlab, jira)

Sketch, in two parts. (a) A `credentialSlot` parameter kind that holds a
choice among the plugin's declared slots, never a secret and never a global
`CredentialId`. (b) For `http`, user-created slots from a declared template
(`credentials.template: { schemes, hostsFrom }`), each with its own consent.
The one-time import from `credentials` into a slot (§5.5) is an agent
operation, never a plugin call. Risk: high tier, as for all credentials.
**Offer (a) with G2. (b) is deferred with `http`.**

### G6 Response headers (github `link`, `x-ratelimit-*`; gitlab `x-total`, `ratelimit-reset`)

The executor passes seven headers (`executor.rs:34-42`). Sketch: add
`link`, `x-total`, `x-total-pages`, `x-next-page`, `x-ratelimit-limit`,
`x-ratelimit-remaining`, `x-ratelimit-reset`, `ratelimit-reset` to the fixed
list. They carry no secrets. `set-cookie` and `www-authenticate` stay out.
Risk: none. **Offer.**

*Status (P4):* implemented. The allowlist moved to the ABI
(`RESPONSE_HEADERS`, exact names: the seven above plus `link`, GitLab's
`x-total`, `x-total-pages`, `x-page`, `x-per-page`, `x-next-page`,
`x-prev-page`, GitHub's `x-ratelimit-limit|remaining|reset|used|resource`,
`ratelimit`, `ratelimit-policy|limit|remaining|reset|observed`, and
`content-language`, `expires`, `age`, `vary`), applied after credential
redaction, with values bounded to 4 KiB and 32 headers. The SDK reads them
with `Response::header`, `header_values` and `link(rel)`.

### G7 Button state and polling (git, docker)

The manifest kinds `device.state` and `background` exist, but no operation
does. Sketch (#191 §5.2): `state.set` only for keys bound to the plugin's
own action instances, 10/s per key, and timers of at least 15 s while
active. Needed only by modules that stay built-in for other reasons.
**Offer later. It is not on the migration path.**

### G8 Run and profile context (notifications, general)

Sketch: the invocation carries a read-only `run` object with the workflow's
name, the node count and statuses, and whether the run failed. That is the
invocation's own run only, never another workflow's definition. Profile read
(`profile` kind, `ProfileScope`) gives the profile name and the active
project's id, key and name, as #191 §5.2 specifies. Risk: low to medium.
**Offer.**

*Status (P4):* the run part is implemented as the op `run.context`
(`Context::run_context`, always allowed): the trigger (`key`, `gesture`,
`app`, `test`, `workflow`), for a workflow the cause of its run (`user`,
`schedule`, `event`, `startup`, `workSession`), and for a node of a
workflow graph the run's summary: the workflow's name, the counts of
succeeded, failed, skipped and cancelled nodes and the ids of the failed
ones, bounded. The invocation's own run only. It is a capability call
that only new guests make, so guests built before it are unchanged. The
agent derives it from the core's `RunOrigin`, which the engine threads
from the key press, gesture, `runAction`, `invokePluginAction` and the
workflow run's trigger; it also restricts G1's `url.open`. Not yet: the
profile read (`profile` kind).

### G9 Notification severity and routing (notifications)

Sketch: `notify {title, body, severity?, critical?}`. The agent applies the
user's routing (desktop, status bar, minimum severity). Those settings move
to the plugin's settings and are read by the agent's notifier for that
plugin, not by plugin code. `critical` maps to the existing status-bar path.
The plugin-name prefix stays. The rate stays 5 per minute unless the
manifest declares up to 30 per minute with a justification (medium tier).
Risk: low. **Offer.**

*Status (P4):* severity only. `notify` takes an optional `severity`
(`info`, `success`, `warning`, `error`; SDK `Context::notify_with`) that
sets how the notification looks and nothing else: the grant, the plugin
name prefix, the status-bar destination and the 5 per minute rate are
unchanged. Not yet: `critical`, the routing (the user's desktop,
status-bar and minimum-severity settings read by the agent's notifier per
plugin) and a manifest-declared rate up to 30 per minute. They need plugin
settings the notifier reads and a consent change, and land with G8 before
P10.

*Status (P10):* the decisions for parity with the built-in
`notifications`:

- **Routing: done.** The manifest's `notifications` permission takes an
  optional `routing` naming the plugin's own settings for `desktop`,
  `statusBar` and `minimumSeverity` (boolean, boolean and a choice of the
  severities; validated). The agent reads them, as the user saved them,
  at every `notify`, and shows the notification through its own notifier
  (the native helper and the status bar of open windows) or not at all.
  The plugin never decides where. Without a routing: the status bar only,
  as before. Desktop notifications are still "show notifications": the
  grant and its low tier are unchanged, the plugin-name prefix stays, and
  the consent text says that the plugin's settings decide where they
  appear. The settings move with the module's migration (copied one to
  one, §5.5).
- **Delivery: done.** A guest that reports the ABI feature
  `NOTIFY_DELIVERY` (`dp_features`, as #257 did for network kinds) gets
  where it was shown, or why not (`NotifyDelivery`; SDK
  `Context::notify_delivered`); every other guest still gets `null`. Not
  shown is an `ok` reply; not granted and the rate limit stay errors.
- **`critical`: no host change.** It is the built-in's "fail the node when
  the notification was shown nowhere"; with the delivery reply the plugin
  decides it itself (`notification-not-shown`). A host-side `critical`
  would add nothing a plugin could not do, and nothing it should.
- **Higher rate: not offered.** A manifest-declared rate above 5 per
  minute is medium tier, which a `default`-class plugin must not need
  (seeding grants the low tier only, §4.2). The plugin keeps 5 per minute:
  a 6th notification in a minute is dropped and its node goes on (or
  fails, when critical). Workflows rarely show more; the difference is
  documented in the plugin's README. No action stays built-in for it.

### G10 Cancellable sleep (general.wait; retries and backoff in any plugin)

Sketch: `time.sleep {ms ≤ remaining deadline}`, woken early by the
invocation's `CallInterrupt` (#231). Risk: none. **Offer.** It is not needed
for the migration (`general` stays core).

### G11 Process inspection (system.process-status)

Sketch: `process.query {name}` returns `{running, count}`, by exact name
only, never PIDs, command lines or a full listing. Medium tier, because it
reveals which programs the user runs. **Possible, low value.** The action
stays built-in.

### G12 Launch a bound resource (workspace, system launchers, git.open-repository)

Sketch: `resource.open {role, application?}`. The host resolves the
project's resource role and the machine binding, checks the path stays
inside the binding (today's `existing_inside`), and launches through the
core launcher. The plugin never sees a path. High tier: it starts
user-chosen programs. **Not in API 1.** The value is low and the risk is
real.

*Superseded (2026-11-20).* This becomes plugin API 1.9, first-party only, as
[X1, X2 and X4](#x1-resource-parameters-as-opaque-handles).

### G13 `process.exec`: not offered

Needed by `git`, `docker` and `system.run-command`. Even with an allowlist
of executables it is code execution:

- `git` takes options that run programs (`-c core.sshCommand=…`,
  `--upload-pack`, `-c core.fsmonitor=…`, hooks from repository
  configuration).
- `docker` access is root-equivalent (bind-mount `/`, privileged
  containers).
- Both need `SSH_AUTH_SOCK`, which lends the user's SSH identity.

A brokered `git.status` or `docker.ps` that ran fixed commands host-side
would keep all of the security-relevant code (argument building, output
parsing, environment curation) in the host, and the plugin would only
format. **Blocker: `git`, `docker` and `system.run-command` stay built-in.**

*Superseded for `git` and `docker` (2026-11-20).* The threat analysis above
still stands, and the 1.9 design answers it rather than dismissing it. A raw
`process.exec` is still **not offered**. In its place is `tool.run`, a
brokered run of `git` or `docker` only, checked against a host-compiled
policy for each tool, granted only to bundled first-party plugins
([X3](#x3-toolrun-brokered-git-and-docker)). The last sentence of the
paragraph above stays true: the authority remains host code. The owner
accepted that in exchange for removability. `system.run-command` stays
built-in and is not affected.

### G14 Scoped file read: not needed

No module that could otherwise move reads files. `git` and `docker` need
more than reads (G13). **Not offered.**

### G15 Project-context write and G16 work-session write: never

`ProjectContext` is never grantable (`permissions.rs:31`), and the work
session's state machine is the agent's. Read access is covered by G8 and by
the `variables` read kind (`workSession.*`, `projects.current.*`), whose
operation does not exist yet. **Blocker: `projects` and `work-session` stay
core.**

*Superseded for project context (2026-11-20).* Project select and cycle
become grantable to the bundled `deckpilot.projects` only
([X6](#x6-project-operations-projectcontext)). The work-session write (G16)
stays **never grantable**.

### G17 Bundled source, pins and tombstones (infrastructure)

Covered in [§4](#4-the-default-system-plugin-set) and [§5.6](#56-the-migration).

### Plugin API 1.9 security reviews (2026-11-20)

These are the written security reviews that the owner's rule requires
before any privileged capability lands ("no privileged capability without
a written security review"). They replace G12, G13 and G15 for the
extraction set ([§11](#11-owner-decisions-of-2026-11-20)). **No code for X2
to X9 may merge before the owner has read and approved this section.** It
changes security policy, so it falls under "permissions need approval" and
is not auto-merged. WP-A4 (`tool.run`) in particular waits for that
approval.

Status: **reviewed design, not implemented.** Nothing here is grantable
until its work package lands with the tests named below.

#### Common rules for every entry

1. **API level.** Each kind validates only with `"engines": { "pluginApi":
   "^1.9" }`. A `^1.8` manifest that declares one is refused, so third-party
   manifests built against 1.8 are unaffected.
2. **First-party gate** ([X12](#x12-first-party-publisher-gate)). `tools`,
   `resources` (with `paths` and `open`), `openUrl.fromParams`,
   `projectContext`, `hostViews` and host-sourced variables validate only
   for the reserved publisher `deckpilot`. They are granted only to a
   **bundled, pinned** package.
3. **Deny by default.** The broker checks every call against the
   **current** grants. A revocation, a disable or an uninstall applies to
   the next call. For `tool.run` it also kills a process that is still
   running (below).
4. **Run origin.** "User-started run" has the meaning G1 and G8 gave it: a
   key press, a gesture, the app, or a workflow whose cause is `user`. It
   never covers a schedule, a lifecycle event, the agent's start or a work
   session, except where an entry says otherwise. The origin is the core's
   `RunOrigin` (`crates/deckpilot-core/src/execution.rs:53`). The guest
   cannot set it.
5. **Audit.** Every call to these capabilities, allowed or denied, writes
   one audit record. The record holds the plugin id and version, the
   operation, the grant key, the run origin, the outcome (a reason code on
   denial), the duration, and for `tool.run` the exit status and the byte
   counts. It never holds a secret, an environment value or an output
   body. A path appears only as the binding's label. A URL appears only as
   its origin.
6. **No exfiltration channel next to authority.** A plugin that holds
   `tools` must declare no `network`, no `credentials` and no `openUrl`.
   The validator refuses that combination. Its results can leave the
   plugin only as its action outputs, its own variables, its key state and
   its log, all of which the user's own configuration consumes.
7. **What stays never grantable**, for every publisher: a raw
   `process.exec` or shell, `filesystem`, `clipboard`, raw device access,
   work-session writes (G16), the `workflows` engine kind (X11), and `ui` or
   `customUi` (plugin UI code).

#### X1 Resource parameters as opaque handles

*Purpose.* git, docker and workspace actions take a `repository`, `compose`,
`location` or `application` resource, as the built-ins do. Today a plugin
parameter of kind Resource is refused (`apps/agent/src/plugins/engine.rs:639-662`).

*Design.* A parameter of kind `resource {kinds[]}` (`directory`, `file`,
`application`, `dockerEnvironment`) reaches the guest as `{handle, kind,
label}`. The host resolves it through `ResourceRef::{Fixed, ProjectRole}`
and the machine bindings. `handle` is a random 128-bit value, bound to the
plugin id and the invocation, and valid until the invocation ends. `label`
is the binding's name, which the user chose. The guest never receives the
path.

*Threats.* A plugin minting, guessing or replaying a handle to reach another
binding. A crafted stored parameter that names a path outside a binding.
Path disclosure (user names, client folder names).

*Mitigations.* Handles are unguessable and are checked against the
invocation's table on every use. A handle from another invocation, another
plugin, or one that has ended is `not-permitted`. Only bindings of the
declared kinds resolve. A `Fixed` reference resolves only to an existing
machine binding, never to a literal path from the document. The legacy
alias rule `Keep` (X10) passes the stored reference to the host resolver,
never to the guest.

*Tier and consent.* None by itself. It confers nothing without X2, X3 or X4.

*Audit.* None (no outside effect).

*Tests (denial matrix).* A handle replayed in a second invocation, a handle
of plugin A used by plugin B, a forged handle, and a handle of a
non-declared kind are each refused. A Resource parameter of a `^1.8` or
third-party manifest is refused at validation. No guest-visible value ever
contains the bound path (a property test over the serialized invocation).

#### X2 `resource.resolve`

*Purpose.* Role defaults (`repositoryRole`, `composeRole`, the workspace
roles): when an action has no fixed resource, the active project's role
resolves it, as `repo.rs:50-94` does today.

*Design.* `resource.resolve {param? | role, projectId?, kinds[]}` returns
`{handle, kind, displayPath?}`. `projectId` must be a project of the active
profile. Without Projects, a role resolves to `no-active-project`.

*Threats.* Enumerating every binding of every project. Path disclosure
through `displayPath`. A confused deputy: resolving a role of a project
other than the one the user is working in.

*Mitigations.* One handle per call, for the named role or parameter only.
There is no list operation. `projectId` defaults to the run's project.
Another project of the same profile is allowed only when the action's own
`project_id` parameter names it. `displayPath` is returned only with
`resources.paths: true`, which the consent dialog shows. The first-party
git, docker and workspace plugins **do not declare `paths`**: they need
only labels. 20 resolves per invocation.

*Tier.* `resources` medium. `resources.paths` medium, separately listed.

*Consent.* Derived during a migration only when the built-in demonstrably
used machine resources on this computer (`demonstrated`,
`apps/agent/src/plugins/migration.rs:709`). Otherwise it is asked for.

*Audit.* The role, the kind, the outcome, and the binding label.

*Tests.* A role of another profile's project is refused. `projectId` that
does not match the action's parameter is refused. Without `paths`,
`displayPath` is absent. Projects disabled gives `no-active-project`, with
no panic. Over 20 calls per invocation gives `Throttled`.

#### X3 `tool.run`: brokered git and docker

This is the largest risk in the plan.

*Purpose.* Let `deckpilot.git` and `deckpilot.docker` run the user's own
`git` and `docker` executables, as the built-ins do (`modules/git/src/repo.rs:115-149`,
`modules/docker/src/actions.rs:565-595`). The owner gets removability: with
the plugin disabled or uninstalled, no process authority is in use. The
parsing and logic can also be released independently.

*Owner's model (2026-11-20).* The manifest allow-lists executables (`git`
and `docker` only). Arguments are an array, never a shell string. The
working directory is a resource handle only. Time and output are limited.
Explicit consent is asked on first use. Every run is audited.

*Hardening adopted within that model.* A host-compiled policy for each
tool, in the proposed `apps/agent/src/plugins/tools/{mod,git,docker}.rs`:

- **Allowed subcommands.** Each argv's subcommand must be in the tool's
  closed list.
- **Allowed options for each subcommand.** Each option must be an exact
  token or a `--name=<value>` whose value passes a typed validator. Each
  positional fills a typed slot: a ref name, a remote name, a container,
  service or project name (`modules/docker/src/cli.rs:74-101`), a bounded
  integer, or a `--since` value. Anything not listed is refused. This
  allowlist is the security boundary.
- **Denylist.** The dangerous flags below are checked first, for a precise
  error and as a second line of defence. A denylist on its own is
  incomplete by nature, so the boundary is the allowlist.
- **No value starts with `-`** in a positional slot. The host inserts `--`
  before positionals.
- **An environment allowlist**, applied over `minimal_environment`
  (`crates/deckpilot-core/src/process.rs:135`). The guest cannot pass any
  environment.
- **The executable is a host-held machine setting of the grant**:
  `gitExecutable` and `dockerExecutable` move out of the plugin's settings.
  The user sets it in DeckPilot's own settings UI. Its file name must be
  `git` or `docker` (`.exe` on Windows). The consent dialog shows the
  resolved path. The plugin cannot read or repoint it. The Docker
  `context` is a host-held setting too.
- **The working directory and git's `-C` are a resolved handle only**, of
  kind `directory` (git) or `directory`/`file` (compose). They are never a
  path the plugin supplies.

The wasm guest never builds the final argv, never sees the environment or
the executable path, and never receives a process handle. It submits
`{tool, args[], cwd: handle, timeoutMs}` and gets back `{exit, stdout,
stderr, truncated, durationMs}`.

##### X3a `tools:git` (read) and `tools:git:network`

*Allowed subcommands and shapes* (exactly what the built-in runs today):

| Subcommand | Allowed form |
| --- | --- |
| `status` | `--porcelain=v2 --branch -z` |
| `log` | `-1 --format=<format from a fixed set of placeholders>`, no `--output`, no path arguments |
| `config` | `--get branch.<ref name>.remote` or `branch.<ref name>.merge` only. No other key, so `credential.*`, `http.*.extraHeader` and `url.*` cannot be read |
| `remote` | bare (names only). Not `-v` or `get-url`, which can print URLs with embedded credentials |
| `rev-parse` | `--verify -q <ref name>` |
| `ls-remote` | `-- <remote name> <ref name>`, only with `tools:git:network`. The remote must be a **name** listed by `git remote`, never a URL (a URL would let the plugin choose the server, and `ext::` URLs run commands) |

*Host-fixed prefix*, which the plugin cannot change (as implemented,
after the security review of WP-A4; see
[below](#x3-as-implemented-and-the-review-of-wp-a4)):
`-C <handle dir> --no-pager --no-optional-locks -c core.fsmonitor=false -c log.showSignature=false -c protocol.allow=never -c protocol.https.allow=always -c protocol.ssh.allow=always -c protocol.git.allow=always -c protocol.ext.allow=never -c protocol.file.allow=never -c credential.interactive=never -c core.askPass=`.
The environment is `GIT_TERMINAL_PROMPT=0`, `GIT_OPTIONAL_LOCKS=0`,
`GIT_PROTOCOL_FROM_USER=0`, `GCM_INTERACTIVE=never`, an empty
`GIT_ASKPASS`, `GIT_CEILING_DIRECTORIES=<the binding's parent>`,
`LC_ALL=C`, plus `GIT_EXEC_PATH` and `XDG_CONFIG_HOME` from the agent's own
environment. `SSH_AUTH_SOCK` is added **only** for `ls-remote` under
`tools:git:network`.

*Denied always* (refused before the allowlist check): `-c`, `--config-env`,
`-C`, `--git-dir`, `--work-tree`, `--namespace`, `--exec-path`,
`--upload-pack` and `-u`, `--receive-pack`, `--output` and `-o`,
`--ext-diff`, `--textconv`, `--exec`, `--super-prefix`, any `core.*` or
`alias.*` key, and any subcommand that is not a git builtin. Git expands an
alias only for a name that is not a builtin, so the subcommand allowlist
also excludes aliases.

*Threats.* Argument injection that runs programs (`-c core.sshCommand`,
`--upload-pack`, `ext::`). Writing files (`log --output`). Reading
secrets from configuration. SSRF through a plugin-chosen remote. Lending
the SSH identity. Output that echoes credentials.

*Mitigations.* The allowlist, the denylist and the fixed prefix above. The
network sub-grant is separate, and its consent says "uses your SSH identity
and git credential helpers to contact the remotes configured in your
repository". On stderr and stdout the host redacts URL userinfo
(`https://user:token@host` becomes `https://host`) before the guest sees
it. The time limit is `networkTimeoutSeconds` (5-120, default 20) for
`ls-remote` and 10 s for local reads. Output is capped by the core's
`ProcessLimits`, and above that by the 1 MiB host-call reply.

*Residual risk, stated honestly.*

- **Repository-local configuration is trusted by git.** A repository the
  user cloned from a hostile source can set `core.sshCommand`,
  `remote.<name>.uploadpack` or `core.gitProxy` in `.git/config`, and
  `ls-remote` would run them. `git status` can run filters
  (`filter.*.process`) for files that need them. The built-in has exactly
  the same exposure today. The plugin cannot write that configuration (no
  `config` set and no file system), so it gains nothing the repository's
  author did not already have. WP-A4 should also force
  `-c core.sshCommand=ssh` and `-c core.gitProxy=` for `ls-remote`, and
  measure whether that breaks common setups before it is made mandatory. WP-A4 (2026-11-20) does **not** force them: they would also override
  the user's own global `core.sshCommand` (a custom key or agent), which
  has not been measured. This stays an accepted risk, as for the
  built-in. The environment overrides (`GIT_SSH_COMMAND`, `GIT_CONFIG_*`,
  `GIT_ASKPASS`, `GIT_DIR`) never reach git.
- **The isolation gain is limited.** The authority stays in the agent. A
  bug in the host policy is a bug in trusted code, exactly as G13 warned.
  The sandbox isolates parsing and formatting only. The gain is
  removability, a typed and reviewable policy instead of free-form module
  code, and independent releases of the parsing.

##### X3b `tools:docker` (read)

*Allowed:* `context show`; `version --format {{json .}}`; `compose version
--short`; `ps --no-trunc --format {{json .}} [--all] [--filter
name=<name>]`; `compose -f <file> -p <project> ps --format json`;
`compose -f <file> -p <project> logs --tail <1-1000> [--since <v>]
[<service>…]`; `logs --tail <1-1000> [--since <v>] -- <container>`. The
compose file is always the handle's path, which the host inserts.

*Denied always:* `run`, `exec`, `create`, `cp`, `attach`, `build`, `push`,
`pull`, `login`, `logout`, `inspect` (it prints environment variables,
which often hold secrets), `compose config` (it prints the interpolated
file, secrets included), `compose run|exec|cp|build|push|pull`, `system`,
`volume`, `network`, `plugin`, `swarm`, `secret`, `image`, `-v` and
`--volume`, `--mount`, `--privileged`, `--cap-add`, `--device`,
`--pid`, `--network`, `--security-opt`, `-H` and `--host`, `--context`,
`--config`, `--tls*`, `-e` and `--env`, `--env-file`,
`--project-directory`, and any plugin-supplied `-f` or `--file`.

*Environment:* `DOCKER_HOST`, `DOCKER_CONTEXT`, `DOCKER_CONFIG`,
`DOCKER_CERT_PATH` and `DOCKER_TLS_VERIFY` from the agent's own environment
or the host-held `context`, never from the plugin. `SSH_AUTH_SOCK` only
when the host-held context is an `ssh://` one.

*Threats.* Docker access is root-equivalent. Reading container secrets.
Pointing the CLI at another daemon (SSRF to a remote engine). Log output
carrying tokens.

*Mitigations.* No subcommand that creates, changes or enters a container
is reachable with the read grant. `logs` output is marked sensitive and
capped at 64 KiB, as today. The read timeout is 30 s.

*Residual.* `ps --format {{json .}}` includes container command lines and
labels, which can hold secrets. The built-in returns the same data. The
docker plugin has no network or credential grant (common rule 6), so the
data can leave it only through the user's own configuration.

##### X3c `tools:docker:modify`

*Allowed:* `compose -f <file> -p <project> up --detach [<service>…]`,
`compose … down [--remove-orphans]`, `compose … restart [<service>…]`,
and `start|stop|restart -- <container>`. Never `down -v` or `--volumes`,
`--rmi`, `up --build`, `--force-recreate` with `--renew-anon-volumes`, or
`kill`.

*Threats.* A compromised plugin taking services up or down at a bad time.
Running whatever the compose file declares (privileged containers, a bind
mount of `/`). Lifecycle-driven loops.

*Mitigations.* A separate grant, **off by default**, which replaces
`allowStateChanges`. It needs explicit consent and is never derived. It is
offered during a migration only when `allowStateChanges` was on, and even
then it is asked for. It is allowed only in user-started runs, and in
workflow runs with cause `workSession` (a session the user started; this
keeps "start work, compose up" working). It is never allowed from a
schedule, a lifecycle event or the agent's start. Limits: 1 modifying run
at a time per plugin, and 6 per minute. The timeout is `timeoutSeconds`
(10-1800, default 300). The compose file is only the user's bound
resource.

*Residual.* `up` runs the user's compose file with the daemon's full
authority, which is what the user asked for. The plugin chooses *when*,
within the run-origin rule. This matches the built-in with
`allowStateChanges` on. A scheduled workflow that ran `compose up` under
the built-in stops working. The owner must confirm that, or allow the
`schedule` cause explicitly.

##### Common to X3a to X3c

*Tier.* **High.** Explicit consent for each plugin and each sub-grant
(`tools:git`, `tools:git:network`, `tools:docker`, `tools:docker:modify`).
It is never derived: the migration engine caps derivation at medium
(`crates/deckpilot-plugin-store/src/manager.rs:731`).

*Consent.* One time, when the plugin is first used or at the migration
prompt. It names the executable path, the sub-grant, and for network the
SSH identity. The plugin cannot trigger the prompt itself.

*Process lifecycle.* The process runs without a shell, in its own process
group. It is killed on cancellation, on the deadline, on a disable, on a
revocation of the grant and on an uninstall. At most 4 concurrent runs per
plugin, 20 starts per second per plugin, and a queue beyond that. These are
proposed values, fixed by WP-A4 with tests. 32 git keys refreshing every
30 s fit easily (risk 8 in the plan).

*Audit.* The tool, the subcommand, the option names (never values), the
handle label, the run origin, the exit status, the duration, the byte
counts, and the denial reason.

*Tests: the denial matrix* (`apps/agent/src/plugins/tools/tests.rs`, plus
end-to-end tests in `apps/agent/tests/plugins_tools.rs` with a fake
executable that records its argv and environment):

- git: each denied flag above, alone and as `--flag=value`, `-cfoo=bar`,
  combined short options, and after `--`. A remote given as a URL. `ext::`,
  `file://` and `-` as a remote. `config --get` of any key other than the
  two branch keys. `log --output`. An alias name. A non-builtin
  subcommand. An unknown option.
- docker: each denied subcommand and flag. `-H`. `--context`. A
  plugin-supplied `-f`. `compose config`. `inspect`. `down -v`. Modify
  operations without `tools:docker:modify`, and modify from a `schedule`,
  `event` and `startup` run.
- both: a cwd that is not a live handle. A path argument outside the
  handle. Environment values not on the allowlist (the fake executable
  asserts its whole environment). `SSH_AUTH_SOCK` absent without the
  network sub-grant. Timeout kills the process group. Output over the cap
  is truncated and flagged. A revocation mid-run kills the process. An
  executable setting whose file name is not `git` or `docker` is refused.
  A third-party or `^1.8` manifest declaring `tools` is refused. A `tools`
  manifest that also declares `network`, `credentials` or `openUrl` is
  refused.

##### X3 as implemented and the review of WP-A4

WP-A4 (`apps/agent/src/plugins/tools/`, `executor/tools.rs`) had an
independent security review on 2026-11-20 ("ship after fixes"). Each
finding is fixed and has a regression test that reproduces the attack
(`apps/agent/src/plugins/tools/tests.rs` and `tests/runs.rs`, with fake
executables and, for git's own behaviour, the real `git` in temporary
repositories; never a real docker):

| Finding | Fix | Test |
| --- | --- | --- |
| M1 git discovery escapes the binding (a folder inside a dotfiles repository lists the home folder) | `GIT_CEILING_DIRECTORIES=<the binding's parent>`, and before every run `rev-parse --show-toplevel`, whose real path must be the binding or inside it (`not-a-repository`, `tool-outside-binding`) | `git_never_finds_a_repository_above_the_binding`, `a_work_tree_outside_the_binding_is_refused` |
| M2 compose on a folder loads a file outside it (the folders above, `COMPOSE_FILE=../compose.yaml` in `.env`) | the host picks `compose.yaml`, `compose.yml`, `docker-compose.yaml` or `docker-compose.yml` **inside** the folder (real path contained) and passes it as `-f`; none is a refusal | `the_compose_file_of_a_folder_is_inside_it`, `compose_on_a_folder_runs_the_folders_own_file_only` |
| M3 a plugin-chosen `--project-name` reaches another project | the plugin passes no `-f`, `-p` or `--project-name` (denylisted); the host pins the name: the file's literal top-level `name:`, else compose's default for its folder | `a_plugin_can_never_name_the_compose_project`, `compose_on_a_folder_runs_the_folders_own_file_only` |
| I1 the user's settings unused | `gitExecutable`, `networkTimeoutSeconds`, `dockerExecutable`, `context` and `timeoutSeconds` of the built-in modules are read by the agent whenever it loads its modules (`ToolSettings`); on macOS `git` and `docker` are also looked for where Homebrew and Docker Desktop install them | `the_users_module_settings_are_the_hosts`, `the_executable_must_be_the_tools_own` |
| L1 a hostile repository runs `gpg` on `log` | `-c log.showSignature=false`; a format must start with `format:` or `tformat:` (a bare word names a `pretty.*` of the repository) and has no `%G` placeholder | `a_hostile_repository_cannot_run_gpg_on_log`, `log_formats_must_be_explicit_and_never_check_signatures` |
| L2 remote helper transports (`foo::bar`) | `GIT_PROTOCOL_FROM_USER=0`, `protocol.allow=never` with `https`, `ssh` and `git` allowed again | `remote_helper_transports_never_run` |
| L3 relative `PATH` entries | only absolute, non-empty entries, plus the executable's own folder | `only_absolute_path_entries_reach_the_child` |
| L4 credential prompts and windows | `credential.interactive=never`, empty `core.askPass` and `GIT_ASKPASS`, `GCM_INTERACTIVE=never`, no `SSH_ASKPASS` | `git_runs_exactly_the_canonical_argv_in_the_allowlisted_environment` |
| L5 a `setsid` grandchild holding the pipes outlives the time limit | the core's process runner waits for the output at most the grace period after the program ended, then returns what it has (truncated) | `deckpilot-core` `a_detached_grandchild_holding_the_pipes_does_not_outlast_the_run` |
| I2 redaction gaps | userinfo with `/` in the password, and the values of `access_token`, `private_token`, `oauth_token`, `token` and `password` query parameters | `credentials_in_queries_and_passwords_with_slashes_are_redacted` |
| Re-review: a `.git` file (`gitdir: ../secret/.git`) or a `.git` link leads to another repository | `rev-parse --show-toplevel --absolute-git-dir`: the real git directory must be inside the binding too, except a genuine linked worktree (`<common>/worktrees/<name>` with a `commondir`, whose `gitdir` back-pointer names this work tree's `.git`); a submodule's git directory (`.git/modules/…` of the superproject) is refused | `a_git_directory_outside_the_binding_is_refused_except_for_a_linked_worktree` |
| Re-review: userinfo containing `@` (`https://user:p@ss@host/`) | the **last** `@` before the host that ends a `user[:pass]` is the end of the userinfo | `credentials_in_queries_and_passwords_with_slashes_are_redacted` |
| I3 broker denials audited without the subcommand | the broker's record of a `tool.run` names the tool and its subcommand (`git status`; `?` when it is not a plain word), never another argument | broker `tool_run_denials_name_the_tool_and_subcommand_only` |

*Accepted risks that remain, stated precisely.*

- **Repository-local configuration.** `.git/config` of the bound
  repository may still set `core.sshCommand`, `core.gitProxy`,
  `remote.<name>.uploadpack` (run by `ls-remote`), `filter.*` (run by
  `status` for files that need them) and `protocol.<name>.allow=always`
  for a remote helper of a remote it configures. The plugin cannot write
  any of it; the built-in has the same exposure.
- **The project's `.env` and the compose file.** The `.env` inside the
  binding still interpolates the compose file and sets the other
  `COMPOSE_*` settings (profiles, for example); `-f` and `-p` override
  `COMPOSE_FILE` and `COMPOSE_PROJECT_NAME`. A compose file may `include`
  or `extends` files outside the folder, and `up` runs whatever it
  declares (privileged containers, bind mounts). That is the user's own
  file, as for the built-in.
- **The project name.** A file whose `name:` is interpolated (`${X}`)
  gets the folder's name, which may not be the project its containers were
  started under from the command line; the run then sees no containers.
  The built-in's `projectName` parameter has no plugin equivalent.
- **A repository below the binding's root only.** A binding that is a
  subfolder of a repository is `not-a-repository` (it used to work with
  the built-in); the binding must be the repository's own folder. Linked
  worktrees work; **submodules are not supported as bindings yet**
  (their git directory lives in the superproject), and neither are bare
  repositories.
- **Settings after the built-ins are removed (WP-C6).** `ToolSettings`
  reads the built-in `git` and `docker` modules' configuration. When WP-C6
  removes those modules, it must keep these values as host-held settings
  (migrated, not lost) and feed them to `tools::configure`, or every run
  falls back to `PATH` and the default time limits.
- **Settings and `PATH`.** A configured executable that is a name other
  than `git` or `docker` (not an absolute path) is refused. A
  desktop-started agent outside macOS finds the tools only on its own
  `PATH`, as the built-in did.
- **Output redaction** is best effort: a secret that is not in a URL's
  userinfo or one of the named query parameters reaches the plugin, which
  holds no network grant (common rule 6). A path with `:` before an `@`
  may be shortened more than needed (over-redaction, never under).

*As implemented, follow-up of 2026-11-20 (host gaps found while building
the first-party plugins).*

- **Docker reads without a handle.** `ps`, `logs` of a container,
  `version`, `context show` and `compose version` read no folder; a
  `tool.run` of one of them may omit `cwd` (`ToolRun.cwd` is optional in
  the ABI, `Context::run_tool_unbound` in the SDK). The agent then runs it
  with its host-held context in an empty folder it made itself
  (`<temp>/deckpilot-empty-<64 random bits>`, mode 0700, created with
  `create`, never reused once anything appeared in it); a Docker
  environment handle now runs in that folder too, instead of the system's
  temporary folder. Compose's project commands and every git command still
  need their handle (`tool-invalid-cwd` without one). This gives
  `deckpilot.docker`'s `status`, `containers` and container `logs` the
  built-in's behaviour (no resource, no project). *Tests:*
  `reads_need_no_resource` (`apps/agent/tests/plugins_docker.rs`), the
  ABI's `requests_are_bounded`.
- **A Compose project bound as a Docker environment.** As the built-in's
  compose role allowed, a `dockerEnvironment` binding whose value is an
  absolute path of an existing folder or file is, for Compose's project
  commands, that folder or file (by its real path, with M2's compose file
  rules); any other Docker environment stays a context name, and is
  refused for Compose.
- **`tools:docker:modify` is optional and off by default (X3c).** An
  action names the sub-grants it needs in `requiresPermissions` (plugin
  API 1.9, one of `tools:git:network`, `tools:docker:modify`,
  `resources:open`, `resources:paths`, `projectContext:select`,
  `openUrl:from-params`, declared by the manifest). A sub-grant an action
  names is optional: enabling, a plan's activation and an update's
  activation need every other grant only (`GrantSet::is_optional`,
  `missing_grants`); while it is not granted only the actions naming it
  are unavailable ("needs the permission `…`: allow it in Settings ›
  Plugins") and refused before the plugin runs; granting it makes them
  available, revoking it again stops only them. The consent dialog and
  Settings › Plugins show it with its own check, off by default for a high
  tier one. `deckpilot.docker`'s Compose up, down and restart name
  `tools:docker:modify`. The broker still refuses every call without the
  grant (deny by default). *Tests:*
  `changing_containers_is_a_separate_grant_off_by_default`, the store's
  `sub_grants_actions_name_are_optional_for_enabling`, the manifest's
  `actions_name_the_optional_sub_grants_they_need`, the desktop's
  `review-dialog.test.tsx`.
- **Key state per key.** A state call still runs no `tool.run` and gets no
  handle, but `run.context` is now answered in it, with the key's
  `keyScope`: 128 bits of SHA-256 of the plugin id, each `resource`
  parameter with its stored reference (never a path) and, where a binding
  follows the active project (a role, or an empty parameter), the active
  project's id. Every run with the same bindings (a press, a workflow
  node) gets the same scope, so the git and docker plugins keep and read
  state per key. A key of an action that changes something may show state
  from plugin API 1.9 (rule 6): Compose up and down keys show the
  built-in's toggle again, as a read of what the last run stored. *Tests:*
  `the_key_scope_names_the_bindings_of_a_key` (engine), the plugins'
  `keys_of_different_*_keep_their_own_state`, broker
  `a_state_call_reads_and_nothing_else_whatever_is_granted`.

#### X4 `resource.open`

*Purpose.* `workspace.open-in-editor`, `workspace.open-project` and
`git.open-repository`: open a bound folder or file in a bound application,
optionally at a line and column.

*Design.* `resource.open {handle, with?: handle, position?: {line,
column}, editor?}`. `with` must be a handle of kind `application`. `editor`
picks a host-held argument plan (`vscode`, `cursor`, `jetbrains`, `zed`,
`sublime`, `other`), ported from `modules/workspace/src/editor.rs:48-67`.
The launch goes through `crates/deckpilot-core/src/launch.rs`.

*Threats.* Launching an arbitrary program. Argument injection into the
editor (a path that starts with `-`). Path traversal from a bound location
(`../../.ssh`). Symlink escape. Opening many windows.

*Mitigations.* Only applications that are themselves machine bindings (the
user chose them), or the operating system's default handler. Paths are
canonical absolute paths checked with `existing_inside` and `join_inside`
(moved to the core, WP-A3), so they never start with `-`. Line and column
are integers from 1 to 1,000,000. Only in user-started runs. At most 1 per
second and 5 per invocation (`open-project` opens the editor, the docs and
the URLs). *Clarified 2026-11-20 (implementation):* a token bucket with a
burst of 5 (the per-invocation cap) that refills at 1 per second, so a
single `open-project` is never throttled by its own opens; `url.openParam`
likewise has its own bucket with a burst of 10 refilling at 1 per 2 s,
plus the shared 20 per 10 minutes window.

*Tier.* **High.** Explicit consent: it starts programs the user chose.

*Audit.* The labels of both handles, the editor plan, and the outcome.

*Residual.* A time-of-check to time-of-use gap between the containment
check and the launch, if a local attacker swaps a symlink. The built-in has
the same gap. The opened application does whatever it does with the path:
an editor may run workspace tasks it trusts, and that stays the editor's
trust model.

*Tests.* An unbound application, a handle of kind `directory` as `with`, a
`..` or symlink escape, and a path outside the location are each refused.
It is refused from a schedule, an event or startup run. The rate limits
hold. The editor argv matches the fixtures for each plan.

*As implemented, follow-up of 2026-11-20: `path`.* `resource.open
{handle, path?, …}` opens a file or folder inside a `directory` handle,
for the built-in workspace's `path`: a relative path of `/`-separated
names (no `..`, `.`, empty name, backslash, colon or control character,
at most 1,024 bytes; the ABI refuses anything else), joined with
`join_inside` and resolved with `existing_inside` (`bound_inside`): it must
exist and its real path stay inside the binding, so a `..` or a link that
leads out is `invalid-path`. The agent opens it by its canonical absolute
path (never starting with `-`); a line needs a file (`invalid-position`),
and a `path` with a handle that is not a folder is refused. The record
says that a relative path was opened, never which. *Tests:*
`a_path_inside_a_bound_folder_opens_and_nothing_outside_it` (executor)
and `a_path_inside_the_bound_folder_opens_and_nothing_outside_it`
(`apps/agent/tests/plugins_workspace.rs`).

*Decision of 2026-11-20 (WP-A5, coordinator within the owner's
authorization): URL roles.* A project's URL role (a binding of kind `url`)
resolves to a handle of the new ABI kind `url` (`resource.resolve {role,
kinds: ["url"]}`, by label only) and `resource.open` opens it with the
system's default browser only: never with `with`, `position` or `editor`,
under X5's URL rules (http or https, no userinfo, printable ASCII, the host
written literally, at most 2,048 bytes), in user-started runs, within the
5 per invocation, audited by label and origin. This gives
`workspace.open-project` its URL roles as the built-in has them. *Tests:*
a bound address opens in the browser and nowhere else; with an
application, a position or an editor, as a non-literal URL, as `with`,
and from a schedule it is refused.

#### X5 `url.openParam`

*Purpose.* `workspace.open-urls` and the URL roles of `open-project` open
URLs the user typed into the action, on any host. `openUrl.hosts` cannot
express that.

*Design.* `url.openParam {param, index?}`. The host opens the **literal
stored value** of this invocation's own parameter (`Lines`, so `index` is
from 0 to 9). The plugin never passes a URL.

*Threats.* A plugin opening a URL it chose (phishing, drive-by). A crafted
parameter value (`javascript:`, `file:`, a custom scheme, userinfo
spoofing such as `https://bank.example@evil.example`). Template injection
through variables.

*Mitigations.* Only `http` and `https`. No userinfo. At most 2,048 bytes.
Values produced by templates or workflow mappings are **excluded** in 1.9:
only a value the user authored in the document. Private and local hosts
are allowed, because the user typed them, as `system.open-url` allows. Only
in user-started runs. 1 per 2 s and 20 per 10 minutes, as `url.open`. The
per-invocation limit is 10, not `url.open`'s 3, to match `open-urls` (at
most 10).

*Tier and consent.* `openUrl.fromParams`, medium. Derived in a migration
when the built-in was in use.

*Audit.* The origin only, never the path or query.

*Tests.* A templated value is refused. Each of `javascript:`, `file:`,
`data:`, a custom scheme and userinfo is refused. An index out of range is
refused. It is refused from a schedule. The 11th call in an invocation is
`Throttled`.

*Decision of 2026-11-20 (WP-A5, coordinator within the owner's
authorization): provenance instead of no workflows.* The core's
`resolve_parameters` reports where each value came from
(`deckpilot_core::resolution::Provenance`: `Literal`, `Default`,
`Template`, `Reference`, `Mapped`), the engine marks a workflow node's
overrides (its mappings) `Mapped`, and the handler's context carries it.
`url.openParam` opens only a `Literal` value: one the user wrote as it is
into the instance, a binding's overrides or a step's overrides. It does so
in every user-started run, workflows the user started included. Defaults
(which the plugin's manifest supplies), templates, references and
mappings stay refused (`url-param-not-literal`), and so does every value
when the caller gave no provenance. *Tests:* a literal value in a
user-started workflow opens; a template-expanded or mapped value is
refused; a non-user run is refused.

#### X6 Project operations (`projectContext`)

*Purpose.* `deckpilot.projects` reads, lists, selects and cycles the active
project. The data and `ActiveProjectContext` stay in the core
([§3.3](#33-projects-and-work-session)).

*Design.* `project.current`, `project.list` (ids, keys and names of the
active profile's projects only: no bindings, no paths),
`project.select {id}`, and `project.cycle {direction, wrap}`. They are
applied by the agent's existing `ActiveProjectContext`, which emits
`ProjectChanged` with source `plugin:<id>`.

*Threats.* A confused deputy: switching the project silently changes which
repository git touches and which compose file docker brings up. Loops with
`LifecycleEvent::ProjectChanged` triggers. Reading other profiles'
projects.

*Mitigations.* Select and cycle are allowed only in user-started runs and
in workflow runs with cause `user`. They are **never** allowed from a
lifecycle event, a schedule or the agent's start, so the plugin cannot loop
on `ProjectChanged`. Rate limit 2 per second. The active profile only. Each
switch is shown in the status bar ("Project changed by Projects"), so a
silent switch is visible.

*Tier.* Read ops (`current`, `list`) medium. Select and cycle **high**,
explicit consent. `projectContext` leaves `NEVER_GRANTABLE` only behind the
first-party gate.

*Consent.* See [X13](#x13-migration-consent-and-the-upgrade-path). Users of
project cycling wait for one consent.

*Audit.* The operation, the from and to project ids, and the run origin.

*Tests.* Select from a lifecycle-triggered run is refused. Two selects
within 500 ms: the second is `Throttled`. An id of another profile gives
`not-found`. `ProjectChanged{source: plugin:deckpilot.projects}` is
emitted. A third-party manifest declaring `projectContext` is refused at
validation.

#### X7 Host-published context variables

*Purpose.* `projects.current.{id,key,name}` become
`deckpilot.projects.current.*`, scope Project, while their values stay the
host's.

*Design.* A variable declared with `scope: "project"` and `source:
"host:project.{id,key,name}"`. The host computes and publishes it. The
plugin never writes it: `variables.publish` for such an id is refused.

*Threats.* A plugin forging project values that other workflows trust.

*Mitigations.* No new authority. The values are the host's own. The values
are published only while the plugin is enabled. 1.9 has only these three
`host:` sources. First-party only.

*Tier.* None.

*Tests.* `variables.publish` on a host-sourced id is refused. The values
disappear when the plugin is disabled and return when it is re-enabled. A
`host:` source other than the three is refused at validation.

#### X8 `hostViews`, `statusItems` and `dashboardCards`

*Purpose.* Workflows and Projects become **app-drawn host views that the
plugin activates**. Plugins ship no UI code. What moves is ownership,
activation and removability. The React code stays in the app bundle, as the
`settings` view kind's renderer does today.

*Design.* Closed enumerations: view kinds `workflows` and `projects`,
`statusItems: ["activeProject"]`, `dashboardCards: ["activeProject"]`, and
a navigation entry at a position the host fixes. All are drawn by the host
from core data.

*Threats.* UI spoofing (a plugin view that mimics DeckPilot's own chrome or
a consent dialog). Crowding the app's navigation. Data access through a
view.

*Mitigations.* No plugin markup, script or styles. The host draws the view
and decides its strings. There is no data ABI: the view reads core IPC as
the app does today. First-party only, so a third party cannot add chrome.
Disabling or uninstalling the plugin hides the view. The data stays in core
tables and returns on re-enable.

*Tier and consent.* `hostViews`, low, derived. It grants placement, not
data.

*Tests.* A third-party manifest with `hostViews` is refused. An unknown
view kind is refused under `^1.9`, and is ignored by an older app (IPC
`PluginView.kind` is a string, `crates/deckpilot-ipc/src/plugins.rs:187-193`).
With the plugin disabled there is no navigation entry, status item or card
(WP-T1's desktop test).

#### X9 Profile read (`profile` kind)

*Purpose.* git and docker show the project key. `general` and
`notifications` already declare the need (G8). The kind exists, but it has
no operation yet ([permissions.md](sdk/permissions.md)).

*Design.* `profile.read` returns the profile name with `read: profile`,
and the active project's id, key and name with `read: project`. It never
returns layouts, other profiles, bindings or paths.

*Threats.* Disclosure of client or project names.

*Mitigations.* Only the scopes declared. It is the existing kind for every
publisher, so it is not first-party gated.

*Tier.* Medium (existing). Explicit consent for third parties. Derived in a
migration where the built-in declared `ProfileRead`.

*Audit.* The scope.

*Tests.* An undeclared scope is refused. Disabling the plugin denies the
op. Without an active project the result is `null`, not an error.

#### X10 Alias parameter rules `Keep` and `Set`

*Purpose.* Route the legacy ids (plan §4.1).

*Review.* No new authority. One requirement: `Keep(resource)` passes the
stored `ResourceRef` to the **host** resolver (X1). It must never pass it to
the guest as a path. `Set` is not used in 1.9: the deprecated
`projects.next|previous|active` stay same-name deprecated plugin actions.

*Tests.* Contract tests in `crates/deckpilot-modules/tests/contracts.rs`. A
routed legacy `git.status` with a fixed repository reaches the guest as a
handle and never as a path.

#### X11 Workflow engine API: not offered

The Workflows plugin needs no data ABI, because its view is host-rendered
(X8). `workflow.list` and `workflow.run` stay unimplemented, and the
`workflows` kind stays never grantable. A later plugin that must run
workflows needs its own review.

#### X12 First-party publisher gate

*Purpose.* Keep every 1.9 privileged kind out of third-party reach.

*Design.*

- The validator accepts the kinds only through
  `Manifest::validate_first_party` (publisher `deckpilot`).
- The grant needs, at every agent start and every enable, an installed
  record with `source = 'bundled'` whose files match the compiled-in sha256
  pins, plus the macOS code seal.
- A `deckpilot.*` package from a registry, even one the registry's key set
  authorizes for the `deckpilot` namespace (B6), and any sideloaded or
  developer-mode package ([local-testing.md](local-testing.md)) **does not
  receive these kinds**. Its manifest is refused if it declares them.

*Threats.* A third party publishing under a look-alike id. A tampered
bundle on disk. A registry compromise delivering a privileged
`deckpilot.*` update.

*Mitigations.* The gate above. The registry never serves
`BUNDLED_PLUGIN_IDS`. Updates to these plugins ride app releases only.

*Residual.* The trust is DeckPilot's release pipeline. A compromised
release compromises the agent anyway, so the gate adds no new trust root.
Opening any of these kinds to third parties needs a separate written
review.

*Tests.* A registry-sourced `deckpilot.git` with `tools` is refused. A
bundled copy whose `plugin.wasm` does not match the pin is quarantined and
holds no grant. A sideloaded `deckpilot.*` manifest with any gated kind is
refused. A third-party publisher with each gated kind is refused.

#### X13 Migration consent and the upgrade path

*Purpose.* Existing users keep their keys ([§11](#11-owner-decisions-of-2026-11-20) D4).

*Design.*

- The migration engine installs and enables a bundled extracted plugin only
  where the module's data is in use (a saved `modules` row or a stored
  instance). Fresh installs and unused modules get nothing (no silent
  install of high-tier plugins).
- Low tiers are derived. Medium tiers are derived only when demonstrated.
- High tiers are **always** asked once: `tools:*`, `resources.open`, and
  `projectContext` select and cycle.
- `tools:docker:modify` is offered only when `allowStateChanges` was on,
  and it is asked for, not pre-checked.
- Settings are copied (`gitExecutable`, `dockerExecutable` and `context`
  into host-held tool settings, the rest into plugin settings).
- No `deckpilot.db` write.
- An encrypted, verified backup `pre-plugin-extraction` is taken before
  the first cut-over, and its name is journaled.
- **Eligibility without the built-in.** v0.10.0 ships with the built-ins
  removed (D2, D5). Users who upgrade straight from v0.9.x have never run
  these migrations. Today eligibility requires "the built-in module is
  compiled in and enabled" (`eligibility`,
  `apps/agent/src/plugins/migration.rs:650`), and `demonstrated` reads the
  built-in's manifest. Each removed module therefore needs a static
  **legacy descriptor** kept in the agent as data: its id, its declared
  capabilities, and its settings keys and kinds. Eligibility then uses the
  stored `modules` row and the stored instances. This is a precondition of
  WP-C5 and WP-C6.

*Threats.* Consent fatigue or a pre-approved dialog. A plugin triggering
consent prompts. Data loss. A confused deputy where a lifecycle event
completes a migration step on the user's behalf.

*Mitigations.* Only the agent starts the prompt. The plugin cannot. One
dialog per plugin lists every high-tier item, each unchecked. Declining
keeps the keys Unavailable with "Allow" and "Restore" actions. Documents
are never rewritten, so every step is reversible. Consent is recorded
`decided_by = user:migration:<module>`.

*Residual.* With the built-ins removed (D2), a user who has not consented
has no fallback. Their git, docker, workspace and project-cycling keys show
Unavailable until they click. The owner accepted this.

*Tests.* Per module in `apps/agent/src/plugins/migration/tests.rs`:
eligible, awaiting consent (the keys are Unavailable with a reason),
consent given (cut over), declined (still Unavailable, recoverable),
`allowStateChanges` off (modify never offered), and the backup recorded
before the first cut-over.

## 8. Installer size

Measured: the assets of `v0.6.0-alpha.3`
(`gh release view v0.6.0-alpha.3 -R knittingcode/deckpilot --json assets`).

| Asset | Size |
| --- | --- |
| `DeckPilot_0.6.0-alpha.3_aarch64.dmg` | 15.5 MB |
| `DeckPilot_0.6.0-alpha.3_x64.dmg` | 16.3 MB |
| `DeckPilot_0.6.0-alpha.3_x64_en-US.msi` | 17.2 MB |
| `DeckPilot_0.6.0-alpha.3_x64-setup.exe` | 12.3 MB |
| `DeckPilot_0.6.0-alpha.3_amd64.deb` | 18.2 MB |
| `DeckPilot_0.6.0-alpha.3_amd64.AppImage` | 95.1 MB (bundles its own libraries) |

A local release build (arm64, `lto = true`, `codegen-units = 1`,
`strip = true`) has a 12.6 MB `deckpilot-agent` and a 19.5 MB
`deckpilot-desktop`. Both link `deckpilot-modules`
(`apps/agent/Cargo.toml:64`, `apps/desktop/src-tauri/Cargo.toml:19`), so
every module is in the installer twice. The plugin host built from
`plugins/runtime` is 12.0 MB. The sample plugin `hello_weather.wasm` is
227 KB.

Module size, estimated from release rlibs. These are pre-LTO and include
metadata, so final machine code is roughly 10–20% of them. There are no
heavy dependencies: `git` and `docker` drive their CLIs, with no `git2` and
no `bollard`.

| Crate | Release rlib | Exclusive heavy dependencies (agent, default features) |
| --- | --- | --- |
| `http` | 1.34 MB | the agent's whole HTTP/TLS stack: `reqwest` (4.6 MB rlib), `hyper` and `hyper-util` (4.0), `rustls` (8.4), `ring` (3.5), `tokio-rustls`, `rustls-platform-verifier`, `tower` (`cargo tree -p deckpilot-agent -e normal -i reqwest` lists only the four HTTP modules) |
| `docker` | 0.93 MB | none |
| `gitlab` | 0.89 MB | via `http` |
| `github` | 0.86 MB | via `http` |
| `jira` | 0.79 MB | via `http` |
| `git` | 0.63 MB | none |
| `system` | 0.51 MB | `sysinfo` (0.83 MB) |
| `projects` | 0.45 MB | none |
| `workspace` | 0.41 MB | none |
| `notifications` | 0.28 MB | none |
| `work-session` | 0.13 MB | `chrono` (also used by the agent) |
| `general` | 0.11 MB | none |

Estimate:

- **Optional module code** (`git`, `docker`, `http`, `github`, `gitlab`,
  `jira`, `workspace`: 5.8 MB of rlibs) is about 0.6–1.2 MB of machine code
  per binary, so 1.2–2.4 MB uncompressed for both. At the roughly 50%
  compression the dmg shows (32 MB of binaries in a 15.5 MB image), that is
  **0.6–1.2 MB per installer, 4–8%**.
- **The HTTP/TLS stack** is in the agent only because of the HTTP modules,
  worth about 2–3 MB uncompressed. It is **not a saving**, though. The
  agent's `plugins` feature needs the same stack for brokered fetches and
  the HTTPS registry client (`apps/agent/Cargo.toml` features
  `dep:reqwest`, `dep:rustls`, `dep:deckpilot-module-http`). The desktop
  binary has it anyway, through `tauri-plugin-updater`.
- **Shipping plugins** adds the plugin host (12.0 MB uncompressed, about
  5 MB compressed) and about 0.2–0.3 MB per bundled plugin.

Net: an installer with plugins enabled and the optional modules removed is
about **4 MB larger** than today's. Migration is justified by isolation, by
updates independent of the app and by the user's control, not by size.
These figures are estimates from rlibs. A measured figure needs one
release build with `deckpilot-modules` reduced, which this audit did not
make (disk budget).

## 9. PR sequence

Small, dependency-ordered PRs. *Owner* is the area each PR touches. Each PR
waits for the listed Marketplace PRs to merge, so it never edits a file
they change.

| # | PR | Files | Depends on | Notes |
| --- | --- | --- | --- | --- |
| P0 | This audit and design | `docs/marketplace/module-migration.md`, `module-matrix.md`, `docs/README.md` | — | docs only |
| P1 | Core action aliases and routing, with an empty table | `crates/deckpilot-core/src/alias.rs` (new), `module.rs` (`action_registry`, `availability`), `registry.rs` (`AliasHandler`); tests in `crates/deckpilot-core` | #231 (touches core variables and `deckpilot-modules/tests/contracts.rs`) | Reserves `deckpilot.core`, `deckpilot.project`, `deckpilot.resource`. No behaviour change: the table is empty |
| P2 | Move the HTTP transport to `crates/deckpilot-net` | `modules/http/src/{client,policy}.rs` → `crates/deckpilot-net`; imports in `modules/{http,github,gitlab,jira}`, `apps/agent/src/plugins/executor.rs` | #231, #232 (both edit `executor.rs`) | Pure move, so the broker's transport no longer lives in a module crate (§3) |
| P3 | `plugins.db` schema 3 | `crates/deckpilot-plugin-store/src/{db,state,store,manager}.rs` | #232 (schema 2) | `source = 'bundled'`, `module_migrations`, `plugin_tombstones`, refusal tests for schema 2 builds |
| P4 | Host features G1, G6 (then G8, G9) | `plugins/runtime/crates/{abi,broker}`, `plugins/runtime/guest/plugin-sdk`, `apps/agent/src/plugins/executor.rs` | #231, #232 | Marketplace stream; coordinate ownership with its author |
| P5 | Bundled plugins: build, pins, seeding | `plugins/first-party/` (new), `crates/deckpilot-plugin-bundled` (new: index and pins), `apps/agent/src/plugins/bundled.rs` (new), `apps/desktop/src-tauri/tauri.release.conf.json` (`resources`), `tools/release-pipeline/check-bundled-plugins.sh` (new) plus separation fixtures | #225, P3, owner decision on §4.4 | Uses the `default` class only, with a test fixture plugin. No module changes yet |
| P6 | Migration engine | `apps/agent/src/plugins/migration.rs` (new), the routing hand-off to the core in `apps/agent/src/service.rs` (`load_modules`) | P1, P3, P5 | Eligibility, steps, drift, failure, retries. Tested with a fixture module and plugin |
| P7 | IPC and app: migration notice, consent, Retry; alias-aware editors | `crates/deckpilot-ipc/src/plugins.rs`, `apps/desktop/src/plugins/migration*.tsx` (new), `apps/desktop/src/lib/plugin-actions.ts` | #228, P6 | Additive protocol fields |
| P8 | `deckpilot.github` plugin source and contract test | `plugins/first-party/github/` (new), `crates/deckpilot-modules/tests/contracts.rs` | P4 (G1, G6), P5 | Built and pinned, not yet routed |
| P9 | GitHub cut-over | one `ACTION_ALIASES` block, the `github` `MigrationPlan`, a `migration`-class entry in `bundled/pins.json` | P6, P7, P8 | The first behaviour change, in plugin-enabled builds only. The built-in stays as the legacy implementation. **Landed** ([§6](#6-the-first-module), *As built (P9)*) |
| P10 | `deckpilot.notifications` as the first default plugin | `plugins/first-party/notifications/`, its alias block | P4 (G8, G9), P9 | Proves first launch, tombstones and fresh installs. *Done*: G9 routing and delivery, class `default` in the engine (§6 "As built (P10)") |
| P11 | G2 user origins, then gitlab and jira | runtime, executor, settings consent UI, `plugins/first-party/{gitlab,jira}/` | P9 | *Done* for GitLab and Jira: G2 (#266), `openUrl.userOrigins`, both plugins and their cut-overs ([§6](#6-the-first-module)). GitHub Enterprise users stay on the built-in until `deckpilot.github` gets a user-origin slot (follow-up) |
| P13 | Plugins on in builds with the `plugins` feature | `apps/agent/src/{main.rs,plugins/mod.rs}`, agent tests | P10, P11 | *Done* (§5.10): no switch needed; `--disable-plugins` / `DECKPILOT_ENABLE_PLUGINS=0` turn plugins off. The release workflow and packaging enable the feature separately |
| P12 | Remove a legacy built-in | `modules/<name>`, `crates/deckpilot-modules/src/lib.rs` | §5.7 criteria | The alias entries stay forever |

*P12 for the extraction set* is split into P34 and P35 below. The §5.7
criterion 1 is waived for them (§11 D2).

#### P14 onwards: the extraction of 2026-11-20

Each PR is one work package (WP) of the extraction plan and lands on its
own, squash-merged when its checks pass. The exceptions are P16 and P21,
which need the owner's approval (security policy). Hot files have one owner
per wave: `alias.rs`, `migration.rs` `PLANS`, `executor.rs`, the manifest
crate, both schema copies, the runtime `abi`/`broker`, `bundled.json`,
`deckpilot-modules`, `App.tsx`, `app-sidebar.tsx` and `modules/index.ts`.

| # | WP | PR | Depends on | Notes |
| --- | --- | --- | --- | --- |
| P14 | WP-00 | Land the `app-review` worktree on `main` | — | Blocks everything below |
| P15 | WP-01 | Bundled allowlist accepts `locales/<tag>.json` and the `visualAssets` formats (`crates/deckpilot-plugin-bundled`, `check-bundled-plugins.sh`) | P14 | Fixes a latent seeding refusal |
| P16 | WP-02 | This decision record and the 1.9 security reviews (docs only) | — | **Owner approval**, not auto-merge |
| P17 | WP-B0 | `deckpilot.github` user-origin slot for GitHub Enterprise | P14 | Existing API; makes GHE users eligible (D2 precondition) |
| P18 | WP-A1 | Manifest kinds, `^1.9` gate, first-party gate, tiers, both schemas | P14, P16 | Validator only, no new authority |
| P19 | WP-A2 | ABI capabilities, broker authorization, guest SDK, `sdk/` docs | P18 | Deny by default for each op |
| P20 | WP-A3 | Resource handles (X1, X2); path helpers move to the core; `executor.rs` split | P19 | |
| P21 | WP-A4 | `tool.run` (X3) with the denial matrix | P20, **P16 approved** | **Owner approval** |
| P22 | WP-A5 | `resource.open` (X4), `url.openParam` (X5) | P20 | Parallel with P21 after the split |
| P23 | WP-A6 | Project ops (X6), host context variables (X7), profile read (X9) | P20 | Parallel with P21 and P22 |
| P24 | WP-B1 | `deckpilot.git` | P21 | |
| P25 | WP-B2 | `deckpilot.docker` | P21 | |
| P26 | WP-B3 | `deckpilot.workspace` | P22 | |
| P27 | WP-B4 | `deckpilot.projects` (actions, variables, host views) | P23 | |
| P28 | WP-B5 | `deckpilot.workflows` (manifest only, host view) | P19 | |
| P29 | WP-C1 | Bundle and route: `bundled.json`, aliases, `PLANS`, `Keep`, pre-extraction backup, contract tests, legacy descriptors (X13) | P24-P28 | Owns the hot files |
| P30 | WP-C2 | Variable aliases `projects.current.*` | P29 | |
| P31 | WP-C3 | Desktop gating; resource-binding editor to Settings › Resources | P30 | Parallel with P32 |
| P32 | WP-C4 | Agent and Tauri gating (`require_projects`, `settle_project`, tray) | P30 | |
| P33 | WP-T1 | "Core without optional plugins" check, a required CI job | P31, P32 | Acceptance gate for P34 and P35 |
| P34 | WP-C5 | Remove the `github`, `gitlab`, `jira` built-ins | P17, P33 | §5.7 criterion 1 waived (D2) |
| P35 | WP-C6 | Remove the `git`, `docker`, `workspace`, `projects` built-ins; `ProjectCatalog` to the core | P33, P29's legacy descriptors | **Before v0.10.0** (D1, D5), not "one release later" |

**Release cut line.** v0.10.0 is cut only after P35 has merged and the
whole set is green: full CI, WP-T1, the denial matrix, the migration tests
and the release smoke. Nothing in P14 to P35 is released before that. The
work can merge to `main` gradually, because the plugins stay unrouted
until P29.

This PR touches none of `modules/`, `apps/agent/src/plugins/`, `plugins/`
or `docs/distribution/`. P2 and P4 touch files the Marketplace stream owns,
so they wait for #231 and #232 and are coordinated with that stream rather
than run in parallel.

## 10. Blockers and owner decisions

| # | Decision or blocker | Blocks |
| --- | --- | --- |
| B1 | Registry trust configuration and keys (U-10, U-12) | independent updates of bundled plugins; "fresh install gets only the default set" (§5.8); Marketplace-sourced replacements |
| B2 | Bundled trust: option B (pins in the agent), with or without A (new decision) | P5 and everything after it |
| B3 | Consent policy: derived grants for low and medium tiers, and explicit consent for credentials and private network during a migration (§5.5) | P6, P7 |
| B4 | ~~Confirm that `git`, `docker`, `system.run-command`, `http`, `workspace` and `system.process-status` stay built-in (G13 and §6), and that `general`, `projects`, `work-session` and the `system` launchers are core~~ **Superseded 2026-11-20 by [§11](#11-owner-decisions-of-2026-11-20).** `system.run-command` and `system.process-status` remain built-in; `general`, `work-session`, `http` and the `system` launchers remain core | the scope of P12 |
| B5 | Where first-party plugin sources live: in this repository (`plugins/first-party/`, built into the bundle by the app's release build) and also submitted to `knittingcode/deckpilot-plugins` by pinned commit for registry updates | P5, P8 |
| B6 | Reserve the `deckpilot` publisher in the registry | registry publication of first-party plugins |
| B7 | The removal criteria for legacy implementations (§5.7) | P12 |
| B8 | Approval of the plugin API 1.9 security reviews ([§7](#plugin-api-19-security-reviews-2026-11-20)) | P19 to P23, especially P21 (`tool.run`) |
| B9 | Docker modify from scheduled workflows: refused by X3c (allowed only in user-started and work-session runs). Confirm, or allow the `schedule` cause explicitly | P21, P25 |
| B10 | Workspace and Projects keys also wait for one consent (`resources.open` and `projectContext` select/cycle are high tier, X13), not only Git and Docker keys. Confirm, or name a lower tier for one of them with a reason | P29 |

## 11. Owner decisions of 2026-11-20

Decided by the owner on 2026-11-20. These decisions supersede B4 of
2026-11-15 and the conclusions of G12, G13 and G15 for the modules named.
The analysis in those sections still stands. The security reviews in
[§7](#plugin-api-19-security-reviews-2026-11-20) answer it.

| # | Decision |
| --- | --- |
| D1 | **Extract Workflows, Docker, Git, GitHub, GitLab, Jira, Projects and Workspace into plugins.** No duplicate developer functionality stays in the core. The core keeps only system capabilities: devices, profiles, layouts, pages and folders, the protected Back key, navigation, gestures and dispatch, the action and workflow-**execution** primitives, rendering, live preview, IPC, the plugin host and permissions, the Marketplace client, encrypted storage, backup and restore, security, the notifications infrastructure, settings, updates, the shared UI, the appearance systems and i18n. `work-session`, `http` and the `system` launchers stay core |
| D2 | **The legacy `github`, `gitlab` and `jira` built-ins are removed** once their plugins cover every user (GitHub Enterprise included, P17) and the "core without optional plugins" check (P33) passes. This explicitly waives the §5.7 "two minor releases" rule. Users whose migration waits on consent see recoverable Unavailable keys |
| D3 | **Process execution for Git and Docker is a new high-tier permission** ([X3](#x3-toolrun-brokered-git-and-docker)). The manifest allow-lists the executables (`git` and `docker` only). Arguments are an array. There is no shell. The working directory is limited to the project's resource folders (resource handles). Time and output are limited. Explicit consent is asked on first use. Every run is audited. Hardening adopted within that model: allowed subcommands and options for each tool, a denylist of dangerous flags, an environment allowlist, mutating Docker operations as a separate grant that is off by default, and the first-party publisher gate. Third-party plugins cannot request it |
| D4 | **Existing users.** The extracted plugins ship bundled. The migration engine installs and enables only those whose data or keys are in use. Data and keys are preserved. Git and Docker keys wait for the one-time consent (see B10 for Workspace and Projects) |
| D5 | **Release v0.10.0 only when the whole extraction is complete and green** (the cut line in [§9](#9-pr-sequence)) |
| D6 | **Plugins cannot ship UI code.** The Workflows and Projects screens become app-drawn host views that the plugin activates ([X8](#x8-hostviews-statusitems-and-dashboardcards)). Ownership, activation and removability move to the plugin. The React code stays in the app. "Extracting" these two screens does not take their code out of the app binary, and cannot without a plugin UI framework, which D6 forbids |

**What stays in the core, stated plainly**

- The **project data model** stays in the core as a neutral project-context
  primitive (§3.3). It covers `Project`, `ActiveProjectContext`, the
  storage tables, the FKs, portability and IPC. `deckpilot.projects` owns
  every user-facing part of it.
- The **workflow engine**, `BindingTarget::Workflow`, the execution IPC and
  the run monitoring views stay in the core. With `deckpilot.workflows`
  off, keys that run workflows still run them, and existing workflow
  bindings show read-only.
- The **resource-binding store** (`machine_config`) and its editor stay in
  the core (Settings › Resources), because the `system.open-*` launchers
  use them.
- **No `deckpilot.db` schema change.** Stored profiles and layouts are
  never rewritten. Routing is by `ACTION_ALIASES` (§5.2).

**Mapping to the PR sequence.** P14 to P35 in [§9](#9-pr-sequence), with
P16 (this record) and P21 (`tool.run`) needing the owner's approval. The
release cut line is v0.10.0 after P35.
