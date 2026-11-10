//! Action types of the projects module.
//!
//! Handlers never change state themselves. Switching actions request a
//! [`ContextEffect::SelectProject`]; the engine applies it only after checking
//! that the project belongs to the active profile.

use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, ContextEffect,
    CycleDirection, ExecutionContext, KeyFace, ParameterKind, ParameterSpec, Profile, Project,
    ProjectSwitcher,
};

pub const NEXT: &str = "projects.next";
pub const PREVIOUS: &str = "projects.previous";
pub const SELECT: &str = "projects.select";
pub const ACTIVE: &str = "projects.active";
pub const RESOLVE_RESOURCE: &str = "projects.resolve-resource";

/// # Panics
///
/// Never: the identifiers are valid constants.
fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("projects action type ids are valid")
}

pub fn next_id() -> ActionTypeId {
    id(NEXT)
}

pub fn previous_id() -> ActionTypeId {
    id(PREVIOUS)
}

pub fn select_id() -> ActionTypeId {
    id(SELECT)
}

pub fn active_id() -> ActionTypeId {
    id(ACTIVE)
}

pub fn resolve_resource_id() -> ActionTypeId {
    id(RESOLVE_RESOURCE)
}

/// Handlers for all action types of the module.
pub fn handlers(wrap_around: bool) -> Vec<Box<dyn ActionHandler>> {
    vec![
        Box::new(Step::new(CycleDirection::Next, wrap_around)),
        Box::new(Step::new(CycleDirection::Previous, wrap_around)),
        Box::new(Select::new()),
        Box::new(Active::new()),
        Box::new(ResolveResource::new()),
    ]
}

fn action_type(id: ActionTypeId, name: &str, description: &str) -> ActionType {
    ActionType {
        id,
        name: name.into(),
        description: description.into(),
        category: "Projects".into(),
        parameters: Vec::new(),
        requires_project: false,
    }
}

/// Shows the active project's key below the label, unless the label
/// template already contains the project.
fn with_project_detail(face: KeyFace, template: &str, project: Option<&Project>) -> KeyFace {
    if template.contains("{project.") {
        face
    } else {
        face.with_detail(project.map_or("-", |p| p.key.as_str()))
    }
}

/// `projects.next` and `projects.previous`: move through the profile's
/// project order.
#[derive(Debug)]
pub struct Step {
    definition: ActionType,
    direction: CycleDirection,
    wrap_around: bool,
}

impl Step {
    pub fn new(direction: CycleDirection, wrap_around: bool) -> Self {
        let definition = match direction {
            CycleDirection::Next => action_type(
                next_id(),
                "Next project",
                "Makes the next project of the profile active.",
            ),
            CycleDirection::Previous => action_type(
                previous_id(),
                "Previous project",
                "Makes the previous project of the profile active.",
            ),
        };
        Self {
            definition,
            direction,
            wrap_around,
        }
    }
}

impl ActionHandler for Step {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(&self, _: &ActionParameters, context: &ExecutionContext<'_>) -> ActionResult {
        let projects = &context.profile.projects;
        if projects.is_empty() {
            return ActionResult::failure("this profile has no projects");
        }
        let current = context.project.map(|p| p.id);
        if !self.wrap_around
            && let Some(position) = current.and_then(|id| projects.iter().position(|p| p.id == id))
        {
            let at_end = match self.direction {
                CycleDirection::Next => position + 1 == projects.len(),
                CycleDirection::Previous => position == 0,
            };
            if at_end {
                return ActionResult::failure(match self.direction {
                    CycleDirection::Next => "already at the last project",
                    CycleDirection::Previous => "already at the first project",
                });
            }
        }
        let Some(target) = ProjectSwitcher::cycle(projects, current, self.direction) else {
            return ActionResult::failure("this profile has no projects");
        };
        let key = context
            .profile
            .project(target)
            .map_or_else(String::new, |p| p.key.to_string());
        ActionResult::success(format!("switched to {key}"))
            .with_effect(ContextEffect::SelectProject { project: target })
    }

    fn decorate_face(
        &self,
        face: KeyFace,
        _: &ActionParameters,
        template: &str,
        project: Option<&Project>,
    ) -> KeyFace {
        with_project_detail(face, template, project)
    }
}

/// `projects.select`: makes one specific project of the profile active.
#[derive(Debug)]
pub struct Select(ActionType);

impl Select {
    pub fn new() -> Self {
        let mut definition = action_type(
            select_id(),
            "Select project",
            "Makes a specific project of the profile active.",
        );
        definition.parameters.push(ParameterSpec {
            key: "project".into(),
            label: "Project".into(),
            description: None,
            kind: ParameterKind::Project,
            required: true,
            default: None,
        });
        Self(definition)
    }
}

impl Default for Select {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for Select {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let Some(id) = parameters.project("project") else {
            return ActionResult::failure("no project configured");
        };
        match context.profile.project(id) {
            Some(project) => ActionResult::success(format!("selected {}", project.key))
                .with_effect(ContextEffect::SelectProject { project: id }),
            None => ActionResult::failure("the project does not belong to the active profile"),
        }
    }

    fn check(&self, parameters: &ActionParameters, profile: &Profile) -> Vec<String> {
        match parameters.project("project") {
            Some(id) if profile.project(id).is_none() => {
                vec!["the selected project does not belong to this profile".into()]
            }
            _ => Vec::new(),
        }
    }

    fn decorate_face(
        &self,
        face: KeyFace,
        parameters: &ActionParameters,
        _: &str,
        project: Option<&Project>,
    ) -> KeyFace {
        let target = parameters.project("project");
        face.highlighted(project.is_some_and(|p| Some(p.id) == target))
    }
}

/// `projects.active`: reports the active project, e.g. as the first step of a
/// workflow or as a key that always shows it.
#[derive(Debug)]
pub struct Active(ActionType);

impl Active {
    pub fn new() -> Self {
        Self(action_type(
            active_id(),
            "Get active project",
            "Reports the active project of the profile.",
        ))
    }
}

impl Default for Active {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for Active {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(&self, _: &ActionParameters, context: &ExecutionContext<'_>) -> ActionResult {
        match context.project {
            Some(project) => ActionResult::success(format!("{} ({})", project.key, project.name)),
            None => ActionResult::failure("no project is active"),
        }
    }

    fn decorate_face(
        &self,
        face: KeyFace,
        _: &ActionParameters,
        template: &str,
        project: Option<&Project>,
    ) -> KeyFace {
        with_project_detail(face, template, project)
    }
}

/// `projects.resolve-resource`: resolves a logical resource of the active
/// project on this machine and reports the value without acting on it.
#[derive(Debug)]
pub struct ResolveResource(ActionType);

impl ResolveResource {
    pub fn new() -> Self {
        let mut definition = action_type(
            resolve_resource_id(),
            "Resolve project resource",
            "Shows the value this computer binds to a resource of the active project.",
        );
        definition.parameters.push(ParameterSpec {
            key: "resource".into(),
            label: "Resource".into(),
            description: None,
            kind: ParameterKind::Resource { kind: None },
            required: true,
            default: None,
        });
        Self(definition)
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
    use deckpilot_core::{
        FaceStyle, MachineConfig, MachineResourceBinding, Os, ParameterValue, ProjectKey,
        ResourceKey, ResourceKind, ResourceRef,
    };

    fn fixture() -> (Profile, MachineConfig) {
        let mut profile = Profile::new("Work");
        for key in ["SHOP", "CMR", "CMS"] {
            profile
                .projects
                .push(Project::new(ProjectKey::new(key).unwrap(), key));
        }
        profile.projects[0]
            .resources
            .insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
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
        project: Option<usize>,
    ) -> ActionResult {
        let (profile, machine) = fixture();
        let params = handler.definition().normalize(params).unwrap();
        let context = ExecutionContext {
            profile: &profile,
            project: project.map(|index| &profile.projects[index]),
            machine: &machine,
        };
        handler.execute(&params, &context)
    }

    // Project ids are random per fixture, so compare positions via keys.
    fn selected_key(handler: &dyn ActionHandler, project: Option<usize>) -> Option<String> {
        let (profile, machine) = fixture();
        let context = ExecutionContext {
            profile: &profile,
            project: project.map(|index| &profile.projects[index]),
            machine: &machine,
        };
        let result = handler.execute(&ActionParameters::new(), &context);
        match result.effects.first()? {
            ContextEffect::SelectProject { project } => {
                profile.project(*project).map(|p| p.key.to_string())
            }
        }
    }

    #[test]
    fn next_and_previous_follow_the_order() {
        let next = Step::new(CycleDirection::Next, true);
        let previous = Step::new(CycleDirection::Previous, true);
        assert_eq!(selected_key(&next, Some(0)).as_deref(), Some("CMR"));
        assert_eq!(selected_key(&next, Some(2)).as_deref(), Some("SHOP"));
        assert_eq!(selected_key(&previous, Some(0)).as_deref(), Some("CMS"));
        assert_eq!(selected_key(&next, None).as_deref(), Some("SHOP"));
    }

    #[test]
    fn without_wrap_around_the_ends_are_reported() {
        let next = Step::new(CycleDirection::Next, false);
        let previous = Step::new(CycleDirection::Previous, false);
        let result = run(&next, &ActionParameters::new(), Some(2));
        assert_eq!(result.message, "already at the last project");
        assert_eq!(result.effects, Vec::new());
        assert!(!run(&previous, &ActionParameters::new(), Some(0)).is_success());
        assert_eq!(selected_key(&next, Some(1)).as_deref(), Some("CMS"));
    }

    #[test]
    fn switching_without_projects_fails() {
        let profile = Profile::new("Empty");
        let machine = MachineConfig::new(Os::Linux);
        let context = ExecutionContext {
            profile: &profile,
            project: None,
            machine: &machine,
        };
        let next = Step::new(CycleDirection::Next, true);
        let result = next.execute(&ActionParameters::new(), &context);
        assert_eq!(result.message, "this profile has no projects");
    }

    #[test]
    fn select_validates_profile_ownership() {
        let (profile, machine) = fixture();
        let select = Select::new();
        let cms = profile.projects[2].id;
        let own = ActionParameters::new().with("project", ParameterValue::Project(cms));
        assert_eq!(select.check(&own, &profile), Vec::<String>::new());
        let context = ExecutionContext {
            profile: &profile,
            project: None,
            machine: &machine,
        };
        let result = select.execute(&own, &context);
        assert_eq!(
            result.effects,
            vec![ContextEffect::SelectProject { project: cms }]
        );

        let (other, _) = fixture();
        let foreign =
            ActionParameters::new().with("project", ParameterValue::Project(other.projects[0].id));
        assert_eq!(select.check(&foreign, &profile).len(), 1);
        let result = select.execute(&foreign, &context);
        assert!(!result.is_success());
        assert_eq!(result.effects, Vec::new());
        assert!(
            select
                .definition()
                .normalize(&ActionParameters::new())
                .is_err()
        );
    }

    #[test]
    fn active_reports_the_project() {
        let result = run(&Active::new(), &ActionParameters::new(), Some(1));
        assert_eq!(result.message, "CMR (CMR)");
        assert!(!run(&Active::new(), &ActionParameters::new(), None).is_success());
    }

    #[test]
    fn resolve_resource_uses_the_active_project() {
        let params = ActionParameters::new().with(
            "resource",
            ParameterValue::Resource(ResourceRef::ProjectRole {
                role: "repository".into(),
            }),
        );
        let result = run(&ResolveResource::new(), &params, Some(0));
        assert!(result.is_success(), "{}", result.message);
        assert!(result.message.ends_with("/src/shop"));
        assert!(!run(&ResolveResource::new(), &params, None).is_success());
        assert!(!run(&ResolveResource::new(), &params, Some(1)).is_success());
    }

    #[test]
    fn faces_show_the_project_context() {
        let (profile, _) = fixture();
        let face = || KeyFace::new(FaceStyle::Action, "Next");
        let next = Step::new(CycleDirection::Next, true);
        let shop = Some(&profile.projects[0]);
        let decorated = next.decorate_face(face(), &ActionParameters::new(), "Next", shop);
        assert_eq!(decorated.detail.as_deref(), Some("SHOP"));
        let templated = next.decorate_face(face(), &ActionParameters::new(), "{project.key}", shop);
        assert_eq!(templated.detail, None);

        let select = Select::new();
        let params = ActionParameters::new()
            .with("project", ParameterValue::Project(profile.projects[0].id));
        assert!(select.decorate_face(face(), &params, "", shop).highlighted);
        assert!(
            !select
                .decorate_face(face(), &params, "", Some(&profile.projects[1]))
                .highlighted
        );
    }
}
