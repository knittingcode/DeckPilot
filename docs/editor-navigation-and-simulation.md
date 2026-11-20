# Editor navigation and simulated presses

The Device Editor shows a live preview of the device. Two things let that
preview behave like the device without the editor guessing or executing
anything itself:

1. **Editor navigation metadata**: what pressing a key does to the view
   (open a folder, go back, turn the page, switch the layout), declared as
   data and resolved per control in the preview.
2. **Simulated presses** (`simulatePress`, IPC protocol 14): the app asks
   the agent to press a control as the user would, and the agent runs it
   through its own input pipeline.
3. **Live preview** (IPC protocol 16, section 3): the selected device shows
   and runs the editor's unsaved draft, and the editor and the device follow
   each other's navigation.

Both are backend features; the editor's context menu and double-click
navigation build on them.

## 1. Editor navigation

### Declaration

An action type may carry `editorNavigation` (core `ActionType`,
`deckpilot_core::editor_navigation`):

```json
{ "operation": "layout.switch", "targetParameter": "layout", "pageParameter": "page" }
```

| `operation` | Target | Meaning in the editor |
| ----------- | ------ | --------------------- |
| `folder.open` | `targetParameter`: a page id | show that page |
| `folder.back` | none | back to the page's folder (or the page that opened it) |
| `page.next` | none | the next page of the layout |
| `page.previous` | none | the previous page |
| `layout.switch` | `targetParameter`: a layout id; optional `pageParameter`: its page | show that layout |
| `layout.back` | none | the layout shown before |

The operations are an **allowlist** (`NavigationOperation`); nothing else is
accepted. `targetParameter` is required exactly for `folder.open` and
`layout.switch` and must name one of the action's own **text** parameters;
`pageParameter` is allowed only for `layout.switch`
(`EditorNavigation::check`). The declaration is data only: the editor maps
the operation to its own navigation code and never runs the action, and a
plugin is never called on an editor double-click.

Built-in declarations: `deck.next-page` (`page.next`), `deck.previous-page`
(`page.previous`), `deck.switch-layout` (`layout.switch`, target `layout`,
page `page`) and `deck.previous-layout` (`layout.back`). Binding targets
need no declaration: `openPage` is `folder.open`, `back` is `folder.back`.

Plugins declare it per action in the manifest
(`contributes.actions[].editorNavigation`, plugin API 1.7;
[marketplace/sdk/manifest.md](marketplace/sdk/manifest.md#editor-navigation)).
A plugin can only describe its own actions (the entry is part of the
action), the manifest validator and the agent check it against the
allowlist and the action's parameters, and the app checks it again when it
registers the plugin's action types (an invalid declaration is dropped).

### Where the editor reads it

- **The action catalog** (`list_action_types`, and plugin action
  definitions from the agent): `editorNavigation` on the action type,
  omitted when the action does not navigate. Older apps ignore the field.
- **Preview faces** (`preview_layout`): each `ControlFace` may carry
  `navigation`, resolved for that control on that page
  (`deckpilot_core::control_navigation`): the shown binding's target, or
  the action's declared operation with its target read from the instance's
  parameters and the binding's overrides. For example
  `{"operation": "folder.open", "page": "…"}`,
  `{"operation": "layout.switch", "layout": "…", "page": "…"}`.
- **The automatic Back key.** A folder page without a way back gets an
  automatic Back on its first free key with a display. It is not a binding
  of the layout, so an editor that reads bindings only cannot know it; its
  face carries `{"operation": "folder.back", "automatic": true}`.
- **The protected Back key.** On a folder page with the reserved Back
  ([data-model.md](data-model.md#the-protected-back-key)) the first key
  with a display is DeckPilot's own Back, whatever is stored on it; its
  face carries `{"operation": "folder.back", "automatic": true,
  "system": true}` (`system` is optional and left out elsewhere).

A control whose action's target is empty or not an id has no
`navigation`.

## 2. Simulated presses

### Request

```json
{ "type": "simulatePress", "device": {"model": "…", "serial": "…"},
  "profile": "…", "layout": "…", "page": "…",
  "control": {"kind": "key", "index": 3}, "kind": "double", "confirmed": false }
```

`kind` is `single`, `double` or `long`. The answer is
`{"type": "simulatedPress", "outcome": …}` with one of:

| `status` | Fields | Meaning |
| -------- | ------ | ------- |
| `ran` | `dispatched`: `press`, `release`, `click`, `doubleClick`, `longPress` | the press went through the engine; the edges and gestures dispatched, in order |
| `confirmationRequired` | `actionNames`, `reason` | nothing ran; ask the user and send it again with `confirmed: true` |
| `refused` | `code`, `message` | nothing ran |

Refusal codes: `locked`, `noActiveProfile`, `deviceNotConnected`,
`wrongProfile`, `wrongLayout`, `wrongPage` (the device does not show what
the app shows: the preview must follow the device first),
`unknownControl`, `incompatibleControl` (the control cannot be pressed),
`failed`. The app's Tauri command `simulate_press(device, profile, layout,
page, control, kind, confirmed)` returns the same shape and adds
`agentNotRunning` and `unsupported` (an agent before protocol 14, which is
never asked).

### How a press is synthesized

- **The real pipeline.** The agent feeds press and release edges into a
  `GestureEngine` and dispatches what it reports with the same code as a
  device's input: a key without gesture bindings gets its press and release;
  a gesture key gets what the engine recognizes with the key's own timings
  (the page's per-key settings over this computer's defaults). A double
  press on a key with click and double-click bindings fires the double
  click, never two clicks; a long press fires at the key's threshold. There
  is no second execution engine.
- **A virtual clock.** The edges carry instants on a clock of their own:
  single = press, release 40 ms later; double = two such taps 40 ms apart
  (inside the shortest double-click window, 150 ms); long = press, release
  40 ms after the key's long-press time. The engine is then advanced past
  the double-click window, so a pending click is decided. Nothing sleeps
  and the service task is never blocked; the engine times gestures from
  those instants exactly as for a held key.
- **A lane of its own.** Each press uses a new engine and a connection id
  of its own (the device session with bit 63 set). A physical sequence of
  the same key is neither joined, ended nor cancelled, and a stateful key's
  captured press state is kept apart. Only what any page change does
  applies: a simulated press that turns the page drops physical gestures
  recognized on the old page, as a page change from the app does.
- **Nothing is written to the device** but the key images the binding
  itself changes. The USB backend never sees simulated input; the mock-only
  `simulateInput` request is unchanged and still refused without
  `--mock-devices`.

### Checks

Refused while DeckPilot is locked (no unlock prompt), for a device that is
not connected, unless the device shows exactly the requested profile,
layout and page, and for a control the model does not have or that cannot
be pressed.

### Origin and permissions

Bindings run as `RunOrigin::Simulated`, and workflows they start are
recorded with the trigger `simulated` (older readers see `other`; the
workflow cause is `simulated`).

**Simulated is not user initiated.** The user asked the app, not the
device, and the owner's requirement is that simulation must not bypass any
permission. So whatever may act only on the user's direct request stays
refused: plugin dialogs and `url.open` (plugins see the trigger `other`),
in the press itself and in workflows it starts. Deck requests (pages,
layouts, profiles, brightness) are applied as for a key.

### Confirmation

A press whose bindings may change something outside DeckPilot needs
`confirmed`. The agent looks only at what this press of this key can
dispatch (its press and release bindings in raw mode, else the gesture the
kind of press can end in) and follows it into workflows (graph nodes,
steps, sub-workflows and loops), Random Action candidates and Multi Action
Switch states. Needing confirmation:

- an action whose effect is `modifies` (`NodeSpec.effect`): for example
  *Run command*, Docker up/down/restart, *HTTP request* other than `GET` or
  `HEAD`, and every plugin action that declares `modifies` (a `git push`);
- except the built-in `deck` and `projects` modules, whose changes stay in
  DeckPilot (page, layout, profile, brightness, active project);
- an action type the agent does not know (nothing says it is harmless);
- a stateful (work session) key, which may run the session's workflows.

`actionNames` lists them (a workflow's name before its steps), at most 32.
Confirmation never bypasses the settings that gate those actions on this
computer: *Allow running commands* (`allowCommands`), Docker's
`allow_changes`, plugin permission grants and origins apply to a simulated
press exactly as to a real one, so a confirmed *Run command* still fails
with `commands-disabled` while commands are off. Plugin permission tiers are
not part of the decision: a plugin's declared effect is.

## 3. Live preview and navigation sync

While the Device Editor shows a layout of the **active** profile for a
connected device, that device shows **and runs** the editor's unsaved
draft (IPC protocol 16, capability `livePreview`,
`deckpilot_ipc::live_preview`). Three things are kept apart:

| | Owner | Saved |
| --- | --- | --- |
| Saved configuration | the database | yes |
| Working configuration (the draft) | the editor; mirrored in the agent's memory for one device (`Runtime::set_working_layout`) | never |
| Runtime navigation (page stack, layout, layout history, Auto Exit) | the agent, per device and profile | as before; draft-only pages are never remembered |

The database is never the preview channel and nothing is saved
automatically. A device whose working layout is for the layout it shows
resolves to it in `Runtime::layout_of`, the one place every face, binding,
override, gesture, folder key, protected Back and Auto Exit setting is read
from: so what the device draws (the same renderer and device defaults as the
Canvas; MK.2: black LCD, white text) and what its keys run are the draft's.

### Scope: one device

A session is for exactly one device: the one the editor is for (the
toolbar's device). Other devices, also ones that show the same layout (the
same profile, or the same shared layout in another profile), keep the saved
configuration until Save; saving propagates as before. A second window
cannot preview on a device that has a session (`livePreview.deviceBusy`).
No session runs while the edited profile is not the active one (the editor
says changes reach the device when saved), or with an agent before
protocol 16 (likewise).

### Protocol

| Request | Answer | |
| --- | --- | --- |
| `beginLivePreview {device, profile, layout, navigation?}` | `livePreviewStarted {session, navigation}` | the working layout starts as the saved one; one session per connection |
| `applyLivePreview {session, revision, part, last}` | `livePreviewApplied {session, revision, applied}` | a revision in parts (`LayoutPatch`: the header with the page order, then the pages that changed); applied together with the last part |
| `navigateLivePreview {session, navigation}` | `livePreviewNavigated {session, navigation}` | `show {layout, stack}` or `layoutBack`; nothing runs |
| `endLivePreview {session}` | `ok` | the device shows the saved configuration |

Pushed to the owner only (`livePreview` messages): `navigated {session,
navigation}` when the device's navigation changes by itself (a key, a folder
key, the protected Back, a page key, Switch layout, Back to previous layout,
Auto Exit, a reload or revert that removed the page shown, a profile switch)
and `ended {session, reason}` (`locked`, `replaced`). A navigation carries
`profile`, `layout`, the page `stack`, `connected`, `inScope` (the device
shows the session's layout: otherwise the preview is suspended) and `seq`,
which grows with every state reported, so an answer and an event that cross
each other can be ordered. Refusal reasons are in
`deckpilot_ipc::live_preview::reason` (`staleRevision`, `unknownSession`,
`deviceBusy`, `invalidLayout`, `unknownPage`, `wrongModel`,
`inactiveProfile`, `tooLarge`).

**Why parts, not the whole draft.** A layout can be larger than one IPC
message (64 KiB). The editor's edits are immutable updates, so the pages an
edit does not touch stay identical; the app's backend
(`commands/live_preview.rs`) sends the header and the changed pages only,
split into parts of at most 48 KiB of pages, against what the agent applied
last; the first revision of a session (and after a reconnection) is sent in
full. A single page larger than a part is refused (`tooLarge`; the editor
says the live preview is paused).

### Revisions and coalescing

- Revisions only go forward (`RevisionGate`): an older or repeated revision
  is refused (`staleRevision`) and changes nothing; parts of a revision are
  collected and applied atomically; a newer revision drops what an older one
  collected (gaps are fine). At most 4 MiB are staged.
- The editor sends its draft 40 ms after the last edit, at least every
  120 ms while edits keep coming (a colour drag); navigation is never
  delayed, and a pending draft is always sent before it (a page that exists
  only in the draft is on the agent first) and before Save.
- The app's backend keeps one request in flight: drafts that arrive
  meanwhile replace each other, only the latest is sent.
- The agent redraws only keys whose face changed (face diff and frame cache);
  the device thread writes queued images once per key, the latest
  (`devices.rs`, `coalesce`), so the USB link is never flooded. Applying and
  navigating never write to the database.

### Navigation both ways

- Editor → device: a double-click on a folder or Back key, the page
  navigation, the breadcrumbs, a new page shown: the editor's page stack is
  sent as `show`. The device's runtime takes it as is; nothing is run, and
  entering a folder this way starts no Auto Exit countdown (entering it by a
  key does). Choosing another layout in the editor moves the device to it
  (as Switch layout, so the device's Back to previous layout returns).
- Device → editor: the editor shows the page and trail of every `navigated`
  in the editor's layout. When the device switches to another layout of the
  profile the editor follows it if nothing is unsaved; with unsaved changes
  it stays (the preview is suspended, the notice above the device offers to
  open the other layout, asking first) and the device's Back to previous
  layout brings the draft back.
- Opening the editor (or choosing another device) shows the layout and page
  the device shows.

### Save, Revert, Undo

- **Save** stores the draft and the agent reloads; the working layout equals
  the saved one, so no key is drawn again, the device stays on its page, and
  the agent does not restart.
- **Revert** and **Undo** are new revisions. Where the page shown no longer
  exists, the editor and the device go to the nearest page they came
  through that still exists (the folder it was in), else the start page.
  The same rule now applies when the saved configuration is reloaded (a page
  deleted elsewhere) and when a device reconnects.

### Ownership and cleanup

The session belongs to the connection that began it (the app's backend
holds it on a connection of its own, like the live key states).

| Event | What happens |
| --- | --- |
| the editor closes, switches device, layout or profile (after asking over unsaved changes), or ends the session | the device shows the saved configuration, on its page or the nearest one |
| the app quits or crashes, the main window reloads or closes | the connection closes: the same |
| the device is unplugged | the session stays; plugged in again, it shows the draft at its page |
| the agent restarts | the preview is memory only: the device shows the saved configuration; the app connects again, begins the session again and sends the latest draft in full and the editor's place |
| DeckPilot locks | every session ends (`ended {locked}`); the device shows "Locked" |
| another profile becomes active | the session is suspended and resumes when the profile is back |

Unsaved changes are never discarded without asking: switching the layout,
profile or device, leaving the Device Editor for another page, closing the
main window (Windows, Linux; on macOS it hides and keeps the editor) and
quitting (⌘Q) ask with the app's destructive dialog.

### Execution

The draft runs on physical presses exactly as a saved layout would:
unsaved gesture assignments and binding overrides run, through the same
engine, plugin executor, permission and compatibility checks. Selecting,
editing and dragging in the editor never run anything; a double-click is
navigation only. **Test action** (section 2) presses the working
configuration while the preview is live, without asking about unsaved
changes. Plugin key states decorate the draft's faces as they decorate saved
ones, so a plugin's update never brings back an older title or look.

### Checks

`inspectMockDevice {device}` (protocol 16, refused without
`--mock-devices`) answers a simulated device's navigation and, per control,
a hash of its frame and how often it was written. `scripts/live-check.mjs`
uses it with the layout probe (`live`, `dblclick`, `edit` commands) against
a running development session.

## Compatibility

Protocol 16 adds the capability `livePreview` (section 3); the client
refuses its requests locally below it, the agent refuses them from an older
connection, and the editor then says changes reach the device when saved.

Protocol 14 adds the capability `simulateInput`; the client refuses
`simulatePress` locally below it and the agent refuses it from an older
connection. Plugin API 1.7 adds `editorNavigation`; a manifest using it
needs `^1.7`. See [compatibility.md](compatibility.md).

## Tests

- core: `editor_navigation` (allowlist, checks),
  `control_navigation_names_folders_back_and_the_automatic_back`;
  deck module: `navigation_actions_declare_their_editor_navigation`.
- manifest: `editor_navigation_is_plugin_api_1_7_and_an_allowlist`.
- IPC: `simulated_presses_are_protocol_14`, the `simulation` module;
  peers: `a_protocol_13_agent_is_never_asked_to_simulate_a_press`.
- agent (`tests/simulate_press.rs`, simulated MK.2 only): recognition like
  physical input, thresholds, workflows, refusals, confirmation with the
  settings still applying, physical input undisturbed, a protocol 13 app;
  `plugins_e2e` `pages_open_only_in_runs_the_user_started` (a simulated
  press is refused `url.open`); plugin engine unit tests.
- desktop: `previews_and_the_catalog_carry_editor_navigation`,
  `editor_navigation_reaches_the_catalog_only_when_it_checks_out`,
  `simulated_presses_answer_ran_confirmation_or_refused`.
- live preview: IPC `live_preview` module (patches, chunking, revisions),
  `live_preview_is_protocol_16`, peers
  `a_protocol_15_agent_is_never_asked_for_a_live_preview`; core
  `device_runtime::working_tests` (working layouts per device, editor
  navigation, both directions, nearest page, save); agent
  `tests/live_preview.rs` (simulated MK.2s: frames equal the renderer's,
  rapid edits, stale revisions, presses run the draft, no echo, save draws
  nothing, revert, connection drop, agent restart, isolation and busy,
  no database writes, lock, mock inspection, protocol 15) and
  `plugins_e2e` `an_unsaved_binding_runs_a_plugin_action_with_its_permission_checks`;
  desktop `commands::live_preview` (`Outbox`); frontend
  `use-live-preview.test.tsx`, `preview-interaction.test.tsx` (live preview),
  `unsaved-guards.test.tsx`, `use-layout-editing.test.tsx`.
