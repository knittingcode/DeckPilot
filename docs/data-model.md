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
  `bindings: ControlBinding[]`, optional `gestures: KeyGestureSettings[]`
  (`control`, optional `doubleClickMs` 150–1000, optional `longPressMs`
  300–5000; unset values use this computer's defaults, setting
  `gestures.defaults`, 300 and 700 ms).
- **ControlBinding** – `control`, `trigger` (`press`, `release`,
  `rotateClockwise`, `rotateCounterClockwise`, `touch { gesture }`, `click`,
  `doubleClick`, `longPress`), `target` (`action { action, overrides? }`,
  `workflow`, `openPage`, `back`, `stateful { machine, rules? }`), optional
  `label`, optional `appearance`
  (see [Key appearance](#key-appearance)).

Folders are pages opened with `openPage` and left with `back`. Layout
validation checks that controls and triggers exist on the model, that each
control/trigger pair is bound at most once per page, that targets exist and
that icons a binding shows are in the profile (`MissingIcon`).

**Gestures** (see `docs/gestures-and-work-session.md`). `press` and
`release` are raw edges and fire at once; every binding from before 0.7 is
one and stays one. `click`, `doubleClick` and `longPress` are recognized by
the agent's gesture engine, each with its own target; a control on a page
uses one kind or the other (`MixedTriggers`), and gestures need a control
that can be pressed and released. A click fires on the press when nothing
else is bound, on the release when a long press is, and after the
double-click window only when a double click is. `overrides` of an action
target replace some of the saved instance's parameters for that binding
only, with literals, `reference` or `template` values resolved when it
fires (no node outputs); they are checked like a workflow node's
overrides when the layout is saved (`InvalidOverrides`). A key's face shows
its `press` binding, else its `click` binding. Deleting an action is refused
while any binding, of any trigger, runs it.

**Stateful keys** (`deckpilot_core::stateful`). A `stateful` target asks a
built-in state machine for an operation; the only machine is `workSession`.
`rules` is a list of `{ state, gesture, operation }` with the machine's own
names (`idle`, `working`, … and `start`, `verify`, `finish`, …), and no
expressions; without `rules` the machine's default table applies and is not
stored. The agent picks the rule from the machine's state at the press of
the gesture and lets the machine apply it, so the binding never sets state.
Validation: a stateful target only on `click`, `doubleClick` or `longPress`
(`StatefulNeedsGesture`); every stateful binding of a key has the same
machine and rules (`InconsistentStateful`); and each rule's operation is
allowed in its state, each `(state, gesture)` appears once, its gesture is
bound to the machine on that key, and `finish` is only a `click`
(`InvalidStateful`). A stateful key references no action, workflow or
project, and travels with its layout on export and import unchanged.

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

## Storage schema (version 9)

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
- **Migration 8** adds `shared_variables`, `shared_variable_values` and
  `provenance` (see [Shared variables](#shared-variables) and
  [Provenance](#provenance)). It is additive: no profile document is
  rewritten, and no provenance or variable is created for existing data.
- **Migration 9** adds the work session tables `work_session_settings`,
  `work_sessions`, `work_session_intervals`, `work_session_reviews` and
  `work_session_transitions` (see [Work sessions](#work-sessions)). It is
  additive: the new tables start empty and nothing else changes.

Since version 0.4 the database is encrypted with SQLCipher; see
[security.md](security.md). Every upgrade of an existing database is
preceded by an integrity check and a verified, encrypted backup in
`backups/`, runs in one `IMMEDIATE` transaction that re-reads the version
(two processes opening an old database at once cannot both migrate it), and
is verified afterwards. The upgrade is recorded in `migration_history` as
`unverified` in the same transaction and becomes `ok` only once the
committed database passes its integrity check. If the check fails (or the
process stops before it ran), the database stays upgraded but is refused
with `MigrationFailed`, naming the backup, on every start until a check
passes; it is never used unchecked, and nothing is upgraded or backed up
again. Restoring the named backup returns to the previous state.

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
| `shared_variables` | Shared variable definitions: key, type, allowed scopes, default, `sensitive`, declaring module, origin |
| `shared_variable_values` | Values per scope key: global, a profile, or a project of that profile |
| `provenance`       | Where seeded or imported items came from, with a snapshot as created |
| `work_session_settings` | Per profile: start and verification workflow, approval lifetime, review threshold, elapsed format |
| `work_sessions`    | Work sessions: state, times, last verification, approval, heartbeat, notice; at most one active |
| `work_session_intervals` | Counted stretches of time of a session |
| `work_session_reviews` | Gaps of uncertain time flagged for review, and how they were resolved |
| `work_session_transitions` | Audit log of every state change of a session |

Constraints keep projects inside their profile:

- `projects.id` is the primary key, so a project id exists once across all
  profiles; `(profile_id, key)` is unique, ignoring case.
- `default_projects` and `active_projects` reference
  `projects (profile_id, id)` with a composite foreign key. A profile's default
  or active project can therefore only be one of its own projects, and both
  disappear with the project.
- Saving a profile whose project id belongs to another profile fails
  (`ProjectOfAnotherProfile`) and writes nothing.

### Shared variables

Definitions are database-wide (`shared_variables`, keyed by the variable key
without `shared.`, e.g. `projects.favorite`); values live in
`shared_variable_values`, at most one per scope key (unique index over key,
scope, profile and project). Values of the execution scope exist only during
a run and are never stored.

- Every write is checked with `VariableDefinition::check_value`: the value's
  type and that the definition allows the scope. A definition without scopes
  or with a default of the wrong type is refused.
- A profile value needs an existing profile. A project value references
  `projects (profile_id, id)` with a composite foreign key, so it can only
  name a project of its own profile, and disappears with the project or the
  profile. `projectId`/`profileId` values of a profile or project must name
  that profile's own data.
- Keys follow `VariableKey` (2–4 segments, lowercase start); the reserved
  roots `steps`, `system` and `workflow` cannot be keys at all, also not
  through a crafted row (unreadable rows are ignored). Keys a module
  declares (`declare_module_variables`, `module_id` set) belong to the
  module: a user variable cannot take, change or delete them, and a module
  cannot take a key a user variable or another module holds (reported, the
  holder is left alone). Users may use a module's group for other keys
  (`projects.favorite`).
- Changing a definition so that stored values would no longer fit (another
  type, a scope removed) is refused (`VariableValuesConflict`); values are
  never dropped silently. Deleting a variable deletes its values.
- A variable that is still used cannot be deleted, and its type cannot
  change (`VariableInUse`, with every use). `shared_variable_references`
  lists the uses in all profiles: reference and template parameter values
  of actions, step and node overrides, workflow mappings (template and
  whole-value), key and action labels, inline actions of bindings, and —
  indirectly — the triggers of the workflows that use it. Plain text
  values are never parsed, so `{{` in them is no use.
- A `secretRef` value is the name of a credential, never the secret, which
  stays in `credentials`. `sensitive` marks values that are only ever shown
  redacted.
- Values are literals and are never expanded again: `"{{ shared.a.b }}"`
  stored as a value is that text, so values cannot form cycles.
- `SqliteStore::shared_variables_for(profile)` returns a snapshot of the
  definitions, the global values and the values of that profile and its
  projects (never another profile's); its `resolve` applies the precedence
  execution > project > profile > global > default and resolves a
  `projectId`/`profileId` naming another profile as missing. A value changed
  later is seen by the next snapshot.
- Exports (payload version 2) carry the user's definitions and the values
  of the exported profiles and their projects; global values only with the
  application settings; values and defaults of sensitive variables never
  (decision D5, see [security.md](security.md#encrypted-exports)). An import
  creates missing definitions (origin `import`), keeps existing ones, and
  sets values in the same transaction as the profiles.

### Provenance

`provenance` records items the user did not create: `(profile_id, entity,
entity_id)` with `entity` one of `action`, `workflow`, `layout`, `project`,
`variable`, the `origin` (`seed`, `import`, `migration`), the catalog and a
stable `seed_key`, and `snapshot`, the canonical JSON of the item without
its id (keys sorted) as created. No row means the user created the item;
profile documents are not changed. An item is *edited* when its canonical
JSON differs from the snapshot (`provenance_status`). Rows go away with the
item (saving a profile without it) and with the profile.

The development seed (`tools/dev`) records a row for every action, workflow,
layout and project it creates (catalog `dev-seed`, keys such as
`work.action.next-project`). Only newly created data gets provenance. Data from before schema 8 —
including seeded data of development databases, such as the
`v0.5.0-2` fixture — has none and is treated as the user's: nothing removes
it automatically.

### Work sessions

The work session the agent runs (design: `docs/gestures-and-work-session.md`
§5–§6; contract: `deckpilot_core::work_session`). The store
(`crates/deckpilot-storage/src/work_sessions.rs`, methods on `SqliteStore`)
applies the transitions T1–T11 of the core table and nothing else. Times are
wall-clock milliseconds supplied by the agent.

- **One active session per database** (decision D7): `work_sessions.active`
  is `1` exactly while the state is not `stopped` or `aborted` (a `CHECK`),
  and unique. A second start is refused (`AnotherSessionActive`), also for a
  writer that bypasses the API.
- **Transitions** are one `IMMEDIATE` transaction each: the session is read,
  checked to be the expected session in the expected state (and, for
  results and approvals, the current verification id), updated with a
  compare-and-set on `(id, state)`, and logged in
  `work_session_transitions` (`idle → starting` for T1, `starting →
  aborted` for T3, `stopping → stopped` for T11). A request that does not
  match changes nothing and answers `Refused` (`UnknownSession`,
  `WrongState`, `StaleVerification`, `ApprovalExpired`,
  `ApprovalFromAnotherBoot`); a request already applied answers `Repeated`.
  So a result of an older verification, or an approval of an older session,
  can never move the current one.
- **Final stop** (T10): only from `readyToFinish`, with the session's
  current approval, given to the same agent instance (`approval_boot`), at
  most `approval_ttl_seconds` old (decision D9: 600 by default) and not from
  the future. It fixes `stopped_at` and closes the open interval in the same
  transaction. The same approval again answers `Repeated` ("already
  finished"), also after T11, because the approval id is kept.
- **Elapsed time is never stored.** It is the sum of `work_session_intervals`
  (`elapsed_ms` of the contract, an open interval counting up to now, a
  negative one as zero). The heartbeat only moves `last_alive_at`.
  Interval 1 opens at T2 (the start workflow is not counted, D10).
- **Gaps** (decision D8): a gap longer than `review_gap_seconds` (900 by
  default) while the timer runs is recorded in `work_session_reviews`
  (`agentDown`, `sleep`, `clockBack`, `clockForward`) with the notice
  `needsReview`, and stays counted. *Excluded* cuts it out of the
  intervals; *counted* on an excluded gap adds it back (except where another
  excluded gap still applies, and never before the start or after the stop),
  joining the split intervals again. Both rewrite the session's intervals in
  one transaction; elapsed time stays a plain sum. A clock set back added no
  time and cannot be excluded. The notice goes once no review is open.
- **Notices** (`problem`, `problem_message`, `problem_at`) belong to their
  session. While no session is active, the notice to show is that of the
  newest session (`idle_work_session_notice`, e.g. `startFailed` after T3);
  clearing it clears the notices of all finished and aborted sessions.
- **Verification details** are kept only as the non-sensitive text of the
  contract, without control characters and at most 200 characters.
- **Retention**: the newest 1000 finished or aborted sessions are kept, with
  their intervals, reviews and log; the active session is never removed.
- `work_session_settings` belongs to its profile (cascade). Sessions have no
  foreign key to the profile, so history outlives it; deleting a profile
  that has the active session is refused (`WorkSessionActive`).
- Machine-local runtime state: not exported.

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

## Deleting actions safely

An action is deleted only when nothing uses it. `SqliteStore::
action_references` lists every use in the profile: layout bindings (layout,
page, control, trigger), steps of step-list workflows, action nodes of graph
workflows, and — as indirect uses — the triggers of each workflow that runs
the action. `delete_action` refuses with `ActionInUse` and that list while
any use remains, and changes nothing.

Uses are removed only by an explicit choice, for all uses or for selected
ones (each must still be a current use, else nothing changes):

- `replace_action_references(from, to)` points bindings, steps and nodes at
  another action of the profile. Step overrides and node overrides and
  mappings are kept when both actions have the same type, and cleared
  (reported in `clearedSettings`) when the type differs.
- `unbind_action` clears the key bindings (the control becomes empty) and
  removes the steps. Graph nodes are refused: they are part of a connected
  graph, so they are replaced or removed in the workflow editor.

Each operation loads, changes and saves the profile in one transaction.
Triggers are never changed here; they follow their workflow.

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
  0.5.0 (schema 5): the 0.5.0 agent refuses it and changes nothing; schema
  8 cannot be opened by 0.6.0 (schema 7) in the same way. Going back from
  schema 8 means restoring the `deckpilot-schema7-…` backup; shared
  variables and everything else changed after the upgrade are lost. A
  build at schema 8 refuses schema 9 the same way; going back means
  restoring the `deckpilot-schema8-…` backup, which loses the work
  sessions recorded since.
- Profile documents are written in document version 5 from schema 8 on.
  Version 4 documents (and older) are read unchanged and are not rewritten
  by the upgrade.
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
`tests/fixtures/v0.5.0-2/`): the upgrade from schema 5 to 9 keeps profiles,
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
The storage unit tests in `src/upgrade_tests.rs` compare the same fixture row
by row: every row and table definition of schema 5 is unchanged after the
upgrade, which only adds `workflow_executions`, `credentials`,
`shared_variables`, `shared_variable_values`, `provenance` and the five
work session tables (all empty) and one `migration_history` row, and opening the upgraded database again
changes nothing. They also make the upgrade fail at injected points (after
migrations 6, 7, 8 and 9, just before the commit, and while the backup is
written) and kill a child process at the same points with the transaction
open: in every case the database is still the schema 5 fixture row for row,
the backup made before the upgrade is complete and holds that state, an
unverified backup copy is never left under a backup's name (it is written as
`….db.partial` and renamed once verified), and the next start upgrades
normally. A failed integrity check after the commit (injected) leaves the
database at schema 9 marked `unverified` and refused, row for row
unchanged by each refused start, with the backup intact; it is accepted and
marked `ok` once a check passes, and restoring the named backup returns to
the fixture row for row. A process killed between the commit and the check
is verified on the next start.

The same unit tests run on a 0.6.0 database (schema 7,
`tests/fixtures/v0.6.0/`, created by the 0.6.0 tools with a workflow run, a
credential, an active project and launches added so that every schema 7
table has rows): every row is kept, the upgrade adds only the empty
schema 8 and 9 tables, failures and kills after migration 8 or 9 or before
the commit leave the schema 7 database and its backup intact, and the next
start upgrades. `tests/upgrade_from_0_6_0.rs` checks the data through the API,
that shared variables work on the upgraded database, that the
`deckpilot-schema7-…` backup restores the 0.6.0 state (losing what changed
afterwards), and that a newer schema is refused without changes.

They run a third time on a schema 8 database (`tests/fixtures/schema8/`:
the 0.6.0 fixture upgraded by `main` at schema 8, plus shared variables
with global, profile and project values and a provenance row, so that
every schema 8 table has rows): every row is kept, the upgrade adds only
the five empty work session tables, and failures and kills after migration
9, before the commit or while the backup is written leave the schema 8
database and its backup intact. `tests/upgrade_work_sessions_from_schema_8.rs`
checks the variables and provenance through the API, that a work session on
the upgraded database survives a reopen (which neither upgrades nor backs
up again), that the `deckpilot-schema8-…` backup restores the schema 8 state
(losing the session), and that schema 10 is refused without changes. The
store's own tests (`src/work_sessions/tests.rs`) cover every transition and
refusal, the one-active-session constraint, the idempotent final stop (also
from two connections at once), approval lifetime and boot, stale and old
sessions' results, gaps, exclusion and re-inclusion, retention, and a
restart in each persisted state followed by its recovery transition.

Upgrades from schemas 1 to 4 are covered by the storage unit tests
(`migrates_a_version_one_database_without_losing_data`,
`upgrades_are_preceded_by_a_verified_encrypted_backup`); no database from a
public release older than 0.5.0 exists (none was published).
