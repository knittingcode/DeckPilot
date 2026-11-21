use deckpilot_core::{
    ActionInstanceId, ActionParameters, ActiveProjectContext, Availability, ModuleId, Os,
    ParameterValue, Profile, Project, ProjectCatalog, ProjectKey,
};

use super::*;

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
#[allow(clippy::too_many_lines)] // One line per built-in action type.
fn builtin_modules_register_with_namespaced_action_types() {
    let registry = builtin(Os::Linux);
    let modules = registry.modules();
    let ids: Vec<_> = modules.iter().map(|m| m.manifest.id.to_string()).collect();
    assert_eq!(
        ids,
        [
            "deck",
            "general",
            "http",
            "notifications",
            "system",
            "work-session"
        ]
    );
    assert!(modules.iter().all(|m| m.enabled && m.supported));

    let types: Vec<_> = modules
        .iter()
        .flat_map(|m| &m.action_types)
        .map(|t| t.id.to_string())
        .collect();
    assert_eq!(
        types,
        [
            "deck.next-page",
            "deck.previous-page",
            "deck.page-indicator",
            "deck.switch-profile",
            "deck.brightness",
            "deck.random",
            "deck.multi-switch",
            "deck.switch-layout",
            "deck.previous-layout",
            "general.log",
            "general.wait",
            "general.delay",
            "http.request",
            "http.health-check",
            "notifications.show",
            "notifications.workflow-summary",
            "system.open-app",
            "system.open-url",
            "system.open-file",
            "system.open-directory",
            "system.run-command",
            "system.process-status",
            "system.cpu",
            "system.memory",
            "system.network",
            "system.disk",
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
    assert_eq!(pages, ["work-session/work-session"]);
    registry
        .set_enabled(&work_session::module_id(), false)
        .unwrap();
    assert_eq!(registry.pages(), Vec::new());
}

#[test]
fn disabled_modules_keep_actions_but_do_not_run_them() {
    use deckpilot_core::{Engine, MachineConfig, OnFailure, Workflow};
    let mut modules = builtin(Os::Linux);
    let mut work = profile("Work", &["SHOP", "CMR"]);
    let log = add_action(
        &mut work,
        &modules.action_registry(),
        "Log",
        general::log_id(),
        &ActionParameters::new().with("message", ParameterValue::Text("hi".into())),
    );

    modules.set_enabled(&general::module_id(), false).unwrap();
    assert!(matches!(
        modules.availability(&general::log_id()),
        Availability::ModuleDisabled { .. }
    ));
    let engine = Engine::new(modules.action_registry());
    let machine = MachineConfig::new(Os::Linux);
    let mut context = ActiveProjectContext::new(&work);
    let result = engine
        .run_action(&work, &machine, &mut context, log)
        .unwrap();
    assert!(!result.is_success());
    assert_eq!(
        result.message,
        "this action is unavailable: the General module is disabled"
    );
    // Nothing changed: the action is still configured.
    assert!(work.action(log).is_some());

    // Workflows that use it report the problem instead of hiding it.
    let workflow = Workflow::new("W").step(log, OnFailure::Stop);
    let issues = workflow.validate(&work, engine.registry());
    assert_eq!(issues, ["step 1 (Log): the General module is disabled"]);

    // Re-enabling makes the same stored action work again.
    modules.set_enabled(&general::module_id(), true).unwrap();
    let engine = Engine::new(modules.action_registry());
    let result = engine
        .run_action(&work, &machine, &mut context, log)
        .unwrap();
    assert!(result.is_success());
}

/// The built-ins that moved to first-party plugins are gone (§11 D1, D2,
/// WP-C5, WP-C6): their stable legacy ids are never unknown action types.
/// Each resolves through `ACTION_ALIASES` and, with no plugin to serve it,
/// is unavailable with the plugin it now comes from; a stored instance
/// keeps its id and parameters and fails with that reason, without a
/// panic.
#[test]
fn legacy_ids_of_removed_builtins_are_unavailable_with_a_reason() {
    use deckpilot_core::alias::ACTION_ALIASES;
    use deckpilot_core::{
        ActionInstance, ActionTypeId, Engine, MachineConfig, OnFailure, Workflow,
    };
    let modules = builtin(Os::Linux);
    let registry = modules.action_registry();
    let removed = [
        "github",
        "gitlab",
        "jira",
        "git",
        "docker",
        "workspace",
        "projects",
    ];
    let legacy: Vec<_> = ACTION_ALIASES
        .iter()
        .filter(|alias| removed.contains(&alias.module))
        .collect();
    assert!(legacy.len() > 40, "every legacy id of the seven modules");
    let mut work = profile("Work", &["SHOP"]);
    for alias in &legacy {
        let id = ActionTypeId::new(alias.legacy).unwrap();
        assert!(
            modules
                .manifest(&ModuleId::new(alias.module).unwrap())
                .is_none()
        );
        let reason = registry
            .unavailable_reason(&id)
            .unwrap_or_else(|| panic!("{} has a reason", alias.legacy));
        assert_eq!(
            reason,
            format!(
                "`{}` now comes from `{}`: `{}` is not installed; install it from the \
                 Marketplace",
                alias.legacy, alias.plugin, alias.plugin
            )
        );
        assert!(registry.definition(&id).is_none());
        work.actions.push(ActionInstance {
            id: ActionInstanceId::new(),
            name: alias.legacy.to_owned(),
            action_type: id,
            parameters: ActionParameters::new(),
            description: String::new(),
            icon: None,
            label: None,
        });
    }
    let stored = work.clone();
    let engine = Engine::new(registry);
    let machine = MachineConfig::new(Os::Linux);
    let mut context = ActiveProjectContext::new(&work);
    for action in &stored.actions {
        let result = engine
            .run_action(&work, &machine, &mut context, action.id)
            .unwrap();
        assert!(!result.is_success(), "{}", action.action_type);
        assert!(
            result.message.contains("now comes from"),
            "{}: {}",
            action.action_type,
            result.message
        );
    }
    // A workflow over them reports each step; nothing was rewritten.
    let mut workflow = Workflow::new("W");
    for action in &stored.actions {
        workflow = workflow.step(action.id, OnFailure::Continue);
    }
    assert_eq!(
        workflow.validate(&work, engine.registry()).len(),
        stored.actions.len()
    );
    work.workflows.push(workflow);
    assert_eq!(work.actions, stored.actions);
}

/// Who owns which variable among the built-in modules, and that every
/// legacy id resolves as the canonical id its owner declares.
#[test]
fn builtin_variables_have_explicit_owners() {
    use deckpilot_core::variables::{
        ALIASES, MissingReason, NoLiveState, ProjectSource, Resolution, ResolutionContext,
        VariableEnvironment, VariableId, VariableOwner,
    };
    let registry = builtin(Os::Linux);
    let owners: Vec<(String, Option<String>)> = registry
        .variables()
        .variables()
        .map(|v| {
            (
                v.declaration.id.to_string(),
                v.owner.module().map(ToString::to_string),
            )
        })
        .collect();
    let module = |id: &str, owner: &str| (id.to_owned(), Some(owner.to_owned()));
    let core = |id: &str| (id.to_owned(), None);
    assert_eq!(
        owners,
        [
            core("system.device.current.id"),
            core("system.platform"),
            core("system.profile.current.id"),
            core("system.profile.current.name"),
            module("workSession.canFinish", "work-session"),
            module("workSession.elapsedSeconds", "work-session"),
            module("workSession.id", "work-session"),
            module("workSession.projectId", "work-session"),
            module("workSession.startedAt", "work-session"),
            module("workSession.state", "work-session"),
            core("workflow.execution.id"),
            core("workflow.id"),
        ]
    );
    for alias in ALIASES {
        // `projects.current.*` are the Projects plugin's now
        // (`MODULE_VARIABLE_ALIASES`): declared once it is registered.
        if alias.owner == "projects" {
            assert!(
                deckpilot_core::variables::MODULE_VARIABLE_ALIASES
                    .iter()
                    .any(|m| m.legacy == alias.canonical),
                "{}",
                alias.canonical
            );
            continue;
        }
        let declared = registry
            .variables()
            .get(&alias.alias.canonical())
            .unwrap_or_else(|| panic!("{} is declared", alias.canonical));
        assert_eq!(
            *declared.owner,
            VariableOwner::Module {
                module: ModuleId::new(alias.owner).unwrap()
            }
        );
        // An alias keeps the type it always had.
        assert_eq!(declared.declaration.value_type, alias.alias.value_type());
        assert!(declared.declaration.read_only);
        assert!(!declared.declaration.sensitive);
    }

    // The project in the context: without the Projects plugin there is no
    // owner, so `projects.current.key` has no value, and nothing is lost.
    let work = profile("Work", &["SHOP"]);
    let context = ResolutionContext::preview(work.id)
        .with_project(Some(work.projects[0].id), ProjectSource::Application);
    let environment = VariableEnvironment {
        context: &context,
        profile: &work,
        os: Os::Linux,
        live: &NoLiveState,
    };
    let key = VariableId::new("projects.current.key").unwrap();
    assert!(!matches!(
        registry.variables().resolve(&key, &environment),
        Resolution::Value { .. }
    ));
    // Module roots that parse are exactly those of modules that declare
    // variables.
    let mut namespaces: Vec<String> = builtin(Os::Linux)
        .modules()
        .iter()
        .filter(|m| !m.manifest.variables.is_empty())
        .map(|m| deckpilot_core::variables::module_namespace(&m.manifest.id))
        .collect();
    namespaces.sort();
    // `projects` stays a namespace that parses: its legacy references
    // keep working through the Projects plugin.
    let mut known: Vec<String> = deckpilot_core::variables::MODULE_NAMESPACES
        .iter()
        .filter(|n| **n != "projects")
        .map(|n| (*n).to_owned())
        .collect();
    known.sort();
    assert_eq!(namespaces, known);
    // Without a session the work session variables have no value.
    let state = VariableId::new("workSession.state").unwrap();
    assert_eq!(
        registry.variables().resolve(&state, &environment),
        Resolution::Missing(MissingReason::NoValue)
    );
}
