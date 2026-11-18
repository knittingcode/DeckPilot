# Device controls and action compatibility

Which physical controls an action can be bound to, how DeckPilot decides
whether a binding can run, and what every layer does with one that cannot.
*Design record*: the contract of the device-capability redesign. Sections
1 to 3 are implemented; sections 4 and 5 (the plugin manifest and the
app's shell) are planned and listed here so the types already fit them.
Where the code differs, the code and its tests win.

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

## 4. Plugin manifest (planned, plugin API 1.3)

`contributes.actions[].controls` (optional), the same shape as
`ActionControls`:

```json
"controls": { "key": ["press", "click", "long-press"],
              "encoder": ["rotate-clockwise", "rotate-counter-clockwise"] }
```

- Using `controls` (or `contributes.navigation`) requires
  `engines.pluginApi` `^1.3`; agents up to 1.2 refuse such a manifest
  (unknown fields).
- Without `controls`: the legacy default of section 2. A plugin declaring
  `device.requires: ["dials"]` without `controls` still gets no encoder
  binding.
- Validation (`ActionControls::issues` holds the core of it): kinds are
  `key`, `encoder`, `touchZone`; inputs valid for the kind (no rotation on
  a key, no tap on an encoder); no empty set; at most
  `MAX_INPUTS_PER_KIND` (8) inputs per kind.
- Plugin navigation entries and host-rendered views
  (`contributes.navigation`) are specified with that change.

Until then the agent converts manifest actions with `controls: None`
(`apps/agent/src/plugins/engine.rs`).

## 5. App shell (planned)

The device editor will show incompatible actions disabled, with the
reason ("Requires a dial. The selected Stream Deck MK.2 key supports press
actions only."), refuse a drop on an incompatible control and offer a
compatible-only filter, using `ActionType.controls` and the
`preview_layout` reasons above.
