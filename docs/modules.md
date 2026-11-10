# Feature modules

DeckPilot is built from feature modules. The core application (profiles,
persistence, IPC, the action engine, devices and layouts) knows nothing about
individual features. A module contributes:

- action types and their executors (`ActionHandler`s),
- navigation pages and dashboard cards in the configuration app,
- a configuration schema (edited in Settings › Modules),
- declared capabilities (what its code may do),
- dependencies on other modules and the platforms it supports.

Modules are compiled in and registered explicitly. There is no loading of
third-party executables and no remote installation.

## Built-in modules

| Module     | Pages | Capabilities |
| ---------- | ----- | ------------ |
| `general`  | — | read profile |
| `projects` | Projects | read profile, change active project, read machine resources |

### Action catalog

Modules define generic action *types*; users create configured action
*instances*. No type or instance is ever generated per project or resource,
and nothing is added to the Action Library automatically.

| Type | Parameters | Result |
| ---- | ---------- | ------ |
| `general.log` | `message` | the rendered message |
| `projects.next` / `projects.previous` | — | switches along the project order (respects `wrapAround`) |
| `projects.select` | `project_id` (optional) | with a project: selects it; without: reports the current project and changes nothing |
| `projects.active` | — | reports the current project |
| `projects.repository` | `project_id` (optional) | resolves the project's `repository` resource |
| `projects.documentation` | `project_id` (optional) | resolves the project's `docs` resource |
| `projects.resolve-resource` | `project_id` (optional), `resource` (name) | resolves any named resource of the project |

`project_id` is a stable project id. Unset (absent or `null`) means *the
current project*, decided when the action runs; the editor's "Current
project" option stores exactly that, never the id of the project active at
configuration time. An explicit project must belong to the action's profile:
the app refuses to save anything else and the engine refuses to run it.

The three resource types share one resolver
(`modules/projects/src/resources.rs`). They only report what this computer
binds to the resource; opening a folder or a browser, running Git, Docker or
an editor belongs to the modules for those tools. Results are structured
(`ActionResult.data`): `projectId`, `projectKey`, `projectName`, and for
resources `resource`, `resourceKey`, `kind` and `value`. Missing resources
fail with a message naming the project and resource.

The projects module owns the Projects page, project management (create,
edit, reorder, delete, default project, explicit copy to another profile),
active project switching (actions, the tray and the app all use
`switching::step`) and project resources. Its setting `wrapAround` decides
whether next/previous wrap around at the ends.

## Where things live

| Part | Location |
| ---- | -------- |
| `ModuleManifest`, `FeatureModule`, `ModuleRegistry`, `Capability` | `crates/deckpilot-core/src/module.rs` |
| Action dispatch and capability checks | `crates/deckpilot-core/src/registry.rs` |
| Module crates | `modules/general`, `modules/projects` |
| List of built-in modules | `crates/deckpilot-modules` (the only place that names them) |
| Module state | `modules` table (`ConfigStore::module_states`) |
| Backend commands of a module | `apps/desktop/src-tauri/src/modules/` |
| Frontend of a module | `apps/desktop/src/modules/<id>/`, registered in `src/modules/index.ts` |

## Rules

- **Namespaces.** Every action type id is `<module id>.<name>`. Registering a
  module whose handler uses another namespace fails. The owner of any stored
  action can therefore be found from the action alone.
- **Dispatch.** The engine executes actions through the `ActionRegistry`
  built from the enabled modules. There is no list of action types in the
  core.
- **Capabilities.** A handler only sees what its module declared. The
  `ExecutionContext` a module receives is restricted to its capabilities:
  `profile()` and `project()` require `profile-read`, resource resolution
  requires `machine-resources`. Handlers return effects instead of changing
  state; the registry rejects an effect the module did not declare (a project
  switch needs `project-context`), and the engine only selects projects of the
  active profile.
- **No side doors.** Handlers are only reachable inside the core. Execution,
  profile checks (`check_parameters`) and key decoration (`decorate_face`) all
  go through the registry, which applies the same capability limits.
- **Profile checks.** `ActionHandler::check` validates parameters against the
  action's profile, e.g. that a selected project belongs to it. The app
  refuses to save such actions and the engine refuses to run them.
- **Key faces.** `ActionHandler::decorate_face` lets a module add context to
  a key, such as the active project below a *Next project* key or the
  highlight of a *Select project* key.

## Security boundary

Modules are compiled-in Rust code, so the boundary protects against mistakes
and keeps responsibilities honest; it is not a sandbox against hostile code
(a module crate could in principle use the standard library directly).
Within that scope:

| Path | Enforcement |
| ---- | ----------- |
| Action execution (keys, workflows, IPC `runAction`/`runWorkflow`) | registry dispatch; disabled modules have no handler; restricted context; effect check |
| Profile checks and key faces | registry; profile/project hidden without `profile-read` |
| Tray and app project switching | agent and app refuse while the projects module is disabled |
| Project management and resource checks in the app | `AppState::require_module` |
| Module enable/disable | the app waits until a running agent has reloaded |

## Enabling and disabling

The agent and the app read module states at start; the app changes them and
asks the agent to reload. When a module is disabled:

- its pages, dashboard cards and action types disappear from the app,
- its action types are not executable; running a stored action reports
  "the Projects module is disabled",
- actions, workflows and layouts that use it are kept unchanged and shown as
  unavailable; they work again once the module is enabled,
- its backend commands are refused (for projects: editing projects and
  switching the active project, including from the tray).

A module cannot be disabled while an enabled module depends on it, and cannot
be enabled without its dependencies or on an unsupported platform. Invalid
stored states fall back to safe values instead of failing the start.

## Remaining dependencies of the core on projects

These are deliberate for now and documented so they can move later:

- `Profile.projects`, `Profile.default_project`, `Project`, `ProjectKey`,
  `ActiveProjectContext`, `ProjectSwitcher` and `CycleDirection` are part of
  the core profile model and runtime session.
- Label templates `{project.key}` / `{project.name}` (`face::render_label`).
- `ResourceRef::ProjectRole` and `MachineConfig::resolve` use the active
  project.
- `StepCondition::ActiveProject` in workflows.
- `ParameterKind::Project` / `ParameterValue::Project`, and
  `ContextEffect::SelectProject` with `Capability::ProjectContext`.
- IPC requests `selectProject`, `cycleProject`, `getActiveProject`, and the
  active project in the agent status and tray.
- Storage tables `projects`, `default_projects`, `active_projects`.

None of these depend on a module crate; `deckpilot-core` has no dependency on
any module.

## Planned interfaces

For a later milestone, so that these parts can move into modules without
breaking stored data:

- **Module-owned data:** `ModuleData` — a per-profile, per-module JSON value
  (`profile_module_data(profile_id, module_id, version, document)`) handed to
  the module, with the profile's lifecycle (duplicate, delete) applied by the
  core through `FeatureModule::remap_ids` / `on_profile_deleted` hooks.
- **Runtime context:** a typed slot map in the session
  (`ContextSlots<ModuleId, Value>`) instead of the built-in active project,
  with `ContextEffect::Set { slot, value }` checked against a capability per
  slot. Label templates would ask modules for placeholders
  (`FeatureModule::placeholders`).
- **Module migrations:** `FeatureModule::migrations() -> &[ModuleMigration]`
  with `ModuleMigration { version: u32, apply: fn(&Transaction) -> Result }`,
  tracked per module in a `module_migrations (module_id, version)` table and
  run after the core migrations in the same transaction. Migrations of
  disabled modules still run, so enabling a module later never sees old data.

## Adding a module

1. Create a crate under `modules/<id>` that depends on `deckpilot-core` and
   implements `FeatureModule` with a manifest and its handlers. Use
   `<id>.<name>` action type ids.
2. Register it in `deckpilot_modules::builtin`.
3. If it has pages, add `apps/desktop/src/modules/<id>/` and list it in
   `FRONTEND_MODULES`; backend commands go to
   `apps/desktop/src-tauri/src/modules/<id>.rs` and check
   `AppState::require_module`.
4. Add tests for the handlers (with an `ExecutionContext`) and for the module
   in `crates/deckpilot-modules/src/tests.rs`.

The Action Library and Workflow editor generate their forms from the
parameter schema, so a new action type needs no UI code.
