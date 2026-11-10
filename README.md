# DeckPilot

DeckPilot is an open-source, lightweight, cross-platform application for managing
Stream Deck devices on Linux, macOS and Windows.

> **Status: early development.** This repository contains the foundation: the
> domain model, local storage and a configuration application. DeckPilot does
> **not** communicate with Stream Deck hardware yet. See [Known limitations](#known-limitations).

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
  deckpilot-device/    Device models, capabilities, controls, events
  deckpilot-core/      Actions, workflows, profiles, projects, resources, engine
  deckpilot-storage/   ConfigStore trait and SQLite implementation
apps/
  desktop/             Tauri 2 configuration app (React, TypeScript, shadcn/ui)
    src-tauri/         Rust side of the configuration app
docs/                  Architecture, data model, development guide, roadmap
```

## Getting started

Requirements: Rust via rustup (the version is pinned in `rust-toolchain.toml`), Node.js 22 or newer, pnpm, and the
[Tauri system prerequisites](https://tauri.app/start/prerequisites/) for your
operating system.

```sh
pnpm install
pnpm dev            # start the configuration app in development mode
cargo test --workspace
```

See [docs/development.md](docs/development.md) for all commands.

## Documentation

- [Architecture](docs/architecture.md)
- [Data model](docs/data-model.md)
- [Development](docs/development.md)
- [Roadmap](docs/roadmap.md)

## Known limitations

- No device communication: discovery, key images and input handling are not
  implemented. The Elgato Stream Deck MK.2 descriptor is based on public
  protocol documentation and has **not** been tested on a physical device.
- The background agent does not exist yet; the configuration app is the only
  executable.
- Only hardware-independent built-in actions exist (log message, cycle/select
  project, resolve resource).
- Workflows and device layouts exist in the domain model but have no editor.
- Database encryption is available in the storage crate behind the `sqlcipher`
  feature but is not enabled in the app; there is no password setup or
  import/export yet.
- CI runs on Linux, macOS and Windows; local verification so far has been on
  macOS only.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
