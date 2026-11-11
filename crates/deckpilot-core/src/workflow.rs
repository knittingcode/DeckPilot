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

/// Whether a workflow may change the application's active project.
///
/// A run always has its own project context (see [`crate::graph`]): project
/// selections inside it affect the nodes after them and nothing else. Only
/// with [`ProjectScope::Application`] does the project a run ends with
/// become DeckPilot's active project, and only if nothing else changed the
/// active project during the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectScope {
    /// Project changes stay in the run. The default for new workflows.
    Execution,
    /// The workflow may change DeckPilot's active project: a permission the
    /// user grants per workflow.
    Application,
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
    /// The visual form. When present it is what runs; `steps` is then only
    /// kept for reference and for the editor's conversion notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<crate::graph::WorkflowGraph>,
    /// Whether the workflow may change the active project. Unset for
    /// workflows saved before this setting existed; see
    /// [`Workflow::effective_project_scope`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_scope: Option<ProjectScope>,
    /// Values the workflow takes when a Sub-workflow node calls it. Graph
    /// workflows only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<crate::graph::WorkflowInput>,
    /// Values the workflow returns to a Sub-workflow node, set by its
    /// Output nodes. Graph workflows only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<crate::graph::WorkflowOutput>,
    /// Schedules and lifecycle events that start the workflow in the agent
    /// (see [`crate::trigger`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<crate::trigger::WorkflowTrigger>,
    /// The triggers are paused: none of them starts the workflow.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub triggers_paused: bool,
}

impl Workflow {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: WorkflowId::new(),
            name: name.into(),
            description: String::new(),
            steps: Vec::new(),
            graph: None,
            project_scope: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            triggers: Vec::new(),
            triggers_paused: false,
        }
    }

    /// The project scope that applies. A workflow saved before the setting
    /// existed changed the active project whenever one of its actions
    /// selected a project; it keeps doing so, so that upgrading changes
    /// nothing. Any other workflow keeps project changes in its runs.
    pub fn effective_project_scope(
        &self,
        profile: &Profile,
        registry: &ActionRegistry,
    ) -> ProjectScope {
        match self.project_scope {
            Some(scope) => scope,
            None if self.selects_projects(profile, registry) => ProjectScope::Application,
            None => ProjectScope::Execution,
        }
    }

    /// Whether any of the workflow's actions selects a project (declared by
    /// its action type's node spec).
    pub fn selects_projects(&self, profile: &Profile, registry: &ActionRegistry) -> bool {
        let changes = |action: ActionInstanceId| {
            profile
                .action(action)
                .and_then(|instance| registry.definition(&instance.action_type))
                .is_some_and(|definition| definition.node.changes_context)
        };
        match &self.graph {
            Some(graph) => graph.nodes.iter().any(|node| match &node.kind {
                crate::graph::NodeKind::Action { action, .. } => changes(*action),
                _ => false,
            }),
            None => self.steps.iter().any(|step| changes(step.action)),
        }
    }

    /// Problems of the workflow's graph in `profile`, with this workflow's
    /// own (possibly unsaved) inputs and outputs, and of its triggers.
    pub fn graph_issues(
        &self,
        profile: &Profile,
        registry: &ActionRegistry,
    ) -> Vec<crate::graph::GraphIssue> {
        let mut issues = self
            .graph
            .as_ref()
            .map(|graph| graph.validate_as(self, profile, registry))
            .unwrap_or_default();
        issues.extend(self.trigger_issues(profile, registry));
        issues
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
        {
            for issue in self.graph_issues(profile, registry) {
                if issue.severity == crate::graph::IssueSeverity::Error {
                    issues.push(match issue.node {
                        Some(node) => format!("{node}: {}", issue.message),
                        None => issue.message,
                    });
                }
            }
        }
        for (index, step) in self.steps.iter().enumerate() {
            let position = index + 1;
            let Some(action) = profile.action(step.action) else {
                issues.push(format!(
                    "step {position}: the action does not exist in this profile"
                ));
                continue;
            };
            let Some(handler) = registry.handler(&action.action_type) else {
                let reason = registry
                    .unavailable_reason(&action.action_type)
                    .unwrap_or("its action type is not available");
                issues.push(format!("step {position} ({}): {reason}", action.name));
                continue;
            };
            if let Ok(parameters) = handler
                .definition()
                .normalize(&action.parameters.overlay(&step.overrides))
            {
                for problem in registry.check_parameters(&action.action_type, &parameters, profile)
                {
                    issues.push(format!("step {position} ({}): {problem}", action.name));
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{echo_type, registry, select_type, with_projects};

    #[test]
    fn workflows_without_a_scope_keep_their_earlier_behavior() {
        let registry = registry();
        let mut profile = with_projects("Work", &["SHOP", "CMS"]);
        let select = registry
            .create_instance("Next", select_type(), &ActionParameters::new())
            .unwrap();
        let echo = registry
            .create_instance(
                "Echo",
                echo_type(),
                &ActionParameters::new().with("message", crate::ParameterValue::Text("hi".into())),
            )
            .unwrap();
        let (select, echo) = (
            {
                let id = select.id;
                profile.actions.push(select);
                id
            },
            {
                let id = echo.id;
                profile.actions.push(echo);
                id
            },
        );

        // Saved before the setting: selecting projects changed the active
        // project, and still does.
        let old = Workflow::new("Old").step(select, OnFailure::Stop);
        assert_eq!(
            old.effective_project_scope(&profile, &registry),
            ProjectScope::Application
        );
        let plain = Workflow::new("Plain").step(echo, OnFailure::Stop);
        assert_eq!(
            plain.effective_project_scope(&profile, &registry),
            ProjectScope::Execution
        );
        // An explicit choice wins.
        let mut chosen = old.clone();
        chosen.project_scope = Some(ProjectScope::Execution);
        assert_eq!(
            chosen.effective_project_scope(&profile, &registry),
            ProjectScope::Execution
        );
        // Stored as camelCase; absent when unset.
        let json = serde_json::to_value(&chosen).unwrap();
        assert_eq!(json["projectScope"], "execution");
        assert!(
            serde_json::to_value(&old)
                .unwrap()
                .get("projectScope")
                .is_none()
        );
    }
}
