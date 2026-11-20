use super::*;
use crate::testing::{
    FIXTURE_ALIASES, LEGACY_CREDENTIAL, LegacyModule, PluginFixture, TestModule, plugin_fetch,
};
use crate::{
    ActionInstance, ActionParameters, ActiveProjectContext, Availability, Engine, ExecutionContext,
    MachineConfig, ModuleError, ModuleRegistry, Os, ParameterValue, Profile,
};

fn id(value: &str) -> ModuleId {
    ModuleId::new(value).unwrap()
}

fn fetch() -> ActionTypeId {
    ActionTypeId::new("legacy.fetch").unwrap()
}

fn target() -> ActionTypeId {
    ActionTypeId::new("deckpilot.legacy.fetch").unwrap()
}

fn other() -> ActionTypeId {
    ActionTypeId::new("legacy.other").unwrap()
}

fn migrated() -> LegacyRouting {
    LegacyRouting::new().with(id("legacy"), LegacyRoute::Plugin)
}

/// The fixture aliases, the built-in `legacy`, and `plugin` if any.
fn modules(builtin: bool, plugin: Option<PluginFixture>, routing: LegacyRouting) -> ModuleRegistry {
    let mut registry = ModuleRegistry::new(Os::Linux).with_aliases(FIXTURE_ALIASES);
    registry.register(Box::new(TestModule::base())).unwrap();
    if builtin {
        registry.register(Box::new(LegacyModule::new())).unwrap();
    }
    if let Some(plugin) = plugin {
        registry.register(Box::new(plugin)).unwrap();
    }
    registry.set_legacy_routing(routing);
    registry
}

fn message(text: &str) -> ActionParameters {
    ActionParameters::new().with("message", ParameterValue::Text(text.into()))
}

/// Runs `action_type` through the registry the way the engine does:
/// normalized, then dispatched.
fn run(
    registry: &ModuleRegistry,
    action_type: &ActionTypeId,
    parameters: &ActionParameters,
) -> String {
    let actions = registry.action_registry();
    let profile = Profile::new("Work");
    let machine = MachineConfig::new(Os::Linux);
    let context = ExecutionContext::new(&profile, None, &machine);
    match actions.normalize(action_type, parameters) {
        Ok(parameters) => actions.dispatch(action_type, &parameters, &context).message,
        Err(error) => format!("refused: {error}"),
    }
}

fn outcome(registry: &ModuleRegistry) -> AliasOutcome {
    let statuses = registry.action_aliases();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].legacy, fetch());
    assert_eq!(statuses[0].target, target());
    assert_eq!(statuses[0].plugin, id("deckpilot.legacy"));
    statuses[0].outcome.clone()
}

#[test]
fn the_shipped_table_routes_only_what_it_lists() {
    // Even with a routing that says "migrated" and the plugin there: the
    // shipped table (GitHub's and the notifications block) has no alias
    // for `legacy`, and its own entries are not routed by the agent.
    let mut registry = ModuleRegistry::new(Os::Linux);
    registry.register(Box::new(LegacyModule::new())).unwrap();
    registry.register(Box::new(PluginFixture::new())).unwrap();
    registry.set_legacy_routing(migrated());
    let statuses = registry.action_aliases();
    assert_eq!(statuses.len(), ACTION_ALIASES.len());
    for status in &statuses {
        assert_ne!(status.plugin, id("deckpilot.legacy"));
        // Not routed by the agent: the built-in's, never the plugin's.
        assert_eq!(status.outcome, AliasOutcome::Builtin, "{}", status.legacy);
    }
    assert_eq!(
        run(&registry, &fetch(), &message("hi")),
        "builtin: hi [message]"
    );
    assert_eq!(registry.availability(&fetch()), Availability::Available);
    let actions = registry.action_registry();
    assert_eq!(actions.module_of(&fetch()), Some(&id("legacy")));
    assert_eq!(
        actions.definition(&fetch()),
        Some(&crate::testing::legacy_fetch())
    );
}

#[test]
fn the_builtin_serves_a_legacy_type_until_the_agent_routes_it() {
    let registry = modules(true, Some(PluginFixture::new()), LegacyRouting::new());
    assert_eq!(outcome(&registry), AliasOutcome::Builtin);
    assert_eq!(
        run(&registry, &fetch(), &message("hi")),
        "builtin: hi [message]"
    );
    let actions = registry.action_registry();
    assert_eq!(actions.module_of(&fetch()), Some(&id("legacy")));
    assert!(!actions.definition(&fetch()).unwrap().deprecated);
}

#[test]
fn a_migrated_legacy_type_runs_the_plugin_with_its_capabilities() {
    let registry = modules(true, Some(PluginFixture::new()), migrated());
    assert_eq!(outcome(&registry), AliasOutcome::Plugin);
    assert_eq!(
        run(&registry, &fetch(), &message("hi")),
        "plugin: hi [message]"
    );
    // The canonical type runs the plugin too; the module's other types
    // stay with the built-in.
    assert_eq!(
        run(&registry, &target(), &message("hi")),
        "plugin: hi [message]"
    );
    assert_eq!(
        run(&registry, &other(), &message("hi")),
        "builtin: hi [message]"
    );

    let actions = registry.action_registry();
    // Dispatch, its capability checks and `runs_detached` are the plugin's.
    assert_eq!(actions.module_of(&fetch()), Some(&id("deckpilot.legacy")));
    assert_eq!(actions.module_of(&other()), Some(&id("legacy")));
    assert!(actions.runs_detached(&fetch()));
    assert!(!actions.runs_detached(&other()));
    assert_eq!(registry.availability(&fetch()), Availability::Available);
}

#[test]
fn the_catalog_shows_the_plugins_definition_under_the_legacy_id_hidden() {
    let registry = modules(true, Some(PluginFixture::new()), migrated());
    let actions = registry.action_registry();
    let legacy = actions.definition(&fetch()).unwrap();
    let canonical = actions.definition(&target()).unwrap();
    assert_eq!(legacy.id, fetch());
    // Hidden from new instances like a deprecated type, still editable.
    assert!(legacy.deprecated);
    assert!(!canonical.deprecated);
    assert_eq!(legacy.name, canonical.name);
    assert_eq!(legacy.node, canonical.node);
    // The plugin's parameters, and the translated `credential` so that
    // stored instances keep validating.
    let keys: Vec<&str> = legacy.parameters.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["message", "credential"]);
    assert_eq!(legacy.parameters[0], plugin_fetch().parameters[0]);
    // Both are listed exactly once each.
    let listed: Vec<String> = actions
        .action_types()
        .map(|t| t.id.to_string())
        .filter(|t| t.contains("fetch"))
        .collect();
    assert_eq!(listed, ["deckpilot.legacy.fetch", "legacy.fetch"]);
    // The module list keeps the built-in's own definitions.
    let builtin = registry
        .modules()
        .into_iter()
        .find(|m| m.manifest.id == id("legacy"))
        .unwrap();
    assert!(
        builtin
            .action_types
            .iter()
            .any(|t| t.id == fetch() && !t.deprecated)
    );
}

#[test]
fn a_credential_override_is_dropped_only_when_it_is_the_default() {
    let registry = modules(true, Some(PluginFixture::new()), migrated());
    let with = |credential: &str| {
        message("hi").with("credential", ParameterValue::Text(credential.into()))
    };
    assert_eq!(run(&registry, &fetch(), &with("")), "plugin: hi [message]");
    assert_eq!(
        run(&registry, &fetch(), &with(LEGACY_CREDENTIAL)),
        "plugin: hi [message]"
    );
    // Any other value is refused when it is saved (normalized), with the
    // error a built-in's invalid parameter gets, whatever the plugin's
    // capabilities; and still when it runs without being saved again.
    assert_eq!(
        run(&registry, &fetch(), &with("personal")),
        "refused: invalid parameters: parameter `credential`: names something the plugin \
         providing `legacy.fetch` cannot use; leave it empty to use the plugin's own"
    );
    let actions = registry.action_registry();
    assert!(matches!(
        actions.normalize(&fetch(), &with("personal")),
        Err(crate::RegistryError::InvalidParameters(problems))
            if problems == [crate::ParameterError::Invalid {
                key: "credential".into(),
                reason: "names something the plugin providing `legacy.fetch` cannot use; \
                         leave it empty to use the plugin's own".into(),
            }]
    ));
    assert_eq!(
        actions
            .validate_parameters(&fetch(), &with("personal"))
            .len(),
        1
    );
    assert_eq!(
        actions.validate_parameters(&fetch(), &with(LEGACY_CREDENTIAL)),
        []
    );
    let mut instance = ActionInstance {
        id: crate::ActionInstanceId::new(),
        name: "Fetch".into(),
        action_type: fetch(),
        parameters: with("personal"),
        description: String::new(),
        icon: None,
        label: None,
    };
    assert!(matches!(
        actions.validate_instance(&mut instance),
        Err(crate::RegistryError::InvalidParameters(_))
    ));
    let profile = Profile::new("Work");
    let machine = MachineConfig::new(Os::Linux);
    let context = ExecutionContext::new(&profile, None, &machine);
    assert_eq!(
        actions
            .dispatch(&fetch(), &with("personal"), &context)
            .message,
        "parameter `credential` names something the plugin providing `legacy.fetch` cannot \
         use; leave it empty to use the plugin's own"
    );
    // Checks against the profile run with the plugin's capabilities: the
    // fixture may not read the profile, so it reports nothing.
    assert_eq!(
        actions.check_parameters(&fetch(), &with("personal"), &profile),
        Vec::<String>::new()
    );
    // The handler itself reports what cannot be translated.
    let handler = actions.handler(&fetch()).unwrap();
    assert_eq!(handler.check(&with("personal"), &profile).len(), 1);
    assert_eq!(handler.check(&with(""), &profile), Vec::<String>::new());
    // The canonical type has no such parameter.
    assert!(run(&registry, &target(), &with("")).starts_with("refused: "));
}

#[test]
fn a_faulty_missing_or_incompatible_plugin_falls_back_to_the_builtin() {
    let faulty = modules(
        true,
        Some(PluginFixture::faulty("the registry revoked this version")),
        migrated(),
    );
    let missing = modules(true, None, migrated());
    let mut narrower = plugin_fetch();
    narrower.node.timeout_ms = 1;
    let incompatible = modules(true, Some(PluginFixture::providing(narrower)), migrated());
    let mut renamed = plugin_fetch();
    renamed.id = ActionTypeId::new("deckpilot.legacy.get").unwrap();
    let without = modules(true, Some(PluginFixture::providing(renamed)), migrated());
    for (registry, reason) in [
        (&faulty, "the registry revoked this version"),
        // Named after the built-in while the plugin is not there.
        (&missing, "Legacy (`deckpilot.legacy`) is not installed"),
        (
            &incompatible,
            "`deckpilot.legacy.fetch` of Legacy plugin (`deckpilot.legacy`) cannot replace \
             `legacy.fetch`: the node's timeout is shorter",
        ),
        (
            &without,
            "Legacy plugin (`deckpilot.legacy`) does not provide `deckpilot.legacy.fetch`",
        ),
    ] {
        assert_eq!(
            outcome(registry),
            AliasOutcome::Fallback {
                reason: reason.into()
            },
            "{reason}"
        );
        assert_eq!(
            run(registry, &fetch(), &message("hi")),
            "builtin: hi [message]"
        );
        assert_eq!(registry.availability(&fetch()), Availability::Available);
        assert_eq!(
            registry.action_registry().module_of(&fetch()),
            Some(&id("legacy"))
        );
    }
}

#[test]
fn a_plugin_the_user_disabled_or_removed_leaves_the_type_unavailable() {
    // Disabled in the registry by the user: no fallback.
    let mut disabled = modules(true, Some(PluginFixture::new()), migrated());
    disabled
        .set_enabled(&id("deckpilot.legacy"), false)
        .unwrap();
    // Disabled or uninstalled in the plugin store, as the agent reports it.
    let withdrawn = LegacyRouting::new().with(id("legacy"), LegacyRoute::PluginWithdrawn);
    let removed = modules(true, None, withdrawn.clone());
    let store_disabled = modules(
        true,
        Some(PluginFixture::faulty("the plugin is disabled")),
        withdrawn,
    );
    for (registry, name) in [
        (&disabled, "Legacy plugin"),
        (&removed, "Legacy"),
        (&store_disabled, "Legacy plugin"),
    ] {
        let reason = format!("{name} (`deckpilot.legacy`) is disabled");
        assert_eq!(
            outcome(registry),
            AliasOutcome::Unavailable {
                reason: reason.clone()
            }
        );
        assert_eq!(
            run(registry, &fetch(), &message("hi")),
            format!("refused: action type `legacy.fetch` is unavailable: {reason}")
        );
        assert_eq!(
            registry.availability(&fetch()),
            Availability::ModuleDisabled {
                module: id("deckpilot.legacy"),
                name: name.into()
            }
        );
        let actions = registry.action_registry();
        assert_eq!(actions.unavailable_reason(&fetch()), Some(reason.as_str()));
        assert!(actions.definition(&fetch()).is_none());
        // The module's other types are not affected.
        assert_eq!(
            run(registry, &other(), &message("hi")),
            "builtin: hi [message]"
        );
    }
}

#[test]
fn without_the_builtin_the_plugin_is_the_only_way() {
    let served = modules(false, Some(PluginFixture::new()), migrated());
    assert_eq!(outcome(&served), AliasOutcome::Plugin);
    assert_eq!(
        run(&served, &fetch(), &message("hi")),
        "plugin: hi [message]"
    );
    assert_eq!(served.availability(&fetch()), Availability::Available);
    // Without the built-in, there is no setting to compare with: only an
    // empty override is dropped.
    assert_eq!(
        run(
            &served,
            &fetch(),
            &message("hi").with("credential", ParameterValue::Text(String::new()))
        ),
        "plugin: hi [message]"
    );

    let missing = modules(false, None, migrated());
    let reason = "`deckpilot.legacy` is not installed; install it from the Marketplace";
    assert_eq!(
        outcome(&missing),
        AliasOutcome::Unavailable {
            reason: reason.into()
        }
    );
    assert_eq!(
        run(&missing, &fetch(), &message("hi")),
        format!("refused: action type `legacy.fetch` is unavailable: {reason}")
    );
    assert_eq!(missing.availability(&fetch()), Availability::Unknown);

    let faulty = modules(
        false,
        Some(PluginFixture::faulty("the plugin's files were modified")),
        migrated(),
    );
    assert_eq!(
        outcome(&faulty),
        AliasOutcome::Unavailable {
            reason: "the plugin's files were modified".into()
        }
    );
    assert_eq!(
        faulty.availability(&fetch()),
        Availability::ModuleDisabled {
            module: id("deckpilot.legacy"),
            name: "Legacy plugin".into()
        }
    );
}

#[test]
fn stored_profiles_are_never_rewritten() {
    let builtin = modules(true, Some(PluginFixture::new()), LegacyRouting::new());
    let instance = builtin
        .action_registry()
        .create_instance(
            "Fetch",
            fetch(),
            &message("hi").with("credential", ParameterValue::Text(LEGACY_CREDENTIAL.into())),
        )
        .unwrap();
    let mut profile = Profile::new("Work");
    profile.actions.push(instance.clone());
    let stored = serde_json::to_string(&profile).unwrap();

    let migrated = modules(true, Some(PluginFixture::new()), migrated());
    let machine = MachineConfig::new(Os::Linux);
    for registry in [&builtin, &migrated] {
        let engine = Engine::new(registry.action_registry());
        let mut context = ActiveProjectContext::new(&profile);
        let result = engine
            .run_action(&profile, &machine, &mut context, instance.id)
            .unwrap();
        assert!(result.is_success(), "{}", result.message);
        // Editing validates the instance under its legacy id, as it is.
        let mut edited: ActionInstance = instance.clone();
        registry
            .action_registry()
            .validate_instance(&mut edited)
            .unwrap();
        assert_eq!(edited.action_type, fetch());
        assert_eq!(edited.parameters, instance.parameters);
    }
    assert_eq!(serde_json::to_string(&profile).unwrap(), stored);
    assert_eq!(profile.actions[0].action_type, fetch());
    // Routing back (an older app, or a rollback) needs nothing either.
    assert_eq!(
        run(&builtin, &fetch(), &profile.actions[0].parameters),
        "builtin: hi [credential,message]"
    );
}

#[test]
fn reserved_ids_never_register() {
    assert_eq!(
        RESERVED_MODULE_IDS,
        ["deckpilot.core", "deckpilot.project", "deckpilot.resource"]
    );
    for reserved in RESERVED_MODULE_IDS {
        let module = id(reserved);
        assert!(module.is_reserved());
        let mut registry = ModuleRegistry::new(Os::Linux);
        assert_eq!(
            registry.register(Box::new(PluginFixture::new().with_module_id(reserved))),
            Err(ModuleError::Reserved(module.clone()))
        );
        assert!(!registry.contains(&module));
        // Their stored legacy types stay unknown.
        let legacy = ActionTypeId::new(format!("{reserved}.log")).unwrap();
        assert_eq!(registry.availability(&legacy), Availability::Unknown);
    }
    assert!(!id("deckpilot.github").is_reserved());
    assert!(!id("deckpilot.cores").is_reserved());
    assert!(!id("core").is_reserved());
}

/// The extraction set (`module-migration.md` §11 D2, D4): while the
/// migration waits for the user's consent, the legacy type is unavailable
/// with that reason, the built-in compiled in or not, the plugin there or
/// not; the module's other types are not affected.
#[test]
fn a_migration_waiting_for_consent_leaves_the_type_unavailable() {
    let waiting = LegacyRouting::new().with(id("legacy"), LegacyRoute::AwaitingConsent);
    for (builtin, plugin) in [(true, true), (true, false), (false, true)] {
        let registry = modules(builtin, plugin.then(PluginFixture::new), waiting.clone());
        let name = if plugin { "Legacy plugin" } else { "Legacy" };
        let reason = format!(
            "{name} (`deckpilot.legacy`) waits for your consent: allow it in Settings › Plugins"
        );
        assert_eq!(
            outcome(&registry),
            AliasOutcome::Unavailable {
                reason: reason.clone()
            },
            "{builtin} {plugin}"
        );
        assert_eq!(
            run(&registry, &fetch(), &message("hi")),
            format!("refused: action type `legacy.fetch` is unavailable: {reason}")
        );
        assert!(matches!(
            registry.availability(&fetch()),
            Availability::ModuleDisabled { .. }
        ));
        if builtin {
            assert_eq!(
                run(&registry, &other(), &message("hi")),
                "builtin: hi [message]"
            );
        }
    }
}

/// A handler that reports the parameters it got, `key=value` sorted.
struct Keys(crate::ActionType);

impl crate::ActionHandler for Keys {
    fn definition(&self) -> &crate::ActionType {
        &self.0
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        _context: &ExecutionContext<'_>,
    ) -> crate::ActionResult {
        let keys: Vec<String> = parameters
            .iter()
            .map(|(key, value)| format!("{key}={}", serde_json::to_string(value).unwrap()))
            .collect();
        crate::ActionResult::success(keys.join(" "))
    }
}

/// `Rename`, `Keep` and `Unsupported` as the alias handler applies them:
/// the legacy type keeps the legacy key, the plugin gets its own; a kept
/// parameter passes unchanged; an unsupported value is refused when saved
/// and when run, never dropped; an empty one passes.
#[test]
fn renamed_kept_and_unsupported_parameters_reach_the_plugin_as_declared() {
    use crate::{ParameterKind, ParameterSpec, ResourceKey, ResourceRef};
    let spec = |key: &str, kind: ParameterKind| ParameterSpec {
        key: key.into(),
        label: key.into(),
        description: None,
        kind,
        required: false,
        default: None,
    };
    let mut plugin = plugin_fetch();
    plugin.parameters = vec![
        spec("repository", ParameterKind::Resource { kind: None }),
        spec("projectId", ParameterKind::Project),
        spec(
            "projectName",
            ParameterKind::Text {
                multiline: false,
                max_length: Some(64),
            },
        ),
    ];
    let alias = ActionAlias {
        params: &[
            ParamRule::Keep("repository"),
            ParamRule::Rename("project_id", "projectId"),
            ParamRule::Unsupported("projectName"),
        ],
        ..FIXTURE_ALIASES[0]
    };
    let handler = crate::registry::AliasHandler::new(&alias, Box::new(Keys(plugin)), None, None);
    let keys: Vec<&str> = crate::ActionHandler::definition(&handler)
        .parameters
        .iter()
        .map(|p| p.key.as_str())
        .collect();
    assert_eq!(keys, ["repository", "project_id", "projectName"]);
    let profile = Profile::new("Work");
    let project = crate::ProjectId::new();
    let machine = MachineConfig::new(Os::Linux);
    let context = ExecutionContext::new(&profile, None, &machine);
    let repository = ParameterValue::Resource(ResourceRef::Fixed {
        key: ResourceKey::new("shop.repo").unwrap(),
    });
    let stored = ActionParameters::new()
        .with("repository", repository.clone())
        .with("project_id", ParameterValue::Project(project))
        .with("projectName", ParameterValue::Text(" ".into()));
    let ran = crate::ActionHandler::execute(&handler, &stored, &context);
    assert_eq!(
        ran.status,
        crate::ActionStatus::Succeeded,
        "{}",
        ran.message
    );
    assert_eq!(
        ran.message,
        format!(
            "projectId={} projectName={} repository={}",
            serde_json::to_string(&ParameterValue::Project(project)).unwrap(),
            serde_json::to_string(&ParameterValue::Text(" ".into())).unwrap(),
            serde_json::to_string(&repository).unwrap()
        )
    );
    assert_eq!(crate::ActionHandler::validate(&handler, &stored), []);

    let named = stored.with("projectName", ParameterValue::Text("shop".into()));
    let refused = crate::ActionHandler::execute(&handler, &named, &context);
    assert_eq!(refused.status, crate::ActionStatus::Failed);
    assert!(
        refused
            .message
            .starts_with("parameter `projectName` is not supported by the plugin providing"),
        "{}",
        refused.message
    );
    assert_eq!(crate::ActionHandler::validate(&handler, &named).len(), 1);
    assert_eq!(
        crate::ActionHandler::check(&handler, &named, &profile).len(),
        1
    );
}

/// `AtMost` as the alias handler applies it: a value up to the bound
/// passes unchanged, a larger one is refused when saved and when run,
/// never clamped; and the contract check takes the bound for the
/// built-in's.
#[test]
fn a_capped_parameter_passes_up_to_its_bound_and_is_never_clamped() {
    use crate::{ParameterKind, ParameterSpec};
    let spec = |max: i64| ParameterSpec {
        key: "line".into(),
        label: "Line".into(),
        description: None,
        kind: ParameterKind::Integer {
            min: Some(1),
            max: Some(max),
        },
        required: false,
        default: None,
    };
    let mut plugin = plugin_fetch();
    plugin.parameters = vec![spec(1_000_000)];
    let alias = ActionAlias {
        params: &[ParamRule::AtMost("line", 1_000_000)],
        ..FIXTURE_ALIASES[0]
    };
    let mut builtin = crate::testing::legacy_fetch();
    builtin.parameters = vec![spec(10_000_000)];
    let target = {
        let mut target = plugin.clone();
        target.node = builtin.node.clone();
        target.node.cancellable = true;
        target.controls = builtin.controls.clone();
        target
    };
    let problems = super::incompatibilities(&alias, &builtin, &target);
    assert!(
        !problems.iter().any(|p| p.contains("`line`")),
        "{problems:?}"
    );
    // Without the rule the plugin's bound is narrower.
    let plain = ActionAlias {
        params: &[],
        ..FIXTURE_ALIASES[0]
    };
    assert!(
        super::incompatibilities(&plain, &builtin, &target)
            .contains(&"parameter `line`: its kind or constraints are narrower".to_owned())
    );
    // A plugin bound below the cap is still narrower.
    let mut lower = target.clone();
    lower.parameters = vec![spec(999)];
    assert!(
        super::incompatibilities(&alias, &builtin, &lower)
            .contains(&"parameter `line`: its kind or constraints are narrower".to_owned())
    );

    let handler = crate::registry::AliasHandler::new(&alias, Box::new(Keys(plugin)), None, None);
    let profile = Profile::new("Work");
    let machine = MachineConfig::new(Os::Linux);
    let context = ExecutionContext::new(&profile, None, &machine);
    let within = ActionParameters::new().with("line", ParameterValue::Integer(1_000_000));
    let ran = crate::ActionHandler::execute(&handler, &within, &context);
    assert_eq!(
        ran.status,
        crate::ActionStatus::Succeeded,
        "{}",
        ran.message
    );
    assert_eq!(
        ran.message,
        format!(
            "line={}",
            serde_json::to_string(&ParameterValue::Integer(1_000_000)).unwrap()
        )
    );
    let above = ActionParameters::new().with("line", ParameterValue::Integer(1_000_001));
    let refused = crate::ActionHandler::execute(&handler, &above, &context);
    assert_eq!(refused.status, crate::ActionStatus::Failed);
    assert!(
        refused
            .message
            .starts_with("parameter `line` is 1000001, above 1000000"),
        "{}",
        refused.message
    );
    assert_eq!(
        crate::ActionHandler::check(&handler, &above, &profile).len(),
        1
    );
}
