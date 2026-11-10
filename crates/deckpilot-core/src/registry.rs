//! Registry of available action types and their handlers.

use std::collections::BTreeMap;
use std::fmt;

use thiserror::Error;

use crate::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionResult, ActionType, ActionTypeId,
    ExecutionContext, ParameterError,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegistryError {
    #[error("action type `{0}` is already registered")]
    Duplicate(ActionTypeId),
    #[error("unknown action type `{0}`")]
    UnknownActionType(ActionTypeId),
    #[error("action name must not be empty")]
    EmptyName,
    #[error("invalid parameters: {}", join(.0))]
    InvalidParameters(Vec<ParameterError>),
}

fn join(errors: &[ParameterError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Default)]
pub struct ActionRegistry {
    handlers: BTreeMap<ActionTypeId, Box<dyn ActionHandler>>,
}

impl fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionRegistry")
            .field("types", &self.handlers.keys())
            .finish()
    }
}

impl ActionRegistry {
    /// Registry containing every built-in action type.
    ///
    /// # Panics
    ///
    /// Panics if two built-in handlers share an id, which is a programming error
    /// covered by unit tests.
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        for handler in crate::builtin::handlers() {
            registry
                .register(handler)
                .expect("built-in action type ids are unique");
        }
        registry
    }

    pub fn register(&mut self, handler: Box<dyn ActionHandler>) -> Result<(), RegistryError> {
        let id = handler.definition().id.clone();
        if self.handlers.contains_key(&id) {
            return Err(RegistryError::Duplicate(id));
        }
        self.handlers.insert(id, handler);
        Ok(())
    }

    pub fn handler(&self, id: &ActionTypeId) -> Option<&dyn ActionHandler> {
        self.handlers.get(id).map(AsRef::as_ref)
    }

    pub fn action_types(&self) -> impl Iterator<Item = &ActionType> {
        self.handlers.values().map(|handler| handler.definition())
    }

    /// Validates parameters and returns them normalized for `action_type`.
    pub fn normalize(
        &self,
        action_type: &ActionTypeId,
        parameters: &ActionParameters,
    ) -> Result<ActionParameters, RegistryError> {
        let handler = self
            .handler(action_type)
            .ok_or_else(|| RegistryError::UnknownActionType(action_type.clone()))?;
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
        let name = name.trim();
        if name.is_empty() {
            return Err(RegistryError::EmptyName);
        }
        let parameters = self.normalize(&action_type, parameters)?;
        Ok(ActionInstance {
            id: ActionInstanceId::new(),
            name: name.to_owned(),
            action_type,
            parameters,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ParameterValue, builtin};

    #[test]
    fn builtins_are_registered() {
        let registry = ActionRegistry::with_builtins();
        assert!(registry.action_types().count() >= 4);
        assert!(registry.handler(&builtin::log_message_id()).is_some());
    }

    #[test]
    fn rejects_duplicate_registration() {
        let mut registry = ActionRegistry::with_builtins();
        let duplicate = builtin::handlers().into_iter().next().unwrap();
        assert!(matches!(
            registry.register(duplicate),
            Err(RegistryError::Duplicate(_))
        ));
    }

    #[test]
    fn create_instance_validates_input() {
        let registry = ActionRegistry::with_builtins();
        let params = ActionParameters::new().with("message", ParameterValue::Text("Hello".into()));

        let instance = registry
            .create_instance(" Greet ", builtin::log_message_id(), &params)
            .unwrap();
        assert_eq!(instance.name, "Greet");

        assert_eq!(
            registry.create_instance("", builtin::log_message_id(), &params),
            Err(RegistryError::EmptyName)
        );
        assert!(matches!(
            registry.create_instance("Greet", builtin::log_message_id(), &ActionParameters::new()),
            Err(RegistryError::InvalidParameters(_))
        ));
        let unknown = ActionTypeId::new("vendor.unknown").unwrap();
        assert!(matches!(
            registry.create_instance("X", unknown, &params),
            Err(RegistryError::UnknownActionType(_))
        ));
    }
}
