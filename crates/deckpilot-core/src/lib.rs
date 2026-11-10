//! Core domain model of DeckPilot.
//!
//! The model is *action-first*: users configure [`ActionInstance`]s and
//! [`Workflow`]s inside a [`Profile`] and only afterwards bind them to device
//! controls through a [`DeviceLayout`]. Nothing in an action refers to a
//! physical button, an operating system, or an absolute path. Machine-specific
//! values are resolved at execution time through [`MachineConfig`].

pub mod action;
pub mod device_runtime;
pub mod execution;
pub mod face;
pub mod id;
pub mod layout;
pub mod module;
pub mod platform;
pub mod profile;
pub mod project;
pub mod registry;
pub mod resource;
pub mod runtime;
pub mod workflow;

#[cfg(test)]
pub(crate) mod testing;

pub use action::{
    ActionInstance, ActionParameters, ActionType, ActionTypeId, MAX_LABEL_CHARS, ParameterError,
    ParameterKind, ParameterSpec, ParameterValue, validate_presentation,
};
pub use device_runtime::{LayoutStatus, Runtime, RuntimeOutcome, page_faces};
pub use execution::{ActionResult, ActionStatus, ContextEffect, ExecutionContext};
pub use face::{FaceStyle, KeyFace, render_label};
pub use id::{ActionInstanceId, LayoutId, PageId, ProfileId, ProjectId, WorkflowId};
pub use layout::{BindingTarget, ControlBinding, DeviceLayout, LayoutIssue, LayoutPage, Trigger};
pub use module::{
    Availability, Capability, FeatureModule, ModuleError, ModuleId, ModuleInfo, ModuleManifest,
    ModulePage, ModuleRegistry, ModuleSettings, ModuleStates,
};
pub use platform::Os;
pub use profile::{Profile, ProfileIssue, Usage};
pub use project::{
    ActiveProjectContext, CycleDirection, Project, ProjectContextError, ProjectKey, ProjectSwitcher,
};
pub use registry::{ActionHandler, ActionRegistry, RegistryError};
pub use resource::{
    MachineConfig, MachineResourceBinding, ResolveError, ResourceKey, ResourceKind, ResourceRef,
};
pub use runtime::{Engine, EngineError, StepReport, WorkflowReport};
pub use workflow::{OnFailure, StepCondition, Workflow, WorkflowStep};

/// Error returned when a textual identifier does not satisfy its format rules.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} `{value}`: {reason}")]
pub struct IdentifierError {
    pub kind: &'static str,
    pub value: String,
    pub reason: &'static str,
}
