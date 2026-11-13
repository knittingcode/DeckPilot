//! What the variable picker lists: metadata of every reference available
//! in a context. Entries never carry values; values come from previews.

use serde::{Deserialize, Serialize};

use super::registry::{Deprecation, VariableOwner};
use super::{VariableRef, VariableScope, VariableType};
use crate::{ModuleId, ParameterKind};

/// The picker's groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableGroup {
    /// Shared variables with a global value or default.
    Shared,
    /// System variables (`system.…`).
    System,
    /// Shared variables set for the context's profile.
    Profile,
    /// Shared variables set for the context's project.
    Project,
    /// The running workflow: `workflow.…`, its inputs and variables.
    Workflow,
    /// Outputs of earlier nodes.
    StepOutput,
    /// Variables a module declares (since protocol 9).
    Module,
}

/// Whether an entry can have a value in the context it was listed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReferenceAvailability {
    Available,
    /// Has a value only while the workflow runs.
    RuntimeOnly,
    /// Not in this context, e.g. a project variable without a project.
    NotInContext,
    /// Its owner cannot provide it: the module is disabled, unsupported or
    /// not installed (since protocol 9; older clients see `notInContext`).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub reference: VariableRef,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub group: VariableGroup,
    pub value_type: VariableType,
    /// For shared variables: the scope whose value applies in the context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<VariableScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<ModuleId>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    /// The value depends on the context or changes while running.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dynamic: bool,
    pub availability: ReferenceAvailability,
    /// Who owns the variable (since protocol 9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<VariableOwner>,
    /// Core and module variables cannot be changed (since protocol 9).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// An older id kept as an alias; the picker inserts the canonical id
    /// instead (since protocol 9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<Deprecation>,
}

impl CatalogEntry {
    /// The entry as a client before protocol 9 knows it, or `None` if such
    /// a client could not read it (a module root).
    pub fn for_protocol_8(self) -> Option<Self> {
        if matches!(self.reference, VariableRef::Module(_)) || self.group == VariableGroup::Module {
            return None;
        }
        Some(Self {
            availability: match self.availability {
                ReferenceAvailability::Unavailable => ReferenceAvailability::NotInContext,
                other => other,
            },
            owner: None,
            read_only: false,
            deprecated: None,
            ..self
        })
    }
}

/// What the picker asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogFilter {
    /// Words that must all occur in the reference, label, description,
    /// scope, type or module, ignoring case.
    pub query: String,
    /// Empty: every group.
    pub groups: Vec<VariableGroup>,
    /// Only entries whose type can fill a parameter of this kind.
    pub compatible_with: Option<ParameterKind>,
    /// Also entries that are [`ReferenceAvailability::NotInContext`].
    pub include_unavailable: bool,
}

impl CatalogFilter {
    pub fn matches(&self, entry: &CatalogEntry) -> bool {
        if !self.groups.is_empty() && !self.groups.contains(&entry.group) {
            return false;
        }
        if let Some(kind) = &self.compatible_with
            && !entry.value_type.assignable_to(kind)
        {
            return false;
        }
        if !self.include_unavailable
            && matches!(
                entry.availability,
                ReferenceAvailability::NotInContext | ReferenceAvailability::Unavailable
            )
        {
            return false;
        }
        let haystack = [
            entry.reference.to_string(),
            entry.label.clone(),
            entry.description.clone(),
            entry.scope.map(|s| s.name().to_owned()).unwrap_or_default(),
            entry.value_type.name().to_owned(),
            entry
                .module
                .as_ref()
                .or_else(|| entry.owner.as_ref().and_then(VariableOwner::module))
                .map(ToString::to_string)
                .unwrap_or_default(),
            entry
                .deprecated
                .as_ref()
                .map(|d| d.canonical.to_string())
                .unwrap_or_default(),
        ]
        .join("\n")
        .to_lowercase();
        self.query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }
}
