//! Resolving references: the explicit context, the injected resolvers, and
//! what a preview or a run makes of the result.
//!
//! A reference is always resolved in an explicit [`ResolutionContext`]. The
//! agent's resolver is the only implementation that sees real data; the
//! configuration app asks it for previews and never resolves anything
//! itself. [`preview`] and [`bind`] are the single places where redaction
//! and type checks happen, for previews and for runs alike.

use std::fmt;

use deckpilot_device::DeviceId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{ScopeKey, VariableDefinition, VariableRef, VariableType, VariableValue};
use crate::graph::NodeId;
use crate::{
    CredentialSource, ExecutionId, MachineConfig, MachineResourceBinding, ParameterKind,
    ParameterValue, ProfileId, Project, ProjectId, ResolveError, ResourceKind, ResourceRef,
    WorkflowId,
};

/// Where the context's project comes from. A preview always says which
/// one it used, so an execution-local project is never silently replaced by
/// the application's active project.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectSource {
    /// DeckPilot's active project of the profile.
    #[default]
    Application,
    /// The project context of one workflow run, which may differ from the
    /// active project.
    Execution,
    /// Chosen by whoever asks for a preview.
    Explicit,
}

/// Whether references are resolved for a preview in the editor or for a
/// run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolutionPhase {
    #[default]
    Preview,
    Execution,
}

/// Everything a reference may depend on. Ids only: resolvers look up the
/// data themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionContext {
    pub profile: ProfileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectId>,
    #[serde(default)]
    pub project_source: ProjectSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<WorkflowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionId>,
    #[serde(default)]
    pub phase: ResolutionPhase,
}

impl ResolutionContext {
    /// A preview context for `profile` without a project.
    pub fn preview(profile: ProfileId) -> Self {
        Self {
            profile,
            project: None,
            project_source: ProjectSource::Application,
            device: None,
            workflow: None,
            execution: None,
            phase: ResolutionPhase::Preview,
        }
    }

    #[must_use]
    pub fn with_project(self, project: Option<ProjectId>, source: ProjectSource) -> Self {
        Self {
            project,
            project_source: source,
            ..self
        }
    }

    #[must_use]
    pub fn with_device(self, device: Option<DeviceId>) -> Self {
        Self { device, ..self }
    }

    #[must_use]
    pub fn with_workflow(self, workflow: WorkflowId) -> Self {
        Self {
            workflow: Some(workflow),
            ..self
        }
    }

    /// The context of a run: project changes inside it stay in the run.
    #[must_use]
    pub fn for_execution(self, execution: ExecutionId) -> Self {
        Self {
            execution: Some(execution),
            phase: ResolutionPhase::Execution,
            ..self
        }
    }
}

/// Where a resolved value came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ValueSource {
    /// A value set at this scope.
    Scope { scope: ScopeKey },
    /// The variable's default.
    Default,
    /// Taken from the context.
    System,
    /// An earlier node of the run.
    StepOutput { node: NodeId },
}

/// Why a reference has no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MissingReason {
    /// No such shared variable.
    UnknownVariable,
    /// Defined, but no scope in the context has a value and there is no
    /// default.
    NoValue,
    NoProject,
    NoDevice,
    NoWorkflow,
    NoExecution,
    /// The node has not produced the output.
    NoOutput,
    /// The value names a project or profile outside the context's profile.
    OtherProfile,
}

impl MissingReason {
    pub fn describe(self) -> &'static str {
        match self {
            Self::UnknownVariable => "no such variable",
            Self::NoValue => "no value is set",
            Self::NoProject => "no project is active",
            Self::NoDevice => "no device is in the context",
            Self::NoWorkflow => "no workflow is in the context",
            Self::NoExecution => "no run is in the context",
            Self::NoOutput => "the node has not produced it",
            Self::OtherProfile => "it belongs to another profile",
        }
    }
}

/// What a resolver found for one reference, before type checks and
/// redaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Value {
        value: VariableValue,
        source: ValueSource,
        sensitive: bool,
    },
    /// Exists only while running, e.g. `workflow.execution.id` or a node
    /// output, or the project of a run that has not started.
    UnavailableUntilRuntime,
    Missing(MissingReason),
    /// The caller may not read it.
    PermissionDenied,
}

/// Resolves references in an explicit context. The agent holds the real
/// implementation; tests use [`super::doubles::MemoryVariables`].
pub trait VariableResolver: Send + Sync {
    fn resolve(&self, reference: &VariableRef, context: &ResolutionContext) -> Resolution;
}

/// The application's active project of a profile: the global selection,
/// not the project context of a run.
pub trait ProjectResolver: Send + Sync {
    fn active_project(&self, profile: ProfileId) -> Option<ProjectId>;
}

/// Machine bindings of logical resources.
pub trait ResourceResolver: Send + Sync {
    fn resolve_resource(
        &self,
        reference: &ResourceRef,
        project: Option<&Project>,
        expected: Option<ResourceKind>,
    ) -> Result<MachineResourceBinding, ResolveError>;
}

impl ResourceResolver for MachineConfig {
    fn resolve_resource(
        &self,
        reference: &ResourceRef,
        project: Option<&Project>,
        expected: Option<ResourceKind>,
    ) -> Result<MachineResourceBinding, ResolveError> {
        self.resolve(reference, project, expected).cloned()
    }
}

/// The device a run was started from, if any.
pub trait DeviceContext: Send + Sync {
    fn current_device(&self) -> Option<DeviceId>;
}

/// The services an execution depends on, handed to it explicitly; there is
/// no global registry of services. Cancellation, progress and permissions
/// stay where they are ([`crate::ExecutionControl`] and the module's
/// capabilities on [`crate::ExecutionContext`]); credential secrets are read
/// through the existing [`CredentialSource`].
#[derive(Clone, Copy)]
pub struct ExecutionDependencies<'a> {
    pub variables: &'a dyn VariableResolver,
    pub projects: &'a dyn ProjectResolver,
    pub resources: &'a dyn ResourceResolver,
    pub credentials: &'a dyn CredentialSource,
    pub device: &'a dyn DeviceContext,
}

impl fmt::Debug for ExecutionDependencies<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionDependencies")
            .finish_non_exhaustive()
    }
}

/// The value of `definition` in `context`: the value of the most specific
/// scope that has one ([`super::VariableScope::PRECEDENCE`]), else the default.
/// Values at scopes the definition does not allow, or that do not apply to
/// the context, are ignored. Among values of the same scope the first wins;
/// the store keeps at most one per scope key.
pub fn select_value<'a>(
    definition: &'a VariableDefinition,
    values: impl IntoIterator<Item = (&'a ScopeKey, &'a VariableValue)>,
    context: &ResolutionContext,
) -> Option<(&'a VariableValue, ValueSource)> {
    let mut best: Option<(&ScopeKey, &VariableValue)> = None;
    for (scope, value) in values {
        if !definition.scopes.contains(&scope.scope()) || !scope.applies_to(context) {
            continue;
        }
        if best.is_none_or(|(current, _)| scope.scope().rank() < current.scope().rank()) {
            best = Some((scope, value));
        }
    }
    match best {
        Some((scope, value)) => Some((value, ValueSource::Scope { scope: *scope })),
        None => definition
            .default
            .as_ref()
            .map(|value| (value, ValueSource::Default)),
    }
}

/// What a preview shows for one reference. Sensitive values are never part
/// of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReferenceStatus {
    Resolved {
        value: VariableValue,
        source: ValueSource,
    },
    /// Has a value, which is sensitive.
    Redacted {
        value_type: VariableType,
        source: ValueSource,
    },
    UnavailableUntilRuntime,
    Missing {
        reason: MissingReason,
    },
    /// The value cannot fill the parameter it is meant for.
    TypeMismatch {
        found: VariableType,
    },
    PermissionDenied,
    /// The reference itself is not valid.
    Invalid {
        message: String,
    },
}

/// The preview of `resolution` for a parameter of `expected` kind (or for
/// display only, without one).
pub fn preview(resolution: Resolution, expected: Option<&ParameterKind>) -> ReferenceStatus {
    match resolution {
        Resolution::Value {
            value,
            source,
            sensitive,
        } => {
            let found = value.value_type();
            if expected.is_some_and(|kind| value.to_parameter(kind).is_none()) {
                ReferenceStatus::TypeMismatch { found }
            } else if sensitive {
                ReferenceStatus::Redacted {
                    value_type: found,
                    source,
                }
            } else {
                ReferenceStatus::Resolved { value, source }
            }
        }
        Resolution::UnavailableUntilRuntime => ReferenceStatus::UnavailableUntilRuntime,
        Resolution::Missing(reason) => ReferenceStatus::Missing { reason },
        Resolution::PermissionDenied => ReferenceStatus::PermissionDenied,
    }
}

/// One reference a preview asks about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    /// The reference without braces, as typed; invalid text is answered
    /// with [`ReferenceStatus::Invalid`].
    pub reference: String,
    /// The kind of the parameter it fills, for the type check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<ParameterKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferencePreview {
    pub reference: String,
    pub status: ReferenceStatus,
}

/// Parses, resolves and previews one item.
pub fn preview_reference(
    resolver: &dyn VariableResolver,
    item: &PreviewItem,
    context: &ResolutionContext,
) -> ReferencePreview {
    let status = match VariableRef::parse(&item.reference) {
        Ok(reference) => preview(
            resolver.resolve(&reference, context),
            item.expected.as_ref(),
        ),
        Err(error) => ReferenceStatus::Invalid {
            message: error.to_string(),
        },
    };
    ReferencePreview {
        reference: item.reference.trim().to_owned(),
        status,
    }
}

/// Why a reference could not fill a parameter at run time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceProblem {
    Missing(MissingReason),
    TypeMismatch {
        found: VariableType,
        expected: &'static str,
    },
    PermissionDenied,
    Unavailable,
}

/// A reference that could not be resolved. The message names the parameter
/// and the reference, never a value.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub struct ReferenceError {
    pub parameter: Option<String>,
    pub reference: String,
    pub problem: ReferenceProblem,
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(parameter) = &self.parameter {
            write!(f, "parameter `{parameter}`: ")?;
        }
        let reference = &self.reference;
        match &self.problem {
            ReferenceProblem::Missing(reason) => {
                write!(
                    f,
                    "{{{{ {reference} }}}} has no value ({})",
                    reason.describe()
                )
            }
            ReferenceProblem::TypeMismatch { found, expected } => write!(
                f,
                "{{{{ {reference} }}}} is a {} value, the parameter expects {expected}",
                found.name()
            ),
            ReferenceProblem::PermissionDenied => {
                write!(f, "{{{{ {reference} }}}} may not be read here")
            }
            ReferenceProblem::Unavailable => {
                write!(f, "{{{{ {reference} }}}} is not available in this run")
            }
        }
    }
}

impl ReferenceError {
    /// Stable code for [`crate::ActionResult::code`].
    pub fn code(&self) -> &'static str {
        match self.problem {
            ReferenceProblem::Missing(_) => "reference-missing",
            ReferenceProblem::TypeMismatch { .. } => "reference-type",
            ReferenceProblem::PermissionDenied => "reference-denied",
            ReferenceProblem::Unavailable => "reference-unavailable",
        }
    }
}

/// The value of parameter `parameter` (of `kind`) from a whole-value
/// reference, at run time.
pub fn bind(
    parameter: &str,
    kind: &ParameterKind,
    reference: &VariableRef,
    resolution: Resolution,
) -> Result<ParameterValue, ReferenceError> {
    let error = |problem| ReferenceError {
        parameter: Some(parameter.to_owned()),
        reference: reference.to_string(),
        problem,
    };
    match resolution {
        Resolution::Value { value, .. } => value.to_parameter(kind).ok_or_else(|| {
            error(ReferenceProblem::TypeMismatch {
                found: value.value_type(),
                expected: kind_name(kind),
            })
        }),
        Resolution::UnavailableUntilRuntime => Err(error(ReferenceProblem::Unavailable)),
        Resolution::Missing(reason) => Err(error(ReferenceProblem::Missing(reason))),
        Resolution::PermissionDenied => Err(error(ReferenceProblem::PermissionDenied)),
    }
}

fn kind_name(kind: &ParameterKind) -> &'static str {
    match kind {
        ParameterKind::Text { .. }
        | ParameterKind::Lines { .. }
        | ParameterKind::Pairs { .. }
        | ParameterKind::Code { .. } => "text",
        ParameterKind::Integer { .. } => "an integer",
        ParameterKind::Boolean => "true or false",
        ParameterKind::Choice { .. } => "one of its choices",
        ParameterKind::Resource { .. } => "a resource",
        ParameterKind::Project => "a project",
        ParameterKind::Credential => "a credential",
    }
}
