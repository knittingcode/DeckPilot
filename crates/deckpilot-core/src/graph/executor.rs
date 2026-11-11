//! Executes a validated workflow graph.
//!
//! # Semantics
//!
//! - **Scheduling.** Nodes run in waves. A wave holds every node whose
//!   incoming connections are all resolved, in topological order (ties by
//!   node id), so a run is deterministic. A wave has more than one node only
//!   after a Parallel node; its nodes run concurrently (at most
//!   [`ExecutionLimits::max_parallel`] at a time) and their results are
//!   applied in that same order.
//! - **Activation.** A node runs if its incoming connection was taken: an
//!   action took `out` or `error`, a condition `true` or `false`. Otherwise
//!   it is skipped, and so is everything only reachable through it. A Join
//!   waits until all incoming branches are resolved and runs if at least one
//!   of them was taken.
//! - **Failures.** A failed action continues through `error` when that is
//!   connected (the failure is handled), through `out` with
//!   `OnError::Continue`, and otherwise fails the workflow: nodes already
//!   running in the same wave finish, nothing new starts.
//! - **Project context.** A run has its own project context, captured from
//!   the active project when it starts. It flows along the connections: a
//!   node sees the context of the node it follows, and an action that
//!   selects a project changes it for the nodes after it only. Parallel
//!   branches each carry their own copy. A Join continues with the context
//!   its branches agree on; branches that end on different projects fail
//!   the Join (`context-conflict`) instead of one silently winning. The run's
//!   final context is that of the nodes it ended with, under the same rule;
//!   when they disagree the run keeps its initial project and reports the
//!   conflict. Whether the final context becomes the application's active
//!   project is decided by the caller (see `Workflow::project_scope`).
//! - **Deadlines.** Actions are synchronous and cannot be interrupted. An
//!   action that exceeds its timeout is reported as failed and its effects
//!   are discarded.
//! - **Cancellation** is checked before every node and attempt and
//!   interrupts delays and retry waits. Running actions finish.
//! - **Sub-workflows.** A Call node runs another workflow with its inputs
//!   (converted to their declared kinds) and the node's project context,
//!   under a token that is cancelled with the run or at the node's timeout.
//!   The callee's declared outputs, set by its Output nodes, become the
//!   node's outputs; its project context continues after the node, like a
//!   Loop's. Its own nodes are not reported, only the Call node.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::mapping::{Outputs, convert, lookup, resolve};
use super::{Mapping, NodeId, NodeKind, OnError, Port, Predicate, WorkflowGraph};
use crate::{
    ActionInstance, ActionParameters, ActionRegistry, ActionResult, ActiveProjectContext,
    EngineError, ParameterKind, ParameterValue, Profile, WorkflowId,
};

pub use crate::cancel::{CancelReason, CancellationToken};

#[derive(Debug, Clone, Copy)]
pub struct ExecutionLimits {
    /// Nodes of a parallel wave that run at the same time.
    pub max_parallel: usize,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self { max_parallel: 4 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeStatus {
    Succeeded,
    /// Failed; see the report's error. Handled if the error port was taken.
    Failed,
    /// Not run: its branch was not taken, or the workflow ended before it.
    Skipped,
    Cancelled,
}

/// A failure, safe to show and to store: no secrets, no raw output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepError {
    /// Stable: `action.failed`, `timeout`, `cancelled`, `mapping`,
    /// `context-conflict`, `unavailable`, `loop.failed`,
    /// `subworkflow.failed`.
    pub code: String,
    pub message: String,
    pub retryable: bool,
    /// For `action.failed`: the action's own error code, e.g.
    /// `exit-nonzero` (see [`ActionResult::code`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_code: Option<String>,
}

impl StepError {
    fn new(code: &str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            retryable,
            action_code: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeReport {
    pub node: NodeId,
    pub status: NodeStatus,
    /// Attempts made (actions; 0 for nodes that did not run).
    pub attempts: u32,
    /// Milliseconds after the start of the run.
    pub started_ms: u64,
    pub duration_ms: u64,
    /// The action's message, or what the node did.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// Structured outputs, available to later nodes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<StepError>,
    /// For actions: the instance that ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<crate::ActionInstanceId>,
    /// For Call nodes the workflow that ran, for Output nodes the workflow
    /// they belong to: the workflow whose declared outputs `output` holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<WorkflowId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphReport {
    pub workflow: WorkflowId,
    pub status: RunStatus,
    pub duration_ms: u64,
    /// Every node, in the order it was resolved.
    pub nodes: Vec<NodeReport>,
    /// Set when the branches the run ended with selected different
    /// projects; the run's context then stays at its initial project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_conflict: Option<String>,
}

impl GraphReport {
    pub fn node(&self, id: &NodeId) -> Option<&NodeReport> {
        self.nodes.iter().find(|n| &n.node == id)
    }
}

/// Progress of a run, for visualization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExecutionEvent {
    NodeStarted {
        node: NodeId,
    },
    Retrying {
        node: NodeId,
        attempt: u32,
        wait_ms: u64,
    },
    NodeFinished {
        report: NodeReport,
    },
    /// A running action reported progress.
    NodeProgress {
        node: NodeId,
        progress: crate::Progress,
    },
}

/// Runs one action instance with the given parameter overrides against a
/// context. Effects are applied to that context.
pub(crate) type RunAction<'a> = dyn Fn(
        &ActionInstance,
        &ActionParameters,
        &mut ActiveProjectContext,
        &crate::ExecutionControl<'_>,
    ) -> Result<ActionResult, EngineError>
    + Sync
    + 'a;

/// How a nested workflow run (Loop, Call) ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NestedRun {
    pub status: RunStatus,
    /// The callee's declared outputs, when it succeeded.
    pub outputs: BTreeMap<String, String>,
    /// The first failure, `node: message`, when it failed.
    pub failure: Option<String>,
}

/// Runs a nested workflow with inputs, a context it changes and the token
/// that cancels it.
pub(crate) type RunNested<'a> = dyn Fn(
        WorkflowId,
        &BTreeMap<String, String>,
        &mut ActiveProjectContext,
        &CancellationToken,
    ) -> Result<NestedRun, EngineError>
    + Sync
    + 'a;

pub(crate) struct Executor<'a> {
    pub workflow: WorkflowId,
    pub graph: &'a WorkflowGraph,
    pub profile: &'a Profile,
    pub registry: &'a ActionRegistry,
    pub run_action: &'a RunAction<'a>,
    pub run_nested: &'a RunNested<'a>,
    /// The run's inputs: the outputs of its start node.
    pub inputs: &'a BTreeMap<String, String>,
    pub on_event: &'a (dyn Fn(ExecutionEvent) + Sync),
    pub cancel: &'a CancellationToken,
    pub limits: ExecutionLimits,
    /// How the run is going, for actions that report on it.
    pub summary: std::sync::Mutex<crate::RunSummary>,
    pub started: Instant,
}

/// What a run has resolved so far.
struct RunState {
    start: Instant,
    initial: ActiveProjectContext,
    /// Resolved nodes and the output ports they took.
    taken: BTreeMap<NodeId, BTreeSet<Port>>,
    /// The context each node that ran passes on.
    contexts: BTreeMap<NodeId, Carried>,
    outputs: Outputs,
    reports: Vec<NodeReport>,
    failed: bool,
    stopped: bool,
}

/// A context passed along a connection, with the nodes that changed it
/// (`None`: the run's initial context).
type Carried = (Option<ActiveProjectContext>, Vec<NodeId>);

/// What a node did: its report and the output ports it took.
struct Outcome {
    report: NodeReport,
    ports: Vec<Port>,
    /// The node failed and nothing handles it.
    fails_workflow: bool,
    stops_workflow: bool,
    /// Context after the node, when the node can change it.
    context: Option<ActiveProjectContext>,
}

impl Executor<'_> {
    pub fn run(&self, context: &mut ActiveProjectContext) -> Result<GraphReport, EngineError> {
        let start = Instant::now();
        let order: Vec<NodeId> = self
            .graph
            .topological_order()
            .ok_or_else(|| EngineError::InvalidWorkflow("the workflow contains a cycle".into()))?
            .into_iter()
            .cloned()
            .collect();
        let mut state = RunState {
            start,
            initial: context.clone(),
            taken: BTreeMap::new(),
            contexts: BTreeMap::new(),
            outputs: Outputs::new(),
            reports: Vec::new(),
            failed: false,
            stopped: false,
        };
        loop {
            let wave: Vec<NodeId> = order
                .iter()
                .filter(|id| !state.taken.contains_key(*id))
                .filter(|id| {
                    self.graph
                        .incoming(id)
                        .all(|e| state.taken.contains_key(&e.from))
                })
                .cloned()
                .collect();
            if wave.is_empty() {
                break;
            }
            self.run_wave(&wave, &mut state);
        }

        let status = if self.cancel.is_cancelled()
            && state
                .reports
                .iter()
                .any(|r| r.status == NodeStatus::Cancelled)
        {
            RunStatus::Cancelled
        } else if state.failed {
            RunStatus::Failed
        } else {
            RunStatus::Succeeded
        };
        let context_conflict = match self.final_context(&state.taken, &state.contexts) {
            Ok((Some(last), _)) => {
                *context = last;
                None
            }
            Ok((None, _)) => None,
            Err(message) => Some(message),
        };
        Ok(GraphReport {
            workflow: self.workflow,
            status,
            duration_ms: ms(start),
            nodes: state.reports,
            context_conflict,
        })
    }

    /// Resolves one wave: skips what is not activated, runs the rest (in
    /// parallel when there are several) and records their outcomes.
    fn run_wave(&self, wave: &[NodeId], state: &mut RunState) {
        let mut active: Vec<(&NodeId, Carried)> = Vec::new();
        let mut outcomes = Vec::new();
        for id in wave {
            let activated = self.activated(id, &state.taken);
            if !activated || state.failed || state.stopped || self.cancel.is_cancelled() {
                // After a cancellation nothing else runs: everything left is
                // reported as cancelled, not as a branch not taken.
                let status = if self.cancel.is_cancelled() {
                    NodeStatus::Cancelled
                } else {
                    NodeStatus::Skipped
                };
                let report = idle(id, status, ms(state.start));
                (self.on_event)(ExecutionEvent::NodeFinished {
                    report: report.clone(),
                });
                state.reports.push(report);
                state.taken.insert(id.clone(), BTreeSet::new());
                continue;
            }
            match self.input_context(id, &state.taken, &state.contexts) {
                Ok(input) => active.push((id, input)),
                Err(message) => {
                    (self.on_event)(ExecutionEvent::NodeStarted { node: id.clone() });
                    let mut report = idle(id, NodeStatus::Failed, ms(state.start));
                    report.error = Some(StepError::new("context-conflict", message, false));
                    let mut outcome = finished(report, vec![]);
                    outcome.fails_workflow = true;
                    outcomes.push(outcome);
                }
            }
        }
        let runnable: Vec<(&NodeId, ActiveProjectContext)> = active
            .iter()
            .map(|(id, (input, _))| (*id, input.clone().unwrap_or_else(|| state.initial.clone())))
            .collect();
        if runnable.len() > 1 {
            outcomes.extend(self.run_concurrently(&runnable, &state.outputs, state.start));
        } else {
            outcomes.extend(
                runnable
                    .iter()
                    .map(|(id, input)| self.node(id, input, &state.outputs, state.start)),
            );
        }
        let carried: BTreeMap<&NodeId, Carried> = active.into_iter().collect();
        for outcome in outcomes {
            let received = carried.get(&outcome.report.node).cloned();
            self.record(outcome, received, state);
        }
    }

    /// Records what a node did: the context it passes on (what it received
    /// plus its own change), its outputs, the ports it took and its report.
    fn record(&self, mut outcome: Outcome, received: Option<Carried>, state: &mut RunState) {
        state.failed |= outcome.fails_workflow;
        state.stopped |= outcome.stops_workflow;
        let id = outcome.report.node.clone();
        if matches!(
            outcome.report.status,
            NodeStatus::Succeeded | NodeStatus::Failed
        ) {
            let (input, mut changes) = received.unwrap_or((None, Vec::new()));
            let input = input.unwrap_or_else(|| state.initial.clone());
            let passed = match outcome.context.take() {
                Some(after) if after != input => {
                    changes.push(id.clone());
                    after
                }
                _ => input,
            };
            state.contexts.insert(id.clone(), (Some(passed), changes));
        }
        if outcome.report.status == NodeStatus::Succeeded {
            state
                .outputs
                .insert(id.clone(), outcome.report.output.clone());
        }
        // The summary counts actions and sub-workflows: what a user thinks
        // of as steps.
        let is_action = self
            .graph
            .node(&id)
            .is_some_and(|n| matches!(n.kind, NodeKind::Action { .. } | NodeKind::Call { .. }));
        if is_action {
            let mut summary = self
                .summary
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match outcome.report.status {
                NodeStatus::Succeeded => summary.succeeded += 1,
                NodeStatus::Failed => {
                    summary.failed += 1;
                    summary.failed_nodes.push(id.as_str().to_owned());
                }
                NodeStatus::Skipped => summary.skipped += 1,
                NodeStatus::Cancelled => summary.cancelled += 1,
            }
        }
        (self.on_event)(ExecutionEvent::NodeFinished {
            report: outcome.report.clone(),
        });
        state.taken.insert(id, outcome.ports.into_iter().collect());
        state.reports.push(outcome.report);
    }

    /// The context a node starts with: that of the node it follows, or for a
    /// Join, the merge of its taken branches (see [`Executor::merge`]).
    fn input_context(
        &self,
        id: &NodeId,
        taken: &BTreeMap<NodeId, BTreeSet<Port>>,
        contexts: &BTreeMap<NodeId, Carried>,
    ) -> Result<Carried, String> {
        let incoming: Vec<&Carried> = self
            .graph
            .incoming(id)
            .filter(|edge| {
                taken
                    .get(&edge.from)
                    .is_some_and(|ports| ports.contains(&edge.port))
            })
            .filter_map(|edge| contexts.get(&edge.from))
            .collect();
        self.merge(&incoming)
    }

    /// The context of the nodes the run ended with: nodes that ran and whose
    /// taken connections lead to no node that ran.
    fn final_context(
        &self,
        taken: &BTreeMap<NodeId, BTreeSet<Port>>,
        contexts: &BTreeMap<NodeId, Carried>,
    ) -> Result<Carried, String> {
        let ends: Vec<&Carried> = contexts
            .iter()
            .filter(|(id, _)| {
                !self.graph.outgoing(id).any(|edge| {
                    taken
                        .get(*id)
                        .is_some_and(|ports| ports.contains(&edge.port))
                        && contexts.contains_key(&edge.to)
                })
            })
            .map(|(_, carried)| carried)
            .collect();
        self.merge(&ends)
    }

    /// Merges the contexts of branches. Each context carries the nodes that
    /// changed it; the changes all branches share happened before they
    /// split. A branch without changes of its own carries that shared
    /// context and agrees with any other branch. Branches with changes of
    /// their own must end on the same project; otherwise the merge fails.
    fn merge(&self, branches: &[&Carried]) -> Result<Carried, String> {
        let Some(first) = branches.first() else {
            return Ok((None, Vec::new()));
        };
        let shared = branches
            .iter()
            .skip(1)
            .fold(first.1.len(), |len, (_, changes)| {
                first.1[..len]
                    .iter()
                    .zip(changes)
                    .take_while(|(a, b)| a == b)
                    .count()
            });
        let diverged: Vec<&&Carried> = branches
            .iter()
            .filter(|(_, changes)| changes.len() > shared)
            .collect();
        let Some(winner) = diverged.first() else {
            return Ok((*first).clone());
        };
        if diverged.iter().all(|(context, _)| context == &winner.0) {
            // Keep every change, in a fixed order, so later merges see them.
            let mut merged = winner.1.clone();
            for (_, other) in diverged.iter().skip(1) {
                for id in &other[shared..] {
                    if !merged.contains(id) {
                        merged.push(id.clone());
                    }
                }
            }
            return Ok((winner.0.clone(), merged));
        }
        let mut keys: Vec<String> = diverged
            .iter()
            .map(|(context, _)| {
                context
                    .as_ref()
                    .and_then(|c| c.project(self.profile).ok().flatten())
                    .map_or_else(|| "no project".to_owned(), |p| p.key.to_string())
            })
            .collect();
        keys.sort();
        keys.dedup();
        Err(format!(
            "branches selected different projects ({}); a Join continues only when they agree",
            keys.join(", ")
        ))
    }

    /// Runs the nodes of a parallel wave, at most `max_parallel` at a time,
    /// each with its own context. Outcomes keep the order of `active`.
    fn run_concurrently(
        &self,
        active: &[(&NodeId, ActiveProjectContext)],
        outputs: &Outputs,
        start: Instant,
    ) -> Vec<Outcome> {
        let mut outcomes = Vec::with_capacity(active.len());
        for chunk in active.chunks(self.limits.max_parallel.max(1)) {
            let results: Vec<Outcome> = std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|(id, input)| scope.spawn(move || self.node(id, input, outputs, start)))
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().expect("a workflow node panicked"))
                    .collect()
            });
            outcomes.extend(results);
        }
        outcomes
    }

    fn activated(&self, id: &NodeId, taken: &BTreeMap<NodeId, BTreeSet<Port>>) -> bool {
        let node = self.graph.node(id);
        if matches!(node.map(|n| &n.kind), Some(NodeKind::Trigger)) {
            return true;
        }
        self.graph.incoming(id).any(|edge| {
            taken
                .get(&edge.from)
                .is_some_and(|ports| ports.contains(&edge.port))
        })
    }

    fn node(
        &self,
        id: &NodeId,
        context: &ActiveProjectContext,
        outputs: &Outputs,
        start: Instant,
    ) -> Outcome {
        (self.on_event)(ExecutionEvent::NodeStarted { node: id.clone() });
        let begun = Instant::now();
        let started_ms = ms(start);
        let Some(node) = self.graph.node(id) else {
            return finished(idle(id, NodeStatus::Skipped, started_ms), vec![]);
        };
        let mut outcome = match &node.kind {
            NodeKind::Trigger => {
                let mut report = done(id, started_ms, "");
                report.output.clone_from(self.inputs);
                finished(report, vec![Port::Out])
            }
            NodeKind::Parallel | NodeKind::Join => {
                finished(done(id, started_ms, ""), vec![Port::Out])
            }
            NodeKind::Stop => {
                let mut outcome = finished(done(id, started_ms, "workflow stopped"), vec![]);
                outcome.stops_workflow = true;
                outcome
            }
            NodeKind::Delay { ms: wait } => {
                if self.cancel.wait(Duration::from_millis(u64::from(*wait))) {
                    finished(
                        done(id, started_ms, &format!("waited {wait} ms")),
                        vec![Port::Out],
                    )
                } else {
                    finished(idle(id, NodeStatus::Cancelled, started_ms), vec![])
                }
            }
            NodeKind::Condition { predicate } => {
                let met = self.evaluate(predicate, context, outputs);
                let port = if met { Port::True } else { Port::False };
                finished(
                    done(
                        id,
                        started_ms,
                        if met {
                            "condition met"
                        } else {
                            "condition not met"
                        },
                    ),
                    vec![port],
                )
            }
            NodeKind::Loop { workflow, times } => {
                self.run_loop(id, *workflow, *times, context, started_ms)
            }
            NodeKind::Call {
                workflow,
                inputs,
                policy,
            } => self.run_call(id, *workflow, inputs, *policy, context, outputs, started_ms),
            NodeKind::Output { values } => self.set_outputs(id, values, outputs, started_ms),
            NodeKind::Action {
                action,
                overrides,
                mappings,
                policy,
            } => self.run_action_node(
                id, *action, overrides, mappings, *policy, context, outputs, started_ms,
            ),
        };
        outcome.report.duration_ms = u64::try_from(begun.elapsed().as_millis()).unwrap_or(u64::MAX);
        outcome
    }

    fn evaluate(
        &self,
        predicate: &Predicate,
        context: &ActiveProjectContext,
        outputs: &Outputs,
    ) -> bool {
        match predicate {
            Predicate::Equals { reference, value } => lookup(outputs, reference) == Some(value),
            Predicate::NotEmpty { reference } => {
                lookup(outputs, reference).is_some_and(|v| !v.is_empty())
            }
            // Outputs are recorded for every succeeded node, and only for them.
            Predicate::Succeeded { node } => outputs.contains_key(node),
            Predicate::ActiveProject { key } => context
                .project(self.profile)
                .ok()
                .flatten()
                .is_some_and(|project| project.key.as_str() == key),
        }
    }

    fn run_loop(
        &self,
        id: &NodeId,
        workflow: WorkflowId,
        times: u32,
        context: &ActiveProjectContext,
        started_ms: u64,
    ) -> Outcome {
        let mut working = context.clone();
        for iteration in 1..=times {
            if self.cancel.is_cancelled() {
                return finished(idle(id, NodeStatus::Cancelled, started_ms), vec![]);
            }
            let run = (self.run_nested)(workflow, &BTreeMap::new(), &mut working, self.cancel);
            let succeeded = matches!(
                run,
                Ok(NestedRun {
                    status: RunStatus::Succeeded,
                    ..
                })
            );
            if !succeeded {
                // An iteration stopped by a cancellation is not a failure.
                if self.cancel.is_cancelled() {
                    let mut report = idle(id, NodeStatus::Cancelled, started_ms);
                    report.attempts = iteration;
                    return finished(report, vec![]);
                }
                let mut report = idle(id, NodeStatus::Failed, started_ms);
                report.attempts = iteration;
                report.error = Some(StepError::new(
                    "loop.failed",
                    format!("iteration {iteration} of {times} failed"),
                    false,
                ));
                let mut outcome = finished(report, vec![]);
                outcome.fails_workflow = true;
                return outcome;
            }
        }
        let mut report = done(id, started_ms, &format!("ran {times} times"));
        report.attempts = times;
        let mut outcome = finished(report, vec![Port::Out]);
        outcome.context = Some(working);
        outcome
    }

    /// Runs a sub-workflow once. See the module documentation.
    #[allow(clippy::too_many_arguments)]
    fn run_call(
        &self,
        id: &NodeId,
        workflow: WorkflowId,
        mappings: &BTreeMap<String, Mapping>,
        policy: super::ErrorPolicy,
        context: &ActiveProjectContext,
        outputs: &Outputs,
        started_ms: u64,
    ) -> Outcome {
        let failed = |code: &str, message: String| {
            let mut report = idle(id, NodeStatus::Failed, started_ms);
            report.workflow = Some(workflow);
            report.error = Some(StepError::new(code, message, false));
            report
        };
        if self.cancel.is_cancelled() {
            return finished(idle(id, NodeStatus::Cancelled, started_ms), vec![]);
        }
        let Some(callee) = self.profile.workflow(workflow) else {
            let report = failed("unavailable", "the workflow no longer exists".to_owned());
            return self.route_failure(report, policy.on_error, id);
        };
        let inputs = match self.call_inputs(callee, mappings, outputs) {
            Ok(inputs) => inputs,
            Err(message) => {
                let report = failed("mapping", message);
                return self.route_failure(report, policy.on_error, id);
            }
        };
        let timeout = Duration::from_millis(u64::from(
            policy.timeout_ms.unwrap_or(super::DEFAULT_CALL_TIMEOUT_MS),
        ));
        let begun = Instant::now();
        let token = self.cancel.child_with_deadline(begun + timeout);
        let mut working = context.clone();
        let result = (self.run_nested)(workflow, &inputs, &mut working, &token);
        let late = begun.elapsed() > timeout;
        let succeeded = matches!(&result, Ok(run) if run.status == RunStatus::Succeeded);
        if self.cancel.is_cancelled() && (!succeeded || late) {
            // Stopped by a cancellation of the run: not a failure.
            let mut report = idle(id, NodeStatus::Cancelled, started_ms);
            report.attempts = 1;
            report.workflow = Some(workflow);
            report.error = Some(StepError::new(
                "cancelled",
                "cancelled while running",
                false,
            ));
            return finished(report, vec![]);
        }
        let mut report = match result {
            Ok(_) if late || (token.is_cancelled() && !succeeded) => failed(
                "timeout",
                format!(
                    "{} did not finish within {} ms; its result was discarded",
                    callee.name,
                    timeout.as_millis()
                ),
            ),
            Ok(run) if run.status == RunStatus::Succeeded => {
                let mut report = done(id, started_ms, &format!("ran {}", callee.name));
                report.attempts = 1;
                report.workflow = Some(workflow);
                report.output = run
                    .outputs
                    .into_iter()
                    .filter(|(key, _)| callee.outputs.iter().any(|o| &o.key == key))
                    .collect();
                let mut outcome = finished(report, vec![Port::Out]);
                outcome.context = Some(working);
                return outcome;
            }
            Ok(run) => failed(
                "subworkflow.failed",
                match run.failure {
                    Some(failure) => format!("{} failed: {failure}", callee.name),
                    None => format!("{} failed", callee.name),
                },
            ),
            Err(error) => failed(
                "subworkflow.failed",
                format!("{} could not run: {error}", callee.name),
            ),
        };
        report.attempts = 1;
        self.route_failure(report, policy.on_error, id)
    }

    /// The values of `callee`'s inputs: mapped, otherwise the default;
    /// each converted to its declared kind. A required input needs a
    /// mapping.
    fn call_inputs(
        &self,
        callee: &crate::Workflow,
        mappings: &BTreeMap<String, Mapping>,
        outputs: &Outputs,
    ) -> Result<BTreeMap<String, String>, String> {
        if let Some(key) = mappings
            .keys()
            .find(|key| !callee.inputs.iter().any(|input| &input.key == *key))
        {
            return Err(format!("`{key}` is not an input of {}", callee.name));
        }
        let mut values = BTreeMap::new();
        for input in &callee.inputs {
            let text = match (mappings.get(&input.key), &input.default) {
                (Some(mapping), _) => resolve(mapping, outputs)?,
                (None, _) if input.required => {
                    return Err(format!("the input `{}` needs a value", input.key));
                }
                (None, Some(default)) => default.clone(),
                (None, None) => continue,
            };
            let value = convert(input.kind, &text, self.profile)
                .map_err(|e| format!("the input `{}` {e}", input.key))?;
            values.insert(input.key.clone(), value);
        }
        Ok(values)
    }

    /// An Output node: this workflow's declared outputs from earlier nodes,
    /// each converted to its declared kind.
    fn set_outputs(
        &self,
        id: &NodeId,
        mappings: &BTreeMap<String, Mapping>,
        outputs: &Outputs,
        started_ms: u64,
    ) -> Outcome {
        let declared = self
            .profile
            .workflow(self.workflow)
            .map(|w| w.outputs.as_slice())
            .unwrap_or_default();
        let values: Result<BTreeMap<String, String>, String> = mappings
            .iter()
            .map(|(key, mapping)| {
                let spec = declared
                    .iter()
                    .find(|o| &o.key == key)
                    .ok_or_else(|| format!("`{key}` is not an output of this workflow"))?;
                let text = resolve(mapping, outputs)?;
                let value =
                    convert(spec.kind, &text, self.profile).map_err(|e| format!("`{key}` {e}"))?;
                Ok((key.clone(), value))
            })
            .collect();
        match values {
            Ok(values) => {
                let mut report = done(id, started_ms, &format!("set {} outputs", values.len()));
                report.output = values;
                report.workflow = Some(self.workflow);
                finished(report, vec![Port::Out])
            }
            Err(message) => {
                let mut report = idle(id, NodeStatus::Failed, started_ms);
                report.workflow = Some(self.workflow);
                report.error = Some(StepError::new("mapping", message, false));
                let mut outcome = finished(report, vec![]);
                outcome.fails_workflow = true;
                outcome
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_action_node(
        &self,
        id: &NodeId,
        action: crate::ActionInstanceId,
        overrides: &ActionParameters,
        mappings: &BTreeMap<String, Mapping>,
        policy: super::ErrorPolicy,
        context: &ActiveProjectContext,
        outputs: &Outputs,
        started_ms: u64,
    ) -> Outcome {
        let fail = |report: NodeReport, error: StepError| {
            let mut report = report;
            report.status = NodeStatus::Failed;
            report.error = Some(error);
            report
        };
        let Some(instance) = self.profile.action(action) else {
            let report = fail(
                idle(id, NodeStatus::Failed, started_ms),
                StepError::new("unavailable", "the action no longer exists", false),
            );
            return self.route_failure(report, policy.on_error, id);
        };
        let definition = self.registry.definition(&instance.action_type);
        let parameters = match self.mapped_parameters(overrides, mappings, instance, outputs) {
            Ok(parameters) => parameters,
            Err(message) => {
                let mut report = fail(
                    idle(id, NodeStatus::Failed, started_ms),
                    StepError::new("mapping", message, false),
                );
                report.action = Some(action);
                return self.route_failure(report, policy.on_error, id);
            }
        };
        let spec = definition.map(|d| d.node.clone()).unwrap_or_default();
        let timeout = node_timeout(policy.timeout_ms, spec.timeout_ms);
        let attempts = 1 + if spec.retry_safe {
            policy.retry.attempts.min(super::MAX_RETRIES)
        } else {
            0
        };
        let mut last = None;
        for attempt in 1..=attempts {
            if self.cancel.is_cancelled() {
                let mut report = idle(id, NodeStatus::Cancelled, started_ms);
                report.attempts = attempt - 1;
                report.action = Some(action);
                return finished(report, vec![]);
            }
            let mut working = context.clone();
            let (status, error, message, output) = self.attempt(
                id,
                instance,
                &parameters,
                &mut working,
                timeout,
                spec.retry_safe,
            );
            let report = NodeReport {
                node: id.clone(),
                status,
                attempts: attempt,
                started_ms,
                duration_ms: 0,
                message,
                output,
                error,
                action: Some(action),
                workflow: None,
            };
            if status == NodeStatus::Succeeded {
                let mut outcome = finished(report, vec![Port::Out]);
                outcome.context = Some(working);
                return outcome;
            }
            if status == NodeStatus::Cancelled {
                // Stopped by a cancellation of the run: not a failure.
                return finished(report, vec![]);
            }
            last = Some(report);
            if attempt < attempts {
                let wait =
                    u64::from(policy.retry.backoff_ms).saturating_mul(1 << (attempt - 1).min(16));
                (self.on_event)(ExecutionEvent::Retrying {
                    node: id.clone(),
                    attempt: attempt + 1,
                    wait_ms: wait,
                });
                if !self.cancel.wait(Duration::from_millis(wait)) {
                    // Cancelled while waiting to retry.
                    let mut report = idle(id, NodeStatus::Cancelled, started_ms);
                    report.attempts = attempt;
                    report.action = Some(action);
                    report.error = Some(StepError::new(
                        "cancelled",
                        "cancelled while waiting to retry",
                        false,
                    ));
                    return finished(report, vec![]);
                }
            }
        }
        let report = last.unwrap_or_else(|| idle(id, NodeStatus::Failed, started_ms));
        self.route_failure(report, policy.on_error, id)
    }

    /// Runs one attempt of an action with its own token, cancelled with the
    /// run or at the timeout, and classifies the result.
    fn attempt(
        &self,
        id: &NodeId,
        instance: &ActionInstance,
        parameters: &ActionParameters,
        working: &mut ActiveProjectContext,
        timeout: Duration,
        retry_safe: bool,
    ) -> (
        NodeStatus,
        Option<StepError>,
        String,
        BTreeMap<String, String>,
    ) {
        let begun = Instant::now();
        let token = self.cancel.child_with_deadline(begun + timeout);
        let progress = |progress: crate::Progress| {
            (self.on_event)(ExecutionEvent::NodeProgress {
                node: id.clone(),
                progress,
            });
        };
        let mut summary = self
            .summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        summary.elapsed_ms = ms(self.started);
        let control = crate::ExecutionControl::new(&token, &progress).with_summary(summary);
        let result = (self.run_action)(instance, parameters, working, &control);
        let late = begun.elapsed() > timeout;
        classify(
            result,
            late,
            timeout,
            retry_safe,
            self.cancel.is_cancelled(),
        )
    }

    /// A failed action continues through `error` if connected, through `out`
    /// with `Continue`, and otherwise fails the workflow.
    fn route_failure(&self, report: NodeReport, on_error: OnError, id: &NodeId) -> Outcome {
        if self.graph.outgoing(id).any(|e| e.port == Port::Error) {
            return finished(report, vec![Port::Error]);
        }
        match on_error {
            OnError::Continue => finished(report, vec![Port::Out]),
            OnError::Stop => {
                let mut outcome = finished(report, vec![]);
                outcome.fails_workflow = true;
                outcome
            }
        }
    }

    /// Overrides plus mapped values, converted to the parameters' kinds.
    fn mapped_parameters(
        &self,
        overrides: &ActionParameters,
        mappings: &BTreeMap<String, Mapping>,
        instance: &ActionInstance,
        outputs: &Outputs,
    ) -> Result<ActionParameters, String> {
        let mut parameters = overrides.clone();
        let definition = self.registry.definition(&instance.action_type);
        for (key, mapping) in mappings {
            let text = resolve(mapping, outputs)?;
            let kind = definition
                .and_then(|d| d.parameters.iter().find(|spec| &spec.key == key))
                .map(|spec| &spec.kind)
                .ok_or_else(|| format!("`{key}` is not a parameter of this action"))?;
            let value = match kind {
                ParameterKind::Text { .. }
                | ParameterKind::Lines { .. }
                | ParameterKind::Pairs { .. }
                | ParameterKind::Code { .. }
                | ParameterKind::Credential => ParameterValue::Text(text),
                ParameterKind::Integer { .. } => ParameterValue::Integer(
                    text.trim()
                        .parse()
                        .map_err(|_| format!("`{key}` expects a whole number, got “{text}”"))?,
                ),
                ParameterKind::Boolean => match text.trim() {
                    "true" => ParameterValue::Boolean(true),
                    "false" => ParameterValue::Boolean(false),
                    _ => return Err(format!("`{key}` expects true or false, got “{text}”")),
                },
                ParameterKind::Choice { .. } => ParameterValue::Choice(text),
                ParameterKind::Project => {
                    let project = self
                        .profile
                        .projects
                        .iter()
                        .find(|p| p.key.as_str() == text.trim())
                        .ok_or_else(|| format!("this profile has no project “{text}”"))?;
                    ParameterValue::Project(project.id)
                }
                ParameterKind::Resource { .. } => {
                    return Err(format!("`{key}` refers to a resource and cannot be mapped"));
                }
            };
            parameters = parameters.with(key.clone(), value);
        }
        Ok(parameters)
    }
}

fn ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn idle(id: &NodeId, status: NodeStatus, started_ms: u64) -> NodeReport {
    NodeReport {
        node: id.clone(),
        status,
        attempts: 0,
        started_ms,
        duration_ms: 0,
        message: String::new(),
        output: BTreeMap::new(),
        error: None,
        action: None,
        workflow: None,
    }
}

fn done(id: &NodeId, started_ms: u64, message: &str) -> NodeReport {
    let mut report = idle(id, NodeStatus::Succeeded, started_ms);
    message.clone_into(&mut report.message);
    report
}

fn finished(report: NodeReport, ports: Vec<Port>) -> Outcome {
    Outcome {
        report,
        ports,
        fails_workflow: false,
        stops_workflow: false,
        context: None,
    }
}

/// The outcome of one attempt: status, error, message and outputs.
///
/// - An action that succeeded in time succeeded, even if the run was
///   cancelled meanwhile: its effects happened.
/// - Otherwise, when the run was cancelled, the attempt is cancelled — not
///   failed and not timed out.
/// - A late result is a timeout, whatever it was.
fn classify(
    result: Result<ActionResult, EngineError>,
    late: bool,
    timeout: Duration,
    retry_safe: bool,
    cancelled: bool,
) -> (
    NodeStatus,
    Option<StepError>,
    String,
    BTreeMap<String, String>,
) {
    let succeeded = matches!(&result, Ok(r) if r.is_success());
    if cancelled && (!succeeded || late) {
        let message = result.map(|r| r.message).unwrap_or_default();
        return (
            NodeStatus::Cancelled,
            Some(StepError::new(
                "cancelled",
                "cancelled while running",
                false,
            )),
            message,
            BTreeMap::new(),
        );
    }
    match result {
        Ok(result) if late => (
            NodeStatus::Failed,
            Some(StepError::new(
                "timeout",
                format!(
                    "did not finish within {} ms; its result was discarded",
                    timeout.as_millis()
                ),
                true,
            )),
            result.message,
            BTreeMap::new(),
        ),
        Ok(result) if result.is_success() => {
            (NodeStatus::Succeeded, None, result.message, result.data)
        }
        Ok(result) => (
            NodeStatus::Failed,
            Some(StepError {
                action_code: result.code.clone(),
                ..StepError::new("action.failed", result.message.clone(), retry_safe)
            }),
            result.message,
            BTreeMap::new(),
        ),
        Err(error) => (
            NodeStatus::Failed,
            Some(StepError::new("action.failed", error.to_string(), false)),
            String::new(),
            BTreeMap::new(),
        ),
    }
}

/// A node's time limit: its override, else its action type's own, and never
/// more than [`super::MAX_TIMEOUT_MS`]. Longer limits of an action (a
/// command's or Compose's) apply only when it runs outside a workflow.
pub(crate) fn node_timeout(override_ms: Option<u32>, declared_ms: u32) -> Duration {
    Duration::from_millis(u64::from(
        override_ms
            .unwrap_or(declared_ms)
            .min(super::MAX_TIMEOUT_MS),
    ))
}
