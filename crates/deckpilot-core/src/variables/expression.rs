//! The reference grammar: the workflow mapping syntax with more roots.
//!
//! A reference is written `{{ <root>.<path> }}`, exactly like the existing
//! `{{ steps.<node>.output.<key> }}`, and is still plain substitution: no
//! expressions, filters or code. The roots are closed:
//!
//! | Root       | Example                              | Meaning                         |
//! |------------|--------------------------------------|---------------------------------|
//! | `steps`    | `steps.git_status.output.branch`     | an earlier node's output (unchanged; `steps.vars.output.<key>` is a workflow variable) |
//! | `shared`   | `shared.projects.favorite`           | a shared variable               |
//! | `system`   | `system.project.current.id`          | a read-only system variable     |
//! |            | `system.work.session.elapsedSeconds` | the active work session (D5)    |
//! | `workflow` | `workflow.execution.id`              | the running workflow            |
//! | a module's namespace | `projects.current.key`     | a variable the module declares (see `docs/variables.md`) |
//!
//! `system.project.current.…` and `system.work.session.…` are aliases of
//! `projects.current.…` and `workSession.…` ([`super::ALIASES`]); they keep
//! their spelling when parsed and printed.
//!
//! `steps.` references are parsed by [`crate::graph::Reference::parse`], so
//! every template valid today parses to the same references.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::registry::{RESERVED_ROOTS, VariableId};
use super::{VariableKey, VariableType};
use crate::graph::Reference;
use crate::work_session::WorkSessionVariable;

/// A read-only value taken from the context of the resolution, never set by
/// the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemVariable {
    ProjectCurrentId,
    ProjectCurrentKey,
    ProjectCurrentName,
    ProfileCurrentId,
    ProfileCurrentName,
    /// `<model>#<serial>` of the device whose control started the run.
    DeviceCurrentId,
    /// `linux`, `macOs`, `windows` or `other`.
    Platform,
    WorkflowId,
    WorkflowExecutionId,
    /// `system.work.session.…`: the active work session, resolved by the
    /// agent from its live snapshot (`docs/gestures-and-work-session.md`
    /// §8.1).
    WorkSessionId,
    WorkSessionState,
    WorkSessionStartedAt,
    WorkSessionElapsedSeconds,
    WorkSessionProjectId,
    WorkSessionCanFinish,
}

impl SystemVariable {
    pub const ALL: [Self; 15] = [
        Self::ProjectCurrentId,
        Self::ProjectCurrentKey,
        Self::ProjectCurrentName,
        Self::ProfileCurrentId,
        Self::ProfileCurrentName,
        Self::DeviceCurrentId,
        Self::Platform,
        Self::WorkflowId,
        Self::WorkflowExecutionId,
        Self::WorkSessionId,
        Self::WorkSessionState,
        Self::WorkSessionStartedAt,
        Self::WorkSessionElapsedSeconds,
        Self::WorkSessionProjectId,
        Self::WorkSessionCanFinish,
    ];

    /// The work session variable this is, if it is one.
    pub fn work_session(self) -> Option<WorkSessionVariable> {
        Some(match self {
            Self::WorkSessionId => WorkSessionVariable::Id,
            Self::WorkSessionState => WorkSessionVariable::State,
            Self::WorkSessionStartedAt => WorkSessionVariable::StartedAt,
            Self::WorkSessionElapsedSeconds => WorkSessionVariable::ElapsedSeconds,
            Self::WorkSessionProjectId => WorkSessionVariable::ProjectId,
            Self::WorkSessionCanFinish => WorkSessionVariable::CanFinish,
            _ => return None,
        })
    }

    /// The reference path, without braces.
    pub fn path(self) -> &'static str {
        match self {
            Self::ProjectCurrentId => "system.project.current.id",
            Self::ProjectCurrentKey => "system.project.current.key",
            Self::ProjectCurrentName => "system.project.current.name",
            Self::ProfileCurrentId => "system.profile.current.id",
            Self::ProfileCurrentName => "system.profile.current.name",
            Self::DeviceCurrentId => "system.device.current.id",
            Self::Platform => "system.platform",
            Self::WorkflowId => "workflow.id",
            Self::WorkflowExecutionId => "workflow.execution.id",
            Self::WorkSessionId
            | Self::WorkSessionState
            | Self::WorkSessionStartedAt
            | Self::WorkSessionElapsedSeconds
            | Self::WorkSessionProjectId
            | Self::WorkSessionCanFinish => {
                self.work_session().map_or("", WorkSessionVariable::path)
            }
        }
    }

    pub fn value_type(self) -> VariableType {
        match self {
            Self::ProjectCurrentId => VariableType::ProjectId,
            Self::ProfileCurrentId => VariableType::ProfileId,
            _ => self
                .work_session()
                .map_or(VariableType::String, WorkSessionVariable::value_type),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ProjectCurrentId => "Id of the project in the context",
            Self::ProjectCurrentKey => "Key of the project in the context",
            Self::ProjectCurrentName => "Name of the project in the context",
            Self::ProfileCurrentId => "Id of the profile in the context",
            Self::ProfileCurrentName => "Name of the profile in the context",
            Self::DeviceCurrentId => "The device whose control started the run",
            Self::Platform => "Operating system of this computer",
            Self::WorkflowId => "Id of the running workflow",
            Self::WorkflowExecutionId => "Id of this run of the workflow",
            Self::WorkSessionId
            | Self::WorkSessionState
            | Self::WorkSessionStartedAt
            | Self::WorkSessionElapsedSeconds
            | Self::WorkSessionProjectId
            | Self::WorkSessionCanFinish => self
                .work_session()
                .map_or("", WorkSessionVariable::description),
        }
    }

    /// The value exists only while something runs: a preview reports it as
    /// unavailable until run time.
    pub fn runtime_only(self) -> bool {
        matches!(self, Self::WorkflowExecutionId)
    }

    /// The value changes with the context (project, device, run) or the
    /// work session, so the picker refreshes it on context events.
    pub fn dynamic(self) -> bool {
        !matches!(self, Self::Platform)
    }

    fn from_path(path: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.path() == path)
    }
}

/// What a `{{ … }}` placeholder refers to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum VariableRef {
    /// `steps.<node>.output.<key>`, including workflow variables
    /// (`steps.vars.output.<key>`).
    Step(Reference),
    /// `shared.<key>`.
    Shared(VariableKey),
    /// `system.…` and `workflow.…`, including the aliases of module
    /// variables, which keep their spelling.
    System(SystemVariable),
    /// `<namespace>.…`: a variable a module declares, by its canonical id.
    /// Parsed without knowing the modules: whether it exists is decided
    /// when it is resolved, so a reference to a disabled or missing module
    /// stays readable.
    Module(VariableId),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExpressionError {
    #[error(
        "`{0}` is not a reference; use {{{{ steps.<node>.output.<key> }}}}, \
         {{{{ shared.<namespace>.<name> }}}}, {{{{ system.… }}}}, {{{{ workflow.… }}}} \
         or a module variable such as {{{{ projects.current.key }}}}"
    )]
    Invalid(String),
    #[error("a `{{{{` is not closed")]
    Unclosed,
}

impl VariableRef {
    /// Parses a reference without braces; surrounding spaces are ignored.
    pub fn parse(text: &str) -> Result<Self, ExpressionError> {
        let trimmed = text.trim();
        let invalid = || ExpressionError::Invalid(trimmed.to_owned());
        let (root, rest) = trimmed.split_once('.').ok_or_else(invalid)?;
        match root {
            "steps" => Reference::parse(trimmed)
                .map(Self::Step)
                .map_err(|_| invalid()),
            "shared" => VariableKey::new(rest)
                .map(Self::Shared)
                .map_err(|_| invalid()),
            "system" | "workflow" => SystemVariable::from_path(trimmed)
                .map(Self::System)
                .ok_or_else(invalid),
            root if RESERVED_ROOTS.contains(&root) => Err(invalid()),
            _ => VariableId::new(trimmed)
                .map(Self::Module)
                .map_err(|_| invalid()),
        }
    }

    /// The canonical id of a registered variable: a core or module
    /// variable, or the canonical id of an alias. `None` for shared
    /// variables and node outputs.
    pub fn registered_id(&self) -> Option<VariableId> {
        match self {
            Self::System(variable) => Some(variable.canonical()),
            Self::Module(id) => Some(id.clone()),
            Self::Shared(_) | Self::Step(_) => None,
        }
    }
}

impl fmt::Display for VariableRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Step(reference) => reference.fmt(f),
            Self::Shared(key) => write!(f, "shared.{key}"),
            Self::System(variable) => f.write_str(variable.path()),
            Self::Module(id) => id.fmt(f),
        }
    }
}

impl TryFrom<String> for VariableRef {
    type Error = ExpressionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<WorkSessionVariable> for SystemVariable {
    fn from(variable: WorkSessionVariable) -> Self {
        match variable {
            WorkSessionVariable::Id => Self::WorkSessionId,
            WorkSessionVariable::State => Self::WorkSessionState,
            WorkSessionVariable::StartedAt => Self::WorkSessionStartedAt,
            WorkSessionVariable::ElapsedSeconds => Self::WorkSessionElapsedSeconds,
            WorkSessionVariable::ProjectId => Self::WorkSessionProjectId,
            WorkSessionVariable::CanFinish => Self::WorkSessionCanFinish,
        }
    }
}

impl From<VariableRef> for String {
    fn from(value: VariableRef) -> Self {
        value.to_string()
    }
}

/// Literal text and references of a template, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionPart {
    Text(String),
    Reference(VariableRef),
}

/// Parses a template such as `cd {{ shared.paths.root }}/{{ steps.a.output.b }}`.
/// The splitting rules are those of [`crate::graph::parse_template`].
pub fn parse_expression(template: &str) -> Result<Vec<ExpressionPart>, ExpressionError> {
    let mut parts = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            parts.push(ExpressionPart::Text(rest[..start].to_owned()));
        }
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or(ExpressionError::Unclosed)?;
        parts.push(ExpressionPart::Reference(VariableRef::parse(
            &after[..end],
        )?));
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        parts.push(ExpressionPart::Text(rest.to_owned()));
    }
    Ok(parts)
}

/// Every reference in `template`.
pub fn references_in_expression(template: &str) -> Result<Vec<VariableRef>, ExpressionError> {
    Ok(parse_expression(template)?
        .into_iter()
        .filter_map(|part| match part {
            ExpressionPart::Reference(reference) => Some(reference),
            ExpressionPart::Text(_) => None,
        })
        .collect())
}
