# Workflow graphs

Workflows can be graphs of connected nodes instead of a list of steps. This
document describes the model, the contract modules implement to provide
nodes, and how the engine runs a graph. The code lives in
`crates/deckpilot-core/src/graph/`.

## Compatibility

A workflow has `steps` and, optionally, a `graph`. Without a graph it runs
with the sequential engine exactly as before; nothing is converted
automatically and no stored workflow changes. When the graph editor opens a
sequential workflow it converts the steps (`graph::from_steps`) and shows
**migration notes** where the graph behaves differently. The graph is stored
only when the user saves it; the step list stays untouched.

The one documented difference: a step with "previous step succeeded/failed"
checks the step directly before it in the graph, while the sequential engine
checks the last step that ran. The conversion notes every case where this
matters (the step before is itself conditional).

## Model

| Node | Ports | Purpose |
|---|---|---|
| Trigger | `out` | Start; exactly one |
| Action | `out`, `error` | Runs an action instance of the profile |
| Condition | `true`, `false` | Output equals a value, output not empty, node succeeded, active project is |
| Parallel | `out` (several) | Starts its branches at the same time |
| Join | `out` | Waits for all incoming branches |
| Loop | `out` | Runs another workflow of the profile 1–100 times |
| Sub-workflow (`call`) | `out`, `error` | Runs another workflow once with inputs; its outputs become the node's |
| Outputs (`output`) | `out` | Sets the workflow's declared outputs for the node that called it |
| Delay | `out` | Waits up to 10 minutes; cancellable |
| Stop | — | Ends the workflow successfully |

Graphs are acyclic; repetition is a Loop node, and a workflow may not run
itself through Loop or Sub-workflow nodes (directly or through others;
nesting at most four levels). Only
Parallel may connect one port to several nodes, and only Join may have
several incoming connections. At most 200 nodes.

Node ids are readable slugs (`git_status`) because data mappings refer to
them. Editor positions are stored with the nodes and ignored by the engine.
The stored format has a `version` (currently 1) for future migrations.

### Data mapping

An action node's parameter can take its value from an earlier node:

- **Output**: the whole value of `steps.<node>.output.<key>`, with an
  optional default when the output is missing;
- **Template**: text with `{{ steps.<node>.output.<key> }}` placeholders.

Substitution is plain text. There are no expressions, filters or code; values
are never evaluated or passed to a shell, and placeholders inside values are
not expanded again. Validation checks that the referenced node runs before
(is an ancestor) and declares that output. At run time values are converted
to the parameter's kind (whole numbers, true/false, project keys of this
profile); resource parameters cannot be mapped. Mappings refer only to node
ids and output keys, so they are portable between computers.

### Sub-workflows

A workflow can declare a typed interface (stored as optional `inputs` and
`outputs` of the workflow; workflows without them are unchanged):

- **Inputs**: `key`, `label`, `kind` (a `ValueKind`: text, whole number,
  yes/no, path, URL, project key), `required`, `default`, `sensitive`. The
  workflow's nodes read an input as an output of its start node
  (`steps.start.output.<key>`), with the same mappings as everywhere else.
- **Outputs**: `key`, `label`, `kind`, `sensitive`. An **Outputs** node
  (`{"type": "output", "values": {"<key>": <mapping>}}`) sets them from
  earlier nodes. Several may run (for example one per branch); a later one
  overrides the keys it sets. Outputs no Output node sets are absent.

A **Sub-workflow** node (`{"type": "call", "workflow": "<id>", "inputs":
{"<key>": <mapping>}, "policy": {...}}`) runs another workflow of the same
profile once:

- **Inputs** are mappings, exactly like action parameters: one earlier
  output or text with outputs; nothing is evaluated. An input without a
  mapping gets its default; a required input must be mapped. Values are
  converted to the input's kind when the node runs (whole numbers and
  yes/no must parse, project keys must belong to the profile); a value that
  does not convert fails the node with `mapping` before the callee starts.
- **Outputs**: the callee's declared outputs become the node's outputs, so
  later nodes map them as `steps.<node>.output.<key>`. Output values are
  converted to their kind by the Outputs node that sets them.
- **Execution.** The callee runs with the node's project context, under a
  token that is cancelled when the run is cancelled or at the node's
  timeout (default 10 minutes, at most 10). Its project changes continue
  after the node, as with a Loop. Its own nodes are not reported; the node
  reports its status, outputs and error. A failure of the callee fails the
  node with `subworkflow.failed` ("Greet failed: hello: …"), a timeout
  with `timeout`, a cancellation is `cancelled`. Failure handling works as
  for actions (`error` port, *continue*); sub-workflows are never retried.
- **Running on its own.** A workflow with required inputs cannot be run
  directly (from the app or a deck); it is meant to be called. Optional
  inputs take their defaults when it runs on its own.
- **Validation**: the callee exists in the profile; no recursion, directly
  or indirectly, through Sub-workflow and Loop nodes alike; at most four
  levels of nesting; every required input is mapped and only declared
  inputs are; references to a Sub-workflow node use outputs the callee
  declares; Outputs nodes set only declared outputs; declared keys are
  unique and valid and defaults fit their kind. A Loop cannot run a
  workflow with required inputs.
- **Sensitive values.** A value that comes from a sensitive output (an
  action's, a sensitive input, a sensitive sub-workflow output) may only be
  passed to a sensitive input or set as a sensitive output; validation
  rejects anything else. History and live events keep only a Sub-workflow
  or Outputs node's non-sensitive declared outputs, and never the inputs a
  start node received.

## Module node SDK

Modules provide nodes through their action types. `ActionType::node`
(`NodeSpec`) declares how an action behaves as a node; the editor reads it
from the module list and never imports module code.

| Field | Meaning |
|---|---|
| `version` | Version of the node's parameter and output schema |
| `icon` | Icon name |
| `outputs` | Structured outputs (`key`, `label`, `kind`, `always`) |
| `timeoutMs` | Default deadline (default 30 s) |
| `retrySafe` | Retrying after a failure is safe; retries are refused otherwise |
| `idempotent` | Running twice equals running once |
| `changesContext` | Selects a project: changes the run's project context for the nodes after it |
| `cancellable` | Stops promptly when cancelled or at its timeout (default `false`) |

Together with the action type (namespaced id such as `projects.select`,
name, description, category, parameter schema) and the module manifest
(module id, capabilities, platforms), this is the complete node contract.
Outputs are the `data` of a successful `ActionResult`.

Execution uses the handler interface
`ActionHandler::execute(parameters, context) -> ActionResult`. The complete
contract — schemas, execution context, capabilities, cancellation, timeouts,
retries, errors, progress and compatibility — is in
[Feature modules](modules.md#workflow-node-contract).

The Projects module declares its nodes: `select`, `next`/`previous` change
the project (next/previous are not retry-safe), `active` and the resource
actions are idempotent; all output the project's id, key and name, the
resource actions also the resource, its key, kind and value.

## Execution semantics

- **Validation first.** A graph runs only if validation reports no errors:
  node types exist and their module is enabled, the action exists in the
  profile, parameters are valid, references point to earlier nodes and
  declared outputs, projects belong to the profile, loops are bounded, and
  no context-changing action runs in a parallel branch. Errors are attached
  to nodes so the editor can show them in place.
- **Deterministic scheduling.** Nodes run in waves in topological order (ties
  by id). Only after a Parallel node does a wave hold several nodes; they run
  concurrently (at most four at a time) and their results are applied in the
  same fixed order.
- **Branches.** A node runs if its incoming connection was taken; otherwise it
  is skipped along with everything only reachable through it. A Join runs if
  at least one incoming branch was taken, after all of them are resolved.
- **Failures.** A failed action continues through `error` if connected
  (handled), through `out` with *continue*, and otherwise fails the workflow:
  nodes already running finish, nothing new starts. Errors have a stable code
  (`action.failed`, `timeout`, `mapping`, `parallel-context`, `loop.failed`,
  `subworkflow.failed`, `unavailable`), a message and whether a retry could
  help.
- **Retries** (retry-safe actions only): up to five, with exponential backoff.
- **Deadlines.** Each attempt's token is cancelled at its timeout, so
  cooperative actions stop there. A result that arrives after the timeout
  counts as a timeout and its effects (project changes, outputs) are
  discarded.
- **Cancellation** reaches running actions through their token (cooperative
  actions stop at once), and interrupts delays and retry waits; everything
  not yet run is reported as cancelled. A node stopped by a cancellation is
  *cancelled*, never *failed* or *timed out*; an action that finished
  successfully despite a cancellation keeps its result.
- **Project context.** Each run has its own project context; see
  [Project context and the active project](#project-context-and-the-active-project).
  Workflows belong to a profile and only ever see that profile's actions,
  projects and workflows.

A run produces a report with, per node, status, attempts, start and duration,
message, outputs and error, and progress events (`nodeStarted`, `retrying`,
`nodeFinished`) for visualization.

## Project context and the active project

Three things are kept apart:

- **The active project** of DeckPilot: what the deck and the app show, chosen
  by the user (project switcher, Stream Deck controls, the app) — explicit
  operations that change it immediately.
- **A run's project context**: the project a workflow run works with. It
  starts as the active project at the moment the run starts (recorded as the
  run's `initialProject`) and belongs to that run alone.
- **An explicit change of the active project by a workflow**: allowed only
  for workflows with the permission described below.

Inside a run:

- A project-selecting action (its node spec declares `changesContext`)
  changes the context for the nodes after it, along the connections. Nothing
  else — not the active project, not other runs — sees the change.
- Parallel branches each carry their own copy, so branches cannot overwrite
  one another. Project changes in parallel branches are allowed.
- A **Join** merges the contexts of its taken branches deterministically:
  every context carries the nodes that changed it; changes all branches share
  happened before they split. Branches without changes of their own agree
  with anything. Branches with changes of their own must end on the same
  project; otherwise the Join fails with `context-conflict` — no branch
  silently wins.
- The run's **final context** is that of the nodes it ended with, by the same
  rule. When they disagree the final context is the initial one and the
  report says so (`contextConflict`).

The **project scope** of a workflow (`projectScope`) decides whether its runs
may change the active project:

- `execution` — project changes stay in the run. New workflows use this.
- `application` — a permission the user grants per workflow ("May change the
  active project" in the editor). When a run ends on another project than it
  started with, that project becomes the active project, by
  **compare-and-set**: only if the active profile and project are still the
  ones the run started with, and the Projects module is enabled. If the user,
  a device or another run changed the active project meanwhile, their change
  stands; the run records that its change was not made and why. With several
  permitted runs, the first to finish switches and the others are refused.

**Compatibility.** Workflows saved before the setting existed have no
`projectScope`. They behave as before: a workflow whose actions select
projects changes the active project (`application`), any other keeps
changes in its runs (`execution`). The editor shows this derived value and
stores it explicitly when the workflow is saved again. No stored data is
rewritten.

Each finished run records what happened (`project`): `unchanged`,
`keptInRun`, `applied`, or `notApplied` with a reason.

## Concurrency and resource safety

- **Bounded.** At most four workflows run at a time, a workflow runs once at
  a time, and a parallel wave runs at most four nodes at a time; nested Loop
  runs execute inside their node. A run never holds more than four threads.
- **Deterministic.** Waves follow topological order with ties broken by node
  id; parallel results are applied in that order.
- **No shared mutable state.** A run works on its own copies of the profile,
  machine configuration and project context. Action handlers receive
  read-only views; resources are resolved against the node's own context, so
  two runs on different projects resolve different resources.
- **Retries and timeouts** each attempt on a fresh copy of the node's input
  context; a failed or late attempt's changes are discarded.
- **Reentrancy.** A workflow cannot start while it runs; a Loop or
  Sub-workflow cannot run its own workflow (validation rejects recursion
  and limits nesting). Nested runs execute inside their node's thread.
- **One writer.** Only the agent's single task changes runtime state and
  writes execution history; run threads report through a channel.
- **Shutdown.** Running workflows are recorded as interrupted when the agent
  stops; a record still marked running when the database opens (a crash) is
  marked interrupted then. The agent waits for its IPC connections to close
  before it reports that it stopped.

## Triggers

A workflow can start by itself: its optional `triggers` (stored with the
workflow; workflows without them are unchanged) are schedules and lifecycle
events, and `triggersPaused` pauses all of them.

```json
"triggers": [
  { "type": "schedule", "schedule": { "every": "interval", "minutes": 15 } },
  { "type": "schedule", "schedule": { "every": "weekly", "days": ["mon", "fri"], "at": "09:00" } },
  { "type": "lifecycle", "event": "deviceConnected",
    "inputs": { "serial": { "type": "event", "field": "deviceId" } } }
]
```

- **Schedules**: every *n* minutes (1 minute to 7 days), every day at a
  local time, or on chosen weekdays at a local time (`HH:MM`).
- **Lifecycle events**: `agentStarted` (once per agent start, when the
  configuration is first unlocked), `profileActivated`, `projectChanged`
  (within the active profile), `deviceConnected`, `deviceDisconnected`.
- **Inputs.** A trigger gives the workflow's inputs a fixed value or a
  field of the event: `firedAt` (local `YYYY-MM-DD HH:MM`), `profileName`,
  `projectKey`, `projectName`, `previousProjectKey` (project changed),
  `deviceId` (the serial) and `deviceModel` (device events). Values are
  converted to the input's kind as for sub-workflows; a field fits a text
  input or one of its own kind (project keys). Inputs without a value get
  their default.
- **Validation**: intervals of at least one minute, valid times and
  weekdays, only declared inputs, fields the event has, fixed values of the
  input's kind, a value for every required input, and no self-triggering:
  a workflow started by project changes may not change the active project
  (its project scope must be `execution`), or it would start itself again.

**When triggers fire.** The agent runs one scheduler and watches its own
state after every event it handles (active profile, active project,
connected devices):

- only for the **active profile**, and only while the configuration is
  **unlocked** — nothing that happened while locked is replayed; unlocking
  takes the current state as the starting point, and devices connected
  while locked do not count as connected;
- a firing is **skipped** while the previous run of the workflow still
  runs, while its triggers are paused, and when four workflows already run;
- runs are recorded in history with the trigger kind (`schedule`,
  `lifecycle`) and which one (`every 15 min`, `deviceConnected`).

**Time.** The scheduler is checked every few seconds:

- An interval counts real time from when the trigger was first seen (agent
  started and unlocked, profile activated, workflow saved or resumed).
- Daily and weekly times are local wall-clock times in the computer's time
  zone and fire at most once per local date.
- **No catch-up.** A firing may be up to two minutes late; one that is
  later — the computer slept, the agent did not run, the configuration was
  locked — is skipped and never made up. An interval then continues with
  its next due time; a time first seen within its minute waits for the
  next day.
- **Daylight saving time.** A local time that does not exist on a day
  (clocks go forward) does not fire that day; a time that occurs twice
  (clocks go back) fires once. Intervals are not affected.

In the editor, triggers are edited in the workflow panel (nothing
selected): add a schedule or an event, choose values for the workflow's
inputs, pause them. The start node shows them.

## Running in the agent

Workflows run in the background agent, whether they are started from the
app, a Stream Deck control or another client; no window has to be open.

- **Executions.** Every run gets an execution id. `startWorkflow` answers
  immediately; `runWorkflow` (older clients) answers when the run ends. A
  workflow runs at most once at a time, and at most four workflows run
  together.
- **Snapshot.** A run executes on its own thread with a copy of the active
  profile, the machine configuration and the project selection, so the agent
  keeps serving devices and requests, and configuration changes during a run
  do not affect it. What a run's project changes mean for the active project
  is described below. Workflows without a graph run their steps, reported as
  nodes
  `step_1`, `step_2`, … — the ids the graph conversion gives them.
- **Progress.** Subscribers receive `executionStarted`, `executionProgress`
  (a node started, finished or is retried, with resolved and total nodes) and
  `executionFinished`. Progress is the share of resolved nodes, never an
  estimate of time.
- **Cancellation** (`cancelExecution`) stops a run before its next node or
  attempt and interrupts delays and retry waits; an action already running
  finishes. Locking the configuration cancels every run.
- **History.** Runs are recorded in the encrypted database when they start
  and when they end: state, trigger, times, progress and the report of every
  node. The newest 50 runs per workflow and 500 per profile are kept;
  `clearExecutionHistory` deletes them, and deleting a profile deletes its
  history. Records are runtime state and are not exported.
- **Interruption.** When the agent stops, running workflows are recorded as
  interrupted. A run still marked running when the database is opened (the
  agent crashed or the computer lost power) is marked interrupted then.
- **Safe outputs.** Reports, events and records only contain the outputs the
  node's action type declares in its node spec (for Sub-workflow and
  Outputs nodes: the workflow's declared outputs), never sensitive ones,
  and messages are shortened to 500 characters. Parameters, workflow inputs
  and other action data are never recorded.
- **Profiles.** Lists, details and cancellation are limited to the active
  profile's runs.

The configuration app reads the agent's list when it opens or the agent
reconnects, so runs started from the deck or before the app restarted appear
with their real progress; running workflows show in the status bar with
progress and a Cancel button.

## Editor

Workflows → a workflow opens in the graph editor (`apps/desktop/src/workflow-builder`):

- **Catalog** (left): the flow nodes (Condition, Parallel, Join, Loop,
  Sub-workflow, Outputs, Delay, Stop) and, per enabled module, the profile's actions of the action types
  the module provides, with a button to create a new action of each type.
  The catalog is built from the module registry; the editor has no
  module-specific code. Search filters by name, description and module.
  Entries are dragged onto the canvas (placed where dropped), or added with
  a click or the keyboard (Enter/Space) after the selected node, connected
  through its first free output and placed where they cover no other node.
- **Canvas** (centre): zoom, pan, minimap, typed outputs (*Next*, *On error*,
  *Yes*, *No*). Connections the engine would reject — into the start node,
  a second connection from an output (other than a Parallel node's), a
  second incoming connection (other than a Join's), cycles — are refused
  while dragging, and dropping one says why. Existing connections can be
  dragged to another node (reconnect) under the same rules. Nodes move by
  dragging or with the arrow keys (selected nodes); each move is one undo
  step. Undo and redo (⌘Z, ⇧⌘Z), copy and paste (⌘C, ⌘V, with new node
  ids), delete (⌫), save (⌘S).
- **Arrange** lays the graph out in columns by distance from the start and
  orders each column to keep branches together and connections from
  crossing (Yes above No). It only runs when asked for and is undoable;
  otherwise node positions are always the user's.
- **View.** A workflow opens where it was left (viewport remembered per
  workflow in this window); a workflow opened for the first time fits small
  graphs and starts larger ones at a readable zoom at the start node.
  Saving keeps the view, selection and panels. Which panels are open is
  remembered; narrow windows start without the catalog.
- **Inspector** (right): the selected node — label, id, problems, its run
  result, and its settings: the action and, per parameter, the action's
  value, a value for this node, or a value **from earlier nodes**. Mappings
  are edited as parts: one output (chosen from the declared outputs of
  nodes that run before, with their types), or text composed of text parts
  and outputs. The editor shows a preview (with the shown run's values where
  known), references that are missing or do not run before the node, and
  whether the value fits the parameter (numbers, yes/no, choices and
  projects must convert; resources cannot be mapped). Nothing is evaluated
  in the editor. Then failure handling, retries (only for retry-safe
  actions), timeout (saying whether the action can be interrupted), and
  **about this node**: its type id and version, module, the module's
  permissions, behaviour (retry-safe, idempotent, interruptible, selects a
  project) and its inputs and outputs with their types. A **Sub-workflow**
  node chooses the workflow, maps each of its inputs (required ones say so;
  optional ones show their default), sets failure handling and timeout,
  and lists the outputs it provides. An **Outputs** node maps each of the
  workflow's declared outputs.
- **Inputs and outputs** of the open workflow are edited in the workflow
  panel (nothing selected): key, type, label, required, default and
  sensitive. The start node then offers the inputs to mappings.
- **Validation** is the engine's (`validate_workflow_graph`), run as the
  graph changes. Problems show on the nodes, in the minimap and in the
  status bar. A workflow with errors cannot be saved, so a stored graph
  always runs.
- **Workflows without a graph** keep their step editor. *Open in graph
  editor* converts the steps (`convert_workflow`) and shows the notes where
  the graph behaves differently. Nothing is stored until the graph is saved,
  and the steps are kept.

## Watching runs

The editor shows runs of the open workflow on the canvas, whether they were
started in the editor, from a Stream Deck or while the app was closed:

- **Live.** The newest run is followed automatically: running nodes are
  highlighted (with the attempt when an action is retried), finished nodes
  show succeeded, failed, skipped or cancelled with their duration, and the
  connections a run took are emphasized. The toolbar shows the run's state
  and real progress (resolved nodes of all nodes); the titlebar's Run
  button becomes *Cancel run*.
- **Node details.** Selecting a node shows what it did in the shown run:
  result, attempts, start and duration, its message, a structured error
  (code, message, whether a retry may help) and its outputs — only the
  outputs its action declares.
- **Run details.** Below the history, the shown run is broken down: total
  and summed node time with the slowest nodes, failures with error code,
  action code and whether a retry may help, and a timeline of every node's
  start and duration relative to the run, with parallel branches marked.
  Choosing a node shows its retries, failure and declared outputs (shortened
  previews; sensitive outputs are shown as *not kept*). Retry waits are known
  only for runs watched live; the history keeps only the final attempt.
- **History.** Without a selection, the inspector lists the workflow's last
  20 runs with state, trigger (app or Stream Deck), start time and duration;
  a run is shown on the canvas by selecting it. Running entries have
  progress and Cancel. The history can be cleared. Runs started by a
  schedule or an event show which one.
- **Reconnection.** The list is read again when the window opens or the
  agent reconnects, and node reports of finished runs are loaded on demand,
  so closing and reopening the app loses nothing.
- **Status bar.** Running workflows of the active profile appear as
  background activity with progress and Cancel; failures and interruptions
  raise a notification.

## Roadmap

Milestones: the model, SDK and engine; running graphs in the agent; the
visual editor; watching runs on the graph with their history; sub-workflows
with typed inputs and outputs.
