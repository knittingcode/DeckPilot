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
lock. It is the only process that reads the operating system's credential
store: with the default system-managed key it opens the configuration at
login by itself (and creates it on first start); with password protection it
starts **locked** until the configuration app hands it the key derived from
the password. It also enforces automatic locking (see
[security.md](security.md#trust-boundary-between-the-app-and-the-agent)).

### `deckpilot-keystore`

Storage of database keys in the platform credential store (macOS Keychain,
Windows Credential Manager, Secret Service) behind a small `KeyStore` trait,
an in-memory store for tests, separate service names for development builds,
and no file fallback.

### `deckpilot-presence`

Operating-system user verification for the configuration app: Touch ID or
the login password through `LocalAuthentication` on macOS; reports no method
elsewhere. Its Objective-C calls are the only `unsafe` code in the workspace,
kept in one module.

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
- SQLCipher encryption always, with a raw 256-bit key that is either random
  and kept in the credential store or derived from the user's password
  (`vault`: setup, unlock, switching between the two, password change,
  legacy migration, crash recovery); security policies (`policy`);
  DeckPilot does not implement cryptography itself
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

Native windows besides the main window come from one catalog
(`src-tauri/src/windows.rs`) and are opened by identifier only — by the app,
or by the agent over IPC: the prompts that open the configuration
(`security.*`), export and import (`portability.*`), update progress
(`updates.progress`), the agent log (`app.logs`), the workflow summary, and
windows declared by enabled modules. Every window renders through the shared
`ModalShell` and is sized to its content by the window manager (no
per-window dimensions); the catalog decides whether a window may open while
the configuration is locked. Only the export and import windows may use file
dialogs.

### Main window shell

The main window has one shared titlebar (`AppTitlebar`), the workspace or
settings sidebar, and the page. Pages render only their content; their title,
subtitle, breadcrumbs, status and commands go to the titlebar through
`useTitlebar` (`lib/titlebar.ts`). Any component of a page may contribute —
including module pages — and the titlebar imports no page or module. Actions
have a placement: *primary* (always visible), *secondary* (visible while
there is room, otherwise in the overflow menu) or *overflow*.

Platform differences live in `use-window-chrome.ts`:

- **macOS:** the window uses the overlay titlebar (`titleBarStyle: Overlay`,
  `hiddenTitle`, `trafficLightPosition` in `tauri.conf.json`). The native
  traffic lights stay, with a safe area that disappears in fullscreen.
  Dragging empty titlebar space moves the window (`startDragging`, the only
  added window permission). A double-click follows the system setting "Double-click a
  window's title bar to" (zoom, minimize or nothing) via
  `titlebar_double_click`.
- **Windows and Linux:** the native titlebar and window manager decorations
  stay unchanged; the app titlebar is a toolbar below them.

Sizes are tokens in `index.css` (`--titlebar-height`, `--titlebar-control`,
`--titlebar-icon`, …). The navigation region (traffic-light safe area,
sidebar toggle, back, forward) has a fixed width, so it never moves when the
sidebar collapses; a separate element continues the expanded sidebar's
background and divider. Native dialog windows (`ModalShell`) have no app
titlebar.

### Status bar

A compact bar at the bottom of the main window (`AppStatusBar`), in workspace
and settings mode, never in dialog windows. It renders three shared stores
(`lib/status.ts`) and imports no module:

- **Status items**, registered with `useStatusItem` by core services
  (agent, devices, active profile, updates) and by enabled modules through
  `FrontendModule.statusItems` (Projects: the active project). Module
  components mount only while the module is enabled, so disabling it removes
  its items without touching its configuration. Each item has a namespaced
  id, owner, placement (left, center, right), priority and an optional
  click action or popover. Items below priority 80 move into the overflow
  menu as the window narrows. Items are data rendered by the app's own
  components; nothing from IPC is rendered as HTML.
- **Background operations**: workflow runs and update download and
  installation, with real progress only. Cancellation appears only for
  operations that declare a safe `cancel`. Finished operations stay visible
  briefly.
- **Notifications** from existing events: agent stopped, version mismatch,
  device disconnected, update available or failed, workflow failed, security
  problems. Repeats of an unread event are combined, the list is bounded,
  and messages are one-line summaries.

The agent indicator reports only states the app knows: connecting, running,
stopped and version mismatch. Uptime is derived from the agent's start time.
Simulated devices are counted separately from physical ones.

## Configuration separation

| Area                   | Contents                                       | Portable |
| ---------------------- | ---------------------------------------------- | -------- |
| Profiles               | Actions, workflows, projects, layouts          | Yes      |
| Machine configuration  | Resource bindings, OS                          | No       |
| Runtime state          | Active profile, active project per profile, device brightness | No |
| Secrets (planned)      | Credentials needed by actions                  | Never    |

Portable data never contains absolute paths: `ResourceKey` rejects them by
construction.

## Security model

See [security.md](security.md): an always-encrypted database whose key is
system-managed (default) or password-derived, sensitive operations behind a
fresh authentication, encrypted exports (XChaCha20-Poly1305, Argon2id) as
the portable backup, and no secrets in portable profiles.
