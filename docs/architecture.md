# Architecture

## Processes

DeckPilot consists of two cooperating processes:

| Process                               | Responsibility                                                              |
| ------------------------------------- | --------------------------------------------------------------------------- |
| Background agent (`deckpilot-agent`)  | Owns devices, renders layouts, dispatches input, runs actions, system tray |
| Configuration app (`deckpilot-desktop`) | Edits profiles, projects, actions, layouts and machine settings          |

The agent runs independently of the configuration app. The app is launched on
demand, connects to the agent over local IPC, and closing its window never
interrupts device handling. The configuration app never opens HID devices and
never executes actions itself. See [background-agent.md](background-agent.md) for the agent's
architecture, lifecycle and IPC protocol.

Both processes open the same local SQLite database. The configuration app
writes configuration documents and asks the agent to reload them; the agent
writes runtime state (active profile, active project, brightness).

## Crates

```
deckpilot-device ◄── deckpilot-core ◄── deckpilot-storage
   ▲      ▲               ▲   ▲                ▲
   │  deckpilot-hid       │  deckpilot-ipc     │
   │      ▲               │   ▲                │
   └──────┴── deckpilot-agent (binary) ────────┤
                          └─── deckpilot-desktop (Tauri app)
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
- `DeviceBackend` (discover, open) and `DeviceHandle` (read input, set images
  and brightness, flush): the driver interface
- `Frame`: device-independent RGB image; drivers convert it to their format
- `KeyStateTracker`: turns key-state snapshots into press/release events
  without duplicates
- `mock::MockBackend` (feature `mock`): simulated devices for tests

Nothing outside a driver assumes a fixed number of keys.

### `deckpilot-hid`

USB HID driver for the Stream Deck MK.2 using `elgato-streamdeck` and
`hidapi`. Only models with a descriptor are exposed.

### `deckpilot-ipc`

Shared by the agent and the configuration app: data-directory paths, the
per-user IPC endpoint, the client token, the versioned protocol types,
newline-delimited JSON framing and an async client.

### `apps/agent`

The background agent: device sessions and I/O threads, layout rendering,
request handling, IPC server, system tray, autostart and single-instance
lock.

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
- `runtime`: `Engine` (runs actions and workflows)
- `device_runtime`: `Runtime` (active profile and project, connected devices,
  page navigation and folders, input dispatch, layout status) and
  `LayoutStatus`
- `face`: `KeyFace`, the device-independent description of what a key shows

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
configuration from storage, apply one change through the domain model,
persist it and notify the agent. Runtime changes and action execution are
forwarded to the agent; a background task subscribes to agent events and
forwards them to the window. The frontend is React + TypeScript with Tailwind CSS and shadcn/ui
components; `src/lib/types.ts` mirrors the serialized Rust types.

## Configuration separation

| Area                   | Contents                                       | Portable |
| ---------------------- | ---------------------------------------------- | -------- |
| Profiles               | Actions, workflows, projects, layouts          | Yes      |
| Machine configuration  | Resource bindings, OS                          | No       |
| Runtime state          | Active profile, active project per profile, device brightness | No |
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
