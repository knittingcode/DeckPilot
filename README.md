# DeckPilot

DeckPilot is an open-source, lightweight, cross-platform application for managing
Stream Deck devices on Linux, macOS and Windows.

> **Status: early development.** DeckPilot consists of a background agent
> (devices, actions, system tray) and a configuration app. Support for the
> Elgato Stream Deck MK.2 is implemented but **has not yet been tested on a
> physical device**. See [Known limitations](#known-limitations).

## Concepts

DeckPilot is *action-first*: you configure what should happen before deciding
which control triggers it.

- **Profiles** are independent sets of actions, workflows, projects and device
  layouts. `Start Work` in one profile can do something entirely different from
  `Start Work` in another.
- **Projects** live inside a profile and are selected cyclically
  (`SHOP → CMR → CMS → SHOP`). Project-dependent actions resolve their target
  from the active project. Switching projects never changes Git branches,
  restarts containers or closes applications.
- **Machine resource bindings** map logical resource keys such as
  `shop.repository` to concrete paths or identifiers on each computer, so
  profiles stay portable between machines and operating systems.
- **Device models** describe capabilities (key grids, encoders, touch surfaces)
  independently of actions. Layouts are validated against them.

All configuration is stored locally. DeckPilot has no cloud service, no
synchronization and no telemetry.

## Repository layout

```
crates/
  deckpilot-device/    Device models, capabilities, driver interface, mock devices
  deckpilot-core/      Actions, workflows, profiles, projects, resources, runtime
  deckpilot-storage/   ConfigStore trait and SQLite implementation
  deckpilot-hid/       USB HID driver (Stream Deck MK.2)
  deckpilot-ipc/       Agent IPC protocol, endpoint and client
apps/
  agent/               Background agent with system tray
  desktop/             Tauri 2 configuration app (React, TypeScript, shadcn/ui)
    src-tauri/         Rust side of the configuration app
packaging/linux/       udev rule for non-root device access
docs/                  Architecture, agent, data model, development, roadmap
```

## Getting started

Requirements: Rust via rustup (the version is pinned in `rust-toolchain.toml`), Node.js 22 or newer, pnpm, and the
[Tauri system prerequisites](https://tauri.app/start/prerequisites/) for your
operating system.

```sh
pnpm install
cargo build -p deckpilot-agent   # the app starts the agent next to it
pnpm dev                         # configuration app in development mode
cargo test --workspace
```

Run the agent with a simulated Stream Deck instead of hardware:

```sh
cargo run -p deckpilot-agent -- --mock-devices
```

On Linux, install `packaging/linux/70-deckpilot-streamdeck.rules` to access
the device without root.

See [docs/development.md](docs/development.md) for all commands.

## Documentation

- [Architecture](docs/architecture.md)
- [Background agent and IPC](docs/background-agent.md)
- [Data model](docs/data-model.md)
- [Development](docs/development.md)
- [Roadmap](docs/roadmap.md)

## Known limitations

- The Stream Deck MK.2 driver has **not** been tested on a physical device.
  Only the MK.2 is supported; other models are not exposed.
- Installers do not bundle the agent yet; it must be placed next to the
  configuration app executable (as in development builds).
- Keys show text labels only; icons are not supported yet.
- Device discovery rescans every 2 seconds instead of using OS hotplug
  notifications.
- On Linux desktops without AppIndicator support, the tray icon is not shown
  (the agent keeps working).
- Only hardware-independent built-in actions exist (log message, cycle/select
  project, resolve resource).
- Workflows and layouts have no editor; layouts can be generated from a
  profile's actions.
- There is no update mechanism; the tray has no “Check for Updates” entry.
- Database encryption is available in the storage crate behind the `sqlcipher`
  feature but is not enabled in the app; there is no password setup or
  import/export yet.
- CI builds and tests on Linux, macOS and Windows; the agent and app have been
  run manually on macOS only.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
