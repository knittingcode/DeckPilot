//! Execution engine and device-event runtime.
//!
//! [`Engine`] executes action instances and workflows against a profile.
//! [`Runtime`] adds device handling on top: it tracks connected devices and
//! the page each one shows, and dispatches control input to bindings. Both are
//! hardware-independent; a device backend only has to produce
//! [`DeviceEvent`]s.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use deckpilot_device::{DeviceEvent, DeviceId, DeviceRegistry, ModelCatalog};

use crate::project::ProjectContextError;
use crate::{
    ActionInstance, ActionInstanceId, ActionRegistry, ActionResult, ActionStatus,
    ActiveProjectContext, BindingTarget, ContextEffect, DeviceLayout, ExecutionContext,
    MachineConfig, OnFailure, PageId, Profile, WorkflowId,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EngineError {
    #[error("action does not exist in this profile")]
    UnknownAction(ActionInstanceId),
    #[error("workflow does not exist in this profile")]
    UnknownWorkflow(WorkflowId),
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

/// Outcome of handling one device event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RuntimeOutcome {
    /// The event did not trigger anything.
    Ignored {
        reason: String,
    },
    DeviceConnected {
        device: DeviceId,
    },
    DeviceDisconnected {
        device: DeviceId,
    },
    Navigated {
        device: DeviceId,
        page: PageId,
    },
    Action {
        result: ActionResult,
    },
    Workflow {
        report: WorkflowReport,
    },
}

/// Hardware-independent runtime for one active profile.
#[derive(Debug)]
pub struct Runtime {
    engine: Engine,
    catalog: ModelCatalog,
    profile: Profile,
    machine: MachineConfig,
    projects: ActiveProjectContext,
    devices: DeviceRegistry,
    /// Page stack per connected device; the last entry is the visible page.
    navigation: BTreeMap<DeviceId, Vec<PageId>>,
}

impl Runtime {
    pub fn new(
        engine: Engine,
        catalog: ModelCatalog,
        profile: Profile,
        machine: MachineConfig,
    ) -> Self {
        let projects = ActiveProjectContext::new(&profile);
        Self {
            engine,
            catalog,
            profile,
            machine,
            projects,
            devices: DeviceRegistry::default(),
            navigation: BTreeMap::new(),
        }
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn project_context(&self) -> &ActiveProjectContext {
        &self.projects
    }

    pub fn devices(&self) -> &DeviceRegistry {
        &self.devices
    }

    /// The page currently shown on `device`, if it is connected and has a layout.
    pub fn current_page(&self, device: &DeviceId) -> Option<PageId> {
        self.navigation
            .get(device)
            .and_then(|stack| stack.last().copied())
    }

    fn layout_for(&self, device: &DeviceId) -> Option<&DeviceLayout> {
        self.profile
            .layouts
            .iter()
            .find(|layout| layout.model == device.model)
    }

    pub fn handle_event(&mut self, event: &DeviceEvent) -> Result<RuntimeOutcome, EngineError> {
        let ignored = |reason: &str| {
            Ok(RuntimeOutcome::Ignored {
                reason: reason.into(),
            })
        };
        match event {
            DeviceEvent::Connected { device } => {
                self.devices.apply(event);
                let home = self.layout_for(device).map(|layout| layout.home_page);
                self.navigation
                    .insert(device.clone(), home.into_iter().collect());
                Ok(RuntimeOutcome::DeviceConnected {
                    device: device.clone(),
                })
            }
            DeviceEvent::Disconnected { device } => {
                self.devices.apply(event);
                self.navigation.remove(device);
                Ok(RuntimeOutcome::DeviceDisconnected {
                    device: device.clone(),
                })
            }
            DeviceEvent::Input {
                device,
                control,
                input,
            } => {
                if !self.devices.is_connected(device) {
                    return ignored("device is not connected");
                }
                let Some(model) = self.catalog.get(&device.model) else {
                    return ignored("unknown device model");
                };
                if model.capabilities.validate_input(*control, *input).is_err() {
                    return ignored("input is not supported by the device model");
                }
                let (Some(layout), Some(page_id)) =
                    (self.layout_for(device), self.current_page(device))
                else {
                    return ignored("no layout for this device model");
                };
                let binding = layout
                    .page(page_id)
                    .and_then(|page| page.binding_for(*control, *input))
                    .map(|binding| binding.target);
                match binding {
                    None => ignored("control is not bound"),
                    Some(target) => self.dispatch(device, target),
                }
            }
        }
    }

    fn dispatch(
        &mut self,
        device: &DeviceId,
        target: BindingTarget,
    ) -> Result<RuntimeOutcome, EngineError> {
        match target {
            BindingTarget::Action { action } => {
                let result = self.engine.run_action(
                    &self.profile,
                    &self.machine,
                    &mut self.projects,
                    action,
                )?;
                Ok(RuntimeOutcome::Action { result })
            }
            BindingTarget::Workflow { workflow } => {
                let report = self.engine.run_workflow(
                    &self.profile,
                    &self.machine,
                    &mut self.projects,
                    workflow,
                )?;
                Ok(RuntimeOutcome::Workflow { report })
            }
            BindingTarget::OpenPage { page } => {
                let stack = self.navigation.entry(device.clone()).or_default();
                stack.push(page);
                Ok(RuntimeOutcome::Navigated {
                    device: device.clone(),
                    page,
                })
            }
            BindingTarget::Back => {
                let stack = self.navigation.entry(device.clone()).or_default();
                if stack.len() > 1 {
                    stack.pop();
                }
                let page = *stack.last().expect("stack holds at least the home page");
                Ok(RuntimeOutcome::Navigated {
                    device: device.clone(),
                    page,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionParameters, ControlBinding, LayoutPage, Os, ParameterValue, Project, ProjectKey,
        Trigger, Workflow, builtin,
    };
    use deckpilot_device::{ControlId, ControlInput, DeviceModelId, models};

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

    #[test]
    fn runtime_dispatches_device_input_and_navigates_folders() {
        let mut f = fixture();
        let model = DeviceModelId::new(models::STREAM_DECK_MK2_ID);
        let mut layout = DeviceLayout::new("Main", model.clone());
        let folder = LayoutPage::new("Folder");
        let folder_id = folder.id;
        layout.pages.push(folder);
        let home = layout.home_page;
        let bind = |index, target| ControlBinding {
            control: ControlId::Key { index },
            trigger: Trigger::Press,
            target,
            label: None,
        };
        layout.page_mut(home).unwrap().bindings.extend([
            bind(0, BindingTarget::Action { action: f.next }),
            bind(1, BindingTarget::OpenPage { page: folder_id }),
        ]);
        layout
            .page_mut(folder_id)
            .unwrap()
            .bindings
            .push(bind(1, BindingTarget::Back));
        f.profile.layouts.push(layout);

        let mut runtime = Runtime::new(f.engine, ModelCatalog::builtin(), f.profile, machine());
        let device = DeviceId {
            model,
            serial: "TEST".into(),
        };
        let press = |index| DeviceEvent::Input {
            device: device.clone(),
            control: ControlId::Key { index },
            input: ControlInput::Press,
        };

        assert!(matches!(
            runtime.handle_event(&press(0)).unwrap(),
            RuntimeOutcome::Ignored { .. }
        ));
        runtime
            .handle_event(&DeviceEvent::Connected {
                device: device.clone(),
            })
            .unwrap();
        assert_eq!(runtime.current_page(&device), Some(home));

        let outcome = runtime.handle_event(&press(0)).unwrap();
        assert!(matches!(outcome, RuntimeOutcome::Action { ref result } if result.is_success()));
        let active = runtime
            .project_context()
            .project(runtime.profile())
            .unwrap()
            .unwrap();
        assert_eq!(active.key.as_str(), "CMR");

        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(runtime.current_page(&device), Some(folder_id));
        runtime.handle_event(&press(1)).unwrap();
        assert_eq!(runtime.current_page(&device), Some(home));

        let unbound = runtime.handle_event(&press(9)).unwrap();
        assert!(matches!(unbound, RuntimeOutcome::Ignored { .. }));

        runtime
            .handle_event(&DeviceEvent::Disconnected {
                device: device.clone(),
            })
            .unwrap();
        assert_eq!(runtime.current_page(&device), None);
    }
}
