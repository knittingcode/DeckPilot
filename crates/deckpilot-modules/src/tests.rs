use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActiveProjectContext, Availability, BindingTarget,
    ControlBinding, DeviceLayout, Engine, MachineConfig, ModuleId, OnFailure, ParameterValue,
    Profile, Project, ProjectKey, Runtime, Trigger, Workflow,
};
use deckpilot_device::{ControlId, DeviceEvent, DeviceId, DeviceModelId, ModelCatalog, models};

use super::*;
use crate::projects::{ProjectCatalog, actions};

fn projects_id() -> ModuleId {
    projects::module_id()
}

fn profile(name: &str, keys: &[&str]) -> Profile {
    let mut profile = Profile::new(name);
    for key in keys {
        profile
            .add_project(Project::new(ProjectKey::new(*key).unwrap(), *key))
            .unwrap();
    }
    profile
}

fn add_action(
    profile: &mut Profile,
    registry: &deckpilot_core::ActionRegistry,
    name: &str,
    action_type: deckpilot_core::ActionTypeId,
    parameters: &ActionParameters,
) -> ActionInstanceId {
    let instance = registry
        .create_instance(name, action_type, parameters)
        .unwrap();
    let id = instance.id;
    profile.actions.push(instance);
    id
}

#[test]
fn builtin_modules_register_with_namespaced_action_types() {
    let registry = builtin(Os::Linux);
    let modules = registry.modules();
    let ids: Vec<_> = modules.iter().map(|m| m.manifest.id.to_string()).collect();
    assert_eq!(ids, ["general", "projects"]);
    assert!(modules.iter().all(|m| m.enabled && m.supported));

    let types: Vec<_> = modules
        .iter()
        .flat_map(|m| &m.action_types)
        .map(|t| t.id.to_string())
        .collect();
    assert_eq!(
        types,
        [
            "general.log",
            "projects.next",
            "projects.previous",
            "projects.select",
            "projects.active",
            "projects.repository",
            "projects.documentation",
            "projects.resolve-resource"
        ]
    );
    let executable = registry.action_registry().action_types().count();
    assert_eq!(executable, types.len());
}

#[test]
fn navigation_pages_come_from_enabled_modules() {
    let mut registry = builtin(Os::Linux);
    let pages: Vec<_> = registry
        .pages()
        .into_iter()
        .map(|(module, page)| format!("{module}/{}", page.id))
        .collect();
    assert_eq!(pages, ["projects/projects"]);
    registry.set_enabled(&projects_id(), false).unwrap();
    assert_eq!(registry.pages(), Vec::new());
}

#[test]
fn disabled_modules_keep_actions_but_do_not_run_them() {
    let mut modules = builtin(Os::Linux);
    let mut work = profile("Work", &["SHOP", "CMR"]);
    let next = add_action(
        &mut work,
        &modules.action_registry(),
        "Next",
        actions::next_id(),
        &ActionParameters::new(),
    );

    modules.set_enabled(&projects_id(), false).unwrap();
    assert!(matches!(
        modules.availability(&actions::next_id()),
        Availability::ModuleDisabled { .. }
    ));
    let engine = Engine::new(modules.action_registry());
    let machine = MachineConfig::new(Os::Linux);
    let mut context = ActiveProjectContext::new(&work);
    let result = engine
        .run_action(&work, &machine, &mut context, next)
        .unwrap();
    assert!(!result.is_success());
    assert_eq!(
        result.message,
        "this action is unavailable: the Projects module is disabled"
    );
    // Nothing changed: the action is still configured and the project too.
    assert!(work.action(next).is_some());
    assert_eq!(context.project_id(), Some(work.projects[0].id));

    // Workflows that use it report the problem instead of hiding it.
    let workflow = Workflow::new("W").step(next, OnFailure::Stop);
    let issues = workflow.validate(&work, engine.registry());
    assert_eq!(issues, ["step 1 (Next): the Projects module is disabled"]);

    // Re-enabling makes the same stored action work again.
    modules.set_enabled(&projects_id(), true).unwrap();
    let engine = Engine::new(modules.action_registry());
    let result = engine
        .run_action(&work, &machine, &mut context, next)
        .unwrap();
    assert!(result.is_success());
    assert_eq!(context.project_id(), Some(work.projects[1].id));
}

#[test]
fn module_configuration_reaches_the_handlers() {
    let mut modules = builtin(Os::Linux);
    let mut work = profile("Work", &["SHOP", "CMR"]);
    modules
        .set_config(
            &projects_id(),
            &ActionParameters::new().with(projects::WRAP_AROUND, ParameterValue::Boolean(false)),
        )
        .unwrap();
    let registry = modules.action_registry();
    let next = add_action(
        &mut work,
        &registry,
        "Next",
        actions::next_id(),
        &ActionParameters::new(),
    );
    let engine = Engine::new(registry);
    let machine = MachineConfig::new(Os::Linux);
    let mut context = ActiveProjectContext::new(&work);
    assert!(
        engine
            .run_action(&work, &machine, &mut context, next)
            .unwrap()
            .is_success()
    );
    let at_end = engine
        .run_action(&work, &machine, &mut context, next)
        .unwrap();
    assert_eq!(at_end.message, "already at the last project");
}

#[test]
fn selection_is_limited_to_the_actions_profile() {
    let modules = builtin(Os::Linux);
    let registry = modules.action_registry();
    let mut work = profile("Work", &["SHOP", "CMR"]);
    let home = profile("Home", &["BLOG"]);
    // A hand-edited or corrupted action pointing at another profile's project.
    let foreign = add_action(
        &mut work,
        &registry,
        "Foreign",
        actions::select_id(),
        &ActionParameters::new().with("project_id", ParameterValue::Project(home.projects[0].id)),
    );
    assert_eq!(
        registry.check_in_profile(work.action(foreign).unwrap(), &work),
        ["the selected project does not belong to this profile"]
    );
    let engine = Engine::new(registry);
    let machine = MachineConfig::new(Os::Linux);
    let mut context = ActiveProjectContext::new(&work);
    let result = engine
        .run_action(&work, &machine, &mut context, foreign)
        .unwrap();
    assert!(!result.is_success());
    assert_eq!(context.project_id(), Some(work.projects[0].id));
}

fn device() -> DeviceId {
    DeviceId {
        model: DeviceModelId::new(models::STREAM_DECK_MK2_ID),
        serial: "SIM".into(),
    }
}

fn press(index: u16) -> DeviceEvent {
    DeviceEvent::Input {
        device: device(),
        control: ControlId::Key { index },
        input: deckpilot_device::ControlInput::Press,
    }
}

/// A profile with a "Next" key (label `label`) and a key selecting `select`.
fn deck_profile(
    name: &str,
    keys: &[&str],
    label: &str,
    registry: &deckpilot_core::ActionRegistry,
) -> Profile {
    let mut profile = profile(name, keys);
    let next = add_action(
        &mut profile,
        registry,
        "Next",
        actions::next_id(),
        &ActionParameters::new(),
    );
    let last = profile.projects.last().unwrap().id;
    let select = add_action(
        &mut profile,
        registry,
        "Last",
        actions::select_id(),
        &ActionParameters::new().with("project_id", ParameterValue::Project(last)),
    );
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let bind = |index, action, label: Option<&str>| ControlBinding {
        control: ControlId::Key { index },
        trigger: Trigger::Press,
        target: BindingTarget::Action { action },
        label: label.map(Into::into),
    };
    layout.pages[0]
        .bindings
        .extend([bind(0, next, Some(label)), bind(1, select, None)]);
    profile.layouts.push(layout);
    profile
}

#[test]
fn dynamic_labels_follow_project_and_profile_changes() {
    let modules = builtin(Os::Linux);
    let registry = modules.action_registry();
    let work = deck_profile("Work", &["SHOP", "CMR"], "{project.key}", &registry);
    let home = deck_profile(
        "Home",
        &["BLOG", "GARDEN"],
        "{profile}: {project.name}",
        &registry,
    );
    let mut runtime = Runtime::new(
        Engine::new(registry),
        ModelCatalog::builtin(),
        MachineConfig::new(Os::Linux),
    );
    let title = |runtime: &Runtime| runtime.faces(&device())[0].1.title.clone();
    let highlighted = |runtime: &Runtime| runtime.faces(&device())[1].1.highlighted;

    runtime.set_profile(Some(work.clone()), None);
    runtime
        .handle_event(&DeviceEvent::Connected { device: device() })
        .unwrap();
    assert_eq!(title(&runtime), "SHOP");
    assert!(!highlighted(&runtime));
    runtime.handle_event(&press(0)).unwrap();
    assert_eq!(title(&runtime), "CMR");
    assert!(highlighted(&runtime), "the CMR selector is now active");
    let work_project = runtime.active_project_id();

    // Another profile has its own projects and its own active project.
    runtime.set_profile(Some(home.clone()), None);
    assert_eq!(title(&runtime), "Home: BLOG");
    runtime.handle_event(&press(1)).unwrap();
    assert_eq!(title(&runtime), "Home: GARDEN");

    // Switching back restores the stored selection of the first profile.
    runtime.set_profile(Some(work), work_project);
    assert_eq!(title(&runtime), "CMR");
}

#[test]
fn workflows_combine_generic_project_actions() {
    use deckpilot_core::{
        MachineResourceBinding, ResourceKey, ResourceKind, StepCondition, WorkflowReport,
    };
    let modules = builtin(Os::Linux);
    let registry = modules.action_registry();
    let mut work = profile("Work", &["SHOP", "CMR"]);
    for (index, key) in ["shop", "cmr"].iter().enumerate() {
        work.projects[index].resources.insert(
            "repository".into(),
            ResourceKey::new(format!("{key}.repository")).unwrap(),
        );
    }
    let mut machine = MachineConfig::new(Os::Linux);
    for key in ["shop", "cmr"] {
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new(format!("{key}.repository")).unwrap(),
            kind: ResourceKind::Directory,
            value: format!("/src/{key}"),
        });
    }
    let none = ActionParameters::new();
    let current = add_action(&mut work, &registry, "Current", actions::select_id(), &none);
    let repository = add_action(
        &mut work,
        &registry,
        "Repo",
        actions::repository_id(),
        &none,
    );
    let next = add_action(&mut work, &registry, "Next", actions::next_id(), &none);
    let shop = work.projects[0].id;
    let shop_repository = add_action(
        &mut work,
        &registry,
        "Repo SHOP",
        actions::repository_id(),
        &ActionParameters::new().with(projects::actions::PROJECT_ID, ParameterValue::Project(shop)),
    );
    let mut workflow = Workflow::new("Start")
        .step(current, OnFailure::Stop)
        .step(repository, OnFailure::Stop)
        .step(next, OnFailure::Stop)
        .step(repository, OnFailure::Stop)
        .step(shop_repository, OnFailure::Stop)
        .step(repository, OnFailure::Stop);
    // Only while CMR is active (after the switch in step 3).
    workflow.steps[5].condition = StepCondition::ActiveProject {
        project: work.projects[1].id,
    };
    let workflow_id = workflow.id;
    assert_eq!(workflow.validate(&work, &registry), Vec::<String>::new());
    work.workflows.push(workflow);

    let engine = Engine::new(registry);
    let mut context = ActiveProjectContext::new(&work);
    let report: WorkflowReport = engine
        .run_workflow(&work, &machine, &mut context, workflow_id)
        .unwrap();
    let values: Vec<_> = report
        .steps
        .iter()
        .map(|s| {
            s.result
                .data
                .get("value")
                .or(s.result.data.get("projectKey"))
                .cloned()
        })
        .collect();
    assert_eq!(
        values,
        [
            Some("SHOP".into()),
            Some("/src/shop".into()),
            Some("CMR".into()),
            Some("/src/cmr".into()),
            // A fixed project, independent of the active one.
            Some("/src/shop".into()),
            Some("/src/cmr".into()),
        ]
    );
    assert_eq!(context.project_id(), Some(work.projects[1].id));
}
