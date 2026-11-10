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
    Capability, ContextEffect, ExecutionContext, KeyFace, ModuleId, ParameterError, Profile,
    Project,
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

    pub fn handler(&self, id: &ActionTypeId) -> Option<&dyn ActionHandler> {
        self.handlers.get(id).map(|entry| entry.handler.as_ref())
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
        self.handler(&instance.action_type)
            .map(|handler| handler.check(&instance.parameters, profile))
            .unwrap_or_default()
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
        let context = ExecutionContext {
            profile: &profile,
            project: None,
            machine: &machine,
        };
        let result = registry.dispatch(&select_type(), &ActionParameters::new(), &context);
        assert_eq!(
            result.message,
            "this action is unavailable: the Test module is disabled"
        );
    }

    #[test]
    fn dispatch_enforces_module_capabilities() {
        let mut profile = Profile::new("Work");
        profile
            .projects
            .push(Project::new(ProjectKey::new("A").unwrap(), "A"));
        let machine = MachineConfig::new(Os::Linux);
        let context = ExecutionContext {
            profile: &profile,
            project: None,
            machine: &machine,
        };
        let module = ModuleId::new("test").unwrap();

        let mut limited = ActionRegistry::default();
        limited
            .register_for_module(module.clone(), &[], Box::new(Select::new()))
            .unwrap();
        let denied = limited.dispatch(&select_type(), &ActionParameters::new(), &context);
        assert!(!denied.is_success());
        assert!(denied.message.contains("may not change the active project"));

        let mut allowed = ActionRegistry::default();
        allowed
            .register_for_module(
                module.clone(),
                &[Capability::ProjectContext],
                Box::new(Select::new()),
            )
            .unwrap();
        assert!(
            allowed
                .dispatch(&select_type(), &ActionParameters::new(), &context)
                .is_success()
        );
        assert_eq!(allowed.module_of(&select_type()), Some(&module));
    }
}
