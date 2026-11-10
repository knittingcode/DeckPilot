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

## Roadmap

This is the first milestone (model, SDK, validation, conversion, engine).
Next: running graphs in the agent with live progress, cancellation and a
bounded execution history over IPC; the visual editor; execution
visualization and status bar integration.
