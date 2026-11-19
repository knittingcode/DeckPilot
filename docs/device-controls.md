# Device controls and action compatibility

Which physical controls an action can be bound to, how DeckPilot decides
whether a binding can run, and what every layer does with one that cannot.
*Design record*: the contract of the device-capability redesign. Every
section is implemented. Where the code differs, the code and its tests
win.

## 1. Control model (`deckpilot-device`)

A model is described by its `DeviceCapabilities` (`keys`, `encoders`,
`touch`, `brightness`); a control by its `ControlId` (`key`, `encoder`,
`touchZone`, with an index) and its `ControlKind`. What one control can do
is derived from the descriptor, never from a model's name:

```rust
pub struct ControlCapabilities {
    pub kind: ControlKind,
    /// Hardware events the device reports for this control.
    pub inputs: BTreeSet<InputCapability>,
    /// Gestures DeckPilot derives from the native inputs.
    pub derived: BTreeSet<InputCapability>,
    /// Output on the control itself.
    pub display: Option<DisplayCapability>, // image spec, title
}

pub enum InputCapability {        // serialized in kebab case
    Press, Release,               // keys, pressable encoders
    RotateClockwise, RotateCounterClockwise,
    Tap, LongTouch, Swipe,        // touch zones (TouchSurface.gestures)
    Click, DoubleClick, LongPress, // derived by the gesture engine
}

impl DeviceCapabilities {
    pub fn control_capabilities(&self, id: ControlId) -> Option<ControlCapabilities>;
}
```

- **Key**: inputs `press`, `release`; derived `click`, `double-click`,
  `long-press`; a display (image and title) when the grid has an image
  spec.
- **Encoder**: inputs `rotate-clockwise`, `rotate-counter-clockwise`, and
  `press`, `release` only when it is `pressable`; the derived gestures
  only when pressable; no display.
- **Touch zone**: the inputs of its surface's gestures (`Tap` → `tap`,
  `LongPress` → `long-touch`, `Swipe` → `swipe`); nothing derived; a
  display when the surface has an image spec.
- A derived gesture is never a hardware input. `None`: the model has no
  such control.

Every binding `Trigger` maps to exactly one input
(`Trigger::input_capability`): `press` → `press`, `release` → `release`,
`rotateClockwise` → `rotate-clockwise`, `rotateCounterClockwise` →
`rotate-counter-clockwise`, `touch { tap }` → `tap`, `touch { longPress }` →
`long-touch`, `touch { swipe }` → `swipe`, `click` → `click`, `doubleClick`
→ `double-click`, `longPress` → `long-press`.

| Model | Keys | Encoders | Touch zones |
| --- | --- | --- | --- |
| Stream Deck MK.2 | 15, press/release + gestures, 72 px display | – | – |
| Stream Deck XL | 32, press/release + gestures, 96 px display | – | – |
| Stream Deck + | 8, press/release + gestures, 120 px display | 4, both rotations, pressable (press/release + gestures) | 4, tap, long touch, swipe, 200 × 100 px display |

## 2. Action compatibility (`deckpilot-core`)

`ActionType` has an optional `controls` field (serde default, not sent
when `None`), so stored definitions, plugin definitions and IPC payloads
from before it decode unchanged:

```rust
/// Per control kind, the inputs the action can be bound to; a kind that is
/// absent is not supported. Serialized as the map itself:
/// {"key": ["press", "click"], "encoder": ["rotate-clockwise"]}
pub struct ActionControls {
    pub bindings: BTreeMap<ControlKind, BTreeSet<InputCapability>>,
}
pub struct ActionType { …, pub controls: Option<ActionControls> }
```

**Legacy default.** A type without a declaration (`controls: None`: a
plugin's, a definition from an older version) gets
`ActionControls::legacy()`: keys only, with `press`, `release`, `click`,
`double-click` and `long-press`. Never encoders or touch zones.

```rust
pub fn compatibility(action: &ActionType, caps: &ControlCapabilities, trigger: Trigger)
    -> Compatibility; // Compatible | Incompatible(IncompatibilityReason)

pub enum IncompatibilityReason {
    ControlKindUnsupported { kind, supported: Vec<ControlKind> },
    InputUnsupportedByAction { kind, input, supported: Vec<InputCapability> },
    InputUnsupportedByControl { kind, input },
}
```

The rules, in order (the first that fails is the reason): the control
exists on the device (`control_capabilities` is `Some`); the action
supports the control's kind; it supports the trigger's input on that kind;
the control has that input natively or as a derived gesture. Reasons
serialize tagged (`{"type": "inputUnsupportedByAction", "kind": "encoder",
"input": "press", "supported": [...]}`) and read as a sentence ("the action
cannot be bound to an encoder; it works on keys only").

Only bindings that run an action are checked. Workflow, `openPage`, `back`
and stateful bindings keep their rules: any control, any trigger the
control supports. A binding to a control the model lacks, or for an input
the control cannot produce, stays `UnknownControl` / `UnsupportedTrigger`
of layout validation; a missing action stays `DanglingTarget`; a type that
cannot run here (its module disabled, a plugin not installed) is shown as
unavailable as before, and is not reported as incompatible.

### Built-in declarations

Every built-in action declares its controls; a contract test
(`crates/deckpilot-modules/tests/contracts.rs`,
`every_builtin_action_declares_its_controls`) fails for one that does not.
One-shot actions are keys only. Encoders are declared only for actions that
do the same thing once per event, with no amount (`STEP_ENCODER_INPUTS`:
either rotation, a press or a click): each rotation event the device
reports runs them once, whatever its number of detents. No built-in is
declared for touch zones.

| Action types | Key | Encoder |
| --- | --- | --- |
| `deck.next-page`, `deck.previous-page`, `deck.brightness`, `projects.cycle`, `projects.next`, `projects.previous` | press, release, click, double-click, long-press | rotate-clockwise, rotate-counter-clockwise, press, click |
| every other built-in (`deck.page-indicator`, `deck.switch-profile`, `deck.random`, `deck.multi-switch`, `docker.*`, `general.*`, `git.*`, `github.*`, `gitlab.*`, `http.*`, `jira.*`, `notifications.*`, the other `projects.*`, `system.*`, `workspace.*`) | press, release, click, double-click, long-press | – |

A first-party plugin may serve a legacy built-in id (`deckpilot_core::alias`)
only if it supports every control input the built-in declares
(`alias::incompatibilities`), so stored bindings of the legacy id stay
available.

## 3. Enforcement

Nothing is ever deleted or converted. An incompatible binding stays in the
profile, byte for byte, and works again as soon as a compatible action type
or device is there (an updated plugin, a re-enabled module).

1. **Core.** `DeviceLayout::validate` is unchanged (it has no action
   types). `DeviceLayout::check_compatibility(catalog, registry, profile)`,
   next to `check_overrides`, reports
   `LayoutIssue::IncompatibleAction { page, control, trigger, reason }`;
   `DeviceLayout::incompatible_bindings` returns the bindings with their
   page id and action. `Runtime::layout_status` includes them.
2. **App (`save_layout`).** A binding that is incompatible and *new* is
   refused with its reason, and nothing is saved. A binding that is in the
   stored version of the same layout unchanged (same page, control,
   trigger and action instance) is kept and reported in the answer's
   `incompatible` list (`DeviceLayout::compatibility_for_save`). So a
   profile stored before compatibility existed, or imported, can always be
   saved again, with or without other changes; only adding (or moving) an
   incompatible binding is refused. `preview_layout` answers each control's
   incompatible bindings (`incompatible: [{trigger, reason, message}]`) so
   the editor can explain.
3. **Import** (`deckpilot-portability`). Incompatible bindings are
   imported as they are and reported as warnings
   (`import::compatibility_warnings`, shown in the import preview).
4. **Agent.** On load and reload the agent logs the active profile's
   incompatible bindings (`Runtime::unavailable_bindings`, computed from
   the loaded action types every time, so nothing goes stale). The runtime
   refuses to dispatch one: a raw input or a recognized gesture of such a
   binding returns `RuntimeOutcome::Unavailable { action, control, trigger,
   reason, first }` and no handler runs. The first refusal of a binding
   since the profile or the action types were loaded is logged as a warning
   and shown in the app's status bar (no desktop notification); later ones
   are logged at debug level. A key whose shown binding is incompatible
   shows the existing unavailable face (its label, `Unavailable`) and asks
   for no live state.
5. **IPC.** `ActionType.controls` travels in the existing payloads (agent
   plugin summaries, the app's action types); older payloads without it
   decode as `None`.

**Migration.** No schema or document version changes: bindings are
stored exactly as before, and declarations live in code. A test opens the
storage fixtures of 0.5.0, 0.6.0 and schema 8
(`crates/deckpilot-portability/tests/control_compatibility_fixtures.rs`),
loads, checks and saves every profile, and asserts that every binding is
unchanged byte for byte and every key binding compatible. The fixtures'
seeded Stream Deck + layout has a tap on the touch strip bound to *Log*
(keys only): it is reported and kept. The Stream Deck + has no driver, so
such bindings never ran before either.

## 4. Plugin manifest (plugin API 1.3)

Status: implemented. The agent provides plugin API 1.3 (`PLUGIN_API` in
`apps/agent/src/plugins/service.rs`); the author reference is
[marketplace/sdk/manifest.md](marketplace/sdk/manifest.md#controls).

`contributes.actions[].controls` (optional), the same shape as
`ActionControls`:

```json
"controls": { "key": ["press", "click", "long-press"],
              "encoder": ["rotate-clockwise", "rotate-counter-clockwise"] }
```

- Using `controls`, `contributes.navigation` or `contributes.views`
  requires `engines.pluginApi` `^1.3` (a validator rule of
  `deckpilot-plugin-manifest`); agents up to 1.2 refuse such a manifest
  (unknown fields), which is the intended gate.
- Without `controls`: the legacy default of section 2. A plugin declaring
  `device.requires: ["dials"]` without `controls` still gets no encoder
  binding.
- Validation (the rules of `ActionControls::issues`, in the manifest
  validator with JSON paths): kinds are `key`, `encoder`, `touchZone`;
  inputs valid for the kind (no rotation on a key, no tap on an encoder);
  no empty set; each input once; at most `MAX_INPUTS_PER_KIND` (8) inputs
  per kind.
- The agent registers a plugin action's `ActionType.controls` from the
  manifest's `controls` as sets (`apps/agent/src/plugins/engine.rs`);
  `None` without them.

`contributes.navigation` (≤ 4 entries: `id`, `label` ≤ 24 characters,
`icon`, `view`) and `contributes.views` (≤ 8: `id`, `kind` ∈ `variables`,
`actions`, `settings`) add toolbar entries that open host-rendered views:

- `view` names a declared view; ids are unique local ids (≤ 32
  characters).
- Icons are package files under `assets/` (`.png`, `.webp`, `.svg`; no
  absolute path, scheme, `..` or backslash) and so in the integrity
  record. PNG/WebP ≤ 256 KiB and ≤ 512×512 (from the header); SVG ≤ 64 KiB,
  parsed (quick-xml) and rebuilt from an allow-list; scripts,
  `foreignObject`, event handlers, links other than `#id`, `<style>` with
  `url(`/`@import`, DTDs and entities are refused
  (`deckpilot_plugin_manifest::sanitize_svg`). The package reader refuses
  such a file at build and install time; the agent re-checks the digest
  and sanitizes again on every request.
- IPC (feature `navigation` in `PluginRuntimeStatus.features`):
  `PluginSummary.navigation` lists the entries with their views;
  `getPluginIcon` with `navigation: { entry, theme }` sends the icon in
  chunks (`image/png`, `image/webp` or `image/svg+xml`).
- Desktop commands: `list_plugin_navigation` (entries of enabled plugins)
  and `get_plugin_navigation_icon` (a `data:` URL for `<img>`, `null` on
  any failure so the app shows a fallback icon). The toolbar itself comes
  with the app shell (section 5).

## 5. App shell

Status: implemented in the device editor and the toolbar
(`apps/desktop/src/lib/compatibility.ts`, `layout-editor/`,
`shell/plugin-navigation.ts`). The app mirrors the rules of section 2 only
to explain and to disable what the backend would refuse; `save_layout`
stays the authority.

- **Library.** With one control selected, entries (profile actions and
  catalog types) that cannot go on it at all are shown disabled with the
  reason, e.g. "Requires a dial. The selected Stream Deck MK.2 key
  supports press actions only." (`controlKindUnsupported`), or "Requires a
  press on a dial. The selected Knob Box dial supports turn actions only."
  (no input the action takes is one the control has). **Compatible only**
  hides them (`deckpilot.editor.compatibleOnly`, local storage).
- **Drops.** A drop that would bind an action to a control it cannot run
  from (also a moved control's bindings) is refused: nothing changes, the
  control shows `data-drop="rejected"` with a red outline, the reason is
  in a toast and announced (dnd-kit announcements and a live region). A
  compatible drop picks the first free trigger the action can run from.
- **Inspector.** Each trigger's choices disable actions incompatible with
  it ("(not on this control)"), from the reason variants: "Does not react
  to double click on a dial. On a dial it reacts to press, turn right, turn
  left and click." (`inputUnsupportedByAction`), "The selected … dial has
  no press; it supports turn actions only." (`inputUnsupportedByControl`).
  A stored incompatible binding (`preview_layout` `incompatible`) shows
  "Not run from this control: <reason> It stays assigned but is not run."
- **Save.** A refusal shows the backend's reason with the trigger named
  as in the editor; bindings kept by `save_layout` (`incompatible`) are
  listed in a warning.
- **Plugin toolbar entries.** `list_plugin_navigation` (read again on
  plugin events), in the user's order (`deckpilot.toolbar.pluginOrder`),
  at most four in the toolbar and the rest under "More plugin views" (all
  of them in a narrow window). Icons come from `get_plugin_navigation_icon`
  for the current theme and are shown only through `<img>` when they are
  a base64 `data:` URL of PNG, WebP or SVG; anything else shows the app's
  own icon. An entry opens `plugin-view/<plugin>/<entry>`, a view the app
  draws for the kind (`variables`: the variables table under the plugin's
  namespace; `actions`: the Action Library with the plugin's actions;
  `settings`: its configuration form). Unknown kinds have no entry.
