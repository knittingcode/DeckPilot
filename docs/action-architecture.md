# Action architecture: action types, instances, shared variables

Status: **design and contract**. The types in
`crates/deckpilot-core/src/variables/` and their mirror at the end of
`apps/desktop/src/lib/types.ts` exist and are tested; nothing uses them yet.
Behaviour is unchanged until the workstreams below land.

This document is the shared reference for the work that follows: what exists
today (audit), the target model, the contracts every workstream builds on,
and who owns which files.

## 1. Audit of the current code (0.6.0)

| Area | Where | What exists |
|---|---|---|
| Action types | `crates/deckpilot-core/src/action.rs`, `registry.rs` | `ActionType` (id `<module>.<name>`, `ParameterSpec`s, `requires_project`, `NodeSpec`). `ParameterKind`: text, integer, boolean, choice, resource, project, lines, pairs, code, credential. `ParameterValue` is literal only (text, integer, boolean, choice, resource, project). `ActionType::normalize` validates and fills defaults. Handlers implement `ActionHandler` (`execute`, `check`, `decorate_face`, `state_policy`, `button_state`). |
| Action instances | `ActionInstance` in `action.rs`, stored inside the profile JSON document (`profiles.document`) | id, name, `action_type`, `parameters`, description, icon, label. No provenance or origin field. Created by `ActionRegistry::create_instance` and the Tauri commands in `apps/desktop/src-tauri/src/commands/actions.rs`. |
| Demo data | `tools/dev/src/seed.rs`, `seed/examples.rs` | The **only** place that creates example projects, actions, workflows and layouts, into the dev data directory (`.dev/data`). Verified: no production path (agent, app, storage, portability) creates profiles or actions; `create_profile` saves `Profile::new` (empty); git history shows no production seeding in any release. **A fresh production database is empty**, as `docs/development.md` says. Seeded items carry no marker: they cannot be told apart from user items except by content, and ids are random per seed. The upgrade fixture `crates/deckpilot-storage/tests/fixtures/v0.5.0-2` was created by the dev tools and so contains seeded items without provenance. |
| Profile initialization | `Profile::new` in `profile.rs` | No actions, workflows, projects or layouts. |
| Projects actions | `modules/projects/src/actions.rs`, `switching.rs` | `projects.next`, `projects.previous` (step with the module's `wrapAround` setting), `projects.select` (`project_id` parameter, literal `ProjectId`; without it reports the current project), `projects.active`, `projects.repository`, `projects.documentation`, `projects.resolve-resource`. There is **no `projects.cycle`** and no `projects.current` (the read is `projects.active`). Switching is requested with `ContextEffect::SelectProject`, applied by the runtime (`runtime.rs`) after a capability check (`ProjectContext`). Cross-profile selection is already refused (`check_project`, `ActiveProjectContext::select`). |
| Active project | `ActiveProjectContext` (`project.rs`), agent `service.rs` | One per profile, persisted in `active_projects`. The agent handles `SelectProject`/`CycleProject` requests and device input on one service loop (`&mut self`), so switches are serialized; `after_change` re-renders keys and emits `AgentEvent::State`. |
| Key bindings | `layout.rs` | `ControlBinding { control, trigger, target, label, appearance }`; `BindingTarget` = `Action { action }` \| `Workflow { workflow }` \| `OpenPage` \| `Back`. Bindings reference saved instances only; `BindingTarget` is `Copy`. Key labels use a separate presentation template (`{profile}`, `{project.key}`, `{project.name}`). |
| Workflow graphs | `crates/deckpilot-core/src/graph/` | `NodeKind::Action { action: ActionInstanceId, overrides, mappings, policy }` — a node must point at a saved instance. `Mapping` = `Output { reference, default }` \| `Template { template }`. `GRAPH_VERSION = 1`. |
| Mapping syntax | `graph/mapping.rs` | **The only expression syntax:** `{{ steps.<node>.output.<key> }}`, plain substitution, no evaluation, values never re-expanded. Values are text and are converted to the parameter kind in `executor.rs::mapped_parameters` (resource parameters cannot be mapped). |
| Workflow variables (#43) | `graph/mod.rs` `WorkflowVariable`, `NodeKind::SetVariable` | Per run only, read as `steps.vars.output.<key>` (`VARIABLES_NODE`), typed by `ValueKind` (text, integer, boolean, path, url, projectKey), `sensitive` flag, flow along edges, conflicts at joins fail. Workflow inputs/outputs similar. |
| Run project context | `graph/executor.rs` | Each run captures the active project and carries its own context along edges; `Workflow::project_scope` (`Execution` default \| `Application`) decides whether the final context becomes the application's active project. |
| Resource resolution | `resource.rs`, `ExecutionContext::resolve` | `ResourceRef::Fixed { key }` \| `ProjectRole { role }` resolved against `MachineConfig` bindings for the context's project. |
| Module execution context | `execution.rs` | `ExecutionContext` holds `&Profile`, `Option<&Project>`, `&MachineConfig`, capabilities, optional `ExecutionControl` (cancellation, progress, run summary), `&dyn CredentialSource`, `&dyn Notifier`. Already constructor-injected (no service locator); access is gated by module capabilities. There is no variable resolver. |
| Storage | `crates/deckpilot-storage/src/sqlite.rs` | **Schema version 7** (`MIGRATIONS.len()`), `DOCUMENT_VERSION = 4`. Migrations run in one transaction after an **encrypted, verified backup** in `backups/`, recorded in `migration_history`, with a post-commit check. Older binaries refuse newer schemas ("schema version N is newer than supported"). Module-owned tables via `module_schemas`. |
| IPC | `crates/deckpilot-ipc/src/protocol.rs` | **`PROTOCOL_VERSION = 6`**, `MIN_PROTOCOL_VERSION = 1`, `negotiate = min(client, agent)`, `Capability` list derived from the protocol, `Request::since_protocol`. Events: `State`, `ActionCompleted`, `ExecutionStarted/Progress/Finished`, `Notification`, … |
| Frontend | `apps/desktop/src` | Action Library `pages/actions.tsx` + `components/action-editor.tsx`; one schema-driven `components/parameter-field.tsx` (+ `structured-fields.tsx`) used by the action editor, `pages/workflow-steps.tsx`, the builder inspector and module settings. Mapping UI: `workflow-builder/mapping.ts`, `mapping-editor.tsx` (`OutputSelect`, `MappingEditor`), `variables-editor.tsx`, `triggers-editor.tsx`. Layout designer `pages/layout-designer.tsx`. A contract test (`crates/deckpilot-modules/tests/contracts.rs`) checks that `types.ts` can draw every `ParameterKind`. |

Consequences for the design:

1. The expression language is extended, not replaced: new roots inside the
   same `{{ … }}` grammar.
2. Seeded-action cleanup cannot be reliable for existing data (no marker).
   Provenance is recorded for newly seeded data only; legacy items are left
   alone (decision D2).
3. Instance parameters are literal today. References in instances need an
   explicit new parameter value variant; existing text containing `{{` must
   keep meaning literal text.

## 2. Concepts

- **Action type** — provided by a module: id, parameter schema, node spec,
  handler. Unchanged.
- **Action instance** — a saved, named configuration of a type in the
  Action Library of a profile. Parameters may now be literals *or*
  references.
- **Inline invocation** — a type plus parameters stored directly in a
  workflow node or a key binding, without a library entry.
- **Binding** — what connects an instance or an inline invocation to a key,
  a workflow node or (through a workflow) a trigger.
- **Shared variable** — a typed, named value (`shared.<namespace>.<name>`)
  with values at one or more scopes.
- **System variable** — read-only, taken from the resolution context
  (`system.…`, `workflow.…`).
- **Reference** — `{{ <root>.<path> }}` in a parameter, mapping or
  template; resolved at execution time, never stored as its value.

## 3. Data model

### 3.1 Shared variables (schema 8, owned by E)

```sql
CREATE TABLE shared_variables (
    key          TEXT PRIMARY KEY NOT NULL,      -- 'projects.favorite'
    label        TEXT NOT NULL DEFAULT '',
    description  TEXT NOT NULL DEFAULT '',
    value_type   TEXT NOT NULL,                  -- JSON VariableType
    scopes       TEXT NOT NULL,                  -- JSON array of VariableScope
    default_value TEXT,                          -- JSON VariableValue
    sensitive    INTEGER NOT NULL DEFAULT 0,
    module_id    TEXT,                           -- NULL: user variable
    origin       TEXT NOT NULL DEFAULT 'user',   -- see 3.3
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE TABLE shared_variable_values (
    key        TEXT NOT NULL REFERENCES shared_variables (key) ON DELETE CASCADE,
    scope      TEXT NOT NULL CHECK (scope IN ('global', 'profile', 'project')),
    profile_id TEXT REFERENCES profiles (id) ON DELETE CASCADE,
    project_id TEXT REFERENCES projects (id) ON DELETE CASCADE,
    value      TEXT NOT NULL,                    -- JSON VariableValue
    updated_at INTEGER NOT NULL,
    CHECK ((scope = 'global'  AND profile_id IS NULL     AND project_id IS NULL)
        OR (scope = 'profile' AND profile_id IS NOT NULL AND project_id IS NULL)
        OR (scope = 'project' AND profile_id IS NOT NULL AND project_id IS NOT NULL))
);
CREATE UNIQUE INDEX shared_variable_values_slot
    ON shared_variable_values (key, scope, IFNULL(profile_id, ''), IFNULL(project_id, ''));
```

- Definitions are database-wide; values are per scope key. Execution values
  are never stored.
- A project value must name a project of `profile_id` (checked on write,
  like `default_projects`).
- `secretRef` values hold a credential *name*; secrets stay in the
  credential store.
- Writes go through `VariableDefinition::check_value` (type and allowed
  scope).

### 3.2 Parameter values and invocations (document changes, owned by A and C)

Added to `ParameterValue` (A):

```jsonc
{ "type": "reference", "value": "shared.projects.favorite" } // whole value, native type
{ "type": "template",  "value": "cd {{ shared.paths.root }}" } // text kinds only
```

Existing `text` values are never parsed for `{{`; only `template` values
are. `ActionType::normalize` accepts a reference for any kind whose type can
be assigned (`VariableType::assignable_to`) and defers the value check to
resolution.

Added to the graph (C):

```jsonc
// NodeKind::Invoke: an inline action, no saved instance
{ "id": "open", "type": "invoke", "actionType": "projects.select",
  "parameters": { ... }, "mappings": { ... }, "policy": { ... } }
// Mapping::Variable: whole-value reference with any root
{ "type": "variable", "reference": "shared.projects.favorite", "default": {...} }
```

`Mapping::Template` keeps its shape; its text is parsed with the extended
grammar. `Mapping::Output` stays for `steps.` references (no rewrite of
stored graphs). A graph that contains an `invoke` node or a `variable`
mapping is saved with `GRAPH_VERSION = 2`; version 1 graphs load unchanged.

Added to bindings (C): `BindingTarget::Invoke { actionType, parameters }`.
`BindingTarget` stops being `Copy`.

Both bump `DOCUMENT_VERSION` to 5 (E) so that an export with these shapes
is not imported by an older version.

### 3.3 Provenance (schema 8, owned by E)

```sql
CREATE TABLE provenance (
    profile_id  TEXT NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    entity      TEXT NOT NULL CHECK (entity IN ('action', 'workflow', 'layout', 'project', 'variable')),
    entity_id   TEXT NOT NULL,
    origin      TEXT NOT NULL CHECK (origin IN ('seed', 'import', 'migration')),
    catalog     TEXT,         -- 'dev-seed'
    seed_key    TEXT,         -- stable name in the seed, e.g. 'work.next-project'
    seed_version INTEGER,
    snapshot    TEXT NOT NULL, -- canonical JSON of the entity without its id, as created
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (profile_id, entity, entity_id)
);
```

- No row means *user-created*. Profile documents are not changed.
- **Edited** = the entity's canonical JSON differs from `snapshot`.
- Cleanup removes an entity only if it has `origin = 'seed'`, is unedited,
  and has no usages (`Profile::action_usages`, `workflow_usages`, layout
  bindings, workflow nodes, triggers). Bindings are never deleted by
  cleanup; a binding whose target would be removed keeps the target.
- The dev seed records a row for every item it creates.
- Legacy data (before provenance) is left alone (D2): no automatic cleanup
  and no cleanup tool. Users remove such items themselves through the
  normal safe deletion, which lists every reference first. Production
  databases never received seeded data.

## 4. Shared variable registry

- Key: `namespace.name[.more]`, 2–4 segments, each 1–32 ASCII letters,
  digits and `_`, starting with a lowercase letter (`VariableKey`).
- Types (`VariableType`): `string`, `integer`, `boolean`, `projectId`,
  `profileId`, `resource{kind?}`, `json`, `secretRef`.
- Values (`VariableValue`) are literals of the declared type. They are
  never expanded again, so references cannot form cycles (a value
  `"{{ shared.a.b }}"` is that text).
- `sensitive`: the value is only ever shown as redacted.
- `module`: set for variables a module declares in its manifest (later);
  user variables have none.

### Scopes and precedence

`VariableScope`: `execution` > `project` > `profile` > `global`, then the
definition's default (`select_value`). A value applies only if the
definition allows its scope and its `ScopeKey` matches the context
(`ScopeKey::applies_to`): profile values only for that profile, project
values only for that project of that profile. Values therefore never cross
profiles; a `projectId`/`profileId` value naming another profile resolves to
`missing(otherProfile)`.

Execution values are set during a run and vanish with it. A workflow node
that writes persistent (profile, project or global) values is out of scope
for this effort (D7).

### Reserved namespaces

Reserved are only the roots `steps`, `system` and `workflow`, and the keys
a module declares itself (D4). Users may create variables in a group named
after a module, e.g. `shared.projects.favorite`, as long as the key is not
one the module declares.

The contract code currently reserves more namespaces
(`RESERVED_NAMESPACES` also lists `shared`, `vars`, `env`, `secret`,
`secrets`, `deckpilot`, `internal`). Workstream A narrows the list to
`steps`, `system`, `workflow` and adds the check against module-declared
keys when module variables exist.

## 5. Expression syntax

One grammar: `{{ <root>.<path> }}`, plain substitution, closed set of roots
(`VariableRef::parse`):

| Root | Form | Notes |
|---|---|---|
| `steps` | `steps.<node>.output.<key>` | unchanged; parsed by `graph::Reference::parse`; `steps.vars.output.<key>` stays the workflow-variable form |
| `shared` | `shared.<namespace>.<name>` | shared variables |
| `system` | `system.project.current.id` / `.key` / `.name`, `system.profile.current.id` / `.name`, `system.device.current.id`, `system.platform` | read-only |
| `workflow` | `workflow.id`, `workflow.execution.id` | read-only, runtime |

A whole-value reference (`ParameterValue::Reference`, `Mapping::Variable`)
keeps the native type (`VariableValue::to_parameter`). A template renders
each reference as text (`render_text`); a `secretRef` or `resource` in a
template is a type error. Key labels accept the same references
(`{{ system.project.current.key }}`); the existing `{profile}`,
`{project.key}` and `{project.name}` placeholders keep working as aliases
(D6).

Every template valid today parses to the same references (tested in
`variables/tests.rs`).

## 6. System variables

Resolved from the explicit `ResolutionContext`, never from global state:

| Reference | Type | From |
|---|---|---|
| `system.project.current.id` / `.key` / `.name` | projectId / string | `context.project` — the run's project inside a run, the active project for a key press |
| `system.profile.current.id` / `.name` | profileId / string | `context.profile` |
| `system.device.current.id` | string (`model#serial`) | the device whose control started it |
| `system.platform` | string (`linux`, `macOs`, `windows`, `other`) | this computer |
| `workflow.id` | string | the running workflow |
| `workflow.execution.id` | string | the run; runtime only |

In a preview with `projectSource = execution`, project variables are
`unavailableUntilRuntime`: the active project is never substituted for a
run's project.

## 7. Resolution pipeline

One pipeline for key presses, workflow nodes, triggers, inline invocations
and previews (A owns it, in `crates/deckpilot-core/src/resolution.rs`):

1. **Load definition** — the action type from the registry; unknown or
   unavailable types fail with the existing codes.
2. **Instance or inline** — saved instance parameters, node overrides and
   mappings, or the inline invocation's parameters.
3. **Capture context** — build `ResolutionContext` (profile, project and its
   source, device, workflow, execution, phase).
4. **Resolve variables** — every `reference`/`template` value through the
   injected `VariableResolver`.
5. **Resolve workflow outputs** — `steps.` references from the run's
   outputs (same resolver interface; the executor supplies outputs).
6. **Resolve resources** — `ResourceRef`s through `ResourceResolver` for
   the context's project.
7. **Validate types** — `bind` per reference, then `ActionType::normalize`
   on the result (ranges, lengths, choices).
8. **Permissions** — module capabilities as today; variables a caller may
   not read are `permissionDenied`.
9. **Execute** — handler with an `ExecutionContext` built from
   `ExecutionDependencies`; cancellation, timeouts and retries unchanged
   (each retry re-runs from step 3 only if the node is retry-safe, using
   the same captured context).
10. **Typed outputs** — `ActionResult::data` plus declared `OutputSpec`
    kinds.
11. **Redact** — sensitive outputs, sensitive variables and credentials are
    removed from messages, events and history (`safe_report`, existing
    `sensitive` flags, plus sensitive shared variables).
12. **Record** — execution history as today.

Errors are `ReferenceError`: they name the parameter and the reference and
never contain a value, e.g. ``parameter `project_id`: {{ system.project.current.id }} has no value (no project is active)``
(codes `reference-missing`, `reference-type`, `reference-denied`,
`reference-unavailable`).

## 8. Dependency injection

`ExecutionDependencies<'a>` bundles `&dyn` services handed to an execution
explicitly; there is no global mutable registry:

| Service | Trait | Production impl (owner) | Test double |
|---|---|---|---|
| Variables | `VariableResolver` | agent resolver over the store and runtime (A) | `doubles::MemoryVariables` |
| Active project | `ProjectResolver` | agent runtime (A) | `doubles::FixedProjects` |
| Resources | `ResourceResolver` | `MachineConfig` (done) | `MachineConfig` |
| Credential references | existing `CredentialSource` | keystore (done) | `MemoryCredentials` |
| Device | `DeviceContext` | agent device runtime (A) | `doubles::FixedDevice` |
| Cancellation, progress | existing `ExecutionControl` | engine | `CancellationToken` |
| Permissions | existing capabilities on `ExecutionContext` | registry | `restricted(..)` |

A adds `ExecutionContext::with_dependencies` and accessors
(`variable(&VariableRef)` gated by a new `Capability::VariablesRead`,
`device()`), keeping every existing constructor working.

## 9. Projects module (B)

- `projects.select`: parameter `project_id` accepts a literal, a
  `reference` (typed `projectId` shared variable, `system.…`) or a mapped
  workflow output. Scope keeps today's behaviour (D1): a key press changes
  the global active project; inside a workflow the workflow's own
  `project_scope` decides. No new required field. Cross-profile targets
  fail (`project-other-profile`).
- `projects.cycle`: one type with `direction` (`next` \| `previous`), wraps
  per module setting; 0 projects → `no-projects`; 1 project → success
  without change; active project deleted → starts from the first/last;
  reorder → follows current order. To be safe under concurrent presses the
  handler returns a new `ContextEffect::CycleProject { direction }` that the
  runtime applies against the *current* active project at apply time
  (not a precomputed target). A profile change between press and apply
  drops the effect. Device faces update through the existing
  `after_change`; the agent emits `projectChanged` (§10).
  `projects.next`/`previous` are hidden from the creation catalog (D3);
  existing instances keep working.
- `projects.current`: alias of `projects.active` semantics with typed
  outputs (`id`, `key`, `name`); it replaces `projects.active`, which is
  hidden from the creation catalog while existing instances keep working
  (D3).
- `projects.resolve-resource`: unchanged contract, resolving through
  `ResourceResolver`; project parameter may be a reference.

## 10. IPC and Tauri commands

Protocol **7**, `Capability::Variables` (since 7). Requests (A):

```jsonc
{ "type": "listVariables", "context": ResolutionContext, "filter": CatalogFilter }
  → { "type": "variables", "entries": CatalogEntry[] }          // metadata only
{ "type": "previewReferences", "context": ResolutionContext, "items": PreviewItem[] }
  → { "type": "referencePreviews", "previews": ReferencePreview[] }
```

Events (A emits; agents on protocol 7 only):

```jsonc
{ "type": "projectChanged", "profile": id, "project": id|null, "source": ProjectSource }
{ "type": "profileChanged", "profile": id }
{ "type": "sharedVariablesChanged", "keys": string[] }           // no values
{ "type": "resourceBindingsChanged" }
{ "type": "deviceContextChanged", "device": DeviceId|null }
{ "type": "executionContextChanged", "execution": id, "project": id|null }
```

Older apps never receive them (the agent checks the negotiated protocol,
like `Notification`). A new `ContextEffect` variant is stripped from
`ActionResult`s sent to clients below 7.

Tauri commands (D): `list_shared_variables`, `save_shared_variable`,
`delete_shared_variable`, `set_shared_variable_value`,
`clear_shared_variable_value` (write the database, then `notify_reload`; the
agent emits `sharedVariablesChanged`), and `list_variables`,
`preview_references` (forwarded to the agent; with an older agent they
return "not supported" and the picker shows metadata from the local
definitions only, without values).

Step outputs of a graph being edited are listed by the frontend from the
builder's graph (`workflow-builder/mapping.ts::outputOptions`, metadata
only); their previews are always `unavailableUntilRuntime`.

## 11. Universal variable picker and live value inspector (F)

One component, `apps/desktop/src/components/variable-picker/`, used by
every schema-driven editor: `parameter-field.tsx` (Action Library, workflow
steps, builder inspector, layout designer inline actions, module settings),
`mapping-editor.tsx`, `triggers-editor.tsx` and the Shared Variables page.
No other implementation.

- Search (debounced) over reference, label, description, scope, type and
  module; groups Shared / System / Profile / Project / Workflow / Step
  outputs; filter by the field's `ParameterKind` (`compatibleWith`); keyboard
  navigation; accessible listbox semantics.
- Opens from a button beside compatible fields and on typing `{{`;
  inserts `{{ reference }}` at the cursor (template) or sets a whole-value
  reference.
- Inspector per entry: full reference, type, scope, description, current
  value, source, dynamic/static, availability — values only from
  `previewReferences` in an explicit context `{profile, project, device?,
  workflow?, execution?}` that the picker shows. Obsolete preview requests
  are dropped by a generation counter.
- Statuses shown: resolved, unavailable until run time, missing (reason),
  type mismatch, permission denied, redacted. Sensitive entries: metadata
  and "redacted" only — never in results, tooltips, logs, diagnostics,
  history or exports.
- Refreshes on the events in §10; no polling.
- Editors store the reference, never a previewed value.

## 12. Migration plan

Schema 7 → 8 (E), one migration step, additive:

1. `shared_variables`, `shared_variable_values`, `provenance` tables.
2. No rewrite of profile documents. `DOCUMENT_VERSION` 4 → 5 only marks
   documents that may contain the new shapes; version 4 documents decode
   unchanged.
3. Runs under the existing machinery: encrypted verified backup in
   `backups/`, one transaction, `migration_history`, post-commit check.
4. Tests: upgrade of `tests/fixtures/v0.5.0-2` and of a new 0.6.0 fixture
   (schema 7) — every row kept, seeded items without provenance untouched,
   reopening changes nothing, killed and failed upgrades roll back.

## 13. Compatibility and rollback limits

- A 0.6 agent or app refuses a schema 8 database (existing check). Going
  back means restoring the pre-upgrade backup; changes made after the
  upgrade are lost. Document this in the release notes.
- Mixed app/agent versions: negotiated protocol; without `Capability::
  Variables` the app hides live values and refuses to save references
  (the older agent could not resolve them).
- Exports with document version 5 are refused by older importers.
- Version 1 graphs, literal parameters, `steps.` templates, workflow
  variables and bindings to saved instances behave exactly as before.

## 14. Work split

| WS | Scope | Owns (files) | Blocked by |
|---|---|---|---|
| **0** | this contract | `crates/deckpilot-core/src/variables/**`, the types section at the end of `apps/desktop/src/lib/types.ts`, this document | — |
| **A** core | pipeline, `ParameterValue::{Reference,Template}`, `ExecutionDependencies` in `ExecutionContext`, `Capability::VariablesRead`, agent resolver, IPC protocol 7 (requests, events, negotiation), agent handlers | `crates/deckpilot-core/src/{resolution.rs (new), action.rs, execution.rs (except `ContextEffect`), registry.rs, runtime.rs, module.rs (capability)}`, `crates/deckpilot-ipc/src/protocol.rs`, `apps/agent/src/service/variables.rs` (new) and the request/event arms it needs in `apps/agent/src/service.rs` | 0. Real data needs E's store API (uses `MemoryVariables` until then) |
| **B** projects | `projects.select` scope/references, `projects.cycle`, `projects.current`, `ContextEffect::CycleProject`, device state and `projectChanged` | `modules/projects/**`, `crates/deckpilot-core/src/project.rs`, the `ContextEffect` enum in `execution.rs`, effect application in `runtime.rs` (`execute`, coordinate with A), `apps/agent/src/service.rs::cycle_project`/`after_change`, `apps/desktop/src-tauri/src/modules/projects.rs`, `apps/desktop/src/modules/projects/**` | 0. Cycle: nothing. Select with references: A (pipeline) |
| **C** workflows | extended grammar in templates, `Mapping::Variable`, `NodeKind::Invoke`, `BindingTarget::Invoke`, `GRAPH_VERSION` 2, executor through the pipeline, validation, builder UI | `crates/deckpilot-core/src/graph/**`, `crates/deckpilot-core/src/layout.rs`, `device_runtime.rs` dispatch of inline bindings, `apps/desktop/src/workflow-builder/**`, `docs/workflow-graphs.md` | 0. Model/validation: nothing. Executor: A. UI picker: F |
| **D** library UI | Action Library (create/edit with references), inline action config in the layout designer, Shared Variables page, Tauri commands | `apps/desktop/src/pages/{actions.tsx, layout-designer.tsx, shared-variables.tsx (new)}`, `components/action-editor.tsx`, `apps/desktop/src/lib/api.ts`, `apps/desktop/src-tauri/src/commands/{variables.rs (new), actions.rs, mod.rs}`, `src-tauri/src/main.rs` registration, `src-tauri/src/tests.rs` | 0; E for storage calls; F for the picker (stub until then) |
| **E** storage | schema 8, store API for variables (`SharedVariables` reads/writes), provenance, seed provenance, dev-only legacy cleanup, `DOCUMENT_VERSION` 5, portability, fixtures | `crates/deckpilot-storage/src/{sqlite.rs, variables.rs (new), provenance.rs (new), lib.rs}`, `crates/deckpilot-storage/tests/**`, `crates/deckpilot-portability/**`, `tools/dev/src/seed*.rs`, `docs/data-model.md` | 0 |
| **F** picker | the universal picker and inspector, live-value hooks, autocomplete, integration into `parameter-field.tsx`, `mapping-editor.tsx`, `triggers-editor.tsx`, module settings | `apps/desktop/src/components/variable-picker/**` (new), `apps/desktop/src/lib/variables.ts` (new), `components/parameter-field.tsx`, `components/structured-fields.tsx`, `workflow-builder/mapping-editor.tsx` (picker insertion only; C owns the rest), `settings/pages/module-settings.tsx` | 0. Live values: A (IPC); can start on a mocked `api.ts` |

`apps/desktop/src/lib/types.ts`: each workstream appends its own section;
changes to the contract section go through a contract PR.

Order: 0 → (A, B-cycle, C-model, E, F-mock in parallel) → A pipeline → B
select, C executor → A IPC → F live → D integration.

## 15. Tests by workstream

| Test | Owner |
|---|---|
| searching, type filtering (catalog filter) | 0 (unit, done), F (UI) |
| autocomplete on `{{`, insertion at the cursor, keyboard navigation, accessibility | F |
| live updates on events, obsolete previews dropped | A (events emitted), F (refresh) |
| runtime-only values (`workflow.execution.id`, step outputs) | 0 (done), A, F (display) |
| context isolation: run project vs active project, cross-profile values | 0 (done), A, B, C |
| missing references (unknown variable, no value, no project, no device) | 0 (done), A |
| circular references: values are never re-expanded; store rejects nothing special | 0 (done), E (store round-trip) |
| redaction: previews, errors, events, history, exports, diagnostics | 0 (preview, errors), A (events, history), C (node reports), E (exports), F (UI) |
| permission failures | 0 (done), A (capability) |
| action execution after a variable change uses the new value (no cached preview) | A + E |
| workflow using shared variables (templates, whole-value mappings, inline nodes) | C |
| `projects.cycle`: 0/1 projects, wrap, deleted active, reorder, concurrent presses, profile change, faces, event | B |
| backward compatibility: legacy templates, v1 graphs, literal parameters, fixtures 0.5.0-2 and 0.6.0, rollback via backup | 0 (grammar, done), C, E |
| seed provenance and cleanup never deleting edited or referenced items | E |

## 16. Decisions

Decided by the user (D1–D4); D5–D7 are coordinator defaults the user can
still override.

- **D1** `projects.select` scope keeps today's behaviour: a key press sets
  the global active project; inside a workflow the workflow's own
  `project_scope` applies. No new required field.
- **D2** Legacy seeded data without provenance is left alone: no automatic
  cleanup and no `--clean-legacy` tool. Provenance is recorded for newly
  seeded data only. Users remove legacy items manually through safe
  deletion with the reference list.
- **D3** `projects.next`, `projects.previous` and `projects.active` are
  hidden from the creation catalog, replaced by `projects.cycle` (direction
  `next`/`previous`) and `projects.current`. Existing instances keep
  working.
- **D4** Users may create shared variables in module-named groups (e.g.
  `shared.projects.favorite`). Reserved are only `system.`, `workflow.`,
  `steps.` and keys a module declares itself.
- **D5** Exports: definitions and profile/project values travel with the
  profile; global values only with the app settings; sensitive values
  never.
- **D6** Key labels support `{{ system.project.current.key }}` (and the
  other references); `{project.key}`, `{project.name}` and `{profile}` stay
  as aliases.
- **D7** A workflow node that writes persistent shared variables is out of
  scope for this effort (future work; needs a permission and a prompt).
