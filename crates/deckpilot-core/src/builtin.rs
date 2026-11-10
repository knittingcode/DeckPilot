//! Built-in action types that work without hardware or side effects on the
//! host system.

use crate::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, ContextEffect,
    CycleDirection, ExecutionContext, ParameterKind, ParameterSpec, ParameterValue,
};

pub const LOG_MESSAGE: &str = "deckpilot.core.log";
pub const CYCLE_PROJECT: &str = "deckpilot.project.cycle";
pub const SELECT_PROJECT: &str = "deckpilot.project.select";
pub const RESOLVE_RESOURCE: &str = "deckpilot.resource.resolve";

fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("built-in action type ids are valid")
}

pub fn log_message_id() -> ActionTypeId {
    id(LOG_MESSAGE)
}

pub fn cycle_project_id() -> ActionTypeId {
    id(CYCLE_PROJECT)
}

pub fn select_project_id() -> ActionTypeId {
    id(SELECT_PROJECT)
}

pub fn resolve_resource_id() -> ActionTypeId {
    id(RESOLVE_RESOURCE)
}

/// All built-in handlers.
pub fn handlers() -> Vec<Box<dyn ActionHandler>> {
    vec![
        Box::new(LogMessage::new()),
        Box::new(CycleProject::new()),
        Box::new(SelectProject::new()),
        Box::new(ResolveResource::new()),
    ]
}

fn spec(key: &str, label: &str, kind: ParameterKind, required: bool) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: label.into(),
        description: None,
        kind,
        required,
        default: None,
    }
}

/// Produces a message, expanding `{profile}`, `{project.key}` and
/// `{project.name}` from the execution context.
#[derive(Debug)]
pub struct LogMessage(ActionType);

impl LogMessage {
    pub fn new() -> Self {
        let mut message = spec(
            "message",
            "Message",
            ParameterKind::Text {
                multiline: true,
                max_length: Some(1000),
            },
            true,
        );
        message.description =
            Some("Supports {profile}, {project.key} and {project.name} placeholders.".into());
        Self(ActionType {
            id: log_message_id(),
            name: "Log message".into(),
            description: "Writes a message to the execution log.".into(),
            category: "General".into(),
            parameters: vec![message],
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
        match render_template(template, context) {
            Ok(message) => ActionResult::success(message),
            Err(error) => ActionResult::failure(error),
        }
    }
}

fn render_template(template: &str, context: &ExecutionContext<'_>) -> Result<String, String> {
    let mut output = template.replace("{profile}", &context.profile.name);
    if output.contains("{project.") {
        let project = context
            .project
            .ok_or_else(|| "message refers to the active project, but none is active".to_owned())?;
        output = output
            .replace("{project.key}", project.key.as_str())
            .replace("{project.name}", &project.name);
    }
    Ok(output)
}

/// Switches to the next or previous project of the profile, wrapping around.
#[derive(Debug)]
pub struct CycleProject(ActionType);

impl CycleProject {
    pub fn new() -> Self {
        let mut direction = spec(
            "direction",
            "Direction",
            ParameterKind::Choice {
                options: vec!["next".into(), "previous".into()],
            },
            true,
        );
        direction.default = Some(ParameterValue::Choice("next".into()));
        Self(ActionType {
            id: cycle_project_id(),
            name: "Cycle project".into(),
            description: "Makes the next or previous project active.".into(),
            category: "Projects".into(),
            parameters: vec![direction],
            requires_project: false,
        })
    }
}

impl Default for CycleProject {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for CycleProject {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        if context.profile.projects.is_empty() {
            return ActionResult::failure("profile has no projects");
        }
        let direction = match parameters.text("direction") {
            Some("previous") => CycleDirection::Previous,
            _ => CycleDirection::Next,
        };
        ActionResult::success("project switched")
            .with_effect(ContextEffect::CycleProject { direction })
    }
}

/// Makes a specific project active, identified by its key.
#[derive(Debug)]
pub struct SelectProject(ActionType);

impl SelectProject {
    pub fn new() -> Self {
        Self(ActionType {
            id: select_project_id(),
            name: "Select project".into(),
            description: "Makes the project with the given key active.".into(),
            category: "Projects".into(),
            parameters: vec![spec(
                "project",
                "Project key",
                ParameterKind::Text {
                    multiline: false,
                    max_length: Some(32),
                },
                true,
            )],
            requires_project: false,
        })
    }
}

impl Default for SelectProject {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for SelectProject {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let key = parameters.text("project").unwrap_or_default();
        match context.profile.project_by_key(key) {
            Some(project) => ActionResult::success(format!("selected {}", project.key))
                .with_effect(ContextEffect::SelectProject {
                    project: project.id,
                }),
            None => ActionResult::failure(format!("profile has no project `{key}`")),
        }
    }
}

/// Resolves a logical resource on this machine and reports the result
/// without acting on it. Useful to verify machine bindings.
#[derive(Debug)]
pub struct ResolveResource(ActionType);

impl ResolveResource {
    pub fn new() -> Self {
        Self(ActionType {
            id: resolve_resource_id(),
            name: "Resolve resource".into(),
            description: "Shows the machine-specific value bound to a logical resource.".into(),
            category: "Resources".into(),
            parameters: vec![spec(
                "resource",
                "Resource",
                ParameterKind::Resource { kind: None },
                true,
            )],
            requires_project: false,
        })
    }
}

impl Default for ResolveResource {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for ResolveResource {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let Some(reference) = parameters.resource("resource") else {
            return ActionResult::failure("no resource configured");
        };
        match context.resolve(reference, None) {
            Ok(binding) => ActionResult::success(format!(
                "{} ({:?}) = {}",
                binding.key, binding.kind, binding.value
            )),
            Err(error) => ActionResult::failure(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MachineConfig, MachineResourceBinding, Os, Profile, Project, ProjectKey, ResourceKey,
        ResourceKind, ResourceRef,
    };

    fn fixture() -> (Profile, MachineConfig) {
        let mut profile = Profile::new("Work");
        let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
        project
            .resources
            .insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
        profile.projects.push(project);
        let mut machine = MachineConfig::new(Os::Linux);
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new("shop.repo").unwrap(),
            kind: ResourceKind::Directory,
            value: "/src/shop".into(),
        });
        (profile, machine)
    }

    fn run(
        handler: &dyn ActionHandler,
        params: &ActionParameters,
        with_project: bool,
    ) -> ActionResult {
        let (profile, machine) = fixture();
        let params = handler.definition().normalize(params).unwrap();
        let context = ExecutionContext {
            profile: &profile,
            project: with_project.then(|| &profile.projects[0]),
            machine: &machine,
        };
        handler.execute(&params, &context)
    }

    #[test]
    fn log_message_expands_placeholders() {
        let params = ActionParameters::new().with(
            "message",
            ParameterValue::Text("{profile}: {project.key} / {project.name}".into()),
        );
        let result = run(&LogMessage::new(), &params, true);
        assert!(result.is_success());
        assert_eq!(result.message, "Work: SHOP / Shop");

        let without_project = run(&LogMessage::new(), &params, false);
        assert!(!without_project.is_success());
    }

    #[test]
    fn cycle_project_requests_an_effect() {
        let result = run(&CycleProject::new(), &ActionParameters::new(), true);
        assert_eq!(
            result.effects,
            vec![ContextEffect::CycleProject {
                direction: CycleDirection::Next
            }]
        );
    }

    #[test]
    fn select_project_matches_key_case_insensitively() {
        let params = ActionParameters::new().with("project", ParameterValue::Text("shop".into()));
        let result = run(&SelectProject::new(), &params, false);
        assert!(result.is_success());
        assert!(matches!(
            result.effects[0],
            ContextEffect::SelectProject { .. }
        ));

        let params = ActionParameters::new().with("project", ParameterValue::Text("NOPE".into()));
        assert!(!run(&SelectProject::new(), &params, false).is_success());
    }

    #[test]
    fn resolve_resource_uses_active_project() {
        let params = ActionParameters::new().with(
            "resource",
            ParameterValue::Resource(ResourceRef::ProjectRole {
                role: "repository".into(),
            }),
        );
        let result = run(&ResolveResource::new(), &params, true);
        assert!(result.is_success(), "{}", result.message);
        assert!(result.message.ends_with("/src/shop"));
        assert!(!run(&ResolveResource::new(), &params, false).is_success());
    }
}
