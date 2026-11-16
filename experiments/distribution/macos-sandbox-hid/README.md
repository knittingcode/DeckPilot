# macOS App Sandbox and HID (hidapi)

Question ([capability matrix](../../../docs/distribution/capability-matrix.md),
section 2.4): can the DeckPilot agent, sandboxed as the Mac App Store
requires, enumerate and open HID devices with hidapi?

Run: `tools/dev/target-slot.sh run -- experiments/distribution/macos-sandbox-hid/run.sh <work dir>`.
It builds `src/main.rs` (hidapi 2.6.7 with default features, the agent's
version; devices open in exclusive mode as the agent opens them), signs it
ad hoc with the hardened runtime in four variants, and writes one report
per variant to `results/`. `summarize.py` turns the reports into the
table below. Nothing is installed; the bundles use experiment identifiers
(`io.github.knittingcode.deckpilot.experiment.hid-*`), never DeckPilot's.
Their sandbox containers stay in `~/Library/Containers` and can be removed
by hand.

| Variant | Signature | Started by |
| --- | --- | --- |
| `unsandboxed-direct` | hardened runtime, no entitlements (control) | the shell |
| `sandbox-usb-open` | `app-sandbox` + `device.usb` ([entitlements](entitlements/sandbox-usb.plist)) | LaunchServices (`open`), so it is its own responsible process |
| `sandbox-usb-direct` | the same bundle's executable | the shell |
| `sandbox-only-open` | `app-sandbox` only ([entitlements](entitlements/sandbox-only.plist)), a control for `device.usb` | LaunchServices |

Apple documents no HID-specific App Sandbox entitlement.
`com.apple.developer.hid.virtual.device` is for virtual devices and the
DriverKit HID entitlements are for driver extensions; neither applies to
hidapi, so `device.usb` is the only candidate.

## Result (2026-11-16, macOS 27.2 (26B123), arm64)

**No Stream Deck was attached**: `ioreg -p IOUSB` lists only the three
XHCI controllers, and no variant found a device of vendor `0x0fd9`
([environment](results/environment.txt)). The devices tested are the
built-in ones (internal keyboard and trackpad interfaces, sensors, the
keyboard backlight, a headset control).

- **Sandbox enforced** in all three sandboxed variants:
  `APP_SANDBOX_CONTAINER_ID` is set and the real `~/Library/Preferences` is
  denied (`EPERM`); readable in the control.
- **Enumeration works in every variant**, with or without `device.usb`:
  22 HID interfaces each, the same set.
- **Opening needs `device.usb`.** Without it every open fails with
  `kIOReturnNotPermitted` (`0xE00002E2`). With it, the sandboxed process
  opens exactly the interfaces the unsandboxed control opens, in exclusive
  and shared mode, started either way, and a zero-timeout read succeeds on
  each handle.
- **Keyboard and pointer interfaces are refused in every variant,
  sandboxed or not**, with the same `kIOReturnNotPermitted`: generic
  desktop keyboard, mouse and pointer, the keyboard's consumer-control and
  digitizer collections, and two vendor collections of the same device.
  `IOHIDCheckAccess(ListenEvent)` (Input Monitoring) reports `denied` for
  the control and the `device.usb` variants and `unknown` for the
  sandbox-only bundle, which never opened anything. This is the Input
  Monitoring (TCC) rule, not the App Sandbox: the sandbox adds no denial
  beyond it.

| product | usage_page:usage | unsandboxed-direct | sandbox-usb-open | sandbox-usb-direct | sandbox-only-open |
| --- | --- | --- | --- | --- | --- |
| Apple Internal Keyboard / Trackpad | 0x0001:0x0001 | denied/denied | denied/denied | denied/denied | denied/denied |
| Apple Internal Keyboard / Trackpad | 0x0001:0x0002 | denied/denied | denied/denied | denied/denied | denied/denied |
| Apple Internal Keyboard / Trackpad | 0x0001:0x0006 | denied/denied | denied/denied | denied/denied | denied/denied |
| Apple Internal Keyboard / Trackpad | 0x000c:0x0001 | denied/denied | denied/denied | denied/denied | denied/denied |
| Apple Internal Keyboard / Trackpad | 0x000d:0x0005 | denied/denied | denied/denied | denied/denied | denied/denied |
| Apple Internal Keyboard / Trackpad | 0xff00:0x0003 | ok/ok | ok/ok | ok/ok | denied/denied |
| Apple Internal Keyboard / Trackpad | 0xff00:0x0006 | denied/denied | denied/denied | denied/denied | denied/denied |
| Apple Internal Keyboard / Trackpad | 0xff00:0x000b | ok/ok | ok/ok | ok/ok | denied/denied |
| Apple Internal Keyboard / Trackpad | 0xff00:0x000c | denied/denied | denied/denied | denied/denied | denied/denied |
| Apple Internal Keyboard / Trackpad | 0xff00:0x000d | ok/ok | ok/ok | ok/ok | denied/denied |
| Apple Internal Keyboard / Trackpad | 0xff00:0x005f | ok/ok | ok/ok | ok/ok | denied/denied |
| BTM | 0xff00:0x0048 | ok/ok | ok/ok | ok/ok | denied/denied |
| Headset | 0x000c:0x0001 | ok/ok | ok/ok | ok/ok | denied/denied |
| Keyboard Backlight | 0xff00:0x000f | ok/ok | ok/ok | ok/ok | denied/denied |
| accel | 0xff00:0x0003 | ok/ok | ok/ok | ok/ok | denied/denied |
| als | 0xff00:0x0004 | ok/ok | ok/ok | ok/ok | denied/denied |
| als-temp | 0xff00:0x0005 | ok/ok | ok/ok | ok/ok | denied/denied |
| cma | 0xff0c:0x0001 | ok/ok | ok/ok | ok/ok | denied/denied |
| devmotion6 | 0xff0c:0x0005 | ok/ok | ok/ok | ok/ok | denied/denied |
| gyro | 0xff00:0x0009 | ok/ok | ok/ok | ok/ok | denied/denied |
| las | 0x0020:0x008a | ok/ok | ok/ok | ok/ok | denied/denied |
| wakehint | 0xff00:0x00ff | ok/ok | ok/ok | ok/ok | denied/denied |

Cells are `exclusive/shared`. Raw reports: [`results/`](results/).

## What this does and does not show

Shown: inside the App Sandbox with `com.apple.security.device.usb`, hidapi
enumerates all HID interfaces and opens every one that an unsandboxed
process with the same TCC state can open, exclusively as the agent does.
Without the entitlement nothing opens. The sandbox therefore is not, on
this evidence, what would stop the agent.

Not shown, still **Unverified**:

- **A Stream Deck.** None was attached. The deciding question is whether
  macOS treats a Stream Deck's interface like the keyboard interfaces
  above (refused without Input Monitoring) or like the vendor and
  headset interfaces (opened). The direct GitHub build opens Stream Decks
  today without asking for Input Monitoring, which points to the latter,
  but that was not observed here. Next step: attach a Stream Deck and rerun
  `run.sh`; the report's `elgato_devices` line and the vendor `0x0fd9` rows
  answer it.
- **Built-in devices only.** No USB-attached HID device was tested. The
  built-in keyboard and trackpad are on SPI, not USB; `device.usb` still
  governs opening them through IOKit, as the `sandbox-only-open` control
  shows.
- **An ad hoc signature, not an Apple Distribution one** with a
  provisioning profile, and no App Review. A store-signed build may differ,
  and review may still refuse the app.
- **Only `read_timeout(0)`**: no output report or feature report was sent
  (the built-in devices are not ones to write to). A Stream Deck test must
  also set an image or brightness.
