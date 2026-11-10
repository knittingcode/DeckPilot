# Roadmap

Milestones are deliberately small. Each one ends with working, tested
functionality.

## Milestone 0 – Foundation (done)

- Cargo and pnpm workspaces, CI on Linux, macOS and Windows
- Domain model: actions, workflows, profiles, projects, resources, devices,
  layouts
- Execution engine and hardware-independent runtime
- SQLite storage with migrations; optional SQLCipher build
- Configuration app: dashboard, actions, profiles, projects, devices, settings

## Milestone 1 – Background agent and MK.2 support (current)

Implemented:

- `deckpilot-agent` binary that runs independently of the configuration app,
  with a system tray on Linux, macOS and Windows
- HID driver for the Stream Deck MK.2 (discovery, reconnection, key input, key
  images, brightness)
- Authenticated, versioned local IPC; configuration reload; event subscription
- Profile and project switching without restarting the agent
- Key image rendering from labels; generated default layouts
- Per-user start at login

Remaining before the milestone is complete:

- Test the MK.2 driver on a physical device on all three platforms and mark
  the model as verified
- Bundle the agent with the configuration app installers

## Milestone 1.x – Polish

- Icons on keys
- Event-driven device discovery where the OS provides hotplug notifications

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
