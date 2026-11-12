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
- **Tray.** On macOS the agent runs as an accessory app (no Dock icon). On
  Linux the tray uses AppIndicator; desktop environments without
  AppIndicator/StatusNotifier support (for example GNOME without the
  extension) do not show the icon, but the agent keeps working. Run with
  `--headless` to disable the tray entirely.

### Tray menu

- **Open DeckPilot**: focuses the configuration window if it is connected,
  otherwise starts the configuration app found next to the agent (or
  `DECKPILOT_DESKTOP_BIN`)
- **Active Profile**: lists profiles; selecting one switches immediately and
  redraws all devices
- **Project**: shows the active project (informational)
- **Connected Devices**: devices and their connection state
- **Refresh Devices**: rescans and redraws all keys
- **About DeckPilot** and **Quit DeckPilot**

"Check for Updates" is not offered because update distribution does not
exist yet.

## Lifecycle

| Situation                        | Behaviour                                                        |
| -------------------------------- | ---------------------------------------------------------------- |
| Agent starts                     | Takes an exclusive lock on `agent.lock`; a second instance exits |
| Configuration app starts         | Connects to the running agent, or starts it if it is not running |
| Configuration app closes         | Agent keeps running                                              |
| Device unplugged                 | Device marked disconnected; agent keeps running                  |
| Device plugged in again          | Reopened on the next scan, layout redrawn                        |
| Quit (tray), `Shutdown` request, Ctrl+C, SIGTERM | Keys blanked, IPC closed, token removed, lock released |

The lock is an OS file lock, so it is released automatically if the process
crashes.

### Start at login

The agent can register itself to start at login using per-user mechanisms
only (no system services):

- macOS: a LaunchAgent in `~/Library/LaunchAgents`
- Linux: an XDG autostart entry in `~/.config/autostart`
- Windows: the `HKEY_CURRENT_USER\…\Run` registry key

Use the checkbox in Settings, or `deckpilot-agent autostart enable|disable|status`.

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
oversized messages close the connection.

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
| `subscribe`             | Receive `state`, `actionCompleted`, `showWindow`, `openWindow`, `shuttingDown`, `execution*` and `notification` events |
| `showWindow`            | Bring the configuration window to the front          |
| `shutdown`              | Stop the agent gracefully                            |
| `simulateInput`         | Inject input into a simulated device (`--mock-devices` only) |

The current protocol is 6: actions show notifications, which the agent
sends to the app as `notification` events for its status bar (see
[Integration modules](integration-modules.md#notifications-notifications)).
Protocol 5 runs workflows in the background (see
[Workflow graphs](workflow-graphs.md#running-in-the-agent)). Protocol 4 added
the encrypted database; protocol 3 made action results only carry
`selectProject` effects, with action types in module namespaces
(`projects.next`). Project switching requests are refused while the projects
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
| Stream Deck MK.2  | `0fd9:0080` | Driver implemented, **not yet tested on a physical device** |
| Stream Deck XL    | `0fd9:006c` | Layout design only (descriptor, no driver) |
| Stream Deck +     | `0fd9:0084` | Layout design only (descriptor, no driver) |

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
DECKPILOT_DATA_DIR=/tmp/deckpilot-dev cargo run -p deckpilot-agent -- --mock-devices
```

Logs go to stderr and to `<data dir>/logs/agent.YYYY-MM-DD.log` (seven files
are kept). Use `--log-level debug` for request and input details. Action
results are logged by name and status only; result messages are not logged
because they may contain machine-specific paths.

The configuration app starts `deckpilot-agent` from the same directory as its
own executable when no agent is running (`DECKPILOT_AGENT_BIN` overrides the
path). In development, build both first:

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
- `apps/desktop/src-tauri/src/tests.rs` drives the configuration app's
  commands against a running agent.

With `--mock-devices`, the agent can also be exercised manually: the
simulated MK.2 accepts `simulateInput` requests.

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
