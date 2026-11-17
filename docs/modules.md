# Feature modules

DeckPilot is built from feature modules. The core application (profiles,
persistence, IPC, the action engine, devices and layouts) knows nothing about
individual features. A module contributes:

- action types and their executors (`ActionHandler`s),
- navigation pages and dashboard cards in the configuration app,
- a configuration schema (edited in Settings › Modules),
- declared capabilities (what its code may do),
- dependencies on other modules and the platforms it supports.

Modules are compiled in and registered explicitly. There is no loading of
third-party executables and no remote installation.

In builds with the experimental plugin marketplace enabled, built-in
modules move to first-party plugins one by one
([marketplace/module-migration.md](marketplace/module-migration.md)).
The first is `github`: such a build runs the stored `github.*` actions in
the bundled plugin `deckpilot.github` once the user approves the use of
the token, and keeps the built-in as the fallback and for every
configuration the plugin cannot serve yet (GitHub Enterprise, private
hosts). Release builds keep plugins off, and so every module below.

## Built-in modules

| Module     | Pages | Capabilities |
| ---------- | ----- | ------------ |
| `deck` | — | change pages and profiles on the deck |
| `docker` | — (settings) | read profile, read machine resources, run programs (`docker`) |
| `general`  | — | read profile |
| `git` | — (settings) | read profile, read machine resources, run programs (`git`), open folders |
| `github` | — (settings) | send network requests, use stored credentials, open pages |
| `gitlab` | — (settings) | send network requests, use stored credentials, open pages |
| `http` | — (settings with credentials) | send network requests, use stored credentials |
| `jira` | — (settings) | send network requests, use stored credentials, open pages |
| `notifications` | — (settings) | read profile, show notifications |
| `projects` | Projects | read profile, change active project, read machine resources |
| `system` | — (settings) | read machine resources, open applications/files/pages, run programs, see running programs |
| `workspace` | — | read profile, read machine resources, open applications/files/pages |

The integration modules and their actions are described in
[Integration modules](integration-modules.md).

### Action catalog

Modules define generic action *types*; users create configured action
*instances*. No type or instance is ever generated per project or resource,
and nothing is added to the Action Library automatically.

| Type | Parameters | Result |
| ---- | ---------- | ------ |
| `deck.next-page` / `deck.previous-page` | `wrapAround` (default on) | shows the next or previous page of the layout on the device whose key ran it; outputs `pageId`, `pageName` when it moved |
| `deck.page-indicator` | — | nothing when pressed; its key shows the page as `2/5` |
| `deck.switch-profile` | `target` (`next` \| `previous` \| `profile`, default `next`), `profile` (a profile id), `wrapAround` (default on) | makes another profile the active one; outputs `profileId`, `profileName` when it switched |
| `deck.brightness` | `change` (`set` \| `brighter` \| `dimmer`, default `set`), `level` (0–100), `step` (1–100, default 10) | sets the display brightness of the devices or steps it, within 0–100; output `brightness` |
| `general.log` | `message` | the rendered message |
| `general.wait` | `seconds` (1–300) | waits; output `waitedMs`; stops at once when cancelled (the reference workflow node) |
| `projects.cycle` | `direction` (`next` \| `previous`, default `next`) | switches along the project order (respects `wrapAround`) |
| `projects.select` | `project_id` (optional) | with a project: selects it; without: reports the current project and changes nothing |
| `projects.current` | — | reports the current project: `projectId`, `projectKey`, `projectName`, `profileId` |
| `projects.repository` | `project_id` (optional) | resolves the project's `repository` resource |
| `projects.documentation` | `project_id` (optional) | resolves the project's `docs` resource |
| `projects.resolve-resource` | `project_id` (optional), `resource` (name) | resolves any named resource of the project |
| `projects.next` / `projects.previous` | — | *deprecated*: `projects.cycle` with that direction |
| `projects.active` | — | *deprecated*: `projects.current` without `profileId` |

A type marked `deprecated` (`ActionType.deprecated`, sent to the app only
when true) is replaced by another one. Saved actions, workflow nodes and
keys of that type keep working and can still be edited, but the Action
Library and the workflow builder no longer offer it for new ones.

### Deck actions

The `deck` module's actions act on the deck itself: the pages a device
shows and the active profile. A handler cannot reach the device runtime or
the agent, so each action *asks*, as the project actions do with
`ContextEffect`: its result carries a `DeckRequest`
(`crates/deckpilot-core/src/execution.rs`), which the registry lets
through only for a module with the capability `DeckControl`.

| Request | Applied by | Effect |
| ------- | ---------- | ------ |
| `NextPage`, `PreviousPage` | the device runtime (`Runtime::dispatch`) | moves the device whose key ran the action |
| `SwitchProfile` | the agent (`apps/agent/src/service/deck.rs`) | activates a profile through the path of `setActiveProfile` |
| `Brightness` | the agent | changes the brightness setting through the path of `setBrightness` |

A request never leaves the process: it is not serialized with the result,
so it is in no IPC answer, event or execution history, and a plugin (whose
results arrive as data) cannot make one. Who runs an action decides what
becomes of its request:

| Run from | Page request | Profile switch |
| -------- | ------------ | -------------- |
| a key or a gesture | applied to that device | applied |
| the app ("Run", IPC `runAction`) | fails with `no-device`: no device asked | applied (brightness too) |
| a workflow step or node | fails with `not-from-a-key` | fails with `not-from-a-key` (brightness too) |

An action never reports a success that did nothing.

**Pages.** A layout's *pages* are its top-level pages: the pages in no
folder, in layout order (`DeviceLayout::top_level_pages`). Next and
previous move along them and continue at the other end with `wrapAround`;
without it the last (or first) page stays and the action still succeeds
(“there is no next page”), as it does when the layout has one page. Inside
a folder they are relative to the top-level page the folder belongs to and
leave the folder. A page opened from a key of the home page of a layout
without a stored page tree is a folder of the home page, as before, so no
existing layout gains pages by itself. *Go to page* is not an action: a
key that opens a page is the `openPage` binding.

**Page indicator.** Key labels expand `{page.number}`, `{page.count}` and
`{page.name}` (see [Key labels](data-model.md#key-labels)) for every key.
The indicator is a key that shows `{page.number}/{page.count}` unless its
label uses a page placeholder itself.

**Profiles.** `target` chooses the next or previous profile in the order
the agent lists profiles (by name), or one profile by id. The agent has
one active profile, so every connected device follows. Switching to the
profile that is active, or stepping past the end without `wrapAround`,
changes nothing and succeeds. A profile that no longer exists fails with
`profile-not-found`; the action editor lists the profiles that exist and
marks a stored one that was deleted.

**Brightness.** There is one brightness for all devices, the setting of
the app's Devices page (`device_brightness`): the action changes that
setting, so the app shows the new level, it is kept across restarts, and
it reaches only devices whose model has a brightness. A step stops at 0
and 100.

### Cycling projects

`projects.cycle` is one action for both directions. The handler reports the
project it leads to and requests a `ContextEffect::CycleProject`; the engine
applies it against the project that is active *when it applies it*, and
reports the applied effect as the `SelectProject` it resulted in. The agent
handles key presses, IPC requests and tray commands one at a time, so N
presses — however quickly they arrive, from the device or several clients —
move exactly N projects on.

- No projects: fails with `no-projects`, nothing changes.
- One project: succeeds without a change (`"<KEY> is the only project"`),
  also without `wrapAround`.
- Without `wrapAround` the ends fail with `at-last-project` /
  `at-first-project`.
- The order is the profile's current order, so a reorder takes effect on
  the next press.
- The active project was deleted: when the agent reloads the profile, its
  default (or first) project becomes active; the next press continues from
  there. A context still naming a deleted project starts from the first
  (next) or last (previous) project.
- Each profile keeps its own active project: after a profile change, presses
  cycle the new profile's projects, and coming back shows and continues from
  the project the first profile had.
- On a key press the change goes to the global active project: the agent
  persists it, redraws the keys that show the project at once and sends a
  `state` event with the new `activeProject`. Inside a workflow it changes
  the run's project, and the workflow's `projectScope` decides whether it
  becomes the active project when the run ends.

`project_id` is a stable project id. Unset (absent or `null`) means *the
current project*, decided when the action runs; the editor's "Current
project" option stores exactly that, never the id of the project active at
configuration time. An explicit project must belong to the action's profile:
the app refuses to save anything else and the engine refuses to run it.

The three resource types share one resolver
(`modules/projects/src/resources.rs`). They only report what this computer
binds to the resource; opening a folder or a browser, running Git, Docker or
an editor belongs to the modules for those tools. Results are structured
(`ActionResult.data`): `projectId`, `projectKey`, `projectName`, and for
resources `resource`, `resourceKey`, `kind` and `value`. Missing resources
fail with a message naming the project and resource.

Failures carry the shared codes `no-active-project`, `not-permitted`,
`resource-role-missing`, `resource-unbound`, `resource-kind-mismatch` and
`invalid-parameters`, plus `project-not-in-profile` (an explicit project of
another profile), `no-projects`, `at-last-project` and `at-first-project`
(cycling without `wrapAround`). `projects.cycle`, `projects.next`,
`projects.previous` and `projects.select` are declared as modifying, since
switching the active project changes state; cycling is neither idempotent
nor retry-safe. The resource types, `projects.current` and
`projects.active` only read.

The projects module owns the Projects page, project management (create,
edit, reorder, delete, default project, explicit copy to another profile),
active project switching (actions, the tray and the app all step with
`ProjectSwitcher::step`, through `switching::step` outside the engine) and
project resources. Its setting `wrapAround` decides whether cycling wraps
around at the ends.

## Workflow node contract

Every action type is also a workflow node. This is the contract between a
module, the Rust engine (the only place actions execute) and the editor
(which only reads metadata and never runs module code). Contract version 1.

**Identity and versioning.** A node type is its action type id,
`<module>.<name>` (e.g. `general.wait`); the module id prefix ties it to its
module and the module's capabilities. `NodeSpec::version` versions the
node's parameter and output schema. Changing a parameter's meaning or an
output's key or kind requires a new version and a module migration
(`ModuleManifest::migrations`) for stored instances; adding an optional
parameter or an output does not.

**Parameter schema.** `ActionType::parameters`: key, label, description,
kind (`text`, `integer` with bounds, `boolean`, `choice`, `resource`,
`project`, and the structured text kinds below), required, default.

Structured text kinds are stored as plain text — exactly the format older
versions stored — and edited with structured controls that every module
gets from its schema: `lines` (a list, one item per line, items kept as
typed; e.g. command arguments, Compose services), `pairs` (name/value
rows, one `name<separator>value` per line, with duplicate and secret-name
checks; e.g. HTTP headers and query parameters), `code` (e.g. a JSON body
with a validity hint) and `credential` (the name of a credential stored on
this computer, chosen from a list; secrets are never loaded). The engine
validates them the same way when an action is saved and maps workflow
values into them as text. The engine validates and normalizes
parameters before a handler runs; handlers receive only valid values.

**Input schema.** A node's inputs are its parameters. Each can be the
action's configured value, a value for this node (`overrides`), or a data
mapping from earlier nodes' outputs (one output, or text with
`{{ steps.<node>.output.<key> }}`). Mapped values are converted to the
parameter's kind; the engine checks references against earlier nodes'
declared outputs. Resource parameters cannot be mapped.

**Output schema.** `NodeSpec::outputs`: key, label, kind (`text`,
`integer`, `boolean`, `path`, `url`, `projectKey`), whether it is always
present on success, and whether it is `sensitive`. Outputs are the `data`
of a successful `ActionResult`. Only declared outputs are passed to later
nodes' reports, events and history (safe output serialization); sensitive
outputs (a program's output, a response body) reach later nodes but never
events or history. Messages are shortened to 500 characters. Never put
secrets in outputs or messages. Build outputs with `OutputSpec::new(..)`
and `.optional()` / `.sensitive()`.

**Effect.** `NodeSpec::effect`: `readOnly` (default), `opens` (opens
something for the user) or `modifies` (changes state). Shown in the catalog
and the inspector.

**Execution context.** `ActionHandler::execute(parameters, context)` gets
an `ExecutionContext`: the profile, the node's project (the run's own
project context, not necessarily DeckPilot's active project), this
computer's resource bindings — each available only with the matching
capability — and, in workflows, the attempt's cancellation token and a
progress reporter. Handlers do not mutate state; they request changes as
`ContextEffect`s (e.g. `SelectProject`), which the engine applies to the
run's context after checking the module's capability.

**Capability permissions.** The module manifest declares capabilities
(`ProfileRead`, `ProjectContext`, `MachineResources`, `OpenExternal`,
`ProcessExecution`, `ProcessInspect`, `Network`, `Notifications`,
`Credentials`, `VariablesRead`, `DeckControl`); the context refuses the
rest (`CapabilityDenied`). Core
helpers check them (`resolve`, `run_process`); for the others a handler
calls `context.permit(capability)` before acting. Actions that are risky
even for a permitted module, such as running user-chosen commands, are
additionally off until a machine-specific module setting allows them. A node that selects projects declares
`changesContext`; it changes the run's context only. Changing DeckPilot's
active project is never a node's decision: it is the workflow's project
scope (see [Workflow graphs](workflow-graphs.md#project-context-and-the-active-project)).

**Cancellation.** In a workflow, every attempt has a token
(`context.cancellation()`), cancelled when the run is cancelled or the
attempt's timeout passes (`CancelReason::Cancelled` /
`DeadlineExceeded`). Long-running handlers check `context.is_cancelled()`,
wait with `context.wait(duration)` instead of sleeping, run programs with
`context.run_process(command, timeout, limits)` — `deckpilot_core::process::run`
bounded by the action's own time limit, so a single action started from a
key is bounded too — (which stops the program's process group:
`SIGTERM`, `SIGKILL` after a grace period; the process tree on Windows; no
orphans, nothing outside the group), and return promptly. They declare
`NodeSpec::cancellable = true`. A handler that cannot be interrupted keeps
the default `false`; the editor then says that it finishes before a
cancellation or timeout takes effect, and nothing claims otherwise. After a
cancellation the agent waits up to 10 s for the run to stop; a run still
busy with a non-interruptible action is then recorded as cancelled, and the
action's late result is discarded.

**Timeout.** `NodeSpec::timeout_ms` (default 30 s; a node may override it).
A node's time limit is at most 10 minutes (`MAX_TIMEOUT_MS`), whether it is
the action type's own or the node's override: action types declare at most
that, and the engine caps anything longer. An action's own longer limit — a
command's `timeoutSeconds`, the Docker module's time limit — still applies
when it runs outside a workflow, e.g. from a key. Cooperative handlers stop
at the deadline; a result arriving late is a timeout and its effects are
discarded.

**Retryability.** `NodeSpec::retry_safe`: the engine retries a failed
attempt (up to five times, exponential backoff, cancellable waits) only when
it is safe; otherwise retry settings are refused. Each attempt starts from
the node's input context.

**Idempotency.** `NodeSpec::idempotent`: running twice has the same effect
as once. Informational for users and future replays.

**Structured errors.** A failed node reports `StepError { code, message,
retryable, actionCode }` with stable codes: `action.failed`, `timeout`,
`cancelled`, `mapping`, `context-conflict`, `variable-conflict`, `unavailable`, `loop.failed`,
`subworkflow.failed`.
For `action.failed`, `actionCode` is the action's own code
(`ActionResult::error(code, message)`), documented by its module, e.g.
`exit-nonzero`. Cancellation is
never reported as a failure or a timeout, and a timeout never as a
cancellation.

**Progress events.** `context.report_progress(percent, message)` emits
`nodeProgress` (percent 0–100 optional, short safe message); the editor
shows it on the running node. Outside workflows it is ignored. The engine
emits `nodeStarted`, `retrying` and `nodeFinished` itself.

**Backward compatibility.** Every `NodeSpec` field has a default, so action
types that predate a field keep working (`cancellable` defaults to `false`,
the honest value). Stored graphs carry a format version
(`WorkflowGraph::version`); workflows without a graph run as steps.

**Reference implementation.** `general.wait` (`modules/general/src/lib.rs`)
implements the whole contract: a bounded integer parameter with a default,
a declared typed output, its own timeout, `retrySafe`, `idempotent`,
`cancellable`, cooperative waiting through the context and progress
reports. Its tests show each part.

## Where things live

| Part | Location |
| ---- | -------- |
| `ModuleManifest`, `FeatureModule`, `ModuleRegistry`, `Capability` | `crates/deckpilot-core/src/module.rs` |
| Button state types (`ButtonState`, `RefreshPolicy`, `StateError`) | `crates/deckpilot-core/src/button_state.rs` |
| Scheduling and caching of button states in the agent | `apps/agent/src/service/states.rs` |
| Action dispatch and capability checks | `crates/deckpilot-core/src/registry.rs` |
| Module crates | `modules/general`, `modules/projects` |
| List of built-in modules | `crates/deckpilot-modules` (the only place that names them) |
| Module state | `modules` table (`ConfigStore::module_states`) |
| Backend commands of a module | `apps/desktop/src-tauri/src/modules/` |
| Frontend of a module | `apps/desktop/src/modules/<id>/`, registered in `src/modules/index.ts` |

## Rules

- **Namespaces.** Every action type id is `<module id>.<name>`. Registering a
  module whose handler uses another namespace fails. The owner of any stored
  action can therefore be found from the action alone: everything before the
  last `.` (`ModuleId::owner_of`). Built-in module ids never contain a `.`,
  so built-in action types have two segments; a plugin's module id is its
  plugin id (`acme.hello-weather`), so its action types have three
  (`acme.hello-weather.greet`, experimental, `docs/marketplace/`).
- **Dispatch.** The engine executes actions through the `ActionRegistry`
  built from the enabled modules. There is no list of action types in the
  core.
- **Capabilities.** A handler only sees what its module declared. The
  `ExecutionContext` a module receives is restricted to its capabilities:
  `profile()` and `project()` require `profile-read`, resource resolution
  requires `machine-resources`. Handlers return effects instead of changing
  state; the registry rejects an effect the module did not declare (a project
  switch needs `project-context`), and the engine only selects projects of the
  active profile.
- **No side doors.** Handlers are only reachable inside the core. Execution,
  profile checks (`check_parameters`) and key decoration (`decorate_face`) all
  go through the registry, which applies the same capability limits.
- **Profile checks.** `ActionHandler::check` validates parameters against the
  action's profile, e.g. that a selected project belongs to it. The app
  refuses to save such actions and the engine refuses to run them.
- **Key faces.** `ActionHandler::decorate_face` lets a module add context to
  a key, such as the active project below a *Next project* key or the
  highlight of a *Select project* key. It runs on every redraw and must not
  do any work; live state that needs work (running a program, reading a
  file) is a [button state](#button-state).

## Button state

A module can show live state on keys that run its actions: a repository's
branch, whether a Compose project is up. The contract
(`crates/deckpilot-core/src/button_state.rs`, `ActionHandler` in
`registry.rs`):

```rust
pub trait ActionHandler {
    // ...
    /// `None` (the default): no live state; `button_state` is never called.
    fn state_policy(&self, parameters: &ActionParameters) -> Option<RefreshPolicy>;
    fn button_state(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ButtonState, StateError>;
}

pub struct ButtonState {
    pub label: Option<String>,     // replaces the key's title
    pub detail: Option<String>,    // replaces its secondary line
    pub indicator: Option<Indicator>, // ok | warning | error | running | unknown | loading
    pub progress: Option<u8>,      // 0–100, bar along the bottom edge
    pub toggle: Option<bool>,      // on/off mark along the top edge
    pub accent: Option<Accent>,    // background: blue | green | amber | red | purple | teal | grey
    pub glyph: Option<Glyph>,      // a shape above the title, from a closed set of 16
    pub badge: Option<String>,     // at most 4 characters, top left corner
    pub series: Option<Vec<u8>>,   // 2 to 32 values 0–100, a line behind the texts
}

pub enum RefreshPolicy {
    OnEvents { min_interval: Duration }, // only after something happened
    Every { interval: Duration },        // polled while shown, and on events
}

pub struct StateError { pub code: String, pub message: String }
```

**Who decides what.** The module decides *what* the state is; the core
decides *when* it is asked and *how* it looks. The renderer only knows
`ButtonState` and draws the indicator as a dot in the top-right corner, a
ring while loading, the progress bar and the toggle mark; the accent as the
key's background from a fixed palette, the glyph as a vector shape at the
key's resolution, the badge on a small plate and the series as a line
(`crates/deckpilot-render/src/marks.rs`). A state carries no pixels, fonts
or markup. What the user chose for the key wins: its background over the
accent, its icon over the glyph. The renderer knows nothing about Git,
Docker or any other module.

**When providers run.** The agent asks for a key's state only while the key
is shown on a device, off the device loop, on its own threads:

- at most once per `RefreshPolicy::min_interval()` for the same key
  (action instance and project), measured between starts, whatever happens
  in between; declared intervals are bounded to 1 s – 24 h;
- never while a call for that key is still running, and at most four calls
  at a time;
- `OnEvents`: again only after an event — a key press, a project switch, a
  configuration reload or a finished workflow — once the interval allows;
- `Every`: again every interval while shown, and after events once the
  interval allows.

Nothing polls per frame: the agent sleeps until the next state is due.
States are cached per key; a key shown again shows its last state at once
while a fresh one is fetched, and states of keys not shown for 10 minutes
are dropped. Reloading the configuration, switching profiles, locking and
shutting down cancel running calls (their results are discarded) and clear
the cache.

**What a provider gets.** The same `ExecutionContext` as `execute`,
restricted to the module's capabilities, with the key's project as the
project (the active one), stored credentials when the module declares
`credentials`, no notifications, and a cancellation token that is cancelled
on reload and shutdown and after 20 s. Run programs with
`context.run_process`, which honours the token. `parameters` are the
instance's normalized parameters.

**Rules for providers.**

- Read only. A provider never changes anything, whatever the action does
  when pressed (the Docker *Compose up* key shows whether the project is
  up; it does not start it).
- Be cheap and bounded: one short command or request, its own time limit.
  Declare the longest interval that is still useful; polled providers that
  run programs should use at least 30 s.
- Failures are values. Return `StateError` with a stable code from the
  module's documented list; the key shows an error mark and `Error` as its
  secondary line. A panic is caught and shown the same way. Only the code
  is logged, once when a key turns to an error, never the message — still,
  never put secrets in it.
- Texts are shortened by the core to 32 characters (control characters
  removed), error messages to 120, progress to 100.

**States the agent sets itself.** `Loading` (a grey ring) until the first
answer arrives, `Error` on failure, and `Running` with the run's progress
(finished nodes in percent) on keys bound to a workflow while it runs.

**Rendering.** Faces (`KeyFace`, including the state fields) are compared
per key, and only keys whose face changed are redrawn and sent to the
device. Rendered images are cached by face and size, so a state that flips
back does not draw its image again.

**Built-in providers.**

| Action types | State | Interval |
| ------------ | ----- | -------- |
| `git.status` | branch as detail; green clean, amber uncommitted changes, red conflicts | 30 s |
| `git.branch` | branch as detail (`@<commit>` when detached) | 30 s |
| `docker.status` | green when the engine answers, red (`Unavailable`) otherwise | 30 s |
| `docker.containers` | `N running`; green when any runs, grey otherwise | 30 s |
| `docker.compose-status`, `-up`, `-down`, `-restart` | `running/total up`; green all, amber some, grey none; up and down keys also toggle (on when all run) | 30 s |

Each reads with the module's existing read-only command (`git status`
without optional locks; `docker version`, `docker ps`, `docker compose
ps`).

**Testing a provider.** Call `ActionHandler::button_state` with an
`ExecutionContext`, or `ActionRegistry::button_state` to include the
capability limits; use a fake program for external tools (see the Git and
Docker module tests). The agent's scheduler is tested with explicit times
in `apps/agent/src/service/states.rs`.

**Previewing states in the app.** `KeyFace` carries `indicator`,
`progress`, `toggle`, `accent`, `glyph`, `badge` and `series` as
optional fields (omitted when unset), so stored
data and the IPC protocol are unchanged. The configuration app does not
run providers; a preview would apply a sample `ButtonState` to a face with
`KeyFace::with_state`.

## Security boundary

Modules are compiled-in Rust code, so the boundary protects against mistakes
and keeps responsibilities honest; it is not a sandbox against hostile code
(a module crate could in principle use the standard library directly).
Within that scope:

| Path | Enforcement |
| ---- | ----------- |
| Action execution (keys, workflows, IPC `runAction`/`runWorkflow`) | registry dispatch; disabled modules have no handler; restricted context; effect check |
| Profile checks and key faces | registry; profile/project hidden without `profile-read` |
| Tray and app project switching | agent and app refuse while the projects module is disabled |
| Project management and resource checks in the app | `AppState::require_module` |
| Module enable/disable | the app waits until a running agent has reloaded |

## Enabling and disabling

The agent and the app read module states at start; the app changes them and
asks the agent to reload. When a module is disabled:

- its pages, dashboard cards and action types disappear from the app,
- its action types are not executable; running a stored action reports
  "the Projects module is disabled",
- actions, workflows and layouts that use it are kept unchanged and shown as
  unavailable; they work again once the module is enabled,
- its backend commands are refused (for projects: editing projects and
  switching the active project, including from the tray).

A module cannot be disabled while an enabled module depends on it, and cannot
be enabled without its dependencies or on an unsupported platform. Invalid
stored states fall back to safe values instead of failing the start.

## Remaining dependencies of the core on projects

These are deliberate for now and documented so they can move later:

- `Profile.projects`, `Profile.default_project`, `Project`, `ProjectKey`,
  `ActiveProjectContext`, `ProjectSwitcher` and `CycleDirection` are part of
  the core profile model and runtime session.
- Label templates `{project.key}` / `{project.name}` (`face::render_label`).
- `ResourceRef::ProjectRole` and `MachineConfig::resolve` use the active
  project.
- `StepCondition::ActiveProject` in workflows.
- `ParameterKind::Project` / `ParameterValue::Project`, and
  `ContextEffect::SelectProject` with `Capability::ProjectContext`.
- IPC requests `selectProject`, `cycleProject`, `getActiveProject`, and the
  active project in the agent status and tray.
- Storage tables `projects`, `default_projects`, `active_projects`.

None of these depend on a module crate; `deckpilot-core` has no dependency on
any module.

## Portability contract

Each module declares in its manifest what may leave the computer in an
export (`Portability`):

- `version` — the format version of the module's exported data (`0`: the
  module exports nothing);
- `settings` — the scope of each setting: `portable`, `machineSpecific` or
  `sensitive`. Settings that are not listed are treated as machine-specific
  and never exported.

On import, the core checks the module's data without running module code:
the module must be installed, the exported version must not be newer than
the module's contract (older versions are accepted), settings that are not
portable for the local module are dropped and reported, and the rest must
satisfy the module's configuration schema. Data of modules this build does
not include is reported and not imported. Actions of such modules are
imported and shown as unavailable.

## Module-owned schema

Modules can own database tables through declared, versioned SQL steps
(`ModuleManifest::migrations`). The core applies pending steps for every
registered module — enabled or not, so enabling a module later never meets
an old schema — in one transaction per module, and records the version in
`module_schemas`. A module schema newer than the installed module is
refused. The projects module declares version 1 to take ownership of the
`projects`, `default_projects` and `active_projects` tables created by core
schema 3; later changes to those tables belong in its own steps.

## Still planned

- **Module-owned profile data:** a per-profile, per-module document handed
  to the module, with the profile's lifecycle (duplicate, delete) applied by
  the core through hooks, so that projects can leave the core profile model.
- **Runtime context slots** instead of the built-in active project, with
  effects checked against a capability per slot, and module-provided label
  placeholders.
- **Import migrations in code** for modules whose exported data changes
  shape; until then, a module accepts older export versions only if their
  settings still validate.

## Adding a module

1. Create a crate under `modules/<id>` that depends on `deckpilot-core` and
   implements `FeatureModule` with a manifest and its handlers. Use
   `<id>.<name>` action type ids.
2. Register it in `deckpilot_modules::builtin`.
3. If it has pages, add `apps/desktop/src/modules/<id>/` and list it in
   `FRONTEND_MODULES`; backend commands go to
   `apps/desktop/src-tauri/src/modules/<id>.rs` and check
   `AppState::require_module`.
4. Add tests for the handlers (with an `ExecutionContext`) and for the module
   in `crates/deckpilot-modules/src/tests.rs`.

The Action Library and Workflow editor generate their forms from the
parameter schema, so a new action type needs no UI code.
