//! Workflow graphs: nodes connected by typed edges.
//!
//! A [`WorkflowGraph`] is the visual form of a workflow. It is stored in the
//! workflow next to the legacy step list (`Workflow::steps`); a workflow
//! without a graph keeps running with the sequential engine exactly as
//! before. Graphs are acyclic: repetition is a [`NodeKind::Loop`] that runs
//! another workflow a bounded number of times.
//!
//! Module-provided nodes are action types. What a node offers to the editor
//! and the engine — outputs, timeout, retry safety — is declared in
//! [`NodeSpec`] on the action type, so the editor never needs module code.

mod convert;
pub(crate) mod executor;
mod mapping;
mod validate;

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ActionInstanceId, ActionParameters, WorkflowId};

pub use convert::{Conversion, from_steps};
pub use executor::{
    CancelReason, CancellationToken, ExecutionEvent, ExecutionLimits, GraphReport, NodeReport,
    NodeStatus, RunStatus, StepError,
};
pub use mapping::{Reference, TemplateError, parse_template, references_in};
pub use validate::{GraphIssue, IssueSeverity};

/// Format version of [`WorkflowGraph`]. Bumped when the stored shape changes;
/// older versions are migrated when a profile is loaded.
pub const GRAPH_VERSION: u32 = 1;

/// Upper bounds that keep every workflow finite.
pub const MAX_NODES: usize = 200;
pub const MAX_LOOP_ITERATIONS: u32 = 100;
pub const MAX_NESTING: usize = 4;
pub const MAX_RETRIES: u32 = 5;
pub const MAX_TIMEOUT_MS: u32 = 10 * 60 * 1000;
pub const MAX_DELAY_MS: u32 = 10 * 60 * 1000;

/// Identifier of a node, unique within its graph, e.g. `git_status`. Used in
/// data mappings (`{{ steps.git_status.output.branch }}`), so it is a
/// restricted, readable slug.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NodeId(String);

impl NodeId {
    pub fn new(id: impl Into<String>) -> Result<Self, String> {
        let id = id.into();
        let valid = !id.is_empty()
            && id.len() <= 48
            && id.starts_with(|c: char| c.is_ascii_lowercase())
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if valid {
            Ok(Self(id))
        } else {
            Err(format!(
                "invalid node id `{id}`: use 1–48 lowercase letters, digits and `_`, starting with a letter"
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NodeId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<NodeId> for String {
    fn from(value: NodeId) -> Self {
        value.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An output port of a node. Which ports exist depends on the node kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Port {
    /// Normal continuation (actions: on success, or on failure with
    /// `OnError::Continue`).
    Out,
    /// Action failure, when connected.
    Error,
    /// Condition met.
    True,
    /// Condition not met.
    False,
}

/// Position in the editor; ignored by the engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

// `f64` is never NaN here; positions are compared as data.
impl Eq for Position {}

/// What happens when an action node fails and its error port is not
/// connected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnError {
    /// The workflow fails; running parallel branches finish, nothing new
    /// starts.
    #[default]
    Stop,
    /// The workflow continues through the `out` port.
    Continue,
}

/// Retry of a failed action. Only allowed for action types that declare
/// retries as safe ([`NodeSpec::retry_safe`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryPolicy {
    /// Additional attempts after the first, at most [`MAX_RETRIES`].
    pub attempts: u32,
    /// Wait before the first retry; doubles for each further one.
    pub backoff_ms: u32,
}

/// Error handling of an action node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPolicy {
    #[serde(default)]
    pub on_error: OnError,
    #[serde(default)]
    pub retry: RetryPolicy,
    /// Overrides the action type's timeout, at most [`MAX_TIMEOUT_MS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
}

/// A value for an action parameter in this node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Mapping {
    /// The whole value comes from an earlier node's output.
    Output {
        reference: Reference,
        /// Used when the output is missing at run time. Without it, a missing
        /// output fails the node.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
    },
    /// Text with `{{ steps.<node>.output.<key> }}` placeholders. Plain
    /// substitution only: no expressions, no code.
    Template { template: String },
}

/// What a condition node tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Predicate {
    /// An earlier node's output equals a value.
    Equals { reference: Reference, value: String },
    /// An earlier node's output exists and is not empty.
    NotEmpty { reference: Reference },
    /// An earlier node finished successfully (and was not skipped).
    Succeeded { node: NodeId },
    /// The active project — as captured at the start or changed by an
    /// earlier node — is this project key.
    ActiveProject { key: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NodeKind {
    /// Where the workflow starts. Exactly one per graph.
    Trigger,
    /// Runs an action instance of the profile.
    Action {
        action: ActionInstanceId,
        /// Fixed values that replace the action's own for this node.
        #[serde(default, skip_serializing_if = "ActionParameters::is_empty")]
        overrides: ActionParameters,
        /// Values from earlier nodes, by parameter key.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        mappings: BTreeMap<String, Mapping>,
        #[serde(default)]
        policy: ErrorPolicy,
    },
    /// Continues through `true` or `false`.
    Condition { predicate: Predicate },
    /// Starts every branch connected to `out` at the same time.
    Parallel,
    /// Waits until every incoming branch has finished or was skipped.
    Join,
    /// Runs another workflow of the profile `times` times in sequence.
    Loop { workflow: WorkflowId, times: u32 },
    /// Waits, then continues. Cancellation interrupts the wait.
    Delay { ms: u32 },
    /// Ends the workflow successfully; running branches finish.
    Stop,
}

impl NodeKind {
    /// Output ports the kind offers.
    pub fn ports(&self) -> &'static [Port] {
        match self {
            Self::Action { .. } => &[Port::Out, Port::Error],
            Self::Condition { .. } => &[Port::True, Port::False],
            Self::Stop => &[],
            Self::Trigger
            | Self::Parallel
            | Self::Join
            | Self::Loop { .. }
            | Self::Delay { .. } => &[Port::Out],
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Trigger => "trigger",
            Self::Action { .. } => "action",
            Self::Condition { .. } => "condition",
            Self::Parallel => "parallel",
            Self::Join => "join",
            Self::Loop { .. } => "loop",
            Self::Delay { .. } => "delay",
            Self::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: NodeId,
    #[serde(flatten)]
    pub kind: NodeKind,
    /// Shown instead of the default title.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default)]
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub from: NodeId,
    #[serde(default = "default_port")]
    pub port: Port,
    pub to: NodeId,
}

fn default_port() -> Port {
    Port::Out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowGraph {
    pub version: u32,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Default for WorkflowGraph {
    fn default() -> Self {
        Self {
            version: GRAPH_VERSION,
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

impl WorkflowGraph {
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| &n.id == id)
    }

    pub fn trigger(&self) -> Option<&Node> {
        self.nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Trigger))
    }

    pub fn outgoing<'a>(&'a self, id: &'a NodeId) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |e| &e.from == id)
    }

    pub fn incoming<'a>(&'a self, id: &'a NodeId) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |e| &e.to == id)
    }

    /// Nodes in a deterministic topological order (by id among peers), or
    /// `None` if the graph has a cycle.
    pub fn topological_order(&self) -> Option<Vec<&NodeId>> {
        let mut indegree: BTreeMap<&NodeId, usize> =
            self.nodes.iter().map(|n| (&n.id, 0)).collect();
        for edge in &self.edges {
            if let Some(count) = indegree.get_mut(&edge.to) {
                *count += 1;
            }
        }
        let mut ready: Vec<&NodeId> = indegree
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(id) = ready.first().copied() {
            ready.remove(0);
            order.push(id);
            for edge in self.outgoing(id) {
                if let Some(count) = indegree.get_mut(&edge.to) {
                    *count -= 1;
                    if *count == 0 {
                        let position = ready.partition_point(|other| *other < &edge.to);
                        ready.insert(position, &edge.to);
                    }
                }
            }
        }
        (order.len() == self.nodes.len()).then_some(order)
    }
}

/// What kind of value an output holds; used to check mappings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueKind {
    #[default]
    Text,
    Integer,
    Boolean,
    Path,
    Url,
    /// A project key of the profile.
    ProjectKey,
}

/// A structured output an action provides on success, e.g. `branch`.
///
/// Built with [`OutputSpec::new`] and the `sensitive`/`optional` helpers in
/// module code, so new fields do not break modules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSpec {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub kind: ValueKind,
    /// The output is present on every success.
    #[serde(default)]
    pub always: bool,
    /// The value may contain data that must not be kept, such as a
    /// program's output or a response body: later nodes can use it, but
    /// execution history and live run events leave it out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
}

impl OutputSpec {
    /// An output present on every success.
    pub fn new(key: impl Into<String>, label: impl Into<String>, kind: ValueKind) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
            always: true,
            sensitive: false,
        }
    }

    /// Present only on some successes.
    #[must_use]
    pub fn optional(self) -> Self {
        Self {
            always: false,
            ..self
        }
    }

    /// Kept out of history and live events; see [`OutputSpec::sensitive`].
    #[must_use]
    pub fn sensitive(self) -> Self {
        Self {
            sensitive: true,
            ..self
        }
    }
}

/// What running an action does to the computer or elsewhere, from the
/// user's point of view. Shown in the catalog and the inspector, and used
/// to ask for explicit permission where needed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionEffect {
    /// Only reads: status, lists, checks. Safe to run any time.
    #[default]
    ReadOnly,
    /// Opens something for the user (an application, a page, a folder)
    /// without changing data.
    Opens,
    /// Changes state: runs a command, starts or stops services, sends a
    /// request that may change data.
    Modifies,
}

/// How an action type behaves as a workflow node: the module SDK contract
/// for the editor and the engine. Declared by the module on its action type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
// Independent, declared properties of the node contract.
#[allow(clippy::struct_excessive_bools)]
pub struct NodeSpec {
    /// Version of the node's parameter and output schema.
    pub version: u32,
    /// Icon name from the UI icon set.
    pub icon: Option<String>,
    pub outputs: Vec<OutputSpec>,
    /// Default execution deadline.
    pub timeout_ms: u32,
    /// Running the action again after a failure is safe.
    pub retry_safe: bool,
    /// Running the action twice has the same effect as once.
    pub idempotent: bool,
    /// The action selects a project: it changes the run's project context
    /// for the nodes after it.
    pub changes_context: bool,
    /// The action checks its cancellation token (`ExecutionContext::
    /// is_cancelled`, `wait`) and stops promptly when the run is cancelled
    /// or its timeout passes. When `false`, a running action finishes before
    /// cancellation or a timeout takes effect, and the editor says so.
    pub cancellable: bool,
    /// What running the action does; see [`ActionEffect`].
    pub effect: ActionEffect,
}

impl Default for NodeSpec {
    fn default() -> Self {
        Self {
            version: 1,
            icon: None,
            outputs: Vec::new(),
            timeout_ms: 30_000,
            retry_safe: false,
            idempotent: false,
            changes_context: false,
            cancellable: false,
            effect: ActionEffect::ReadOnly,
        }
    }
}

#[cfg(test)]
mod tests;
