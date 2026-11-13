//! The resolution pipeline: how configured parameters become the literal
//! values a handler receives.
//!
//! Every way an action runs (a key press, a workflow node, a trigger, an
//! inline invocation) goes through [`resolve_parameters`] (see
//! `docs/action-architecture.md`, section 7):
//!
//! 1. the definition comes from the registry and the configured values from
//!    the instance, the node's overrides and mappings, or the inline
//!    invocation (done by the caller, [`crate::Engine`]);
//! 2. the [`ResolutionContext`] is captured once per run of the action;
//! 3. every [`ParameterValue::Reference`] and [`ParameterValue::Template`] is
//!    resolved through a [`ContextResolver`]: core and module variables
//!    (`system.`, `workflow.`, module namespaces and the legacy aliases)
//!    through the [`VariableRegistry`] from the explicit context, `shared.`
//!    through the injected [`VariableResolver`], `steps.` from the run's
//!    outputs;
//! 4. each reference is bound to its parameter's kind ([`bind`]) and the
//!    result is validated by [`crate::ActionType::normalize`];
//! 5. values of sensitive variables are collected in a [`Redactor`] that
//!    removes them from the handler's message, data and progress.
//!
//! Resolved values are never expanded again: a value that looks like a
//! reference is text, so references cannot form cycles.

use std::collections::BTreeMap;

use deckpilot_device::DeviceId;

use crate::graph::NodeId;
use crate::variables::{
    ExpressionPart, LiveState, MissingReason, ProjectSource, ReferenceError, Resolution,
    ResolutionContext, ResolutionPhase, SystemVariable, ValueSource, VariableEnvironment,
    VariableRef, VariableRegistry, VariableResolver, VariableValue, bind, parse_expression,
};
use crate::{
    ActionParameters, ActionResult, ActionType, ExecutionId, Os, ParameterKind, ParameterValue,
    Profile, ProjectId, WorkflowId,
};

/// Replaces values that must not be shown.
pub const REDACTED: &str = "[redacted]";

/// Where an action was started from, beyond the profile and the project:
/// what the context of its references is built from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Invocation {
    /// The device whose control started it.
    pub device: Option<DeviceId>,
    /// The workflow it runs in.
    pub workflow: Option<WorkflowId>,
    /// The run it belongs to.
    pub execution: Option<ExecutionId>,
}

impl Invocation {
    #[must_use]
    pub fn from_device(device: DeviceId) -> Self {
        Self {
            device: Some(device),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_workflow(self, workflow: WorkflowId) -> Self {
        Self {
            workflow: Some(workflow),
            ..self
        }
    }

    #[must_use]
    pub fn with_execution(self, execution: Option<ExecutionId>) -> Self {
        Self { execution, ..self }
    }

    /// The context of an action that runs now, with `project` as its
    /// project. Inside a workflow the project is the run's own, which may
    /// differ from the application's active project.
    pub fn context(&self, profile: &Profile, project: Option<ProjectId>) -> ResolutionContext {
        let source = if self.workflow.is_some() {
            ProjectSource::Execution
        } else {
            ProjectSource::Application
        };
        ResolutionContext {
            profile: profile.id,
            project,
            project_source: source,
            device: self.device.clone(),
            workflow: self.workflow,
            execution: self.execution,
            phase: ResolutionPhase::Execution,
        }
    }
}

/// Outputs of the nodes of a run, by node and key.
pub trait StepOutputs: Sync {
    fn output(&self, node: &NodeId, key: &str) -> Option<&str>;
}

impl StepOutputs for BTreeMap<NodeId, BTreeMap<String, String>> {
    fn output(&self, node: &NodeId, key: &str) -> Option<&str> {
        self.get(node)?.get(key).map(String::as_str)
    }
}

/// A [`VariableResolver`] that knows no shared variables, for engines
/// without a variable store.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSharedVariables;

impl VariableResolver for NoSharedVariables {
    fn resolve(&self, reference: &VariableRef, _context: &ResolutionContext) -> Resolution {
        match reference {
            VariableRef::Shared(_) => Resolution::Missing(MissingReason::UnknownVariable),
            // Only shared variables are asked for (see `ContextResolver`).
            VariableRef::Step(_) | VariableRef::System(_) | VariableRef::Module(_) => {
                Resolution::UnavailableUntilRuntime
            }
        }
    }
}

/// The value of a system variable in `context`, taken from `profile` (the
/// profile the context names) and this computer. The project is the
/// context's, never the application's active project in its place: a
/// preview of a run's project is unavailable until the run.
///
/// The core's variables are resolved with this by the
/// [`VariableRegistry`]. Project values here are those of the legacy
/// `system.project.current.…` ids, which the registry resolves through the
/// projects module instead; work session variables are not in the context
/// and have no value here.
pub fn system_value(
    variable: SystemVariable,
    profile: &Profile,
    os: Os,
    context: &ResolutionContext,
) -> Resolution {
    if context.profile != profile.id {
        return Resolution::Missing(MissingReason::OtherProfile);
    }
    let preview = context.phase == ResolutionPhase::Preview;
    let value = |value: VariableValue| Resolution::Value {
        value,
        source: ValueSource::System,
        sensitive: false,
    };
    match variable {
        SystemVariable::ProjectCurrentId
        | SystemVariable::ProjectCurrentKey
        | SystemVariable::ProjectCurrentName => {
            if preview && context.project_source == ProjectSource::Execution {
                return Resolution::UnavailableUntilRuntime;
            }
            let Some(id) = context.project else {
                return Resolution::Missing(MissingReason::NoProject);
            };
            let Some(project) = profile.project(id) else {
                return Resolution::Missing(MissingReason::OtherProfile);
            };
            value(match variable {
                SystemVariable::ProjectCurrentId => VariableValue::ProjectId(project.id),
                SystemVariable::ProjectCurrentKey => VariableValue::String(project.key.to_string()),
                _ => VariableValue::String(project.name.clone()),
            })
        }
        SystemVariable::ProfileCurrentId => value(VariableValue::ProfileId(profile.id)),
        SystemVariable::ProfileCurrentName => value(VariableValue::String(profile.name.clone())),
        SystemVariable::DeviceCurrentId => match &context.device {
            Some(device) => value(VariableValue::String(device.to_string())),
            None => Resolution::Missing(MissingReason::NoDevice),
        },
        SystemVariable::Platform => value(VariableValue::String(platform_name(os).to_owned())),
        SystemVariable::WorkflowId => match context.workflow {
            Some(workflow) => value(VariableValue::String(workflow.to_string())),
            None => Resolution::Missing(MissingReason::NoWorkflow),
        },
        SystemVariable::WorkflowExecutionId => match context.execution {
            _ if preview => Resolution::UnavailableUntilRuntime,
            Some(execution) => value(VariableValue::String(execution.to_string())),
            None => Resolution::Missing(MissingReason::NoExecution),
        },
        SystemVariable::WorkSessionId
        | SystemVariable::WorkSessionState
        | SystemVariable::WorkSessionStartedAt
        | SystemVariable::WorkSessionElapsedSeconds
        | SystemVariable::WorkSessionProjectId
        | SystemVariable::WorkSessionCanFinish => Resolution::Missing(MissingReason::NoValue),
    }
}

/// `system.platform` of `os`.
pub fn platform_name(os: Os) -> &'static str {
    match os {
        Os::Linux => "linux",
        Os::MacOs => "macOs",
        Os::Windows => "windows",
        Os::Other => "other",
    }
}

/// A shared value that names a project or profile outside `profile` is
/// treated as missing, so values never cross profiles even if a store
/// returned one.
pub fn confine_to_profile(resolution: Resolution, profile: &Profile) -> Resolution {
    match &resolution {
        Resolution::Value {
            value: VariableValue::ProjectId(id),
            ..
        } if profile.project(*id).is_none() => Resolution::Missing(MissingReason::OtherProfile),
        Resolution::Value {
            value: VariableValue::ProfileId(id),
            ..
        } if *id != profile.id => Resolution::Missing(MissingReason::OtherProfile),
        _ => resolution,
    }
}

/// Resolves every root for an action that runs in `profile`: core and
/// module variables (and the legacy aliases of module variables) through
/// `registry`, shared variables through `shared`, node outputs from
/// `outputs`. This one resolver serves key presses, workflow nodes,
/// triggers, inline invocations, parameter overrides and previews.
#[derive(Clone, Copy)]
pub struct ContextResolver<'a> {
    pub profile: &'a Profile,
    pub os: Os,
    pub shared: &'a dyn VariableResolver,
    /// The outputs of the run so far; `None` outside a workflow run.
    pub outputs: Option<&'a dyn StepOutputs>,
    /// The core's and the modules' variables.
    pub registry: &'a VariableRegistry,
    /// Live values outside the context, such as the work session.
    pub live: &'a dyn LiveState,
}

impl std::fmt::Debug for ContextResolver<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextResolver")
            .field("profile", &self.profile.id)
            .finish_non_exhaustive()
    }
}

impl VariableResolver for ContextResolver<'_> {
    fn resolve(&self, reference: &VariableRef, context: &ResolutionContext) -> Resolution {
        if context.profile != self.profile.id {
            return Resolution::Missing(MissingReason::OtherProfile);
        }
        match reference {
            // Core and module variables, and the legacy ids of module
            // variables: computed when asked (never cached); a project of
            // another profile is missing. Legacy ids resolve as they always
            // did, whatever the state of the module that owns them now.
            VariableRef::System(_) | VariableRef::Module(_) => {
                let environment = VariableEnvironment {
                    context,
                    profile: self.profile,
                    os: self.os,
                    live: self.live,
                };
                let resolution = match reference {
                    VariableRef::System(variable) => {
                        self.registry.resolve_legacy(*variable, &environment)
                    }
                    VariableRef::Module(id) => self.registry.resolve(id, &environment),
                    VariableRef::Shared(_) | VariableRef::Step(_) => {
                        Resolution::Missing(MissingReason::UnknownVariable)
                    }
                };
                confine_to_profile(resolution, self.profile)
            }
            VariableRef::Shared(_) => {
                confine_to_profile(self.shared.resolve(reference, context), self.profile)
            }
            VariableRef::Step(_) if context.phase == ResolutionPhase::Preview => {
                Resolution::UnavailableUntilRuntime
            }
            VariableRef::Step(step) => match self.outputs {
                None => Resolution::UnavailableUntilRuntime,
                Some(outputs) => outputs.output(&step.node, &step.output).map_or(
                    Resolution::Missing(MissingReason::NoOutput),
                    |text| Resolution::Value {
                        value: VariableValue::String(text.to_owned()),
                        source: ValueSource::StepOutput {
                            node: step.node.clone(),
                        },
                        sensitive: false,
                    },
                ),
            },
        }
    }
}

/// Removes the values of sensitive variables from text that is shown,
/// logged or stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Redactor {
    /// Longest first, so that a value containing another is removed whole.
    secrets: Vec<String>,
}

impl Redactor {
    /// Adds a value to remove. Empty text is ignored.
    pub fn add(&mut self, secret: &str) {
        if secret.is_empty() || self.secrets.iter().any(|s| s == secret) {
            return;
        }
        self.secrets.push(secret.to_owned());
        self.secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }

    /// Adds what `value` looks like as text.
    pub fn add_value(&mut self, value: &VariableValue) {
        match value {
            VariableValue::Resource(_) | VariableValue::SecretRef(_) => {}
            VariableValue::Json(json) => {
                self.add(&json.to_string());
                if let Some(text) = json.as_str() {
                    self.add(text);
                }
            }
            other => {
                if let Some(text) = other.render_text() {
                    self.add(&text);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    pub fn redact(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for secret in &self.secrets {
            if text.contains(secret.as_str()) {
                text = text.replace(secret.as_str(), REDACTED);
            }
        }
        text
    }

    /// The result without sensitive values in its message, data or code.
    pub fn redact_result(&self, mut result: ActionResult) -> ActionResult {
        if self.is_empty() {
            return result;
        }
        result.message = self.redact(&result.message);
        for value in result.data.values_mut() {
            *value = self.redact(value);
        }
        result.code = result.code.map(|code| self.redact(&code));
        result
    }
}

/// Parameters with every reference resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Normalized literals only.
    pub parameters: ActionParameters,
    /// The sensitive values that went into them.
    pub redactor: Redactor,
}

/// Why the parameters could not be resolved. The message names parameters
/// and references and never contains a sensitive value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionFailure {
    References(Vec<ReferenceError>),
    /// The resolved values do not fit the schema (ranges, lengths,
    /// choices); sensitive values are redacted from the messages.
    Invalid(Vec<String>),
}

impl ResolutionFailure {
    /// The failed result of the action.
    pub fn into_result(self) -> ActionResult {
        match self {
            Self::References(errors) => {
                let code = errors
                    .first()
                    .map_or("reference-missing", ReferenceError::code);
                let message: Vec<String> = errors.iter().map(ToString::to_string).collect();
                ActionResult::error(code, message.join("; "))
            }
            Self::Invalid(details) => {
                ActionResult::failure(format!("invalid configuration: {}", details.join("; ")))
            }
        }
    }
}

/// Resolves the references of `configured` (already overlaid: instance,
/// overrides, mappings) for an action of `definition`, then validates and
/// normalizes the result. Literal values are left exactly as they are.
pub fn resolve_parameters(
    definition: &ActionType,
    configured: &ActionParameters,
    resolver: &dyn VariableResolver,
    context: &ResolutionContext,
    profile: &Profile,
) -> Result<Resolved, ResolutionFailure> {
    let mut redactor = Redactor::default();
    let mut errors = Vec::new();
    let mut literal = configured.clone();
    // Defaults may be references too; resolve what normalize would fill.
    let mut values: Vec<(String, ParameterValue)> = configured
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for spec in &definition.parameters {
        if configured.get(&spec.key).is_none()
            && let Some(default) = &spec.default
        {
            values.push((spec.key.clone(), default.clone()));
        }
    }
    for (key, value) in values {
        if value.is_literal() {
            continue;
        }
        // Unknown parameters are reported by normalize.
        let Some(spec) = definition.parameters.iter().find(|spec| spec.key == key) else {
            continue;
        };
        let outcome = match &value {
            ParameterValue::Reference(reference) => resolve_reference(
                &key,
                &spec.kind,
                reference,
                resolver,
                context,
                profile,
                &mut redactor,
            ),
            ParameterValue::Template(template) => {
                render_template(&key, template, resolver, context, &mut redactor)
            }
            _ => unreachable!("literal values are skipped"),
        };
        match outcome {
            Ok(value) => literal = literal.with(key, value),
            Err(mut problems) => errors.append(&mut problems),
        }
    }
    if !errors.is_empty() {
        return Err(ResolutionFailure::References(errors));
    }
    let parameters = definition.normalize(&literal).map_err(|problems| {
        ResolutionFailure::Invalid(
            problems
                .iter()
                .map(|problem| redactor.redact(&problem.to_string()))
                .collect(),
        )
    })?;
    debug_assert!(parameters.is_resolved());
    Ok(Resolved {
        parameters,
        redactor,
    })
}

fn resolve_reference(
    key: &str,
    kind: &ParameterKind,
    reference: &VariableRef,
    resolver: &dyn VariableResolver,
    context: &ResolutionContext,
    profile: &Profile,
    redactor: &mut Redactor,
) -> Result<ParameterValue, Vec<ReferenceError>> {
    let resolution = resolver.resolve(reference, context);
    if let Resolution::Value {
        value, sensitive, ..
    } = &resolution
        && *sensitive
    {
        redactor.add_value(value);
    }
    // Node outputs are text, converted to the parameter's kind as mapped
    // values are.
    if let (
        VariableRef::Step(_),
        Resolution::Value {
            value: VariableValue::String(text),
            ..
        },
    ) = (reference, &resolution)
        && let Some(value) = convert_output(kind, text, profile)
    {
        return Ok(value);
    }
    bind(key, kind, reference, resolution).map_err(|error| vec![error])
}

/// A node output (text) as a value of `kind`, like a mapped value;
/// `None` if it cannot be converted (reported as a type mismatch).
fn convert_output(kind: &ParameterKind, text: &str, profile: &Profile) -> Option<ParameterValue> {
    Some(match kind {
        ParameterKind::Integer { .. } => ParameterValue::Integer(text.trim().parse().ok()?),
        ParameterKind::Boolean => match text.trim() {
            "true" => ParameterValue::Boolean(true),
            "false" => ParameterValue::Boolean(false),
            _ => return None,
        },
        ParameterKind::Project => ParameterValue::Project(
            profile
                .projects
                .iter()
                .find(|p| p.key.as_str() == text.trim())?
                .id,
        ),
        ParameterKind::Resource { .. } => return None,
        ParameterKind::Choice { .. } => ParameterValue::Choice(text.to_owned()),
        ParameterKind::Text { .. }
        | ParameterKind::Lines { .. }
        | ParameterKind::Pairs { .. }
        | ParameterKind::Code { .. }
        | ParameterKind::Credential => ParameterValue::Text(text.to_owned()),
    })
}

fn render_template(
    key: &str,
    template: &str,
    resolver: &dyn VariableResolver,
    context: &ResolutionContext,
    redactor: &mut Redactor,
) -> Result<ParameterValue, Vec<ReferenceError>> {
    let text_kind = ParameterKind::Text {
        multiline: true,
        max_length: None,
    };
    let Ok(parts) = parse_expression(template) else {
        // Validated by normalize: let it report the template.
        return Ok(ParameterValue::Template(template.to_owned()));
    };
    let mut text = String::new();
    let mut errors = Vec::new();
    for part in parts {
        match part {
            ExpressionPart::Text(literal) => text.push_str(&literal),
            ExpressionPart::Reference(reference) => {
                let resolution = resolver.resolve(&reference, context);
                if let Resolution::Value {
                    value, sensitive, ..
                } = &resolution
                    && *sensitive
                {
                    redactor.add_value(value);
                }
                match bind(key, &text_kind, &reference, resolution) {
                    Ok(ParameterValue::Text(rendered)) => text.push_str(&rendered),
                    Ok(_) => unreachable!("a text parameter receives text"),
                    Err(error) => errors.push(error),
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(ParameterValue::Text(text))
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests;
