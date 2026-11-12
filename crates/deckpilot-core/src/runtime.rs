//! Execution engine for actions and workflows.
//!
//! [`Engine`] executes action instances and workflows against a profile. The
//! device-facing [`Runtime`](crate::device_runtime::Runtime) builds on it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::executor::{Executor, NestedRun, RunStatus};
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
#[derive(Debug)]
pub struct Engine {
    registry: ActionRegistry,
    credentials: std::sync::Arc<dyn crate::CredentialSource>,
    notifier: std::sync::Arc<dyn crate::notify::Notifier>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(ActionRegistry::default())
    }
}

impl Engine {
    pub fn new(registry: ActionRegistry) -> Self {
        Self {
            registry,
            credentials: std::sync::Arc::new(crate::NoCredentials),
            notifier: std::sync::Arc::new(crate::notify::NoNotifier),
        }
    }

    /// The engine with a way to show notifications.
    #[must_use]
    pub fn with_notifier(self, notifier: std::sync::Arc<dyn crate::notify::Notifier>) -> Self {
        Self { notifier, ..self }
    }

    /// The engine with the credentials stored on this computer.
    #[must_use]
    pub fn with_credentials(
        self,
        credentials: std::sync::Arc<dyn crate::CredentialSource>,
    ) -> Self {
        Self {
            credentials,
            ..self
        }
    }

    pub fn registry(&self) -> &ActionRegistry {
        &self.registry
    }

    /// The refresh policy of the live state of `action` in `profile`, if
    /// its module provides one.
    pub fn state_policy(
        &self,
        profile: &Profile,
        action: ActionInstanceId,
    ) -> Option<crate::RefreshPolicy> {
        let instance = profile.action(action)?;
        let handler = self.registry.handler(&instance.action_type)?;
        let parameters = handler.definition().normalize(&instance.parameters).ok();
        self.registry.state_policy(
            &instance.action_type,
            parameters.as_ref().unwrap_or(&instance.parameters),
        )
    }

    /// The live state of a key that runs `action` of `profile` with
    /// `project` as its project. Runs the module's provider on the calling
    /// thread, with this engine's credentials, no notifications and
    /// `cancel` as the cancellation token. `None` if the action has no
    /// live state.
    pub fn button_state(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        project: Option<&crate::Project>,
        action: ActionInstanceId,
        cancel: &CancellationToken,
    ) -> Option<Result<crate::ButtonState, crate::StateError>> {
        let instance = profile.action(action)?;
        let handler = self.registry.handler(&instance.action_type)?;
        let Ok(parameters) = handler.definition().normalize(&instance.parameters) else {
            return Some(Err(crate::StateError::new(
                "invalid-configuration",
                "the action's configuration is invalid",
            )));
        };
        if handler.definition().requires_project && project.is_none() {
            return Some(Err(crate::StateError::new(
                "no-active-project",
                "this action requires an active project",
            )));
        }
        let ignore = |_: crate::Progress| {};
        let control = crate::ExecutionControl::new(cancel, &ignore);
        let context = ExecutionContext::new(profile, project, machine)
            .with_credentials(self.credentials.as_ref())
            .with_control(&control);
        self.registry
            .button_state(&instance.action_type, &parameters, &context)
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
        self.execute(profile, machine, context, instance, None, None)
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
                    self.execute(profile, machine, context, instance, overrides, None)?
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

    /// Checks that `workflow` exists and can run on its own, without
    /// running it: a graph must have no errors, and every required input
    /// needs a default. Lets a caller reject a run before starting it in
    /// the background.
    pub fn check_runnable(
        &self,
        profile: &Profile,
        workflow: WorkflowId,
    ) -> Result<(), EngineError> {
        self.check_runnable_with(profile, workflow, &std::collections::BTreeMap::new())
    }

    /// [`Engine::check_runnable`] for a run with these input values.
    pub fn check_runnable_with(
        &self,
        profile: &Profile,
        workflow: WorkflowId,
        inputs: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), EngineError> {
        let definition = profile
            .workflow(workflow)
            .ok_or(EngineError::UnknownWorkflow(workflow))?;
        if definition.graph.is_some() {
            workflow_inputs(definition, inputs)?;
        }
        self.check_graph(profile, workflow)
    }

    /// The graph of `workflow`, if it has one, has no errors.
    fn check_graph(&self, profile: &Profile, workflow: WorkflowId) -> Result<(), EngineError> {
        let definition = profile
            .workflow(workflow)
            .ok_or(EngineError::UnknownWorkflow(workflow))?;
        if let Some(graph) = &definition.graph {
            let issues = graph.validate(workflow, profile, &self.registry);
            if !WorkflowGraph::is_runnable(&issues) {
                let first = issues
                    .iter()
                    .find(|i| i.severity == IssueSeverity::Error)
                    .map(|i| i.message.clone())
                    .unwrap_or_default();
                return Err(EngineError::InvalidWorkflow(first));
            }
        }
        Ok(())
    }

    /// Runs `workflow` with progress events and cancellation: its graph if it
    /// has one, otherwise its steps. Steps are reported as nodes named
    /// `step_<n>`, the names [`crate::graph::from_steps`] gives them, so a
    /// run can be shown on the converted graph.
    pub fn execute_workflow(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        workflow: WorkflowId,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(ExecutionEvent) + Sync),
    ) -> Result<GraphReport, EngineError> {
        self.execute_workflow_with(
            profile,
            machine,
            context,
            workflow,
            &std::collections::BTreeMap::new(),
            cancel,
            on_event,
        )
    }

    /// [`Engine::execute_workflow`] with values for the workflow's inputs,
    /// e.g. from a trigger. Inputs without a value get their default; a
    /// required one without a value is an error. Workflows without a graph
    /// have no inputs.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_workflow_with(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        workflow: WorkflowId,
        inputs: &std::collections::BTreeMap<String, String>,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(ExecutionEvent) + Sync),
    ) -> Result<GraphReport, EngineError> {
        let definition = profile
            .workflow(workflow)
            .ok_or(EngineError::UnknownWorkflow(workflow))?;
        if definition.graph.is_some() {
            let inputs = workflow_inputs(definition, inputs)?;
            self.run_graph_nested(
                profile, machine, context, workflow, &inputs, cancel, on_event, 0,
            )
        } else {
            self.run_steps(profile, machine, context, workflow, cancel, on_event)
        }
    }

    /// The sequential engine of [`Engine::run_workflow`] with events and
    /// cancellation between steps.
    #[allow(clippy::too_many_lines)]
    fn run_steps(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        workflow: WorkflowId,
        cancel: &CancellationToken,
        on_event: &(dyn Fn(ExecutionEvent) + Sync),
    ) -> Result<GraphReport, EngineError> {
        use crate::graph::{NodeId, NodeReport, NodeStatus, StepError};
        let definition = profile
            .workflow(workflow)
            .ok_or(EngineError::UnknownWorkflow(workflow))?;
        let start = std::time::Instant::now();
        let elapsed = |since: std::time::Instant| {
            u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
        };
        let mut nodes = Vec::with_capacity(definition.steps.len());
        let mut failed = false;
        let mut stopped = false;
        let mut previous: Option<bool> = None;
        for (index, step) in definition.steps.iter().enumerate() {
            let node = NodeId::new(format!("step_{}", index + 1)).expect("step ids are valid");
            let mut report = NodeReport {
                node: node.clone(),
                status: NodeStatus::Skipped,
                attempts: 0,
                started_ms: elapsed(start),
                duration_ms: 0,
                message: String::new(),
                output: std::collections::BTreeMap::new(),
                error: None,
                action: Some(step.action),
                workflow: None,
                sets_variables: false,
            };
            let run = match step.condition {
                StepCondition::Always => true,
                StepCondition::PreviousSucceeded => previous == Some(true),
                StepCondition::PreviousFailed => previous == Some(false),
                StepCondition::ActiveProject { project } => context.project_id() == Some(project),
            };
            if cancel.is_cancelled() {
                report.status = NodeStatus::Cancelled;
            } else if stopped {
                "not run".clone_into(&mut report.message);
            } else if !run {
                "condition not met".clone_into(&mut report.message);
            } else {
                on_event(ExecutionEvent::NodeStarted { node: node.clone() });
                let begun = std::time::Instant::now();
                let token = cancel.child();
                let progress = |progress: crate::Progress| {
                    on_event(ExecutionEvent::NodeProgress {
                        node: node.clone(),
                        progress,
                    });
                };
                let control = crate::ExecutionControl::new(&token, &progress);
                let result = match profile.action(step.action) {
                    Some(instance) => {
                        let overrides = (!step.overrides.is_empty()).then_some(&step.overrides);
                        self.execute(
                            profile,
                            machine,
                            context,
                            instance,
                            overrides,
                            Some(&control),
                        )?
                    }
                    None => ActionResult::failure("action no longer exists"),
                };
                report.attempts = 1;
                report.duration_ms = elapsed(begun);
                let succeeded = result.is_success();
                previous = Some(succeeded);
                if succeeded {
                    report.status = NodeStatus::Succeeded;
                    report.output = result.data;
                } else {
                    report.status = NodeStatus::Failed;
                    report.error = Some(StepError {
                        code: "action.failed".into(),
                        message: result.message.clone(),
                        retryable: false,
                        action_code: result.code.clone(),
                    });
                    failed = true;
                    stopped = step.on_failure == OnFailure::Stop;
                }
                report.message = result.message;
            }
            on_event(ExecutionEvent::NodeFinished {
                report: report.clone(),
            });
            nodes.push(report);
        }
        let status = if nodes.iter().any(|n| n.status == NodeStatus::Cancelled) {
            RunStatus::Cancelled
        } else if failed {
            RunStatus::Failed
        } else {
            RunStatus::Succeeded
        };
        Ok(GraphReport {
            workflow,
            status,
            duration_ms: elapsed(start),
            nodes,
            context_conflict: None,
        })
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
        let definition = profile
            .workflow(workflow)
            .ok_or(EngineError::UnknownWorkflow(workflow))?;
        let inputs = workflow_inputs(definition, &std::collections::BTreeMap::new())?;
        self.run_graph_nested(
            profile, machine, context, workflow, &inputs, cancel, on_event, 0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_graph_nested(
        &self,
        profile: &Profile,
        machine: &MachineConfig,
        context: &mut ActiveProjectContext,
        workflow: WorkflowId,
        inputs: &std::collections::BTreeMap<String, String>,
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
        self.check_graph(profile, workflow)?;
        if depth > MAX_NESTING {
            return Err(EngineError::InvalidWorkflow(
                "workflows are nested too deeply".into(),
            ));
        }
        let run_action = |instance: &ActionInstance,
                          overrides: &ActionParameters,
                          context: &mut ActiveProjectContext,
                          control: &crate::ExecutionControl<'_>| {
            let overrides = (!overrides.is_empty()).then_some(overrides);
            self.execute(
                profile,
                machine,
                context,
                instance,
                overrides,
                Some(control),
            )
        };
        // Nested workflows report their progress as part of the Loop or
        // Call node; their own nodes are not reported.
        let run_nested = |nested: WorkflowId,
                          values: &std::collections::BTreeMap<String, String>,
                          context: &mut ActiveProjectContext,
                          token: &CancellationToken| {
            let inner = profile
                .workflow(nested)
                .ok_or(EngineError::UnknownWorkflow(nested))?;
            let report = if let Some(inner_graph) = &inner.graph {
                let values = workflow_inputs(inner, values)?;
                let report = self.run_graph_nested(
                    profile,
                    machine,
                    context,
                    nested,
                    &values,
                    token,
                    &|_| {},
                    depth + 1,
                )?;
                let outputs = if report.status == RunStatus::Succeeded {
                    workflow_outputs(inner_graph, &report)
                } else {
                    std::collections::BTreeMap::new()
                };
                (report, outputs)
            } else {
                let report = self.run_steps(profile, machine, context, nested, token, &|_| {})?;
                (report, std::collections::BTreeMap::new())
            };
            let (report, outputs) = report;
            let failure = report
                .nodes
                .iter()
                .find(|n| n.status == crate::graph::NodeStatus::Failed)
                .map(|n| {
                    let message = n.error.as_ref().map_or(n.message.as_str(), |e| &e.message);
                    format!("{}: {}", n.node, crate::history::shorten(message))
                });
            Ok(NestedRun {
                status: report.status,
                outputs,
                failure,
            })
        };
        Executor {
            workflow,
            graph,
            profile,
            registry: &self.registry,
            run_action: &run_action,
            run_nested: &run_nested,
            inputs,
            on_event,
            cancel,
            limits: ExecutionLimits::default(),
            summary: std::sync::Mutex::new(crate::RunSummary {
                workflow: Some(workflow),
                ..crate::RunSummary::default()
            }),
            started: std::time::Instant::now(),
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
        control: Option<&crate::ExecutionControl<'_>>,
    ) -> Result<ActionResult, EngineError> {
        let project = context.project(profile)?;
        let base = ExecutionContext::new(profile, project, machine)
            .with_credentials(self.credentials.as_ref())
            .with_notifier(self.notifier.as_ref());
        let base = match control {
            Some(control) => base.with_control(control),
            None => base,
        };
        let Some(handler) = self.registry.handler(&instance.action_type) else {
            return Ok(self
                .registry
                .dispatch(&instance.action_type, &instance.parameters, &base));
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
        let execution = base;
        let mut result = self
            .registry
            .dispatch(&instance.action_type, &parameters, &execution);
        if result.is_success() {
            let requested = std::mem::take(&mut result.effects);
            for effect in &requested {
                match apply_effect(profile, context, effect, &mut result) {
                    Ok(applied) => result.effects.push(applied),
                    Err(failure) => {
                        result = failure;
                        break;
                    }
                }
            }
        }
        Ok(result)
    }
}

/// Applies `effect` to `context` and returns it as applied. Selection is
/// validated against the active profile, so an action can never activate
/// another profile's project.
///
/// A cycle steps from the project active *now*: effects are applied one
/// at a time by whoever owns `context`, so several cycles requested in a
/// row each move one project on. It is reported as the selection it made,
/// and `result`'s project data is set to that project.
fn apply_effect(
    profile: &Profile,
    context: &mut ActiveProjectContext,
    effect: &ContextEffect,
    result: &mut ActionResult,
) -> Result<ContextEffect, ActionResult> {
    match *effect {
        ContextEffect::SelectProject { project } => context
            .select(profile, project)
            .map(|()| ContextEffect::SelectProject { project })
            .map_err(|error| ActionResult::failure(error.to_string())),
        ContextEffect::CycleProject {
            direction,
            wrap_around,
        } => {
            let current = context
                .project(profile)
                .map_err(|error| ActionResult::failure(error.to_string()))?
                .map(|p| p.id);
            let target =
                crate::ProjectSwitcher::step(&profile.projects, current, direction, wrap_around)
                    .map_err(|error| ActionResult::error(error.code(), error.to_string()))?;
            context
                .select(profile, target)
                .map_err(|error| ActionResult::failure(error.to_string()))?;
            if let Some(project) = profile.project(target) {
                let id = project.id.to_string();
                if result.data.get("projectId").is_some_and(|seen| *seen != id) {
                    // The handler saw another project than the one active now.
                    result.message = format!("switched to {}", project.key);
                }
                result.data.insert("projectId".into(), id);
                result
                    .data
                    .insert("projectKey".into(), project.key.to_string());
                result
                    .data
                    .insert("projectName".into(), project.name.clone());
            }
            Ok(ContextEffect::SelectProject { project: target })
        }
    }
}

/// The values of `workflow`'s declared inputs for a run: `given` values for
/// declared inputs, otherwise their defaults. A required input without a
/// value is an error; values are checked by the caller.
fn workflow_inputs(
    workflow: &crate::Workflow,
    given: &std::collections::BTreeMap<String, String>,
) -> Result<std::collections::BTreeMap<String, String>, EngineError> {
    let mut values = std::collections::BTreeMap::new();
    for input in &workflow.inputs {
        match (given.get(&input.key), &input.default) {
            (Some(value), _) => {
                values.insert(input.key.clone(), value.clone());
            }
            (None, Some(default)) if !input.required => {
                values.insert(input.key.clone(), default.clone());
            }
            (None, _) if input.required => {
                return Err(EngineError::InvalidWorkflow(format!(
                    "{} needs a value for its input `{}`; run it from a Sub-workflow node",
                    workflow.name, input.key
                )));
            }
            (None, _) => {}
        }
    }
    Ok(values)
}

/// The declared outputs a finished run set: the values of its Output nodes
/// that succeeded, in the order they finished (a later one overrides).
fn workflow_outputs(
    graph: &WorkflowGraph,
    report: &GraphReport,
) -> std::collections::BTreeMap<String, String> {
    let mut outputs = std::collections::BTreeMap::new();
    for node in &report.nodes {
        let is_output = graph
            .node(&node.node)
            .is_some_and(|n| matches!(n.kind, crate::graph::NodeKind::Output { .. }));
        if is_output && node.status == crate::graph::NodeStatus::Succeeded {
            outputs.extend(node.output.clone());
        }
    }
    outputs
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
                    crate::graph::NodeStatus::Skipped if !node.message.is_empty() => {
                        ActionResult::skipped(node.message)
                    }
                    crate::graph::NodeStatus::Skipped => ActionResult::skipped("not run"),
                    crate::graph::NodeStatus::Cancelled => ActionResult::skipped("cancelled"),
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
    fn a_cycle_steps_from_the_project_active_when_it_is_applied() {
        let profile = with_projects("Work", &["SHOP", "CMR", "CMS"]);
        let (cmr, cms) = (profile.projects[1].id, profile.projects[2].id);
        let cycle = |direction, wrap_around| ContextEffect::CycleProject {
            direction,
            wrap_around,
        };
        let next = cycle(crate::CycleDirection::Next, true);
        // The handler saw SHOP and expected CMR, but CMR became active
        // before the effect was applied: the cycle moves on from CMR.
        let mut context = ActiveProjectContext::new(&profile);
        context.select(&profile, cmr).unwrap();
        let mut result = ActionResult::success("switched to CMR")
            .with_data("projectId", cmr.to_string())
            .with_data("projectKey", "CMR");
        let applied = apply_effect(&profile, &mut context, &next, &mut result).unwrap();
        assert_eq!(applied, ContextEffect::SelectProject { project: cms });
        assert_eq!(context.project_id(), Some(cms));
        assert_eq!(result.data["projectKey"], "CMS");
        assert_eq!(result.data["projectId"], cms.to_string());
        assert_eq!(result.message, "switched to CMS");

        // Without wrap-around the end fails with its code and changes nothing.
        let failure = apply_effect(
            &profile,
            &mut context,
            &cycle(crate::CycleDirection::Next, false),
            &mut ActionResult::success(""),
        )
        .unwrap_err();
        assert_eq!(failure.code.as_deref(), Some("at-last-project"));
        assert_eq!(context.project_id(), Some(cms));

        // A context of another profile (the profile changed between the
        // press and the effect) is never changed.
        let home = with_projects("Home", &["BLOG"]);
        assert!(apply_effect(&home, &mut context, &next, &mut ActionResult::success("")).is_err());
        assert_eq!(context.project_id(), Some(cms));

        let empty = with_projects("Empty", &[]);
        let mut none = ActiveProjectContext::new(&empty);
        let failure =
            apply_effect(&empty, &mut none, &next, &mut ActionResult::success("")).unwrap_err();
        assert_eq!(failure.code.as_deref(), Some("no-projects"));
    }

    #[test]
    fn button_states_of_action_instances() {
        let f = fixture();
        let machine = machine();
        let cms = &f.profile.projects[2];
        let cancel = CancellationToken::new();
        assert!(f.engine.state_policy(&f.profile, f.log).is_some());
        assert!(f.engine.state_policy(&f.profile, f.next).is_none());
        let state = f
            .engine
            .button_state(&f.profile, &machine, Some(cms), f.log, &cancel)
            .unwrap()
            .unwrap();
        assert_eq!(state.label.as_deref(), Some("now on CMS"));
        assert!(
            f.engine
                .button_state(&f.profile, &machine, None, f.next, &cancel)
                .is_none()
        );
        assert!(
            f.engine
                .button_state(&f.profile, &machine, None, ActionInstanceId::new(), &cancel)
                .is_none()
        );
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
    fn observed_step_runs_report_nodes_and_events() {
        let mut f = fixture();
        let workflow = Workflow::new("Switch and announce")
            .step(f.next, OnFailure::Stop)
            .step(f.log, OnFailure::Stop);
        let workflow_id = workflow.id;
        f.profile.workflows.push(workflow);

        let events = std::sync::Mutex::new(Vec::new());
        let mut context = ActiveProjectContext::new(&f.profile);
        let report = f
            .engine
            .execute_workflow(
                &f.profile,
                &machine(),
                &mut context,
                workflow_id,
                &CancellationToken::new(),
                &|event| events.lock().unwrap().push(event),
            )
            .unwrap();
        assert_eq!(report.status, RunStatus::Succeeded);
        let ids: Vec<_> = report.nodes.iter().map(|n| n.node.to_string()).collect();
        assert_eq!(ids, ["step_1", "step_2"]);
        assert_eq!(report.nodes[1].message, "now on CMR");
        // Started and finished for each step, in order.
        assert_eq!(events.lock().unwrap().len(), 4);
        // The effect reached the caller's context.
        assert_eq!(
            context.project(&f.profile).unwrap().unwrap().key.as_str(),
            "CMR"
        );
    }

    #[test]
    fn cancelled_step_runs_stop_before_the_next_step() {
        let mut f = fixture();
        let workflow = Workflow::new("Two")
            .step(f.log, OnFailure::Stop)
            .step(f.log, OnFailure::Stop);
        let workflow_id = workflow.id;
        f.profile.workflows.push(workflow);
        let cancel = CancellationToken::new();
        let mut context = ActiveProjectContext::new(&f.profile);
        let report = f
            .engine
            .execute_workflow(
                &f.profile,
                &machine(),
                &mut context,
                workflow_id,
                &cancel,
                &|event| {
                    if matches!(event, ExecutionEvent::NodeFinished { .. }) {
                        cancel.cancel();
                    }
                },
            )
            .unwrap();
        assert_eq!(report.status, RunStatus::Cancelled);
        assert_eq!(report.nodes[0].status, crate::graph::NodeStatus::Succeeded);
        assert_eq!(report.nodes[1].status, crate::graph::NodeStatus::Cancelled);
        assert_eq!(report.into_steps().status, ActionStatus::Failed);
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
