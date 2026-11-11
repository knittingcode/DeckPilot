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
| Delay | `out` | Waits up to 10 minutes; cancellable |
| Stop | — | Ends the workflow successfully |

Graphs are acyclic; repetition is a Loop node, and a workflow may not loop
into itself (directly or through others; nesting at most four levels). Only
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
| `changesContext` | Changes the active project; not allowed in parallel branches |

Together with the action type (namespaced id such as `projects.select`,
name, description, category, parameter schema) and the module manifest
(module id, capabilities, platforms), this is the complete node contract.
Outputs are the `data` of a successful `ActionResult`.

Execution uses the existing handler interface:
`ActionHandler::execute(parameters, context) -> ActionResult`. The context
is permission-scoped by the module's declared capabilities, carries the
profile, the active project and this computer's resource bindings.

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
  `unavailable`), a message and whether a retry could help.
- **Retries** (retry-safe actions only): up to five, with exponential backoff.
- **Deadlines.** Handlers are synchronous and cannot be interrupted. A result
  that arrives after the node's timeout counts as a timeout and its effects
  (project changes, outputs) are discarded.
- **Cancellation** is checked before every node and attempt and interrupts
  delays and retry waits; everything not yet run is reported as cancelled.
- **Project context.** The active project is captured when the run starts and
  changes only through actions, in order. Workflows belong to a profile and
  only ever see that profile's actions, projects and workflows.

A run produces a report with, per node, status, attempts, start and duration,
message, outputs and error, and progress events (`nodeStarted`, `retrying`,
`nodeFinished`) for visualization.

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
  do not affect it. A project change the run makes is applied when it ends,
  if the same profile is still active (with several runs, the last to finish
  wins). Workflows without a graph run their steps, reported as nodes
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
  node's action type declares in its node spec, and messages are shortened to
  500 characters. Parameters and other action data are never recorded.
- **Profiles.** Lists, details and cancellation are limited to the active
  profile's runs.

The configuration app reads the agent's list when it opens or the agent
reconnects, so runs started from the deck or before the app restarted appear
with their real progress; running workflows show in the status bar with
progress and a Cancel button.

## Editor

Workflows → a workflow opens in the graph editor (`apps/desktop/src/workflow-builder`):

- **Catalog** (left): the flow nodes (Condition, Parallel, Join, Loop, Delay,
  Stop) and, per enabled module, the profile's actions of the action types
  the module provides, with a button to create a new action of each type.
  The catalog is built from the module registry; the editor has no
  module-specific code. Search filters by name, description and module.
  Entries are dragged onto the canvas, or clicked to add them after the
  selected node (connected through its first free output).
- **Canvas** (centre): zoom, pan, minimap, typed outputs (*Next*, *On error*,
  *Yes*, *No*). Connections the engine would reject — into the start node,
  a second connection from an output (other than a Parallel node's), a
  second incoming connection (other than a Join's), cycles — are refused
  while dragging. Undo and redo (⌘Z, ⇧⌘Z), copy and paste (⌘C, ⌘V, with new
  node ids), delete (⌫), arrange (layered by depth), save (⌘S).
- **Inspector** (right): the selected node — label, id, problems, and its
  settings: the action and, per parameter, the action's value, a value for
  this node, or a value from earlier nodes (one declared output, or text
  with `{{ steps.<node>.output.<key> }}`); what happens on failure, retries
  (only for retry-safe actions), timeout; condition, loop and delay
  settings. Without a selection: the workflow's name and description,
  graph-wide problems, migration notes, duplicate and delete.
- **Validation** is the engine's (`validate_workflow_graph`), run as the
  graph changes. Problems show on the nodes, in the minimap and in the
  status bar. A workflow with errors cannot be saved, so a stored graph
  always runs.
- **Workflows without a graph** keep their step editor. *Open in graph
  editor* converts the steps (`convert_workflow`) and shows the notes where
  the graph behaves differently. Nothing is stored until the graph is saved,
  and the steps are kept.

## Roadmap

Milestones so far: the model, SDK and engine; running graphs in the agent;
the visual editor. Next: execution visualization on the graph and a history
view.
