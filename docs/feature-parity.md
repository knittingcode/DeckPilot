# Built-in actions and Stream Deck feature parity

Status: **audit and design record**. Section 1 describes `main` at
`dc1a8a2e` (0.6.0-alpha.3 plus the changes merged since); the later
sections are the design for what is missing and are updated as the work
lands. Where the code differs, the code and the tests win.

The goal is functional parity with the essential actions and interaction
patterns people who use a Stream Deck expect — folders, pages, profile
switching, per-gesture key logic, multi actions, system and device actions,
a soundboard — in DeckPilot's own architecture and design. It is not a
visual clone: no artwork, icons, wording or layouts of other products are
used. Behaviour was taken from public documentation only; no reference
screenshots were available for this audit.

## Classification

**Status**

| Status | Meaning |
| --- | --- |
| verified | Implemented and covered by tests that are named |
| incomplete | Implemented in part; what is missing is listed |
| missing | Not implemented |
| platform | Depends on the operating system; the differences are listed |
| needs SDK | Could only become a plugin with a host capability the plugin SDK does not have |

**Home**

| Home | Meaning |
| --- | --- |
| core | Part of the agent and the app: the model, the runtime or an engine |
| built-in module | A module compiled into DeckPilot (`modules/*`), with its capabilities declared in its manifest. Available offline on a fresh install |
| default plugin | A sandboxed plugin shipped with DeckPilot as a verified package |
| optional plugin | A plugin the user installs |

The plugin system is experimental and off in released builds (cargo feature
`plugins`, `--enable-plugins`, `VITE_DECKPILOT_PLUGINS=1`), and its sandbox
has no process, file system, input injection, audio or device capability
(`docs/marketplace/module-migration.md`, G13–G16). Nothing in this document
is a plugin today, and nothing here adds a capability to the plugin broker.
See [Core or plugin](#3-core-or-plugin).

## 1. Audit

### 1.1 What exists

| Area | Where | What exists |
| --- | --- | --- |
| Layouts and pages | `crates/deckpilot-core/src/layout.rs`, `pages.rs` | `DeviceLayout { id, name, model, homePage, pages, explicitTree }` per device *model*, any number per profile, one active per model. `LayoutPage { id, name, parent, bindings, gestures }`. Pages have stable random ids (`PageId`); nothing refers to a page by position |
| Folders | `pages.rs`, `device_runtime.rs` | A folder is a page with a `parent`: `BindingTarget::OpenPage { page }` enters it, `BindingTarget::Back` returns to its parent. Any depth. Validation refuses an unknown parent, a cycle (`ParentCycle`) and a parent on the home page; `set_parent`, `move_page` and `copy_page` refuse cycles (`PageCycle`). A folder page without a Back binding gets an automatic Back key on its first free key, never on a bound one (`automatic_back`). Layouts from before `parent` have their tree inferred, never rewritten on read |
| Navigation state | `device_runtime.rs` (`navigation`, `parked`) | One page stack per connected device. A device that reconnects returns to its page; a reload keeps stacks whose pages still exist; a profile change resets every device to the home page of the new profile. Nothing is kept across an agent restart |
| Profiles | `profile.rs`, agent `service.rs::set_active_profile`, storage `settings` | One **global** active profile for the agent (setting `active_profile`), switched over IPC (`setActiveProfile`), from the app and from the tray. Every connected device shows that profile with the layout for its model. There is no per-device profile and no action that switches profiles |
| Gesture engine | `crates/deckpilot-core/src/gesture/`, `apps/agent/src/service/gestures.rs` | Click, double click and long press per key, each with its own binding (`Trigger::{Click, DoubleClick, LongPress}`), recognized from timestamped press and release edges by a pure state machine. Exactly one gesture per input sequence; a click waits only as long as the bound gestures require (`ClickTiming`); per-key and per-computer timings (double click 150–1000 ms, long press 300–5000 ms); pending gestures are dropped on disconnect, page change, profile change and lock. Raw `press`/`release` bindings fire at once and cannot be mixed with gestures on one control (`MixedTriggers`) |
| Stateful keys | `crates/deckpilot-core/src/stateful.rs`, `apps/agent/src/service/stateful.rs` | `BindingTarget::Stateful`: a built-in state machine picks the operation from its state and the gesture. The only machine is the Work Session. There is no generic multi-state key |
| Workflows | `workflow.rs`, `graph/`, `runtime.rs`, agent `service/executions.rs` | Step workflows (ordered actions with a condition, stop/continue on failure and overrides) and graph workflows (action, condition, parallel, join, loop, sub-workflow call, set variable, output, **delay**, stop). Background runs in the agent, at most four at a time and one per workflow, with progress events, cancellation, timeouts, retries, history and bounded nesting. A key bound to a workflow starts a run |
| Action types and instances | `action.rs`, `registry.rs`, `module.rs` | Modules provide action *types* (`<module>.<name>`) with a parameter schema; the user saves configured *instances* in the profile's Action Library; keys, steps and nodes reference instances by id. A handler changes agent state only by returning `ContextEffect`s, which the engine applies after a capability check. Unknown or disabled types stay in the profile and show as unavailable |
| Built-in modules | `crates/deckpilot-modules/src/lib.rs`, `modules/*` | `general` (log, wait), `projects`, `system` (open application, URL, file, folder; run an allow-listed command without a shell, off until allowed on this computer; process status), `workspace`, `git`, `docker`, `http`, `notifications`, `github`, `gitlab`, `jira`, `work-session` |
| Devices | `crates/deckpilot-device`, `crates/deckpilot-hid`, agent `devices.rs` | `DeviceCapabilities { keys, encoders, touch, brightness }` per model; MK.2 has a driver, XL and Stream Deck + are descriptors for layout design. `DeviceHandle`: read input, set and clear images, set brightness, flush. One brightness for all devices (setting, IPC `setBrightness`), sent only to devices that have it. Simulated devices (`MockBackend`, `--mock-devices`, IPC `simulateInput`). There is no sleep, wake, idle timeout or timer |
| Variables and labels | `variables/`, `face.rs` | Shared, system and module variables with `{{ … }}` references in parameters; key labels expand `{profile}`, `{project.key}` and `{project.name}`. Nothing about the page a device shows |
| Key faces | `face.rs`, `button_state.rs`, `crates/deckpilot-render` | Title, detail, highlight, per-binding appearance (colours, font size, title position, icon with fit) and live state from the action's module (label, detail, indicator, progress, toggle) |
| Layout editor | `apps/desktop/src/layout-editor/` | Device canvas with selection, keyboard navigation, undo and redo, copy, cut, paste, duplicate; a library of the profile's actions and workflows and navigation targets, dragged onto keys with `@dnd-kit`; a context menu (assign, open folder, clipboard, clear); page tabs, a page tree with reorder, move and copy between layouts; an inspector with the gesture editor, labels, parameter overrides, timings and appearance |
| Action Library | `apps/desktop/src/pages/actions.tsx`, `components/action-type-chooser.tsx` | The profile's instances grouped by category with search; a new action starts from a searchable list of types grouped by module |
| Workflow editors | `apps/desktop/src/pages/workflow-steps.tsx`, `workflow-builder/` | A step list with drag reorder, per-step condition, failure policy and overrides; a graph editor with a node catalog, validation in place, run panel with progress and cancel |
| IPC | `crates/deckpilot-ipc` | Protocol 10; capabilities are derived from the negotiated protocol; frames up to 64 KiB with paged answers for larger lists |
| Storage | `crates/deckpilot-storage` | Schema 9 (SQLCipher); profile documents in format 5; a `settings` key/value table (`active_profile`, `device_brightness`, …); module settings in `modules`; upgrades after a verified encrypted backup |
| Tests | `apps/agent/tests`, `crates/*/tests`, `apps/desktop/src/**/*.test.tsx`, `apps/desktop/src-tauri` | The real agent in process (`TestAgent`) or as the executable (`--mock-devices mk2 --no-usb`) with a simulated MK.2 and temporary data; upgrade fixtures of 0.5.0, 0.6.0 and schema 8; alpha.3-style databases in the plugin migration tests; component tests with a mocked command layer. The WebDriver harness (`apps/desktop/src-tauri/tests/marketplace_gui.rs`, Linux and Windows, not a required check) drives the Marketplace only |
| UI text | `apps/desktop/src` | English literals in the components; there is no localization layer |

### 1.2 Defects found by the audit

- **Dragging one key onto another throws.** `components/device-canvas.tsx`
  starts the drag with `data: { control }`, without `type: "control"`, so
  `dropOnControl` (`layout-editor/editing.ts`) treats it as a library entry
  and reads `target.type` of `undefined`. Moving or swapping keys by drag
  therefore does not work; the pure function is tested with the right
  shape, the component is not. Fixed with the folder editor work.
- **Deleting a page asks nothing.** "Delete page" in the page bar removes
  the page and the keys that open it at once (undo restores it until the
  layout is saved).
- **A duplicated profile is not announced to the agent** (`duplicate_profile`
  does not ask for a reload), so it is missing from the agent's profile list
  until the next reload.

## 2. Parity matrix

Evidence levels used in this document and in pull requests:
**physical** (a real Stream Deck, pressed by a person), **native** (the
packaged app observed in its window), **integration** (the real agent or the
app's commands with a simulated device), **simulated** (unit tests and
component tests). Nothing here has physical evidence yet.

### 2.1 P0

| Feature | Status | Home | What exists | What is missing |
| --- | --- | --- | --- | --- |
| **Folders**: own key layout, nesting, stable ids, no cycles, persisted with the profile | verified (integration, simulated) | core | `device_runtime.rs` tests `navigates_folders_and_ignores_unusable_input`, `back_returns_to_the_folder_and_folders_without_back_get_one`; `pages/tests.rs`; `apps/agent/tests/mock_devices.rs` | — |
| Folder key on the device enters it; Back key | verified (integration) | core | as above; automatic Back on the first free key | — |
| Create a folder by dragging onto an empty key or from the key menu | missing | core (app) | A page is added in the page bar, then "Open …" is dragged onto a key: two steps, and the new page is top level, not inside the page it was created from | "New folder" in the library and the key menu: creates the page inside the current one and binds the key in one undo step |
| Double-click a folder key to enter it in the editor | missing | core (app) | Shift+Enter and the context menu | Double-click |
| Custom title and icon of a folder | verified (simulated) | core | Binding label and appearance apply to folder keys like any other | — |
| Move actions in and out of folders | incomplete | core (app) | Cut and paste across pages; move and copy whole pages | Drag between keys is broken (1.2) |
| Rename; delete with confirmation | incomplete | core (app) | Rename in the page bar; delete without confirmation | Confirmation that names the keys and pages that go with it |
| Automatic return after an action runs | missing | core | — | A setting of the folder page |
| Navigation state preserved sensibly | incomplete | core | Reconnect and reload keep the page | The page a profile showed is forgotten when the profile changes and when the agent restarts |
| **Switch Profile**: a specific profile, next, previous, wraparound | missing | built-in module | Switching over IPC, from the app and the tray | An action; the editor lists the real profiles |
| Device selection, cross-device switching | missing | core | One active profile for every device | Per-device profiles do not exist: see [5.4](#54-switch-profile) and the owner decisions |
| Deleted or missing profile | missing | built-in module | `setActiveProfile` answers `NotFound` | The action fails with a clear message and the key shows it |
| **Key Logic**: single press, double press, hold with their own actions, timings, deterministic recognition, cancellation, persistence | verified (integration, simulated) | core | Engine tests incl. a property test over random sequences; `apps/agent/tests/gestures.rs`, `gesture_bindings.rs`, `gesture_e2e.rs`; the click-delay trade-off is documented in `gestures-and-work-session.md` §2.3 and shown in the editor | — |
| Key Logic editor with a visual slot per gesture, drag into slots | incomplete | core (app) | Three rows with a select each, overrides, label and timings | Slots that accept drops from the library; replace and remove in the slot |
| Visual feedback on the device | missing | core | — | A short mark on the key when a gesture is recognized |
| **Multi Action**: run in order, delays, per-step configuration, errors, progress, cancel, nested with bounded recursion | verified (integration) | core | Step and graph workflows; `apps/agent/tests/workflow_e2e.rs`; step editor with drag reorder | Shown as "workflow", not offered as an action that can be dropped on a key and edited there |
| Run supported actions in parallel | verified (integration) | core | Graph workflows (Parallel, Join) | Not available in the step list |
| Delay between steps | incomplete | core | Graph `delay` (ms); `general.wait` (1–300 s) in a step list | A delay row in the step list |
| **Page navigation**: go to a page | verified (integration) | core | `OpenPage` | — |
| Next page, previous page | missing | built-in module | — | Actions over the layout's top-level pages |
| Page indicator | missing | built-in module | The editor shows "Page n of m" | A key that shows it on the device |
| Page creation, deletion, reordering | verified (integration, simulated) | core | Page bar, page tree, `reorder_pages`, `move_page`, `copy_page` | — |
| Per-profile page state | incomplete | core | Per device, reset on profile change | See navigation state above |

### 2.2 P1

| Feature | Status | Home | What exists | What is missing |
| --- | --- | --- | --- | --- |
| **Multi Action Switch** | missing | core | Stateful keys exist for the Work Session only | A key with two or more states, each with its own workflow, title and icon; see [5.7](#57-multi-action-switch) |
| **Random Action** | missing | built-in module | — | See [5.8](#58-random-action) |
| **Website** | verified (simulated) | built-in module (`system`) | `system.open-url`: `http`, `https`, `mailto` | — |
| **Open** (file, folder) | verified (simulated) | built-in module (`system`) | `system.open-file`, `system.open-directory`, inside bound locations | — |
| **Open Application** | verified (simulated), platform | built-in module (`system`) | `system.open-app` with a bound application and arguments | — |
| **Close Application** | missing, platform | built-in module (`system`) | `system.process-status` reports whether it runs | A graceful quit request; see [5.9](#59-system-actions) |
| **Hotkey**, **Hotkey Switch**, **Text** | missing, platform | built-in module | — | Keystroke injection; see [5.10](#510-hotkey-hotkey-switch-and-text) |
| **Multimedia** | missing, platform | built-in module | — | Media keys; see 5.10 |
| **Device: brightness** | incomplete | built-in module | A global setting and `setBrightness` | An action (set, step up, step down) |
| **Device: sleep, wake, timer** | missing | core + built-in module | — | A sleep state of the device in the agent; see [5.11](#511-device-actions) |
| **Device status** | incomplete | core | The app's Devices page and dashboard | A key face |
| Virtual device toggle | missing | — | Simulated devices are a development tool | Not planned: there is no virtual device for users |

### 2.3 P2

| Feature | Status | Home | What is missing |
| --- | --- | --- | --- |
| **Soundboard**: play, stop, modes, volume, fades, output device | missing, platform | built-in module | Everything; see [5.12](#512-soundboard) |
| Advanced state configuration | missing | core | Follows the Multi Action Switch |

### 2.4 Action Library

| Feature | Status | What exists | What is missing |
| --- | --- | --- | --- |
| One searchable catalog with categories | incomplete | Two lists: the profile's instances (Action Library, the editor's library) and the types (when creating an action) | One catalog in the layout editor that offers types as well as saved actions |
| Expand and collapse, search | incomplete | Search in the Action Library and the type chooser | Collapsible categories; search in the editor's library |
| Drag and drop | incomplete | Saved actions, workflows and navigation targets onto keys | Types onto keys (creates the action and binds it) |
| Keyboard navigation | incomplete | Canvas and library entries are focusable; `KeyboardSensor` | Arrow keys in the catalog |
| Platform and permission indicators, disabled states | incomplete | Plugin actions that cannot run are shown disabled with the reason | The same for built-in types (unsupported platform, permission not given) |
| Inspector adapts to the action; nested actions editable in place | incomplete | Parameter overrides per binding; the gesture editor | Editing the action's own parameters from the key |

### 2.5 Defaults and compatibility

| Requirement | Status | Evidence or plan |
| --- | --- | --- |
| Essential actions offline on a fresh install | verified for what exists | Built-in modules are compiled in; nothing is downloaded. The new modules are built in too |
| No registry needed at first launch | verified | Released builds have plugins off |
| Optional integrations not installed automatically; removed plugins not reinstalled | verified (integration) | `apps/agent/tests/plugins_*.rs` (tombstones) |
| Existing profiles, pages, layouts, bindings, workflows, variables, action ids kept | by design | See [4](#4-compatibility-rules); each pull request tests an alpha.3-style database |

## 3. Core or plugin

| Feature | Home | Why |
| --- | --- | --- |
| Folders, pages, page navigation | core | The page stack is the device runtime's |
| Switch Profile | built-in module `deck` | Changes agent state; applied by the agent like a project switch |
| Key Logic | core | The gesture engine |
| Multi Action | core | The workflow engine; a presentation, not a second executor |
| Multi Action Switch, Random Action | core / built-in module `deck` | They start workflows and keep key state in the agent |
| Device actions | built-in module `deck` | Raw device access is never grantable to a plugin |
| Website, Open, Open Application, Close Application | built-in module `system` | Launching is already core code (`deckpilot_core::launch`); `module-migration.md` §3.2 keeps `system` built in |
| Hotkey, Hotkey Switch, Text, Multimedia | built-in module `input` | Injecting input into other applications; the most sensitive power in this document |
| Soundboard | built-in module `soundboard` | Audio output and file reads |

**What it would take to make System, Input and Soundboard default plugins
later** (an owner decision, not planned here). Each would need a host
capability that does one narrow thing with the host keeping every
security-relevant decision, in the style of G11 and G12 of
`module-migration.md`:

- `launch.resource`: open a resource the user bound on this computer
  (already proposed as G12). Never a path or a command from the plugin.
- `input.sendShortcut`: send one key combination from a list the user
  approved for that plugin when granting, to the foreground application
  only, rate limited, never text and never a sequence the plugin builds at
  run time. Free text typing has no narrow form: a plugin that can type
  can run anything a terminal accepts. **Not offered.**
- `media.key`: play/pause, next, previous, volume up/down, mute. Narrow
  and low risk.
- `audio.play`: play a file the user picked through a host file dialog
  (the plugin gets a handle, never a path), with host-side decoding,
  volume limits and a stop that always works.

Until such capabilities are designed, reviewed and consented to in the
grant dialog, these stay built-in modules, where the code is part of the
signed application and the switch that enables them is a setting of this
computer.

## 4. Compatibility rules

These rules decide the designs below.

1. **No change to how existing data is read.** `BindingTarget`,
   `Trigger`, `NodeKind` and `ParameterValue` keep their variants. A build
   that knows nothing of the new features (0.6.0-alpha.3, same schema 9)
   must still open a database written by a newer build: a new enum variant
   in a profile document would make the whole profile unreadable there.
2. **New behaviour arrives as action types.** A type an older build does
   not know is shown as unavailable and left alone; the instance, its
   parameters and every key bound to it survive a round trip through the
   older build.
3. **New fields are optional and omitted when unset** (as `parent`,
   `gestures` and `appearance` were), so documents without them serialize
   exactly as before. An older build ignores them and drops them when it
   saves that layout; nothing else is lost.
4. **Runtime state goes in the `settings` table** (key/value, schema 9).
   No schema migration is needed for anything in P0. A feature that needs
   its own tables gets a migration with the usual tests (wrong key and a
   newer schema leave the file byte-identical).
5. **No IPC change where the existing requests suffice.** The app writes
   configuration to the store and asks the agent to reload, so new action
   types and layout fields need no new request. A new request takes
   protocol 11 with a `Capability`, and the app hides what an older agent
   cannot do.

## 5. Designs

### 5.1 The `deck` module and deck requests

A new built-in module `deck` (`modules/deck`) provides the actions that
act on the device and the agent rather than on the computer: page
navigation, profile switching and, later, device actions. It is enabled by
default and works offline.

A handler cannot reach the device runtime. As with `ContextEffect` for
projects, it *asks*: `ActionResult` gains `requests: Vec<DeckRequest>`
(never serialized: it does not cross IPC, is not stored in history and
cannot be produced by a plugin, whose results come back as JSON). The
registry lets a request through only for a module that holds the new
`Capability::DeckControl`.

| Request | Applied by | Effect |
| --- | --- | --- |
| `NextPage { wrap }`, `PreviousPage { wrap }` | `Runtime` | Moves the pressing device along the top-level pages |
| `SwitchProfile { target, wrap }` | agent | Activates a profile, as `setActiveProfile` does |

A request needs the device whose key ran the action. Run from the app
("Run" in the Action Library) or from a workflow without a device, a
navigation request fails with `no-device` rather than guessing a device;
a profile switch needs no device and works everywhere an action runs on
the service task.

### 5.2 Pages and folders

DeckPilot keeps one tree of pages per layout. In the words used by the
editor and this document:

- A **page** is a top-level page of the layout: the home page and every
  page without a parent. They have an order (the layout's page order, home
  first). *Next page* and *Previous page* move along them.
- A **folder** is a page with a parent. It is entered from a key of its
  parent and left with Back.

So an existing layout keeps its behaviour exactly: pages opened from the
home page are folders (stored or inferred), and nothing becomes part of
the next/previous sequence by itself. Inside a folder, *Next page* and
*Previous page* are relative to the top-level page the folder belongs to,
and leave the folder.

- `deck.next-page`, `deck.previous-page`: parameter `wrapAround`
  (default on). Without wraparound the last page stays and the action
  reports `no-more-pages`, which is not a failure of the key. With one
  page the action does nothing.
- *Go to page* is the existing `OpenPage` binding, unchanged.
- `deck.page-indicator` does nothing when pressed. Key labels gain
  `{page.number}`, `{page.count}` and `{page.name}` (the position of the
  top-level page the device shows, among the layout's top-level pages),
  and the indicator's default label is `{page.number}/{page.count}`.
  Any key can use the placeholders.
- Adding a page never changes existing keys. The editor offers "add
  navigation keys" for a new page, which uses free keys only and says so
  when none is free.

**Automatic return.** `LayoutPage.autoReturn` (optional, omitted when
false): after a key of the page ran an action or started a workflow, the
device returns to the folder's parent, as Back would. It applies to
actions and workflows only (not to navigation keys), whatever the action's
result, at the moment the key is handled. It is a property of the folder,
so it is set once and applies to every key in it.

**Navigation state.** The runtime keeps, per profile and device, the page
stack a device showed when the profile was left, and restores it when the
profile becomes active again, if every page still exists. The stacks of
the active profile are saved in `settings` (`navigation`) and restored
after an agent restart with the same check. Disconnect, reconnect and
reload behave as before.

### 5.3 Folders in the editor

- "New folder" in the library (draggable) and in the key menu creates a
  page inside the current one, named "Folder" (then "Folder 2", …), and
  binds the key to open it, as one undo step. An occupied key is refused
  by the drag and offered as "Replace with a new folder" in the menu.
- Double-click on a folder key enters it; the breadcrumb and Back in the
  page bar leave it.
- Deleting a page asks first, naming the keys that open it and the pages
  inside it that are deleted with it.
- Drag between keys moves or swaps (defect 1.2).
- A folder's title and icon are its key's label and appearance.

### 5.4 Switch Profile

`deck.switch-profile`: `mode` (`profile`, `next`, `previous`), `profile`
(a new parameter kind `profile`, drawn as a list of the real profiles and
stored as the profile's id in a text value), `wrapAround`.

- Next and previous follow the order the app lists profiles in.
- A profile that no longer exists fails with `profile-not-found`; the
  action's check reports it when the action is saved or shown.
- Switching invalidates pending gestures and redraws every device at once
  (the path `setActiveProfile` already takes); the app follows through the
  state event it already listens to.

**Devices.** The agent has one active profile; every device shows it with
the layout for its model. A per-device profile (two decks on different
profiles) needs a session per device in the runtime, a per-device active
project question and a way to show both in the app and the tray. It is not
part of this work; see the owner decisions. Until then the action has no
device parameter, and says in its description that every device follows.

### 5.5 Key Logic

The engine, the data model and the persistence exist and are tested. The
work is the editor: one slot per gesture that shows what is in it and
accepts a drop from the library (replace), with remove and "edit action"
in the slot, the timings next to the gestures they belong to, and the
existing note on when a single press fires.

The trade-off stays as documented: with a double press bound, a single
press is reported only after the double-press interval (300 ms by default)
has passed without a second press; with only a hold bound, on release;
with neither, at once. A key never runs its single press when a double
press was made.

### 5.6 Multi Action

A Multi Action is a step workflow. No executor is added. The work is
presentation: "Multi Action" in the catalog creates a workflow and binds
it; its steps are edited in a sequence editor opened from the key (the
existing step editor, with a delay row and with errors and progress of the
last run). Parallel steps remain the graph editor's.

### 5.7 Multi Action Switch

Owner-visible behaviour to settle before coding:

- Two to eight states; each has a workflow or action, and optionally a
  title and icon.
- A press runs the current state's target. The key advances **when the
  run succeeds**; a failed or cancelled run leaves the state (option:
  "advance anyway").
- The state is per key binding, kept in `settings`, restored after a
  restart (option: "start at the first state after a restart").
- Reset: long press (option), or an action `deck.reset-switch`.

It needs a representation that an older build can read (rule 1), so it is
an action type whose parameters name the states, with the agent starting
the runs through a deck request. Designed in detail with its pull request.

### 5.8 Random Action

`deck.random`: a list of candidates (saved actions and workflows) with
optional weights and "never the same twice in a row". Uniform by default.
An empty list fails with `no-candidates` when saved and when run. The
random source is injected into the handler, so tests are deterministic.

### 5.9 System actions

Additive changes to `system` (the module-migration work audits the same
module; sequence the merges):

- `system.close-app`: asks a bound application to quit (macOS: an Apple
  event `quit`; Windows: `WM_CLOSE` to its top-level windows; Linux: the
  desktop's close request where available, otherwise `SIGTERM`). Never
  `SIGKILL`/`TerminateProcess`. Fails with `not-running` when it is not
  running. Unsaved-changes dialogs stay the application's.

### 5.10 Hotkey, Hotkey Switch and Text

A new built-in module `input`, **off until the user allows it on this
computer** (`allowInput`, machine-specific, never exported or imported,
like `allowCommands`).

| | macOS | Windows | Linux X11 | Linux Wayland |
| --- | --- | --- | --- | --- |
| Mechanism | `CGEvent` | `SendInput` | XTest | none that works everywhere without a portal session |
| Permission | Accessibility, granted by the user in System Settings | none; cannot reach elevated windows (UIPI) | none | — |
| Status shown | granted / not granted, with a button that opens the settings pane | "does not reach applications running as administrator" | available | **unavailable** |

- The agent never asks for elevation and never retries around a refusal.
- Text is at most 2,000 characters, typed at a bounded rate; it is a
  sensitive parameter: never in logs, execution history or events.
- Hotkey Switch alternates two shortcuts; its state is the key's.
- Multimedia (play/pause, next, previous, stop, volume up/down, mute) uses
  the platform's media keys through the same mechanism and the same
  setting.

Crate choice and licence review happen in that pull request. Candidates
must be MIT, Apache-2.0, BSD or Zlib, maintained, and build on the three
CI platforms without system packages the CI images lack. The workspace
forbids `unsafe` in its own code, so the platform calls come from a
dependency. The repository has no automated licence check today (no
`cargo-deny` or similar); the review is by hand and recorded in the pull
request.

### 5.11 Device actions

- `deck.brightness`: set, step up, step down; changes the same setting the
  app changes, only for devices with the `brightness` capability.
- Sleep and wake: the agent blanks a device (brightness 0 and cleared
  keys) and restores it on the next press, which is swallowed. A sleep
  timer is a setting of this computer. Needs agent state and a status
  field, so it comes with protocol 11.
- Actions a device cannot do are listed as unavailable for it, with the
  reason, from its capability descriptor.

### 5.12 Soundboard

A built-in module `soundboard`: `soundboard.play` (file, mode
play/stop, play/restart, play/overlap, loop/stop; volume; fade in and
out; output device) and `soundboard.stop`. Playback runs on its own
thread with a command channel; the service task only sends commands, and
stop and agent shutdown always end playback.

Formats depend on a licence decision. WAV and FLAC decoders exist under
MIT/Apache-2.0. The maintained pure-Rust decoders for MP3 and AAC/M4A
(`symphonia`) are MPL-2.0, a file-level copyleft licence that is
compatible with shipping inside an Apache-2.0 application but is not
"permissive"; the MP3 patents have expired, AAC-LC decoding is still
patent-encumbered in some jurisdictions. The alternative for M4A is the
platform decoder (AVFoundation, Media Foundation), which is per-platform
code. See the owner decisions.

## 6. Owner decisions

| # | Decision | Default taken until decided |
| --- | --- | --- |
| O1 | Should System, Input and Soundboard become default plugins once narrow host capabilities exist (section 3)? | Built-in modules |
| O2 | Per-device profiles (two devices on different profiles at once) | Not built; Switch Profile changes the profile of every device |
| O3 | May MPL-2.0 dependencies ship in DeckPilot (MP3 and M4A decoding)? | No: the soundboard starts with WAV and FLAC |
| O4 | M4A/AAC through platform decoders, given the patent situation | Not built |
| O5 | Multi Action Switch details (5.7): advance on failure, reset, state after a restart | As written in 5.7 |
| O6 | Keystroke injection on Wayland through the RemoteDesktop portal (asks the user per session) | Marked unavailable |

## 7. Test plan

Every pull request names its evidence level. The scenario the work must
cover by the end, and where each step is tested:

| Step | Layer |
| --- | --- |
| Drag a folder onto a key; open it by double-click; add an action inside; navigate back | simulated (component tests of the editor) |
| Press the folder key on the device; device navigation | integration (agent executable, simulated MK.2) |
| Switch profiles | integration |
| Configure and execute the three gestures | simulated (editor), integration (`gesture_e2e.rs`) |
| Multi Action, Multi Action Switch, Random Action | integration |
| Navigate pages | integration |
| System actions | simulated (recording launcher), integration |
| Play and stop audio | simulated (recording output), native where a device exists |
| Restart the agent; state persistence | integration |
| Reopen the app; configuration from 0.6.0-alpha.3 | integration (app commands, alpha.3-style database) |
| No duplicate agent processes | integration (`apps/agent/tests/lifecycle.rs`, instance lock) |

Physical checks are the owner's: they need the installed app to be stopped
and a build of the branch run against a copy of the configuration or a
temporary one. No automated test opens a connected Stream Deck (`--no-usb`).
