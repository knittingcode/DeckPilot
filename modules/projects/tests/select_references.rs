//! `projects.select` with a reference instead of a fixed project: a typed
//! `projectId` shared variable or `system.project.current.id`, resolved by
//! the engine when the key is pressed. Cross-profile targets never become
//! active.

use std::sync::Arc;

use deckpilot_core::variables::doubles::MemoryVariables;
use deckpilot_core::variables::{
    ScopeKey, VariableDefinition, VariableKey, VariableRef, VariableScope, VariableType,
    VariableValue,
};
use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActionResult, ActiveProjectContext, Engine, MachineConfig,
    ModuleRegistry, Os, ParameterValue, Profile, Project, ProjectId, ProjectKey,
};
use deckpilot_module_projects::ProjectsModule;
use deckpilot_module_projects::actions::{PROJECT_ID, select_id};

fn key(text: &str) -> VariableKey {
    VariableKey::new(text).unwrap()
}

struct Fixture {
    engine: Engine,
    profile: Profile,
    machine: MachineConfig,
}

/// A profile with SHOP and CMR and a `projects.favorite` variable whose
/// profile value `favorite` picks (none: no value).
fn fixture(favorite: impl Fn(&Profile) -> Option<ProjectId>) -> Fixture {
    let mut modules = ModuleRegistry::new(Os::Linux);
    modules.register(Box::new(ProjectsModule::new())).unwrap();
    let mut profile = Profile::new("Work");
    for project in ["SHOP", "CMR"] {
        profile
            .projects
            .push(Project::new(ProjectKey::new(project).unwrap(), project));
    }
    let mut variables = MemoryVariables::new(profile.clone()).define(VariableDefinition::new(
        key("projects.favorite"),
        VariableType::ProjectId,
        &[VariableScope::Profile],
    ));
    if let Some(project) = favorite(&profile) {
        variables = variables.set(
            &key("projects.favorite"),
            ScopeKey::Profile {
                profile: profile.id,
            },
            VariableValue::ProjectId(project),
        );
    }
    Fixture {
        engine: Engine::new(modules.action_registry())
            .with_variables(Arc::new(variables))
            .with_variable_registry(Arc::new(modules.variables().clone())),
        profile,
        machine: MachineConfig::new(Os::Linux),
    }
}

impl Fixture {
    fn select(&mut self, value: ParameterValue) -> Result<ActionInstanceId, String> {
        let parameters = ActionParameters::new().with(PROJECT_ID, value);
        let instance = self
            .engine
            .registry()
            .create_instance("Favorite", select_id(), &parameters)
            .map_err(|e| e.to_string())?;
        let id = instance.id;
        self.profile.actions.push(instance);
        Ok(id)
    }

    fn reference(&mut self, reference: &str) -> ActionInstanceId {
        self.select(ParameterValue::Reference(
            VariableRef::parse(reference).unwrap(),
        ))
        .unwrap()
    }

    fn press(&self, action: ActionInstanceId, context: &mut ActiveProjectContext) -> ActionResult {
        self.engine
            .run_action(&self.profile, &self.machine, context, action)
            .unwrap()
    }
}

#[test]
fn a_shared_project_variable_selects_its_project() {
    let mut f = fixture(|profile| Some(profile.projects[1].id));
    let cmr = f.profile.projects[1].id;
    let favorite = f.reference("shared.projects.favorite");
    let mut context = ActiveProjectContext::new(&f.profile);
    assert_eq!(context.project_id(), Some(f.profile.projects[0].id));
    let result = f.press(favorite, &mut context);
    assert!(result.is_success(), "{}", result.message);
    assert_eq!(result.message, "selected CMR");
    assert_eq!(context.project_id(), Some(cmr));
}

#[test]
fn the_current_project_reference_keeps_the_selection() {
    let mut f = fixture(|_| None);
    let cmr = f.profile.projects[1].id;
    // The legacy id and the module's own id, both typed project ids.
    for text in ["system.project.current.id", "projects.current.id"] {
        let current = f.reference(text);
        let mut context = ActiveProjectContext::restore(&f.profile, Some(cmr));
        let result = f.press(current, &mut context);
        assert!(result.is_success(), "{text}: {}", result.message);
        assert_eq!(context.project_id(), Some(cmr));
    }
}

#[test]
fn a_missing_value_changes_nothing() {
    let mut f = fixture(|_| None);
    let favorite = f.reference("shared.projects.favorite");
    let mut context = ActiveProjectContext::new(&f.profile);
    let before = context.project_id();
    let result = f.press(favorite, &mut context);
    assert_eq!(result.code.as_deref(), Some("reference-missing"));
    assert_eq!(
        result.message,
        "parameter `project_id`: {{ shared.projects.favorite }} has no value (no value is set)"
    );
    assert_eq!(context.project_id(), before);
}

#[test]
fn another_profiles_project_never_becomes_active() {
    let garden = Project::new(ProjectKey::new("GARDEN").unwrap(), "Garden");
    let garden_id = garden.id;
    let mut f = fixture(|_| None);
    // A default that names a project of another profile, as a broken
    // import might leave it.
    let variables = MemoryVariables::new(f.profile.clone()).define(VariableDefinition {
        default: Some(VariableValue::ProjectId(garden_id)),
        ..VariableDefinition::new(
            key("projects.favorite"),
            VariableType::ProjectId,
            &[VariableScope::Profile],
        )
    });
    let mut modules = ModuleRegistry::new(Os::Linux);
    modules.register(Box::new(ProjectsModule::new())).unwrap();
    f.engine = Engine::new(modules.action_registry())
        .with_variables(Arc::new(variables))
        .with_variable_registry(Arc::new(modules.variables().clone()));
    let favorite = f.reference("shared.projects.favorite");
    let mut context = ActiveProjectContext::new(&f.profile);
    let before = context.project_id();
    let result = f.press(favorite, &mut context);
    assert!(!result.is_success());
    assert!(
        result.message.contains("another profile"),
        "{}",
        result.message
    );
    assert_eq!(context.project_id(), before);
}

#[test]
fn only_project_typed_references_are_accepted() {
    let mut f = fixture(|_| None);
    let key_reference =
        ParameterValue::Reference(VariableRef::parse("system.project.current.key").unwrap());
    assert!(f.select(key_reference).is_err());
    let template = ParameterValue::Template("{{ shared.projects.favorite }}".into());
    assert!(f.select(template).is_err());
}
