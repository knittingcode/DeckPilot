# Architecture

## Processes

DeckPilot is designed as two cooperating processes:

| Process              | Responsibility                                                  | Status      |
| -------------------- | --------------------------------------------------------------- | ----------- |
| Background agent     | Talks to devices, dispatches input, runs actions and workflows  | Planned     |
| Configuration app    | Edits profiles, projects, actions and machine settings          | Implemented |

The agent must run independently of the configuration app. The app is launched
on demand, and closing its window must never interrupt device handling. Both
processes share the local SQLite database; the agent will be notified of
configuration changes over a local IPC channel (to be designed in the agent
milestone).

Until the agent exists, the configuration app can test-run hardware-independent
actions in its own process.

## Crates

```
deckpilot-device  ◄── deckpilot-core ◄── deckpilot-storage
        ▲                  ▲                    ▲
        └──────────────────┴── deckpilot-desktop (Tauri app)
```

### `deckpilot-device`

Hardware-independent description of control surfaces.

- `DeviceModel`, `DeviceModelId`, `UsbIdentity`, `HardwareSupport`
- `DeviceCapabilities` with optional `KeyGrid`, `EncoderSpec` and `TouchSurface`;
  validates whether a control exists and whether an input can occur on it
- `ControlId` (`Key`, `Encoder`, `TouchZone`) and `ControlInput` (`Press`,
  `Release`, `Rotate`, `Touch`)
- `DeviceEvent` (`Connected`, `Disconnected`, `Input`) and `DeviceId`
  (model + serial)
- `DeviceRegistry` tracks connection state of multiple devices across hotplug
- `ModelCatalog` with the built-in Stream Deck MK.2 descriptor

Nothing in the rest of the system assumes a fixed number of keys. A future HID
backend only needs to produce `DeviceEvent`s and consume image updates.

### `deckpilot-core`

The domain model and execution engine.

- `action`: `ActionType` (schema), `ParameterSpec`/`ParameterKind`,
  `ActionParameters`, `ActionInstance`; parameters are validated and normalized
  against the schema
- `registry`: `ActionHandler` trait and `ActionRegistry`
- `builtin`: hardware-independent built-in actions
- `workflow`: `Workflow`, `WorkflowStep`, `OnFailure`
- `execution`: `ExecutionContext`, `ActionResult`, `ContextEffect`
- `project`: `Project`, `ProjectKey`, `ProjectSwitcher`, `ActiveProjectContext`
- `resource`: `ResourceKey`, `ResourceRef`, `MachineConfig`,
  `MachineResourceBinding`
- `layout`: `DeviceLayout`, `LayoutPage`, `ControlBinding`, `Trigger`,
  `BindingTarget`
- `profile`: `Profile` and consistency validation
- `platform`: `Os`
- `runtime`: `Engine` (runs actions and workflows) and `Runtime` (handles device
  events, page navigation and folders)

Action handlers are pure with respect to the runtime context: they receive a
read-only `ExecutionContext` and return `ContextEffect`s (for example "switch to
the next project"), which the engine applies. This keeps handlers testable and
makes workflow steps observe the effects of earlier steps.

Platform-specific behaviour (launching applications, opening terminals,
controlling Docker) will be added behind explicit interfaces that handlers
receive through the execution context. No such handler exists yet.

### `deckpilot-storage`

- `ConfigStore` trait used by the application
- `SqliteStore` implementation with versioned migrations (`PRAGMA user_version`)
- Profiles and machine configuration are stored as versioned JSON documents
- Optional SQLCipher encryption (`sqlcipher` feature) using `PRAGMA key`;
  DeckPilot does not implement cryptography itself

### `apps/desktop`

Tauri 2 application. The Rust side (`src-tauri`) exposes commands that load
state from storage, apply one change through the domain model and persist the
result. The frontend is React + TypeScript with Tailwind CSS and shadcn/ui
components; `src/lib/types.ts` mirrors the serialized Rust types.

## Configuration separation

| Area                   | Contents                                       | Portable |
| ---------------------- | ---------------------------------------------- | -------- |
| Profiles               | Actions, workflows, projects, layouts          | Yes      |
| Machine configuration  | Resource bindings, OS                          | No       |
| Runtime state          | Active profile, active project per profile     | No       |
| Secrets (planned)      | Credentials needed by actions                  | Never    |

Portable data never contains absolute paths: `ResourceKey` rejects them by
construction.

## Security model (planned)

- The database can be encrypted with SQLCipher. The key will be derived from a
  user password requested during initial setup, imports, exports and on the
  first launch after an update.
- Exports will be encrypted with an established, audited format; no custom
  cryptography.
- Secrets will be stored separately from portable profiles.
