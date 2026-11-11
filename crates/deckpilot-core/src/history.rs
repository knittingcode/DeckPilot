//! Records of workflow runs: what the agent reports while a workflow runs
//! and keeps afterwards as execution history.
//!
//! Records only hold what is safe to show and to store: node outputs are
//! limited to the outputs the node's action type declares (see
//! [`NodeSpec::outputs`](crate::graph::NodeSpec)), and messages are
//! shortened. Parameters and undeclared action data are never recorded.

use serde::{Deserialize, Serialize};

use crate::graph::{ExecutionEvent, GraphReport, NodeId, NodeReport, NodeStatus, RunStatus};
use crate::{
    ActionRegistry, ExecutionId, Profile, ProfileId, ProjectId, ProjectScope, Workflow, WorkflowId,
};

/// Longest message kept, in characters.
pub const MAX_MESSAGE_CHARS: usize = 500;
/// Most outputs kept per node.
pub const MAX_OUTPUTS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// The agent stopped before the run finished.
    Interrupted,
}

impl ExecutionState {
    pub fn is_finished(self) -> bool {
        self != Self::Running
    }

    /// Stable name, as stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}

impl From<RunStatus> for ExecutionState {
    fn from(status: RunStatus) -> Self {
        match status {
            RunStatus::Succeeded => Self::Succeeded,
            RunStatus::Failed => Self::Failed,
            RunStatus::Cancelled => Self::Cancelled,
        }
    }
}

/// What started a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionTrigger {
    /// The configuration app.
    App,
    /// A Stream Deck control.
    Device,
}

/// What a finished run did to the application's active project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProjectOutcome {
    /// The run ended on the project it started with.
    Unchanged,
    /// The run selected `project`; the change stayed in the run (its
    /// workflow may not change the active project).
    KeptInRun { project: Option<ProjectId> },
    /// `project` became the active project.
    Applied { project: ProjectId },
    /// The workflow may change the active project, but this change was not
    /// made, e.g. because the active project changed during the run.
    NotApplied {
        project: Option<ProjectId>,
        reason: String,
    },
}

/// A run without its node details, for lists and live updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSummary {
    pub id: ExecutionId,
    pub workflow: WorkflowId,
    /// The workflow's name when the run started.
    pub workflow_name: String,
    pub profile: ProfileId,
    pub trigger: ExecutionTrigger,
    pub state: ExecutionState,
    /// Milliseconds since the Unix epoch.
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// Nodes (or steps) of the workflow, and how many are resolved: real
    /// progress, not an estimate of time.
    pub total_nodes: u32,
    pub completed_nodes: u32,
    /// Nodes running right now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running_nodes: Vec<NodeId>,
    /// Why the run failed, was cancelled or was interrupted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// The active project when the run started: the run's initial context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_project: Option<ProjectId>,
    /// Whether the run could change the active project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_scope: Option<ProjectScope>,
    /// What the finished run did to the active project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectOutcome>,
}

/// A run with the report of every resolved node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRecord {
    #[serde(flatten)]
    pub summary: ExecutionSummary,
    /// In the order the nodes were resolved.
    #[serde(default)]
    pub nodes: Vec<NodeReport>,
}

impl ExecutionRecord {
    /// A run of `workflow` that starts now.
    pub fn start(
        id: ExecutionId,
        workflow: &Workflow,
        profile: &Profile,
        trigger: ExecutionTrigger,
        now_ms: u64,
    ) -> Self {
        let total = workflow
            .graph
            .as_ref()
            .map_or(workflow.steps.len(), |graph| graph.nodes.len());
        Self {
            summary: ExecutionSummary {
                id,
                workflow: workflow.id,
                workflow_name: shorten(&workflow.name),
                profile: profile.id,
                trigger,
                state: ExecutionState::Running,
                started_at: now_ms,
                finished_at: None,
                total_nodes: u32::try_from(total).unwrap_or(u32::MAX),
                completed_nodes: 0,
                running_nodes: Vec::new(),
                message: String::new(),
                initial_project: None,
                project_scope: None,
                project: None,
            },
            nodes: Vec::new(),
        }
    }

    /// Applies a progress event of a running run. The event must already be
    /// made safe with [`safe_event`].
    pub fn apply(&mut self, event: &ExecutionEvent) {
        if self.summary.state.is_finished() {
            return;
        }
        match event {
            ExecutionEvent::NodeStarted { node } => {
                if !self.summary.running_nodes.contains(node) {
                    self.summary.running_nodes.push(node.clone());
                }
            }
            ExecutionEvent::Retrying { .. } | ExecutionEvent::NodeProgress { .. } => {}
            ExecutionEvent::NodeFinished { report } => {
                self.summary.running_nodes.retain(|n| n != &report.node);
                match self.nodes.iter_mut().find(|n| n.node == report.node) {
                    Some(existing) => *existing = report.clone(),
                    None => self.nodes.push(report.clone()),
                }
                self.summary.completed_nodes = u32::try_from(self.nodes.len())
                    .unwrap_or(u32::MAX)
                    .min(self.summary.total_nodes);
            }
        }
    }

    /// Completes the run with its final report, made safe with
    /// [`safe_report`]. The report replaces the nodes seen so far.
    pub fn finish(&mut self, report: GraphReport, now_ms: u64) {
        self.summary.state = report.status.into();
        self.summary.message = match report.status {
            RunStatus::Succeeded => String::new(),
            RunStatus::Cancelled => "cancelled".into(),
            RunStatus::Failed => report
                .nodes
                .iter()
                .find(|n| n.status == NodeStatus::Failed)
                .and_then(|n| n.error.as_ref())
                .map(|e| shorten(&e.message))
                .unwrap_or_default(),
        };
        self.nodes = report.nodes;
        self.close(now_ms);
    }

    /// Ends the run without a report: it could not run, or the agent
    /// stopped.
    pub fn end(&mut self, state: ExecutionState, message: &str, now_ms: u64) {
        self.summary.state = state;
        self.summary.message = shorten(message);
        for node in &self.summary.running_nodes {
            if !self.nodes.iter().any(|n| &n.node == node) {
                self.nodes.push(NodeReport {
                    node: node.clone(),
                    status: NodeStatus::Cancelled,
                    attempts: 0,
                    started_ms: now_ms.saturating_sub(self.summary.started_at),
                    duration_ms: 0,
                    message: String::new(),
                    output: std::collections::BTreeMap::new(),
                    error: None,
                    action: None,
                });
            }
        }
        self.close(now_ms);
    }

    fn close(&mut self, now_ms: u64) {
        self.summary.finished_at = Some(now_ms.max(self.summary.started_at));
        self.summary.running_nodes.clear();
        self.summary.completed_nodes = u32::try_from(self.nodes.len())
            .unwrap_or(u32::MAX)
            .min(self.summary.total_nodes);
    }
}

/// `report` with what may be shown and stored: outputs the node's action
/// type declares and does not mark sensitive, and shortened messages.
pub fn safe_report(
    report: &NodeReport,
    profile: &Profile,
    registry: &ActionRegistry,
) -> NodeReport {
    let declared: Vec<&str> = report
        .action
        .and_then(|action| profile.action(action))
        .and_then(|instance| registry.definition(&instance.action_type))
        .map(|definition| {
            definition
                .node
                .outputs
                .iter()
                .filter(|o| !o.sensitive)
                .map(|o| o.key.as_str())
                .collect()
        })
        .unwrap_or_default();
    let mut safe = report.clone();
    safe.message = shorten(&report.message);
    safe.output = report
        .output
        .iter()
        .filter(|(key, _)| declared.contains(&key.as_str()))
        .take(MAX_OUTPUTS)
        .map(|(key, value)| (key.clone(), shorten(value)))
        .collect();
    if let Some(error) = &mut safe.error {
        error.message = shorten(&error.message);
    }
    safe
}

/// `event` with its report made safe; see [`safe_report`].
pub fn safe_event(
    event: ExecutionEvent,
    profile: &Profile,
    registry: &ActionRegistry,
) -> ExecutionEvent {
    match event {
        ExecutionEvent::NodeFinished { report } => ExecutionEvent::NodeFinished {
            report: safe_report(&report, profile, registry),
        },
        ExecutionEvent::NodeProgress { node, progress } => ExecutionEvent::NodeProgress {
            node,
            progress: crate::Progress {
                percent: progress.percent.map(|p| p.min(100)),
                message: shorten(&progress.message),
            },
        },
        other => other,
    }
}

/// `text` limited to [`MAX_MESSAGE_CHARS`] characters.
pub fn shorten(text: &str) -> String {
    match text.char_indices().nth(MAX_MESSAGE_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::graph::StepError;

    fn report(node: &str, status: NodeStatus) -> NodeReport {
        NodeReport {
            node: NodeId::new(node).unwrap(),
            status,
            attempts: 1,
            started_ms: 0,
            duration_ms: 5,
            message: String::new(),
            output: BTreeMap::new(),
            error: None,
            action: None,
        }
    }

    fn record() -> ExecutionRecord {
        let profile = Profile::new("Test");
        let mut workflow = Workflow::new("Build");
        workflow.steps = vec![
            crate::WorkflowStep::new(crate::ActionInstanceId::new(), crate::OnFailure::Stop),
            crate::WorkflowStep::new(crate::ActionInstanceId::new(), crate::OnFailure::Stop),
        ];
        ExecutionRecord::start(
            ExecutionId::new(),
            &workflow,
            &profile,
            ExecutionTrigger::App,
            1_000,
        )
    }

    #[test]
    fn progress_follows_events() {
        let mut record = record();
        assert_eq!(record.summary.total_nodes, 2);
        let step = NodeId::new("step_1").unwrap();
        record.apply(&ExecutionEvent::NodeStarted { node: step.clone() });
        assert_eq!(record.summary.running_nodes, vec![step]);
        record.apply(&ExecutionEvent::NodeFinished {
            report: report("step_1", NodeStatus::Succeeded),
        });
        assert_eq!(record.summary.running_nodes, Vec::<NodeId>::new());
        assert_eq!(record.summary.completed_nodes, 1);
        // Repeated reports for a node replace it.
        record.apply(&ExecutionEvent::NodeFinished {
            report: report("step_1", NodeStatus::Succeeded),
        });
        assert_eq!(record.summary.completed_nodes, 1);
    }

    #[test]
    fn finishing_takes_the_report_and_the_failure() {
        let mut record = record();
        let mut failed = report("step_2", NodeStatus::Failed);
        failed.error = Some(StepError {
            code: "action.failed".into(),
            message: "no such file".into(),
            retryable: false,
            action_code: None,
        });
        record.finish(
            GraphReport {
                workflow: record.summary.workflow,
                status: RunStatus::Failed,
                duration_ms: 9,
                nodes: vec![report("step_1", NodeStatus::Succeeded), failed],
                context_conflict: None,
            },
            2_000,
        );
        assert_eq!(record.summary.state, ExecutionState::Failed);
        assert_eq!(record.summary.message, "no such file");
        assert_eq!(record.summary.finished_at, Some(2_000));
        assert_eq!(record.summary.completed_nodes, 2);
        // Finished runs ignore late events.
        record.apply(&ExecutionEvent::NodeStarted {
            node: NodeId::new("step_1").unwrap(),
        });
        assert_eq!(record.summary.running_nodes, Vec::<NodeId>::new());
    }

    #[test]
    fn interrupted_runs_mark_running_nodes() {
        let mut record = record();
        record.apply(&ExecutionEvent::NodeStarted {
            node: NodeId::new("step_1").unwrap(),
        });
        record.end(ExecutionState::Interrupted, "the agent stopped", 1_500);
        assert_eq!(record.summary.state, ExecutionState::Interrupted);
        assert_eq!(record.nodes[0].status, NodeStatus::Cancelled);
        assert_eq!(record.summary.running_nodes, Vec::<NodeId>::new());
    }

    #[test]
    fn reports_keep_only_declared_outputs() {
        let profile = Profile::new("Test");
        let registry = ActionRegistry::default();
        let mut node = report("step_1", NodeStatus::Succeeded);
        node.output.insert("token".into(), "secret".into());
        node.message = "x".repeat(MAX_MESSAGE_CHARS + 10);
        let safe = safe_report(&node, &profile, &registry);
        assert!(safe.output.is_empty());
        assert_eq!(safe.message.chars().count(), MAX_MESSAGE_CHARS + 1);
        assert_eq!(shorten("äöü"), "äöü");
    }

    #[test]
    fn records_round_trip_as_json() {
        let record = record();
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains(r#""state":"running""#));
        assert_eq!(
            serde_json::from_str::<ExecutionRecord>(&json).unwrap(),
            record
        );
    }
}
