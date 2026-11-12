# Data model

All entities serialize to JSON with camelCase field names. Entity identifiers
are random UUIDs. Textual identifiers are validated:

| Identifier     | Format                                         | Example              |
| -------------- | ---------------------------------------------- | -------------------- |
| `ActionTypeId` | `<module id>.<name>`, lowercase dotted         | `projects.next`      |
| `ModuleId`     | lowercase letter, then `a-z 0-9 -`             | `projects`           |
| `ResourceKey`  | lowercase dotted                               | `shop.repository`    |
| `ProjectKey`   | 1–32 ASCII letters, digits, `_`, `-`; unique case-insensitively per profile | `SHOP` |
| `DeviceModelId`| free-form                                      | `elgato.streamdeck.mk2` |

## Profile

```
Profile
├── id, name, description
├── actions:   ActionInstance[]
├── workflows: Workflow[]
├── projects:  Project[]        (stored in the `projects` table; order = cycle order)
├── defaultProject             (stored in `default_projects`)
├── layouts:   DeviceLayout[]   (any number per device model)
├── activeLayouts              (the layout used for each model)
└── icons:     Icon[]           (images for key icons; omitted when empty)
```

`Profile::validate` reports duplicate project keys, a default project or
project parameters that do not belong to the profile, workflow steps that
reference missing actions, invalid or duplicate icons, and layout problems. `Profile::duplicate` copies a
profile with new identifiers for everything in it and remaps all internal
references, so the copy is fully independent.

## Actions

- **ActionType** – defined in code by an `ActionHandler`: id, name,
  description, category, `parameters: ParameterSpec[]`, `requiresProject`.
- **ParameterSpec** – `key`, `label`, `kind`, `required`, optional `default`.
- **ParameterKind** – `text` (multiline, maxLength), `integer` (min, max),
  `boolean`, `choice` (options), `resource` (optional `ResourceKind`),
  `project` (a project of the action's own profile, stored by id).
- **ParameterValue** – tagged: `{ "type": "text", "value": "…" }`.
  Resource values hold a `ResourceRef`:
  - `{ "type": "fixed", "key": "shop.repository" }`
  - `{ "type": "projectRole", "role": "repository" }` – resolved from the
    active project
  Project values hold a project id: `{ "type": "project", "value": "<uuid>" }`.
- **ActionInstance** – `id`, `name`, `actionType`, `parameters`, optional
  `description`, `icon` (icon name) and `label` (key text). Created and edited
  through `ActionRegistry::validate_instance`, which validates parameters and
  fills defaults. The same action type can be configured many times with
  different parameters, each with its own stable identifier.
- **Usage** – where an action or workflow is referenced (workflow step or
  layout binding). Referenced actions and workflows cannot be deleted.

### Key labels

Action labels and binding labels may contain `{profile}`, `{project.key}` and
`{project.name}`. They are resolved against the active project whenever a key
is drawn, so a project switcher labelled `{project.key}` shows SHOP, CMR,
CMS, SHOP as it is pressed.

### Action types of the built-in modules

Action types are provided by [feature modules](modules.md#action-catalog).
A new installation has no actions; the Action Library only contains actions
the user created. `ActionResult` carries an optional `data` map with
structured details (for example the resolved project and resource).

## Workflows

`Workflow { id, name, description, steps: WorkflowStep[] }` with
`WorkflowStep { action, onFailure: "stop" | "continue", condition, overrides }`.

- `condition`: `always`, `previousSucceeded`, `previousFailed` or
  `activeProject { project }`. Steps whose condition is not met are reported
  as `skipped` and do not count as failures.
- `overrides`: parameter values that replace the action's own values for this
  step only.

Steps run in order in the agent; context effects of a step (such as a project
switch) apply before the next step. Requests from elsewhere are processed only
after the workflow finishes, so a workflow always sees a consistent context.
The result is a `WorkflowReport` with a `StepReport` per step.

## Execution

- **ExecutionContext** – profile, active project (optional), machine
  configuration.
- **ActionResult** – `status` (`succeeded` / `failed`), `message`, `effects`.
- **ContextEffect** – `cycleProject { direction }`, `selectProject { project }`.

## Projects

- **Project** – `id`, `key`, `name`, `resources: { role → ResourceKey }`.
- **ProjectSwitcher** – cyclic selection with wrap-around in both directions.
- **ActiveProjectContext** – bound to exactly one profile; using it with
  another profile is an error. Stale selections fall back to the first project.

## Machine resources

- **MachineConfig** – `os`, `bindings: MachineResourceBinding[]`.
- **MachineResourceBinding** – `key`, `kind`, `value`.
- **ResourceKind** – `directory`, `application`, `terminal`,
  `dockerEnvironment`, `service`, `url`, `other`.

Resolution errors: no active project, role not defined by the project, key not
bound on this machine, kind mismatch.

## Devices and layouts

- **DeviceModel** – `id`, `vendor`, `name`, `usb`, `capabilities`, `support`
  (`descriptorOnly`: no driver; `unverified`: driver not tested on hardware;
  `verified`).
- **DeviceCapabilities** – optional `keys` (rows, columns, image spec),
  `encoders` (count, pressable), `touch` (zones, image spec, gestures).
- **ControlId** – `{ "kind": "key", "index": 0 }`, `encoder`, `touchZone`.
- **DeviceLayout** – `id`, `name`, `model`, `homePage`, `pages` (in
  order), optional `explicitTree` (see [Pages and folders](#pages-and-folders)).
- **LayoutPage** – `id`, `name`, optional `parent` (the folder it is in),
  `bindings: ControlBinding[]`.
- **ControlBinding** – `control`, `trigger` (`press`, `release`,
  `rotateClockwise`, `rotateCounterClockwise`, `touch { gesture }`), `target`
  (`action`, `workflow`, `openPage`, `back`), optional `label`, optional
  `appearance` (see [Key appearance](#key-appearance)).

Folders are pages opened with `openPage` and left with `back`. Layout
validation checks that controls and triggers exist on the model, that each
control/trigger pair is bound at most once per page, that targets exist and
that icons a binding shows are in the profile (`MissingIcon`).

A profile uses its first layout for a model. `DeviceLayout::generate` creates
one from the profile's actions and workflows using only the model's
capabilities: items fill the keys in order; when they do not fit, the last key
opens a next page (“More”) and the first key of following pages goes back.

- **LayoutStatus** – per device model: `noProfile`, `unknownModel`, `missing`,
  `compatible { layout, name }`, `invalid { layout, name, issues }`.
- **KeyFace** – what a key shows: `style` (`empty`, `action`, `workflow`,
  `folder`, `back`, `notice`), `title`, optional `detail`, `highlighted`. A
  project switcher shows the active project as detail; a *Select project*
  key is highlighted while its project is active. Without a profile or layout
  the first key shows a notice and the others stay blank. Optional fields:
  `appearance` (the binding's, when it sets anything), and the live state
  `indicator`, `progress`, `toggle` (see
  [Button state](modules.md#button-state)).

### Pages and folders

A layout's pages form a tree: every page except the home page may be in a
folder (`parent`), and Back returns to the folder. `pages` is the order the
editor shows.

**Old layouts and inference.** Layouts written before `parent` existed
store no tree. Their tree is inferred when read: a page's folder is the
page that opens it first in a breadth-first walk from the home page
through `openPage` bindings (pages in layout order, bindings in page
order). Stored parents always win, inference never creates a cycle, and
pages nothing opens are top level. Reading or saving a layout without
structural edits changes nothing in it.

The first structural edit — reordering, refiling, moving or copying a
page — stores the whole tree (`DeviceLayout::infer_parents`) and sets
`explicitTree: true`. This is lossless (bindings and stored parents are
untouched; the tree stays the same) and idempotent (doing it again changes
nothing). With `explicitTree`, nothing is inferred and a page without
`parent` is top level. An editor that changes parents itself sets
`explicitTree` (or calls `page_tree` first to get the inferred parents).

**Validation.** Errors: a parent that does not exist (`UnknownParent`),
parents that form a cycle (`ParentCycle`), a parent on the home page
(`HomeHasParent`).

**Back.** A page inside a folder must have a way back: a Back binding or a
binding that opens one of its folders. A page without one is a *warning*
(`MissingBack`, from `DeviceLayout::warnings`), not an error, so older
layouts keep loading, saving and importing. For such a page the runtime
shows an automatic Back key on its first free key (lowest key index
without a binding; `DeviceLayout::automatic_back`) — never on a bound key;
the layout preview shows it too.

**Navigation.** Opening a page pushes it on the device's page stack. Back
goes to the current page's folder (also when the page was opened directly
from elsewhere); for a top-level page, back to the page that opened it.

**Moving and copying** (`Profile::move_page`, `Profile::copy_page`) take a
page with its bindings and every page inside it, at any depth:

- *Within a layout*, a move puts the page into another folder (`parent`;
  none: top level), keeping identifiers; moving a page into itself or one
  of its folders is refused (`PageCycle`). A copy gets new identifiers, sits
  next to the original unless a folder is given, and its links to pages
  inside the copied tree point to the copies.
- *To another layout*, the destination's model must support every copied
  binding (otherwise `IncompatiblePage`, nothing changes); the page goes
  into the given folder or the destination's home page. Links to pages
  outside the moved tree are refused (`OpensOutside`). A moved page keeps
  its identifiers, and keys of the old layout that opened it are removed.
- The home page cannot be moved (`HomePage`). Reordering must list every
  page once (`PageOrder`).

App commands (Tauri):

| Command | Arguments | Result |
| ------- | --------- | ------ |
| `page_tree` | `layout` (saved or not) | `{ pages: [{ id, name, parent, stored, depth, automaticBack }], warnings }` |
| `reorder_pages` | `profileId`, `layoutId`, `order` | the saved layout |
| `move_page` | `profileId`, `layoutId`, `pageId`, optional `targetLayoutId`, optional `parentId` | `{ page, removedKeys, layouts }` |
| `copy_page` | same | same (`page` is the copy) |

`save_layout` accepts `parent` and `explicitTree` and refuses the errors
above.

### Key appearance

`ControlBinding.appearance` is optional, and so is every field in it; unset
fields keep the look of the key's style. Bindings without it (all layouts
written before it existed) serialize exactly as before.

```json
{
  "background": "#102030",
  "textColor": "#ffcc00",
  "fontSize": "large",
  "titleAlign": "bottom",
  "showTitle": true,
  "icon": { "id": "<icon id>", "fit": "crop" }
}
```

| Field | Values | Default |
| ----- | ------ | ------- |
| `background`, `textColor` | `#rrggbb` (either case; stored lower case) | the style's colours |
| `fontSize` | `small` (the 8 px font), `medium` (8 px, 16 px from 120 px keys), `large` (one step above medium) | `medium` |
| `titleAlign` | `top`, `middle`, `bottom` | `middle`; `bottom` with an icon |
| `showTitle` | `false` hides the title and the secondary line | shown |
| `icon` | `{ id, fit }`: an icon of the profile; `fit` is `fit` (whole icon, aspect kept, transparent borders), `fill` (stretched to the key), `crop` (covers the key, aspect kept, overflow cut off) | no icon; `fit` |

Invalid colours or values make the layout unreadable and are refused when
saving; an icon id that is not in the profile is a layout issue. The app
does not store an appearance that sets nothing.

The key is drawn in this order: background, icon (scaled to the key's image
size), highlight border, title (in the text colour; with a dark shadow over
an icon), then the live state marks. The same renderer
(`crates/deckpilot-render`) draws device images in the agent and previews
in the app.

### Icons

```
Icon
├── id      IconId (UUID)
├── name    1–64 characters
├── width, height   pixels, at most 256 each
└── png     PNG bytes as base64, at most 192 KiB
```

- **Import** (`import_icon` in the app): PNG or JPEG only, recognised by
  content; at most 2 MiB and 4096 × 4096 pixels, checked by the decoder
  before it allocates (at most 80 MiB). The image is scaled down to at most
  256 pixels per side (never up) and encoded again as PNG, which drops all
  metadata (EXIF, text chunks, colour profiles); if the PNG is still larger
  than 192 KiB it is scaled down further. Only that PNG is stored. Importing
  the same image twice returns the existing icon.
- **Limits**: at most 64 icons per profile. An icon shown by a key cannot be
  deleted (`InUse`).
- **Storage and export**: icons are part of the profile document, so they
  are copied with profile copies and travel in encrypted exports. On import
  (`deckpilot-portability`), every icon is checked (byte size, PNG header,
  declared dimensions), decoded with the same limits and encoded again;
  an export with a damaged or lying icon is refused, and a key that refers
  to a missing icon makes the profile invalid.
- **Drawing**: the agent decodes the active profile's icons (with limits)
  when the configuration is loaded; an icon that cannot be decoded is left
  out and its key is drawn without it.

App commands (Tauri):

| Command | Arguments | Result |
| ------- | --------- | ------ |
| `list_icons` | `profileId` | `IconInfo[]`: `id`, `name`, `width`, `height`, `bytes`, `dataUrl` (PNG `data:` URL), `usages` |
| `import_icon` | `profileId`, `name`, `data` (the file as base64) | `IconInfo` |
| `delete_icon` | `profileId`, `iconId` | — (refused while in use) |
| `preview_layout` | as before, plus optional `render` | `ControlFace[]`; with `render: true` each also has `image`, the key as the device shows it (PNG `data:` URL at the model's image size) |

## Storage schema (version 7)

Migrations run in a transaction each; a failed migration leaves the database
unchanged.

- **Migration 2** adds `app_launches` (start history per component, used to
  detect the first start after an update) and rewrites stored documents in
  document format 2, which only adds optional fields.
- **Migration 3** moves projects out of the profile documents into their own
  tables, stores module state and renames action types to module namespaces.
  See [Migration 3](#migration-3) below.
- **Migration 4** converts the projects actions to the generic catalog. See
  [Migration 4](#migration-4).
- **Migration 5** adds `migration_history` (every upgrade with its backup)
  and `module_schemas` (the schema version of each module's own tables).
- **Migration 6** adds `workflow_executions` (execution history).
- **Migration 7** adds `credentials`: secrets that modules use by name
  (e.g. an HTTP request's `credential`). They are machine-local: not part of
  profiles, machine configuration or module settings, never exported, and
  never sent to the configuration app after saving.

Since version 0.4 the database is encrypted with SQLCipher; see
[security.md](security.md). Every upgrade of an existing database is
preceded by an integrity check and a verified, encrypted backup in
`backups/`, runs in one `IMMEDIATE` transaction that re-reads the version
(two processes opening an old database at once cannot both migrate it), and
is verified afterwards.

| Table              | Purpose                                              |
| ------------------ | ---------------------------------------------------- |
| `profiles`         | One row per profile; JSON document (format 3) + version |
| `projects`         | Projects: `profile_id` (cascade), `key`, `name`, `position`, `resources` |
| `default_projects` | Default project per profile                          |
| `active_projects`  | Active project per profile (runtime state)           |
| `machine_config`   | Single row; JSON document + document version          |
| `modules`          | Enabled flag and configuration per feature module     |
| `settings`         | Key/value: active profile, device brightness, update preferences |
| `app_launches`     | Recent start versions per component                   |
| `migration_notes`  | Changes a migration made to user data, shown in Settings |
| `migration_history`| Schema upgrades: from, to, backup file, result, time  |
| `module_schemas`   | Schema version of each module's own tables            |
| `workflow_executions` | Execution history: summary and safe node reports     |
| `credentials`      | Named secrets with a description; this computer only  |

Constraints keep projects inside their profile:

- `projects.id` is the primary key, so a project id exists once across all
  profiles; `(profile_id, key)` is unique, ignoring case.
- `default_projects` and `active_projects` reference
  `projects (profile_id, id)` with a composite foreign key. A profile's default
  or active project can therefore only be one of its own projects, and both
  disappear with the project.
- Saving a profile whose project id belongs to another profile fails
  (`ProjectOfAnotherProfile`) and writes nothing.

### Migration 3

Profiles are processed in id order; every decision depends only on the stored
data, so running the migration on the same data gives the same result.

- Each project stays in the profile whose document contained it. Projects are
  never assigned to another profile.
- If an earlier profile already uses a project id (possible when profile
  documents were copied by hand), the project gets a new id derived from the
  profile id and the old id (UUID v5), and every reference inside that
  profile's document follows. Other profiles are not touched.
- The same id twice inside one profile: the first project keeps it and all
  references; the later one gets a derived id.
- A key used twice inside one profile: the later project gets a `-2`, `-3`, …
  suffix.
- The default and active project are kept only if they belong to the same
  profile; otherwise they are cleared.
- `deckpilot.core.log` → `general.log`; `deckpilot.project.cycle` →
  `projects.next` or `projects.previous` (by its direction);
  `deckpilot.project.select` → `projects.select` with the project referenced by
  id instead of key; `deckpilot.resource.resolve` →
  `projects.resolve-resource`. Step overrides follow. A select action whose key
  matches no project of its profile keeps no project, so the user picks one.

Every change beyond a plain rename is recorded in `migration_notes`. Tests
cover a version 1 database, a version 2 database with every legacy action
type, and ambiguous data (shared ids, duplicate keys, a foreign active
project) including that a second run produces the same ids.

### Migration 4

Only documents change (document format 4); action ids, names, workflows and
layout bindings stay as they are.

- `projects.select`: `project` → `project_id`, also in step overrides. A
  select action without a project (its legacy key matched nothing) now
  reports the current project instead of failing; a note asks the user to
  review it.
- `projects.resolve-resource`: the `resource` parameter becomes a resource
  name. "Role X of the active project" becomes `resource = X`. "Fixed key K"
  becomes `project_id = P, resource = X` when exactly one project of the same
  profile maps X to K; otherwise the resource is left empty (the editor asks
  for it) and a note names the old key. Step overrides follow the same rules.

A test migrates every case and checks that ids, order and layout bindings are
unchanged.

## Compatibility between versions

- A DeckPilot build reads every older schema and upgrades it; it **refuses to
  open a newer schema** (`UnsupportedSchema`) instead of guessing, and the
  app explains which version is needed.
- Upgrades are one-way. To go back to an older DeckPilot, restore the
  encrypted backup that was made before the upgrade (`backups/
  deckpilot-schema<N>-<time>.db`, encrypted with the key in use at the time) in place of `deckpilot.db`,
  then install the older version. A failed application update never restores
  an older binary automatically, because it might not read the migrated
  database.
- Module tables are versioned separately (`module_schemas`); a module schema
  newer than the installed module is refused the same way.
- Rollback limitations: there is no in-place downgrade, ever. Restoring a
  backup discards everything changed after the upgrade (profiles, history,
  credentials stored since). The backup is encrypted with the key in use at
  the time; after a later switch between password and system key, the
  backup still needs the old protection. Schema 7 cannot be opened by
  0.5.0 (schema 5): the 0.5.0 agent refuses it and changes nothing.
- A wrong key or password never reinitializes, upgrades or backs up the
  database; the file stays byte-identical.
- A newer schema is refused before anything is written to the file — the
  version is read before the journal mode is switched to WAL, which would
  rewrite the header.
- A damaged file that opens with the right key (truncated, damaged pages) is
  reported as damaged (`Corrupt`), not as a wrong password, and is left as it
  is. A file whose first page does not authenticate cannot be told apart
  from a wrong key and is reported as one; it is not written either.

Tested with a real database created by the 0.5.0-2 tools
(`crates/deckpilot-storage/tests/upgrade_from_0_5_0.rs`, fixture in
`tests/fixtures/v0.5.0-2/`): the upgrade from schema 5 to 7 keeps profiles,
projects and their resources, actions, step workflows, layouts, the active
profile and default project, machine bindings and settings; it is preceded
by an encrypted backup that can be restored; reopening does not upgrade or
back up again; graph workflows, execution history and credentials added
afterwards are kept (and secrets are never readable in the file); a wrong
key changes nothing; a newer schema is refused without changes, also in
rollback-journal mode; a truncated or damaged copy is reported as damaged
and left byte-identical, without side files or backups. The real
0.5.0-2 agent given a schema 7 database is tested by
`tools/compat/matrix.sh` ([Compatibility](compatibility.md)).
Upgrades from schemas 1 to 4 are covered by the storage unit tests
(`migrates_a_version_one_database_without_losing_data`,
`upgrades_are_preceded_by_a_verified_encrypted_backup`); no database from a
public release older than 0.5.0 exists (none was published).
