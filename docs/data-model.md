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
└── layouts:   DeviceLayout[]
```

`Profile::validate` reports duplicate project keys, workflow steps that
reference missing actions, and layout problems.

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
- **ActionInstance** – `id`, `name`, `actionType`, `parameters`. Created only
  through `ActionRegistry::create_instance`, which validates parameters and
  fills defaults.

### Built-in action types

| Id                           | Purpose                                         |
| ---------------------------- | ----------------------------------------------- |
| `deckpilot.core.log`         | Message with `{profile}`, `{project.key}`, `{project.name}` placeholders |
| `deckpilot.project.cycle`    | Next / previous project                         |
| `deckpilot.project.select`   | Select a project by key                         |
| `deckpilot.resource.resolve` | Show the machine value bound to a resource      |

## Workflows

`Workflow { id, name, steps: WorkflowStep[] }` with
`WorkflowStep { action: ActionInstanceId, onFailure: "stop" | "continue" }`.
Steps run in order; context effects of a step are applied before the next
step. The result is a `WorkflowReport` with a `StepReport` per executed step.

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

## Storage schema (version 1)

| Table            | Purpose                                              |
| ---------------- | ---------------------------------------------------- |
| `profiles`       | One row per profile; JSON document + document version |
| `machine_config` | Single row; JSON document + document version          |
| `profile_state`  | Active project per profile (cascade-deleted)          |
| `settings`       | Key/value: active profile, device brightness          |
