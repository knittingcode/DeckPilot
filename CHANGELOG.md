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

### Added

- Live preview on the device: while the Device Editor shows a layout of
  the active profile, the selected device shows **and runs** the unsaved
  draft (titles, icons, colours, text appearance, gestures, action
  parameters, folders, pages), drawn by the same renderer as the Canvas;
  nothing is saved until Save, and other devices on the same layout keep
  the saved one. Navigation follows both ways: a double-click on a folder,
  Back, a page or a layout key moves the device (never running anything),
  and the device's own folder keys, protected Back, page keys, layout
  switches, Back to previous layout and Auto Exit move the editor. Save
  keeps the device where it is without redrawing it; Revert and Undo go to
  the nearest page that is still there. Closing the editor, the window or
  the app, or the app crashing, leaves the device on the saved
  configuration; after an agent restart the draft is sent again. Leaving
  the Device Editor, closing the window and quitting ask first over
  unsaved changes. IPC protocol 16 (capability `livePreview`); with an
  older agent the editor says that changes reach the device when saved.

- One key header and title editor for every key with a display (actions,
  Multi Actions, plugin actions, layout navigation and folder keys; the
  folder's protected Back key stays without one): at the top of the
  Inspector the key's live preview (the image the Canvas shows; clicking
  it opens the icon menu, never runs the key), its visible title (for a
  folder key apart from the folder's name) and background. The **T▾**
  button next to the title opens the title appearance popover: Show title,
  Top / Middle / Bottom, Reset, font, size in px, bold / italic /
  underline, text colour and the text layout (wrap, single line, auto fit,
  marquee). Every change shows on the Canvas at once and is an undo step;
  opening it changes nothing; Reset clears the overrides and never writes
  the device's defaults. The icon menu chooses from the profile's icon
  library, imports a PNG or JPEG file, pastes one from the clipboard (the
  WebView's clipboard; other formats are refused with a message) and
  removes the key's own icon. New optional appearance fields `fontFamily`
  (only fonts built into the renderer: Inter), `bold` (Inter SemiBold),
  `italic` (a 10° slant measured with its overhang) and `underline` (the
  font's underline), drawn by the renderer for the device and the preview
  alike; layouts without them look exactly as before
  (`docs/key-text.md`, "Title appearance").
- Plugin API 1.8: an action's `title` suggests a default title, says the
  title is dynamic, suggests a position and lists the parts of the look
  the editor offers; a title the user wrote now beats a plugin's dynamic
  title (`docs/marketplace/sdk/manifest.md`, "Titles").

- Restoring DeckPilot's own plugins (IPC protocol 15, capability and
  plugin runtime feature `restoreBundled`): Settings › Plugins › Installed
  lists the bundled plugins you uninstalled under *Removed DeckPilot
  plugins*, each with **Restore**, which installs the copy that comes with
  DeckPilot again (checked against the pins compiled into the agent, and
  the app's code seal on macOS, before anything changes), clears your
  earlier removal and brings back the settings and credentials you kept;
  a module that moved to the plugin runs on it again. A broken bundled
  plugin gets **Reinstall**, which replaces its files with that copy and
  keeps its settings, credentials and permission choices. The refusal of a
  plan that needs a removed bundled plugin now names this action. Older
  apps and agents are unaffected.

- A protected system Back key in folders: on every page inside a folder
  the first key with a display (key 0, top left) is DeckPilot's own Back,
  drawn and run by the agent (also with every plugin disabled) with one
  core vector icon (`crates/deckpilot-render/assets/icons/folder-back.svg`,
  no title), returning exactly one folder level. Nothing can be bound,
  moved, dropped, pasted or cleared there, in the editor and when a layout
  is saved; it is left out of multi-selection operations, and its menu
  offers only *Go back* and *Test*. Models whose keys have no display keep
  the host's navigation (`docs/data-model.md`, "The protected Back key").
- Auto Exit for folders (`autoExit`, 1–3600 s, off by default), set in the
  Inspector of the key that opens the folder: the agent leaves the folder
  one level up that long after a device entered it, also with the app
  closed. Moving within the folder does not restart it; leaving, another
  layout or profile, locking and disconnecting cancel it; every device
  counts on its own; nothing is stored.
- Database schema 11: migration 11 marks existing folders `reserved` when
  their first key is free or already goes back, and `legacy` (Back stays on
  the first free key, nothing moves, a migration note says how to change
  it) when it runs something else. "Move Back to the first key" in the
  editor moves that action to the first free key. Encrypted backup first,
  no binding lost; imports of older exports get the same rule.

- The Device Editor's interactive preview, backend half
  (`docs/editor-navigation-and-simulation.md`). Action types may declare
  `editorNavigation` (`folder.open`, `folder.back`, `page.next`,
  `page.previous`, `layout.switch`, `layout.back`, with the parameter that
  holds the target): the deck's *Next page*, *Previous page*, *Switch
  layout* and *Back to previous layout* do, and plugins may in their
  manifest (plugin API 1.7), as data only that the editor maps to its own
  navigation without calling the plugin. Preview faces say what pressing a
  key does to the view, the automatic Back key of a folder page included.
- Simulated presses (IPC protocol 14, capability `simulateInput`): the app
  can press a key of a connected device (single, double or long) through
  the agent's own gesture engine and dispatch, with the key's timings, on a
  lane separate from the device's physical keys and without writing
  anything to the device but the key images the action changes. Refused
  while locked and unless the device shows the profile, layout and page
  the app shows. It is not a user-initiated run (plugin dialogs and
  `url.open` stay refused), and a press that may change something outside
  DeckPilot (a command, a Docker change, an HTTP request other than GET, a
  plugin action that modifies, a workflow with such a step) runs only after
  the user confirmed it; the computer's settings and plugin permissions
  still apply. New Tauri command `simulate_press`.

- Plugin localization (plugin API 1.6, IPC protocol 13): a plugin package
  may carry translations as `locales/<tag>.json` (declared in the
  manifest's new `locales`, with `defaultLocale`), signed and pinned with
  the package's other files and checked at install (JSON objects of plain
  strings, at most 64 KiB and 32 files, no markup). Manifest texts may name
  keys as `"t:key.path"`; they are shown in the app's language with a fixed
  fallback (tag, its language, the plugin's default, English, then a label
  from the key), and plugin dialogs may use keys too. The Rust SDK has
  `Context::locale` and `Context::t` (with interpolation and plurals) and
  `MockHost::localized` (`docs/marketplace/sdk/localization.md`). A plugin
  reaches only its own translations.
- The agent follows the app's language: the app sends it with the new
  `setLocale` request (protocol 13, capability `locale`); the texts the
  agent draws on keys (*Back*, *Folder*, *Unavailable*, the default names
  of the deck actions such as *Next page*) and the tray menu are in English
  or Polish, and only the keys showing such texts are redrawn when the
  language changes; titles the user wrote never change. Plugin names,
  actions and settings update live. An app or agent of protocol 12 keeps
  working in English.

- Plugin dialogs (plugin API 1.5): a plugin with the new `dialogs`
  permission (medium tier) can ask DeckPilot to show a standard
  confirmation, alert, form or selection dialog and gets the user's answer,
  checked by the agent against the request. Dialogs open only in an action
  the user started, show the plugin's name, never DeckPilot's lock or shield
  icons, always have Cancel, and have no secret fields. At most one per
  plugin is open and six are shown a minute; disabling, updating or
  uninstalling the plugin closes its dialog. The Rust SDK has
  `Context::dialog` (`docs/marketplace/sdk/dialogs.md`).

- Text modes for key titles: in the device editor's Inspector, a key's
  Text appearance chooses **Wrap** (word boundaries, explicit line breaks,
  a maximum number of lines ending with “…”), **Single line** (cut with
  “…” or clipped, never smaller), **Auto fit** (shrinks only as needed, down
  to a minimum, on one or several lines; says when it does not fit even
  then) or **Marquee** (scrolls a title that is too wide, with pauses at
  both ends, at a chosen speed), plus a text size in points and left,
  centre or right alignment. One layout implementation in the shared
  renderer serves the preview and the devices, at every key size; templates
  and plugin labels are resolved before they are fitted, so plugins get the
  modes without changes. Scrolling keys animate only on the shown page of a
  connected device, at most 15 frames a second, without writing unchanged
  frames, and stand still in the preview when the system reduces motion
  (`docs/key-text.md`). Existing layouts keep their wrapped titles.

- Database schema 12: folder keys are core navigation controls. A key
  that opens a folder of its page has exactly one binding, a press that
  opens the folder at once (no gestures); saving, importing and the agent
  enforce it (`invalid.folderControl.*`). Migration 12 (encrypted backup
  first) keeps such keys opening and moves their other bindings to the
  layout's recovered list with a migration note; the key's Inspector shows
  only the folder's settings and "Recovered gestures (n)" with *Move to a
  free key* and *Discard*. Imports of older exports get the same rule with
  a warning (`docs/data-model.md`, "Folder controls").

- A shared **Icon Library** and **Background Library** for every
  configurable key. The Icon Library has 471 built-in Lucide icons in 19
  categories (offline, searchable, drawn by the keys' renderer, so the key
  looks as its tile), your imported icons (My Icons) and plugins' icons. The
  Background Library starts with Transparent, Black and White, then solid
  colours (any custom colour), built-in gradients, your images and plugins'
  backgrounds. Keys store only the asset's id; gradients and images are
  rasterised for the device. A key whose plugin asset is unavailable keeps
  it, draws the device default (no icon of its own) and draws it again
  once the plugin is back.
- Plugins can contribute icons and backgrounds (plugin API 1.8,
  `contributes.visualAssets`): PNG icons, PNG or JPEG images and gradients,
  packaged, signed and checked like every other package file; no plugin
  code runs for them.
- Plugin API 1.9, the platform for moving Git, Docker, Workspace, Projects
  and Workflows into DeckPilot's own plugins (reviewed in
  `docs/marketplace/module-migration.md` §7): manifest permissions `tools`
  (brokered `git` and `docker` limited to an allowlist of subcommands and
  options, with denylisted flags refused, separate network and modify
  grants), `resources`, `projectContext` and `hostViews`,
  `openUrl.fromParams`, `resource` parameters and the `workflows` and
  `projects` views, with their grant tiers, capability operations, broker
  checks and SDK calls. Only plugins shipped inside the app can declare
  them; third-party and registry packages are refused. The agent carries
  out `resource` parameters and `resource.resolve` (opaque handles bound to
  one invocation and plugin, labels only, the declared kinds only, project
  roles only with an active project and Projects on, audited) and answers
  the other operations as not implemented.
- Plugin API 1.9's project context: `project.current` and `project.list`
  (ids, keys and names of the active profile's projects), and
  `project.select` and `project.cycle` only in runs the user started, never
  from a lifecycle event or a schedule, so a plugin cannot loop on the
  switch it caused. A switch is applied like one from the app and its
  `projectChanged` event names the plugin. `profile.read` answers the
  granted scopes. Variables a plugin declares with a `host:project.id`,
  `.key` or `.name` source carry the project in the context, published by
  DeckPilot only while the plugin is enabled and holds `projectContext`.
  Every operation needs an active Projects feature and is audited.
- Plugin API 1.9's `tool.run`, carried out by the agent: the user's own
  `git` or `docker` (the executable, context and time limits set for the
  built-in Git and Docker modules, or the agent's `PATH`, plus Homebrew's
  and Docker Desktop's folders on macOS; the file must be named `git` or
  `docker`), never a shell, with every argument
  checked again at run time against the denylists, the host's table, the
  manifest policy and the plugin's own declaration (a manifest can only
  narrow), typed values that never start with `-`, and a canonical
  command line with the host's fixed prefix (`-C <handle> --no-pager
  --no-optional-locks` and hardening configuration: no `gpg` on `log`, no
  `ext::`, `file://` or remote helper transports, no credential prompts;
  docker's `--context`, and the compose file, folder and project name
  taken from the binding, never from the plugin). git works only on the
  bound folder's own repository (never one above it), compose only on a
  compose file inside the bound folder. The working directory is a
  resource handle of the invocation; the environment is an allowlist (no credentials, no git or
  compose overrides, `SSH_AUTH_SOCK` only for `ls-remote` with
  `tools:git:network` or an `ssh://` docker engine). `ls-remote` contacts
  only a remote `git remote` lists. Time limits per class (10 s local git,
  20 s network, 30 s docker reads, 300 s docker changes; the plugin may
  only shorten them), output caps (128 KiB, 64 KiB for logs, 16 KiB of
  errors), URL credentials (userinfo and token or password query
  parameters) removed from the output, at most 4 runs per
  plugin and one modifying run at a time; the process group is stopped on
  the deadline, an abort and a revoked grant. Docker changes need
  `tools:docker:modify` (off by default) and a run the user or their work
  session started. Every run is audited by subcommand, option names,
  handle label, exit status and byte counts, never by value. A program that
  leaves its process group no longer keeps a run waiting past its time
  limit.
- Plugin API 1.9: optional permissions. An action can name the sub-grants
  it needs (`requiresPermissions`: `tools:docker:modify`,
  `tools:git:network`, `resources:open`, `resources:paths`,
  `projectContext:select`, `openUrl:from-params`); the plugin is enabled
  without them, and only the actions naming one wait for it ("needs the
  permission …") until it is granted. The consent dialog and Settings ›
  Plugins show each one with its own switch, off by default when it is
  high risk. Docker's Compose up, down and restart name
  `tools:docker:modify`, so the Docker plugin's reads work without it.
- Plugin API 1.9: docker's `ps`, `logs` of a container, `version` and
  `context show` run without a resource, in an empty folder of the
  agent's own (the Docker plugin's status, containers and logs no longer
  need a Docker environment or a project); `resource.open` opens a file or
  folder inside a bound folder (`path`, kept inside it, links included),
  so the Workspace plugin's `path` works as the built-in's; a key state
  call can read which key it is (`run.context`'s `keyScope`), so Git and
  Docker keys that differ only in their resources keep their own state,
  and Compose up and down keys show their toggle again.

### Changed

- The GitHub, GitLab, Jira, Git, Docker, Workspace and Projects built-in
  modules are replaced by their bundled first-party plugins
  (`deckpilot.github`, `.gitlab`, `.jira`, `.git`, `.docker`, `.workspace`,
  `.projects`), now their only implementation
  (`docs/integration-modules.md`, "Moved to first-party plugins"). Existing
  keys, actions and workflows keep their ids and are never rewritten: they
  run again once the plugin is allowed (installed, enabled and, where it
  asks, consented in Settings › Plugins), and until then they are
  unavailable with the reason; there is no built-in fallback. An upgrade
  from 0.9.x migrates as before. Settings, credentials, projects and
  resource bindings are preserved. A build or a start without plugins has
  no project switching: the agent refuses it with "the Projects plugin is
  not active", and nothing is deleted.
- In builds with plugins, Workflows and Projects are what their plugins
  make them: the Workflows screen and its navigation entry only while
  `deckpilot.workflows` is enabled, and the Projects screen, the active
  project in the status bar, its dashboard card and every project control
  (project parameters, `{project.*}` hints, "while project" conditions, a
  workflow's project permission) only while `deckpilot.projects` is.
  Enabling or disabling the
  plugin shows or hides them at once. Nothing is deleted: keys that run
  workflows still run them, and graph workflows already on keys show
  read-only with the reason; Multi Actions stay the key editor's own. The
  agent refuses project requests while no project context is active
  (`projectsOff`), the app refuses workflow and project management while
  the plugin is off (`invalid.pluginFeatureOff`), and plugin answers carry
  the new optional `statusItems` and `dashboardCards` (no protocol
  change). Git, Docker, Workspace, GitHub, GitLab, Jira and Projects no
  longer have module cards or settings pages there: their plugins' pages
  replace them. Builds without plugins keep the Workflows screen.
- This computer's resource bindings moved from the Projects screen to
  **Settings › Resources**, which works without the Projects plugin (the
  core's `system.open-*` launchers use them).
- The key header: clicking the key preview opens one menu with an **Icon**
  and a **Background** section (library, import, paste, remove; background
  library, custom colour, reset to the device default); the Background row
  under the title is gone. **Show title** is a switch under the title
  instead of a checkbox in the title popover.
- A folder key's title is the folder's name: renamed in the key header,
  shown on the key, in the breadcrumbs and the Page Navigator; the separate
  folder name field is gone. A folder key title that differed from its
  folder's name is kept (schema 13 migration, after the encrypted backup,
  and imports) and offered as the folder's name or discarded.

- Keys that set no colours take their device model's defaults, the same
  for every kind of key: on the Stream Deck MK.2 (and XL and +) a
  transparent background on the black display and white text. Action,
  workflow, folder, Back and notice keys no longer get a dark tint of their
  own; a live state's accent colour and colours you chose are unchanged.
  The device and the editor's previews draw the same colours, and the
  editor's colour fields and contrast hint show the model's defaults. A
  key's background can now be set to **Transparent**, which versions up to
  0.9.0 cannot read.

- The device editor's Inspector shows only real settings: it is empty when
  nothing is selected and for a folder's protected Back key, has no
  instruction paragraphs, and leaves out actions a control cannot run
  instead of listing them disabled.

- A key title containing a line break now breaks there.
- The IPC protocol is version 12: plugin dialogs, sent only to an app that
  announces the `pluginDialogs` feature. Apps and agents of protocol 11 work
  with it unchanged.

### Fixed

- The Notifications, GitLab and Jira plugins that come with DeckPilot are
  now version 0.1.1: they were rebuilt with the current plugin SDK under
  0.1.0, so an installation upgraded from 0.9.0 would have kept its 0.1.0
  copy and shown the plugin as broken. A release check now refuses a
  bundled plugin with the version of a released one but other files.
- *Choose from icon library…* showed only the icons imported into the
  profile, so on a new installation it was empty: the library had no
  built-in collection and plugins' icons were never listed.
- The preview's **Test action** no longer says the device shows another
  page while it shows the editor's page (it compared the page's name with
  its id).
- An icon imported (or deleted) in the editor now reaches the device's keys
  at once: the agent reloads the configuration.

## [0.9.0] - 2026-11-20

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

- The plugin database (`plugins/plugins.db`, builds with plugins) is
  upgraded to schema 5, which records the plugins installed only as
  dependencies. An encrypted copy of the previous file is written to
  `plugins/.backups` first. DeckPilot 0.8.1 refuses the upgraded plugin
  database (its other features keep working); to go back, restore that
  copy as `plugins.db`. The configuration database is not changed by this.
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
- Plugin dependencies (experimental plugins, plugin API 1.4; the agent now
  provides 1.4). A plugin manifest can declare the plugins and built-in
  modules it needs, required or optional, each with a version range, and
  actions can name the optional ones they need. Installing a plugin from
  the Marketplace first shows the whole plan: every plugin and built-in
  module it needs, in install order, why each is needed, and each
  plugin's own permissions. Every package is downloaded and checked
  against the signed registry before anything is installed, nothing is
  installed until you confirm, and if a step fails, what was done is
  undone. A dependency never gets the permissions of the plugin that needs
  it. Optional integrations are installed only when you select them;
  without one, only the actions that use it are unavailable. Plugins that
  others need cannot be turned off or uninstalled on their own: the
  Marketplace shows which plugins are turned off with it. Plugins
  installed only as dependencies are offered for removal, never removed
  unless you check them. Updates and rollbacks never move a plugin to a
  version that another installed plugin does not accept. Plugin details
  list a version's dependencies and optional integrations with their state
  here. Manifests without dependencies work as before. See
  docs/marketplace/dependencies.md.

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
