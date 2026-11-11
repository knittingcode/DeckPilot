//! Execution context handed to action handlers and the results they return.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    Capability, MachineConfig, MachineResourceBinding, Profile, Project, ProjectId, ResolveError,
    ResourceKind, ResourceRef,
};

/// Read-only view of everything an action may depend on at execution time.
///
/// Handlers never mutate state directly. Changes to the runtime context are
/// requested through [`ContextEffect`]s in the returned [`ActionResult`], which
/// keeps handlers deterministic and easy to test.
///
/// Access is limited by the [`Capability`]s of the module that provides the
/// handler: [`ExecutionContext::profile`] and [`ExecutionContext::project`]
/// require [`Capability::ProfileRead`], and [`ExecutionContext::resolve`]
/// requires [`Capability::MachineResources`]. The
/// [`ActionRegistry`](crate::ActionRegistry) applies the limits before it calls
/// a module's handler.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionContext<'a> {
    profile: &'a Profile,
    project: Option<&'a Project>,
    machine: &'a MachineConfig,
    /// `None` means unrestricted (the core itself and trusted handlers).
    capabilities: Option<&'a BTreeSet<Capability>>,
    /// Cancellation and progress, when the action runs in a workflow.
    control: Option<&'a ExecutionControl<'a>>,
}

/// Progress a running action reports, shown on its workflow node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// 0–100, when the action knows how far it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    /// Short and safe to show: no secrets, no raw output.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

/// What the engine hands an action that runs as a workflow node: the token
/// that is cancelled when the run is cancelled or the attempt's timeout
/// passes, and a way to report progress.
pub struct ExecutionControl<'a> {
    cancel: &'a crate::CancellationToken,
    progress: &'a (dyn Fn(Progress) + Sync),
}

impl<'a> ExecutionControl<'a> {
    pub fn new(
        cancel: &'a crate::CancellationToken,
        progress: &'a (dyn Fn(Progress) + Sync),
    ) -> Self {
        Self { cancel, progress }
    }
}

impl fmt::Debug for ExecutionControl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionControl")
            .field("cancel", self.cancel)
            .finish_non_exhaustive()
    }
}

impl<'a> ExecutionContext<'a> {
    /// An unrestricted context. Restrictions are added when dispatching.
    pub fn new(
        profile: &'a Profile,
        project: Option<&'a Project>,
        machine: &'a MachineConfig,
    ) -> Self {
        Self {
            profile,
            project,
            machine,
            capabilities: None,
            control: None,
        }
    }

    /// The same context with cancellation and progress for a workflow node.
    #[must_use]
    pub fn with_control(self, control: &'a ExecutionControl<'a>) -> Self {
        Self {
            control: Some(control),
            ..self
        }
    }

    /// The token of the running attempt: cancelled when the run is
    /// cancelled or the attempt's timeout passes. `None` outside workflows
    /// (a single action started from a key or the app).
    pub fn cancellation(&self) -> Option<&'a crate::CancellationToken> {
        self.control.map(|control| control.cancel)
    }

    /// Whether the action should stop as soon as it can.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation()
            .is_some_and(crate::CancellationToken::is_cancelled)
    }

    /// Sleeps `duration`, ending early when cancelled; `false` if cancelled.
    /// Actions wait with this instead of `std::thread::sleep`.
    pub fn wait(&self, duration: std::time::Duration) -> bool {
        if let Some(token) = self.cancellation() {
            return token.wait(duration);
        }
        std::thread::sleep(duration);
        true
    }

    /// Reports progress of a long-running action. Ignored outside workflows.
    pub fn report_progress(&self, percent: Option<u8>, message: impl Into<String>) {
        if let Some(control) = self.control {
            (control.progress)(Progress {
                percent: percent.map(|p| p.min(100)),
                message: message.into(),
            });
        }
    }

    /// The same context, limited to `capabilities`.
    #[must_use]
    pub fn restricted(self, capabilities: &'a BTreeSet<Capability>) -> Self {
        Self {
            capabilities: Some(capabilities),
            ..self
        }
    }

    pub fn allows(&self, capability: Capability) -> bool {
        self.capabilities
            .is_none_or(|granted| granted.contains(&capability))
    }

    /// Checks that the module may use `capability`. Modules call this
    /// before doing what the capability covers (opening a page, sending a
    /// request); the core's own helpers such as
    /// [`ExecutionContext::run_process`] check it themselves.
    pub fn permit(&self, capability: Capability) -> Result<(), CapabilityDenied> {
        self.require(capability)
    }

    /// Runs a program to completion with the module's
    /// [`Capability::ProcessExecution`], stopping it when the run is
    /// cancelled, when the node's timeout passes or after `timeout`,
    /// whichever comes first. Output is captured up to `limits`.
    pub fn run_process(
        &self,
        command: std::process::Command,
        timeout: std::time::Duration,
        limits: crate::process::ProcessLimits,
    ) -> Result<crate::process::ProcessOutput, crate::process::ProcessError> {
        self.require(Capability::ProcessExecution)
            .map_err(crate::process::ProcessError::NotPermitted)?;
        let deadline = std::time::Instant::now() + timeout;
        let token = match self.cancellation() {
            Some(token) => token.child_with_deadline(deadline),
            None => crate::CancellationToken::new().child_with_deadline(deadline),
        };
        crate::process::run(command, Some(&token), limits)
    }

    fn require(&self, capability: Capability) -> Result<(), CapabilityDenied> {
        if self.allows(capability) {
            Ok(())
        } else {
            Err(CapabilityDenied(capability))
        }
    }

    /// The profile the action belongs to.
    pub fn profile(&self) -> Result<&'a Profile, CapabilityDenied> {
        self.require(Capability::ProfileRead)?;
        Ok(self.profile)
    }

    /// The active project of the profile, if any.
    pub fn project(&self) -> Result<Option<&'a Project>, CapabilityDenied> {
        self.require(Capability::ProfileRead)?;
        Ok(self.project)
    }

    /// Resolves a resource reference for the active project on this machine.
    pub fn resolve(
        &self,
        reference: &ResourceRef,
        expected: Option<ResourceKind>,
    ) -> Result<&'a MachineResourceBinding, ResolveError> {
        self.require(Capability::MachineResources)
            .map_err(|_| ResolveError::NotPermitted)?;
        self.machine.resolve(reference, self.project, expected)
    }

    /// Resolves the resource that `project` maps to `role`, on this machine.
    /// `project` need not be the active one; callers check that it belongs
    /// to the profile.
    pub fn resolve_project_resource(
        &self,
        project: &Project,
        role: &str,
    ) -> Result<&'a MachineResourceBinding, ResolveError> {
        self.require(Capability::MachineResources)
            .map_err(|_| ResolveError::NotPermitted)?;
        let reference = ResourceRef::ProjectRole {
            role: role.to_owned(),
        };
        self.machine.resolve(&reference, Some(project), None)
    }
}

/// A module tried to use something its manifest does not declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the module is not allowed to {}", .0.description().to_lowercase())]
pub struct CapabilityDenied(pub Capability);

impl From<CapabilityDenied> for ActionResult {
    fn from(denied: CapabilityDenied) -> Self {
        ActionResult::error("not-permitted", denied.to_string())
    }
}

/// A change to the runtime context requested by an action. The engine
/// applies it after checking that the requesting module holds the matching
/// [`Capability`](crate::Capability) and that the target belongs to the
/// active profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ContextEffect {
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
    /// Structured details of the outcome, e.g. the resolved project and
    /// resource. Keys are camelCase; values are plain text.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub data: BTreeMap<String, String>,
    /// Why a failed action failed, as a stable code from the module's
    /// documented list, e.g. `exit-nonzero` or `resource-unbound`. Shown
    /// with the message and available to workflow conditions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ActionResult {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            status: ActionStatus::Succeeded,
            message: message.into(),
            effects: Vec::new(),
            data: BTreeMap::new(),
            code: None,
        }
    }

    pub fn skipped(message: impl Into<String>) -> Self {
        Self {
            status: ActionStatus::Skipped,
            message: message.into(),
            effects: Vec::new(),
            data: BTreeMap::new(),
            code: None,
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            status: ActionStatus::Failed,
            message: message.into(),
            effects: Vec::new(),
            data: BTreeMap::new(),
            code: None,
        }
    }

    /// A failure with a stable error code.
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: Some(code.into()),
            ..Self::failure(message)
        }
    }

    #[must_use]
    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.insert(key.into(), value.into());
        self
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
