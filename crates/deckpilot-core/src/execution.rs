//! Execution context handed to action handlers and the results they return.

use serde::{Deserialize, Serialize};

use crate::{
    CycleDirection, MachineConfig, MachineResourceBinding, Profile, Project, ProjectId,
    ResolveError, ResourceKind, ResourceRef,
};

/// Read-only view of everything an action may depend on at execution time.
///
/// Handlers never mutate state directly. Changes to the runtime context are
/// requested through [`ContextEffect`]s in the returned [`ActionResult`], which
/// keeps handlers deterministic and easy to test.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionContext<'a> {
    pub profile: &'a Profile,
    pub project: Option<&'a Project>,
    pub machine: &'a MachineConfig,
}

impl<'a> ExecutionContext<'a> {
    /// Resolves a resource reference for the active project on this machine.
    pub fn resolve(
        &self,
        reference: &ResourceRef,
        expected: Option<ResourceKind>,
    ) -> Result<&'a MachineResourceBinding, ResolveError> {
        self.machine.resolve(reference, self.project, expected)
    }
}

/// A change to the runtime context requested by an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContextEffect {
    CycleProject { direction: CycleDirection },
    SelectProject { project: ProjectId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionStatus {
    Succeeded,
    Failed,
    /// Not executed because a workflow step condition was not met.
    Skipped,
}

/// Outcome of executing a single action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub status: ActionStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ContextEffect>,
}

impl ActionResult {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            status: ActionStatus::Succeeded,
            message: message.into(),
            effects: Vec::new(),
        }
    }

    pub fn skipped(message: impl Into<String>) -> Self {
        Self {
            status: ActionStatus::Skipped,
            message: message.into(),
            effects: Vec::new(),
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            status: ActionStatus::Failed,
            message: message.into(),
            effects: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_effect(mut self, effect: ContextEffect) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn is_success(&self) -> bool {
        self.status == ActionStatus::Succeeded
    }
}
