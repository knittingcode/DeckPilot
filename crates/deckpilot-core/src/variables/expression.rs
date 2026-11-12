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
//! | `workflow` | `workflow.execution.id`              | the running workflow            |
//!
//! `steps.` references are parsed by [`crate::graph::Reference::parse`], so
//! every template valid today parses to the same references.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{VariableKey, VariableType};
use crate::graph::Reference;

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
}

impl SystemVariable {
    pub const ALL: [Self; 9] = [
        Self::ProjectCurrentId,
        Self::ProjectCurrentKey,
        Self::ProjectCurrentName,
        Self::ProfileCurrentId,
        Self::ProfileCurrentName,
        Self::DeviceCurrentId,
        Self::Platform,
        Self::WorkflowId,
        Self::WorkflowExecutionId,
    ];

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
        }
    }

    pub fn value_type(self) -> VariableType {
        match self {
            Self::ProjectCurrentId => VariableType::ProjectId,
            Self::ProfileCurrentId => VariableType::ProfileId,
            _ => VariableType::String,
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
        }
    }

    /// The value exists only while something runs: a preview reports it as
    /// unavailable until run time.
    pub fn runtime_only(self) -> bool {
        matches!(self, Self::WorkflowExecutionId)
    }

    /// The value changes with the context (project, device, run), so the
    /// picker refreshes it on context events.
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
    /// `system.…` and `workflow.…`.
    System(SystemVariable),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExpressionError {
    #[error(
        "`{0}` is not a reference; use {{{{ steps.<node>.output.<key> }}}}, \
         {{{{ shared.<namespace>.<name> }}}}, {{{{ system.… }}}} or {{{{ workflow.… }}}}"
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
            _ => Err(invalid()),
        }
    }
}

impl fmt::Display for VariableRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Step(reference) => reference.fmt(f),
            Self::Shared(key) => write!(f, "shared.{key}"),
            Self::System(variable) => f.write_str(variable.path()),
        }
    }
}

impl TryFrom<String> for VariableRef {
    type Error = ExpressionError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
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
