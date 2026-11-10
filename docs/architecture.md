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
   │  deckpilot-hid       │  deckpilot-ipc     │      deckpilot-update
   │      ▲               │   ▲                │             ▲
   └──────┴── deckpilot-agent (binary) ────────┤             │
                          └─── deckpilot-desktop (Tauri app) ┘

tools/dev (deckpilot-dev): development supervisor, seed data, release checks
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

### `deckpilot-update`

Update manifest validation, minisign signature verification, update policy
(modes, check scheduling, idle detection) and agent/app version
compatibility. See [updates.md](updates.md).

### `tools/dev`

`deckpilot-dev`, the development supervisor behind `make up` and `pnpm dev`,
plus demonstration data, version checks, packaging and release verification.
Not shipped. See [development.md](development.md).

### `deckpilot-ipc`

Shared by the agent and the configuration app: data-directory paths, the
per-user IPC endpoint, the client token, the versioned protocol types,
newline-delimited JSON framing and an async client.

### `apps/agent`

The background agent: device sessions and I/O threads, layout rendering,
request handling, IPC server, system tray, autostart and single-instance
lock. It starts **locked**: the configuration app unlocks it with the
database key after the user entered the password (see
[security.md](security.md#trust-boundary-between-the-app-and-the-agent)).

### `deckpilot-core`

The domain model and execution engine.

- `action`: `ActionType` (schema), `ParameterSpec`/`ParameterKind`,
  `ActionParameters`, `ActionInstance`; parameters are validated and normalized
  against the schema
- `registry`: `ActionHandler` trait and `ActionRegistry` (dispatch,
  capability checks)
- `module`: `ModuleManifest`, `FeatureModule`, `ModuleRegistry` (see
  [Feature modules](modules.md))
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
read-only `ExecutionContext` and return `ContextEffect`s (for example "select
project CMR"), which the engine applies after checking the module's
capabilities and that the project belongs to the active profile. This keeps handlers testable and
makes workflow steps observe the effects of earlier steps.

Platform-specific behaviour (launching applications, opening terminals,
controlling Docker) will be added behind explicit interfaces that handlers
receive through the execution context. No such handler exists yet.

### Feature modules (`modules/*`, `deckpilot-modules`)

Action types and feature pages come from modules: `modules/general` and
`modules/projects`. `deckpilot-modules` is the only crate that lists them; the
agent and the app build their registries from it. See
[Feature modules](modules.md).

### `deckpilot-portability`

Encrypted configuration export and import: the archive format
(XChaCha20-Poly1305, Argon2id), the payload, validation of untrusted
imports, the preview and the import plan. See [security.md](security.md).

### `deckpilot-storage`

- `ConfigStore` trait used by the application
- `SqliteStore` implementation with versioned migrations (`PRAGMA user_version`)
- Profiles and machine configuration are stored as versioned JSON documents;
  projects, default and active projects live in tables whose constraints keep
  each project inside its profile
- Module states and migration notes are stored alongside
- SQLCipher encryption by default, with a raw 256-bit key derived from the
  user's password (`vault`: setup, unlock, password change, legacy
  migration, crash recovery); DeckPilot does not implement cryptography
  itself
- Verified encrypted backups before schema upgrades, migration history and
  module-owned schema versions

### `apps/desktop` (configuration studio)

Tauri 2 application. The Rust side (`src-tauri`) exposes commands that load
configuration from storage, apply one change through the domain model,
persist it and notify the agent. Runtime changes and action execution are
forwarded to the agent; a background task subscribes to agent events and
forwards them to the window. The core sections are Dashboard, Action
Library, Workflows, Profiles, Device Models, Layout Designer, Connected
Devices and Settings (appearance, modules, agent, updates); further pages come
from enabled modules (Projects, including this machine's resource mappings).
Profile-scoped pages show the profile they edit and are re-created when
another profile is chosen, so an open editor never writes to a different
profile. Every editor saves through the backend; layouts are
validated against the device model and previewed with the same face logic the
agent uses. The frontend is React + TypeScript with Tailwind CSS and shadcn/ui
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
