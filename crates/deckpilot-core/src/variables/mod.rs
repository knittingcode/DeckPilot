//! Shared variables, system variables and references to them.
//!
//! This is the shared contract of the action architecture (see
//! `docs/action-architecture.md`): the value types, scopes and keys of
//! shared variables, the reference grammar (an extension of the workflow
//! mapping syntax `{{ steps.<node>.output.<key> }}`), the explicit context a
//! reference is resolved in, the resolver traits that are injected into an
//! execution, and the metadata the variable picker shows.
//!
//! Nothing here is wired into execution yet: the engine, the agent and the
//! store keep working exactly as before.

pub mod catalog;
pub mod doubles;
pub mod expression;
pub mod registry;
pub mod resolution;

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    CredentialId, ExecutionId, ModuleId, ParameterKind, ParameterValue, ProfileId, ProjectId,
    ResourceKind, ResourceRef,
};

pub use catalog::{CatalogEntry, CatalogFilter, ReferenceAvailability, VariableGroup};
pub use expression::{
    ExpressionError, ExpressionPart, SystemVariable, VariableRef, parse_expression,
    references_in_expression,
};
pub use registry::{
    ALIASES, ContextDevice, DeclaredScope, Deprecation, LiveState, NoLiveState, Requirement,
    VariableAlias, VariableDeclaration, VariableEnvironment, VariableId, VariableOwner,
    VariableRegistry, VariableRegistryError, VariableSource, module_namespace,
};
pub use resolution::{
    DeviceContext, ExecutionDependencies, MissingReason, PreviewItem, ProjectResolver,
    ProjectSource, ReferenceError, ReferencePreview, ReferenceProblem, ReferenceStatus, Resolution,
    ResolutionContext, ResolutionPhase, ResourceResolver, ValueSource, VariableResolver, bind,
    preview, preview_reference, select_value,
};

/// The type of a shared variable, a system variable or a resolved reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum VariableType {
    String,
    Integer,
    Boolean,
    /// A project of the profile in whose context the value is resolved.
    ProjectId,
    ProfileId,
    /// A logical resource, fixed or a role of the context's project.
    Resource {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ResourceKind>,
    },
    /// Any JSON value.
    Json,
    /// The name of a stored credential. The secret itself is never a
    /// variable value; only an action with the credentials capability reads
    /// it, through the credential source of its execution.
    SecretRef,
}

impl VariableType {
    /// Short name used in search and messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::ProjectId => "projectId",
            Self::ProfileId => "profileId",
            Self::Resource { .. } => "resource",
            Self::Json => "json",
            Self::SecretRef => "secretRef",
        }
    }

    /// Whether a value of this type can fill a parameter of `kind` as a
    /// whole-value reference. Values keep their native type: an integer
    /// fills an integer parameter, a project id a project parameter. Text
    /// parameters accept anything that renders as text; a secret reference
    /// fills only a credential parameter and never renders as text.
    pub fn assignable_to(self, kind: &ParameterKind) -> bool {
        match (self, kind) {
            (Self::SecretRef, ParameterKind::Credential)
            | (Self::Integer, ParameterKind::Integer { .. })
            | (Self::Boolean, ParameterKind::Boolean)
            | (Self::ProjectId, ParameterKind::Project)
            | (Self::String, ParameterKind::Choice { .. }) => true,
            (Self::Resource { kind: have }, ParameterKind::Resource { kind: want }) => {
                have.is_none() || want.is_none() || have == *want
            }
            (Self::Json, ParameterKind::Code { language, .. }) => language == "json",
            (
                Self::String | Self::Integer | Self::Boolean | Self::ProjectId | Self::ProfileId,
                kind,
            )
            | (Self::Json, kind @ ParameterKind::Text { .. }) => is_text(kind),
            _ => false,
        }
    }
}

fn is_text(kind: &ParameterKind) -> bool {
    matches!(
        kind,
        ParameterKind::Text { .. }
            | ParameterKind::Lines { .. }
            | ParameterKind::Pairs { .. }
            | ParameterKind::Code { .. }
    )
}

/// A typed value of a variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum VariableValue {
    String(String),
    Integer(i64),
    Boolean(bool),
    ProjectId(ProjectId),
    ProfileId(ProfileId),
    Resource(ResourceRef),
    Json(serde_json::Value),
    SecretRef(CredentialId),
}

impl VariableValue {
    pub fn value_type(&self) -> VariableType {
        match self {
            Self::String(_) => VariableType::String,
            Self::Integer(_) => VariableType::Integer,
            Self::Boolean(_) => VariableType::Boolean,
            Self::ProjectId(_) => VariableType::ProjectId,
            Self::ProfileId(_) => VariableType::ProfileId,
            Self::Resource(_) => VariableType::Resource { kind: None },
            Self::Json(_) => VariableType::Json,
            Self::SecretRef(_) => VariableType::SecretRef,
        }
    }

    /// Whether the value is of `declared` type. A resource value matches
    /// any resource kind; the kind is checked when it is resolved on the
    /// machine.
    pub fn is_of(&self, declared: VariableType) -> bool {
        match (self.value_type(), declared) {
            (VariableType::Resource { .. }, VariableType::Resource { .. }) => true,
            (have, want) => have == want,
        }
    }

    /// The value inserted into a text template. `None` for values that are
    /// not text: resources, and secret references, which never become text.
    pub fn render_text(&self) -> Option<String> {
        match self {
            Self::String(text) => Some(text.clone()),
            Self::Integer(number) => Some(number.to_string()),
            Self::Boolean(flag) => Some(flag.to_string()),
            Self::ProjectId(id) => Some(id.to_string()),
            Self::ProfileId(id) => Some(id.to_string()),
            Self::Json(json) => Some(json.to_string()),
            Self::Resource(_) | Self::SecretRef(_) => None,
        }
    }

    /// The parameter value for a parameter of `kind`, keeping the native
    /// type where the parameter has one. Constraints of the parameter
    /// (ranges, lengths, choice options) are checked afterwards by
    /// [`crate::ActionType::normalize`], as for literal values.
    pub fn to_parameter(&self, kind: &ParameterKind) -> Option<ParameterValue> {
        if !self.value_type().assignable_to(kind) {
            return None;
        }
        Some(match (self, kind) {
            (Self::Integer(number), ParameterKind::Integer { .. }) => {
                ParameterValue::Integer(*number)
            }
            (Self::Boolean(flag), ParameterKind::Boolean) => ParameterValue::Boolean(*flag),
            (Self::ProjectId(id), ParameterKind::Project) => ParameterValue::Project(*id),
            (Self::Resource(reference), ParameterKind::Resource { .. }) => {
                ParameterValue::Resource(reference.clone())
            }
            (Self::String(text), ParameterKind::Choice { .. }) => {
                ParameterValue::Choice(text.clone())
            }
            (Self::SecretRef(id), ParameterKind::Credential) => {
                ParameterValue::Text(id.as_str().to_owned())
            }
            (value, _) => ParameterValue::Text(value.render_text()?),
        })
    }
}

/// Where a shared variable's value is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableScope {
    /// This DeckPilot installation, for every profile.
    Global,
    /// One profile.
    Profile,
    /// One project of a profile.
    Project,
    /// One workflow run; never stored.
    Execution,
}

impl VariableScope {
    /// Most specific first: the first scope with a value wins.
    pub const PRECEDENCE: [Self; 4] = [Self::Execution, Self::Project, Self::Profile, Self::Global];

    pub fn name(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Profile => "profile",
            Self::Project => "project",
            Self::Execution => "execution",
        }
    }

    /// Position in [`VariableScope::PRECEDENCE`]; lower wins.
    pub fn rank(self) -> usize {
        Self::PRECEDENCE
            .iter()
            .position(|scope| *scope == self)
            .unwrap_or(usize::MAX)
    }
}

/// One concrete place a value is set: a scope and what it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScopeKey {
    Global,
    Profile {
        profile: ProfileId,
    },
    Project {
        profile: ProfileId,
        project: ProjectId,
    },
    Execution {
        execution: ExecutionId,
    },
}

impl ScopeKey {
    pub fn scope(&self) -> VariableScope {
        match self {
            Self::Global => VariableScope::Global,
            Self::Profile { .. } => VariableScope::Profile,
            Self::Project { .. } => VariableScope::Project,
            Self::Execution { .. } => VariableScope::Execution,
        }
    }

    /// Whether a value set here is visible in `context`. A project value is
    /// visible only for that project of that profile, so a value never
    /// crosses profiles.
    pub fn applies_to(&self, context: &ResolutionContext) -> bool {
        match self {
            Self::Global => true,
            Self::Profile { profile } => *profile == context.profile,
            Self::Project { profile, project } => {
                *profile == context.profile && context.project == Some(*project)
            }
            Self::Execution { execution } => context.execution == Some(*execution),
        }
    }
}

/// Namespaces a shared variable cannot use: the other roots of the
/// reference grammar, so that `shared.steps.…` is never mistaken for a node
/// output. `shared` itself needs no reservation: it is always the first
/// segment of a reference and never part of the key, so `shared.shared.x`
/// is the variable `shared.x`. Keys a module declares are reserved by the
/// variable store once modules can declare variables.
pub const RESERVED_NAMESPACES: &[&str] = &["steps", "system", "workflow"];
/// Longest key segment and number of segments of a [`VariableKey`].
pub const MAX_KEY_SEGMENT: usize = 32;
pub const MAX_KEY_SEGMENTS: usize = 4;

/// The key of a shared variable without the `shared.` root, e.g.
/// `projects.favorite`: a namespace and a name, optionally followed by up
/// to two more segments. Each segment is 1–32 ASCII letters, digits and
/// `_`, starting with a lowercase letter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VariableKey(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyError {
    #[error(
        "`{0}` is not a variable key: use namespace.name with letters, digits and `_`, \
         each part starting with a lowercase letter"
    )]
    Invalid(String),
    #[error("the namespace `{0}` is reserved")]
    Reserved(String),
}

impl VariableKey {
    pub fn new(key: impl Into<String>) -> Result<Self, KeyError> {
        let key = key.into();
        let segments: Vec<&str> = key.split('.').collect();
        if !(2..=MAX_KEY_SEGMENTS).contains(&segments.len())
            || !segments.iter().all(|segment| valid_segment(segment))
        {
            return Err(KeyError::Invalid(key));
        }
        if RESERVED_NAMESPACES.contains(&segments[0]) {
            return Err(KeyError::Reserved(segments[0].to_owned()));
        }
        Ok(Self(key))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first segment, e.g. `projects`.
    pub fn namespace(&self) -> &str {
        self.0.split('.').next().unwrap_or_default()
    }
}

pub(crate) fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= MAX_KEY_SEGMENT
        && segment.starts_with(|c: char| c.is_ascii_lowercase())
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl TryFrom<String> for VariableKey {
    type Error = KeyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<VariableKey> for String {
    fn from(value: VariableKey) -> Self {
        value.0
    }
}

impl fmt::Display for VariableKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A shared variable: what it holds and where it may be set. Values are
/// literals of [`VariableDefinition::value_type`]; they are never expanded
/// again, so a value that looks like a reference is plain text and
/// references cannot form cycles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableDefinition {
    pub key: VariableKey,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub value_type: VariableType,
    /// Scopes at which a value may be set. Never empty.
    pub scopes: BTreeSet<VariableScope>,
    /// Used when no scope in the context has a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<VariableValue>,
    /// The value is shown only as redacted: in pickers, previews, logs,
    /// history and exports.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    /// The module that declares the variable; `None` for user variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<ModuleId>,
}

/// What is wrong with a [`VariableDefinition`] or a value for it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DefinitionIssue {
    #[error("`{0}` has no scope where a value can be set")]
    NoScopes(VariableKey),
    #[error("`{key}` holds {expected} values, not {found}")]
    WrongType {
        key: VariableKey,
        expected: &'static str,
        found: &'static str,
    },
    #[error("`{key}` cannot be set at {scope} scope")]
    ScopeNotAllowed {
        key: VariableKey,
        scope: &'static str,
    },
}

impl VariableDefinition {
    pub fn new(key: VariableKey, value_type: VariableType, scopes: &[VariableScope]) -> Self {
        Self {
            key,
            label: String::new(),
            description: String::new(),
            value_type,
            scopes: scopes.iter().copied().collect(),
            default: None,
            sensitive: false,
            module: None,
        }
    }

    /// Every problem of the definition itself.
    pub fn issues(&self) -> Vec<DefinitionIssue> {
        let mut issues = Vec::new();
        if self.scopes.is_empty() {
            issues.push(DefinitionIssue::NoScopes(self.key.clone()));
        }
        if let Some(default) = &self.default
            && let Err(issue) = self.check_type(default)
        {
            issues.push(issue);
        }
        issues
    }

    /// Checks a value about to be set at `scope`.
    pub fn check_value(
        &self,
        scope: &ScopeKey,
        value: &VariableValue,
    ) -> Result<(), DefinitionIssue> {
        if !self.scopes.contains(&scope.scope()) {
            return Err(DefinitionIssue::ScopeNotAllowed {
                key: self.key.clone(),
                scope: scope.scope().name(),
            });
        }
        self.check_type(value)
    }

    fn check_type(&self, value: &VariableValue) -> Result<(), DefinitionIssue> {
        if value.is_of(self.value_type) {
            Ok(())
        } else {
            Err(DefinitionIssue::WrongType {
                key: self.key.clone(),
                expected: self.value_type.name(),
                found: value.value_type().name(),
            })
        }
    }
}

#[cfg(test)]
mod tests;
