# Data model

All entities serialize to JSON with camelCase field names. Entity identifiers
are random UUIDs. Textual identifiers are validated:

| Identifier     | Format                                         | Example              |
| -------------- | ---------------------------------------------- | -------------------- |
| `ActionTypeId` | `<module id>.<name>`, lowercase dotted         | `projects.next`      |
| `ModuleId`     | lowercase letter, then `a-z 0-9 -`             | `projects`           |
| `ResourceKey`  | lowercase dotted                               | `shop.repository`    |
| `ProjectKey`   | 1–32 ASCII letters, digits, `_`, `-`; unique case-insensitively per profile | `SHOP` |
| `DeviceModelId`| free-form                                      | `elgato.streamdeck.mk2` |

## Profile

```
Profile
├── id, name, description
├── actions:   ActionInstance[]
├── workflows: Workflow[]
├── projects:  Project[]        (stored in the `projects` table; order = cycle order)
├── defaultProject             (stored in `default_projects`)
├── layouts:   DeviceLayout[]   (any number per device model)
└── activeLayouts              (the layout used for each model)
```

`Profile::validate` reports duplicate project keys, a default project or
project parameters that do not belong to the profile, workflow steps that
reference missing actions, and layout problems. `Profile::duplicate` copies a
profile with new identifiers for everything in it and remaps all internal
references, so the copy is fully independent.

## Actions

- **ActionType** – defined in code by an `ActionHandler`: id, name,
  description, category, `parameters: ParameterSpec[]`, `requiresProject`.
- **ParameterSpec** – `key`, `label`, `kind`, `required`, optional `default`.
- **ParameterKind** – `text` (multiline, maxLength), `integer` (min, max),
  `boolean`, `choice` (options), `resource` (optional `ResourceKind`),
  `project` (a project of the action's own profile, stored by id).
- **ParameterValue** – tagged: `{ "type": "text", "value": "…" }`.
  Resource values hold a `ResourceRef`:
  - `{ "type": "fixed", "key": "shop.repository" }`
  - `{ "type": "projectRole", "role": "repository" }` – resolved from the
    active project
  Project values hold a project id: `{ "type": "project", "value": "<uuid>" }`.
- **ActionInstance** – `id`, `name`, `actionType`, `parameters`, optional
  `description`, `icon` (icon name) and `label` (key text). Created and edited
  through `ActionRegistry::validate_instance`, which validates parameters and
  fills defaults. The same action type can be configured many times with
  different parameters, each with its own stable identifier.
- **Usage** – where an action or workflow is referenced (workflow step or
  layout binding). Referenced actions and workflows cannot be deleted.

### Key labels

Action labels and binding labels may contain `{profile}`, `{project.key}` and
`{project.name}`. They are resolved against the active project whenever a key
is drawn, so a project switcher labelled `{project.key}` shows SHOP, CMR,
CMS, SHOP as it is pressed.

### Action types of the built-in modules

Action types are provided by [feature modules](modules.md#action-catalog).
A new installation has no actions; the Action Library only contains actions
the user created. `ActionResult` carries an optional `data` map with
structured details (for example the resolved project and resource).

## Workflows

`Workflow { id, name, description, steps: WorkflowStep[] }` with
`WorkflowStep { action, onFailure: "stop" | "continue", condition, overrides }`.

- `condition`: `always`, `previousSucceeded`, `previousFailed` or
  `activeProject { project }`. Steps whose condition is not met are reported
  as `skipped` and do not count as failures.
- `overrides`: parameter values that replace the action's own values for this
  step only.

Steps run in order in the agent; context effects of a step (such as a project
switch) apply before the next step. Requests from elsewhere are processed only
after the workflow finishes, so a workflow always sees a consistent context.
The result is a `WorkflowReport` with a `StepReport` per step.

## Execution

- **ExecutionContext** – profile, active project (optional), machine
  configuration.
- **ActionResult** – `status` (`succeeded` / `failed`), `message`, `effects`.
- **ContextEffect** – `cycleProject { direction }`, `selectProject { project }`.

## Projects

- **Project** – `id`, `key`, `name`, `resources: { role → ResourceKey }`.
- **ProjectSwitcher** – cyclic selection with wrap-around in both directions.
- **ActiveProjectContext** – bound to exactly one profile; using it with
  another profile is an error. Stale selections fall back to the first project.

## Machine resources

- **MachineConfig** – `os`, `bindings: MachineResourceBinding[]`.
- **MachineResourceBinding** – `key`, `kind`, `value`.
- **ResourceKind** – `directory`, `application`, `terminal`,
  `dockerEnvironment`, `service`, `url`, `other`.

Resolution errors: no active project, role not defined by the project, key not
bound on this machine, kind mismatch.

## Devices and layouts

- **DeviceModel** – `id`, `vendor`, `name`, `usb`, `capabilities`, `support`
  (`descriptorOnly`: no driver; `unverified`: driver not tested on hardware;
  `verified`).
- **DeviceCapabilities** – optional `keys` (rows, columns, image spec),
  `encoders` (count, pressable), `touch` (zones, image spec, gestures).
- **ControlId** – `{ "kind": "key", "index": 0 }`, `encoder`, `touchZone`.
- **DeviceLayout** – `id`, `name`, `model`, `homePage`, `pages`.
- **LayoutPage** – `id`, `name`, `bindings: ControlBinding[]`.
- **ControlBinding** – `control`, `trigger` (`press`, `release`,
  `rotateClockwise`, `rotateCounterClockwise`, `touch { gesture }`), `target`
  (`action`, `workflow`, `openPage`, `back`), optional `label`.

Folders are pages opened with `openPage` and left with `back`. Layout
validation checks that controls and triggers exist on the model, that each
control/trigger pair is bound at most once per page, and that targets exist.

A profile uses its first layout for a model. `DeviceLayout::generate` creates
one from the profile's actions and workflows using only the model's
capabilities: items fill the keys in order; when they do not fit, the last key
opens a next page (“More”) and the first key of following pages goes back.

- **LayoutStatus** – per device model: `noProfile`, `unknownModel`, `missing`,
  `compatible { layout, name }`, `invalid { layout, name, issues }`.
- **KeyFace** – what a key shows: `style` (`empty`, `action`, `workflow`,
  `folder`, `back`, `notice`), `title`, optional `detail`, `highlighted`. A
  project switcher shows the active project as detail; a *Select project*
  key is highlighted while its project is active. Without a profile or layout
  the first key shows a notice and the others stay blank.

## Storage schema (version 4)

Migrations run in a transaction each; a failed migration leaves the database
unchanged.

- **Migration 2** adds `app_launches` (start history per component, used to
  detect the first start after an update) and rewrites stored documents in
  document format 2, which only adds optional fields.
- **Migration 3** moves projects out of the profile documents into their own
  tables, stores module state and renames action types to module namespaces.
  See [Migration 3](#migration-3) below.
- **Migration 4** converts the projects actions to the generic catalog. See
  [Migration 4](#migration-4).

| Table              | Purpose                                              |
| ------------------ | ---------------------------------------------------- |
| `profiles`         | One row per profile; JSON document (format 3) + version |
| `projects`         | Projects: `profile_id` (cascade), `key`, `name`, `position`, `resources` |
| `default_projects` | Default project per profile                          |
| `active_projects`  | Active project per profile (runtime state)           |
| `machine_config`   | Single row; JSON document + document version          |
| `modules`          | Enabled flag and configuration per feature module     |
| `settings`         | Key/value: active profile, device brightness, update preferences |
| `app_launches`     | Recent start versions per component                   |
| `migration_notes`  | Changes a migration made to user data, shown in Settings |

Constraints keep projects inside their profile:

- `projects.id` is the primary key, so a project id exists once across all
  profiles; `(profile_id, key)` is unique, ignoring case.
- `default_projects` and `active_projects` reference
  `projects (profile_id, id)` with a composite foreign key. A profile's default
  or active project can therefore only be one of its own projects, and both
  disappear with the project.
- Saving a profile whose project id belongs to another profile fails
  (`ProjectOfAnotherProfile`) and writes nothing.

### Migration 3

Profiles are processed in id order; every decision depends only on the stored
data, so running the migration on the same data gives the same result.

- Each project stays in the profile whose document contained it. Projects are
  never assigned to another profile.
- If an earlier profile already uses a project id (possible when profile
  documents were copied by hand), the project gets a new id derived from the
  profile id and the old id (UUID v5), and every reference inside that
  profile's document follows. Other profiles are not touched.
- The same id twice inside one profile: the first project keeps it and all
  references; the later one gets a derived id.
- A key used twice inside one profile: the later project gets a `-2`, `-3`, …
  suffix.
- The default and active project are kept only if they belong to the same
  profile; otherwise they are cleared.
- `deckpilot.core.log` → `general.log`; `deckpilot.project.cycle` →
  `projects.next` or `projects.previous` (by its direction);
  `deckpilot.project.select` → `projects.select` with the project referenced by
  id instead of key; `deckpilot.resource.resolve` →
  `projects.resolve-resource`. Step overrides follow. A select action whose key
  matches no project of its profile keeps no project, so the user picks one.

Every change beyond a plain rename is recorded in `migration_notes`. Tests
cover a version 1 database, a version 2 database with every legacy action
type, and ambiguous data (shared ids, duplicate keys, a foreign active
project) including that a second run produces the same ids.

### Migration 4

Only documents change (document format 4); action ids, names, workflows and
layout bindings stay as they are.

- `projects.select`: `project` → `project_id`, also in step overrides. A
  select action without a project (its legacy key matched nothing) now
  reports the current project instead of failing; a note asks the user to
  review it.
- `projects.resolve-resource`: the `resource` parameter becomes a resource
  name. "Role X of the active project" becomes `resource = X`. "Fixed key K"
  becomes `project_id = P, resource = X` when exactly one project of the same
  profile maps X to K; otherwise the resource is left empty (the editor asks
  for it) and a note names the old key. Step overrides follow the same rules.

A test migrates every case and checks that ids, order and layout bindings are
unchanged.
