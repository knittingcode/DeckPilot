# Background agent

`deckpilot-agent` is the process that owns device communication and action
execution. It runs independently of the configuration app: the app can be
closed and reopened at any time without interrupting devices.

## Responsibilities

- Discover Stream Deck devices, open them and recover after unplug/replug
- Render the active profile's layout on the keys
- Route key presses to action instances and run them
- Keep runtime state: active profile, active project, device brightness
- Provide a system tray menu
- Serve local IPC requests from the configuration app

## Architecture

```
            main thread                         background thread (Tokio)
 ┌──────────────────────────────┐   proxy   ┌────────────────────────────────────┐
 │ tao event loop + tray-icon   │◄──────────│ Agent (single task, owns all state)│
 │ menu clicks → TrayCommand ───┼──────────►│  • Runtime (profile, projects,     │
 └──────────────────────────────┘ commands  │    navigation, faces)              │
                                            │  • SqliteStore                     │
     IPC clients ──► IPC server task ──────►│  • device sessions                 │
     (config app)    (auth, validation)     │  • broadcast of AgentEvents        │
                                            └──────┬──────────────────▲──────────┘
                                          images,  │                  │ input,
                                          brightness ▼                │ disconnect
                                            ┌────────────────────────────────────┐
                                            │ one I/O thread per device          │
                                            │ DeviceHandle (HID or mock)         │
                                            └────────────────────────────────────┘
```

- **Single owner of state.** All runtime state lives in one Tokio task that
  processes commands from IPC connections, the tray and device threads in
  order. There are no shared mutable structures between threads.
- **Device I/O threads.** HID handles cannot be shared between threads, so
  each opened device has one thread. It blocks in a read with a 100 ms
  timeout (no busy polling) and applies queued image and brightness updates
  between reads. Input is delivered immediately.
- **Discovery.** The agent rescans every 2 seconds (and immediately on
  "Refresh Devices"). Newly found devices are opened; devices that disappeared
  or whose I/O failed are closed and reopened on a later scan, which handles
  temporary disconnects.
- **Rendering.** `Runtime::faces` describes each key in a device-independent
  way (`KeyFace`: style, title, detail, highlight). The agent renders faces to
  RGB frames at the size the model reports, and the driver converts them to
  the device format (for the MK.2: 72 × 72 JPEG, mirrored). Only keys whose
  face changed are redrawn, and rendered frames are cached by face and
  size. Rendering never executes actions.
- **Live key state.** Keys whose action has a module-provided
  [button state](modules.md#button-state) show it (an indicator, detail,
  progress or toggle). Providers run on their own threads, at most four at a
  time and never more often than their declared interval; the agent task
  sleeps until the next one is due and redraws only keys whose state
  changed. Keys bound to a running workflow show a running mark with its
  progress.
- **Live key state for previews.** A client may watch the states of
  actions' keys (`watchKeyStates`, protocol 11): the configuration app does
  for the page its layout editor shows. Watched keys of the active profile
  are read like shown keys (same providers, same cache, same interval; a
  key both shown and watched is read once) also when no device shows them,
  and their changes are sent as `keyStates` messages to the watching
  connection only. A watch ends when its connection closes or is replaced;
  a watch of another profile has no states (`live: false`).
- **Tray.** On macOS the agent runs as an accessory app (no Dock icon):
  `LSUIElement` in the `Info.plist` embedded in its executable, so not even
  a short-lived agent process (a second instance stopping at the lock)
  shows one, and the accessory policy of its event loop. On
  Linux the tray uses AppIndicator; desktop environments without
  AppIndicator/StatusNotifier support (for example GNOME without the
  extension) do not show the icon, but the agent keeps working. Run with
  `--headless` to disable the tray entirely.

### Tray menu

- **Open DeckPilot**: focuses the configuration window if it is connected,
  otherwise starts the configuration app found next to the agent
  (`deckpilot-desktop` in a build directory or Linux package, `DeckPilot` in
  the macOS `.app` and on Windows; inside an AppImage the AppImage itself),
  or `DECKPILOT_DESKTOP_BIN`. While an app it started is still starting (up
  to 20 seconds, until it connects), further clicks start no second one. In
  a development session `DECKPILOT_DESKTOP_BIN` is `deckpilot-dev`, which
  restarts the session's `tauri dev` with its Vite server (see
  [development](development.md#live-development))
- **Active Profile**: lists profiles; selecting one switches immediately and
  redraws all devices
- **Project**: shows the active project (informational)
- **Connected Devices**: devices and their connection state
- **Refresh Devices**: rescans and redraws all keys
- **About DeckPilot** and **Quit DeckPilot**: quits DeckPilot completely;
  the agent stops, and on macOS open configuration apps close too (see
  [Lifecycle](#lifecycle))

"Check for Updates" is not offered because update distribution does not
exist yet.

## Lifecycle

| Situation                        | Behaviour                                                        |
| -------------------------------- | ---------------------------------------------------------------- |
| Agent starts                     | Takes an exclusive lock on `agent.lock`; a second instance exits (after waiting up to 5 s, see below) |
| Configuration app starts         | Connects to the running agent, or starts it if it is not running |
| Configuration app closes         | Agent keeps running                                              |
| Quit from the tray               | Agent stops (`shuttingDown` with reason `quit`); on macOS open configuration apps close too |
| Device unplugged                 | Device marked disconnected; agent keeps running                  |
| Device plugged in again          | Reopened on the next scan, layout redrawn                        |
| Quit (tray), `Shutdown` request, Ctrl+C, SIGTERM | Keys blanked, IPC closed, token removed, lock released |

### Windows, the Dock and the three quits (macOS)

The configuration app is a window onto the agent; the agent stays in the
menu bar as an accessory app that never appears in the Dock.

On macOS the app behaves like Docker Desktop. Closing the main window (the
close button or ⌘W) hides it and switches the app to the **accessory**
activation policy: its icon leaves the Dock, the process keeps running
(still connected and unlocked) and the agent is untouched. Every path that
shows a window switches back to the **regular** policy first, so the window
and its Dock icon return together: the tray's *Open DeckPilot*
(`showWindow`), opening the app from Finder, Spotlight, Launchpad or the
Dock (the application's *reopen* event), and a password or module window.
After any window closes, the policy follows the windows that are still
visible (regular while one is, accessory when none is). The app's own
`Info.plist` has no `LSUIElement`, so a launch always starts in the Dock.
macOS keeps one process per application, so opening it again reaches the
running one; there is never a second app or window, and the agent's
instance lock keeps a single agent and tray icon.

| How                                         | Windows        | App process          | Agent         |
| ------------------------------------------- | -------------- | -------------------- | ------------- |
| Close Window (⌘W, the close button)         | that one       | macOS: keeps running, out of the Dock; Windows, Linux: ends with the last | keeps running |
| Quit DeckPilot UI (⌘Q, the app menu on macOS) | all          | ends                 | keeps running |
| Quit DeckPilot (tray)                       | all            | ends                 | stops         |

- **Open DeckPilot** reaches a running app as `showWindow`, which brings its
  main window to the front; only when no app is connected does the agent
  start one, so there is never a second app. On macOS an app started for a
  single prompt or module window creates its main window for it.
- The tray's Quit reaches the apps as `shuttingDown` with `reason: "quit"`,
  and on macOS the app then exits. Every other stop (`shutdown` from the
  app's "restart agent", `prepareForUpdate`, a signal) gives another
  reason, and the app stays open and reconnects. The field is optional:
  agents before 0.6 send none, and apps that do not know it ignore it, so it
  needs no new protocol; such an app stays open after a tray Quit, as
  before.
- On macOS the application menu's Quit reads "Quit DeckPilot UI", so it is
  not mistaken for the tray's "Quit DeckPilot".

**macOS only.** Closing the main window to the menu bar, the activation
policy, closing the apps on the tray's Quit, creating the main window for
`showWindow` in a single-window app, and the "Quit DeckPilot UI" label
apply to macOS. On Windows and Linux the app behaves as before: it
stays open after the tray's Quit and reconnects, an app without a main
window ignores `showWindow`, and the menu is Tauri's default. The agent
sends the `reason` on every platform.

Possible follow-up: apply the same behaviour to Windows and Linux (closing
the apps on the tray's Quit, the main window for single-window apps); the
decisions live in `apps/desktop/src-tauri/src/lifecycle.rs` and only take
the platform as a parameter.

The lock is an OS file lock, so it is released automatically if the process
crashes. A starting agent waits up to 5 seconds for the lock before it
concludes that another agent is running: a stopping agent closes its
endpoint before it has finished and released the lock, and a restart (the
app's "restart agent", an update) starts the next agent as soon as the
endpoint is gone. After a crash the new agent replaces the stale socket
file and token; the old token is no longer accepted.

### Start at login

The agent can register itself to start at login using per-user mechanisms
only (no system services):

- macOS: a LaunchAgent in `~/Library/LaunchAgents`
- Linux: an XDG autostart entry in `~/.config/autostart`
- Windows: the `HKEY_CURRENT_USER\…\Run` registry key

Use the checkbox in Settings, or `deckpilot-agent autostart enable|disable|status`.

The entry runs the agent's executable, which is at a fixed path for the
macOS app, the Windows installers and the Linux `.deb`/`.rpm`
(`/usr/bin/deckpilot-agent`). An AppImage instead mounts its contents at a
new temporary path on every run (`/tmp/.mount_…`) and unmounts it when its
main process ends. So when the agent runs from an AppImage (`$APPIMAGE` is
set and the agent lies inside `$APPDIR`):

- the autostart entry runs `<the .AppImage file> --agent`; the app's
  executable, started with `--agent`, replaces itself with the bundled
  agent, so the agent is the AppImage's main process and its files stay
  mounted as long as it runs;
- the app starts the agent the same way, and the agent starts the app
  through the `.AppImage` file, so neither depends on the other's mount;
- an enabled entry is rewritten when the agent starts, which repairs
  entries that earlier versions recorded with a path inside the mount.

The mechanism depends on the build's distribution channel, which is
compiled in (see [distribution/README.md](distribution/README.md)). GitHub,
APT, Homebrew and WinGet builds are ordinary installations and use the
entries above. Store builds must not write them: the Mac App Store sandbox
does not allow a LaunchAgent file (a sandboxed app registers a login item
with `SMAppService`), an MSIX package declares a `windows.startupTask`
instead of writing the Run key, and a Flatpak asks the Background portal.
Those are not implemented yet, so a Mac App Store, Microsoft Store or
Flathub build reports start at login as off, refuses to turn it on with a
message saying so, and writes nothing (`autostart::Strategy`).

## Storage ownership

The agent and the configuration app open the same SQLite database (WAL mode
with a busy timeout).

- **Configuration documents** (profiles, machine resource bindings) are
  written by the configuration app. After a change it sends
  `reloadConfiguration` to the agent.
- **Runtime state** (active profile, active project, brightness) is written by
  the agent while it runs. The configuration app changes it through IPC and
  only writes it directly when the agent is not running.

## IPC protocol

Transport:

- **Linux/macOS**: Unix domain socket `agent-<hash>.sock` in a per-user
  directory `deckpilot-<uid>` with mode `0700`, located in
  `$XDG_RUNTIME_DIR` or, without it, the temporary directory (`$TMPDIR`).
  Socket paths are limited to 104 bytes on macOS and 108 on Linux; when the
  path there would be longer (a long `TMPDIR`), the directory is
  `/tmp/deckpilot-<uid>` instead. The choice depends only on the
  environment and the data directory, so the agent and a client started
  with the same environment agree. The agent creates the directory with
  mode `0700`; both the agent and every client refuse it if it is a symlink,
  belongs to another user or is accessible to group or others (it is not
  repaired, since another user may already have placed a socket in it), so
  the token is never sent to a socket someone else controls. On Linux the
  socket file additionally has mode `0600`. Abstract sockets are never used.
- **Windows**: a named pipe with a security descriptor that grants access only
  to the owner and SYSTEM; remote clients are rejected.

The socket name is derived from the data directory, so agents using different
`DECKPILOT_DATA_DIR`s do not interfere.

Authentication: on start the agent writes a random 256-bit token to
`agent.token` in the data directory (mode `0600` on Unix; the data directory
is per-user on Windows). Every connection must begin with a `hello` that
carries the token and the protocol version. Wrong tokens and unsupported
versions are rejected and the connection is closed.

Framing: one JSON object per line, at most 64 KiB. Malformed or unknown
requests receive an `invalidRequest` error and the connection stays open;
an oversized message from a client closes the connection. An answer of
the agent's that does not fit is shortened or refused with the reason
`answerTooLarge`, and an event that does not fit is left out; the
connection stays open (plugin answers: `docs/marketplace/agent-integration.md`,
"Answers of any size").

```jsonc
// client → agent
{"type":"hello","protocol":1,"token":"…","client":"desktop"}
{"type":"request","id":1,"request":{"type":"cycleProject","direction":"next"}}

// agent → client
{"type":"welcome","protocol":1,"agentVersion":"0.1.0"}
{"type":"response","id":1,"response":{"type":"activeProject","project":{…}}}
{"type":"error","id":1,"error":{"code":"notFound","message":"…"}}
{"type":"event","event":{"type":"state","status":{…}}}
```

Requests (`crates/deckpilot-ipc/src/protocol.rs`):

| Request                 | Purpose                                              |
| ----------------------- | ---------------------------------------------------- |
| `getStatus`             | Version, PID, paths, devices, profiles, active state |
| `listDevices`           | Devices with connection and layout status            |
| `listProfiles`          | Profiles and the active one                          |
| `getActiveProfile` / `setActiveProfile` | Read or switch the active profile    |
| `getActiveProject` / `selectProject` / `cycleProject` | Project context        |
| `runAction`             | Run an action of the active profile                  |
| `refreshDevices`        | Rescan and redraw                                    |
| `reloadConfiguration`   | Re-read configuration after the app changed it       |
| `setBrightness`         | 0–100                                                |
| `getAutostart` / `setAutostart` | Start at login                               |
| `runWorkflow`           | Run a workflow and answer when it ends (protocol 2)  |
| `startWorkflow`         | Start a workflow in the background (protocol 5)      |
| `cancelExecution`       | Stop a running workflow between nodes (protocol 5)   |
| `listExecutions` / `getExecution` | Running and recorded runs (protocol 5)     |
| `clearExecutionHistory` | Delete recorded runs of the profile (protocol 5)     |
| `prepareForUpdate`      | Release devices and exit for an update (protocol 2)  |
| `listVariables`         | Variables available in a context, metadata only (protocol 7) |
| `previewReferences`     | What references resolve to, sensitive values redacted (protocol 7) |
| `getWorkSession` / `listWorkSessions` | The current work session and the history (protocol 8) |
| `getWorkSessionSettings` / `saveWorkSessionSettings` | Work Session settings of a profile (protocol 8) |
| `resolveWorkSessionReview` / `clearWorkSessionNotices` | Decide a flagged gap; clear a notice (protocol 8) |
| `subscribe`             | Receive `state`, `actionCompleted`, `showWindow`, `openWindow`, `shuttingDown`, `execution*`, `notification`, context (`projectChanged`, `sharedVariablesChanged`, …; protocol 7) and `workSessionChanged` (protocol 8) events |
| `showWindow`            | Bring the configuration window to the front          |
| `shutdown`              | Stop the agent gracefully                            |
| `simulateInput`         | Inject input into a simulated device (`--mock-devices` only) |

The current protocol is 9. Protocol 9 adds the unified variable registry:
owners, module variables, canonical ids and the `unavailable` status (see
[Variables](variables.md#25-ipc-protocol-9)). Protocol 8 adds the Work
Session requests and events (see
[Key gestures and work sessions](gestures-and-work-session.md#9-ipc-and-tauri)).
Protocol 7 adds shared variables: `listVariables`, `previewReferences` and
the context events. Protocol 6 lets actions show notifications, which the
agent sends to the app as `notification` events for its status bar (see
[Integration modules](integration-modules.md#notifications-notifications)).
Protocol 5 runs workflows in the background (see
[Workflow graphs](workflow-graphs.md#running-in-the-agent)). Protocol 4 added
the encrypted database; protocol 3 made action results only carry
`selectProject` effects, with action types in module namespaces
(`projects.next`); a `cycleProject` requested by `projects.cycle` is applied
by the engine and reported as the `selectProject` it made, so it never
reaches a client. Project switching requests are refused while the projects
module is disabled. Events are only sent to clients whose protocol has them.
Clients negotiate the protocol (stepping down until an older agent accepts
their `hello`) and never send a request newer than the connection's
protocol; the agent refuses such a request with `unsupported`. See
[Compatibility](compatibility.md) for the negotiation, the capabilities and
the tested version matrix.
The agent also accepts protocol 1 and 2 clients
(DeckPilot 0.2), so a newer configuration app can always stop an older agent
during an update. The agent status includes the time of the last device input,
which automatic updates use to wait until the deck is idle.

## Hardware support

| Model             | USB ID      | Status                                  |
| ----------------- | ----------- | --------------------------------------- |
| Stream Deck MK.2  | `0fd9:0080` | Physically verified on macOS (2026-11-13); Windows and Linux: automated tests only, **not tested on a physical device** |
| Stream Deck XL    | `0fd9:006c` | Layout design only (descriptor, no driver) |
| Stream Deck +     | `0fd9:0084` | Layout design only (descriptor, no driver) |

The per-platform status, its levels and evidence are static data in the model
registry; see [Devices](devices.md#verification-levels).

The driver (`crates/deckpilot-hid`) uses `elgato-streamdeck` on top of
`hidapi`. The MK.2 descriptor (15 keys in 5 × 3, 72 × 72 px JPEG, mirrored on
both axes, keys indexed left to right, top to bottom) is checked against the
library's protocol constants by a unit test. Other Stream Deck models are
recognised by the library but deliberately not exposed until they are
described and tested.

Supported operations: discovery, connect/disconnect, key press and release
(state changes are diffed so repeated reports do not trigger twice), key
images, brightness.

The device abstraction (`DeviceBackend`, `DeviceHandle`, `Frame`,
`DeviceCapabilities`) is not specific to the MK.2: encoders, touch zones,
other grids and image sizes are modelled and validated, but no driver for
such hardware exists yet.

### Platform requirements

- **Linux**: access to `hidraw` devices. Install
  `packaging/linux/70-deckpilot-streamdeck.rules` (see the file for the
  commands) and replug the device. Building requires `libudev-dev`.
- **macOS**: no extra setup expected. If macOS asks for Input Monitoring
  permission, grant it to the agent.
- **Windows**: no driver installation expected; the device uses the built-in
  HID driver.

These requirements follow from the HID library's documentation and still
need to be confirmed with real hardware.

## Running locally

```sh
cargo run -p deckpilot-agent                    # tray + USB devices
cargo run -p deckpilot-agent -- --headless      # no tray
cargo run -p deckpilot-agent -- --mock-devices  # simulated MK.2
cargo run -p deckpilot-agent -- --mock-devices mk2,xl,plus
cargo run -p deckpilot-agent -- --mock-devices --no-usb  # simulated only
DECKPILOT_DATA_DIR=/tmp/deckpilot-dev cargo run -p deckpilot-agent -- --mock-devices
```

Logs go to stderr and to `<data dir>/logs/agent.YYYY-MM-DD.log` (seven files
are kept). Use `--log-level debug` for request and input details. Action
results are logged by name and status only; result messages are not logged
because they may contain machine-specific paths.

The configuration app starts `deckpilot-agent` from the same directory as its
own executable when no agent is running. Debug builds of the app accept
`DECKPILOT_AGENT_BIN` naming another agent executable (development, the
desktop end-to-end tests); release builds do not read the variable at all,
so nothing in the environment can make them start another program as the
agent. In development, build both first:

```sh
cargo build -p deckpilot-agent
pnpm dev
```

## Testing

Automated tests never need hardware:

- `deckpilot-device` provides `MockBackend` (feature `mock`): simulated
  devices that can be plugged, unplugged and pressed, and that record every
  frame and brightness change.
- `apps/agent/tests/agent.rs` runs the real agent with mock devices and real
  IPC: rendering, button routing, project cycling and persistence, profile
  switching, unplug/replug, brightness, authentication, request validation,
  single instance, shutdown and client reconnection.
- `apps/agent/tests/hardware.rs` covers the device lifecycle with simulated
  MK.2s: a replugged device gets its page, brightness and images back (also
  when unplugged while keys are redrawn), several devices keep their own
  pages and connections, keys are blanked when the agent stops and stale
  images are replaced when it starts.
- `apps/desktop/src-tauri/src/tests.rs` drives the configuration app's
  commands against a running agent.

With `--mock-devices`, the agent can also be exercised manually: the
simulated MK.2 accepts `simulateInput` requests.

Simulated devices are added next to USB devices. Tests that start the agent
executable (`apps/agent/tests/mock_devices.rs`, `deckpilot-dev hardware-test
--mock`) also pass `--no-usb`, so a Stream Deck plugged into the machine
running them is neither found nor drawn on. In-process tests use a
`MockBackend` alone and never open USB devices. A normal agent always opens
them.

### Hardware test procedure

Perform these steps with a physical Stream Deck MK.2 before marking the model
as verified. Record OS, OS version and firmware (`deckpilot-agent --log-level
debug` logs the device ID).

1. Start the agent; within 2 seconds the device appears in the tray menu and
   in Devices as connected.
2. Create a profile with projects `SHOP`, `CMR`, `CMS`, a *Cycle project*
   action and a *Log message* action; generate a layout in Devices.
3. Check that the keys show the action names, unbound keys are black, and the
   text is upright and not mirrored.
4. Press the project key repeatedly: SHOP → CMR → CMS → SHOP. The key's second
   line and the Dashboard follow; each press switches exactly once.
5. Hold a key for several seconds: no repeated execution.
6. Change brightness in Settings; the device follows.
7. Switch profile from the tray; keys change without restarting the agent.
8. Unplug the device: the agent keeps running and shows it as disconnected.
   Replug: the layout is restored within a few seconds.
9. Quit from the tray: keys go blank.
10. On Linux, repeat step 1 as a normal user with the udev rule installed.
