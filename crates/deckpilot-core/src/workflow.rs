//! Workflows: ordered sequences of action instances.

use serde::{Deserialize, Serialize};

use crate::{ActionInstanceId, ActionParameters, ActionRegistry, Profile, ProjectId, WorkflowId};

/// What to do when a workflow step fails.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnFailure {
    #[default]
    Stop,
    Continue,
}

/// When a step runs. Skipped steps do not count as failures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StepCondition {
    #[default]
    Always,
    /// Only if the most recent executed step succeeded.
    PreviousSucceeded,
    /// Only if the most recent executed step failed.
    PreviousFailed,
    /// Only while a specific project is active.
    ActiveProject { project: ProjectId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    pub action: ActionInstanceId,
    #[serde(default)]
    pub on_failure: OnFailure,
    #[serde(default)]
    pub condition: StepCondition,
    /// Parameter values that replace the action's own values for this step.
    #[serde(default, skip_serializing_if = "ActionParameters::is_empty")]
    pub overrides: ActionParameters,
}

impl WorkflowStep {
    pub fn new(action: ActionInstanceId, on_failure: OnFailure) -> Self {
        Self {
            action,
            on_failure,
            condition: StepCondition::Always,
            overrides: ActionParameters::new(),
        }
    }
}

/// A named sequence of steps. Each step references an action instance of the
/// same profile, so two profiles can define a `Start Work` workflow with
/// entirely different steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: WorkflowId,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub steps: Vec<WorkflowStep>,
}

impl Workflow {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: WorkflowId::new(),
            name: name.into(),
            description: String::new(),
            steps: Vec::new(),
        }
    }

    #[must_use]
    pub fn step(mut self, action: ActionInstanceId, on_failure: OnFailure) -> Self {
        self.steps.push(WorkflowStep::new(action, on_failure));
        self
    }

    /// Problems that prevent saving the workflow in `profile`.
    pub fn validate(&self, profile: &Profile, registry: &ActionRegistry) -> Vec<String> {
        let mut issues = Vec::new();
        if self.name.trim().is_empty() {
            issues.push("workflow name must not be empty".to_owned());
        }
        for (index, step) in self.steps.iter().enumerate() {
            let position = index + 1;
            let Some(action) = profile.action(step.action) else {
                issues.push(format!(
                    "step {position}: the action does not exist in this profile"
                ));
                continue;
            };
            if !step.overrides.is_empty()
                && let Err(error) = registry.normalize(
                    &action.action_type,
                    &action.parameters.overlay(&step.overrides),
                )
            {
                issues.push(format!("step {position} ({}): {error}", action.name));
            }
            if let StepCondition::ActiveProject { project } = step.condition
                && profile.project(project).is_none()
            {
                issues.push(format!(
                    "step {position}: the condition refers to a missing project"
                ));
            }
            if index == 0
                && matches!(
                    step.condition,
                    StepCondition::PreviousSucceeded | StepCondition::PreviousFailed
                )
            {
                issues.push("step 1: there is no previous step to depend on".to_owned());
            }
        }
        issues
    }
}
