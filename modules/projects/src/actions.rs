//! Action types of the projects module.
//!
//! Every type is generic: it works for any project of the profile, chosen
//! through the optional `project_id` parameter (a stable project id). Leaving
//! it unset means "the current project", resolved when the action runs. The
//! module never creates action types or action instances for individual
//! projects or resources.
//!
//! Handlers never change state themselves. Switching actions request a
//! [`ContextEffect::SelectProject`]; the engine applies it only after checking
//! that the project belongs to the active profile.

use deckpilot_core::graph::{NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, ContextEffect,
    CycleDirection, ExecutionContext, KeyFace, ParameterKind, ParameterSpec, Profile, Project,
};

use crate::resources::{self, DOCUMENTATION_ROLE, REPOSITORY_ROLE};

pub const NEXT: &str = "projects.next";
pub const PREVIOUS: &str = "projects.previous";
pub const SELECT: &str = "projects.select";
pub const ACTIVE: &str = "projects.active";
pub const REPOSITORY: &str = "projects.repository";
pub const DOCUMENTATION: &str = "projects.documentation";
pub const RESOLVE_RESOURCE: &str = "projects.resolve-resource";

/// Optional project parameter shared by the project-related types. Unset
/// means the current project.
pub const PROJECT_ID: &str = "project_id";
/// Resource name parameter of `projects.resolve-resource`.
pub const RESOURCE: &str = "resource";

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

pub fn repository_id() -> ActionTypeId {
    id(REPOSITORY)
}

pub fn documentation_id() -> ActionTypeId {
    id(DOCUMENTATION)
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
        Box::new(ProjectResource::repository()),
        Box::new(ProjectResource::documentation()),
        Box::new(ProjectResource::custom()),
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
        node: NodeSpec::default(),
    }
}

fn output(key: &str, label: &str, kind: ValueKind) -> OutputSpec {
    OutputSpec {
        key: key.into(),
        label: label.into(),
        kind,
        always: true,
        sensitive: false,
    }
}

/// Outputs of actions that report a project (`resources::with_project`).
fn project_outputs() -> Vec<OutputSpec> {
    vec![
        output("projectId", "Project id", ValueKind::Text),
        output("projectKey", "Project key", ValueKind::ProjectKey),
        output("projectName", "Project name", ValueKind::Text),
    ]
}

/// Outputs of the resource actions (`resources::resolve`).
fn resource_outputs() -> Vec<OutputSpec> {
    let mut outputs = project_outputs();
    outputs.extend([
        output("resource", "Resource name", ValueKind::Text),
        output("resourceKey", "Resource key", ValueKind::Text),
        output("kind", "Resource kind", ValueKind::Text),
        output("value", "Value on this computer", ValueKind::Text),
    ]);
    outputs
}

/// Node behaviour of the project actions: which outputs they provide,
/// whether they switch the project, and whether repeating them is safe.
fn node(outputs: Vec<OutputSpec>, changes_context: bool, repeatable: bool) -> NodeSpec {
    NodeSpec {
        icon: Some("folder-kanban".into()),
        outputs,
        timeout_ms: 5_000,
        retry_safe: repeatable,
        idempotent: repeatable,
        changes_context,
        ..NodeSpec::default()
    }
}

fn project_parameter() -> ParameterSpec {
    ParameterSpec {
        key: PROJECT_ID.into(),
        label: "Project".into(),
        description: Some("Leave on “Current project” to use whichever project is active.".into()),
        kind: ParameterKind::Project,
        required: false,
        default: None,
    }
}

/// Reports a `project_id` that does not belong to `profile`.
fn check_project(parameters: &ActionParameters, profile: &Profile) -> Vec<String> {
    match parameters.project(PROJECT_ID) {
        Some(id) if profile.project(id).is_none() => {
            vec!["the selected project does not belong to this profile".into()]
        }
        _ => Vec::new(),
    }
}

/// Shows the project an action refers to below its label, unless the label
/// already names it (a template, or an action fixed to one project).
fn with_project_detail(
    face: KeyFace,
    parameters: &ActionParameters,
    template: &str,
    active: Option<&Project>,
) -> KeyFace {
    if template.contains("{project.") || parameters.project(PROJECT_ID).is_some() {
        face
    } else {
        face.with_detail(active.map_or("-", |p| p.key.as_str()))
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
        // Cycling twice moves two projects on: neither idempotent nor safe to retry.
        let definition = ActionType {
            node: node(project_outputs(), true, false),
            ..definition
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
        let (profile, project) = match (context.profile(), context.project()) {
            (Ok(profile), Ok(project)) => (profile, project),
            (Err(denied), _) | (_, Err(denied)) => return denied.into(),
        };
        let target = match crate::switching::step(
            profile,
            project.map(|p| p.id),
            self.direction,
            self.wrap_around,
        ) {
            Ok(target) => target,
            Err(error) => return ActionResult::failure(error.to_string()),
        };
        match profile.project(target) {
            Some(next) => resources::with_project(
                ActionResult::success(format!("switched to {}", next.key)),
                next,
            )
            .with_effect(ContextEffect::SelectProject { project: target }),
            None => ActionResult::failure("this profile has no projects"),
        }
    }

    fn decorate_face(
        &self,
        face: KeyFace,
        parameters: &ActionParameters,
        template: &str,
        project: Option<&Project>,
    ) -> KeyFace {
        with_project_detail(face, parameters, template, project)
    }
}

/// `projects.select`: makes a project of the profile active. Without a
/// `project_id` it reports the current project and changes nothing.
#[derive(Debug)]
pub struct Select(ActionType);

impl Select {
    pub fn new() -> Self {
        let mut definition = action_type(
            select_id(),
            "Select project",
            "Makes a project of the profile active. Without a project it reports the current one.",
        );
        definition.parameters.push(project_parameter());
        definition.node = node(project_outputs(), true, true);
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
        let project = match resources::target_project(parameters, context) {
            Ok(project) => project,
            Err(message) => return ActionResult::failure(message),
        };
        match parameters.project(PROJECT_ID) {
            Some(id) => resources::with_project(
                ActionResult::success(format!("selected {}", project.key)),
                project,
            )
            .with_effect(ContextEffect::SelectProject { project: id }),
            None => resources::with_project(
                ActionResult::success(format!("{} is active", project.key)),
                project,
            ),
        }
    }

    fn check(&self, parameters: &ActionParameters, profile: &Profile) -> Vec<String> {
        check_project(parameters, profile)
    }

    fn decorate_face(
        &self,
        face: KeyFace,
        parameters: &ActionParameters,
        template: &str,
        project: Option<&Project>,
    ) -> KeyFace {
        match parameters.project(PROJECT_ID) {
            Some(target) => face.highlighted(project.is_some_and(|p| p.id == target)),
            None => with_project_detail(face, parameters, template, project),
        }
    }
}

/// `projects.active`: reports the active project, e.g. as the first step of a
/// workflow or as a key that always shows it. Never changes state.
#[derive(Debug)]
pub struct Active(ActionType);

impl Active {
    pub fn new() -> Self {
        let mut definition = action_type(
            active_id(),
            "Get active project",
            "Reports the active project of the profile.",
        );
        definition.node = node(project_outputs(), false, true);
        Self(definition)
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
        match context.project() {
            Err(denied) => denied.into(),
            Ok(Some(project)) => resources::with_project(
                ActionResult::success(format!("{} ({})", project.key, project.name)),
                project,
            ),
            Ok(None) => ActionResult::failure("no project is active"),
        }
    }

    fn decorate_face(
        &self,
        face: KeyFace,
        parameters: &ActionParameters,
        template: &str,
        project: Option<&Project>,
    ) -> KeyFace {
        with_project_detail(face, parameters, template, project)
    }
}

/// Which resource a [`ProjectResource`] type resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Fixed(&'static str),
    /// Taken from the `resource` parameter.
    Configured,
}

/// `projects.repository`, `projects.documentation` and
/// `projects.resolve-resource`: resolve a resource of the selected (or
/// current) project on this machine and report it. All three use the same
/// resolver; nothing is opened or executed. Modules for Git, editors or
/// browsers can build on the result.
#[derive(Debug)]
pub struct ProjectResource {
    definition: ActionType,
    role: Role,
}

impl ProjectResource {
    pub fn repository() -> Self {
        let mut definition = action_type(
            repository_id(),
            "Project repository",
            "Resolves the repository of a project on this computer.",
        );
        definition.parameters.push(project_parameter());
        definition.node = node(resource_outputs(), false, true);
        Self {
            definition,
            role: Role::Fixed(REPOSITORY_ROLE),
        }
    }

    pub fn documentation() -> Self {
        let mut definition = action_type(
            documentation_id(),
            "Project documentation",
            "Resolves the documentation of a project on this computer.",
        );
        definition.parameters.push(project_parameter());
        definition.node = node(resource_outputs(), false, true);
        Self {
            definition,
            role: Role::Fixed(DOCUMENTATION_ROLE),
        }
    }

    pub fn custom() -> Self {
        let mut definition = action_type(
            resolve_resource_id(),
            "Resolve project resource",
            "Resolves a named resource of a project on this computer.",
        );
        definition.parameters.push(project_parameter());
        definition.parameters.push(ParameterSpec {
            key: RESOURCE.into(),
            label: "Resource".into(),
            description: Some(
                "The resource name in the project, for example repository or docs.".into(),
            ),
            kind: ParameterKind::Text {
                multiline: false,
                max_length: Some(64),
            },
            required: true,
            default: None,
        });
        definition.node = node(resource_outputs(), false, true);
        Self {
            definition,
            role: Role::Configured,
        }
    }
}

impl ActionHandler for ProjectResource {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let role = match self.role {
            Role::Fixed(role) => role,
            Role::Configured => parameters.text(RESOURCE).unwrap_or_default().trim(),
        };
        resources::resolve(parameters, context, role)
    }

    fn check(&self, parameters: &ActionParameters, profile: &Profile) -> Vec<String> {
        check_project(parameters, profile)
    }

    fn decorate_face(
        &self,
        face: KeyFace,
        parameters: &ActionParameters,
        template: &str,
        project: Option<&Project>,
    ) -> KeyFace {
        with_project_detail(face, parameters, template, project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::{
        Capability, FaceStyle, MachineConfig, MachineResourceBinding, Os, ParameterValue,
        ProjectId, ProjectKey, ResourceKey, ResourceKind,
    };

    fn fixture() -> (Profile, MachineConfig) {
        let mut profile = Profile::new("Work");
        for key in ["SHOP", "CMR", "CMS"] {
            profile
                .projects
                .push(Project::new(ProjectKey::new(key).unwrap(), key));
        }
        let resources = &mut profile.projects[0].resources;
        resources.insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
        resources.insert("docs".into(), ResourceKey::new("shop.docs").unwrap());
        resources.insert("ci".into(), ResourceKey::new("shop.ci").unwrap());
        profile.projects[1]
            .resources
            .insert("repository".into(), ResourceKey::new("cmr.repo").unwrap());
        let mut machine = MachineConfig::new(Os::Linux);
        for (key, kind, value) in [
            ("shop.repo", ResourceKind::Directory, "/src/shop"),
            ("shop.docs", ResourceKind::Url, "https://docs.example/shop"),
            ("shop.ci", ResourceKind::Url, "https://ci.example/shop"),
        ] {
            machine.set_binding(MachineResourceBinding {
                key: ResourceKey::new(key).unwrap(),
                kind,
                value: value.into(),
            });
        }
        (profile, machine)
    }

    fn project_param(id: ProjectId) -> ActionParameters {
        ActionParameters::new().with(PROJECT_ID, ParameterValue::Project(id))
    }

    fn run_in(
        profile: &Profile,
        machine: &MachineConfig,
        handler: &dyn ActionHandler,
        params: &ActionParameters,
        active: Option<usize>,
    ) -> ActionResult {
        let params = handler.definition().normalize(params).unwrap();
        let context = ExecutionContext::new(
            profile,
            active.map(|index| &profile.projects[index]),
            machine,
        );
        handler.execute(&params, &context)
    }

    fn run(
        handler: &dyn ActionHandler,
        params: &ActionParameters,
        active: Option<usize>,
    ) -> ActionResult {
        let (profile, machine) = fixture();
        run_in(&profile, &machine, handler, params, active)
    }

    /// Key of the project an action switched to, if it switched.
    fn switched_to(handler: &dyn ActionHandler, active: Option<usize>) -> Option<String> {
        let result = run(handler, &ActionParameters::new(), active);
        if result.effects.is_empty() {
            None
        } else {
            result.data.get("projectKey").cloned()
        }
    }

    #[test]
    fn the_catalog_is_generic() {
        let ids: Vec<_> = handlers(true)
            .iter()
            .map(|h| h.definition().id.to_string())
            .collect();
        assert_eq!(
            ids,
            [
                NEXT,
                PREVIOUS,
                SELECT,
                ACTIVE,
                REPOSITORY,
                DOCUMENTATION,
                RESOLVE_RESOURCE
            ]
        );
        // Projects are parameters, never part of a type; the project is
        // optional everywhere and defaults to the current one.
        for handler in handlers(true) {
            let definition = handler.definition();
            if let Some(spec) = definition.parameters.iter().find(|p| p.key == PROJECT_ID) {
                assert!(!spec.required, "{}", definition.id);
            }
        }
    }

    #[test]
    fn next_and_previous_follow_the_order() {
        let next = Step::new(CycleDirection::Next, true);
        let previous = Step::new(CycleDirection::Previous, true);
        assert_eq!(switched_to(&next, Some(0)).as_deref(), Some("CMR"));
        assert_eq!(switched_to(&next, Some(2)).as_deref(), Some("SHOP"));
        assert_eq!(switched_to(&previous, Some(0)).as_deref(), Some("CMS"));
        assert_eq!(switched_to(&next, None).as_deref(), Some("SHOP"));
    }

    #[test]
    fn without_wrap_around_the_ends_are_reported() {
        let next = Step::new(CycleDirection::Next, false);
        let previous = Step::new(CycleDirection::Previous, false);
        let result = run(&next, &ActionParameters::new(), Some(2));
        assert_eq!(result.message, "already at the last project");
        assert_eq!(result.effects, Vec::new());
        assert!(!run(&previous, &ActionParameters::new(), Some(0)).is_success());
        assert_eq!(switched_to(&next, Some(1)).as_deref(), Some("CMS"));
    }

    #[test]
    fn switching_without_projects_fails() {
        let profile = Profile::new("Empty");
        let machine = MachineConfig::new(Os::Linux);
        let next = Step::new(CycleDirection::Next, true);
        let result = run_in(&profile, &machine, &next, &ActionParameters::new(), None);
        assert_eq!(result.message, "this profile has no projects");
    }

    #[test]
    fn select_defaults_to_the_current_project_without_changing_it() {
        let result = run(&Select::new(), &ActionParameters::new(), Some(1));
        assert!(result.is_success());
        assert_eq!(result.effects, Vec::new());
        assert_eq!(result.data["projectKey"], "CMR");
        assert_eq!(result.message, "CMR is active");

        let none = run(&Select::new(), &ActionParameters::new(), None);
        assert_eq!(none.message, "no project is active");
    }

    #[test]
    fn select_validates_profile_ownership() {
        let (profile, machine) = fixture();
        let select = Select::new();
        let cms = profile.projects[2].id;
        assert_eq!(
            select.check(&project_param(cms), &profile),
            Vec::<String>::new()
        );
        let result = run_in(&profile, &machine, &select, &project_param(cms), Some(0));
        assert_eq!(
            result.effects,
            vec![ContextEffect::SelectProject { project: cms }]
        );
        assert_eq!(result.data["projectId"], cms.to_string());

        let (other, _) = fixture();
        let foreign = project_param(other.projects[0].id);
        assert_eq!(select.check(&foreign, &profile).len(), 1);
        let result = run_in(&profile, &machine, &select, &foreign, Some(0));
        assert_eq!(
            result.message,
            "the project does not belong to the active profile"
        );
        assert_eq!(result.effects, Vec::new());
    }

    #[test]
    fn active_reports_the_project_without_changing_it() {
        let result = run(&Active::new(), &ActionParameters::new(), Some(1));
        assert_eq!(result.message, "CMR (CMR)");
        assert_eq!(result.data["projectKey"], "CMR");
        assert_eq!(result.effects, Vec::new());
        assert!(!run(&Active::new(), &ActionParameters::new(), None).is_success());
    }

    #[test]
    fn repository_and_documentation_resolve_the_current_project() {
        let repository = run(
            &ProjectResource::repository(),
            &ActionParameters::new(),
            Some(0),
        );
        assert!(repository.is_success(), "{}", repository.message);
        assert_eq!(repository.data["value"], "/src/shop");
        assert_eq!(repository.data["resource"], "repository");
        assert_eq!(repository.data["resourceKey"], "shop.repo");
        assert_eq!(repository.data["kind"], "directory");
        assert_eq!(repository.data["projectKey"], "SHOP");
        assert_eq!(repository.effects, Vec::new());

        let docs = run(
            &ProjectResource::documentation(),
            &ActionParameters::new(),
            Some(0),
        );
        assert_eq!(docs.data["value"], "https://docs.example/shop");
        assert_eq!(docs.data["kind"], "url");
    }

    #[test]
    fn resources_of_an_explicit_project_and_custom_names() {
        let (profile, machine) = fixture();
        let shop = project_param(profile.projects[0].id);
        // An explicit project wins over the active one.
        let result = run_in(
            &profile,
            &machine,
            &ProjectResource::repository(),
            &shop,
            Some(2),
        );
        assert_eq!(result.data["value"], "/src/shop");

        let custom = shop
            .clone()
            .with(RESOURCE, ParameterValue::Text("ci".into()));
        let result = run_in(
            &profile,
            &machine,
            &ProjectResource::custom(),
            &custom,
            None,
        );
        assert_eq!(result.data["value"], "https://ci.example/shop");
        assert!(
            ProjectResource::custom()
                .definition()
                .normalize(&shop)
                .is_err()
        );
    }

    #[test]
    fn missing_resources_are_reported() {
        // CMR maps a repository that is not bound on this machine.
        let unbound = run(
            &ProjectResource::repository(),
            &ActionParameters::new(),
            Some(1),
        );
        assert!(
            unbound.message.contains("not bound on this machine"),
            "{}",
            unbound.message
        );
        // CMS has no documentation resource at all.
        let undefined = run(
            &ProjectResource::documentation(),
            &ActionParameters::new(),
            Some(2),
        );
        assert!(
            undefined.message.contains("does not define the resource"),
            "{}",
            undefined.message
        );
        let no_project = run(
            &ProjectResource::repository(),
            &ActionParameters::new(),
            None,
        );
        assert_eq!(no_project.message, "no project is active");

        let (profile, machine) = fixture();
        let (other, _) = fixture();
        let foreign = project_param(other.projects[0].id);
        let result = run_in(
            &profile,
            &machine,
            &ProjectResource::repository(),
            &foreign,
            Some(0),
        );
        assert_eq!(
            result.message,
            "the project does not belong to the active profile"
        );
    }

    #[test]
    fn resources_require_the_machine_capability() {
        let (profile, machine) = fixture();
        let granted = [Capability::ProfileRead].into_iter().collect();
        let context = ExecutionContext::new(&profile, Some(&profile.projects[0]), &machine)
            .restricted(&granted);
        let result = ProjectResource::repository().execute(&ActionParameters::new(), &context);
        assert!(!result.is_success());
        assert!(result.message.contains("not allowed"), "{}", result.message);
    }

    #[test]
    fn faces_show_the_project_context() {
        let (profile, _) = fixture();
        let face = || KeyFace::new(FaceStyle::Action, "Next");
        let next = Step::new(CycleDirection::Next, true);
        let shop = Some(&profile.projects[0]);
        let none = ActionParameters::new();
        let decorated = next.decorate_face(face(), &none, "Next", shop);
        assert_eq!(decorated.detail.as_deref(), Some("SHOP"));
        let templated = next.decorate_face(face(), &none, "{project.key}", shop);
        assert_eq!(templated.detail, None);

        let select = Select::new();
        let params = project_param(profile.projects[0].id);
        assert!(select.decorate_face(face(), &params, "", shop).highlighted);
        assert!(
            !select
                .decorate_face(face(), &params, "", Some(&profile.projects[1]))
                .highlighted
        );
        // Without a project, Select shows the current one like Active.
        let current = select.decorate_face(face(), &none, "Current", shop);
        assert_eq!(current.detail.as_deref(), Some("SHOP"));
    }
}
