//! Core domain model of DeckPilot.
//!
//! The model is *action-first*: users configure [`ActionInstance`]s and
//! [`Workflow`]s inside a [`Profile`] and only afterwards bind them to device
//! controls through a [`DeviceLayout`]. Nothing in an action refers to a
//! physical button, an operating system, or an absolute path. Machine-specific
//! values are resolved at execution time through [`MachineConfig`].

pub mod action;
pub mod alias;
pub mod appearance;
pub mod assets;
pub mod button_state;
pub mod cancel;
pub mod controls;
pub mod credentials;
pub mod device_runtime;
pub mod editor_navigation;
pub mod execution;
pub mod face;
pub mod folder;
pub mod folder_control;
pub mod gesture;
pub mod graph;
pub mod history;
pub mod i18n;
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
pub mod resolution;
pub mod resource;
pub mod runtime;
pub mod shared_layouts;
pub mod stateful;
pub mod trigger;
pub mod variables;
pub mod work_session;
pub mod workflow;

#[cfg(test)]
pub(crate) mod testing;

pub use action::{
    ActionInstance, ActionParameters, ActionType, ActionTypeId, MAX_LABEL_CHARS, ParameterError,
    ParameterKind, ParameterSpec, ParameterValue, lines_of, looks_secret, validate_presentation,
};
pub use alias::{
    ACTION_ALIASES, ActionAlias, AliasOutcome, AliasStatus, FIRST_PARTY_PUBLISHER, LegacyRoute,
    LegacyRouting, ParamRule, RESERVED_MODULE_IDS,
};
pub use appearance::{
    ActionTitle, Color, DEFAULT_MARQUEE_PAUSE_MS, DEFAULT_MARQUEE_SPEED, DEFAULT_MIN_TEXT_POINTS,
    DEFAULT_TITLE_FONT, DefaultAppearance, FitLines, FontFamilyId, FontSize, Icon, IconData,
    IconFit, IconRef, KeyAppearance, KeyBackground, MARQUEE_SPEED, MAX_ICON_BYTES, MAX_ICON_NAME,
    MAX_ICON_SIZE, MAX_ICONS, MAX_MARQUEE_PAUSE_MS, MAX_TEXT_LINES, TEXT_SIZE_POINTS, TITLE_FONTS,
    TextAlign, TextAppearance, TextMode, TextOverflow, TitleAlign, TitleFeature, TitleFont,
};
pub use assets::{
    AssetId, AssetRef, BACKGROUND_ASSET_PREFIX, BUILTIN_GRADIENTS, BUILTIN_NAMESPACE, Background,
    BuiltinGradient, Gradient, MAX_GRADIENT_COLORS, builtin_gradient,
};
pub use button_state::{
    Accent, ButtonState, Glyph, Indicator, MAX_REFRESH_INTERVAL, MAX_STATE_BADGE, MAX_STATE_SERIES,
    MIN_REFRESH_INTERVAL, RefreshPolicy, StateError, StateSnapshot,
};
pub use cancel::{CancelReason, CancellationToken};
pub use controls::{
    ActionControls, Compatibility, IncompatibilityReason, KEY_INPUTS, MAX_INPUTS_PER_KIND,
    STEP_ENCODER_INPUTS, compatibility,
};
pub use credentials::{
    CredentialError, CredentialId, CredentialInfo, CredentialSource, MemoryCredentials,
    NoCredentials, Secret,
};
pub use device_runtime::{
    DetachedAction, EditorTarget, LayoutStatus, NavigationError, NavigationState, Runtime,
    RuntimeOutcome, page_faces,
};
pub use editor_navigation::{
    ControlNavigation, EditorNavigation, NavigationOperation, control_navigation,
};
pub use execution::{
    ActionResult, ActionStatus, CapabilityDenied, ContextEffect, DeckRequest, ExecutionContext,
    ExecutionControl, ProfileTarget, Progress, RunOrigin, RunSummary, WorkflowCause,
};
pub use face::{FaceStyle, KeyFace, PageLabel, render_label, render_page_label};
pub use folder::{
    AutoExit, AutoExitError, BackPosition, FolderBack, FolderBackError, FolderBackReport,
    FolderExit, FolderTimers, reserved_back_control,
};
pub use folder_control::{
    FOLDER_CONTROL_TRIGGER, NormalizedFolderControl, NormalizedFolderLabel, RecoveredBindings,
    RecoveredError, RecoveryReason,
};
pub use history::{
    ExecutionRecord, ExecutionState, ExecutionSummary, ExecutionTrigger, ProjectOutcome,
    safe_event, safe_report,
};
pub use id::{
    ActionInstanceId, ExecutionId, IconId, LayoutId, PageId, ProfileId, ProjectId, VerificationId,
    WorkSessionId, WorkflowId,
};
pub use layout::{
    BindingTarget, ControlBinding, DeviceLayout, IncompatibleBinding, LayoutIssue, LayoutPage,
    Trigger, binding_incompatibility,
};
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
pub use registry::{ActionHandler, ActionRegistry, AliasHandler, LayoutNavigation, RegistryError};
pub use resource::{
    MachineConfig, MachineResourceBinding, ResolveError, ResourceKey, ResourceKind, ResourceRef,
};
pub use runtime::{Engine, EngineError, StepReport, WorkflowReport, declared_outputs};
pub use shared_layouts::{
    HistoryMode, LayoutCompatibility, LayoutHistory, LayoutRequirements, ModelFit, Unavailable,
};
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
