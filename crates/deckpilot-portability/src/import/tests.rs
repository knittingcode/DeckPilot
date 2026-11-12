use super::*;
use crate::payload::ExportOptions;

const ALL: ExportOptions = ExportOptions {
    module_settings: true,
    app_settings: true,
};
const NONE: ExportOptions = ExportOptions {
    module_settings: false,
    app_settings: false,
};
use crate::payload::{self, ModuleExport};
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, MachineConfig,
    MachineResourceBinding, OnFailure, Os, ParameterValue, Project, ProjectKey, Workflow,
};
use deckpilot_storage::crypto::{KdfParams, Password};

fn modules() -> ModuleRegistry {
    deckpilot_modules::builtin(Os::Linux)
}

fn catalog() -> ModelCatalog {
    ModelCatalog::builtin()
}

/// A profile with a project mapped to `shop.repo`, an action and a workflow.
fn profile(name: &str) -> Profile {
    let mut profile = Profile::new(name);
    let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
    project
        .resources
        .insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
    profile.projects.push(project);
    let action = ActionInstance {
        id: ActionInstanceId::new(),
        name: "Repository".into(),
        action_type: ActionTypeId::new("projects.repository").unwrap(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    };
    profile
        .workflows
        .push(Workflow::new("Start").step(action.id, OnFailure::Stop));
    profile.actions.push(action);
    profile
}

/// A store with `names` profiles and this computer's binding for `shop.repo`.
fn store_with(names: &[&str]) -> (SqliteStore, Vec<Profile>) {
    let store = SqliteStore::open_in_memory().unwrap();
    let profiles: Vec<Profile> = names.iter().map(|n| profile(n)).collect();
    for p in &profiles {
        store.save_profile(p).unwrap();
    }
    let mut machine = MachineConfig::new(Os::Linux);
    machine.set_binding(MachineResourceBinding {
        key: ResourceKey::new("shop.repo").unwrap(),
        kind: ResourceKind::Directory,
        value: "/home/someone/src/shop".into(),
    });
    store.save_machine_config(&machine).unwrap();
    (store, profiles)
}

fn exported(store: &SqliteStore) -> Payload {
    payload::export(store, &modules(), None, ALL, 1, "0.4.0").unwrap()
}

fn round_trip(payload: &Payload) -> Payload {
    let bytes =
        payload::seal(payload, &Password::new("export pw"), KdfParams::for_tests()).unwrap();
    payload::open(&bytes, &Password::new("export pw")).unwrap()
}

fn apply(store: &SqliteStore, payload: &Payload, options: &ImportOptions) -> ImportSummary {
    validate(payload, &catalog()).unwrap();
    let (changes, summary) = plan(payload, options, store, &modules()).unwrap();
    store.apply_changes(&changes).unwrap();
    summary
}

fn names(store: &SqliteStore) -> Vec<String> {
    store
        .list_profiles()
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect()
}

#[test]
fn exports_portable_data_without_machine_values() {
    let (store, profiles) = store_with(&["Work", "Home"]);
    let payload = exported(&store);
    assert_eq!(payload.profiles.len(), 2);
    assert_eq!(
        payload.resources[&ResourceKey::new("shop.repo").unwrap()],
        Some(ResourceKind::Directory)
    );
    let text = serde_json::to_string(&payload).unwrap();
    assert!(
        !text.contains("/home/someone"),
        "machine paths are never exported"
    );
    // Portable module settings only: the projects module exports wrapAround.
    assert_eq!(payload.modules["projects"].version, 1);
    assert!(
        payload.modules["projects"]
            .settings
            .boolean("wrapAround")
            .is_some()
    );
    assert_eq!(round_trip(&payload), payload);

    let only_home = payload::export(
        &store,
        &modules(),
        Some(&[profiles[1].id]),
        NONE,
        1,
        "0.4.0",
    )
    .unwrap();
    assert_eq!(only_home.profiles.len(), 1);
    assert_eq!(only_home.profiles[0].name, "Home");
    assert!(only_home.modules.is_empty());
    assert!(payload::export(&store, &modules(), Some(&[ProfileId::new()]), NONE, 1, "x").is_err());
}

#[test]
fn merging_into_an_empty_installation_keeps_identifiers() {
    let (source, profiles) = store_with(&["Work", "Home"]);
    let payload = round_trip(&exported(&source));
    let target = SqliteStore::open_in_memory().unwrap();
    let preview = preview(&payload, &target, &modules(), Vec::new()).unwrap();
    assert!(preview.profiles.iter().all(|p| p.conflict.is_none()));
    assert_eq!(
        preview.missing_resources.len(),
        1,
        "shop.repo is not bound here"
    );
    assert_eq!(
        preview.missing_resources[0].kind,
        Some(ResourceKind::Directory)
    );
    assert_eq!(preview.profiles[0].modules, ["projects"]);

    let summary = apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![profiles[0].id],
            ..ImportOptions::default()
        },
    );
    assert_eq!(summary.imported.len(), 1);
    assert_eq!(
        target.load_profile(profiles[0].id).unwrap().as_ref(),
        Some(&profiles[0])
    );
    // Only the selected profile; imported profiles are not activated and
    // the machine configuration is not created from the export.
    assert_eq!(names(&target), ["Work"]);
    assert_eq!(target.active_profile().unwrap(), None);
    assert_eq!(target.load_machine_config().unwrap(), None);
}

#[test]
fn conflicts_are_resolved_as_chosen() {
    let (store, profiles) = store_with(&["Work"]);
    let mut changed = profiles[0].clone();
    changed.description = "from the other computer".into();
    let payload = Payload {
        profiles: vec![changed.clone()],
        ..exported(&store)
    };
    let preview = preview(&payload, &store, &modules(), Vec::new()).unwrap();
    assert!(matches!(
        preview.profiles[0].conflict,
        Some(Conflict::SameId { .. })
    ));
    let options = |resolution| ImportOptions {
        profiles: vec![changed.id],
        conflicts: [(changed.id, resolution)].into(),
        ..ImportOptions::default()
    };

    let summary = apply(&store, &payload, &options(ConflictResolution::Skip));
    assert_eq!(summary.skipped, ["Work"]);
    assert_eq!(names(&store), ["Work"]);

    // Keep both: an independent copy with new identifiers everywhere.
    let summary = apply(&store, &payload, &options(ConflictResolution::KeepBoth));
    assert_eq!(summary.renamed, ["Work (imported)"]);
    assert_eq!(names(&store), ["Work", "Work (imported)"]);
    let copy_id = summary.imported[0].0;
    let copy = store.load_profile(copy_id).unwrap().unwrap();
    assert_ne!(copy.projects[0].id, changed.projects[0].id);
    assert_eq!(copy.workflows[0].steps[0].action, copy.actions[0].id);
    assert_eq!(
        store.load_profile(changed.id).unwrap().unwrap().description,
        ""
    );

    let summary = apply(&store, &payload, &options(ConflictResolution::Replace));
    assert_eq!(summary.removed.len(), 1);
    assert_eq!(
        store.load_profile(changed.id).unwrap().unwrap().description,
        "from the other computer"
    );
}

#[test]
fn projects_never_end_up_in_two_profiles() {
    let (store, profiles) = store_with(&["Work"]);
    // Another profile (different id) that carries Work's project id, e.g.
    // a hand-edited export.
    let mut other = profile("Other");
    other.projects = profiles[0].projects.clone();
    let payload = Payload {
        profiles: vec![other.clone()],
        ..exported(&store)
    };
    let summary = apply(
        &store,
        &payload,
        &ImportOptions {
            profiles: vec![other.id],
            ..ImportOptions::default()
        },
    );
    assert_eq!(summary.renamed, ["Other (imported)"]);
    assert_eq!(names(&store), ["Other (imported)", "Work"]);
}

#[test]
fn replacing_removes_local_profiles_but_not_machine_resources() {
    let (store, _) = store_with(&["Old one", "Old two"]);
    let (source, incoming) = store_with(&["New"]);
    let payload = exported(&source);
    let preview = preview(&payload, &store, &modules(), Vec::new()).unwrap();
    assert_eq!(preview.local_profiles.len(), 2);
    let summary = apply(
        &store,
        &payload,
        &ImportOptions {
            mode: ImportMode::Replace,
            profiles: vec![incoming[0].id],
            ..ImportOptions::default()
        },
    );
    assert_eq!(summary.removed.len(), 2);
    assert_eq!(names(&store), ["New"]);
    let machine = store.load_machine_config().unwrap().unwrap();
    assert_eq!(machine.bindings[0].value, "/home/someone/src/shop");
}

#[test]
fn module_settings_follow_the_portability_contract() {
    let (store, _) = store_with(&[]);
    let mut payload = exported(&store);
    payload.modules.insert(
        "projects".into(),
        ModuleExport {
            version: 1,
            enabled: false,
            settings: ActionParameters::new()
                .with("wrapAround", ParameterValue::Boolean(false))
                .with("somethingLocal", ParameterValue::Text("x".into())),
        },
    );
    payload.modules.insert(
        "general".into(),
        ModuleExport {
            version: 99,
            enabled: true,
            settings: ActionParameters::new(),
        },
    );
    // A machine-specific permission never travels, even when an export
    // (hand-edited or from another tool) carries it.
    payload.modules.insert(
        "system".into(),
        ModuleExport {
            version: 1,
            enabled: true,
            settings: ActionParameters::new().with("allowCommands", ParameterValue::Boolean(true)),
        },
    );
    payload.modules.insert(
        "home-automation".into(),
        ModuleExport {
            version: 1,
            enabled: true,
            settings: ActionParameters::new(),
        },
    );
    let preview = preview(&payload, &store, &modules(), Vec::new()).unwrap();
    let status = |id: &str| preview.modules.iter().find(|m| m.id == id).unwrap().clone();
    assert_eq!(status("projects").status, ModuleStatus::Ready);
    assert_eq!(status("projects").skipped_settings, ["somethingLocal"]);
    assert_eq!(status("general").status, ModuleStatus::Incompatible);
    assert_eq!(status("home-automation").status, ModuleStatus::Missing);

    let summary = apply(
        &store,
        &payload,
        &ImportOptions {
            module_settings: true,
            ..ImportOptions::default()
        },
    );
    assert_eq!(
        summary.modules,
        [
            "docker",
            "git",
            "github",
            "gitlab",
            "http",
            "jira",
            "notifications",
            "projects",
            "system",
            "workspace"
        ]
    );
    assert_eq!(status("system").skipped_settings, ["allowCommands"]);
    let stored = store.module_states().unwrap();
    let projects = &stored[&ModuleId::new("projects").unwrap()];
    assert!(!projects.enabled);
    assert_eq!(projects.config.boolean("wrapAround"), Some(false));
    assert_eq!(projects.config.text("somethingLocal"), None);
    let system = &stored[&ModuleId::new("system").unwrap()];
    assert_eq!(system.config.boolean("allowCommands"), Some(false));
    assert_eq!(stored.len(), 10);
}

#[test]
fn unknown_action_types_are_kept_and_reported() {
    let (source, _) = store_with(&[]);
    let mut work = profile("Work");
    work.actions[0].action_type = ActionTypeId::new("home-automation.lights").unwrap();
    source.save_profile(&work).unwrap();
    let payload = exported(&source);
    let target = SqliteStore::open_in_memory().unwrap();
    let preview = preview(&payload, &target, &modules(), Vec::new()).unwrap();
    assert_eq!(preview.unknown_action_types, ["home-automation.lights"]);
    apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            ..ImportOptions::default()
        },
    );
    let imported = target.load_profile(work.id).unwrap().unwrap();
    assert_eq!(
        imported.actions[0].action_type.as_str(),
        "home-automation.lights"
    );
}

#[test]
fn invalid_exports_are_rejected_before_anything_is_written() {
    let (store, _) = store_with(&["Work"]);
    let base = exported(&store);

    let mut dangling = profile("Broken");
    dangling.actions.clear();
    let broken = Payload {
        profiles: vec![dangling],
        ..base.clone()
    };
    assert!(validate(&broken, &catalog()).is_err());

    let twice = Payload {
        profiles: vec![base.profiles[0].clone(), base.profiles[0].clone()],
        ..base.clone()
    };
    assert!(validate(&twice, &catalog()).is_err());

    let too_many = Payload {
        profiles: (0..=MAX_PROFILES)
            .map(|i| Profile::new(format!("P{i}")))
            .collect(),
        ..base.clone()
    };
    assert!(validate(&too_many, &catalog()).is_err());

    let mut long_name = Profile::new("x".repeat(MAX_NAME_CHARS + 1));
    long_name.projects.clear();
    let long = Payload {
        profiles: vec![long_name],
        ..base.clone()
    };
    assert!(validate(&long, &catalog()).is_err());

    // Selecting a profile that is not in the export.
    assert!(
        plan(
            &base,
            &ImportOptions {
                profiles: vec![ProfileId::new()],
                ..ImportOptions::default()
            },
            &store,
            &modules()
        )
        .is_err()
    );
    // Newer payload versions are refused when opened.
    let newer = Payload {
        version: PAYLOAD_VERSION_FOR_TESTS + 1,
        ..base
    };
    let bytes = payload::seal(&newer, &Password::new("pw"), KdfParams::for_tests()).unwrap();
    assert!(matches!(
        payload::open(&bytes, &Password::new("pw")),
        Err(PortabilityError::UnsupportedVersion(_))
    ));
    assert_eq!(names(&store), ["Work"]);
}

const PAYLOAD_VERSION_FOR_TESTS: u32 = crate::PAYLOAD_VERSION;

#[test]
fn a_failing_import_changes_nothing() {
    let (store, profiles) = store_with(&["Work"]);
    let payload = exported(&store);
    let (mut changes, _) = plan(
        &payload,
        &ImportOptions {
            profiles: vec![profiles[0].id],
            conflicts: [(profiles[0].id, ConflictResolution::KeepBoth)].into(),
            module_settings: true,
            ..ImportOptions::default()
        },
        &store,
        &modules(),
    )
    .unwrap();
    // Corrupt the plan the way a storage error would: a profile that
    // claims another profile's project.
    let mut thief = Profile::new("Thief");
    thief.projects = profiles[0].projects.clone();
    changes.save_profiles.push(thief);
    assert!(store.apply_changes(&changes).is_err());
    assert_eq!(names(&store), ["Work"]);
    assert_eq!(store.module_states().unwrap().len(), 0);
}

#[test]
fn portable_application_settings_travel_without_local_state() {
    use deckpilot_update::policy::{PREFERENCES_SETTING, UpdateMode, UpdatePreferences};

    let (source, _) = store_with(&["Work"]);
    let preferences = UpdatePreferences {
        mode: UpdateMode::DownloadAndAsk,
        automatic_checks: false,
        check_interval_hours: 72,
        last_check: Some(1_000),
        channel: deckpilot_update::channel::Channel::Prerelease,
    };
    source
        .set_setting(
            PREFERENCES_SETTING,
            &serde_json::to_string(&preferences).unwrap(),
        )
        .unwrap();
    // Security settings never leave the computer.
    source
        .set_setting("security.autoLock", "idle15Minutes")
        .unwrap();
    let payload = round_trip(&exported(&source));
    let updates = payload.settings.updates.unwrap();
    assert_eq!(updates.mode, UpdateMode::DownloadAndAsk);
    assert_eq!(updates.check_interval_hours, 72);
    let text = serde_json::to_string(&payload).unwrap();
    // Neither the last check nor the update channel is portable.
    assert!(!text.contains("lastCheck") && !text.contains("autoLock"));
    assert!(!text.contains("channel") && !text.contains("prerelease"));
    let without = payload::export(&source, &modules(), None, NONE, 1, "x").unwrap();
    assert!(without.settings.is_empty());

    let target = SqliteStore::open_in_memory().unwrap();
    let local = UpdatePreferences {
        last_check: Some(5_000),
        ..UpdatePreferences::default()
    };
    target
        .set_setting(PREFERENCES_SETTING, &serde_json::to_string(&local).unwrap())
        .unwrap();
    // Not applied unless chosen.
    apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![payload.profiles[0].id],
            ..ImportOptions::default()
        },
    );
    let stored = |store: &SqliteStore| -> UpdatePreferences {
        serde_json::from_str(&store.setting(PREFERENCES_SETTING).unwrap().unwrap()).unwrap()
    };
    assert_eq!(stored(&target), local);
    let summary = apply(
        &target,
        &payload,
        &ImportOptions {
            app_settings: true,
            ..ImportOptions::default()
        },
    );
    assert_eq!(summary.settings, ["updates"]);
    let applied = stored(&target);
    assert_eq!(applied.mode, UpdateMode::DownloadAndAsk);
    assert!(!applied.automatic_checks);
    assert_eq!(
        applied.last_check,
        Some(5_000),
        "this computer's state is kept"
    );
    assert_eq!(target.setting("security.autoLock").unwrap(), None);
}

/// A profile with an imported icon shown on a key with an appearance.
fn with_icon(store: &SqliteStore, profile: &Profile) -> Profile {
    let png = deckpilot_render::frame_png(&deckpilot_device::Frame::filled(40, 20, [9, 99, 199]))
        .unwrap();
    let mut profile = profile.clone();
    let icon = deckpilot_render::icons::import("Logo", &png).unwrap();
    let id = profile.add_icon(icon).unwrap();
    let mut layout = deckpilot_core::DeviceLayout::generate(
        "Main",
        &deckpilot_device::models::stream_deck_mk2(),
        &profile,
    );
    layout.pages[0].bindings[0].appearance = Some(deckpilot_core::KeyAppearance {
        background: Some(deckpilot_core::Color([1, 2, 3])),
        icon: Some(deckpilot_core::IconRef {
            id,
            fit: deckpilot_core::IconFit::Crop,
        }),
        ..deckpilot_core::KeyAppearance::default()
    });
    profile.layouts.push(layout);
    store.save_profile(&profile).unwrap();
    profile
}

#[test]
fn icons_and_appearances_travel_with_their_profile() {
    let (source, profiles) = store_with(&["Work"]);
    let work = with_icon(&source, &profiles[0]);
    let payload = round_trip(&exported(&source));
    assert_eq!(payload.profiles[0].icons, work.icons);

    let target = SqliteStore::open_in_memory().unwrap();
    apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            ..ImportOptions::default()
        },
    );
    let imported = target.load_profile(work.id).unwrap().unwrap();
    assert_eq!(imported, work);

    // Importing again as a copy keeps the icon and its references.
    let summary = apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            conflicts: BTreeMap::from([(work.id, ConflictResolution::KeepBoth)]),
            ..ImportOptions::default()
        },
    );
    let copy = target.load_profile(summary.imported[0].0).unwrap().unwrap();
    assert_ne!(copy.id, work.id);
    assert_eq!(copy.icons, work.icons);
    assert_eq!(copy.validate(&catalog()), []);
}

#[test]
fn crafted_icons_are_refused() {
    let (source, profiles) = store_with(&["Work"]);
    with_icon(&source, &profiles[0]);
    let payload = exported(&source);

    // Dimensions in the header that do not match the declared ones.
    let mut lying = payload.clone();
    lying.profiles[0].icons[0].width = 255;
    assert!(validate(&lying, &catalog()).is_err());
    // Not a PNG.
    let mut not_png = payload.clone();
    not_png.profiles[0].icons[0].png = deckpilot_core::IconData::new(b"GIF89a".to_vec());
    assert!(validate(&not_png, &catalog()).is_err());
    // A PNG signature and header with damaged image data.
    let mut damaged = payload.clone();
    let mut bytes = damaged.profiles[0].icons[0].png.bytes().to_vec();
    let end = bytes.len();
    bytes.truncate(end - 20);
    damaged.profiles[0].icons[0].png = deckpilot_core::IconData::new(bytes);
    assert!(validate(&damaged, &catalog()).is_err());
    // A key showing an icon the profile does not have.
    let mut dangling = payload.clone();
    dangling.profiles[0].icons.clear();
    assert!(validate(&dangling, &catalog()).is_err());
    // Profiles from before icons existed have none and stay valid.
    let mut old = serde_json::to_value(&payload.profiles[0]).unwrap();
    old.as_object_mut().unwrap().remove("icons");
    old["layouts"] = serde_json::json!([]);
    let old: Profile = serde_json::from_value(old).unwrap();
    assert_eq!(old.icons, []);
    assert_eq!(old.validate(&catalog()), []);
}

/// Click, double click with parameter overrides (a shared variable
/// reference and a project), long press on a workflow, and a key's own
/// timings.
fn with_gestures(store: &SqliteStore, profile: &Profile) -> Profile {
    use deckpilot_core::gesture::KeyGestureSettings;
    use deckpilot_core::variables::VariableRef;
    use deckpilot_core::{BindingTarget, ControlBinding, DeviceLayout, Trigger};
    use deckpilot_device::{ControlId, models};

    let mut profile = profile.clone();
    let action = profile.actions[0].id;
    let workflow = profile.workflows[0].id;
    let shop = profile.projects[0].id;
    let key = ControlId::Key { index: 2 };
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout.pages[0].bindings = vec![
        ControlBinding::new(key, Trigger::Click, BindingTarget::action(action)),
        ControlBinding::new(
            key,
            Trigger::DoubleClick,
            BindingTarget::Action {
                action,
                overrides: ActionParameters::new()
                    .with("project_id", ParameterValue::Project(shop))
                    .with(
                        "resource",
                        ParameterValue::Reference(VariableRef::parse("shared.repo.role").unwrap()),
                    ),
            },
        ),
        ControlBinding::new(
            key,
            Trigger::LongPress,
            BindingTarget::Workflow { workflow },
        ),
    ];
    layout.pages[0].gestures.push(KeyGestureSettings {
        control: key,
        double_click_ms: Some(450),
        long_press_ms: None,
    });
    profile.layouts.push(layout);
    store.save_profile(&profile).unwrap();
    profile
}

#[test]
fn gesture_bindings_travel_with_their_profile() {
    use deckpilot_core::BindingTarget;

    let (source, profiles) = store_with(&["Work"]);
    let work = with_gestures(&source, &profiles[0]);
    let payload = round_trip(&exported(&source));
    assert_eq!(payload.profiles[0].layouts, work.layouts);

    let target = SqliteStore::open_in_memory().unwrap();
    apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            ..ImportOptions::default()
        },
    );
    assert_eq!(target.load_profile(work.id).unwrap().unwrap(), work);

    // As a copy: the bindings point to the copy's action, workflow and
    // project; the overrides and timings are kept.
    let summary = apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            conflicts: BTreeMap::from([(work.id, ConflictResolution::KeepBoth)]),
            ..ImportOptions::default()
        },
    );
    let copy = target.load_profile(summary.imported[0].0).unwrap().unwrap();
    assert_ne!(copy.id, work.id);
    assert_eq!(copy.validate(&catalog()), []);
    let page = &copy.layouts[0].pages[0];
    assert_eq!(page.gestures, work.layouts[0].pages[0].gestures);
    let BindingTarget::Action { action, overrides } = &page.bindings[1].target else {
        panic!("the double click runs an action");
    };
    assert_eq!(*action, copy.actions[0].id);
    assert_eq!(overrides.project("project_id"), Some(copy.projects[0].id));
    assert_eq!(
        overrides.get("resource"),
        Some(&ParameterValue::Reference(
            deckpilot_core::variables::VariableRef::parse("shared.repo.role").unwrap()
        ))
    );
    assert_eq!(
        page.bindings[2].target,
        BindingTarget::Workflow {
            workflow: copy.workflows[0].id
        }
    );
}

/// A work session key with its own rules travels with its layout, also
/// into a copy, where it refers to nothing that would need remapping.
#[test]
fn stateful_keys_travel_with_their_profile() {
    use deckpilot_core::gesture::Gesture;
    use deckpilot_core::stateful::{StatefulBinding, StatefulRule, WorkSessionOperation};
    use deckpilot_core::work_session::WorkSessionState;
    use deckpilot_core::{BindingTarget, ControlBinding, DeviceLayout, Trigger};
    use deckpilot_device::{ControlId, models};

    let (source, profiles) = store_with(&["Work"]);
    let mut work = profiles[0].clone();
    let key = ControlId::Key { index: 7 };
    let session = BindingTarget::Stateful(StatefulBinding::WorkSession {
        rules: vec![
            StatefulRule::new(
                WorkSessionState::Idle,
                Gesture::Click,
                WorkSessionOperation::Start,
            ),
            StatefulRule::new(
                WorkSessionState::ReadyToFinish,
                Gesture::Click,
                WorkSessionOperation::Finish,
            ),
            StatefulRule::new(
                WorkSessionState::ReadyToFinish,
                Gesture::LongPress,
                WorkSessionOperation::Verify,
            ),
        ],
    });
    let defaults = BindingTarget::Stateful(StatefulBinding::work_session());
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    layout.pages[0].bindings = vec![
        ControlBinding::new(key, Trigger::Click, session.clone()),
        ControlBinding::new(key, Trigger::LongPress, session.clone()),
        ControlBinding::new(
            ControlId::Key { index: 8 },
            Trigger::Click,
            defaults.clone(),
        ),
    ];
    work.layouts.push(layout);
    assert_eq!(work.validate(&catalog()), []);
    source.save_profile(&work).unwrap();

    let payload = round_trip(&exported(&source));
    assert_eq!(payload.profiles[0].layouts, work.layouts);
    let target = SqliteStore::open_in_memory().unwrap();
    apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            ..ImportOptions::default()
        },
    );
    assert_eq!(target.load_profile(work.id).unwrap().unwrap(), work);

    let summary = apply(
        &target,
        &payload,
        &ImportOptions {
            profiles: vec![work.id],
            conflicts: BTreeMap::from([(work.id, ConflictResolution::KeepBoth)]),
            ..ImportOptions::default()
        },
    );
    let copy = target.load_profile(summary.imported[0].0).unwrap().unwrap();
    assert_ne!(copy.id, work.id);
    assert_eq!(copy.validate(&catalog()), []);
    let targets: Vec<_> = copy.layouts[0].pages[0]
        .bindings
        .iter()
        .map(|b| b.target.clone())
        .collect();
    assert_eq!(targets, [session.clone(), session, defaults]);
}
