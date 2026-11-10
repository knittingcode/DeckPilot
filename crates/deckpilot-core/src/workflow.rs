//! Workflows: ordered sequences of action instances.

use serde::{Deserialize, Serialize};

use crate::{ActionInstanceId, WorkflowId};

/// What to do when a workflow step fails.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnFailure {
    #[default]
    Stop,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    pub action: ActionInstanceId,
    #[serde(default)]
    pub on_failure: OnFailure,
}

/// A named sequence of steps. Each step references an action instance of the
/// same profile, so two profiles can define a `Start Work` workflow with
/// entirely different steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: WorkflowId,
    pub name: String,
    pub steps: Vec<WorkflowStep>,
}

impl Workflow {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: WorkflowId::new(),
            name: name.into(),
            steps: Vec::new(),
        }
    }

    #[must_use]
    pub fn step(mut self, action: ActionInstanceId, on_failure: OnFailure) -> Self {
        self.steps.push(WorkflowStep { action, on_failure });
        self
    }
}
