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

## Milestone 1 – Background agent and MK.2 support (done; Windows and Linux hardware tests pending)

Implemented:

- `deckpilot-agent` binary that runs independently of the configuration app,
  with a system tray on Linux, macOS and Windows
- HID driver for the Stream Deck MK.2 (discovery, reconnection, key input, key
  images, brightness)
- Authenticated, versioned local IPC; configuration reload; event subscription
- Profile and project switching without restarting the agent
- Key image rendering from labels; generated default layouts
- Per-user start at login

- The agent bundled with the configuration app's release builds
- The MK.2 driver tested on a physical device on macOS
  ([mvp-acceptance.md](mvp-acceptance.md#physical-mk2-2026-11-13))

Remaining before the milestone is complete:

- Test the MK.2 driver on a physical device on Windows and Linux, and with
  more than one device at a time

## Milestone 1.x – Polish

- Icons on keys (done: custom icons and an icon library in 0.6.0)
- Event-driven device discovery where the OS provides hotplug notifications

## Milestone 2 – Configuration studio, live development, updates (done except the first published release)

Implemented:

- Action Library, Workflow editor (conditions, overrides, reordering), profile
  and project management, per-machine resource mappings
- Device model catalog (MK.2, XL, Stream Deck +) and a visual layout designer
  that works without hardware
- Supervised live development environment (`make up`, `pnpm dev`) with
  simulated devices and isolated data
- Signed updates from GitHub Releases with coordinated agent restart, and a
  release pipeline that builds, signs and checks draft releases and
  publishes them from a protected release tag
  ([releasing.md](releasing.md))
- The updater signing key provisioned; signed updates and Keychain
  continuity verified between two Developer ID signed builds
  ([updates.md](updates.md))
- Modular feature architecture: built-in modules (`general`, `projects`) with
  namespaced action types, module-driven navigation, enable/disable without
  losing configuration; projects stored per profile with database constraints
- An empty Action Library on new installations; demonstration data only in
  the development seed

Remaining before the milestone is complete:

- Publish the first release and verify an update between two published
  releases end to end
- Profile switching from the device

## Milestone 3 – Security and portability (v0.4; done on macOS)

Implemented:

- An always SQLCipher-encrypted database; by default with a random key in
  the OS credential store (opens at login without a password), optionally
  password-protected (Argon2id); explicit switching in both directions,
  password change, lock, automatic locking (sleep, inactivity), Touch ID on
  macOS, and a fresh authentication for sensitive operations
- Safe encryption of databases from earlier versions, keeping the original
  until the user removes it
- Verified encrypted backups before schema upgrades, migration history and
  a compatibility policy between versions
- Encrypted configuration export and import with preview, merge/replace,
  conflict resolution and resource mapping
- Module portability contracts and module-owned schema versions
- A signing check for the updater keys and build provenance for releases
- macOS code signing: Developer ID signed, hardened runtime, notarized and
  stapled builds (verified on the `v0.6.0-alpha.2` draft), which keeps
  macOS from asking for Keychain access after updates
- Credentials for modules (HTTP, GitHub, GitLab, Jira) stored on this
  computer and used by name, never exported

Remaining before the milestone is complete:

- The first published signed release and an end-to-end update between two
  published releases
- Windows code signing (Authenticode); Windows and Linux builds are not
  code-signed yet
- Windows Hello and polkit as system authentication; locking with the
  screen lock on Windows

## Milestone 4 – Platform actions (done in 0.6.0)

- Platform services behind explicit interfaces: the System module (open
  applications, web pages, files and folders, run commands), the Workspace
  module (editors, repositories, pages) and the Git and HTTP modules
- Docker/Compose environment actions (the Docker module)
- Actions declare what running them does, and actions that change things
  say so before they are saved; commands and Compose changes are off until
  allowed on each computer

## Milestone 5 – Release builds and update distribution (done; first release pending)

- Release builds for macOS, Windows and Linux, a signed update manifest per
  channel, `SHA256SUMS`, a signing record and build provenance, assembled
  into a draft release and checked before publication
  ([releasing.md](releasing.md))
- Stable and pre-release update channels

Remaining:

- Publish the first pre-release

## Later

- Drivers for more Stream Deck models (XL, Stream Deck +, encoders and
  touch surfaces), after hardware testing; today only the MK.2 has a
  driver
- Hardware testing on Windows and Linux

## Out of scope

- Cloud synchronization, telemetry or analytics
- Plugin marketplace (only a design proposal exists, in open pull request
  #191; nothing of it is implemented)
