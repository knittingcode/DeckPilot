# Roadmap

Milestones are deliberately small. Each one ends with working, tested
functionality.

## Milestone 0 – Foundation (current)

- Cargo and pnpm workspaces, CI on Linux, macOS and Windows
- Domain model: actions, workflows, profiles, projects, resources, devices,
  layouts
- Execution engine and hardware-independent runtime
- SQLite storage with migrations; optional SQLCipher build
- Configuration app: dashboard, actions, profiles, projects, devices, settings

## Milestone 1 – Background agent and MK.2 support

- `deckpilot-agent` binary that runs independently of the configuration app
- HID backend for the Stream Deck MK.2 (discovery, hotplug, key input, key
  images), tested on a physical device before being marked as verified
- Local IPC between the configuration app and the agent; configuration reload
- Autostart integration per operating system
- Key image rendering from labels and icons

## Milestone 2 – Editors

- Workflow editor (ordering steps, failure policy, test run)
- Layout editor with pages and folders, usable without a connected device
- Profile switching from the device

## Milestone 3 – Platform actions

- Platform services behind explicit interfaces: open application, open
  directory, open terminal at a resource, open URL
- Docker/Compose environment actions
- Clear confirmation for any action with side effects

## Milestone 4 – Security and portability

- Password setup, SQLCipher-encrypted database by default
- Password prompts on import, export and first launch after an update
- Encrypted manual import/export of profiles and layouts
- Secret storage separated from portable configuration

## Later

- Additional device models (encoders, touch surfaces) after hardware testing
- Release builds and update distribution through GitHub Releases

## Out of scope

- Cloud synchronization, telemetry or analytics
- Plugin marketplace
