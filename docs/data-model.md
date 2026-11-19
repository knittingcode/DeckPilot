# Data model

All entities serialize to JSON with camelCase field names. Entity identifiers
are random UUIDs. Textual identifiers are validated:

| Identifier     | Format                                         | Example              |
| -------------- | ---------------------------------------------- | -------------------- |
| `ActionTypeId` | `<module id>.<name>`, lowercase dotted         | `projects.next`      |
| `ModuleId`     | lowercase letter, then `a-z 0-9 -`; a plugin's: `<publisher>.<name>` (exactly one `.`, 2-24 and 2-32 characters of kebab-case words) | `projects`, `acme.hello-weather` |
| `ResourceKey`  | lowercase dotted                               | `shop.repository`    |
| `ProjectKey`   | 1–32 ASCII letters, digits, `_`, `-`; unique case-insensitively per profile | `SHOP` |
| `DeviceModelId`| free-form                                      | `elgato.streamdeck.mk2` |

## Profile

```
Profile
├── id, name, description
├── actions:   ActionInstance[] (its own and those of its layouts; shared records, see below)
├── workflows: Workflow[]
├── projects:  Project[]        (stored in the `projects` table; order = cycle order)
├── defaultProject             (stored in `default_projects`)
├── layouts:   DeviceLayout[]   (the layouts it uses, in its order; shared records)
├── activeLayouts              (its default layout for each model)
└── icons:     Icon[]           (images for key icons; omitted when empty; shared records)
```

Since schema 10, layouts, action instances and icons are resources of their
own that profiles refer to; the `Profile` value keeps its shape (see
[Shared layouts](#shared-layouts)).

`Profile::validate` reports duplicate project keys, a default project or
project parameters that do not belong to the profile, workflow steps that
reference missing actions, invalid or duplicate icons, and layout problems. `Profile::duplicate` copies a
profile with new identifiers for everything in it and remaps all internal
references, so the copy is fully independent.

## Actions

- **ActionType** – defined in code by an `ActionHandler`: id, name,
  description, category, `parameters: ParameterSpec[]`, `requiresProject`,
  optional `controls` (per control kind, the inputs the action can be bound
  to, e.g. `{ "key": ["press", "click"], "encoder": ["rotate-clockwise"] }`;
  omitted: keys only, see [Device controls](device-controls.md)).
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

The label of any key (also of folder and Back keys) may contain
`{page.number}`, `{page.count}` and `{page.name}`: the position of the
device among the top-level pages of its layout (from 1), how many there
are, and the name of the page it shows. Inside a folder the position is
that of the page the folder belongs to, and the name is the folder's.

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
  `gestures.defaults`, 300 and 700 ms), optional `autoReturn` (see
  [Pages and folders](#pages-and-folders); omitted when off).
- **ControlBinding** – `control`, `trigger` (`press`, `release`,
  `rotateClockwise`, `rotateCounterClockwise`, `touch { gesture }`, `click`,
  `doubleClick`, `longPress`), `target` (`action { action, overrides? }`,
  `workflow`, `openPage`, `back`, `stateful { machine, rules? }`), optional
  `label`, optional `appearance`
  (see [Key appearance](#key-appearance)).

Folders are pages opened with `openPage` and left with `back`. Layout
validation checks that controls and triggers exist on the model, that each
control/trigger pair is bound at most once per page, that targets exist and
that icons a binding shows are in the profile (`MissingIcon`). With the
action types at hand (saving in the app, the agent), a binding whose action
cannot run from its control or input is `IncompatibleAction`: refused when
it is new, kept and reported when it was stored, never removed or
converted, and not run by the agent (see
[Device controls](device-controls.md)). Bindings are stored as before; the
document version is unchanged.

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

A profile uses its default layout for a model (`activeLayouts`), else its
first layout for the model; a device that switched layouts shows that one
(see [Switching layouts](#switching-layouts)). `DeviceLayout::generate` creates
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
The actions *Next page* and *Previous page*
([Deck actions](modules.md#deck-actions)) move along the top-level pages
and leave nothing of a folder on the stack.

**Folders that close.** A folder page with `autoReturn` closes after one
of its keys ran an action or started a workflow: the device returns to the
folder's parent, as Back would, at the moment the key is handled and
whatever the action's result. Keys that open a page or go back do not
close it, an unbound key does nothing, and a key that already changed the
page (*Next page*) is not followed by a second step. On a page outside
folders the setting has no effect. Layouts without it are stored exactly
as before.

**The page a device returns to.** The runtime remembers, per profile and
device, the top-level page the device showed when the profile was left;
when the profile becomes active again the device shows that page, with
folders closed, if it still is a top-level page of the layout, else the
home page. The first time a profile is shown, devices start on the home
page. The agent saves these pages in the setting `navigation.pages`
whenever they change and restores them after a restart, so a device comes
back on the page it showed. Only pages other than the home page are
saved: a configuration whose devices never leave their home page (or only
open its folders) gets no such setting. This is runtime state of the computer: it is
not part of a profile and is never exported. An unreadable value, a
profile that is gone and a page that is gone are ignored. A device that
reconnects while its profile stays active keeps its whole page stack, as
before.

**Multi Action Switch.** A `deck.multi-switch` action stores its states
in its `states` parameter, one per line: `action:<id>` or `workflow:<id>`,
then optionally ` | <title>` and ` | <glyph>` (a glyph name of the
renderer's set, e.g. `play`). Which state is current belongs to the key
binding (profile, page, control and switch action), so two keys bound to
the same switch go their own way; run from the app, the switch has a state
of its own. The agent saves the states in the setting `switch.states` (a
JSON list of `[profile, page, control, action, index]`) when they change
and reads them when it starts. Like the pages devices show, this is
runtime state of the computer: not part of a profile and never exported.
An unreadable value is ignored (every switch starts at its first state),
an index past the last state is the first state, and entries of switch
actions that were deleted are dropped when the states are next saved.

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

**In the layout editor** (`apps/desktop/src/layout-editor/`):

- *New folder*, dragged from the library onto an empty control or chosen
  in the control's menu, creates a page stored inside the open page
  (`parent`), named "Folder" ("Folder 2", … so that no two pages share a
  name), and binds the control to open it, as one undo step. A control
  that already does something is never overwritten: the drop is refused
  and the menu entry is disabled. Folders can be created inside folders;
  the page id is new and nothing refers to a page by its position.
- A double-click on a control that opens a folder (or Shift+Enter, or the
  menu) enters it; Back and the page location return.
- Dragging a control onto another one moves its bindings, labels, look and
  gesture timings; onto a control that does something, the two swap.
- *Delete page* asks first and says what goes with the page: its own
  controls, the controls on other pages that open it (they are cleared),
  and the pages inside it, which are kept and move to its folder. The
  deletion is part of the draft and can be undone until the layout is
  saved.
- A folder's title and icon are the label and look of the control that
  opens it.
- The library is one searchable list with groups that fold: the profile's
  actions and workflows, navigation (pages, Back, *New folder*) and, under
  "New: <category>", the action types that need no settings (every
  required parameter has a default), from enabled built-in modules:
  *Next page*, *Previous page*, *Page indicator*, *Switch profile* and the
  like. Dropping a type on a control creates an action of it with the
  type's defaults, saved to the profile at once (a control can only run a
  saved action), and binds it; a profile's action that already is exactly
  that (same type, default settings, no label of its own) is reused, so
  the same type on five keys is one action. Types that need a value (a
  URL, an application) are created in the Action Library.
- In the inspector of a key or pressable dial, single click, double click
  and long press are three slots. Each takes what is dragged onto it from
  the library (an action, a workflow, a page, or an action type, whose
  action is created as above), replacing what it held; a slot is emptied
  with its remove button, and the list in the slot does the same by
  keyboard. "New folder" and other keys are refused there. A drop that
  makes a key tell gestures apart asks before the single click starts to
  wait, as choosing from the list does
  ([gestures](gestures-and-work-session.md#22-key-modes)).

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
| `fontSize` | `small`, `medium`, `large`: 13 %, 15.5 % and 19 % of the key's shorter side (about 9, 11 and 14 px on a 72 px key) | `medium` |
| `titleAlign` | `top`, `middle`, `bottom`; `middle` draws no icon above the title | `bottom` under an icon (the key's own or the binding's); `middle` without |
| `showTitle` | `false` hides the title and the secondary line | shown |
| `icon` | `{ id, fit }`: an icon of the profile; `fit` is `fit` (whole icon, aspect kept, transparent borders), `fill` (stretched to the key), `crop` (covers the key, aspect kept, overflow cut off) | no icon; `fit` |

Invalid colours or values make the layout unreadable and are refused when
saving; an icon id that is not in the profile is a layout issue. The app
does not store an appearance that sets nothing.

The key is drawn in this order: background, the binding's icon (scaled to
the key's image size), a live state's chart, the key's mark, title and value
line (on a translucent band over the binding's icon), the highlight border,
then the live state marks. The mark is the state's glyph, else the key's
icon by name (`KeyFace.symbol`: the action's `icon`, else its type's; a
folder, Back, workflow or work session icon for those keys), drawn from the
renderer's built-in icons; a name it does not know is drawn as the
fallback icon (`zap`), as in the app. The binding's own icon wins over
both. Titles (Inter medium) wrap by width and end with `…` when cut; the
value line (the face's detail, Inter semibold, at most two lines) is set
slightly smaller before it is cut; characters outside the font are `?`.
Stored titles are never changed. The same renderer
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
| `preview_layout` | as before, plus optional `render`, `live` (`{ connected, states: KeyStateEntry[] }`) and `keyPixels` (the width a key is shown at in device pixels, clamped to 16-512) | `ControlFace[]`; with `render: true` each also has `image`, the key as the device shows it (PNG `data:` URL at the model's image size, or with `keyPixels` at that width, other displays scaled alike); `projectId: null` uses the profile's active project; with `live` keys show their reported state (stale ones, `connected: false`, with a grey mark; unknown values as `—`) |
| `watch_key_states` | `profileId`, `actions` (none: stop) | the watch's number; `key-states` events (`snapshot`, `changed`, `disconnected`, `unsupported`) carry it |

## Shared layouts

A **layout** (pages, folders, controls and what each control runs) is a
resource of its own, stored once with a stable id (never found by name). A
**profile** (projects, variables, workflows) *uses* any number of layouts,
and a layout may be used by any number of profiles. A **device** (its model's
controls) shows one layout of the active profile. Each profile keeps its
own order of layouts, its own default layout per device model and its own
page state; editing a shared layout, from any profile, edits the one
record, visible in every profile that uses it. Nothing is ever cloned
silently: *Duplicate layout* is the only way to an independent copy.

### Who owns the actions keys run

Today's model binds keys to action *instances* (`BindingTarget::Action
{ action }`), which used to live in a profile. For a layout shared by
profiles the decision is:

- **Action instances are shared records** (`library_actions`), like
  layouts. A layout's bindings name them by id; a profile sees the actions
  it has of its own (*its library*) plus those of the layouts it uses
  (`layout_actions`: what the layout's keys run, and the actions those name,
  e.g. a Multi Action Switch's states). So a shared layout behaves the same
  in every profile, with one record per action, and editing an action
  edits it for every profile whose layouts run it.
- **Profile context is resolved when an action runs**, never stored in the
  action: the active project and its resources, project roles, variables
  (`{{ profile.… }}`, project and profile values), the profile's name in
  labels. A key of a shared layout runs with the context of the profile
  that is active.
- **Workflows and projects stay the profile's.** A binding that starts a
  workflow, or an action or override that names a specific project (a
  *Select project* of SHOP), works in the profiles that have it and is
  *unavailable* in the others (`shared_layouts::availability`): the key
  says “Not in this profile”, nothing runs, and another profile's workflow
  or project is never used. This is not an error of the layout (the
  profile that has the workflow may be the one that uses the layout), so
  validation no longer refuses it (`DanglingTarget` is not reported for
  workflows; foreign project overrides are not `InvalidOverrides`).
- **Icons** shown by keys are shared records too (`library_icons`), with
  the same rules as actions; the limit of 64 icons applies to what a
  profile sees, and assigning a layout that would exceed it is refused.

Rejected alternatives:

- *Actions stay in the profile and a shared layout resolves action ids in
  the running profile.* Another profile never has the same instance id, so
  every key of a shared layout would be unavailable there; making it work
  needs copies of the instances in every profile, which diverge — the
  silent cloning the owner ruled out.
- *Layouts carry their bindings' action definitions inline.* An action used
  by two layouts, or by a layout and a workflow step, would need two copies
  (or one layout to own an action another one runs); editing one would not
  change the other, and the Action Library would lose actions keys run.
- *A single database-wide Action Library for every profile.* Every profile
  would see every other profile's actions, including those naming its
  projects; the per-profile library and its safety checks
  ([Deleting actions safely](#deleting-actions-safely)) would no longer
  hold. The chosen model is that library, scoped: a profile sees its own
  actions and those of the layouts it chose to use, nothing else.
- *Workflows as shared records too.* Workflows carry triggers, execution
  history and work session settings that belong to a profile; sharing
  them would make a schedule of one profile fire in another. Possible
  later; not needed for shared layouts.

### Rules

- **Creating** a layout: in a profile (the device editor, as before) or on
  its own (Layouts page), used by no profile until it is assigned.
- **Assigning** appends it to the profile's layouts; the profile then sees
  the actions and icons it uses. Refused for a model this build does not
  know (the layout is kept unchanged and not used) and when the profile
  would show more than 64 icons.
- **Removing it from a profile** (`unassign_layout`, or a profile saved
  without it) never deletes the layout. What the profile had only through
  the layout goes with it, except actions its own workflows and actions
  name, which become its own. A default it was falls back to the profile's
  first layout for the model; devices that showed it show that.
- **Deleting** a layout (`delete_shared_layout`) lists the profiles that use
  it first; the deletion must name exactly those (the user's
  confirmation), else nothing changes (`LayoutInUse` names the profiles
  that use it now). The profiles stay. The device editor's *Delete layout*
  deletes a layout only this profile uses and refuses one others use too.
  Deleting a profile keeps its layouts (they stay in the Layouts page,
  used by no profile).
- **Duplicating** (`DeviceLayout::duplicated`) gives a new layout id and new
  page ids; links between its pages, its folders and its home page follow
  the copies. Its keys run the same Action Library actions (as five keys
  run one action), show the same icons and start the same workflows.
  *Switch layout* keys keep the layout they name, also when that is the
  original: links to other layouts are never rewritten. Duplicating a
  profile copies everything, its layouts too, with new ids (as before).
- **Device compatibility.** A layout is made for one model and only ever
  used on it (`ModelFit::Native`): a Stream Deck MK.2 layout never runs on a
  Stream Deck +, even where the + has the keys, because their images and
  the controls around them differ. For every other model the app explains
  the fit (`shared_layouts::compatibility`): it has every control and
  input the layout binds (*fits*: duplicate it for that model), or which
  controls are missing. What a layout needs is derived from its bindings
  (`DeviceLayout::requirements`), never stored. Bindings that do not fit
  are kept, as before ([Device controls](device-controls.md)).
- **Runtime resolution** (agent): device → active profile → the layout the
  device switched to in that profile, while it is still assigned and made
  for the model, else the profile's default for the model, else its first
  layout for the model → page → binding (`Runtime::layout_of`). Only the
  active profile runs; a layout two profiles share is one set of bindings
  and actions, so nothing runs or subscribes twice.

### Switching layouts

The `deck` module's *Switch layout* (`deck.switch-layout`) and *Back to
previous layout* (`deck.previous-layout`) move one device between the
layouts of the active profile (see [Deck actions](modules.md#deck-actions)).
They are distinct from *Switch profile* (every device, another profile),
*Next page*/*Previous page* (pages of one layout), folders and their Back
key (a page inside a layout), and from the app's own navigation. Keys
only, pressed; not on dials or touch strips, which have no interaction
mapped to them.

- *Switch layout* names the layout by id (`layout`), optionally a page
  (`page`, the home page when empty or gone) and what Back does afterwards
  (`history`: `push`, the default, remembers the layout left; `replace`
  does not; `clear` forgets the whole way back). The layout must be
  assigned to the active profile (`layout-unavailable`) and made for the
  device's model (`layout-incompatible`); otherwise nothing changes and the
  key reports why. The profile, its projects and variables never change.
- The **way back** (`LayoutHistory`) is kept per profile and device, so two
  profiles sharing Main go their own ways (Work: Main → Teams, Home: Main →
  Media) and two devices too. Main → Teams → Meeting, Back, Back is Main. A
  switch to the layout shown adds nothing (at most it shows the page). A
  switch to a layout that is in the history *unwinds* to it (Meeting →
  Main after Main → Teams → Meeting leaves the history empty), so a layout
  is never in it twice and nothing loops. At most 16 entries; the oldest
  go first. Each entry keeps the pages the device showed, so Back returns
  to the same page (the home page if it is gone).
- *Back to previous layout* with an empty history does nothing and says so
  (“there is no previous layout”); it never falls back to another layout.
  Its key shows that it cannot go back (“No previous layout”) while the
  device has nothing to go back to; the app sees the layout Back would
  return to in the device's status (`DeviceInfo.previousLayout`, an
  optional field).
- A *Switch layout* key without a label of its own is titled with the
  layout it shows, so it follows a rename; one whose layout the profile
  does not use says “Not in this profile”. Back shows the back arrow
  (its type's icon). Labels and icons of the binding win, as for any key.
  Action handlers declare this with `ActionHandler::layout_navigation`.
  Entries for layouts that were removed from the profile, deleted or are
  for another model are dropped, on a reload and when Back passes them.
- **Persistence and lifecycle.** The layout each device switched to, per
  profile, is runtime state of this computer in `device_layouts` (never
  exported): saved by the agent when it changes, read after every start and
  reload, so a device comes back on its layout after a restart of the agent
  and follows the app (a new default for the model, or the layout removed
  or deleted, sends the device to the profile's layout for the model). Its
  foreign key to the profile's assignment means an invalid entry cannot be
  stored. The way back lives in memory for the session: empty after the
  agent starts. The app reads which layout a device shows from the agent's
  status (`DeviceInfo.layout`, per device).

### Storage (schema 10)

| Table | Content |
| ----- | ------- |
| `layouts` | id, name, model, document (`DeviceLayout`, item document version 1), created and updated time; `(id, model)` unique |
| `profile_layouts` | profile (cascade), layout, model, position, `is_default`; composite foreign key to `layouts (id, model)`; at most one default per profile and model (partial unique index) |
| `library_actions`, `library_icons` | one record per action instance and icon (item document version 1) |
| `profile_actions`, `profile_icons` | what a profile shows, in its order, with `own` (its library) or not (through a layout) |
| `layout_actions`, `layout_icons` | what a layout's keys run (and the actions those name) and show |
| `device_layouts` | profile, device, layout: the layout a device switched to; references `profile_layouts`, so it goes with the assignment |

Profile documents are written in document version 6 without `layouts`,
`activeLayouts`, `actions` and `icons`; `SqliteStore::load_profile` fills
them (`layouts::compose`) and `save_profile` stores them
(`layouts::decompose`), so the configuration app, the agent, imports and
the device editor work with `Profile` as before. Actions and icons that no
profile has and no layout uses are removed; layouts only by deletion.

**Migration 10** moves every profile's layouts, actions and icons out of
its document into these tables, each layout assigned to the one profile
that had it, in its order, with its active layouts as defaults. Nothing
becomes shared. Profiles are processed in id order; an id that an earlier
profile already used (documents copied by hand) gets a new one derived from
the profile and the old id (UUID v5) in the later profile, every reference
in that profile's document and its provenance follows, and a note is
recorded in `migration_notes`. The same id twice inside one profile: the
first item keeps it with every reference, the later one gets a derived id.
An active layout that does not exist (it had no effect) is not kept. Each
profile's `updated_at` stays. As every upgrade, it is preceded by the
verified encrypted backup and runs in one transaction.

### Export and import

An export still carries every selected profile with its layouts in its
order, its defaults, actions and icons inline, so its shape is the one
releases up to 0.8.1 read (payload version 2). A layout two exported
profiles use is in both with the same id and is one shared layout again
after the import. An export of everything also carries the layouts no
profile uses with their actions and icons (`library`, payload version 3,
which earlier releases refuse instead of losing them).

An import never overwrites a layout, action or icon that stays in use here
with other content: the imported one gets a new id, and every reference of
the import follows (bindings, defaults, *Switch layout* targets, switch
states). An identical record is reused (so re-importing a profile that
shares an unchanged layout shares it again), and one only profiles the
import replaces use is replaced. Replacing everything also removes the
local layouts no profile uses that the export does not have.

### App commands (Tauri)

| Command | Arguments | Result |
| ------- | --------- | ------ |
| `list_shared_layouts` | — | every layout: id, name, model, model name, pages, bindings, users (`profileId`, `profileName`, `position`, `isDefault`), fit per model |
| `create_shared_layout` | `modelId`, `name`, `assignTo?` | the layout |
| `rename_shared_layout` | `layoutId`, `name` | — |
| `duplicate_shared_layout` | `layoutId`, `name`, `assignTo?` | the copy |
| `delete_shared_layout` | `layoutId`, `confirmedProfiles` | — (refused if the profiles that use it differ) |
| `assign_layout`, `unassign_layout`, `set_default_layout` | `profileId`, `layoutId` | — |
| `reorder_profile_layouts` | `profileId`, `order` (every assigned layout once) | — |
| `profile_layouts` | `profileId` | the profile's layouts in order, with `isDefault` and the keys that cannot run in it |

## Storage schema (version 10)

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
- **Migration 9** adds the Work Session tables `work_session_settings`,
  `work_sessions`, `work_session_intervals`, `work_session_reviews` and
  `work_session_transitions` (see [Work sessions](#work-sessions)). It is
  additive: the new tables start empty and nothing else changes.
- **Migration 10** moves layouts, action instances and icons into shared
  tables (`layouts`, `profile_layouts`, `library_actions`,
  `profile_actions`, `layout_actions`, `library_icons`, `profile_icons`,
  `layout_icons`) and adds `device_layouts`; profile documents are rewritten
  in document version 6. See [Shared layouts](#shared-layouts).

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
| `profiles`         | One row per profile; JSON document (format 6: without projects, layouts, actions, icons) + version |
| `projects`         | Projects: `profile_id` (cascade), `key`, `name`, `position`, `resources` |
| `default_projects` | Default project per profile                          |
| `active_projects`  | Active project per profile (runtime state)           |
| `machine_config`   | Single row; JSON document + document version          |
| `modules`          | Enabled flag and configuration per feature module     |
| `settings`         | Key/value: active profile, device brightness, update preferences, the pages devices show (`navigation.pages`), the states of Multi Action Switches (`switch.states`) |
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
| `layouts`, `profile_layouts` | Shared layouts and which profiles use them, in order, with each profile's default per model |
| `library_actions`, `profile_actions`, `layout_actions` | Action instances, what each profile shows (and has of its own), what each layout uses |
| `library_icons`, `profile_icons`, `layout_icons` | Key icons, the same way |
| `device_layouts` | The layout a device switched to, per profile (runtime state of this computer) |

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
action_references` lists every use in the profile (including the layouts it
shares with other profiles): layout bindings (layout,
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
  sessions recorded since. Releases 0.6.0 to 0.8.1 (schema 9) refuse
  schema 10 the same way and change nothing; going back means restoring
  the `deckpilot-schema9-…` backup, which loses everything changed since
  (layouts created or shared, the layouts devices switched to).
- Profile documents are written in document version 6 from schema 10 on
  (layouts, actions and icons in their own tables); a build that reads at
  most version 5 refuses such a document. Migration 10 rewrites every
  profile document; documents of versions 1 to 5 decode unchanged.
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
`tests/fixtures/v0.5.0-2/`): the upgrade from schema 5 to 10 keeps profiles,
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
upgrade except the profile documents (rewritten by migration 10, checked
item by item), which adds `workflow_executions`, `credentials`,
`shared_variables`, `shared_variable_values`, `provenance` and the five
Work Session tables (all empty), the shared layout tables (holding what the
documents held; `device_layouts` empty) and one `migration_history` row,
and opening the upgraded database again changes nothing. They also make the upgrade fail at injected points (after
migrations 6, 7, 8 and 9, just before the commit, and while the backup is
written) and kill a child process at the same points with the transaction
open: in every case the database is still the schema 5 fixture row for row,
the backup made before the upgrade is complete and holds that state, an
unverified backup copy is never left under a backup's name (it is written as
`….db.partial` and renamed once verified), and the next start upgrades
normally. A failed integrity check after the commit (injected) leaves the
database at schema 10 marked `unverified` and refused, row for row
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
the five empty Work Session tables, and failures and kills after migration
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

A fourth fixture comes from the 0.8.1 release (`tests/fixtures/v0.8.1/`,
schema 9: `deckpilot-dev validation seed` of the tag, its machine paths
replaced with neutral ones). The row-by-row tests run on all four: every
row of every other table is kept, the profile rows keep id, name and time,
and every profile loads exactly as its old document said — layouts in
order, pages, folders, bindings, active layouts, actions and icons — with
each layout assigned to its one profile and no device layout stored.
`tests/shared_layouts.rs` checks the 0.8.1 upgrade through the API, that
the `deckpilot-schema9-…` backup restores the 0.8.1 state, ids two
profiles shared (copied documents) and the store operations of shared
layouts; `deckpilot-portability`'s `tests/shared_layouts.rs` the export and
import.

Upgrades from schemas 1 to 4 are covered by the storage unit tests
(`migrates_a_version_one_database_without_losing_data`,
`upgrades_are_preceded_by_a_verified_encrypted_backup`); no database from a
public release older than 0.5.0 exists (none was published).
