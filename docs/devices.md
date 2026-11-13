# Devices: models, drivers and verification

How DeckPilot describes control-surface hardware, how drivers plug in, and
how far each model has been verified. The design keeps one rule: only a
driver knows a vendor's protocol; everything else (core, workflows, actions,
gestures, layouts, rendering, the configuration app) works with generic
models, capabilities and events.

## Audit (main at 592d5f1, 2026-11-13)

### Where Elgato-specific types and USB ids are used

| Place | What | Kind |
| ----- | ---- | ---- |
| `crates/deckpilot-hid` | `elgato_streamdeck::{Kind, StreamDeck, StreamDeckInput}`, `hidapi`, `model_for(Kind)` | The Elgato adapter. All Elgato protocol types are here and only here. |
| `crates/deckpilot-device/src/catalog.rs` | Built-in descriptors `elgato.streamdeck.{mk2,xl,plus}` with USB `0fd9:0080/006c/0084`; `ModelCatalog::find_by_usb` | Static model data. `find_by_usb` is used only by its own test; discovery by USB id happens in the adapter. |
| `apps/agent/src/service.rs` `default_options` | `deckpilot_hid::ElgatoBackend::new()` | The one place that wires the Elgato driver into the agent. |
| `apps/agent/src/main.rs` | `--mock-devices mk2,xl,plus` | Chooses built-in models for simulated devices. |
| `crates/deckpilot-ipc` | Doc comments name `elgato-hid` as an example backend name | Strings only. |
| `tools/dev` | `hardware-test` targets `elgato.streamdeck.mk2` | Test procedure for that model. |
| Tests (core, storage, portability, agent, desktop) | `models::stream_deck_mk2()`, `"elgato.streamdeck.mk2"` | Fixtures. |

`deckpilot-core`, `deckpilot-render`, `deckpilot-storage`,
`deckpilot-portability` and the modules have **no** dependency on
`deckpilot-hid`, on `elgato-streamdeck` or on USB ids. Core uses the model
catalog only by model id (`ModelCatalog::get`) to validate layouts and pages.

### What is already generic

- `DeviceModelId` (stable string id, e.g. `elgato.streamdeck.mk2`; stored in
  layouts), `DeviceId` (model + serial).
- `DeviceCapabilities`: optional `KeyGrid` (rows, columns, image spec),
  `EncoderSpec` (dials), `TouchSurface` (zones, image, gestures), with
  `controls()`, `has_control`, `has_display`, `validate_input`.
- `ControlId` (`Key`, `Encoder`, `TouchZone`), `ControlInput` (`Press`,
  `Release`, `Rotate`, `Touch`), `DeviceEvent` (`Connected`, `Disconnected`,
  `Input`), `DeviceRegistry`/`ConnectionState`.
- `Frame`: device-independent RGB image. `deckpilot-render` draws faces into
  frames of the size the model's `ImageSpec` asks for; the driver encodes and
  orients them.
- `KeyStateTracker`: turns key-state snapshots into press/release events.
- `MockBackend` (feature `mock`): simulated devices of any model.
- Core runtime, gesture engine, layout validation and the agent's render loop
  read only capabilities and generic events.

### The backend interface

```rust
pub trait DeviceBackend: Send {
    fn name(&self) -> &'static str;                       // driver id, e.g. "elgato-hid"
    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError>;
    fn open(&mut self, device: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError>;
    fn source(&self, device: &DeviceId) -> &'static str;  // which driver provides a device
}

pub trait DeviceHandle: Send {
    fn id(&self) -> &DeviceId;
    fn read_input(&mut self, timeout: Duration) -> Result<Vec<(ControlId, ControlInput)>, DeviceError>;
    fn set_image(&mut self, control: ControlId, frame: &Frame) -> Result<(), DeviceError>;
    fn clear_image(&mut self, control: ControlId) -> Result<(), DeviceError>;
    fn set_brightness(&mut self, percent: u8) -> Result<(), DeviceError>;
    fn flush(&mut self) -> Result<(), DeviceError>;
}
```

`CombinedBackend` merges several backends (USB plus simulated) and opens
each device with the backend that discovered it.

### Gaps found

- The support level was one value for all platforms (`HardwareSupport`), and
  the MK.2 still read `Unverified` although it was tested on hardware on
  macOS.
- No generic description of a *connected device* (vendor, driver, driver
  version) for code outside the agent; IPC carried only the model name and
  the backend name.
- Model descriptors cannot come from a driver: a driver for a new vendor would
  need a change to the built-in catalog.

## Target design

### Model registry (`deckpilot-device`, `ModelCatalog`)

Static data per model: id, vendor, name, capabilities, and the
**verification status per platform** with evidence date and note. Model ids
never change; stored layouts refer to them.

### Verification levels

| Level | Meaning |
| ----- | ------- |
| `unsupported` | Not supported on this platform. |
| `experimental` | A work-in-progress definition; may be incomplete or wrong. |
| `defined` | The model is described; layouts can be designed. No driver. |
| `driverImplemented` | A driver exists and builds for the platform. |
| `automatedTests` | The driver's automated tests (protocol checks, simulated devices) run on the platform. No physical device. |
| `physicallyVerified` | Tested on a physical device on this platform. |

Rules:

- Only the built-in registry can record `physicallyVerified`
  (`PlatformVerification::physically_verified` is crate-private; the public
  constructor caps at `automatedTests`). Simulated devices, tests and drivers
  registered at run time cannot raise a model to it.
- A model is **supported** when it is physically verified on at least one
  platform. Other models are **experimental**: they stay in the registry
  (layouts can be designed for them) but are not listed as supported.
- A platform without an entry reads as `defined`.
- The legacy `support` field (`HardwareSupport`) is kept for clients that
  read it; it is `verified` when the model is physically verified anywhere.

Current data (recorded 2026-11-13):

| Model | macOS | Windows | Linux |
| ----- | ----- | ------- | ----- |
| Stream Deck MK.2 (`elgato.streamdeck.mk2`) | physically verified (2026-11-13: guided `deckpilot-dev hardware-test` and real use: display, keys, brightness, profile switching, USB reconnect, gestures, Work Session timer face) | automated tests | automated tests |
| Stream Deck XL (`elgato.streamdeck.xl`) | defined | defined | defined |
| Stream Deck + (`elgato.streamdeck.plus`) | defined | defined | defined |

"Automated tests" for the MK.2 on Windows and Linux means: the driver builds
and its tests run in CI there (descriptor checked against the protocol
library, simulated devices). Nobody has used a physical MK.2 on those
platforms with DeckPilot.

### Device descriptor (the generic device context)

`deckpilot_device::DeviceDescriptor` describes one connected (or simulated)
device without vendor types. It is the stable device context for code
outside the drivers, including variable resolution:

| Field | Example |
| ----- | ------- |
| `vendorId` | `elgato` (first segment of the model id) |
| `vendorName` | `Elgato` |
| `modelId` | `elgato.streamdeck.mk2` |
| `displayName` | `Stream Deck MK.2` |
| `serial` | hardware identity reported by the device |
| `capabilities` | `DeviceCapabilities` (keys, dials, touch strip) |
| `driver` | `{ "id": "elgato-hid", "version": "0.6.0" }`, or `null` when unknown |
| `verification` | per-platform list of `{ platform, level, evidenceDate, evidence }` |

How to get one:

- From a `DeviceId` and the catalog: `ModelCatalog::describe(&device, driver)`.
- In the agent: `Agent::device_descriptor(&device)` (catalog + the driver of
  the backend that provides the device).
- Over IPC: `AgentStatus.devices[].descriptor` (optional, additive field;
  older agents omit it, older clients ignore it; no protocol bump).

Fields are only ever added.

### Drivers (adapter, strategy, registry)

- **Adapter**: a vendor driver implements `DeviceBackend`/`DeviceHandle` and
  wraps the vendor's HID/protocol layer. It translates native reports into
  `ControlId` + `ControlInput` and accepts generic `Frame`s. Each driver
  reports its id (`name`) and implementation version (`version`).
- **Strategy**: model-specific behaviour (image size, orientation and
  encoding, how inputs are reported) is chosen by the adapter per model,
  driven by the model's `DeviceCapabilities`/`ImageSpec`.
- **Registry/factory**: drivers are registered with the agent's backend;
  discovery yields `DeviceId`s that resolve to descriptors.
- **Capabilities**: keys, dials (encoders), touch strip and brightness are
  the capability families core works with.

Elgato-specific code lives only in `deckpilot-hid`.
