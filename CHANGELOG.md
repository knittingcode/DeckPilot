# Changelog

All notable changes to DeckPilot. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). The section of a version is used
as the notes of its GitHub release and of the in-app update.

**Release status.** The first stable release is 0.6.0 (2026-11-17). The
public pre-releases before it were 0.6.0-alpha.3 and 0.6.0-alpha.4
(0.6.0-alpha.1 and 0.6.0-alpha.2 were not published). 0.5.0 existed only
as two signed draft prereleases (0.5.0-1 and 0.5.0-2) made to test signing,
notarization and packaging, since deleted; they were never made public and
are not offered by the updater. The versions before 0.5.0 were never
released either. 0.6.0 is the first version intended for publication.

## [Unreleased]

### Changed

- Settings open inside the main window instead of a window of their own,
  and the gear is highlighted while they are shown. Settings reopen at the
  section shown last. The device editor keeps its page, key, inspector and
  unsaved changes while the settings are open.
- The toolbar no longer has back and forward buttons, and ⌘[ / ⌘] (Alt+← /
  Alt+→ on Windows and Linux) and the mouse's back and forward buttons no
  longer navigate. The rail, the device and profile selectors, the
  Marketplace, the settings and plugin entries lead to every view; the
  rail's Device Editor returns to the editor from anywhere.
- Settings and the app's pages use a quieter desktop layout: section
  headings, setting rows with the control on the right, hairline separators
  and inline status instead of cards, shadows and large padding. Controls
  move under their label in narrow windows.
- Settings › Profiles lists profiles in a compact table (name, last
  modified, status, actions) with Make active and a menu for rename,
  duplicate and delete; the profile being edited shows its description and
  counts as a short details list.
- The Dashboard, Action Library, Workflows, Projects, Work Session,
  Variables and the plugin Marketplace (catalog, plugin details, installed
  plugins, updates) follow the same layout.
- The settings sidebar is more compact and starts with its search; each
  settings page shows its title and description at the top.
- Scrollbars are thin and quiet everywhere: a 6-pixel thumb on a
  transparent track that darkens while the pointer is over the scrolling
  area and more while it is dragged. High-contrast modes use the system's
  colours.
- The navigation rail is always collapsed to icons, with each page's name
  as a tooltip and a clearer mark on the current page. The toolbar's sidebar
  toggle and the ⌘B / Ctrl+B shortcut are gone.
- The encrypted storage now uses SQLCipher 4.20 (through rusqlite 0.42),
  previously 4.19. The file format and key derivation settings are
  unchanged, so existing databases open as before. The compatibility test
  opens a database written by 0.8.0 with the new build, and a database
  written by the new build with 0.8.0.
- Keys on the device and in the editor's preview are drawn with the Inter
  font instead of the pixel font, and show their action's icon above the
  title (the icon the app shows for it; an imported icon still replaces
  it). Polish letters keep working.
- The device editor's preview is centred and scales with the window. Page
  navigation (previous, page, next, add page) sits directly under it;
  renaming, deleting, the page tree and Move or Copy are in Manage pages.
- The editor's Action Library uses compact rows with category headings; a
  drag shows the action's icon and name.
- System monitor keys show their value under the title: `23%`,
  `12.4/32 GiB`, `↓ 2.1 MB/s` / `↑ 340 kB/s`, `68% used`.
- The editor's preview draws each key at the size it is shown on the
  screen, Retina displays included, instead of enlarging the device's
  72-pixel image, so icons and titles are sharp. The device still gets
  images at its own size. Empty keys are drawn in the editor's key tone in
  both themes.
- Keys are drawn with a slightly smaller icon (about two fifths of the key
  above a title, a little over half without one), a little more room
  between icon and title, and a smaller status dot, on the device and in
  the preview.
- The device editor no longer has a "Preview with project" selector.
  Labels such as `{project.key}` show the profile's active project, as on
  the device. Projects, their assignments and project actions are
  unchanged, and the editor works the same with the Projects module
  disabled or not installed. The hint about project placeholders in the
  action editor appears only with the Projects module, and the Work Session
  controller is offered only with the Work Session module (keys that use it
  keep it).
- The editor's Action Library is always shown at one fixed width; it can no
  longer be collapsed or resized, and the width and collapsed state earlier
  versions remembered are forgotten. In a window narrower than the main
  window's minimum (860 × 560), which only some Linux window managers
  allow, it sits under the editor instead of beside it.
- The toolbar has no "Device Editor" title. The device and profile
  selectors appear only where they apply: in the device editor both; on
  the Action Library, Workflows, Projects and Work Session pages the
  profile; in the settings, the Marketplace, the dashboard and the Layouts
  page neither. Settings show "Settings / section" in the toolbar and their
  title once, on the page. Leaving the editor for any view and coming back
  keeps the device, profile, layout, page and selected keys.

- The configuration database is upgraded to schema 10 (an encrypted
  backup is made first). DeckPilot 0.8.1 and earlier cannot open the
  upgraded database; to go back, restore the `deckpilot-schema9-…` backup.
  Exports of selected profiles remain readable by earlier releases; a full
  export that includes layouts no profile uses requires this version.

### Added

- Plugin API 1.4: a plugin view may declare `context` (`none`, `device`,
  `profile` or `device+profile`) in `contributes.views`, the toolbar
  selectors the app shows with it (`none` when absent). The declaration
  gives the plugin no data and no permission. Manifests without it are
  unchanged; agents up to 0.8.1 (plugin API 1.3) refuse manifests that
  use it.
- The editor's preview shows live values (e.g. CPU usage) as the device
  does, read by the agent; it says when values are stale or unavailable.
  IPC protocol 11 (`watchKeyStates`); older agents and apps keep working
  without it.

- Shared layouts. Layouts are now resources of their own: one layout can
  be used by several profiles, and editing it changes it everywhere it is
  used, while each profile keeps its own projects, variables, workflows,
  layout order, default layout per device and page state. A new Layouts
  page in the navigation rail lists every layout with the profiles that use
  it and the devices it fits; layouts can be created, renamed, duplicated
  (an independent copy), assigned to or removed from profiles, and deleted
  after confirming the profiles that use them. Removing a layout from a
  profile never deletes it. Settings › Profiles lists each profile's
  layouts, and the device editor can open, add and switch between shared
  layouts.
- Switch layout and Back to previous layout (Navigation actions) move one
  device between the active profile's layouts without changing the
  profile; each device remembers its layout across restarts, and the way
  back is kept per device and profile. Switch layout keys are titled with
  the layout they show, and Back shows when there is nothing to go back to.

### Fixed

- Windows: the settings window, and any other window opened from the app
  (export, import, logs), was created but never shown, so Settings could not
  be opened and no profile could be created. Opening a window no longer
  blocks the thread the window needs.

### Security

- Key fonts are read with skrifa (maintained) rather than ttf-parser
  (unmaintained, RUSTSEC-2026-0192).

## [0.8.1] - 2026-11-19

### Security

- The plugin sandbox uses Wasmtime 50.0.2 (was 50.0.1), which fixes the
  Wasmtime advisories of 2026-11-16 (RUSTSEC-2026-0566 to 0573). One of
  them applies to DeckPilot's sandbox, which is built without Wasmtime's
  `gc` feature: a plugin module using a function subtype check could make
  its plugin host process abort (RUSTSEC-2026-0570). Each plugin runs in
  its own sandboxed host process, so the abort stopped only that plugin,
  never the agent or other plugins.

### Fixed

- `deckpilot-registry package` checks each file path the manifest names
  before reading it, so a path such as `assets/../../icon.svg` is refused
  without reading anything outside the plugin directory. Such a package was
  already refused; only the read happened first.

## [0.8.0] - 2026-11-18

### Added

- System monitor keys. The System module has four new read-only actions,
  CPU usage, Memory usage, Network throughput and Disk space, whose keys
  show the value live: `CPU 37%` with a chart of the recent values,
  `RAM 62%`, the bytes per second received and sent (of one network
  interface, or of all but loopback added up), and the free space of the
  volume a folder is on. A status mark turns amber or red at thresholds
  you choose (CPU and memory 70% and 90% by default, disk 20% and 10%
  free). Pressing the key reports the value, which later workflow nodes
  can use. The values are read only while such a key is shown, at most
  every 2 seconds (disk: every 30 seconds), once for all keys.
- Actions declare which controls they work on: keys, dials (encoders) or
  touch zones, and which inputs of each (press, click, long press, a turn
  either way, a tap…). Every built-in action does: one-shot actions work on
  keys; Next page, Previous page, Brightness and the project switching
  actions also work on a dial, once per turn or press. Actions of plugins
  work on keys, as before. See `docs/device-controls.md`.
- Plugin API 1.3: a plugin's manifest can declare which controls and
  inputs each of its actions works on (`controls`, for example a dial's
  turns), and up to four toolbar entries (`navigation`) that open views
  DeckPilot draws itself: the plugin's variables, its actions or its
  settings (`views`). A manifest using them needs `"pluginApi": "^1.3"`.
  Without `controls` a plugin action still works on keys only, and
  `device.requires` grants no dial. The Rust plugin SDK has
  `Action::controls(..)`. See `docs/marketplace/sdk/manifest.md`.
- Toolbar icons of plugins are checked when a package is built and
  installed: PNG or WebP up to 256 KiB and 512×512 pixels, or SVG up to
  64 KiB without scripts, event handlers, external, `data:` or
  `javascript:` links, style imports or entities. The agent re-checks and
  sanitizes them each time it serves one, and the app shows them only as
  images, with its own icon when there is none to show.
- The Device Editor explains which actions fit the selected control. In
  the Action Library, actions that cannot run from it stay visible but
  disabled, with the reason ("Requires a dial. The selected Stream Deck
  MK.2 key supports press actions only."), and **Compatible only** hides
  them. Dropping such an action on a control is refused, shown on the
  control and announced; the inspector offers each trigger only the
  actions that can run from it, says why a saved binding is kept but not
  run, and a refused save names the trigger as the editor does.
- Toolbar entries of enabled plugins, left of the notifications: each
  opens a view DeckPilot draws, the plugin's live variables, its actions
  in the Action Library or its settings. Their icons are shown only as
  images (DeckPilot's own icon when there is none), more than fit are
  under *More plugin views*, the order can be changed from an entry's
  context menu and is kept on this computer, and they go away when the
  plugin is disabled or uninstalled.

### Changed

- A key, dial or touch zone bound to an action that cannot run from it (a
  dial turn bound to a key-only action, for example) no longer runs it.
  The binding is kept as it is, shows as unavailable, and the app's status
  bar says why the first time it is used. Saving a layout refuses a new
  binding like that with the reason; bindings already saved stay and are
  listed, so every existing profile can still be saved. Imports keep them
  and warn. Nothing is removed or converted, and a binding works again as
  soon as a compatible action is installed. Key bindings are not affected:
  every built-in action still works on every key gesture.
- macOS: closing DeckPilot's window (the close button or ⌘W) now keeps the
  app running in the background and takes its icon out of the Dock, like
  Docker Desktop. The tray's *Open DeckPilot*, or opening DeckPilot from
  Finder, Spotlight or the Dock, brings the window and the Dock icon back
  at once, still unlocked. ⌘Q (Quit DeckPilot UI) and the tray's Quit work
  as before.
- A new main window. DeckPilot opens with the Device Editor. The toolbar
  at the top chooses the device (connected devices first, the other
  models as "Not connected") and the profile to edit, and has
  notifications, the Marketplace and the settings at the right; the
  Dashboard, the Action Library, Workflows and module pages are in a slim
  navigation rail at the left.
- In the Device Editor, the Action Library is a sidebar at the right that
  can be resized and collapsed (an overlay in narrow windows), and the
  key inspector is below the device. The device follows the light or
  dark theme.
- The settings open in a window of their own, at the page asked for, and
  now include Profiles. A theme chosen there applies to every window.
- The Marketplace opens from the toolbar as a view of the main window;
  installed plugins are also managed in Settings › Extensions › Plugins.
- Notifications are in the toolbar, with the number of unread ones,
  instead of the status bar.

### Fixed

- Opening the plugin database no longer takes its write lock when its
  schema is already current, and waits for another process's lock while it
  checks the key. Before, the agent and anything else opening it at the
  same time could wait on one another until one failed with "database is
  locked", and a busy database could be reported as a wrong key.
- Linux: the `.deb` and `.rpm` packages now depend on the AppIndicator
  library the background agent's tray icon uses
  (`libayatana-appindicator3-1` on Debian and Ubuntu; the `.rpm` requires
  `libayatana-appindicator3.so.1`, on Fedora from
  `libayatana-appindicator-gtk3`). Where it was not installed,
  for example on a fresh Ubuntu 24.04, the agent of 0.7.0 stopped right
  after starting ("Failed to load ayatana-appindicator3"), and the app
  showed that the background agent was not running. Installing
  `libayatana-appindicator3-1` by hand works around it in 0.7.0.
- macOS: the background agent no longer shows a DeckPilot icon in the Dock
  for a moment when it starts, or for the whole life of a second agent
  process that stops because one is already running.
- A plugin update whose module the plugin host refuses (for example one
  that does not implement the actions its manifest declares) is no longer
  installed: the update fails and the installed version keeps running.
  Before, the update was applied and every action of the plugin failed
  until it was rolled back by hand.
- Linux: the AppImage's background agent could not start on newer
  systems such as Ubuntu 24.04: the AppImage did not include the
  AppIndicator library, and the system's copy cannot be loaded next to the
  older GLib the AppImage carries ("undefined symbol:
  g_once_init_leave_pointer"). The AppImage now includes the library and
  the libraries it needs.
- The background agent keeps running without a tray icon when the tray
  cannot be created, for example when the AppIndicator library is
  missing, instead of stopping. The app can still be opened from the
  application menu.

### Security

- Plugin requests are refused for every address the plugin broker refuses,
  including tunnel and translation ranges (6to4, Teredo, local-use NAT64,
  site-local IPv6 and the 6to4 relay range), not only private ones.
- A plugin request under a server path the user allowed (such as
  `https://ghe.example.com/api/v3`) is refused when an encoded `/`, `\` or
  `.` (`%2f`, `%5c`, `%2e`) would make a `..` segment once a server
  decodes it, such as `/api/v3/..%2fadmin`, which leads outside that path.
- Control characters in a plugin's log messages are escaped, so a plugin
  cannot forge lines in DeckPilot's log.
- The plugin module the host runs is checked against its integrity record
  byte for byte as it is loaded, not only before it is read.

## [0.7.0] - 2026-11-17

### Added

- The plugin Marketplace. Settings › Extensions › Plugins lists the
  plugins of DeckPilot's plugin registry
  (<https://knittingcode.github.io/deckpilot-plugins/>): browse and search
  them, read each plugin's description, version and the permissions it
  asks for, and install it after agreeing to those permissions. Installed
  plugins can be enabled and disabled, configured, bound to keys like any
  other action, updated (with a new consent when an update asks for more),
  rolled back and uninstalled, with or without their settings.
- Plugins run as WebAssembly in a separate, sandboxed plugin host, now
  part of every installer (signed and notarized on macOS, with one
  entitlement, for executable memory). A plugin gets only what it declares
  and you granted: the network hosts it names, notifications, its keys and
  its own storage. It never reaches files, other programs or your stored
  credentials directly.
- The registry is checked on every refresh against its root keys, which
  are built into DeckPilot: its signed key set, index and timestamp, and
  every package by its size and SHA-256. A stale, rolled-back or tampered
  registry is refused, and plugins that are already installed keep
  working. Only a registry whose root-signed key set allows it may serve
  DeckPilot's own `deckpilot.*` plugins, and the plugins built into
  DeckPilot can never be replaced from a registry.
- The first Marketplace plugins, by DeckPilot: Air Quality (Polish air
  quality stations, data from GIOŚ), Markets (ECB euro reference rates,
  via Frankfurter), Pomodoro, Break & Hydration, and Memory (a game on your
  keys).

### Changed

- The GitHub, GitLab and Jira integrations and notifications now run as
  plugins bundled with DeckPilot. On the first start after the update,
  each configured integration moves to its plugin: its settings are
  copied, and a stored token or a server on a private network is used by
  the plugin only after you agree. The built-in integration keeps working
  until the move is complete, and again whenever the plugin fails.
  Configurations the plugin cannot serve yet, such as GitHub Enterprise,
  stay on the built-in integration.
- Plugins are on by default. Start the agent with `--disable-plugins`, or
  set `DECKPILOT_ENABLE_PLUGINS=0`, to turn them off; the built-in
  integrations then run as before.
- Updating from 0.6.0 does not change the configuration database: plugin
  state is kept in a separate encrypted database, created after you
  unlock DeckPilot. 0.6.0 can still open the same data, without the
  plugins' changes and without credentials entered only into a plugin
  (docs/marketplace/module-migration.md, "Rollback and downgrade").

## [0.6.0] - 2026-11-17

The first public release. It also includes the changes of the
unreleased versions 0.1.0 to 0.5.0, which are recorded in the project's
CHANGELOG.md.

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
- One variable registry: modules declare their own read-only variables in
  their namespace (`{{ projects.current.key }}`,
  `{{ workSession.elapsedSeconds }}`), the core keeps only system
  variables, and one resolver serves actions, workflows, keys and previews.
  `system.project.current.…` and `system.work.session.…` keep working
  exactly as before, as deprecated aliases that are never rewritten. A
  module's own ids are reported as unavailable while the module is
  disabled.
- Key gestures: a key can run a different action, workflow or page for a
  click, a double click and a long press, each with its own parameter
  overrides. A press resolves to at most one gesture, and a click waits
  only for the gestures the key uses. Timings default to 300 ms for a
  double click and 700 ms for a long press (Settings › Devices) and can be
  set per key. Keys without gestures, including every existing layout,
  keep reacting to the raw press as before.
- Stateful keys and the Work Session: a key can change its operation with
  the state of a built-in state machine. The Work Session key starts a
  session (running an optional start workflow), verifies the end with an
  optional verification workflow and finishes the session with a click
  once it is ready; the session is operated from the key only. The key
  shows the state and a live timer. The Work Session page shows the live
  state, the last verification and notices, holds the settings, assigns
  the key, and lists the history of sessions. Sessions survive agent
  restarts, and only finishing stops the timer.
- Shared variables: named, typed values (`{{ shared.… }}`) defined
  globally, per profile or per project and used by actions, workflows and
  keys. The Variables page in Settings lists every variable (core, module
  and shared) with search, filters, owner, access and the current value
  when it is safe to show; shared variables are edited there, and a
  variable that is still used cannot be deleted or retyped. A variable
  picker with live previews inserts references in action parameters and
  workflow node mappings. Sensitive values are never shown or exported.
- Keys that turn pages: *Next page* and *Previous page* move a device
  along the pages of its layout (the pages in no folder), with or without
  wraparound, and *Page indicator* shows where it is, such as 2/5. Any key
  label can use `{page.number}`, `{page.count}` and `{page.name}`. Existing
  layouts keep their pages and folders as they are.
- *Switch profile* on a key: the next or previous profile, or one specific
  profile chosen from the profiles that exist. Every connected device
  follows. A key whose profile was deleted says so and changes nothing.
- A folder can close by itself after one of its keys ran (a switch on the
  folder's page in the layout editor).
- A device comes back on the page it showed: when a profile becomes active
  again, and after the agent restarts. Folders are closed.
- Folders in the layout editor: drag *New folder* onto an empty key or
  choose it in the key's menu, double-click a folder key to enter it, and
  create folders inside folders. A key that already does something is
  never overwritten. Deleting a page asks first and says what goes with it.
- The layout editor's library is one searchable list with groups that
  fold, and it offers the actions that need no settings (*Next page*,
  *Switch profile* and the like) directly: dropped on a key, the action is
  created, or the one that exists is reused.
- *Brightness* on a key: set the display brightness of the devices to a
  level, or step it brighter or dimmer.
- Single click, double click and long press of a key are slots in the
  layout editor: drag an action, a workflow or a page onto one to set or
  replace what the gesture does, and remove it with one click.
- *Multi Action* in the layout editor: drag it onto a key or a gesture
  slot and add its steps right there, with delays between them (*Delay*,
  10 ms to 5 minutes). It is an ordinary step workflow, so it runs in the
  agent with progress, cancellation and history, and appears on the
  Workflows page too; *Edit steps* on a key opens any step workflow it
  runs. Workflows saved by earlier versions are read and kept unchanged.
- *Random action* on a key: runs one of several saved actions or
  workflows, picked at random, optionally with weights and never the same
  twice in a row. The pick runs as its own key would.
- *Multi Action Switch* on a key: two to eight states, each running an
  action, a workflow or a Multi Action, with its own title and glyph on
  the key. A press runs the current state and moves on when the run
  succeeds (optionally also when it fails); each key keeps its state
  across restarts. The states are edited in the Action Library.

### Fixed

- Key labels keep their accented letters: Polish (ą ć ę ł ń ó ś ź ż and
  their capitals) and the rest of Latin-1 and Latin Extended-A, `°`, `€`,
  and typographic quotes and dashes are drawn on the device instead of
  `?` (“Mgła”, “12°C”). Characters the key font still lacks are `?`, as
  before.
- Dragging one key onto another in the layout editor moves or swaps the
  keys again; it did nothing.
- HTTP, GitHub, GitLab and Jira actions run from a key or with *Run* in
  the app no longer end the agent (0.6.0-alpha.3); in workflows they
  already worked.
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
- Updates keep reaching installed versions now that the repository is
  named `knittingcode/deckpilot`: releases still announce their downloads
  under the address earlier versions accept, and the app accepts the
  repository's name in any case.
- A duplicated profile appears in the agent's profile list (the tray, the
  dashboard) at once, as a new one does; before, it showed only after
  something else made the agent reload.

## [0.5.0] - Not released (draft prereleases 0.5.0-1 and 0.5.0-2 only)

### Added

- Settings have their own navigation, grouped into General, Hardware,
  Security & Data, Extensions and Advanced, with one page per area. Back and
  forward work like in a browser and the place is remembered.
- Modules are managed in Settings › Modules: search, filters, and an
  explanation when a dependency prevents enabling or disabling one. Modules
  with settings get their own settings page.
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
