# Data model

All entities serialize to JSON with camelCase field names. Entity identifiers
are random UUIDs. Textual identifiers are validated:

| Identifier     | Format                                         | Example              |
| -------------- | ---------------------------------------------- | -------------------- |
| `ActionTypeId` | lowercase dotted (`a-z 0-9 . _ -`)             | `deckpilot.core.log` |
| `ResourceKey`  | lowercase dotted                               | `shop.repository`    |
| `ProjectKey`   | 1–32 ASCII letters, digits, `_`, `-`; unique case-insensitively per profile | `SHOP` |
| `DeviceModelId`| free-form                                      | `elgato.streamdeck.mk2` |

## Profile

```
Profile
├── id, name, description
├── actions:   ActionInstance[]
├── workflows: Workflow[]
├── projects:  Project[]        (order defines the project cycle)
├── defaultProject             (selected when the profile is first activated)
├── layouts:   DeviceLayout[]   (any number per device model)
└── activeLayouts              (the layout used for each model)
```

`Profile::validate` reports duplicate project keys, workflow steps that
reference missing actions, and layout problems. `Profile::duplicate` copies a
profile with new identifiers for everything in it and remaps all internal
references, so the copy is fully independent.

## Actions

- **ActionType** – defined in code by an `ActionHandler`: id, name,
  description, category, `parameters: ParameterSpec[]`, `requiresProject`.
- **ParameterSpec** – `key`, `label`, `kind`, `required`, optional `default`.
- **ParameterKind** – `text` (multiline, maxLength), `integer` (min, max),
  `boolean`, `choice` (options), `resource` (optional `ResourceKind`).
- **ParameterValue** – tagged: `{ "type": "text", "value": "…" }`.
  Resource values hold a `ResourceRef`:
  - `{ "type": "fixed", "key": "shop.repository" }`
  - `{ "type": "projectRole", "role": "repository" }` – resolved from the
    active project
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

### Built-in action types

| Id                           | Purpose                                         |
| ---------------------------- | ----------------------------------------------- |
| `deckpilot.core.log`         | Message with `{profile}`, `{project.key}`, `{project.name}` placeholders |
| `deckpilot.project.cycle`    | Next / previous project                         |
| `deckpilot.project.select`   | Select a project by key                         |
| `deckpilot.resource.resolve` | Show the machine value bound to a resource      |

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

## Storage schema (version 2)

Migration 2 adds `app_launches` (start history per component, used to detect
the first start after an update) and rewrites stored documents in document
format 2 within the same transaction. Format 2 only adds optional fields, so
data written by earlier versions is preserved; a test migrates a database
created by the previous release.

| Table            | Purpose                                              |
| ---------------- | ---------------------------------------------------- |
| `profiles`       | One row per profile; JSON document + document version |
| `machine_config` | Single row; JSON document + document version          |
| `profile_state`  | Active project per profile (cascade-deleted)          |
| `settings`       | Key/value: active profile, device brightness, update preferences |
| `app_launches`   | Recent start versions per component                   |
