//! Execution engine for actions and workflows.
//!
//! [`Engine`] executes action instances and workflows against a profile. The
//! device-facing [`Runtime`](crate::device_runtime::Runtime) builds on it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::executor::{Executor, RunStatus};
use crate::graph::{
    CancellationToken, ExecutionEvent, ExecutionLimits, GraphReport, IssueSeverity, MAX_NESTING,
    WorkflowGraph,
};
use crate::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionRegistry, ActionResult, ActionStatus,
    ActiveProjectContext, ContextEffect, ExecutionContext, MachineConfig, OnFailure, Profile,
    ProjectContextError, StepCondition, WorkflowId,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EngineError {
    #[error("action does not exist in this profile")]
    UnknownAction(ActionInstanceId),
    #[error("workflow does not exist in this profile")]
    UnknownWorkflow(WorkflowId),
    #[error("project does not exist in this profile")]
    UnknownProject(crate::ProjectId),
    #[error("no profile is active")]
    NoActiveProfile,
    #[error(transparent)]
    Context(#[from] ProjectContextError),
    #[error("the workflow cannot run: {0}")]
    InvalidWorkflow(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepReport {
    pub action: ActionInstanceId,
    pub result: ActionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowReport {
    pub workflow: WorkflowId,
    pub status: ActionStatus,
    pub steps: Vec<StepReport>,
}

/// Executes actions and workflows.
#[derive(Debug, Default)]
pub struct Engine {
    registry: ActionRegistry,
}

impl Engine {
    pub fn new(registry: ActionRegistry) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &ActionRegistry {
        &self.registry
    }

    /// Runs one action instance of `profile` and applies its context effects.
    pub fn run_action(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        action: ActionInstanceId,
    ) -> Result<ActionResult, EngineError> {
        let instance = profile
            .action(action)
            .ok_or(EngineError::UnknownAction(action))?;
        self.execute(profile, machine, context, instance, None)
    }

    /// Runs the steps of a workflow in order. Effects of a step are applied
    /// before the next step runs, so a step that switches project affects the
    /// steps after it. Steps whose condition is not met are reported as
    /// skipped and do not change the outcome of "previous step" conditions.
    pub fn run_workflow(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        workflow: WorkflowId,
    ) -> Result<WorkflowReport, EngineError> {
        let definition = profile
            .workflow(workflow)
            .ok_or(EngineError::UnknownWorkflow(workflow))?;
        if definition.graph.is_some() {
            let report = self.run_graph(
                profile,
                machine,
                context,
                workflow,
                &CancellationToken::new(),
                &|_| {},
            )?;
            return Ok(report.into_steps());
        }
        let mut report = WorkflowReport {
            workflow,
            status: ActionStatus::Succeeded,
            steps: Vec::new(),
        };
        let mut previous: Option<bool> = None;
        for step in &definition.steps {
            let run = match step.condition {
                StepCondition::Always => true,
                StepCondition::PreviousSucceeded => previous == Some(true),
                StepCondition::PreviousFailed => previous == Some(false),
                StepCondition::ActiveProject { project } => context.project_id() == Some(project),
            };
            if !run {
                let result = ActionResult::skipped("condition not met");
                report.steps.push(StepReport {
                    action: step.action,
                    result,
                });
                continue;
            }
            let result = match profile.action(step.action) {
                Some(instance) => {
                    let overrides = (!step.overrides.is_empty()).then_some(&step.overrides);
                    self.execute(profile, machine, context, instance, overrides)?
                }
                None => ActionResult::failure("action no longer exists"),
            };
            let failed = !result.is_success();
            previous = Some(!failed);
            report.steps.push(StepReport {
                action: step.action,
                result,
            });
            if failed {
                report.status = ActionStatus::Failed;
                if step.on_failure == OnFailure::Stop {
                    break;
                }
            }
        }
        Ok(report)
    }

    /// Runs the graph of `workflow`. The graph is validated first; a graph
    /// with errors does not run. See [`crate::graph`] for the semantics.
    pub fn run_graph(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        workflow: WorkflowId,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(ExecutionEvent) + Sync),
    ) -> Result<GraphReport, EngineError> {
        self.run_graph_nested(profile, machine, context, workflow, cancel, on_event, 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_graph_nested(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        workflow: WorkflowId,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(ExecutionEvent) + Sync),
        depth: usize,
    ) -> Result<GraphReport, EngineError> {
        let definition = profile
            .workflow(workflow)
            .ok_or(EngineError::UnknownWorkflow(workflow))?;
        let graph = definition
            .graph
            .as_ref()
            .ok_or_else(|| EngineError::InvalidWorkflow("the workflow has no graph".into()))?;
        let issues = graph.validate(workflow, profile, &self.registry);
        if !WorkflowGraph::is_runnable(&issues) {
            let first = issues
                .iter()
                .find(|i| i.severity == IssueSeverity::Error)
                .map(|i| i.message.clone())
                .unwrap_or_default();
            return Err(EngineError::InvalidWorkflow(first));
        }
        if depth > MAX_NESTING {
            return Err(EngineError::InvalidWorkflow(
                "workflows are nested too deeply".into(),
            ));
        }
        let run_action = |instance: &ActionInstance,
                          overrides: &ActionParameters,
                          context: &mut ActiveProjectContext| {
            let overrides = (!overrides.is_empty()).then_some(overrides);
            self.execute(profile, machine, context, instance, overrides)
        };
        // Nested workflows report their progress as part of the Loop node.
        let run_nested = |nested: WorkflowId, context: &mut ActiveProjectContext| {
            let inner = profile
                .workflow(nested)
                .ok_or(EngineError::UnknownWorkflow(nested))?;
            if inner.graph.is_some() {
                let report = self.run_graph_nested(
                    profile,
                    machine,
                    context,
                    nested,
                    cancel,
                    &|_| {},
                    depth + 1,
                )?;
                Ok(report.status == RunStatus::Succeeded)
            } else {
                let report = self.run_workflow(profile, machine, context, nested)?;
                Ok(report.status == ActionStatus::Succeeded)
            }
        };
        Executor {
            workflow,
            graph,
            profile,
            registry: &self.registry,
            run_action: &run_action,
            run_nested: &run_nested,
            on_event,
            cancel,
            limits: ExecutionLimits::default(),
        }
        .run(context)
    }

    pub(crate) fn execute(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        instance: &ActionInstance,
        overrides: Option<&ActionParameters>,
    ) -> Result<ActionResult, EngineError> {
        let project = context.project(profile)?;
        let Some(handler) = self.registry.handler(&instance.action_type) else {
            return Ok(self.registry.dispatch(
                &instance.action_type,
                &instance.parameters,
                &ExecutionContext::new(profile, project, machine),
            ));
        };
        if handler.definition().requires_project && project.is_none() {
            return Ok(ActionResult::failure(
                "this action requires an active project",
            ));
        }
        let configured = match overrides {
            Some(overrides) => instance.parameters.overlay(overrides),
            None => instance.parameters.clone(),
        };
        let parameters = match handler.definition().normalize(&configured) {
            Ok(parameters) => parameters,
            Err(errors) => {
                let details: Vec<_> = errors.iter().map(ToString::to_string).collect();
                return Ok(ActionResult::failure(format!(
                    "invalid configuration: {}",
                    details.join("; ")
                )));
            }
        };
        let problems = self
            .registry
            .check_parameters(&instance.action_type, &parameters, profile);
        if !problems.is_empty() {
            return Ok(ActionResult::failure(format!(
                "invalid configuration: {}",
                problems.join("; ")
            )));
        }
        let execution = ExecutionContext::new(profile, project, machine);
        let mut result = self
            .registry
            .dispatch(&instance.action_type, &parameters, &execution);
        if result.is_success() {
            for effect in &result.effects {
                // Selection is validated against the active profile, so an
                // action can never activate another profile's project.
                let applied = match effect {
                    ContextEffect::SelectProject { project } => context.select(profile, *project),
                };
                if let Err(error) = applied {
                    result = ActionResult::failure(error.to_string());
                    break;
                }
            }
        }
        Ok(result)
    }
}

impl GraphReport {
    /// The action nodes as a step report, for callers of the sequential
    /// interface (device bindings, the agent's last-run summary).
    pub fn into_steps(self) -> WorkflowReport {
        let status = match self.status {
            RunStatus::Succeeded => ActionStatus::Succeeded,
            RunStatus::Failed | RunStatus::Cancelled => ActionStatus::Failed,
        };
        let steps = self
            .nodes
            .into_iter()
            .filter_map(|node| {
                let action = node.action?;
                let result = match node.status {
                    crate::graph::NodeStatus::Succeeded => {
                        let mut result = ActionResult::success(node.message);
                        result.data = node.output;
                        result
                    }
                    crate::graph::NodeStatus::Failed => {
                        ActionResult::failure(node.error.map(|e| e.message).unwrap_or(node.message))
                    }
                    crate::graph::NodeStatus::Skipped | crate::graph::NodeStatus::Cancelled => {
                        ActionResult::skipped("not run")
                    }
                };
                Some(StepReport { action, result })
            })
            .collect();
        WorkflowReport {
            workflow: self.workflow,
            status,
            steps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{echo_type, registry, select_type, with_projects};
    use crate::{ActionParameters, Os, ParameterValue, Workflow};

    struct Fixture {
        profile: Profile,
        log: ActionInstanceId,
        next: ActionInstanceId,
        engine: Engine,
    }

    fn fixture() -> Fixture {
        let engine = Engine::new(registry());
        let registry = engine.registry();
        let mut profile = with_projects("Work", &["SHOP", "CMR", "CMS"]);
        let log = registry
            .create_instance(
                "Announce",
                echo_type(),
                &ActionParameters::new().with(
                    "message",
                    ParameterValue::Text("now on {project.key}".into()),
                ),
            )
            .unwrap();
        let next = registry
            .create_instance("Next project", select_type(), &ActionParameters::new())
            .unwrap();
        let (log_id, next_id) = (log.id, next.id);
        profile.actions.extend([log, next]);
        Fixture {
            profile,
            log: log_id,
            next: next_id,
            engine,
        }
    }

    fn machine() -> MachineConfig {
        MachineConfig::new(Os::current())
    }

    #[test]
    fn workflow_steps_observe_earlier_effects() {
        let mut f = fixture();
        let workflow = Workflow::new("Switch and announce")
            .step(f.next, OnFailure::Stop)
            .step(f.log, OnFailure::Stop);
        let workflow_id = workflow.id;
        f.profile.workflows.push(workflow);

        let mut context = ActiveProjectContext::new(&f.profile);
        let report = f
            .engine
            .run_workflow(&f.profile, &machine(), &mut context, workflow_id)
            .unwrap();
        assert_eq!(report.status, ActionStatus::Succeeded);
        assert_eq!(report.steps[1].result.message, "now on CMR");
    }

    #[test]
    fn workflow_failure_policy() {
        let mut f = fixture();
        let missing = ActionInstanceId::new();
        let stop = Workflow::new("Stop")
            .step(missing, OnFailure::Stop)
            .step(f.log, OnFailure::Stop);
        let cont = Workflow::new("Continue")
            .step(missing, OnFailure::Continue)
            .step(f.log, OnFailure::Stop);
        let (stop_id, cont_id) = (stop.id, cont.id);
        f.profile.workflows.extend([stop, cont]);
        let mut context = ActiveProjectContext::new(&f.profile);

        let report = f
            .engine
            .run_workflow(&f.profile, &machine(), &mut context, stop_id)
            .unwrap();
        assert_eq!(
            (report.status, report.steps.len()),
            (ActionStatus::Failed, 1)
        );

        let report = f
            .engine
            .run_workflow(&f.profile, &machine(), &mut context, cont_id)
            .unwrap();
        assert_eq!(
            (report.status, report.steps.len()),
            (ActionStatus::Failed, 2)
        );
        assert!(report.steps[1].result.is_success());
    }

    #[test]
    fn step_conditions_and_overrides() {
        let mut f = fixture();
        let cms = f.profile.projects[2].id;
        let mut workflow = Workflow::new("Conditional")
            .step(f.log, OnFailure::Continue)
            .step(f.log, OnFailure::Continue)
            .step(f.log, OnFailure::Continue)
            .step(f.log, OnFailure::Continue);
        workflow.steps[1].condition = StepCondition::PreviousFailed;
        workflow.steps[2].condition = StepCondition::ActiveProject { project: cms };
        workflow.steps[3].condition = StepCondition::PreviousSucceeded;
        workflow.steps[3].overrides = ActionParameters::new().with(
            "message",
            ParameterValue::Text("override {project.key}".into()),
        );
        let id = workflow.id;
        f.profile.workflows.push(workflow);
        let mut context = ActiveProjectContext::new(&f.profile);

        let report = f
            .engine
            .run_workflow(&f.profile, &machine(), &mut context, id)
            .unwrap();
        let statuses: Vec<_> = report.steps.iter().map(|s| s.result.status).collect();
        assert_eq!(
            statuses,
            [
                ActionStatus::Succeeded,
                ActionStatus::Skipped,
                ActionStatus::Skipped,
                ActionStatus::Succeeded
            ]
        );
        assert_eq!(report.steps[3].result.message, "override SHOP");
        assert_eq!(report.status, ActionStatus::Succeeded);
        // The action's own configuration is unchanged.
        let log = f.profile.action(f.log).unwrap();
        assert_eq!(log.parameters.text("message"), Some("now on {project.key}"));
    }

    #[test]
    fn workflow_validation_reports_problems() {
        let mut f = fixture();
        let registry = registry();
        let mut workflow = Workflow::new(" ").step(f.log, OnFailure::Stop);
        workflow.steps[0].condition = StepCondition::PreviousSucceeded;
        workflow.steps[0].overrides =
            ActionParameters::new().with("message", ParameterValue::Integer(3));
        workflow = workflow.step(ActionInstanceId::new(), OnFailure::Stop);
        let issues = workflow.validate(&f.profile, &registry);
        assert_eq!(issues.len(), 4, "{issues:?}");

        let valid = Workflow::new("Start Work")
            .step(f.next, OnFailure::Stop)
            .step(f.log, OnFailure::Stop);
        assert_eq!(valid.validate(&f.profile, &registry), Vec::<String>::new());
        f.profile.workflows.push(valid);
    }

    #[test]
    fn invalid_stored_configuration_fails_gracefully() {
        let mut f = fixture();
        let broken = ActionInstance {
            id: ActionInstanceId::new(),
            name: "Broken".into(),
            action_type: echo_type(),
            parameters: ActionParameters::new(),
            description: String::new(),
            icon: None,
            label: None,
        };
        let id = broken.id;
        f.profile.actions.push(broken);
        let mut context = ActiveProjectContext::new(&f.profile);
        let result = f
            .engine
            .run_action(&f.profile, &machine(), &mut context, id)
            .unwrap();
        assert!(!result.is_success());
        assert!(result.message.contains("invalid configuration"));
    }

    #[test]
    fn rejects_foreign_project_context() {
        let f = fixture();
        let other = Profile::new("Other");
        let mut context = ActiveProjectContext::new(&other);
        assert!(matches!(
            f.engine
                .run_action(&f.profile, &machine(), &mut context, f.log),
            Err(EngineError::Context(_))
        ));
    }
}
