# Changelog

All notable changes to DeckPilot. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). The section of a version is used
as the notes of its GitHub release and of the in-app update.

**Release status.** No version has been published yet. 0.5.0 exists only as
two signed draft prereleases (0.5.0-1 and 0.5.0-2) made to test signing,
notarization and packaging; they were never made public and are not offered
by the updater. The versions before 0.5.0 were never released either. 0.6.0
is the first version intended for publication.

## [0.6.0] - Unreleased

The first public release. It includes everything listed for 0.1.0 to 0.5.0
below.

### Added

- The app and the agent negotiate their protocol and the capabilities that
  come with it, so a newer app works with an older agent (and the other way
  round) for what both support, says which features need an agent restart,
  and never sends a request the agent cannot understand. Agents too old for
  this app are reported as needing an update.
- Structured forms for module actions: command arguments and Compose
  services are edited as reorderable lists (shown as the exact list passed
  to the program), HTTP headers and query parameters as rows with duplicate
  and secret warnings, the request body in a JSON editor, and credentials
  are chosen from those stored on this computer. Stored actions keep their
  format. Actions that change things say so before they are saved.
- Upgrading a 0.5.0 configuration is tested with a real database created
  by 0.5.0-2: everything is kept, the encrypted backup is restorable, and a
  wrong key or a newer schema leaves the file untouched.
- The agent reports running workflows. Updates wait for them, and
  restarting the agent from the app asks before interrupting them; nothing
  ends a workflow silently.

- System module: open applications, web pages, files and folders, run
  commands and check whether a program is running. Applications, folders
  and programs come from resources bound on each computer. Commands run
  without a shell, with an argument list, a time limit, bounded output and
  a minimal environment; running them is off until allowed in Settings ›
  System on this computer. Their output reaches later workflow nodes but is
  never kept in history.
- Git module: status, branch, changed files, last commit and remote state
  of a project's repository, and opening it. Uses the repository of the
  workflow's project or an explicitly chosen project, never changes the
  active project, and never changes a repository. Remote state is what was
  last fetched unless a refresh is asked for (`git ls-remote`, which
  changes nothing locally). Detached HEADs, branches without commits and
  missing remotes are reported as such.
- Docker module: Docker availability, containers, Compose project status,
  Compose up, down and restart, and container or service logs. Every
  command names its Docker context (remote contexts work too) and Compose
  projects come from project resources. Starting and stopping services is
  off until allowed on this computer; down never removes volumes; logs are
  bounded and never followed.
- HTTP module: requests (GET, POST, PUT, PATCH, DELETE with query,
  headers and a JSON or text body; typed status, headers and body; a value
  read by JSON pointer) and health checks. Requests to this computer and
  private networks are refused unless allowed on this computer; every
  address and redirect is checked. Credentials are stored on this computer
  and used by name — never in a workflow or an export, only over https,
  and never shown. Responses are bounded; TLS verification is always on.
- Notifications module: show a notification (title, text, severity) or a
  summary of the workflow run, as a desktop notification and in the status
  bar. Text is plain and bounded; a notification that cannot be shown does
  not stop the workflow unless marked critical. Preferences: desktop,
  status bar and the least severity shown.
- Development seed: *Start Work* and *End Work* example workflows that
  combine the Git, Docker, System and Notifications modules using only
  logical resources (development data only; a new installation stays
  empty).
- Action types declare what running them does (read-only, opens something,
  changes things) and failures carry a stable error code; both are shown in
  the workflow editor.
- GitHub, GitLab and Jira modules (read-only): the signed-in account,
  repositories and projects, pull and merge requests with review requests,
  checks and pipelines, Jira issues and saved filters, and opening their
  pages. Tokens are stored credentials used by name, sent only to the
  configured server over https and scrubbed from every result; rate limits
  are reported with the time to wait. Searches are built from fixed
  templates, never from free text.
- Sub-workflows: a workflow can declare typed inputs and outputs and be
  called from another workflow's *Call* node. Recursion is refused, nesting
  is limited, and sensitive inputs and outputs are never kept in history.
- Workflow triggers: workflows can start on a schedule (every few minutes,
  daily or weekly at a local time) or when the agent starts, a profile is
  activated, the project changes or a device connects or disconnects.
  Missed times are skipped, never caught up; a trigger never starts a
  second run of a workflow that is still running, and triggers can be
  paused per workflow.
- Workflow variables: typed variables local to one run, set with a *Set
  variable* node and read like any other output. Parallel branches must
  agree on a value before they join; sensitive values stay in sensitive
  variables.
- Live key state: actions can show a state on their key (an indicator,
  a short detail, progress, on/off). Git keys show the branch and whether
  it is clean, Docker keys show what is running, and workflow keys show
  progress while their workflow runs. States refresh only for keys that
  are shown, at most four at a time, and only changed keys are redrawn.
- Key appearance: background and text colour, font size, title position
  and custom icons per key. Icons are imported from PNG or JPEG, scaled to
  at most 256 px and re-encoded, which drops embedded metadata. The app and
  the agent draw keys with the same renderer.
- Layout editor: multiple selection, keyboard navigation, copy, cut, paste
  and duplicate (also across pages), undo and redo, drag and drop from the
  library and between keys, a context menu per key, and page navigation
  with a breadcrumb.
- Run details in the workflow builder: a timeline of the run with parallel
  nodes on their own lanes, the slowest nodes, retries with their waits,
  failure details with what the error code means, and previews of declared
  outputs (sensitive ones are never shown).
- New step workflows can be created from the workflow switcher and the
  title bar.
- Workspace module: open a file in an editor (VS Code, Cursor, JetBrains
  IDEs, Zed, Sublime) at a line and column, open up to ten web pages, and
  open a project's editor, repository, documents and pages at once from its
  resources. It uses no shell and changes nothing.
- Page tree: pages can be placed in folders, reordered, and moved or copied
  with their subpages, also to another layout when its device supports
  every key. Back returns to the page's folder; a page in a folder without
  a way back gets an automatic Back key on its first free key. Existing
  layouts get their tree inferred without being rewritten.
- Appearance in the layout editor: colours with a contrast hint, text size
  and title position, an icon library with usage counts, PNG and JPEG
  import checked before upload, and previews drawn by the same renderer as
  the device. Changes apply to every selected key and can be undone.

### Fixed

- Duplicating a workflow no longer briefly shows an empty editor.
- On Windows, the agent no longer fails to start when the previous
  instance is still closing its connection, and opening the configuration
  no longer fails while another process is writing to it.
- On Linux, starting a program that was written a moment earlier (for
  example by an installer) no longer fails with "Text file busy".
- Parallel branches of a workflow no longer wait for their slowest
  sibling: each node starts as soon as its inputs are ready, and a node
  that crashes fails the run instead of stalling it.
- Built-in actions: time limits longer than the engine allows are capped,
  switching the active project is marked as changing things, failures carry
  error codes, and every action icon exists in the app.

## [0.5.0] - Not released (draft prereleases 0.5.0-1 and 0.5.0-2 only)

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

## [0.4.0] - Not released

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

## [0.3.0] - Not released

### Added

- Configuration studio: Action Library, workflow editor, profiles and
  projects, per-computer resource mappings, device models and a visual
  layout designer that works without hardware.
- Modular features (General, Projects) and an application updater based on
  signed GitHub releases.

## [0.2.0] - Not released

### Added

- The background agent with a system tray, Stream Deck MK.2 support and an
  authenticated local IPC.

## [0.1.0] - Not released

### Added

- Project foundation: domain model, execution engine, SQLite storage and the
  configuration app.
