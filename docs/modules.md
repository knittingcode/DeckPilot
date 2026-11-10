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

| Module     | Action types | Pages | Capabilities |
| ---------- | ------------ | ----- | ------------ |
| `general`  | `general.log` | — | read profile |
| `projects` | `projects.next`, `projects.previous`, `projects.select`, `projects.active`, `projects.resolve-resource` | Projects | read profile, change active project, read machine resources |

The projects module owns the Projects page, project management (create,
edit, reorder, delete, default project, explicit copy to another profile),
active project switching and project resources. Its setting `wrapAround`
decides whether next/previous wrap around at the ends.

Project data itself (`Project`, the profile's project list and the per-profile
active project) belongs to the core profile model. Label templates
(`{project.key}`), project resource roles and workflow conditions are core
features, so they keep working (and stored data stays readable) while the
module is disabled.

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
- **Capabilities.** Handlers return effects instead of changing state. The
  registry rejects an effect the module did not declare (for example a
  project switch from a module without `project-context`), and the engine
  only selects projects of the active profile.
- **Profile checks.** `ActionHandler::check` validates parameters against the
  action's profile, e.g. that a selected project belongs to it. The app
  refuses to save such actions and the engine refuses to run them.
- **Key faces.** `ActionHandler::decorate_face` lets a module add context to
  a key, such as the active project below a *Next project* key or the
  highlight of a *Select project* key.

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
