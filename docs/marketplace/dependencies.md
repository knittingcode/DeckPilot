# Plugin dependencies (plugin API 1.4)

A plugin can declare the plugins and built-in modules it needs. DeckPilot
then resolves everything installing it takes, verifies all of it, shows the
whole plan with the permissions of every plugin in it, and installs nothing
until the user confirms. Disabling, uninstalling, updating and rolling back
respect the plugins that depend on one another.

A dependency is **only a precondition**. It grants nothing. A dependency
never inherits the permissions of the plugin that needs it, and the plugin
never inherits the dependency's. Every plugin runs with the permissions the
user granted to that plugin, and only those, and every plugin asks for its
own consent.

## 1. The manifest

Both fields are optional and need `"engines": { "pluginApi": "^1.4" }`
(`DEPENDENCIES_API`). The validator refuses them with a lower level. A
manifest without them depends on nothing, which covers every manifest
written before 1.4.

```json
"dependencies": [
  { "id": "acme.projects", "version": ">=1.2.0 <2.0.0" },
  { "id": "projects", "version": ">=0.8.0" },
  { "id": "acme.calendar", "version": ">=1.0.0", "optional": true,
    "justification": "Shows your next meeting on the status key." }
],
"contributes": {
  "actions": [
    { "id": "next-meeting", "requiresDependencies": ["acme.calendar"], … }
  ]
}
```

| Field | Rule |
| --- | --- |
| `dependencies` | 1 to 16 entries, each `id` at most once, never the plugin's own id |
| `dependencies[].id` | A plugin id (`<publisher>.<name>`), or the id of a built-in module (one lowercase word such as `projects`). A `deckpilot.*` id is satisfied only by DeckPilot's own plugin: from the application bundle, or from a registry the root-signed key set authorizes for `deckpilot` |
| `dependencies[].version` | The versions accepted, in the grammar of `engines.deckpilot` (`VersionRange`: one comparator, or a lower and an upper bound, e.g. `>=1.2.0 <2.0.0`). It must be satisfiable. A built-in module is matched against DeckPilot's version |
| `dependencies[].optional` | `true`: an optional integration. It is never installed unless the user selects it. While it is missing only the actions that name it are unavailable |
| `dependencies[].justification` | 1 to 200 characters, shown before installing. The validator warns when an optional dependency has none |
| `contributes.actions[].requiresDependencies` | 1 to 16 unique ids of declared dependencies. While one of them is missing, disabled or in a version outside its range, this action is unavailable on its own. Naming a required dependency is a review warning, since every action needs those already |

The bundled JSON Schema (`crates/deckpilot-plugin-manifest/schema/`, and
`plugins/schemas/`) has the same rules.

### What a dependency is for

Plugins cannot call each other. Each one reaches DeckPilot only through its
own broker. A dependency says that the plugin works only together with the
other plugin, for example because it reads that plugin's published
variables (`variables` permission) or a built-in module's variables
(`projects.current.*`), or because the two are meant to be used together. A
plugin that would merely be more useful next to another one should leave it
out, or declare it optional.

## 2. The registry

The publishing tool copies a version's `dependencies` into its entry of the
signed plugin detail (`DetailVersion.dependencies`, left out when there are
none). That lets a client resolve the whole graph from signed documents
before it downloads anything. When the package arrives, the installer
refuses it if its manifest declares other dependencies than the signed
entry. Documents of plugins without dependencies are byte-for-byte
unchanged.

Clients before 1.4 (DeckPilot 0.8.1 and earlier) refuse a detail document
with the unknown field, so they do not list a plugin that has dependencies.
They could not run it anyway: they also refuse its manifest.

## 3. Resolution (`deckpilot_plugin_store::deps`)

`resolve` is a pure function over a snapshot (`Universe`). The snapshot
holds the installed plugins and their active manifests, the agent's
built-in modules (id, version, on or off, supported here), and the
verified registry versions of every plugin reachable from the requested
one. Resolution walks the required dependencies recursively, and the
optional ones the user selected, depth first in manifest order. The same
inputs always give the same plan.

- **Installed plugins that fit are kept.** An installed, enabled version
  that satisfies every constraint needs nothing (`keep`). A disabled one is
  turned on (`enable`), with consent to whatever it lacks.
- **Built-in modules are never downloaded.** They are checked against
  DeckPilot's version and the operating system. One that is turned off
  blocks the plan until the user turns it on in Settings › Modules.
  Plugins of the application bundle are seeded at first launch and are
  "built in" in the plan.
- **Otherwise the highest version wins.** It must satisfy every
  constraint, not be yanked or revoked, run on this DeckPilot (app
  version, plugin API, platform) and, for a dependency, come from a
  publisher the registry verified. Pre-releases are only used when they
  are requested exactly.
- **Installed plugins constrain too.** Each installed plugin that stays as
  it is adds its own ranges. A shared dependency is therefore never moved
  to a version that another installed plugin does not accept. An installed
  plugin's optional range counts while that dependency is installed.
- **Errors name the plugins involved.** A dependency cycle is refused. So
  are a missing plugin or module, a version conflict (every constraint
  with who imposed it, and the versions that exist), an incompatible or
  withdrawn version, an unverified publisher of a dependency, a dependency
  that is broken or quarantined, and a plan of more than 32 plugins.

A constraint found late can rule out a version chosen earlier. The walk
then starts again with every constraint seen so far, at most 32 times.
Because constraints are kept across rounds, a rare graph where only a
discarded version imposed one is reported as a conflict that a complete
solver might avoid. The resolver never returns a plan that violates a
constraint.

## 4. Installing (`planPluginInstall`, `installPluginPlan`)

1. **`planPluginInstall { plugin, version?, optional? }`.** The agent
   refreshes the registry and resolves the plan. It downloads **every**
   package the plan installs or updates to and verifies each one: pins,
   manifest, artifact contract, and the manifest's dependencies against
   the signed entry. It then loads each module in a host of its own, as an
   update is checked. It answers the plan (`PluginInstallPlan`): the steps,
   dependencies first and the requested plugin last, each with its action
   (`install`, `update`, `enable`, `keep`, `builtin`, `enableBuiltin`), why
   it is needed, its publisher and its own permissions. The answer also
   lists the optional integrations left out and a `fingerprint`. Nothing
   is installed. The verified packages are kept in memory for the
   confirmation.
2. **The app shows the plan** (Marketplace › plugin › Install). It lists
   every plugin with its permissions, each permission with its own
   checkbox. Optional integrations are checkboxes that plan again.
3. **`installPluginPlan { plugin, version?, optional?, fingerprint,
   consent: { pluginId: [key] }, activate }`.** The agent resolves again
   and refuses (`planChanged`) unless the fingerprint is the same. The
   fingerprint covers every step and the pinned sha256 of every package.
   The agent refuses consent for a plugin outside the plan, and consent to
   a permission a plugin does not declare (`unknownPermission`). It then
   calls `PluginManager::apply_plan`:
   - It checks before writing anything: with `activate`, every plugin it
     installs or turns on must end up with all of its permissions,
     otherwise `consentRequired` and nothing changes. A declined
     permission of a dependency stops the whole plan.
   - It installs and updates in plan order. Each step is a crash-safe
     store operation of its own, and new plugins start out disabled.
   - It records the plugins installed only as dependencies
     (`plugin_dependency_installs`).
   - With `activate`, it enables in the same order once everything is
     installed, so a plugin is enabled only after everything it requires.
     Enabling checks the files against their integrity record again.
   - If a step fails, the finished steps are undone in reverse order. A
     plugin the plan enabled is disabled, one it installed is uninstalled
     (keeping any settings and credentials an earlier uninstall kept), and
     one it updated is rolled back. The error (`PlanFailed`) names the step
     and anything that could not be undone.

   A crash in the middle leaves every finished step complete and
   consistent, either installed and disabled or updated. Recovery finishes
   the interrupted step, and plugins installed as dependencies are then
   offered as orphans.

Installing a plugin that is already installed as a dependency adopts it.
From then on it counts as installed by the user and is no longer offered
as an orphan.

`installPlugin` (one plugin, no plan) still works and installs exactly one
plugin. A plugin whose required dependency is missing then installs but
cannot run (see §6).

## 5. Disabling and uninstalling (`planPluginRemoval`, `removePlugin`)

- `disablePlugin` and `uninstallPlugin` are refused with `requiredBy` while
  an enabled plugin requires the plugin. The message names those plugins.
- `planPluginRemoval { plugin, uninstall }` describes the effects without
  changing anything. It lists the installed plugins that require the plugin
  and the enabled ones that would be turned off with it, transitively and
  in order (`disables`). It lists the plugins whose optional actions stop
  (`usedBy`) and, for an uninstall, the plugins installed only as its
  dependencies that nothing else needs any more (`orphans`, transitively).
- `removePlugin { plugin, uninstall, keep?, withDependents, orphans }`
  carries that out. With `withDependents`, the dependents are turned off
  first. They are never uninstalled. After an uninstall, the orphans
  **listed** are uninstalled too, with the same `keep`. An orphan that was
  not listed stays, and so does every plugin the user installed.
- A built-in module that an enabled plugin requires cannot be turned off.
  The plugin's module declares its required dependencies to the agent's
  module registry, which already refuses to disable a module that an
  enabled module requires (`ModuleError::Required`).

## 6. Running

- A plugin whose required dependency is missing, disabled, in a version
  outside its range or unusable cannot run. Its actions are unavailable,
  with the reason (`it requires the plugin acme.projects >=1.2.0, which is
  not installed`). The plugin keeps its state.
- An action that names an optional dependency in `requiresDependencies` is
  unavailable while that dependency is not satisfied. It fails at once with
  `plugin-unavailable` and the reason, and never reaches the plugin. The
  plugin's other actions run.
- Availability follows changes: installing, enabling, disabling or
  updating a dependency re-registers every plugin's actions.

## 7. Updates and rollback

- `updatePlugin` resolves the new version first. It is refused if an
  installed plugin's range excludes that version, or if the version needs
  dependencies that are not installed (`dependency`). In that case,
  `planPluginInstall` with that `version` plans the update together with
  what it needs, and `installPluginPlan` carries it out with the same
  checks and the same rollback. Signatures, pins, permission escalations
  (staged until consent) and the kept settings, credentials and storage
  work as before.
- `rollbackPlugin` is refused if an enabled dependent does not accept the
  previous version.
- A plan never updates a shared dependency to a version another installed
  plugin refuses.

## 8. State: `plugins.db` schema 5

Schema 5 adds `plugin_dependency_installs (plugin_id → plugins ON DELETE
CASCADE, required_by, installed_at)`. The change is additive: a plugin
without a row was installed by the user, which covers every plugin
installed before schema 5. The configuration database (`deckpilot.db`,
schema 10) is unchanged.

Before it upgrades an existing `plugins.db`, the store writes an encrypted
copy with the same key to `plugins/.backups/plugins-schema<n>-<ms>.db`. The
copy is written under a temporary name and renamed only after it opened at
schema `n` and passed `quick_check`. The newest three are kept. An agent
that knows schema 4 (0.8.1) refuses the upgraded file before writing
anything (`UnsupportedSchema`). Going back means restoring that copy as
`plugins.db`, just as a downgrade of the configuration database means
restoring its `deckpilot-schema9-…` backup.

## 9. Tests

| Where | What |
| --- | --- |
| `crates/deckpilot-plugin-manifest/tests/dependencies.rs` | legacy manifests unchanged; parsing, validation and round trip; the 1.4 gate; self, duplicate, unsatisfiable and unknown references; malformed entries refused by the model and the JSON Schema alike |
| `crates/deckpilot-plugin-store/tests/dependencies.rs` | resolution: simple, transitive, shared (with a restart), installed kept or enabled, optional only when selected, built-in modules, bundled plugins, missing, conflicts, cycles, incompatible plugin API and platform, withdrawn, unverified publishers, never breaking an installed plugin's range, updates checked against dependents; status and per-action availability; removal plans and transitive orphans. Plans with real verified packages: dependencies first with their own consent, no inherited permissions, a declined permission installs nothing, rollback of a failed plan (an update rolled back, an install removed), an escalating shared-dependency update, dependency-aware removal and adoption, legacy plugins |
| `crates/deckpilot-plugin-store/src/db.rs` | schema 4 → 5 with a verified encrypted backup, rows kept; a schema 4 build refuses the upgraded file without writing; backup pruning |
| `apps/agent/tests/plugins_e2e.rs` (`plugins_with_dependencies_are_installed_and_removed_together`) | the real agent, plugin host and a signed test registry: the plan, `planChanged`, a declined dependency permission, stray consent, the install with each plugin's consent, dependency state, an action unavailable for its missing optional dependency, `requiredBy`, removal with dependents and an orphan |
| `apps/desktop/src/plugins/dependencies.test.tsx` | the details sections, the plan dialog with per-plugin consent, optional integrations, turning off with dependents, uninstall offering orphans, agents without the feature |

## 10. What is not done

- Built-in modules are never turned on by a plan: one that is off blocks
  the plan until the user turns it on.
- A plugin of the application bundle that the user uninstalled
  (tombstoned) is not reinstalled by a plan. The plan says to restore it
  first. A bundled plugin the user only turned off is turned on again when
  the user confirms a plan that needs it.
- Device capabilities (`supports.device.requires`) are not part of
  resolution. The registry detail does not carry them.
- The Marketplace's Update button still updates one plugin. An update
  that needs new dependencies is refused with the reason, and planning
  that update from the plugin's page is not offered in the UI yet.
