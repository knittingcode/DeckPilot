//! `projects.cycle` and `projects.current` through the engine, as a key
//! press runs them: the effect is applied to the profile's active project
//! context, against whatever project is active when it is applied.

use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActiveProjectContext, ContextEffect, Engine, EngineError,
    FeatureModule, MachineConfig, ModuleRegistry, Os, ParameterValue, Profile, Project, ProjectKey,
};
use deckpilot_module_projects::actions::{
    self, DIRECTION, NEXT_DIRECTION, PREVIOUS_DIRECTION, cycle_id,
};
use deckpilot_module_projects::{ProjectsModule, WRAP_AROUND, module_id};

struct Fixture {
    engine: Engine,
    profile: Profile,
    next: ActionInstanceId,
    previous: ActionInstanceId,
    current: ActionInstanceId,
    machine: MachineConfig,
}

fn fixture_with(keys: &[&str], wrap_around: bool) -> Fixture {
    let mut modules = ModuleRegistry::new(Os::Linux);
    modules.register(Box::new(ProjectsModule::new())).unwrap();
    modules
        .set_config(
            &module_id(),
            &ActionParameters::new().with(WRAP_AROUND, ParameterValue::Boolean(wrap_around)),
        )
        .unwrap();
    let registry = modules.action_registry();
    let mut profile = Profile::new("Work");
    for key in keys {
        profile
            .projects
            .push(Project::new(ProjectKey::new(*key).unwrap(), *key));
    }
    let direction =
        |value: &str| ActionParameters::new().with(DIRECTION, ParameterValue::Choice(value.into()));
    let next = registry
        .create_instance("Next", cycle_id(), &direction(NEXT_DIRECTION))
        .unwrap();
    let previous = registry
        .create_instance("Previous", cycle_id(), &direction(PREVIOUS_DIRECTION))
        .unwrap();
    let current = registry
        .create_instance("Current", actions::current_id(), &ActionParameters::new())
        .unwrap();
    let ids = (next.id, previous.id, current.id);
    profile.actions.extend([next, previous, current]);
    Fixture {
        engine: Engine::new(registry),
        profile,
        next: ids.0,
        previous: ids.1,
        current: ids.2,
        machine: MachineConfig::new(Os::Linux),
    }
}

fn fixture() -> Fixture {
    fixture_with(&["SHOP", "CMR", "CMS"], true)
}

impl Fixture {
    fn press(
        &self,
        context: &mut ActiveProjectContext,
        action: ActionInstanceId,
    ) -> deckpilot_core::ActionResult {
        self.engine
            .run_action(&self.profile, &self.machine, context, action)
            .unwrap()
    }

    fn key(&self, context: &ActiveProjectContext) -> Option<String> {
        context
            .project(&self.profile)
            .unwrap()
            .map(|p| p.key.to_string())
    }
}

#[test]
fn presses_step_through_the_order_and_wrap_around() {
    let f = fixture();
    let mut context = ActiveProjectContext::new(&f.profile);
    let mut seen = Vec::new();
    for _ in 0..4 {
        let result = f.press(&mut context, f.next);
        assert!(result.is_success(), "{}", result.message);
        // The result reports the selection that was made, never the cycle.
        let active = context.project_id().unwrap();
        assert_eq!(
            result.effects,
            vec![ContextEffect::SelectProject { project: active }]
        );
        assert_eq!(result.data["projectId"], active.to_string());
        seen.push(f.key(&context).unwrap());
    }
    assert_eq!(seen, ["CMR", "CMS", "SHOP", "CMR"]);
    f.press(&mut context, f.previous);
    f.press(&mut context, f.previous);
    assert_eq!(f.key(&context).as_deref(), Some("CMS"));
}

#[test]
fn every_press_moves_exactly_one_project() {
    // Presses are applied one after the other to the same context, each
    // from the project the previous one left active.
    let f = fixture();
    let mut context = ActiveProjectContext::new(&f.profile);
    for presses in 1..=10 {
        f.press(&mut context, f.next);
        let expected = &f.profile.projects[presses % 3];
        assert_eq!(context.project_id(), Some(expected.id), "after {presses}");
    }
}

#[test]
fn without_wrap_around_the_ends_fail_without_change() {
    let f = fixture_with(&["SHOP", "CMR", "CMS"], false);
    let mut context = ActiveProjectContext::new(&f.profile);
    let result = f.press(&mut context, f.previous);
    assert_eq!(result.code.as_deref(), Some("at-first-project"));
    assert_eq!(f.key(&context).as_deref(), Some("SHOP"));
    f.press(&mut context, f.next);
    f.press(&mut context, f.next);
    let result = f.press(&mut context, f.next);
    assert_eq!(result.code.as_deref(), Some("at-last-project"));
    assert_eq!(f.key(&context).as_deref(), Some("CMS"));
}

#[test]
fn zero_and_one_project() {
    let empty = fixture_with(&[], true);
    let mut context = ActiveProjectContext::new(&empty.profile);
    let result = empty.press(&mut context, empty.next);
    assert!(!result.is_success());
    assert_eq!(result.code.as_deref(), Some("no-projects"));
    assert_eq!(result.effects, Vec::new());
    assert_eq!(context.project_id(), None);

    for wrap in [true, false] {
        let single = fixture_with(&["ONE"], wrap);
        let mut context = ActiveProjectContext::new(&single.profile);
        for action in [single.next, single.previous] {
            let result = single.press(&mut context, action);
            assert!(result.is_success(), "{}", result.message);
            assert_eq!(result.effects, Vec::new());
            assert_eq!(single.key(&context).as_deref(), Some("ONE"));
        }
    }
}

#[test]
fn a_deleted_active_project_starts_from_the_first() {
    let mut f = fixture();
    let mut context = ActiveProjectContext::new(&f.profile);
    let cmr = f.profile.projects[1].id;
    context.select(&f.profile, cmr).unwrap();
    // CMR is deleted while it is active; the context still names it.
    f.profile.projects.retain(|p| p.id != cmr);
    assert_eq!(context.project_id(), Some(cmr));
    f.press(&mut context, f.next);
    assert_eq!(f.key(&context).as_deref(), Some("SHOP"));

    context
        .select(&f.profile, f.profile.projects[0].id)
        .unwrap();
    let shop = f.profile.projects[0].id;
    f.profile.projects.retain(|p| p.id != shop);
    f.press(&mut context, f.previous);
    assert_eq!(f.key(&context).as_deref(), Some("CMS"));
}

#[test]
fn reordered_projects_are_followed() {
    let mut f = fixture();
    let mut context = ActiveProjectContext::new(&f.profile);
    f.press(&mut context, f.next);
    assert_eq!(f.key(&context).as_deref(), Some("CMR"));
    // New order: CMS, CMR, SHOP.
    f.profile.projects.reverse();
    f.press(&mut context, f.next);
    assert_eq!(f.key(&context).as_deref(), Some("SHOP"));
    f.press(&mut context, f.next);
    assert_eq!(f.key(&context).as_deref(), Some("CMS"));
}

#[test]
fn a_context_of_another_profile_is_never_changed() {
    let f = fixture();
    let other = fixture();
    let mut context = ActiveProjectContext::new(&other.profile);
    let before = context.clone();
    let error = f
        .engine
        .run_action(&f.profile, &f.machine, &mut context, f.next)
        .unwrap_err();
    assert!(matches!(error, EngineError::Context(_)), "{error:?}");
    assert_eq!(context, before);
}

#[test]
fn current_reports_the_active_project() {
    let f = fixture();
    let mut context = ActiveProjectContext::new(&f.profile);
    f.press(&mut context, f.next);
    let result = f.press(&mut context, f.current);
    assert!(result.is_success());
    assert_eq!(result.effects, Vec::new());
    assert_eq!(result.data["projectKey"], "CMR");
    assert_eq!(result.data["profileId"], f.profile.id.to_string());
    assert_eq!(f.key(&context).as_deref(), Some("CMR"));
}

#[test]
fn deprecated_types_still_run() {
    let f = fixture();
    let registry = f.engine.registry();
    let mut profile = f.profile.clone();
    let next = registry
        .create_instance("Old next", actions::next_id(), &ActionParameters::new())
        .unwrap();
    let active = registry
        .create_instance("Old active", actions::active_id(), &ActionParameters::new())
        .unwrap();
    let (next_id, active_id) = (next.id, active.id);
    profile.actions.extend([next, active]);
    let mut context = ActiveProjectContext::new(&profile);
    let result = f
        .engine
        .run_action(&profile, &f.machine, &mut context, next_id)
        .unwrap();
    assert!(result.is_success());
    let result = f
        .engine
        .run_action(&profile, &f.machine, &mut context, active_id)
        .unwrap();
    assert_eq!(result.data["projectKey"], "CMR");
    assert!(
        ProjectsModule::new()
            .action_handlers(&ActionParameters::new())
            .iter()
            .any(|h| h.definition().deprecated)
    );
}
