//! Core domain model of DeckPilot.
//!
//! The model is *action-first*: users configure [`ActionInstance`]s and
//! [`Workflow`]s inside a [`Profile`] and only afterwards bind them to device
//! controls through a [`DeviceLayout`]. Nothing in an action refers to a
//! physical button, an operating system, or an absolute path. Machine-specific
//! values are resolved at execution time through [`MachineConfig`].

pub mod action;
pub mod appearance;
pub mod button_state;
pub mod cancel;
pub mod credentials;
pub mod device_runtime;
pub mod execution;
pub mod face;
pub mod graph;
pub mod history;
pub mod id;
pub mod launch;
pub mod layout;
pub mod module;
pub mod notify;
pub mod pages;
pub mod platform;
pub mod process;
pub mod profile;
pub mod project;
pub mod registry;
pub mod resource;
pub mod runtime;
pub mod trigger;
pub mod workflow;

#[cfg(test)]
pub(crate) mod testing;

pub use action::{
    ActionInstance, ActionParameters, ActionType, ActionTypeId, MAX_LABEL_CHARS, ParameterError,
    ParameterKind, ParameterSpec, ParameterValue, lines_of, looks_secret, validate_presentation,
};
pub use appearance::{
    Color, FontSize, Icon, IconData, IconFit, IconRef, KeyAppearance, MAX_ICON_BYTES,
    MAX_ICON_NAME, MAX_ICON_SIZE, MAX_ICONS, TitleAlign,
};
pub use button_state::{
    ButtonState, Indicator, MAX_REFRESH_INTERVAL, MIN_REFRESH_INTERVAL, RefreshPolicy, StateError,
    StateSnapshot,
};
pub use cancel::{CancelReason, CancellationToken};
pub use credentials::{
    CredentialError, CredentialId, CredentialInfo, CredentialSource, MemoryCredentials,
    NoCredentials, Secret,
};
pub use device_runtime::{LayoutStatus, Runtime, RuntimeOutcome, page_faces};
pub use execution::{
    ActionResult, ActionStatus, CapabilityDenied, ContextEffect, ExecutionContext,
    ExecutionControl, Progress, RunSummary,
};
pub use face::{FaceStyle, KeyFace, render_label};
pub use history::{
    ExecutionRecord, ExecutionState, ExecutionSummary, ExecutionTrigger, ProjectOutcome,
    safe_event, safe_report,
};
pub use id::{
    ActionInstanceId, ExecutionId, IconId, LayoutId, PageId, ProfileId, ProjectId, WorkflowId,
};
pub use layout::{BindingTarget, ControlBinding, DeviceLayout, LayoutIssue, LayoutPage, Trigger};
pub use module::{
    Availability, Capability, FeatureModule, ModuleError, ModuleId, ModuleInfo, ModuleManifest,
    ModuleMigration, ModulePage, ModuleRegistry, ModuleSettings, ModuleStates, ModuleWindow,
    Portability, SettingScope, WindowSize,
};
pub use pages::PageDestination;
pub use platform::Os;
pub use profile::{Profile, ProfileIssue, Usage};
pub use project::{
    ActiveProjectContext, CycleDirection, Project, ProjectContextError, ProjectKey,
    ProjectStepError, ProjectSwitcher,
};
pub use registry::{ActionHandler, ActionRegistry, RegistryError};
pub use resource::{
    MachineConfig, MachineResourceBinding, ResolveError, ResourceKey, ResourceKind, ResourceRef,
};
pub use runtime::{Engine, EngineError, StepReport, WorkflowReport};
pub use trigger::{
    EventField, LifecycleEvent, Schedule, Scheduler, TriggerEvent, TriggerKind, TriggerValue,
    WallTime, WorkflowTrigger,
};
pub use workflow::{OnFailure, ProjectScope, StepCondition, Workflow, WorkflowStep};

/// Error returned when a textual identifier does not satisfy its format rules.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} `{value}`: {reason}")]
pub struct IdentifierError {
    pub kind: &'static str,
    pub value: String,
    pub reason: &'static str,
}
