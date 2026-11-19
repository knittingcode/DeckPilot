//! Dependencies (`docs/marketplace/dependencies.md`): the resolver on
//! hand-made universes, and plans carried out by the plugin manager with
//! real verified packages (consent per plugin, rollback, orphans,
//! dependency-aware removal).

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{Env, base_permissions, package_with_dependencies, with_notifications};
use deckpilot_plugin_manifest::{
    ApiLevel, ApiRequirement, Dependency, DependencyTarget, Engines, HostEnvironment, Platform,
    PluginId, VersionRange,
};
use deckpilot_plugin_store::deps::{
    BuiltinModule, Candidate, DependencyState, InstalledPlugin, Removal, Request, ResolveError,
    StepAction, Universe, action_unmet, dependency_status, missing_required, orphans_after,
    plan_removal, resolve,
};
use deckpilot_plugin_store::{
    Consent, GrantSet, InstallSource, PlannedStep, PluginState, StoreError,
};
use semver::Version;
use serde_json::{Value, json};

// ------------------------------------------------------------- universes

fn pid(text: &str) -> PluginId {
    PluginId::parse(text).unwrap()
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn dep(id: &str, range: &str) -> Dependency {
    Dependency {
        id: DependencyTarget::parse(id).unwrap(),
        version: VersionRange::parse(range).unwrap(),
        optional: None,
        justification: None,
    }
}

fn opt(id: &str, range: &str) -> Dependency {
    Dependency {
        optional: Some(true),
        justification: Some("Adds something.".into()),
        ..dep(id, range)
    }
}

fn engines(api: &str) -> Engines {
    Engines {
        deckpilot: VersionRange::parse(">=0.8.0 <1.0.0").unwrap(),
        plugin_api: ApiRequirement::parse(api).unwrap(),
        sdk: None,
    }
}

fn cand(id: &str, version: &str, dependencies: Vec<Dependency>) -> Candidate {
    Candidate {
        id: pid(id),
        name: id.to_owned(),
        version: v(version),
        engines: engines("^1.4"),
        platforms: None,
        dependencies,
        permissions: Vec::new(),
        withdrawn: None,
        publisher_verified: true,
        bundled: false,
    }
}

fn host() -> HostEnvironment {
    HostEnvironment {
        app_version: v("0.9.0"),
        plugin_api: ApiLevel::new(1, 4),
        platform: Platform::Macos,
    }
}

fn installed(
    id: &str,
    version: &str,
    state: PluginState,
    deps: Vec<Dependency>,
) -> InstalledPlugin {
    InstalledPlugin {
        id: pid(id),
        name: id.to_owned(),
        version: v(version),
        state,
        dependencies: deps,
        as_dependency: false,
        bundled: false,
        action_dependencies: Vec::new(),
    }
}

fn universe(candidates: Vec<Candidate>, plugins: Vec<InstalledPlugin>) -> Universe {
    let mut available: BTreeMap<PluginId, Vec<Candidate>> = BTreeMap::new();
    for candidate in candidates {
        available
            .entry(candidate.id.clone())
            .or_default()
            .push(candidate);
    }
    Universe {
        host: host(),
        installed: plugins.into_iter().map(|p| (p.id.clone(), p)).collect(),
        builtins: BTreeMap::from([(
            "projects".to_owned(),
            BuiltinModule {
                id: "projects".into(),
                name: "Projects".into(),
                version: v("0.9.0"),
                enabled: true,
                supported: true,
            },
        )]),
        available,
    }
}

fn order(plan: &deckpilot_plugin_store::deps::Plan) -> Vec<String> {
    plan.steps.iter().map(|s| s.id.to_string()).collect()
}

fn install(u: &Universe, id: &str) -> Result<deckpilot_plugin_store::deps::Plan, ResolveError> {
    resolve(u, &Request::install(pid(id)))
}

// ------------------------------------------------------------- resolution

#[test]
fn a_legacy_plugin_resolves_to_itself() {
    let u = universe(vec![cand("acme.root", "1.0.0", Vec::new())], Vec::new());
    let plan = install(&u, "acme.root").unwrap();
    assert_eq!(order(&plan), ["acme.root"]);
    assert!(plan.steps[0].requested);
    assert!(matches!(plan.steps[0].action, StepAction::Install { .. }));
}

#[test]
fn simple_and_transitive_dependencies_come_first() {
    let u = universe(
        vec![
            cand("acme.root", "1.0.0", vec![dep("acme.alpha", ">=1.0.0")]),
            cand(
                "acme.alpha",
                "1.0.0",
                vec![dep("acme.beta", ">=1.0.0 <2.0.0")],
            ),
            cand("acme.beta", "1.4.0", Vec::new()),
            cand("acme.beta", "2.0.0", Vec::new()),
        ],
        Vec::new(),
    );
    let plan = install(&u, "acme.root").unwrap();
    assert_eq!(order(&plan), ["acme.beta", "acme.alpha", "acme.root"]);
    // The highest version the range allows.
    assert_eq!(
        plan.step("acme.beta").unwrap().action.version(),
        &v("1.4.0")
    );
    assert_eq!(plan.downloads().count(), 3);
    let b = plan.step("acme.beta").unwrap();
    assert_eq!(b.required_by.len(), 1);
    assert_eq!(
        b.required_by[0].from.as_ref().unwrap().as_str(),
        "acme.alpha"
    );
    // Deterministic.
    assert_eq!(install(&u, "acme.root").unwrap(), plan);
}

#[test]
fn a_shared_dependency_is_installed_once_in_a_version_both_accept() {
    let u = universe(
        vec![
            cand(
                "acme.root",
                "1.0.0",
                vec![dep("acme.left", ">=1.0.0"), dep("acme.right", ">=1.0.0")],
            ),
            cand("acme.left", "1.0.0", vec![dep("acme.shared", ">=1.0.0")]),
            cand("acme.right", "1.0.0", vec![dep("acme.shared", "<1.5.0")]),
            cand("acme.shared", "1.2.0", Vec::new()),
            cand("acme.shared", "1.9.0", Vec::new()),
        ],
        Vec::new(),
    );
    let plan = install(&u, "acme.root").unwrap();
    // `left` picked 1.9.0 first; `right`'s constraint restarts the walk.
    assert_eq!(
        order(&plan),
        ["acme.shared", "acme.left", "acme.right", "acme.root"]
    );
    let shared = plan.step("acme.shared").unwrap();
    assert_eq!(shared.action.version(), &v("1.2.0"));
    assert_eq!(shared.required_by.len(), 2);
}

#[test]
fn installed_dependencies_are_kept_or_enabled_never_downloaded() {
    let u = universe(
        vec![
            cand(
                "acme.root",
                "1.0.0",
                vec![dep("acme.on", ">=1.0.0"), dep("acme.off", ">=1.0.0")],
            ),
            cand("acme.on", "3.0.0", Vec::new()),
            cand("acme.off", "3.0.0", Vec::new()),
        ],
        vec![
            installed("acme.on", "1.1.0", PluginState::Enabled, Vec::new()),
            installed("acme.off", "1.0.0", PluginState::Disabled, Vec::new()),
        ],
    );
    let plan = install(&u, "acme.root").unwrap();
    assert_eq!(
        plan.step("acme.on").unwrap().action,
        StepAction::Keep {
            version: v("1.1.0")
        }
    );
    assert_eq!(
        plan.step("acme.off").unwrap().action,
        StepAction::Enable {
            version: v("1.0.0")
        }
    );
    let downloads: Vec<String> = plan.downloads().map(|s| s.id.to_string()).collect();
    assert_eq!(downloads, ["acme.root"]);
    // A broken dependency cannot be used.
    let mut broken = u.clone();
    broken.installed.get_mut(&pid("acme.on")).unwrap().state = PluginState::Broken;
    assert!(matches!(
        install(&broken, "acme.root"),
        Err(ResolveError::Unavailable { .. })
    ));
}

#[test]
fn optional_dependencies_are_installed_only_when_selected() {
    let u = universe(
        vec![
            cand("acme.root", "1.0.0", vec![opt("acme.calendar", ">=1.0.0")]),
            cand("acme.calendar", "1.0.0", Vec::new()),
        ],
        Vec::new(),
    );
    let plan = install(&u, "acme.root").unwrap();
    assert_eq!(order(&plan), ["acme.root"]);
    assert_eq!(plan.optional.len(), 1);
    assert_eq!(plan.optional[0].dependency.id.as_str(), "acme.calendar");
    assert_eq!(plan.optional[0].installed, None);

    let mut request = Request::install(pid("acme.root"));
    request.optional.insert("acme.calendar".into());
    let plan = resolve(&u, &request).unwrap();
    assert_eq!(order(&plan), ["acme.calendar", "acme.root"]);
    assert!(plan.step("acme.calendar").unwrap().optional);
    assert_eq!(plan.optional, []);
}

#[test]
fn built_in_modules_are_checked_never_downloaded() {
    let mut u = universe(
        vec![cand("acme.root", "1.0.0", vec![dep("projects", ">=0.8.0")])],
        Vec::new(),
    );
    let plan = install(&u, "acme.root").unwrap();
    assert_eq!(
        plan.step("projects").unwrap().action,
        StepAction::Builtin {
            version: v("0.9.0"),
            enabled: true
        }
    );
    assert_eq!(plan.downloads().count(), 1);
    u.builtins.get_mut("projects").unwrap().enabled = false;
    assert!(matches!(
        install(&u, "acme.root")
            .unwrap()
            .step("projects")
            .unwrap()
            .action,
        StepAction::Builtin { enabled: false, .. }
    ));
    // Too old a DeckPilot for the range.
    let old = universe(
        vec![cand("acme.root", "1.0.0", vec![dep("projects", ">=1.0.0")])],
        Vec::new(),
    );
    assert!(matches!(
        install(&old, "acme.root"),
        Err(ResolveError::Conflict { .. })
    ));
    // No such module.
    let none = universe(
        vec![cand("acme.root", "1.0.0", vec![dep("calendar", ">=0.1.0")])],
        Vec::new(),
    );
    let error = install(&none, "acme.root").unwrap_err();
    assert!(matches!(error, ResolveError::Missing { .. }), "{error}");
    assert!(error.to_string().contains("built-in"), "{error}");
}

#[test]
fn bundled_plugins_are_installed_from_the_bundle() {
    let mut bundled = cand("deckpilot.notifications", "1.0.0", Vec::new());
    bundled.bundled = true;
    bundled.publisher_verified = false;
    let u = universe(
        vec![
            cand(
                "acme.root",
                "1.0.0",
                vec![dep("deckpilot.notifications", ">=1.0.0")],
            ),
            bundled,
        ],
        Vec::new(),
    );
    let plan = install(&u, "acme.root").unwrap();
    assert!(matches!(
        plan.step("deckpilot.notifications").unwrap().action,
        StepAction::InstallBundled { .. }
    ));
    assert_eq!(
        plan.downloads().count(),
        1,
        "the bundle is never downloaded"
    );
}

#[test]
fn missing_dependencies_and_version_conflicts_are_named() {
    let missing = universe(
        vec![cand(
            "acme.root",
            "1.0.0",
            vec![dep("acme.gone", ">=1.0.0")],
        )],
        Vec::new(),
    );
    let error = install(&missing, "acme.root").unwrap_err();
    assert!(matches!(&error, ResolveError::Missing { id, .. } if id.as_str() == "acme.gone"));
    assert!(
        error.to_string().contains("acme.root 1.0.0 needs >=1.0.0"),
        "{error}"
    );

    let conflict = universe(
        vec![
            cand(
                "acme.root",
                "1.0.0",
                vec![dep("acme.alpha", ">=1.0.0"), dep("acme.shared", ">=2.0.0")],
            ),
            cand("acme.alpha", "1.0.0", vec![dep("acme.shared", "<2.0.0")]),
            cand("acme.shared", "1.0.0", Vec::new()),
            cand("acme.shared", "2.0.0", Vec::new()),
        ],
        Vec::new(),
    );
    let error = install(&conflict, "acme.root").unwrap_err();
    let ResolveError::Conflict {
        id,
        constraints,
        available,
    } = &error
    else {
        panic!("{error}");
    };
    assert_eq!(id.as_str(), "acme.shared");
    assert_eq!(constraints.len(), 2);
    assert_eq!(available, &[v("2.0.0"), v("1.0.0")]);
}

#[test]
fn cycles_are_refused() {
    let u = universe(
        vec![
            cand("acme.root", "1.0.0", vec![dep("acme.alpha", ">=1.0.0")]),
            cand("acme.alpha", "1.0.0", vec![dep("acme.beta", ">=1.0.0")]),
            cand("acme.beta", "1.0.0", vec![dep("acme.alpha", ">=1.0.0")]),
        ],
        Vec::new(),
    );
    let error = install(&u, "acme.root").unwrap_err();
    assert_eq!(
        error,
        ResolveError::Cycle(vec![
            "acme.alpha".into(),
            "acme.beta".into(),
            "acme.alpha".into()
        ])
    );
    let to_itself = universe(
        vec![
            cand("acme.root", "1.0.0", vec![dep("acme.alpha", ">=1.0.0")]),
            cand("acme.alpha", "1.0.0", vec![dep("acme.root", ">=1.0.0")]),
        ],
        Vec::new(),
    );
    assert!(matches!(
        install(&to_itself, "acme.root"),
        Err(ResolveError::Cycle(_))
    ));
}

#[test]
fn incompatible_withdrawn_and_unverified_versions_are_skipped_or_refused() {
    let mut newer_api = cand("acme.alpha", "2.0.0", Vec::new());
    newer_api.engines = engines("^1.5");
    let mut windows = cand("acme.alpha", "1.9.0", Vec::new());
    windows.platforms = Some(vec![Platform::Windows]);
    let mut yanked = cand("acme.alpha", "1.8.0", Vec::new());
    yanked.withdrawn = Some("broken release".into());
    let fine = cand("acme.alpha", "1.7.0", Vec::new());
    let root = cand("acme.root", "1.0.0", vec![dep("acme.alpha", ">=1.0.0")]);
    let u = universe(
        vec![
            root.clone(),
            newer_api.clone(),
            windows.clone(),
            yanked.clone(),
            fine,
        ],
        Vec::new(),
    );
    // The newest version that can run here and is not withdrawn.
    assert_eq!(
        install(&u, "acme.root")
            .unwrap()
            .step("acme.alpha")
            .unwrap()
            .action
            .version(),
        &v("1.7.0")
    );
    let only_api = universe(vec![root.clone(), newer_api], Vec::new());
    let error = install(&only_api, "acme.root").unwrap_err();
    assert!(
        matches!(&error, ResolveError::Incompatible { reasons, .. }
            if matches!(reasons[0], deckpilot_plugin_manifest::Incompatibility::PluginApi { .. })),
        "{error}"
    );
    let only_windows = universe(vec![root.clone(), windows], Vec::new());
    assert!(matches!(
        install(&only_windows, "acme.root"),
        Err(ResolveError::Incompatible { .. })
    ));
    let only_yanked = universe(vec![root.clone(), yanked], Vec::new());
    assert!(matches!(
        install(&only_yanked, "acme.root"),
        Err(ResolveError::Withdrawn { .. })
    ));
    let mut unverified = cand("acme.alpha", "1.0.0", Vec::new());
    unverified.publisher_verified = false;
    let u = universe(vec![root, unverified.clone()], Vec::new());
    assert!(matches!(
        install(&u, "acme.root"),
        Err(ResolveError::UnverifiedPublisher { .. })
    ));
    // The plugin the user asked for may come from an unverified
    // publisher (the catalog says so); only dependencies may not.
    let u = universe(vec![unverified], Vec::new());
    assert!(install(&u, "acme.alpha").is_ok());
}

#[test]
fn a_shared_dependency_is_never_moved_out_of_an_installed_plugins_range() {
    let candidates = vec![
        cand("acme.root", "1.0.0", vec![dep("acme.shared", ">=2.0.0")]),
        cand("acme.shared", "1.0.0", Vec::new()),
        cand("acme.shared", "2.1.0", Vec::new()),
    ];
    let shared = installed("acme.shared", "1.0.0", PluginState::Enabled, Vec::new());
    // Another installed plugin accepts only 1.x: refused, naming it.
    let strict = universe(
        candidates.clone(),
        vec![
            shared.clone(),
            installed(
                "acme.other",
                "1.0.0",
                PluginState::Enabled,
                vec![dep("acme.shared", ">=1.0.0 <2.0.0")],
            ),
        ],
    );
    let error = install(&strict, "acme.root").unwrap_err();
    assert!(
        error.to_string().contains("acme.other 1.0.0, installed"),
        "{error}"
    );
    // It accepts 2.x too: the shared dependency is updated.
    let lenient = universe(
        candidates,
        vec![
            shared,
            installed(
                "acme.other",
                "1.0.0",
                PluginState::Enabled,
                vec![dep("acme.shared", ">=1.0.0 <3.0.0")],
            ),
        ],
    );
    let plan = install(&lenient, "acme.root").unwrap();
    let step = plan.step("acme.shared").unwrap();
    assert!(matches!(
        &step.action,
        StepAction::Update { from, candidate } if from == &v("1.0.0") && candidate.version == v("2.1.0")
    ));
    // Unless updates are not allowed.
    let mut request = Request::install(pid("acme.root"));
    request.allow_updates = false;
    assert!(matches!(
        resolve(&lenient, &request),
        Err(ResolveError::UpdateNeeded { .. })
    ));
}

#[test]
fn an_update_is_checked_against_its_dependents() {
    // Updating `acme.lib` to 2.0.0 would break the installed `acme.app`.
    let u = universe(
        vec![
            cand("acme.lib", "1.1.0", Vec::new()),
            cand("acme.lib", "2.0.0", Vec::new()),
        ],
        vec![
            installed("acme.lib", "1.0.0", PluginState::Enabled, Vec::new()),
            installed(
                "acme.app",
                "1.0.0",
                PluginState::Enabled,
                vec![dep("acme.lib", ">=1.0.0 <2.0.0")],
            ),
        ],
    );
    let mut request = Request::install(pid("acme.lib"));
    request.version = Some(v("2.0.0"));
    let error = resolve(&u, &request).unwrap_err();
    assert!(matches!(error, ResolveError::Conflict { .. }), "{error}");
    request.version = Some(v("1.1.0"));
    let plan = resolve(&u, &request).unwrap();
    assert!(matches!(
        plan.step("acme.lib").unwrap().action,
        StepAction::Update { .. }
    ));
    // A new version that needs a new dependency brings it in.
    let u = universe(
        vec![
            cand("acme.lib", "1.1.0", vec![dep("acme.extra", ">=1.0.0")]),
            cand("acme.extra", "1.0.0", Vec::new()),
        ],
        vec![installed(
            "acme.lib",
            "1.0.0",
            PluginState::Enabled,
            Vec::new(),
        )],
    );
    request.version = Some(v("1.1.0"));
    assert_eq!(
        order(&resolve(&u, &request).unwrap()),
        ["acme.extra", "acme.lib"]
    );
}

#[test]
fn status_tells_which_actions_and_plugins_can_run() {
    let deps = vec![
        dep("acme.alpha", ">=1.0.0"),
        dep("projects", ">=0.8.0"),
        opt("acme.calendar", ">=1.0.0"),
    ];
    let u = universe(
        Vec::new(),
        vec![installed(
            "acme.alpha",
            "1.0.0",
            PluginState::Disabled,
            Vec::new(),
        )],
    );
    let status = dependency_status(&deps, &u.installed, &u.builtins);
    assert_eq!(
        status[0].state,
        DependencyState::Disabled {
            version: v("1.0.0")
        }
    );
    assert!(status[1].state.is_satisfied() && status[1].builtin);
    assert_eq!(status[2].state, DependencyState::Missing);
    let why = missing_required(&status).unwrap();
    assert!(why.contains("acme.alpha, which is disabled"), "{why}");
    let why = action_unmet(&["acme.calendar".to_owned()], &status).unwrap();
    assert!(
        why.contains("acme.calendar >=1.0.0, which is not installed"),
        "{why}"
    );
    assert_eq!(action_unmet(&[], &status), None);
    let mut enabled = u.clone();
    enabled.installed.get_mut(&pid("acme.alpha")).unwrap().state = PluginState::Enabled;
    let status = dependency_status(&deps, &enabled.installed, &enabled.builtins);
    assert_eq!(
        missing_required(&status),
        None,
        "an optional one never blocks"
    );
}

#[test]
fn removal_lists_dependents_and_orphans() {
    let mut lib = installed(
        "acme.lib",
        "1.0.0",
        PluginState::Enabled,
        vec![dep("acme.base", ">=1.0.0")],
    );
    lib.as_dependency = true;
    let mut base = installed("acme.base", "1.0.0", PluginState::Enabled, Vec::new());
    base.as_dependency = true;
    let mut extra = installed("acme.extra", "1.0.0", PluginState::Enabled, Vec::new());
    extra.as_dependency = true;
    let mut app = installed(
        "acme.app",
        "1.0.0",
        PluginState::Enabled,
        vec![dep("acme.lib", ">=1.0.0"), opt("acme.extra", ">=1.0.0")],
    );
    app.action_dependencies = vec![("sync".into(), vec!["acme.extra".into()])];
    let off = installed(
        "acme.off",
        "1.0.0",
        PluginState::Disabled,
        vec![dep("acme.lib", ">=1.0.0")],
    );
    let mine = installed("acme.mine", "1.0.0", PluginState::Enabled, Vec::new());
    let graph: BTreeMap<PluginId, InstalledPlugin> = [lib, base, extra, app, off, mine]
        .into_iter()
        .map(|p| (p.id.clone(), p))
        .collect();

    let plan = plan_removal(&graph, "acme.lib", Removal::Disable);
    assert!(plan.is_blocked());
    let blocking: Vec<&str> = plan.blocking().map(|d| d.id.as_str()).collect();
    assert_eq!(
        blocking,
        ["acme.app"],
        "a disabled dependent does not block"
    );
    assert_eq!(plan.required_by.len(), 2);
    assert!(plan.orphans.is_empty(), "disabling orphans nothing");

    let plan = plan_removal(&graph, "acme.extra", Removal::Uninstall);
    assert!(!plan.is_blocked(), "an optional dependent never blocks");
    assert_eq!(plan.used_by[0].actions, ["sync"]);

    // Uninstalling the app leaves lib, then base, then extra without a
    // dependent (the disabled `acme.off` still needs lib); the user's
    // own plugin is never an orphan.
    let plan = plan_removal(&graph, "acme.app", Removal::Uninstall);
    let orphans: Vec<&str> = plan.orphans.iter().map(PluginId::as_str).collect();
    assert_eq!(orphans, ["acme.extra"]);
    let orphans = orphans_after(&graph, &["acme.app", "acme.off"]);
    let orphans: Vec<&str> = orphans.iter().map(PluginId::as_str).collect();
    assert_eq!(orphans, ["acme.extra", "acme.lib", "acme.base"]);
    assert_eq!(orphans_after(&graph, &[]), Vec::<PluginId>::new());
}

// ------------------------------------------------- plans with the manager

const DECIDED_BY: &str = "user";

fn all(package: &deckpilot_plugin_package::VerifiedPackage) -> Consent {
    Consent::all(&GrantSet::from_manifest(package.manifest()), DECIDED_BY)
}

fn sideload() -> InstallSource {
    InstallSource::Sideload
}

#[test]
fn a_plan_installs_dependencies_first_with_their_own_consent() {
    let env = Env::new();
    let mut manager = env.open();
    let base =
        package_with_dependencies("acme.base", "1.0.0", &base_permissions(), Value::Null, &[]);
    let root = package_with_dependencies(
        "acme.root",
        "1.0.0",
        &with_notifications(),
        json!([{ "id": "acme.base", "version": ">=1.0.0" }]),
        &[],
    );
    let steps = [
        PlannedStep::Install {
            package: &base,
            source: sideload(),
            consent: all(&base),
            dependency_of: Some(PluginId::parse("acme.root").unwrap()),
        },
        PlannedStep::Install {
            package: &root,
            source: sideload(),
            consent: all(&root),
            dependency_of: None,
        },
    ];
    let applied = manager.apply_plan(&steps, true).unwrap();
    assert_eq!(applied.installed.len(), 2);
    assert_eq!(
        applied
            .enabled
            .iter()
            .map(PluginId::as_str)
            .collect::<Vec<_>>(),
        ["acme.base", "acme.root"]
    );
    let store = manager.store();
    assert_eq!(
        store.plugin(&pid("acme.root")).unwrap().unwrap().state,
        PluginState::Enabled
    );
    // Each plugin holds exactly its own grants.
    let keys = |id: &str| -> BTreeSet<String> {
        store
            .grants(&pid(id))
            .unwrap()
            .into_iter()
            .map(|g| g.key.to_string())
            .collect()
    };
    assert!(!keys("acme.base").contains("notifications"));
    assert!(keys("acme.root").contains("notifications"));
    assert_eq!(keys("acme.root").len(), keys("acme.base").len() + 1);
    let auto = store.dependency_installs().unwrap();
    assert_eq!(
        auto.get(&pid("acme.base")).map(String::as_str),
        Some("acme.root")
    );
    assert!(!auto.contains_key(&pid("acme.root")));

    // Disabling or uninstalling the dependency is refused while the root
    // is enabled, unless the root goes with it.
    let error = manager
        .check_removal(&pid("acme.base"), Removal::Disable, &BTreeSet::new())
        .unwrap_err();
    assert!(
        matches!(&error, StoreError::RequiredBy { dependents, .. } if dependents == &["acme.root"])
    );
    let plan = manager
        .check_removal(
            &pid("acme.base"),
            Removal::Uninstall,
            &BTreeSet::from([pid("acme.root")]),
        )
        .unwrap();
    assert_eq!(plan.required_by.len(), 1);
    // Uninstalling the root leaves the dependency as an orphan.
    let plan = manager
        .check_removal(&pid("acme.root"), Removal::Uninstall, &BTreeSet::new())
        .unwrap();
    assert_eq!(plan.orphans, [pid("acme.base")]);
    manager
        .uninstall_by_user(
            &pid("acme.root"),
            deckpilot_plugin_store::Retention::default(),
        )
        .unwrap();
    let graph = manager.store().dependency_graph().unwrap();
    assert_eq!(orphans_after(&graph, &[]), [pid("acme.base")]);
    // Installing it explicitly adopts it: no longer an orphan.
    manager
        .apply_plan(
            &[PlannedStep::Adopt {
                id: pid("acme.base"),
            }],
            false,
        )
        .unwrap();
    let graph = manager.store().dependency_graph().unwrap();
    assert_eq!(orphans_after(&graph, &[]), Vec::<PluginId>::new());
}

#[test]
fn a_dependency_never_gets_the_requesting_plugins_permissions() {
    let env = Env::new();
    let mut manager = env.open();
    let base =
        package_with_dependencies("acme.base", "1.0.0", &base_permissions(), Value::Null, &[]);
    let root = package_with_dependencies(
        "acme.root",
        "1.0.0",
        &with_notifications(),
        json!([{ "id": "acme.base", "version": ">=1.0.0" }]),
        &[],
    );
    // The root's permission offered as the dependency's consent.
    let steps = [
        PlannedStep::Install {
            package: &base,
            source: sideload(),
            consent: all(&root),
            dependency_of: Some(pid("acme.root")),
        },
        PlannedStep::Install {
            package: &root,
            source: sideload(),
            consent: all(&root),
            dependency_of: None,
        },
    ];
    assert!(matches!(
        manager.apply_plan(&steps, true),
        Err(StoreError::UnknownGrant { .. })
    ));
    // The dependency's permission declined: nothing is installed.
    let steps = [
        PlannedStep::Install {
            package: &base,
            source: sideload(),
            consent: Consent::none(DECIDED_BY),
            dependency_of: Some(pid("acme.root")),
        },
        PlannedStep::Install {
            package: &root,
            source: sideload(),
            consent: all(&root),
            dependency_of: None,
        },
    ];
    let error = manager.apply_plan(&steps, true).unwrap_err();
    assert!(
        matches!(&error, StoreError::ConsentRequired { id, .. } if id.as_str() == "acme.base"),
        "{error}"
    );
    assert_eq!(manager.store().plugins().unwrap().len(), 0);
    // Without activating, it installs disabled without the grant.
    manager.apply_plan(&steps, false).unwrap();
    let base_record = manager.store().plugin(&pid("acme.base")).unwrap().unwrap();
    assert_eq!(base_record.state, PluginState::Disabled);
    assert_eq!(manager.store().grants(&pid("acme.base")).unwrap().len(), 0);
}

#[test]
fn a_failed_plan_is_rolled_back() {
    let env = Env::new();
    let mut manager = env.open();
    let lib_1 =
        package_with_dependencies("acme.lib", "1.0.0", &base_permissions(), Value::Null, &[]);
    let lib_2 =
        package_with_dependencies("acme.lib", "2.0.0", &base_permissions(), Value::Null, &[]);
    let fresh =
        package_with_dependencies("acme.fresh", "1.0.0", &base_permissions(), Value::Null, &[]);
    let root = package_with_dependencies(
        "acme.root",
        "1.0.0",
        &with_notifications(),
        json!([{ "id": "acme.lib", "version": ">=2.0.0" }, { "id": "acme.fresh", "version": ">=1.0.0" }]),
        &[],
    );
    manager.install(&lib_1, &sideload(), &all(&lib_1)).unwrap();
    manager.enable(&pid("acme.lib")).unwrap();
    // The root is installed already, so its step fails after the update
    // of `acme.lib` and the install of `acme.fresh`.
    manager
        .install(&root, &sideload(), &Consent::none(DECIDED_BY))
        .unwrap();
    let before = common::snapshot(&manager);
    let steps = [
        PlannedStep::Update {
            package: &lib_2,
            source: sideload(),
            consent: Consent::none(DECIDED_BY),
        },
        PlannedStep::Install {
            package: &fresh,
            source: sideload(),
            consent: all(&fresh),
            dependency_of: Some(pid("acme.root")),
        },
        PlannedStep::Install {
            package: &root,
            source: sideload(),
            consent: all(&root),
            dependency_of: None,
        },
    ];
    let error = manager.apply_plan(&steps, true).unwrap_err();
    let StoreError::PlanFailed {
        step,
        source,
        undo_failed,
    } = &error
    else {
        panic!("{error}");
    };
    assert!(step.contains("acme.root"), "{step}");
    assert!(matches!(**source, StoreError::AlreadyInstalled { .. }));
    assert!(undo_failed.is_empty(), "{undo_failed:?}");
    let store = manager.store();
    assert!(store.plugin(&pid("acme.fresh")).unwrap().is_none());
    let lib = store.plugin(&pid("acme.lib")).unwrap().unwrap();
    assert_eq!(lib.active().unwrap().version, v("1.0.0"));
    assert_eq!(lib.state, PluginState::Enabled);
    assert!(store.dependency_installs().unwrap().is_empty());
    assert_eq!(store.journal().unwrap().len(), 0);
    assert_eq!(
        common::snapshot(&manager).plugins.len(),
        before.plugins.len()
    );
}

#[test]
fn a_plan_updates_a_shared_dependency_with_consent_for_its_escalation() {
    let env = Env::new();
    let mut manager = env.open();
    let lib_1 =
        package_with_dependencies("acme.lib", "1.0.0", &base_permissions(), Value::Null, &[]);
    let lib_2 =
        package_with_dependencies("acme.lib", "2.0.0", &with_notifications(), Value::Null, &[]);
    manager.install(&lib_1, &sideload(), &all(&lib_1)).unwrap();
    manager.enable(&pid("acme.lib")).unwrap();
    let step = |consent| PlannedStep::Update {
        package: &lib_2,
        source: sideload(),
        consent,
    };
    // The escalation must be consented to.
    assert!(matches!(
        manager.apply_plan(&[step(Consent::none(DECIDED_BY))], true),
        Err(StoreError::ConsentRequired { .. })
    ));
    let applied = manager.apply_plan(&[step(all(&lib_2))], true).unwrap();
    assert_eq!(applied.updated, [(pid("acme.lib"), v("2.0.0"))]);
    let lib = manager.store().plugin(&pid("acme.lib")).unwrap().unwrap();
    assert_eq!(lib.state, PluginState::Enabled);
    assert_eq!(lib.active().unwrap().version, v("2.0.0"));
}

#[test]
fn legacy_plugins_have_no_dependency_records() {
    let env = Env::new();
    let mut manager = env.open();
    let legacy = common::package("1.0.0", &base_permissions());
    manager
        .install(&legacy, &sideload(), &all(&legacy))
        .unwrap();
    let graph = manager.store().dependency_graph().unwrap();
    let plugin = graph.get(&common::id()).unwrap();
    assert!(plugin.dependencies.is_empty() && !plugin.as_dependency);
    assert_eq!(orphans_after(&graph, &[]), Vec::<PluginId>::new());
    let plan = manager
        .check_removal(&common::id(), Removal::Uninstall, &BTreeSet::new())
        .unwrap();
    assert!(plan.required_by.is_empty() && plan.orphans.is_empty());
}
