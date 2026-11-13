# Variables: the unified registry

Status: **implemented** in three stacked changes: the registry and module
declarations, the single resolver with protocol 9, and the Variables page
and picker. This document records the audit of the variable code
on `main` at `592d5f1` and the target design of the unified variable
registry. It extends `docs/action-architecture.md` (§4–§11), which stays the
reference for shared variables, the reference grammar and the resolution
pipeline of actions.

## 1. Audit (main at 592d5f1)

### 1.1 How variables are declared and who owns them

| Kind | Declared in | Owner today | Resolved by |
|---|---|---|---|
| System variables `system.…`, `workflow.…` | the closed enum `SystemVariable` in `crates/deckpilot-core/src/variables/expression.rs` (paths, types, descriptions hard-coded) | the core, for all of them, including project and work session values | `deckpilot_core::resolution::system_value` from the `ResolutionContext` and the profile |
| Work session `system.work.session.…` | `WorkSessionVariable` in `crates/deckpilot-core/src/work_session.rs`, wrapped by `SystemVariable` | the core enum; values only the agent knows | the agent: `apps/agent/src/service/variables.rs::work_session_value` from the live `WorkSessionSnapshot` (through `VariableStore`, the "shared" resolver) |
| Shared variables `shared.<key>` | the configuration database (schema 8: `shared_variables`, `shared_variable_values`), `crates/deckpilot-storage/src/variables.rs` | the user (`module_id IS NULL`); the store can also hold module-declared definitions (`declare_module_variables`), which no module uses | `SharedVariables::resolve` on a per-profile snapshot (`select_value`: execution > project > profile > global > default) |
| Node outputs `steps.<node>.output.<key>` | the workflow graph | the workflow | the executor's outputs |
| Workflow variables `steps.vars.output.<key>` | the workflow (`WorkflowVariable`) | the workflow | the executor |

There is no module declaration of variables: `ModuleManifest` has no
variables, and nothing ties `system.project.current.…` to the projects
module or `system.work.session.…` to the work session module. Disabling the
projects module does not affect `system.project.current.…`; disabling a
module that declared shared variables makes them `permissionDenied`
(`VariableStore::set_disabled_modules`).

### 1.2 Every identifier exposed today, and its canonical id

| Current id | Type | Canonical id (owner) | Status |
|---|---|---|---|
| `system.project.current.id` | projectId | `projects.current.id` (projects) | alias, deprecated since 0.6.0 |
| `system.project.current.key` | string | `projects.current.key` (projects) | alias, deprecated since 0.6.0 |
| `system.project.current.name` | string | `projects.current.name` (projects) | alias, deprecated since 0.6.0 |
| `system.profile.current.id` | profileId | `system.profile.current.id` (core) | unchanged |
| `system.profile.current.name` | string | `system.profile.current.name` (core) | unchanged |
| `system.device.current.id` | string | `system.device.current.id` (core) | unchanged |
| `system.platform` | string | `system.platform` (core) | unchanged |
| `workflow.id` | string | `workflow.id` (core) | unchanged |
| `workflow.execution.id` | string | `workflow.execution.id` (core) | unchanged |
| `system.work.session.id` | string | `workSession.id` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.state` | string | `workSession.state` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.startedAt` | string | `workSession.startedAt` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.elapsedSeconds` | integer | `workSession.elapsedSeconds` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.projectId` | projectId | `workSession.projectId` (work-session) | alias, deprecated since 0.6.0 |
| `system.work.session.canFinish` | boolean | `workSession.canFinish` (work-session) | alias, deprecated since 0.6.0 |
| `shared.<namespace>.<name>[.…]` | as defined | unchanged (user) | unchanged |
| `steps.<node>.output.<key>` | text | unchanged (workflow) | unchanged |
| `steps.vars.output.<key>` | as declared | unchanged (workflow) | unchanged |

Key labels also keep their own placeholders `{profile}`, `{project.key}` and
`{project.name}` (`face::render_label`); they are not references and are not
changed.

### 1.3 Resolution pipeline today

1. A key press (`device_runtime.rs::dispatch`), a workflow node
   (`graph/executor.rs`), a trigger or an inline invocation reaches
   `Engine::run_action_with` with an `Invocation` (device, workflow, run).
2. `Invocation::context` builds the `ResolutionContext` (profile, project and
   its source, device, workflow, execution, phase).
3. `Engine::resolver` builds a `ContextResolver { profile, os, shared,
   outputs }`:
   - `system.` and `workflow.` → `system_value` from the context;
   - `system.work.session.…` and `shared.` → the injected `shared` resolver
     (the agent's `VariableStore`), confined to the profile;
   - `steps.` → the run's outputs.
4. `resolve_parameters` binds whole-value references (`bind`, native types),
   renders templates as text, collects sensitive values in a `Redactor`,
   then `ActionType::normalize` validates.
5. Previews (`previewReferences`) use the same `ContextResolver` with the
   database's current shared variables and `preview` (redaction, type
   check). The catalog (`listVariables`) is built by the agent's
   `catalog()` from `SystemVariable::ALL` and the stored definitions.
6. Graph validation (`graph/validate.rs`) and `ActionType::normalize` check
   static types of `system.` references (`SystemVariable::value_type`).

### 1.4 Caching

- **Agent, runs:** `VariableStore` keeps one `SharedVariables` snapshot per
  profile, taken at the last reload (`reload_variables` after
  `notify_reload`); a run started after a reload reads the new values.
  `reload` returns the changed keys, announced as `sharedVariablesChanged`.
- **Agent, previews:** read the database directly on every request.
- **Work session:** never cached; the snapshot is read from the watch
  channel at each resolution and elapsed time is computed then.
- **System variables:** never cached; computed from the context.
- **App:** `useCatalog`/`usePreviews` (`components/variable-picker/
  use-variables.ts`) hold the last answer and refresh on the agent's context
  events (`agent-variables` Tauri event), with a generation counter that
  drops obsolete answers. No polling.

### 1.5 IPC and events

Protocol 7 added `listVariables` (metadata only), `previewReferences`, and
the events `projectChanged`, `profileChanged`, `sharedVariablesChanged`
(keys only; also the `system.work.session.…` paths that changed),
`resourceBindingsChanged`, `deviceContextChanged`,
`executionContextChanged`. Protocol 8 added the work session requests and
`workSessionChanged`. Responses are adapted to older clients by
`Response::for_protocol`; events newer than the client's protocol are not
sent (`server.rs`).

### 1.6 Frontend

- Variable Picker (`components/variable-picker/`): groups Shared / System /
  Profile / Project / Workflow / Step outputs, search, type filter, live
  previews from the agent; falls back to the static `SYSTEM_ENTRIES` in
  `lib/variables.ts` for agents before protocol 7.
- Shared Variables settings page (`pages/shared-variables.tsx`): a list of
  user variables with an editor, plus a separate read-only "System
  variables" card from the static `SYSTEM_VARIABLES` in
  `lib/shared-variables.ts`.
- `workflow-builder/variable-reference.ts` holds another static table of
  system variable types.

## 2. Target design

### 2.1 Owners and namespaces

- **Core** owns only genuine system variables, in the namespaces `system`
  and `workflow`: `system.profile.current.id`, `system.profile.current.name`,
  `system.device.current.id`, `system.platform`, `workflow.id`,
  `workflow.execution.id`.
- **Modules** own the variables they declare, inside their namespace: the
  module id in lower camel case (`projects` → `projects`, `work-session` →
  `workSession`, `module_namespace`). A reference to a module variable has
  the module namespace as its root: `{{ projects.current.key }}`.
  - Projects owns `projects.current.id`, `projects.current.key`,
    `projects.current.name`.
  - Work Session owns `workSession.id`, `.state`, `.startedAt`,
    `.elapsedSeconds`, `.projectId`, `.canFinish`.
- **Users** own shared variables, `shared.<key>`. The user namespace stays
  open except the reserved prefixes `steps`, `system` and `workflow` (D4,
  "open except reserved"): a user variable `shared.projects.favorite` is
  fine; it can never collide with a module id because shared variables
  always have the `shared.` root.
- Reserved roots that no module may use as its namespace: `steps`,
  `shared`, `system`, `workflow`. A module named after one (the `system`
  module) cannot declare variables.

### 2.2 Declarations and the registry

`crates/deckpilot-core/src/variables/registry.rs`:

- `VariableDeclaration { id, name, description, valueType, scope,
  requires, sensitive, readOnly }`. `scope` (`DeclaredScope`) says what the
  value depends on (machine, profile, project, device, workflow, execution,
  workSession); `requires` lists what must be in the context
  (`Requirement`). Core and module variables are always read-only.
- `ModuleManifest::variables` holds a module's declarations;
  `FeatureModule::variable_source` returns the `VariableSource` that
  resolves them.
- `VariableRegistry` keeps the core's variables and every registered
  module's, by canonical id, with their owner. `ModuleRegistry` keeps it in
  step with the module lifecycle: `register` validates the declarations
  (namespace, reserved namespace, one owner per namespace, no duplicate id,
  read-only, a source exists) and refuses the whole module otherwise;
  `unregister` removes the module and its variables; disabling a module (or
  running on an unsupported platform) marks its variables unavailable.
- A reference whose owner is disabled, unsupported or not installed
  resolves to `Resolution::Unavailable { owner }` (status `unavailable`).
  Nothing is deleted or rewritten; enabling the module again makes the same
  references resolve.
- The registry never stores values: every resolution asks the owner's
  source again, so a module value is never cached nor persisted as a user
  value.

### 2.3 Aliases and deprecation

`ALIASES` in the registry maps every legacy id to its canonical id, with the
owning module and the version that deprecated it (0.6.0). A legacy
reference keeps its spelling (the `VariableRef` remembers it, so saving a
profile never rewrites it), keeps its static type for validation, and
resolves exactly as its canonical id. The picker inserts canonical ids and
hides deprecated entries; the catalog lists them flagged `deprecated` so
older apps keep seeing them.

Deprecated ids (kept indefinitely; removal would need an explicit,
reversible migration of stored references, which is out of scope):

- `system.project.current.id`, `.key`, `.name` → `projects.current.…`
- `system.work.session.id`, `.state`, `.startedAt`, `.elapsedSeconds`,
  `.projectId`, `.canFinish` → `workSession.…`

### 2.4 One resolver

`ContextResolver` resolves every root for actions, workflows, key bindings,
parameter overrides, picker previews and runs:

- `steps.` from the run's outputs;
- `shared.` through the injected shared resolver (store snapshot,
  Project → Profile → Global precedence unchanged);
- `system.`, `workflow.` and module roots through the `VariableRegistry`
  with a `VariableEnvironment { context, profile, os, live }`.

The context is the explicit `ResolutionContext`: active profile, project
(and its source), device and workflow run. Live values that are not part of
the context (the work session) come from `LiveState`, which the agent
implements over its session snapshot. Types are preserved: a source that
answers with another type than declared provides nothing, and whole-value
references keep their native type (`bind`), with no implicit string
conversion for typed parameters.

**Device slot.** The device in the context is read only through the small
`ContextDevice` trait (`VariableEnvironment::device`: `device_id()`,
`display_name()`). It is implemented by today's `DeviceId`. The generic
`deckpilot_device::DeviceDescriptor` of the device refactoring (PR #175:
vendor, model, display name, serial, capabilities, driver, verification)
will implement it once it lands: in the core the descriptor comes from
`DeviceContext::current_device()` and the model catalog's `describe`, in the
agent from `Agent::device_descriptor`. No variable source changes.

### 2.5 IPC (protocol 9)

- `CatalogEntry` gains `owner`, `readOnly` and `deprecated`; groups gain
  `module`; `ReferenceStatus` gains `unavailable`; `ReferenceAvailability`
  gains `unavailable`.
- For clients before protocol 9, `Response::for_protocol` removes entries
  with module roots (they cannot parse them), maps `unavailable` statuses to
  `permissionDenied` and `unavailable` availability to `notInContext`.
  Legacy alias entries stay in the list, so an older app sees exactly the
  ids it knew.
- `sharedVariablesChanged` names both the canonical and the legacy ids of
  work session variables that changed; module enable/disable announces the
  module's ids.

### 2.6 Catalog UI and picker

- One **Variables** page replaces the Shared Variables and System Variables
  sections: one table with search; owner, type and scope filters; current
  value (only when not sensitive, redacted otherwise); description; owner;
  read-only/editable; copy reference; create, edit and delete for user
  variables only, under the existing reference-safety rules. No tabs.
- The picker uses the same catalog: name, owner, description, type, current
  value when safe, availability; inserts `{{ canonical.id }}`.
- Implementation: `apps/desktop/src/pages/variables-table.tsx` (the table,
  inside the settings page `pages/shared-variables.tsx`, which keeps the
  shared-variable editor) and `apps/desktop/src/lib/variable-catalog.ts`
  (rows, filters, owners, the app's copy of the aliases for agents before
  protocol 9). Legacy ids are folded into their canonical row ("also
  readable as …"). Sensitive values are never requested for the table.

### 2.7 Live updates

Project, profile and work session changes already reach the agent's
service loop, which re-renders dependent key faces (`after_change`, work
session faces) and emits `projectChanged`, `profileChanged` and
`sharedVariablesChanged`. Pickers and the Variables page refresh previews
on those events; there is no polling.
