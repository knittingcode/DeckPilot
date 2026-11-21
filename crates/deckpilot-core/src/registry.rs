//! Registry of executable action types and their handlers.
//!
//! The registry used at run time is built by the
//! [`ModuleRegistry`](crate::ModuleRegistry) from the enabled feature
//! modules. The core never refers to a specific action type; it dispatches
//! through this registry.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use thiserror::Error;

use crate::alias::{ActionAlias, ParamRule};
use crate::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionResult, ActionType, ActionTypeId,
    ButtonState, Capability, ContextEffect, ExecutionContext, KeyFace, ModuleId, ParameterError,
    ParameterKind, ParameterSpec, ParameterValue, Profile, Project, RefreshPolicy, StateError,
};

/// What an action does between the layouts of a device
/// ([`ActionHandler::layout_navigation`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutNavigation {
    /// Shows this layout (*Switch layout*).
    SwitchTo(crate::LayoutId),
    /// Returns to the layout shown before (*Back to previous layout*).
    Back,
}

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

    /// The handler hands every run to another process and blocks until it
    /// answers or gives up (a plugin's sandboxed host), possibly for
    /// minutes. Callers that must stay responsive (device input, IPC
    /// requests on the agent's service task) run such actions on a thread
    /// of their own: the [`Runtime`](crate::Runtime) reports a key bound
    /// to one as [`RuntimeOutcome::ActionRequested`](crate::RuntimeOutcome)
    /// instead of running it in place. Workflow nodes already run on
    /// their own threads and call [`ActionHandler::execute`] there.
    fn runs_detached(&self) -> bool {
        false
    }

    /// Whether the action moves a device between layouts (*Switch layout*,
    /// *Back to previous layout*), so that its key can show the target
    /// layout's name and whether there is a way back. `None` (the
    /// default): it does not.
    fn layout_navigation(&self, _parameters: &ActionParameters) -> Option<LayoutNavigation> {
        None
    }

    /// Checks normalized parameters against the profile the action belongs
    /// to, for example that a referenced project exists in that profile.
    fn check(&self, _parameters: &ActionParameters, _profile: &Profile) -> Vec<String> {
        Vec::new()
    }

    /// Problems of normalized parameters that the definition's schema
    /// cannot express and that need nothing but the parameters (no
    /// profile, so no capability): [`ActionRegistry::normalize`] reports
    /// them with the schema's, as
    /// [`RegistryError::InvalidParameters`], whenever an action, a
    /// workflow step or a node is saved. Unlike [`ActionHandler::check`],
    /// it runs for every handler, a plugin's included.
    fn validate(&self, _parameters: &ActionParameters) -> Vec<ParameterError> {
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

/// Serves a legacy built-in action type with a first-party plugin's
/// handler ([`crate::alias`]). Its definition is the plugin's, under the
/// legacy id and hidden from the catalog like a deprecated type, so stored
/// instances, bindings and workflow nodes keep their id and keep working.
/// Parameters the plugin cannot declare are translated by the alias's
/// [`ParamRule`]s before every call.
pub struct AliasHandler {
    definition: ActionType,
    target: Box<dyn ActionHandler>,
    /// Parameters dropped when empty or equal to the legacy module's
    /// setting of the same key (`None`: no setting).
    drops: Vec<(String, Option<ParameterValue>)>,
    /// Legacy keys and the plugin's key for each.
    renames: Vec<(String, String)>,
    /// Parameters the plugin cannot honour any value of.
    unsupported: Vec<String>,
    /// Integer parameters the plugin takes only up to a bound
    /// ([`ParamRule::AtMost`]).
    capped: Vec<(String, i64)>,
}

impl fmt::Debug for AliasHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AliasHandler")
            .field("legacy", &self.definition.id)
            .field("target", &self.target.definition().id)
            .finish_non_exhaustive()
    }
}

impl AliasHandler {
    /// `target` is the plugin's handler of `alias.target`; `builtin` the
    /// legacy definition, when it is compiled in, and `legacy_config` the
    /// legacy module's settings, when it is registered.
    pub fn new(
        alias: &ActionAlias,
        target: Box<dyn ActionHandler>,
        builtin: Option<&ActionType>,
        legacy_config: Option<&ActionParameters>,
    ) -> Self {
        let mut definition = target.definition().clone();
        definition.id = alias.legacy_id();
        definition.deprecated = true;
        let mut drops = Vec::new();
        let mut renames = Vec::new();
        let mut unsupported = Vec::new();
        let mut capped = Vec::new();
        for rule in alias.params {
            match *rule {
                ParamRule::DropIfDefault(key) => {
                    if !definition.parameters.iter().any(|p| p.key == key) {
                        // Stored instances keep validating with the parameter.
                        let spec = builtin
                            .and_then(|b| b.parameters.iter().find(|p| p.key == key))
                            .cloned()
                            .unwrap_or_else(|| ParameterSpec {
                                key: key.to_owned(),
                                label: key.to_owned(),
                                description: None,
                                kind: ParameterKind::Credential,
                                required: false,
                                default: None,
                            });
                        definition.parameters.push(ParameterSpec {
                            required: false,
                            ..spec
                        });
                    }
                    drops.push((
                        key.to_owned(),
                        legacy_config.and_then(|c| c.get(key)).cloned(),
                    ));
                }
                // The legacy type keeps the legacy key: stored instances,
                // overrides and mappings name it.
                ParamRule::Rename(from, to) => {
                    if let Some(spec) = definition.parameters.iter_mut().find(|p| p.key == to) {
                        from.clone_into(&mut spec.key);
                    } else if let Some(spec) =
                        builtin.and_then(|b| b.parameters.iter().find(|p| p.key == from))
                    {
                        definition.parameters.push(ParameterSpec {
                            required: false,
                            ..spec.clone()
                        });
                    }
                    renames.push((from.to_owned(), to.to_owned()));
                }
                ParamRule::Unsupported(key) => unsupported.push(key.to_owned()),
                ParamRule::AtMost(key, most) => capped.push((key.to_owned(), most)),
                ParamRule::Keep(_) => {}
            }
        }
        Self {
            definition,
            target,
            drops,
            renames,
            unsupported,
            capped,
        }
    }

    /// The parameters for the plugin's handler.
    fn translate(&self, parameters: &ActionParameters) -> Result<ActionParameters, String> {
        self.translated(parameters).map_err(|error| match error {
            ParameterError::Invalid { key, reason } => format!("parameter `{key}` {reason}"),
            other => other.to_string(),
        })
    }

    /// The parameters for the plugin's handler, or the first one it
    /// cannot be given. Nothing is dropped but what equals the plugin's
    /// own (an empty or default `credential`), and nothing is guessed.
    fn translated(
        &self,
        parameters: &ActionParameters,
    ) -> Result<ActionParameters, ParameterError> {
        let empty = |value: &ParameterValue| matches!(value, ParameterValue::Text(text) if text.trim().is_empty());
        let mut translated = ActionParameters::new();
        for (key, value) in parameters.iter() {
            if let Some((_, default)) = self.drops.iter().find(|(k, _)| k == key) {
                if !empty(value) && default.as_ref() != Some(value) {
                    return Err(ParameterError::Invalid {
                        key: key.clone(),
                        reason: format!(
                            "names something the plugin providing `{}` cannot use; leave it \
                             empty to use the plugin's own",
                            self.definition.id
                        ),
                    });
                }
                continue;
            }
            if self.unsupported.contains(key) && !empty(value) {
                return Err(ParameterError::Invalid {
                    key: key.clone(),
                    reason: format!(
                        "is not supported by the plugin providing `{}`, and no other value is \
                         guessed for it; clear it to run the action",
                        self.definition.id
                    ),
                });
            }
            if let (Some((_, most)), ParameterValue::Integer(n)) =
                (self.capped.iter().find(|(k, _)| k == key), value)
                && n > most
            {
                return Err(ParameterError::Invalid {
                    key: key.clone(),
                    reason: format!(
                        "is {n}, above {most}, the most the plugin providing `{}` accepts; it \
                         is not lowered for you: set it to {most} or less to run the action",
                        self.definition.id
                    ),
                });
            }
            let plugin_key = match self.renames.iter().find(|(from, _)| from == key) {
                Some((_, to)) => {
                    if parameters.get(to).is_some() {
                        return Err(ParameterError::Invalid {
                            key: to.clone(),
                            reason: format!("is `{key}` of `{}`, which is set", self.definition.id),
                        });
                    }
                    to.clone()
                }
                None => key.clone(),
            };
            translated = translated.with(plugin_key, value.clone());
        }
        Ok(translated)
    }
}

impl ActionHandler for AliasHandler {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        match self.translate(parameters) {
            Ok(parameters) => self.target.execute(&parameters, context),
            Err(problem) => ActionResult::failure(problem),
        }
    }

    fn runs_detached(&self) -> bool {
        self.target.runs_detached()
    }

    fn layout_navigation(&self, parameters: &ActionParameters) -> Option<LayoutNavigation> {
        self.translate(parameters)
            .ok()
            .and_then(|parameters| self.target.layout_navigation(&parameters))
    }

    fn check(&self, parameters: &ActionParameters, profile: &Profile) -> Vec<String> {
        match self.translate(parameters) {
            Ok(parameters) => self.target.check(&parameters, profile),
            Err(problem) => vec![problem],
        }
    }

    /// A parameter the plugin cannot be given is refused when the action
    /// is saved, as the run would refuse it; the rest is the plugin's.
    fn validate(&self, parameters: &ActionParameters) -> Vec<ParameterError> {
        match self.translated(parameters) {
            Ok(parameters) => self.target.validate(&parameters),
            Err(problem) => vec![problem],
        }
    }

    fn decorate_face(
        &self,
        face: KeyFace,
        parameters: &ActionParameters,
        template: &str,
        project: Option<&Project>,
    ) -> KeyFace {
        match self.translate(parameters) {
            Ok(parameters) => self
                .target
                .decorate_face(face, &parameters, template, project),
            Err(_) => face,
        }
    }

    fn state_policy(&self, parameters: &ActionParameters) -> Option<RefreshPolicy> {
        self.target.state_policy(&self.translate(parameters).ok()?)
    }

    fn button_state(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> Result<ButtonState, StateError> {
        let parameters = self
            .translate(parameters)
            .map_err(|problem| StateError::new("invalid-parameters", problem))?;
        self.target.button_state(&parameters, context)
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
        ContextEffect::SelectProject { .. } | ContextEffect::CycleProject { .. } => {
            Capability::ProjectContext
        }
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

    /// What a key running `action_type` with `parameters` does between
    /// layouts; see [`ActionHandler::layout_navigation`].
    pub fn layout_navigation(
        &self,
        action_type: &ActionTypeId,
        parameters: &ActionParameters,
    ) -> Option<LayoutNavigation> {
        self.handler(action_type)?.layout_navigation(parameters)
    }

    /// Whether `action_type` must not run on a thread that has to stay
    /// responsive; see [`ActionHandler::runs_detached`].
    pub fn runs_detached(&self, action_type: &ActionTypeId) -> bool {
        self.handler(action_type)
            .is_some_and(ActionHandler::runs_detached)
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
        if let Some(allowed) = &entry.capabilities
            && result.request.is_some()
            && !allowed.contains(&Capability::DeckControl)
        {
            return ActionResult::failure(format!(
                "the module providing `{action_type}` may not {}",
                Capability::DeckControl.description().to_lowercase()
            ));
        }
        result
    }

    /// Validates parameters and returns them normalized for `action_type`:
    /// the definition's schema (types, required, choices, ranges,
    /// lengths; for a plugin action, its manifest's), then the handler's
    /// [`ActionHandler::validate`].
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
        let normalized = handler
            .definition()
            .normalize(parameters)
            .map_err(RegistryError::InvalidParameters)?;
        let problems = handler.validate(&normalized);
        if problems.is_empty() {
            Ok(normalized)
        } else {
            Err(RegistryError::InvalidParameters(problems))
        }
    }

    /// The handler's own problems with normalized `parameters`
    /// ([`ActionHandler::validate`]); none for a type that cannot run.
    pub fn validate_parameters(
        &self,
        action_type: &ActionTypeId,
        parameters: &ActionParameters,
    ) -> Vec<ParameterError> {
        self.handler(action_type)
            .map(|handler| handler.validate(parameters))
            .unwrap_or_default()
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
