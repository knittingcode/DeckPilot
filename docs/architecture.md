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
- `ModelVerification`: per-platform verification level with evidence
  (see [devices.md](devices.md))
- `DeviceDescriptor`: vendor-neutral description of one device (vendor, model,
  serial, capabilities, driver, verification), the device context for code
  outside the drivers
- `DeviceCapabilities` with optional `KeyGrid`, `EncoderSpec` and `TouchSurface`;
  validates whether a control exists and whether an input can occur on it
- `ControlId` (`Key`, `Encoder`, `TouchZone`) and `ControlInput` (`Press`,
  `Release`, `Rotate`, `Touch`)
- `DeviceEvent` (`Connected`, `Disconnected`, `Input`) and `DeviceId`
  (model + serial)
- `DeviceRegistry` tracks connection state of multiple devices across hotplug
- `ModelCatalog` with the built-in Stream Deck descriptors (MK.2, XL, +)
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

With the `plugins` feature it also runs the plugin backend
([marketplace/agent-integration.md](marketplace/agent-integration.md)): the
plugin store (`deckpilot-plugin-store`, its own encrypted `plugins.db`), the
signed registry client, the permission broker and one sandboxed host per
running plugin. Plugin dependencies (plugin API 1.4,
[marketplace/dependencies.md](marketplace/dependencies.md)) are resolved by
`deckpilot_plugin_store::deps` from the installed plugins, the built-in
modules of the agent's module registry and the signed registry, and carried
out all or nothing by `PluginManager::apply_plan`, with each plugin's own
consent. Each installed plugin is a feature module of the module registry
that declares its required dependencies, so the module registry's
dependency rules (a module required by an enabled module cannot be turned
off) cover plugins and built-in modules alike.

### `deckpilot-keystore`

Storage of database keys in the platform credential store (macOS Keychain,
Windows Credential Manager, Secret Service) behind a small `KeyStore` trait,
an in-memory store for tests, separate service names for development builds,
and no file fallback.

### `deckpilot-render`

Draws `KeyFace`s into `Frame`s (icon, title, value line, appearance, live
state marks) with a frame cache, and imports, checks and scales key icons
(PNG and JPEG decoded with limits, re-encoded as PNG). Text is set in Inter
(medium and semibold, OFL) and the keys' icons come from the Lucide icons
the app uses (ISC), both subset and compiled in (`assets/`, built by
`assets/make-fonts.py`, see `NOTICE`), read with `skrifa` (fontations) and
filled with anti-aliasing by `ab_glyph_rasterizer`. Used by the agent for device images and by the configuration
app for exact previews; the export import uses it to check icons. See
[Key appearance](data-model.md#key-appearance).

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
- `shared_layouts`: layouts as resources shared by profiles — what a layout
  uses and needs, its fit on each device model, what depends on the
  profile it runs in, and the per-device way back between layouts
  (`LayoutHistory`); see [Shared layouts](data-model.md#shared-layouts)
- `profile`: `Profile` and consistency validation
- `platform`: `Os`
- `runtime`: `Engine` (runs actions and workflows)
- `device_runtime`: `Runtime` (active profile and project, connected devices,
  the layout each device shows, page and layout navigation, folders, input
  dispatch, layout status) and `LayoutStatus`
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

Action types and feature pages come from modules: `modules/deck`,
`general`, `http`, `notifications`, `system` and `work-session`.
`deckpilot-modules` is the only crate that lists them; the agent and the app
build their registries from it. The former built-in `github`, `gitlab`,
`jira`, `git`, `docker`, `workspace` and `projects` modules (removed in
WP-C5/WP-C6) are first-party plugins now; their legacy action ids stay in
`ACTION_ALIASES`. See [Feature modules](modules.md).

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
forwards them to the window. The main window opens with the device editor;
Layouts (every layout, the profiles that use it, its fit on each model),
Dashboard, Action Library and Workflows are in its navigation rail, the
plugin Marketplace (builds with plugins) in its toolbar. The settings
(general, profiles, devices, agent, security, updates, plugins, diagnostics,
modules, …) are a window of their own. Settings › Devices shows the
connected devices and the supported models, Settings › Profiles the
profiles; the former Device Models and Profiles pages and links to them lead
there. Further pages come
from enabled modules (Projects, including this machine's resource mappings).
Profile-scoped pages show the profile they edit and are re-created when
another profile is chosen, so an open editor never writes to a different
profile. Every editor saves through the backend; layouts are
validated against the device model and previewed with the same face logic and
renderer the agent uses. Live values in the layout editor's preview (a CPU
monitor's use, a repository's branch) come from the agent too: the editor
watches the states of the shown page's keys (`watch_key_states`, IPC
`watchKeyStates`, protocol 11) and draws them with `preview_layout`'s `live`
argument. The agent reads them with the providers that serve the device,
so the app never samples anything itself (see
[background-agent.md](background-agent.md)). The preview's key images are
drawn by the same renderer at the size the editor shows a key on the
display (`preview_layout`'s `keyPixels`: the key's CSS width times
`devicePixelRatio`), not scaled up from the device's 72 or so pixels; the
device itself still gets images at its own size. Labels follow the
profile's active project, as on the device: the editor has no project
selector of its own, as projects are an optional module. The frontend is React + TypeScript with Tailwind CSS and shadcn/ui
components; `src/lib/types.ts` mirrors the serialized Rust types.

Native windows besides the main window come from one catalog
(`src-tauri/src/windows.rs`) and are opened by identifier only — by the app,
or by the agent over IPC: the prompts that open the configuration
(`security.*`), export and import (`portability.*`), update progress
(`updates.progress`), the agent log (`app.logs`), the workflow summary and
windows declared by enabled modules. Prompts render through the shared
`ModalShell` and are sized to their content by the window manager (no
per-window dimensions). The catalog decides whether a window may open while
the configuration is locked. Only the export and import windows may use file
dialogs.

The settings are not a window: they are a view of the main window (see
below). A request for the former settings window (`app.settings`, from the
agent or an older frontend) shows the main window at the settings instead
(`deckpilot://navigate` with `settings:<page>`, the page a validated page
reference), and still needs the open configuration; no window is created.

Windows keep their own copy of the app's state (profiles, modules, the
profile being edited, the theme) and announce their changes to each other
with app events (`lib/window-sync.ts`): a profile created in another window
shows in the main window at once, and a theme chosen anywhere applies
everywhere. The main window refuses another window's choice of
the profile to edit while the device editor has unsaved changes.

### Main window shell

The main window has one toolbar (`AppTitlebar`), a slim navigation rail
(`AppSidebar`, always collapsed to icons: each has its name as label and
tooltip, the current page is marked; nothing expands it, there is no toggle
and no ⌘B / Ctrl+B) and the page. The toolbar, left to right: room for the
native window controls (macOS only; nothing else), the device and profile
selectors (`shell/toolbar.tsx`), the page's context, then plugin
navigation entries (`shell/plugin-navigation.ts`; with a separator only when
there are any), notifications, the Marketplace and the settings. The device
selector lists connected devices (named by their descriptor) and the
catalog's models without a device ("not connected"); device, profile and
layout of the editor are one shell-level selection
(`shell/editor-selection.tsx`) that refuses to switch over unsaved layout
changes.

The view is a workspace page or the settings, both locations of one model
(`lib/navigation.ts`, `hooks/use-navigation.ts`) that keeps the current one
across launches. There is no back and forward: the rail (its Device Editor
leads back from anywhere), the selectors, the Marketplace, plugin entries
and the gear reach every view. The toolbar's gear (or ⌘, / Ctrl+,) opens the settings at the section
shown last (remembered across launches, `deckpilot.settings.page`), and is
pressed while they are on screen; a section can be opened directly
(`settings:security`). The settings sidebar (`SettingsSidebar`: sections and
their search) sits beside the rail as part of the view, and the section
fills the rest. While the settings
are on screen the workspace page under them stays mounted but hidden, so
the device editor returns exactly as it was (page, key, inspector, unsaved
changes, scroll position); its toolbar contributions wait in a separate
store and its window-wide editing shortcuts are off (`ViewActiveContext`).

The device editor (`layout-editor/editor.tsx`) has the device and its pages
in the centre, the key inspector below it and the Action Library at the
right (`LibraryDock`: always shown, one fixed width from the
`--library-width` token, never collapsed or resized; under the editor in
windows narrower than the main window's 860 px minimum, which only a window
manager ignoring it allows). The toolbar shows no title for the editor: its
device and profile selectors say what is edited. One drag and drop context spans all
three, so library entries drop onto controls and onto gesture slots. The
device's colours are theme tokens (`--device-*` in `index.css`).

The toolbar's device and profile selectors (with their connection and
profile status) appear only for views that work on them
(`lib/toolbar-context.ts`): the device editor shows both, the Action
Library, Workflows and the Projects and Work Session pages the profile, the
dashboard, the Layouts page, the Marketplace and the settings neither. A
plugin view shows what its manifest declares (`contributes.views[].context`,
plugin API 1.4; `none` when absent or unknown). Hiding them changes no
selection: the shell keeps the device, the profile, the layout and, per
layout, the editor's page and selected keys (`EditorSelectionProvider`), so
the editor comes back as it was from any view. Without selectors the page's
context starts where they would.

Pages render only their content; their title,
subtitle, breadcrumbs, status and commands go to the titlebar through
`useTitlebar` (`lib/titlebar.ts`). Settings pages are the exception for the
subtitle: the section's title and description head its content
(`SettingsHeader`), the titlebar shows only the breadcrumbs (not the
title a second time). Any component of a page may contribute —
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

Content pages share one flat layout vocabulary
(`components/settings-layout.tsx`): sections with a heading, description and
commands (`SettingsSection`, `SectionHeader`), groups of rows separated by
hairlines (`SettingsGroup`, `SettingsRow`: label and description left, the
control right, wrapping under the label in a narrow window), label–value
lists (`PropertyList`), a dot-and-word state (`InlineStatus`) and one framed
container for editors and configuration forms (`Panel`, a hairline border
without shadow, never nested). Lists and tables separate rows with
hairlines instead of boxing each item; `EmptyState` has a `plain` variant
for them. Cards (`components/ui/card.tsx`) remain only in the device
editor. Status colours (`--success`, `--warning`) and the 13 px
`text-caption` size are tokens in `index.css`.

Sizes are tokens in `index.css` (`--titlebar-height`, `--titlebar-control`,
`--titlebar-icon`, …). Before the selectors the toolbar only leaves room
for the macOS traffic lights (`--titlebar-controls-inset`, 0 in fullscreen
and on Windows and Linux, where the window manager draws the controls
above the toolbar). Native dialog windows (`ModalShell`) have no app titlebar.

### Status bar

A compact bar at the bottom of the main window (`AppStatusBar`), never in
other windows. It renders two of three shared stores (the third,
notifications, is the toolbar's `NotificationsButton`)
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
| Profiles               | Workflows, projects, the layouts they use (in order, default per model), their own actions and icons | Yes |
| Shared layouts         | Layouts and the actions and icons their keys use, one record each, used by any number of profiles | Yes (with the profiles that use them; all of them in an export of everything) |
| Machine configuration  | Resource bindings, OS                          | No       |
| Runtime state          | Active profile, active project per profile, device brightness, the page and layout each device shows per profile | No |
| Secrets (planned)      | Credentials needed by actions                  | Never    |

Portable data never contains absolute paths: `ResourceKey` rejects them by
construction.

## Security model

See [security.md](security.md): an always-encrypted database whose key is
system-managed (default) or password-derived, sensitive operations behind a
fresh authentication, encrypted exports (XChaCha20-Poly1305, Argon2id) as
the portable backup, and no secrets in portable profiles.
