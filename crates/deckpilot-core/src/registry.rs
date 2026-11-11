//! Registry of executable action types and their handlers.
//!
//! The registry used at run time is built by the
//! [`ModuleRegistry`](crate::ModuleRegistry) from the enabled feature
//! modules. The core never refers to a specific action type; it dispatches
//! through this registry.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use thiserror::Error;

use crate::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionResult, ActionType, ActionTypeId,
    ButtonState, Capability, ContextEffect, ExecutionContext, KeyFace, ModuleId, ParameterError,
    Profile, Project, RefreshPolicy, StateError,
};

/// Implementation of one action type.
pub trait ActionHandler: Send + Sync {
    /// Static definition of the action type handled by this implementation.
    fn definition(&self) -> &ActionType;

    /// Executes the action. `parameters` have already been validated and
    /// normalized against [`ActionHandler::definition`].
    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult;

    /// Checks normalized parameters against the profile the action belongs
    /// to, for example that a referenced project exists in that profile.
    fn check(&self, _parameters: &ActionParameters, _profile: &Profile) -> Vec<String> {
        Vec::new()
    }

    /// Adjusts the key face of a binding that runs this action, e.g. to show
    /// the active project. `template` is the label template in use.
    fn decorate_face(
        &self,
        face: KeyFace,
        _parameters: &ActionParameters,
        _template: &str,
        _project: Option<&Project>,
    ) -> KeyFace {
        face
    }

    /// Whether keys running this action show a live [`ButtonState`], and
    /// how often it may be refreshed. `None` (the default): no live state,
    /// and [`ActionHandler::button_state`] is never called.
    fn state_policy(&self, _parameters: &ActionParameters) -> Option<RefreshPolicy> {
        None
    }

    /// The live state of a key that runs this action with `parameters`
    /// (normalized). Called by the agent off the device loop, at most as
    /// often as [`ActionHandler::state_policy`] allows, with a context
    /// limited to the module's capabilities and a cancellation token
    /// (cancelled on reload, shutdown and after a time limit), so programs
    /// must be run with [`ExecutionContext::run_process`]. Must not change
    /// anything; failures are shown as an error mark on the key.
    fn button_state(
        &self,
        _parameters: &ActionParameters,
        _context: &ExecutionContext<'_>,
    ) -> Result<ButtonState, StateError> {
        Ok(ButtonState::default())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
    #[error("action type `{0}` is already registered")]
    Duplicate(ActionTypeId),
    #[error("unknown action type `{0}`")]
    UnknownActionType(ActionTypeId),
    #[error("action type `{0}` is unavailable: {1}")]
    Unavailable(ActionTypeId, String),
    #[error("action name must not be empty")]
    EmptyName,
    #[error("invalid parameters: {}", join(.0))]
    InvalidParameters(Vec<ParameterError>),
    #[error("{0}")]
    InvalidPresentation(String),
}

fn join(errors: &[ParameterError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

struct Registered {
    handler: Box<dyn ActionHandler>,
    /// `None` for handlers registered directly (trusted, e.g. in tests).
    module: Option<ModuleId>,
    capabilities: Option<BTreeSet<Capability>>,
}

#[derive(Default)]
pub struct ActionRegistry {
    handlers: BTreeMap<ActionTypeId, Registered>,
    /// Known action types that cannot run, with the reason.
    unavailable: BTreeMap<ActionTypeId, String>,
}

impl fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionRegistry")
            .field("types", &self.handlers.keys())
            .field("unavailable", &self.unavailable.keys())
            .finish()
    }
}

/// The capability an effect requires.
fn required_capability(effect: &ContextEffect) -> Capability {
    match effect {
        ContextEffect::SelectProject { .. } => Capability::ProjectContext,
    }
}

impl ActionRegistry {
    /// Registers a trusted handler that is not part of a module and may use
    /// every capability.
    pub fn register(&mut self, handler: Box<dyn ActionHandler>) -> Result<(), RegistryError> {
        self.insert(handler, None, None)
    }

    /// Registers a handler of `module`, limited to `capabilities`.
    pub fn register_for_module(
        &mut self,
        module: ModuleId,
        capabilities: &[Capability],
        handler: Box<dyn ActionHandler>,
    ) -> Result<(), RegistryError> {
        let capabilities = capabilities.iter().copied().collect();
        self.insert(handler, Some(module), Some(capabilities))
    }

    fn insert(
        &mut self,
        handler: Box<dyn ActionHandler>,
        module: Option<ModuleId>,
        capabilities: Option<BTreeSet<Capability>>,
    ) -> Result<(), RegistryError> {
        let id = handler.definition().id.clone();
        if self.handlers.contains_key(&id) {
            return Err(RegistryError::Duplicate(id));
        }
        self.unavailable.remove(&id);
        self.handlers.insert(
            id,
            Registered {
                handler,
                module,
                capabilities,
            },
        );
        Ok(())
    }

    /// Records action types that are known but cannot run.
    pub fn mark_unavailable(
        &mut self,
        types: impl IntoIterator<Item = ActionTypeId>,
        reason: impl Into<String>,
    ) {
        let reason = reason.into();
        for id in types {
            if !self.handlers.contains_key(&id) {
                self.unavailable.insert(id, reason.clone());
            }
        }
    }

    /// Handlers are only reachable inside the core, so that every call goes
    /// through the capability checks of [`ActionRegistry::dispatch`],
    /// [`ActionRegistry::check_parameters`] and
    /// [`ActionRegistry::decorate_face`].
    pub(crate) fn handler(&self, id: &ActionTypeId) -> Option<&dyn ActionHandler> {
        self.handlers.get(id).map(|entry| entry.handler.as_ref())
    }

    /// The definition of an executable action type.
    pub fn definition(&self, id: &ActionTypeId) -> Option<&ActionType> {
        self.handler(id).map(ActionHandler::definition)
    }

    fn allows(&self, id: &ActionTypeId, capability: Capability) -> bool {
        self.handlers.get(id).is_some_and(|entry| {
            entry
                .capabilities
                .as_ref()
                .is_none_or(|granted| granted.contains(&capability))
        })
    }

    /// Checks normalized parameters against `profile`. Handlers without
    /// [`Capability::ProfileRead`] do not see the profile and report nothing.
    pub fn check_parameters(
        &self,
        action_type: &ActionTypeId,
        parameters: &ActionParameters,
        profile: &Profile,
    ) -> Vec<String> {
        match self.handler(action_type) {
            Some(handler) if self.allows(action_type, Capability::ProfileRead) => {
                handler.check(parameters, profile)
            }
            _ => Vec::new(),
        }
    }

    /// The key face for a binding that runs `action_type`, decorated by its
    /// module. Modules without [`Capability::ProfileRead`] do not see the
    /// active project. Returns `None` if the type cannot run.
    pub fn decorate_face(
        &self,
        action_type: &ActionTypeId,
        face: KeyFace,
        parameters: &ActionParameters,
        template: &str,
        project: Option<&Project>,
    ) -> Option<KeyFace> {
        let handler = self.handler(action_type)?;
        let project = project.filter(|_| self.allows(action_type, Capability::ProfileRead));
        Some(handler.decorate_face(face, parameters, template, project))
    }

    /// The refresh policy of `action_type`'s live state, if it has one.
    pub fn state_policy(
        &self,
        action_type: &ActionTypeId,
        parameters: &ActionParameters,
    ) -> Option<RefreshPolicy> {
        self.handler(action_type)?.state_policy(parameters)
    }

    /// The live state of a key running `action_type`, from its module,
    /// with the same capability limits as [`ActionRegistry::dispatch`]. The
    /// result is sanitized (short texts, progress at most 100). `None` if
    /// the type cannot run or provides no state.
    pub fn button_state(
        &self,
        action_type: &ActionTypeId,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Option<Result<ButtonState, StateError>> {
        let entry = self.handlers.get(action_type)?;
        entry.handler.state_policy(parameters)?;
        let restricted;
        let context = match &entry.capabilities {
            Some(granted) => {
                restricted = context.restricted(granted);
                &restricted
            }
            None => context,
        };
        Some(
            entry
                .handler
                .button_state(parameters, context)
                .map(ButtonState::sanitized)
                .map_err(StateError::sanitized),
        )
    }

    /// The module that provides `id`, if it was registered by a module.
    pub fn module_of(&self, id: &ActionTypeId) -> Option<&ModuleId> {
        self.handlers
            .get(id)
            .and_then(|entry| entry.module.as_ref())
    }

    /// Why a known action type cannot run, if it cannot.
    pub fn unavailable_reason(&self, id: &ActionTypeId) -> Option<&str> {
        self.unavailable.get(id).map(String::as_str)
    }

    pub fn action_types(&self) -> impl Iterator<Item = &ActionType> {
        self.handlers
            .values()
            .map(|entry| entry.handler.definition())
    }

    /// Executes `parameters` (already normalized) with the handler of
    /// `action_type`. Effects the handler's module is not allowed to request
    /// turn the result into a failure.
    pub fn dispatch(
        &self,
        action_type: &ActionTypeId,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let Some(entry) = self.handlers.get(action_type) else {
            return ActionResult::failure(match self.unavailable_reason(action_type) {
                Some(reason) => format!("this action is unavailable: {reason}"),
                None => format!("action type `{action_type}` is not available"),
            });
        };
        let restricted;
        let context = match &entry.capabilities {
            Some(granted) => {
                restricted = context.restricted(granted);
                &restricted
            }
            None => context,
        };
        let result = entry.handler.execute(parameters, context);
        if let Some(allowed) = &entry.capabilities
            && let Some(effect) = result
                .effects
                .iter()
                .find(|effect| !allowed.contains(&required_capability(effect)))
        {
            let capability = required_capability(effect);
            return ActionResult::failure(format!(
                "the module providing `{action_type}` may not {}",
                capability.description().to_lowercase()
            ));
        }
        result
    }

    /// Validates parameters and returns them normalized for `action_type`.
    pub fn normalize(
        &self,
        action_type: &ActionTypeId,
        parameters: &ActionParameters,
    ) -> Result<ActionParameters, RegistryError> {
        let handler = self.handler(action_type).ok_or_else(|| {
            match self.unavailable_reason(action_type) {
                Some(reason) => RegistryError::Unavailable(action_type.clone(), reason.to_owned()),
                None => RegistryError::UnknownActionType(action_type.clone()),
            }
        })?;
        handler
            .definition()
            .normalize(parameters)
            .map_err(RegistryError::InvalidParameters)
    }

    /// Creates a validated action instance with a fresh id.
    pub fn create_instance(
        &self,
        name: &str,
        action_type: ActionTypeId,
        parameters: &ActionParameters,
    ) -> Result<ActionInstance, RegistryError> {
        let mut instance = ActionInstance {
            id: ActionInstanceId::new(),
            name: name.to_owned(),
            action_type,
            parameters: parameters.clone(),
            description: String::new(),
            icon: None,
            label: None,
        };
        self.validate_instance(&mut instance)?;
        Ok(instance)
    }

    /// Validates a complete instance (e.g. after editing) and normalizes its
    /// name, parameters and presentation fields in place.
    pub fn validate_instance(&self, instance: &mut ActionInstance) -> Result<(), RegistryError> {
        instance.name = instance.name.trim().to_owned();
        if instance.name.is_empty() {
            return Err(RegistryError::EmptyName);
        }
        instance.description = instance.description.trim().to_owned();
        instance.icon = instance
            .icon
            .take()
            .map(|i| i.trim().to_owned())
            .filter(|i| !i.is_empty());
        instance.label = instance
            .label
            .take()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty());
        crate::action::validate_presentation(instance.icon.as_deref(), instance.label.as_deref())
            .map_err(RegistryError::InvalidPresentation)?;
        instance.parameters = self.normalize(&instance.action_type, &instance.parameters)?;
        Ok(())
    }

    /// Problems of an instance that only show in the context of its profile.
    pub fn check_in_profile(&self, instance: &ActionInstance, profile: &Profile) -> Vec<String> {
        self.check_parameters(&instance.action_type, &instance.parameters, profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Echo, Select, echo_type, select_type};
    use crate::{MachineConfig, Os, ParameterValue, Project, ProjectKey};

    fn registry() -> ActionRegistry {
        let mut registry = ActionRegistry::default();
        registry.register(Box::new(Echo::new())).unwrap();
        registry
    }

    #[test]
    fn rejects_duplicate_registration() {
        let mut registry = registry();
        assert!(matches!(
            registry.register(Box::new(Echo::new())),
            Err(RegistryError::Duplicate(_))
        ));
    }

    #[test]
    fn create_instance_validates_input() {
        let registry = registry();
        let params = ActionParameters::new().with("message", ParameterValue::Text("Hello".into()));

        let instance = registry
            .create_instance(" Greet ", echo_type(), &params)
            .unwrap();
        assert_eq!(instance.name, "Greet");

        assert_eq!(
            registry.create_instance("", echo_type(), &params),
            Err(RegistryError::EmptyName)
        );
        assert!(matches!(
            registry.create_instance("Greet", echo_type(), &ActionParameters::new()),
            Err(RegistryError::InvalidParameters(_))
        ));
        let unknown = ActionTypeId::new("vendor.unknown").unwrap();
        assert!(matches!(
            registry.create_instance("X", unknown, &params),
            Err(RegistryError::UnknownActionType(_))
        ));
    }

    #[test]
    fn unavailable_types_report_why() {
        let mut registry = registry();
        registry.mark_unavailable([select_type()], "the Test module is disabled");
        assert!(matches!(
            registry.create_instance("X", select_type(), &ActionParameters::new()),
            Err(RegistryError::Unavailable(..))
        ));
        let profile = Profile::new("Work");
        let machine = MachineConfig::new(Os::Linux);
        let context = ExecutionContext::new(&profile, None, &machine);
        let result = registry.dispatch(&select_type(), &ActionParameters::new(), &context);
        assert_eq!(
            result.message,
            "this action is unavailable: the Test module is disabled"
        );
    }

    #[test]
    fn button_states_come_from_providers_within_capabilities() {
        let mut profile = Profile::new("Work");
        profile
            .projects
            .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
        let machine = MachineConfig::new(Os::Linux);
        let context = ExecutionContext::new(&profile, Some(&profile.projects[0]), &machine);
        let message =
            |text: &str| ActionParameters::new().with("message", ParameterValue::Text(text.into()));

        let registry = registry();
        assert!(registry.state_policy(&echo_type(), &message("x")).is_some());
        let state = registry
            .button_state(&echo_type(), &message("Git {project.key}"), &context)
            .unwrap()
            .unwrap();
        assert_eq!(state.label.as_deref(), Some("Git SHOP"));
        // Texts are shortened by the core, whatever the module returns.
        let long = registry
            .button_state(&echo_type(), &message(&"x".repeat(100)), &context)
            .unwrap()
            .unwrap();
        assert_eq!(
            long.label.unwrap().len(),
            crate::button_state::MAX_STATE_TEXT
        );
        let failed = registry
            .button_state(&echo_type(), &message("fail"), &context)
            .unwrap()
            .unwrap_err();
        assert_eq!(failed.code, "failed");

        // Types without a provider have no state; unknown types neither.
        let mut with_select = ActionRegistry::default();
        with_select.register(Box::new(Select::new())).unwrap();
        assert_eq!(
            with_select.state_policy(&select_type(), &ActionParameters::new()),
            None
        );
        assert!(
            with_select
                .button_state(&select_type(), &ActionParameters::new(), &context)
                .is_none()
        );
        assert!(
            with_select
                .button_state(&echo_type(), &message("x"), &context)
                .is_none()
        );

        // A module's provider sees only what its manifest declares.
        let mut blind = ActionRegistry::default();
        blind
            .register_for_module(ModuleId::new("test").unwrap(), &[], Box::new(Echo::new()))
            .unwrap();
        let denied = blind
            .button_state(&echo_type(), &message("x"), &context)
            .unwrap()
            .unwrap_err();
        assert_eq!(denied.code, "not-permitted");
    }

    #[test]
    fn dispatch_enforces_module_capabilities() {
        let mut profile = Profile::new("Work");
        profile
            .projects
            .push(Project::new(ProjectKey::new("A").unwrap(), "A"));
        let machine = MachineConfig::new(Os::Linux);
        let context = ExecutionContext::new(&profile, None, &machine);
        let module = ModuleId::new("test").unwrap();

        let register = |capabilities: &[Capability]| {
            let mut registry = ActionRegistry::default();
            registry
                .register_for_module(module.clone(), capabilities, Box::new(Select::new()))
                .unwrap();
            registry
        };
        let run = |registry: &ActionRegistry| {
            registry.dispatch(&select_type(), &ActionParameters::new(), &context)
        };

        // Reading the profile is a capability too.
        let blind = run(&register(&[]));
        assert_eq!(
            blind.message,
            "the module is not allowed to read the active profile"
        );

        // Reading is not enough to switch projects.
        let denied = run(&register(&[Capability::ProfileRead]));
        assert!(!denied.is_success());
        assert!(denied.message.contains("may not change the active project"));
        assert_eq!(denied.effects, Vec::new());

        let allowed = register(&[Capability::ProfileRead, Capability::ProjectContext]);
        assert!(run(&allowed).is_success());
        assert_eq!(allowed.module_of(&select_type()), Some(&module));
    }
}
