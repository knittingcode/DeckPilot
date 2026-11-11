# Changelog

All notable changes to DeckPilot. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). The section of a version is used
as the notes of its GitHub release and of the in-app update.

## [0.6.0] - Unreleased

### Added

- System module: open applications, web pages, files and folders, run
  commands and check whether a program is running. Applications, folders
  and programs come from resources bound on each computer. Commands run
  without a shell, with an argument list, a time limit, bounded output and
  a minimal environment; running them is off until allowed in Settings ›
  System on this computer. Their output reaches later workflow nodes but is
  never kept in history.
- Action types declare what running them does (read-only, opens something,
  changes things) and failures carry a stable error code; both are shown in
  the workflow editor.

## [0.5.0] - Unreleased

### Added

- Settings have their own navigation, grouped into General, Hardware,
  Security & Data, Extensions and Advanced, with one page per area. Back and
  forward work like in a browser and the place is remembered.
- Modules are managed like plugins: search, filters, and an explanation when
  a dependency prevents enabling or disabling one. Modules with settings get
  their own settings page.
- On macOS the toolbar is part of the window's titlebar. Page commands such
  as New workflow, Run workflow or Refresh devices are in the toolbar.
- A status bar shows the agent, connected devices (simulated ones
  separately), the active profile and project, background activity such as
  update downloads and workflow runs, and notifications.
- Workflow graphs (engine): conditions, parallel branches with joins,
  bounded loops, delays, retries, timeouts, cancellation and data passed
  between steps. Existing workflows keep running unchanged.
- A visual workflow editor: a searchable catalog of flow nodes and module
  actions, drag and drop, typed connections, a node inspector with data
  mapping, retries and timeouts, undo and redo, copy and paste, automatic
  arrangement, a minimap, and the engine's validation shown on the nodes.
  Step-based workflows can be opened in the editor; their steps are kept.
- Runs show on the workflow graph as they happen — running, succeeded,
  failed, skipped and cancelled nodes with durations, real progress, node
  results and declared outputs — with a run history per workflow, also for
  runs started from a Stream Deck or while the app was closed.
- Workflows run in the background agent, also while the app is closed, with
  live progress, cancellation and an execution history kept in the
  encrypted database. Runs interrupted by an agent restart are marked as
  such, and the app picks up running workflows when it opens.

### Changed

- The workflow editor: connections can be moved to another node, refused
  connections say why, nodes move with the arrow keys, new nodes never cover
  existing ones, and Arrange keeps branches apart without crossings. Data
  mappings are built from parts — earlier nodes' outputs with their types,
  and text — with a preview and warnings for missing references and values
  that do not fit. The inspector documents each node: module, permissions,
  behaviour, inputs and outputs. The view and open panels are remembered.

- Cancelling a workflow now reaches running actions: actions that support
  it (such as the new Wait action) stop at once, also when their timeout
  passes, and retries waiting for their next attempt end immediately.
  Cancelled steps are reported as cancelled, not failed. Actions that cannot
  be interrupted say so in the editor.

- Workflow runs have their own project context. A project selected in a run
  affects the nodes after it — including in parallel branches, whose changes
  a Join merges or rejects when they disagree — but not DeckPilot's active
  project or other runs. Changing the active project is a permission per
  workflow ("May change the active project"), applied when the run ends and
  only if the active project did not change meanwhile. Workflows saved
  earlier keep their behavior.

- Release validation: test releases (e.g. `v0.5.0-1`) are verified like
  final ones and are marked as pre-releases, so they never become the
  release the app updates to.

### Fixed

- An open configuration app now follows the background agent when the
  configuration is opened elsewhere (another DeckPilot window, the system
  key becoming available) instead of waiting at its unlock prompt.
- Windows release builds: OpenSSL is configured with Strawberry Perl.
- Linux test releases build `.deb` and AppImage (RPM rejects pre-release
  versions).
- Connecting to the agent gives up after a few seconds instead of waiting
  indefinitely when its endpoint is busy or does not answer (on Windows a
  busy named pipe was waited for without a limit).

## [0.4.0] - Unreleased

### Added

- The configuration is always encrypted (SQLCipher, AES-256). By default the
  key is a random key kept in the operating system's credential store
  (macOS Keychain, Windows Credential Manager, Secret Service), so DeckPilot
  opens at login without a password. A password (Argon2id) can be chosen
  instead, and you can switch in both directions.
- Settings → Security: unlock method, password change, lock now, automatic
  locking (sleep, screen lock on macOS and Linux, inactivity), when to ask
  for authentication, Touch ID on macOS.
- Sensitive operations (export, changing the protection, security settings,
  deleting the unencrypted copy) ask for authentication first.
- Encrypted configuration export and import (XChaCha20-Poly1305, Argon2id)
  with a preview, merging or replacing, selected profiles, conflict
  resolution, module and application settings, and resources to map on the
  new computer. Machine paths, keys and passwords are never exported.
- Native windows for prompts, export, import, update progress, the agent log
  and the workflow summary, sized to their content.
- Encrypted, verified backups before database upgrades, a migration history
  and module-owned schema versions.

### Changed

- Configurations of DeckPilot 0.3 are encrypted on the first start; the
  unencrypted original is kept until you delete it in Settings.
- Updates are only installed if their signature names the announced version.

### Security

- A missing database is never replaced silently; changes to the database
  files are serialized between the background agent and the app.
- See [docs/security.md](docs/security.md) for what each protection mode
  guarantees and its limits.

## [0.3.0]

### Added

- Configuration studio: Action Library, workflow editor, profiles and
  projects, per-computer resource mappings, device models and a visual
  layout designer that works without hardware.
- Modular features (General, Projects) and an application updater based on
  signed GitHub releases.

## [0.2.0]

### Added

- The background agent with a system tray, Stream Deck MK.2 support and an
  authenticated local IPC.

## [0.1.0]

### Added

- Project foundation: domain model, execution engine, SQLite storage and the
  configuration app.
