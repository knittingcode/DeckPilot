//! Execution engine for actions and workflows.
//!
//! [`Engine`] executes action instances and workflows against a profile. The
//! device-facing [`Runtime`](crate::device_runtime::Runtime) builds on it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::project::ProjectContextError;
use crate::{
    ActionInstance, ActionInstanceId, ActionRegistry, ActionResult, ActionStatus,
    ActiveProjectContext, ContextEffect, ExecutionContext, MachineConfig, OnFailure, Profile,
    WorkflowId,
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
        self.execute(profile, machine, context, instance)
    }

    /// Runs the steps of a workflow in order. Effects of a step are applied
    /// before the next step runs, so a step that switches project affects the
    /// steps after it.
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
        let mut report = WorkflowReport {
            workflow,
            status: ActionStatus::Succeeded,
            steps: Vec::new(),
        };
        for step in &definition.steps {
            let result = match profile.action(step.action) {
                Some(instance) => self.execute(profile, machine, context, instance)?,
                None => ActionResult::failure("action no longer exists"),
            };
            let failed = !result.is_success();
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

    fn execute(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        instance: &ActionInstance,
    ) -> Result<ActionResult, EngineError> {
        let project = context.project(profile)?;
        let Some(handler) = self.registry.handler(&instance.action_type) else {
            return Ok(ActionResult::failure(format!(
                "action type `{}` is not available",
                instance.action_type
            )));
        };
        if handler.definition().requires_project && project.is_none() {
            return Ok(ActionResult::failure(
                "this action requires an active project",
            ));
        }
        let parameters = match handler.definition().normalize(&instance.parameters) {
            Ok(parameters) => parameters,
            Err(errors) => {
                let details: Vec<_> = errors.iter().map(ToString::to_string).collect();
                return Ok(ActionResult::failure(format!(
                    "invalid configuration: {}",
                    details.join("; ")
                )));
            }
        };
        let execution = ExecutionContext {
            profile,
            project,
            machine,
        };
        let mut result = handler.execute(&parameters, &execution);
        if result.is_success() {
            for effect in &result.effects {
                let applied = match effect {
                    ContextEffect::CycleProject { direction } => {
                        context.cycle(profile, *direction).map(|_| ())
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionParameters, Os, ParameterValue, Project, ProjectKey, Workflow, builtin};

    struct Fixture {
        profile: Profile,
        log: ActionInstanceId,
        next: ActionInstanceId,
        engine: Engine,
    }

    fn fixture() -> Fixture {
        let engine = Engine::new(ActionRegistry::with_builtins());
        let registry = engine.registry();
        let mut profile = Profile::new("Work");
        for key in ["SHOP", "CMR", "CMS"] {
            profile
                .add_project(Project::new(ProjectKey::new(key).unwrap(), key))
                .unwrap();
        }
        let log = registry
            .create_instance(
                "Announce",
                builtin::log_message_id(),
                &ActionParameters::new().with(
                    "message",
                    ParameterValue::Text("now on {project.key}".into()),
                ),
            )
            .unwrap();
        let next = registry
            .create_instance(
                "Next project",
                builtin::cycle_project_id(),
                &ActionParameters::new(),
            )
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
    fn invalid_stored_configuration_fails_gracefully() {
        let mut f = fixture();
        let broken = ActionInstance {
            id: ActionInstanceId::new(),
            name: "Broken".into(),
            action_type: builtin::log_message_id(),
            parameters: ActionParameters::new(),
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
