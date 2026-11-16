# Distribution feasibility experiments (PR G)

Small, self-contained probes for the cells of the
[capability matrix](../../docs/distribution/capability-matrix.md) that the
platforms' documentation does not settle. Each is its own Cargo project
(not a workspace member, its own `Cargo.lock`), uses ad hoc or throwaway
signatures and experiment identifiers only, and records what it did and
did not show. None of them is part of a build or a release, and none
implies that a store would accept DeckPilot.

| Experiment | Question | Status | Matrix |
| --- | --- | --- | --- |
| [`macos-sandbox-hid`](macos-sandbox-hid/README.md) | Can hidapi open HID devices inside the App Sandbox? | Run 2026-11-16 (macOS 27.2, arm64), built-in devices only, no Stream Deck | [E1] |
| [`macos-sandbox-plugin-host`](macos-sandbox-plugin-host/README.md) | Can a sandboxed process spawn the plugin host, run wasmtime's JIT under the hardened runtime, and apply the host's Seatbelt profile? | Run 2026-11-16 (macOS 27.2, arm64) | [E2] |
| [`msix-hid`](msix-hid/README.md) | Does hidapi enumerate from inside a full-trust MSIX? | Prepared, not run (manual job in `distribute.yml`) | [E5] |

The Flatpak draft ([E3]) is in [`packaging/flatpak`](../../packaging/flatpak/README.md)
and the Debian version check ([E4]) in [`packaging/apt`](../../packaging/apt/README.md).

Build through the shared target pool, as the scripts expect:

```sh
tools/dev/target-slot.sh run -- experiments/distribution/macos-sandbox-hid/run.sh <work dir>
tools/dev/target-slot.sh run -- experiments/distribution/macos-sandbox-plugin-host/run.sh <work dir>
```
