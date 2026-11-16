# Moving built-in modules to plugins

Status: **proposed** (design record; no code changes). It applies to the
experimental plugin marketplace. Nothing here changes a release build until
the PRs in [§9](#9-pr-sequence) land, and several steps wait on owner
decisions ([§10](#10-blockers-and-owner-decisions)).

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

## 4. The default system plugin set

### 4.1 What ships bundled

| Plugin | From | Condition | Permissions |
| --- | --- | --- | --- |
| `deckpilot.notifications` | `notifications.show`, `notifications.workflow-summary` | G8 (run summary of the invocation's own workflow) and G9 (`notify` with severity and critical) | `notifications` (low tier) |

`system` and `general` contribute nothing ([§3](#3-the-core-boundary)). The
set is deliberately small. A bundled plugin must not need a permission the
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
  reinstalls or re-enables. Only the user can undo it, by installing or
  enabling the plugin again in Settings › Plugins, which clears the
  tombstone.

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
`.dpplugin` into the bundle (`src-tauri/` or `bundled/`). Packaging the
host and `bundled/` is opt-in: `DECKPILOT_RELEASE_PLUGINS=1` with
`tauri.plugins.conf.json` on top of `tauri.release.conf.json`. The
published release workflow stays unchanged, so no release contains plugins
until the owner enables the marketplace. See
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
| Origin settings (`apiUrl`, `siteUrl`) | a `network` grant for that exact origin | Eligible only if the origin is the plugin's declared default, or G2 exists. `allowPrivateHost = true` is never migrated automatically (G3, high tier) |
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
  the engine, empty like the alias table until a cut-over.
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
- Until P7 adds the consent dialog and Retry, they are engine functions;
  agent tests drive them through test hooks. The tests use a fixture
  legacy module and alias (feature `test-hooks`, never in a release
  build) and the fixture plugin
  `plugins/first-party/test-fixtures/legacy-fixture`.

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
| `plugins.db` | schema 2 → 3 (`module_migrations`, `plugin_tombstones`, `source = 'bundled'`) | refuses schema 3 with `UnsupportedSchema` (`crates/deckpilot-plugin-store/src/db.rs:163`). The agent reports the plugin store as unavailable and runs without plugins (`apps/agent/src/plugins/service.rs:294-302`). Release builds do not enable plugins today |

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

`notifications` follows as the first `default`-class bundled plugin. It is
the one that proves seeding, tombstones and first launch.

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
through the core launcher. Not yet: the restriction to invocations a key
press or the app started. The invocation does not carry its trigger in
ABI 1; that arrives with G8's run context, and the consent text says only
what is enforced today.

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

### G14 Scoped file read: not needed

No module that could otherwise move reads files. `git` and `docker` need
more than reads (G13). **Not offered.**

### G15 Project-context write and G16 work-session write: never

`ProjectContext` is never grantable (`permissions.rs:31`), and the work
session's state machine is the agent's. Read access is covered by G8 and by
the `variables` read kind (`workSession.*`, `projects.current.*`), whose
operation does not exist yet. **Blocker: `projects` and `work-session` stay
core.**

### G17 Bundled source, pins and tombstones (infrastructure)

Covered in [§4](#4-the-default-system-plugin-set) and [§5.6](#56-the-migration).

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
| P9 | GitHub cut-over | one `ACTION_ALIASES` block, a `migration`-class entry in `bundled/pins.json` | P6, P7, P8 | The first behaviour change. The built-in stays as the legacy implementation |
| P10 | `deckpilot.notifications` as the first default plugin | `plugins/first-party/notifications/`, its alias block | P4 (G8, G9), P9 | Proves first launch, tombstones and fresh installs |
| P11 | G2 user origins, then gitlab and jira | runtime, executor, settings consent UI, `plugins/first-party/{gitlab,jira}/` | P9 | GHE users become eligible |
| P12 | Remove a legacy built-in | `modules/<name>`, `crates/deckpilot-modules/src/lib.rs` | §5.7 criteria | The alias entries stay forever |

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
| B4 | Confirm that `git`, `docker`, `system.run-command`, `http`, `workspace` and `system.process-status` stay built-in (G13 and §6), and that `general`, `projects`, `work-session` and the `system` launchers are core | the scope of P12 |
| B5 | Where first-party plugin sources live: in this repository (`plugins/first-party/`, built into the bundle by the app's release build) and also submitted to `knittingcode/deckpilot-plugins` by pinned commit for registry updates | P5, P8 |
| B6 | Reserve the `deckpilot` publisher in the registry | registry publication of first-party plugins |
| B7 | The removal criteria for legacy implementations (§5.7) | P12 |
