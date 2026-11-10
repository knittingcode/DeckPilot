//! The `general` module: action types that do not belong to a specific
//! feature and have no side effects on the host system.

use std::collections::BTreeMap;

use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    ExecutionContext, FeatureModule, ModuleId, ModuleManifest, ParameterKind, ParameterSpec,
    Portability,
};

pub const MODULE_ID: &str = "general";
pub const LOG: &str = "general.log";

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn log_id() -> ActionTypeId {
    ActionTypeId::new(LOG).expect("the action type id is valid")
}

#[derive(Debug)]
pub struct GeneralModule {
    manifest: ModuleManifest,
}

impl GeneralModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "General".into(),
                description: "Basic actions such as writing a message to the execution log.".into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: vec![Capability::ProfileRead],
                config: Vec::new(),
                dependencies: Vec::new(),
                enabled_by_default: true,
                // Only the enabled state travels; there are no settings.
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::new(),
                },
                migrations: Vec::new(),
            },
        }
    }
}

impl Default for GeneralModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for GeneralModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, _config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        vec![Box::new(LogMessage::new())]
    }
}

/// Produces a message, expanding `{profile}`, `{project.key}` and
/// `{project.name}` from the execution context.
#[derive(Debug)]
pub struct LogMessage(ActionType);

impl LogMessage {
    pub fn new() -> Self {
        Self(ActionType {
            id: log_id(),
            name: "Log message".into(),
            description: "Writes a message to the execution log.".into(),
            category: "General".into(),
            parameters: vec![ParameterSpec {
                key: "message".into(),
                label: "Message".into(),
                description: Some(
                    "Supports {profile}, {project.key} and {project.name} placeholders.".into(),
                ),
                kind: ParameterKind::Text {
                    multiline: true,
                    max_length: Some(1000),
                },
                required: true,
                default: None,
            }],
            requires_project: false,
        })
    }
}

impl Default for LogMessage {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for LogMessage {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let template = parameters.text("message").unwrap_or_default();
        let (profile, project) = match (context.profile(), context.project()) {
            (Ok(profile), Ok(project)) => (profile, project),
            (Err(denied), _) | (_, Err(denied)) => return denied.into(),
        };
        let mut output = template.replace("{profile}", &profile.name);
        if output.contains("{project.") {
            let Some(project) = project else {
                return ActionResult::failure(
                    "message refers to the active project, but none is active",
                );
            };
            output = output
                .replace("{project.key}", project.key.as_str())
                .replace("{project.name}", &project.name);
        }
        ActionResult::success(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::{MachineConfig, Os, ParameterValue, Profile, Project, ProjectKey};

    #[test]
    fn log_message_expands_placeholders() {
        let mut profile = Profile::new("Work");
        profile
            .projects
            .push(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"));
        let machine = MachineConfig::new(Os::Linux);
        let handler = LogMessage::new();
        let params = handler
            .definition()
            .normalize(&ActionParameters::new().with(
                "message",
                ParameterValue::Text("{profile}: {project.key} / {project.name}".into()),
            ))
            .unwrap();
        let mut context = ExecutionContext::new(&profile, Some(&profile.projects[0]), &machine);
        let result = handler.execute(&params, &context);
        assert_eq!(result.message, "Work: SHOP / Shop");

        context = ExecutionContext::new(&profile, None, &machine);
        assert!(!handler.execute(&params, &context).is_success());
    }

    #[test]
    fn action_types_are_namespaced() {
        let module = GeneralModule::new();
        for handler in module.action_handlers(&ActionParameters::new()) {
            assert_eq!(
                ModuleId::owner_of(&handler.definition().id),
                Some(module_id())
            );
        }
    }
}
