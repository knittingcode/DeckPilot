//! Action types of the projects module.
//!
//! Every type is generic: it works for any project of the profile, chosen
//! through the optional `project_id` parameter (a stable project id, or a
//! reference to a `projectId` variable such as `shared.projects.favorite`,
//! which the engine resolves before the handler runs). Leaving it unset
//! means "the current project", resolved when the action runs. The
//! module never creates action types or action instances for individual
//! projects or resources.
//!
//! Handlers never change state themselves. Switching actions request a
//! [`ContextEffect::SelectProject`] or [`ContextEffect::CycleProject`]; the
//! engine applies it only after checking that the project belongs to the
//! active profile. A cycle is applied against the project active when the
//! engine applies it, so presses handled one after the other never skip or
//! repeat a project.
//!
//! `projects.next`, `projects.previous` and `projects.active` are
//! [`deprecated`](ActionType::deprecated): saved actions of these types keep
//! working, but new ones use `projects.cycle` and `projects.current`.

use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, ContextEffect,
    CycleDirection, ExecutionContext, KeyFace, ParameterKind, ParameterSpec, ParameterValue,
    Profile, Project, ProjectSwitcher,
};

use crate::resources::{self, DOCUMENTATION_ROLE, REPOSITORY_ROLE};

pub const CYCLE: &str = "projects.cycle";
pub const SELECT: &str = "projects.select";
pub const CURRENT: &str = "projects.current";
pub const REPOSITORY: &str = "projects.repository";
pub const DOCUMENTATION: &str = "projects.documentation";
pub const RESOLVE_RESOURCE: &str = "projects.resolve-resource";
/// Deprecated: `projects.cycle` with direction `next`.
pub const NEXT: &str = "projects.next";
/// Deprecated: `projects.cycle` with direction `previous`.
pub const PREVIOUS: &str = "projects.previous";
/// Deprecated: `projects.current`.
pub const ACTIVE: &str = "projects.active";

/// Optional project parameter shared by the project-related types. Unset
/// means the current project.
pub const PROJECT_ID: &str = "project_id";
/// Resource name parameter of `projects.resolve-resource`.
pub const RESOURCE: &str = "resource";
/// Direction parameter of `projects.cycle`: [`NEXT_DIRECTION`] or
/// [`PREVIOUS_DIRECTION`].
pub const DIRECTION: &str = "direction";
pub const NEXT_DIRECTION: &str = "next";
pub const PREVIOUS_DIRECTION: &str = "previous";

/// # Panics
///
/// Never: the identifiers are valid constants.
fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("projects action type ids are valid")
}

pub fn cycle_id() -> ActionTypeId {
    id(CYCLE)
}

pub fn current_id() -> ActionTypeId {
    id(CURRENT)
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

/// Handlers for all action types of the module; the deprecated ones last.
pub fn handlers(wrap_around: bool) -> Vec<Box<dyn ActionHandler>> {
    vec![
        Box::new(Cycle::new(wrap_around)),
        Box::new(Select::new()),
        Box::new(Current::new()),
        Box::new(ProjectResource::repository()),
        Box::new(ProjectResource::documentation()),
        Box::new(ProjectResource::custom()),
        Box::new(Step::new(CycleDirection::Next, wrap_around)),
        Box::new(Step::new(CycleDirection::Previous, wrap_around)),
        Box::new(Active::new()),
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
        deprecated: false,
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
/// Switching the active project changes state, so those actions modify.
fn node(outputs: Vec<OutputSpec>, changes_context: bool, repeatable: bool) -> NodeSpec {
    NodeSpec {
        icon: Some("folder".into()),
        outputs,
        timeout_ms: 5_000,
        retry_safe: repeatable,
        idempotent: repeatable,
        changes_context,
        effect: if changes_context {
            ActionEffect::Modifies
        } else {
            ActionEffect::ReadOnly
        },
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

/// One step through the profile's project order, as reported by
/// `projects.cycle`, `projects.next` and `projects.previous`.
///
/// The result names the project the step leads to from the project the
/// handler sees and requests a [`ContextEffect::CycleProject`]; the engine
/// steps again from the project active when it applies the effect and
/// corrects the reported project if that differs. Without projects the
/// action fails with `no-projects`; with a single project that is already
/// active it succeeds without changing anything. Without wrap-around the
/// ends fail with `at-last-project` / `at-first-project`. A deleted active
/// project starts from the first (or last) project.
fn cycle(context: &ExecutionContext<'_>, direction: CycleDirection, wrap: bool) -> ActionResult {
    let (profile, current) = match (context.profile(), context.project()) {
        (Ok(profile), Ok(project)) => (profile, project),
        (Err(denied), _) | (_, Err(denied)) => return denied.into(),
    };
    if let ([only], Some(current)) = (profile.projects.as_slice(), current)
        && only.id == current.id
    {
        return resources::with_project(
            ActionResult::success(format!("{} is the only project", current.key)),
            current,
        );
    }
    let target =
        match ProjectSwitcher::step(&profile.projects, current.map(|p| p.id), direction, wrap) {
            Ok(target) => target,
            Err(error) => return ActionResult::error(error.code(), error.to_string()),
        };
    match profile.project(target) {
        Some(next) => resources::with_project(
            ActionResult::success(format!("switched to {}", next.key)),
            next,
        )
        .with_effect(ContextEffect::CycleProject {
            direction,
            wrap_around: wrap,
        }),
        None => ActionResult::error("no-projects", "this profile has no projects"),
    }
}

/// The `direction` parameter of `projects.cycle`.
fn direction_parameter() -> ParameterSpec {
    ParameterSpec {
        key: DIRECTION.into(),
        label: "Direction".into(),
        description: Some(
            "Next or previous project in the profile's order. Whether the ends wrap around is \
             a setting of the Projects module."
                .into(),
        ),
        kind: ParameterKind::Choice {
            options: vec![NEXT_DIRECTION.into(), PREVIOUS_DIRECTION.into()],
        },
        required: true,
        default: Some(ParameterValue::Choice(NEXT_DIRECTION.into())),
    }
}

/// `projects.cycle`: makes the next or previous project of the profile
/// active, one reusable action for both directions.
#[derive(Debug)]
pub struct Cycle {
    definition: ActionType,
    wrap_around: bool,
}

impl Cycle {
    pub fn new(wrap_around: bool) -> Self {
        let mut definition = action_type(
            cycle_id(),
            "Cycle projects",
            "Makes the next or previous project of the profile active.",
        );
        definition.parameters.push(direction_parameter());
        // Cycling twice moves two projects on: neither idempotent nor safe to retry.
        definition.node = node(project_outputs(), true, false);
        Self {
            definition,
            wrap_around,
        }
    }
}

impl ActionHandler for Cycle {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let direction = match parameters.text(DIRECTION) {
            None | Some(NEXT_DIRECTION) => CycleDirection::Next,
            Some(PREVIOUS_DIRECTION) => CycleDirection::Previous,
            Some(_) => {
                return ActionResult::error("invalid-parameters", "unknown direction");
            }
        };
        cycle(context, direction, self.wrap_around)
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

/// `projects.current`: reports the current project — its id, key and name
/// and the profile it belongs to — e.g. as the first step of a workflow or
/// as a key that always shows it. Never changes state. In a workflow the
/// current project is the run's own project.
#[derive(Debug)]
pub struct Current(ActionType);

impl Current {
    pub fn new() -> Self {
        let mut definition = action_type(
            current_id(),
            "Current project",
            "Reports the current project of the profile.",
        );
        let mut outputs = project_outputs();
        outputs.push(output("profileId", "Profile id", ValueKind::Text));
        definition.node = node(outputs, false, true);
        Self(definition)
    }
}

impl Default for Current {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionHandler for Current {
    fn definition(&self) -> &ActionType {
        &self.0
    }

    fn execute(&self, _: &ActionParameters, context: &ExecutionContext<'_>) -> ActionResult {
        let (profile, project) = match (context.profile(), context.project()) {
            (Ok(profile), Ok(project)) => (profile, project),
            (Err(denied), _) | (_, Err(denied)) => return denied.into(),
        };
        match project {
            Some(project) => resources::with_project(
                ActionResult::success(format!("{} ({})", project.key, project.name)),
                project,
            )
            .with_data("profileId", profile.id.to_string()),
            None => ActionResult::error("no-active-project", "no project is active"),
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

/// `projects.next` and `projects.previous` (deprecated, see
/// [`Cycle`]): move through the profile's project order.
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
            deprecated: true,
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
        cycle(context, self.direction, self.wrap_around)
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
            Err(failure) => return failure,
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

/// `projects.active` (deprecated, see [`Current`]): reports the active
/// project. Never changes state.
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
        definition.deprecated = true;
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
            Ok(None) => ActionResult::error("no-active-project", "no project is active"),
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
    fn switching_actions_modify_and_reading_ones_do_not() {
        for handler in handlers(true) {
            let node = &handler.definition().node;
            let expected = if node.changes_context {
                ActionEffect::Modifies
            } else {
                ActionEffect::ReadOnly
            };
            assert_eq!(node.effect, expected, "{}", handler.definition().id);
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
                CYCLE,
                SELECT,
                CURRENT,
                REPOSITORY,
                DOCUMENTATION,
                RESOLVE_RESOURCE,
                NEXT,
                PREVIOUS,
                ACTIVE,
            ]
        );
        // Next, previous and active are replaced by cycle and current: kept
        // for saved actions, not offered for new ones.
        let deprecated: Vec<_> = handlers(true)
            .iter()
            .filter(|h| h.definition().deprecated)
            .map(|h| h.definition().id.to_string())
            .collect();
        assert_eq!(deprecated, [NEXT, PREVIOUS, ACTIVE]);
        // Projects are parameters, never part of a type; the project is
        // optional everywhere and defaults to the current one.
        for handler in handlers(true) {
            let definition = handler.definition();
            if let Some(spec) = definition.parameters.iter().find(|p| p.key == PROJECT_ID) {
                assert!(!spec.required, "{}", definition.id);
            }
        }
    }

    fn direction(value: &str) -> ActionParameters {
        ActionParameters::new().with(DIRECTION, ParameterValue::Choice(value.into()))
    }

    #[test]
    fn cycle_moves_in_either_direction_and_wraps() {
        let cycle = Cycle::new(true);
        let result = run(&cycle, &ActionParameters::new(), Some(0));
        // Next is the default direction.
        assert_eq!(result.data["projectKey"], "CMR");
        assert_eq!(result.message, "switched to CMR");
        assert_eq!(
            result.effects,
            vec![ContextEffect::CycleProject {
                direction: CycleDirection::Next,
                wrap_around: true,
            }]
        );
        assert_eq!(
            run(&cycle, &direction(NEXT_DIRECTION), Some(2)).data["projectKey"],
            "SHOP"
        );
        let back = run(&cycle, &direction(PREVIOUS_DIRECTION), Some(0));
        assert_eq!(back.data["projectKey"], "CMS");
        assert_eq!(
            back.effects,
            vec![ContextEffect::CycleProject {
                direction: CycleDirection::Previous,
                wrap_around: true,
            }]
        );
        // Without an active project (e.g. it was deleted) the first one
        // follows.
        assert_eq!(
            run(&cycle, &ActionParameters::new(), None).data["projectKey"],
            "SHOP"
        );
        assert!(
            cycle
                .definition()
                .normalize(&direction("sideways"))
                .is_err()
        );
    }

    #[test]
    fn cycle_follows_the_wrap_setting() {
        let cycle = Cycle::new(false);
        let last = run(&cycle, &ActionParameters::new(), Some(2));
        assert_eq!(last.code.as_deref(), Some("at-last-project"));
        assert_eq!(last.effects, Vec::new());
        let first = run(&cycle, &direction(PREVIOUS_DIRECTION), Some(0));
        assert_eq!(first.code.as_deref(), Some("at-first-project"));
        assert_eq!(
            run(&cycle, &ActionParameters::new(), Some(1)).effects,
            vec![ContextEffect::CycleProject {
                direction: CycleDirection::Next,
                wrap_around: false,
            }]
        );
    }

    #[test]
    fn cycle_without_projects_fails_and_with_one_changes_nothing() {
        let machine = MachineConfig::new(Os::Linux);
        let empty = Profile::new("Empty");
        let cycle = Cycle::new(true);
        let result = run_in(&empty, &machine, &cycle, &ActionParameters::new(), None);
        assert!(!result.is_success());
        assert_eq!(result.code.as_deref(), Some("no-projects"));
        assert_eq!(result.effects, Vec::new());

        let mut single = Profile::new("Single");
        single
            .projects
            .push(Project::new(ProjectKey::new("ONE").unwrap(), "One"));
        for wrap in [true, false] {
            for value in [NEXT_DIRECTION, PREVIOUS_DIRECTION] {
                let result = run_in(
                    &single,
                    &machine,
                    &Cycle::new(wrap),
                    &direction(value),
                    Some(0),
                );
                assert!(result.is_success(), "{}", result.message);
                assert_eq!(result.message, "ONE is the only project");
                assert_eq!(result.data["projectKey"], "ONE");
                assert_eq!(result.effects, Vec::new());
            }
        }
        // A single project that is not active yet becomes active.
        let result = run_in(&single, &machine, &cycle, &ActionParameters::new(), None);
        assert_eq!(result.data["projectKey"], "ONE");
        assert_eq!(result.effects.len(), 1);
    }

    #[test]
    fn current_reports_typed_project_outputs() {
        let (profile, machine) = fixture();
        let current = Current::new();
        let result = run_in(
            &profile,
            &machine,
            &current,
            &ActionParameters::new(),
            Some(1),
        );
        assert!(result.is_success());
        assert_eq!(result.message, "CMR (CMR)");
        assert_eq!(result.effects, Vec::new());
        assert_eq!(result.data["projectId"], profile.projects[1].id.to_string());
        assert_eq!(result.data["projectKey"], "CMR");
        assert_eq!(result.data["projectName"], "CMR");
        assert_eq!(result.data["profileId"], profile.id.to_string());
        // Every declared output is set, with its kind.
        let outputs = &current.definition().node.outputs;
        for output in outputs {
            assert!(result.data.contains_key(&output.key), "{}", output.key);
            assert!(output.always);
        }
        let kinds: Vec<_> = outputs.iter().map(|o| (o.key.as_str(), o.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("projectId", ValueKind::Text),
                ("projectKey", ValueKind::ProjectKey),
                ("projectName", ValueKind::Text),
                ("profileId", ValueKind::Text),
            ]
        );
        assert!(!current.definition().node.changes_context);

        let none = run_in(&profile, &machine, &current, &ActionParameters::new(), None);
        assert_eq!(none.code.as_deref(), Some("no-active-project"));
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
        assert_eq!(result.code.as_deref(), Some("at-last-project"));
        assert_eq!(result.effects, Vec::new());
        let first = run(&previous, &ActionParameters::new(), Some(0));
        assert_eq!(first.code.as_deref(), Some("at-first-project"));
        assert_eq!(switched_to(&next, Some(1)).as_deref(), Some("CMS"));
    }

    #[test]
    fn switching_without_projects_fails() {
        let profile = Profile::new("Empty");
        let machine = MachineConfig::new(Os::Linux);
        let next = Step::new(CycleDirection::Next, true);
        let result = run_in(&profile, &machine, &next, &ActionParameters::new(), None);
        assert_eq!(result.message, "this profile has no projects");
        assert_eq!(result.code.as_deref(), Some("no-projects"));
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
        assert_eq!(none.code.as_deref(), Some("no-active-project"));
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
        assert_eq!(result.code.as_deref(), Some("project-not-in-profile"));
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
        assert_eq!(unbound.code.as_deref(), Some("resource-unbound"));
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
        assert_eq!(undefined.code.as_deref(), Some("resource-role-missing"));
        let no_project = run(
            &ProjectResource::repository(),
            &ActionParameters::new(),
            None,
        );
        assert_eq!(no_project.message, "no project is active");
        assert_eq!(no_project.code.as_deref(), Some("no-active-project"));

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
        assert_eq!(result.code.as_deref(), Some("not-permitted"));
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
