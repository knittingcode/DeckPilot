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
| `system` | — (settings) | read machine resources, open applications/files/pages, run programs, see running programs |

The integration modules and their actions are described in
[Integration modules](integration-modules.md).

### Action catalog

Modules define generic action *types*; users create configured action
*instances*. No type or instance is ever generated per project or resource,
and nothing is added to the Action Library automatically.

| Type | Parameters | Result |
| ---- | ---------- | ------ |
| `general.log` | `message` | the rendered message |
| `general.wait` | `seconds` (1–300) | waits; output `waitedMs`; stops at once when cancelled (the reference workflow node) |
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

## Workflow node contract

Every action type is also a workflow node. This is the contract between a
module, the Rust engine (the only place actions execute) and the editor
(which only reads metadata and never runs module code). Contract version 1.

**Identity and versioning.** A node type is its action type id,
`<module>.<name>` (e.g. `general.wait`); the module id prefix ties it to its
module and the module's capabilities. `NodeSpec::version` versions the
node's parameter and output schema. Changing a parameter's meaning or an
output's key or kind requires a new version and a module migration
(`ModuleManifest::migrations`) for stored instances; adding an optional
parameter or an output does not.

**Parameter schema.** `ActionType::parameters`: key, label, description,
kind (`text`, `integer` with bounds, `boolean`, `choice`, `resource`,
`project`), required, default. The engine validates and normalizes
parameters before a handler runs; handlers receive only valid values.

**Input schema.** A node's inputs are its parameters. Each can be the
action's configured value, a value for this node (`overrides`), or a data
mapping from earlier nodes' outputs (one output, or text with
`{{ steps.<node>.output.<key> }}`). Mapped values are converted to the
parameter's kind; the engine checks references against earlier nodes'
declared outputs. Resource parameters cannot be mapped.

**Output schema.** `NodeSpec::outputs`: key, label, kind (`text`,
`integer`, `boolean`, `path`, `url`, `projectKey`), whether it is always
present on success, and whether it is `sensitive`. Outputs are the `data`
of a successful `ActionResult`. Only declared outputs are passed to later
nodes' reports, events and history (safe output serialization); sensitive
outputs (a program's output, a response body) reach later nodes but never
events or history. Messages are shortened to 500 characters. Never put
secrets in outputs or messages. Build outputs with `OutputSpec::new(..)`
and `.optional()` / `.sensitive()`.

**Effect.** `NodeSpec::effect`: `readOnly` (default), `opens` (opens
something for the user) or `modifies` (changes state). Shown in the catalog
and the inspector.

**Execution context.** `ActionHandler::execute(parameters, context)` gets
an `ExecutionContext`: the profile, the node's project (the run's own
project context, not necessarily DeckPilot's active project), this
computer's resource bindings — each available only with the matching
capability — and, in workflows, the attempt's cancellation token and a
progress reporter. Handlers do not mutate state; they request changes as
`ContextEffect`s (e.g. `SelectProject`), which the engine applies to the
run's context after checking the module's capability.

**Capability permissions.** The module manifest declares capabilities
(`ProfileRead`, `ProjectContext`, `MachineResources`, `OpenExternal`,
`ProcessExecution`, `ProcessInspect`, `Network`, `Notifications`,
`Credentials`); the context refuses the rest (`CapabilityDenied`). Core
helpers check them (`resolve`, `run_process`); for the others a handler
calls `context.permit(capability)` before acting. Actions that are risky
even for a permitted module, such as running user-chosen commands, are
additionally off until a machine-specific module setting allows them. A node that selects projects declares
`changesContext`; it changes the run's context only. Changing DeckPilot's
active project is never a node's decision: it is the workflow's project
scope (see [Workflow graphs](workflow-graphs.md#project-context-and-the-active-project)).

**Cancellation.** In a workflow, every attempt has a token
(`context.cancellation()`), cancelled when the run is cancelled or the
attempt's timeout passes (`CancelReason::Cancelled` /
`DeadlineExceeded`). Long-running handlers check `context.is_cancelled()`,
wait with `context.wait(duration)` instead of sleeping, run programs with
`context.run_process(command, timeout, limits)` — `deckpilot_core::process::run`
bounded by the action's own time limit, so a single action started from a
key is bounded too — (which stops the program's process group:
`SIGTERM`, `SIGKILL` after a grace period; the process tree on Windows; no
orphans, nothing outside the group), and return promptly. They declare
`NodeSpec::cancellable = true`. A handler that cannot be interrupted keeps
the default `false`; the editor then says that it finishes before a
cancellation or timeout takes effect, and nothing claims otherwise. After a
cancellation the agent waits up to 10 s for the run to stop; a run still
busy with a non-interruptible action is then recorded as cancelled, and the
action's late result is discarded.

**Timeout.** `NodeSpec::timeout_ms` (default 30 s; a node may override it,
at most 10 minutes). Cooperative handlers stop at the deadline; a result
arriving late is a timeout and its effects are discarded.

**Retryability.** `NodeSpec::retry_safe`: the engine retries a failed
attempt (up to five times, exponential backoff, cancellable waits) only when
it is safe; otherwise retry settings are refused. Each attempt starts from
the node's input context.

**Idempotency.** `NodeSpec::idempotent`: running twice has the same effect
as once. Informational for users and future replays.

**Structured errors.** A failed node reports `StepError { code, message,
retryable, actionCode }` with stable codes: `action.failed`, `timeout`,
`cancelled`, `mapping`, `context-conflict`, `unavailable`, `loop.failed`.
For `action.failed`, `actionCode` is the action's own code
(`ActionResult::error(code, message)`), documented by its module, e.g.
`exit-nonzero`. Cancellation is
never reported as a failure or a timeout, and a timeout never as a
cancellation.

**Progress events.** `context.report_progress(percent, message)` emits
`nodeProgress` (percent 0–100 optional, short safe message); the editor
shows it on the running node. Outside workflows it is ignored. The engine
emits `nodeStarted`, `retrying` and `nodeFinished` itself.

**Backward compatibility.** Every `NodeSpec` field has a default, so action
types that predate a field keep working (`cancellable` defaults to `false`,
the honest value). Stored graphs carry a format version
(`WorkflowGraph::version`); workflows without a graph run as steps.

**Reference implementation.** `general.wait` (`modules/general/src/lib.rs`)
implements the whole contract: a bounded integer parameter with a default,
a declared typed output, its own timeout, `retrySafe`, `idempotent`,
`cancellable`, cooperative waiting through the context and progress
reports. Its tests show each part.

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

## Portability contract

Each module declares in its manifest what may leave the computer in an
export (`Portability`):

- `version` — the format version of the module's exported data (`0`: the
  module exports nothing);
- `settings` — the scope of each setting: `portable`, `machineSpecific` or
  `sensitive`. Settings that are not listed are treated as machine-specific
  and never exported.

On import, the core checks the module's data without running module code:
the module must be installed, the exported version must not be newer than
the module's contract (older versions are accepted), settings that are not
portable for the local module are dropped and reported, and the rest must
satisfy the module's configuration schema. Data of modules this build does
not include is reported and not imported. Actions of such modules are
imported and shown as unavailable.

## Module-owned schema

Modules can own database tables through declared, versioned SQL steps
(`ModuleManifest::migrations`). The core applies pending steps for every
registered module — enabled or not, so enabling a module later never meets
an old schema — in one transaction per module, and records the version in
`module_schemas`. A module schema newer than the installed module is
refused. The projects module declares version 1 to take ownership of the
`projects`, `default_projects` and `active_projects` tables created by core
schema 3; later changes to those tables belong in its own steps.

## Still planned

- **Module-owned profile data:** a per-profile, per-module document handed
  to the module, with the profile's lifecycle (duplicate, delete) applied by
  the core through hooks, so that projects can leave the core profile model.
- **Runtime context slots** instead of the built-in active project, with
  effects checked against a capability per slot, and module-provided label
  placeholders.
- **Import migrations in code** for modules whose exported data changes
  shape; until then, a module accepts older export versions only if their
  settings still validate.

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
