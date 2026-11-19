//! Encrypted exports and imports of shared layouts: a layout two profiles
//! use is shared again after the import, layouts of no profile travel in an
//! export of everything (payload version 3), an export without them stays
//! readable by earlier releases (version 2), and an import never overwrites
//! a layout or action of a profile that stays with other content.

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, Os, ParameterValue, Profile, Trigger,
};
use deckpilot_device::{ControlId, DeviceModelId, ModelCatalog, models};
use deckpilot_portability::import::validate;
use deckpilot_portability::payload::{self, ExportOptions, Payload};
use deckpilot_portability::{
    ImportMode, ImportOptions, PAYLOAD_VERSION, PAYLOAD_VERSION_WITHOUT_LIBRARY, plan, preview,
};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore, SqliteStore};

const OPTIONS: ExportOptions = ExportOptions {
    module_settings: false,
    app_settings: false,
};

fn modules() -> deckpilot_core::ModuleRegistry {
    deckpilot_modules::builtin(Os::Linux)
}

fn mk2() -> DeviceModelId {
    DeviceModelId::new(models::STREAM_DECK_MK2_ID)
}

fn action(name: &str) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: ActionTypeId::new("general.log").unwrap(),
        parameters: ActionParameters::new().with("message", ParameterValue::Text(name.into())),
        description: String::new(),
        icon: None,
        label: None,
    }
}

/// The export password of this test run: generated.
fn password() -> Password {
    static PASSWORD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    Password::new(PASSWORD.get_or_init(|| {
        let mut bytes = [0u8; 16];
        deckpilot_storage::crypto::random_bytes(&mut bytes).unwrap();
        format!("test-export-{:032x}", u128::from_le_bytes(bytes))
    }))
}

fn sealed(payload: &Payload) -> Payload {
    let bytes = payload::seal(payload, &password(), KdfParams::for_tests()).unwrap();
    payload::open(&bytes, &password()).unwrap()
}

/// Work and Home share Main (its key runs Hello); a layout Spare is used by
/// no profile.
fn configuration() -> (SqliteStore, Profile, Profile, DeviceLayout, DeviceLayout) {
    let store = SqliteStore::open_in_memory().unwrap();
    let hello = action("Hello");
    let mut main = DeviceLayout::new("Main", mk2());
    let home_page = main.home_page;
    main.page_mut(home_page)
        .unwrap()
        .bindings
        .push(ControlBinding::new(
            ControlId::Key { index: 0 },
            Trigger::Press,
            BindingTarget::action(hello.id),
        ));
    let mut work = Profile::new("Work");
    work.actions.push(hello);
    work.layouts.push(main.clone());
    work.activate_layout(main.id).unwrap();
    store.save_profile(&work).unwrap();
    let home = Profile::new("Home");
    store.save_profile(&home).unwrap();
    store.assign_layout(home.id, main.id).unwrap();
    let spare = DeviceLayout::new("Spare", mk2());
    store.create_shared_layout(&spare, None).unwrap();
    let work = store.load_profile(work.id).unwrap().unwrap();
    let home = store.load_profile(home.id).unwrap().unwrap();
    (store, work, home, main, spare)
}

fn import(store: &SqliteStore, payload: &Payload, mode: ImportMode) {
    validate(payload, &ModelCatalog::builtin()).unwrap();
    let options = ImportOptions {
        mode,
        profiles: payload.profiles.iter().map(|p| p.id).collect(),
        ..ImportOptions::default()
    };
    let (changes, _) = plan(payload, &options, store, &modules()).unwrap();
    store.apply_changes(&changes).unwrap();
}

#[test]
fn an_export_of_everything_round_trips_shared_and_unused_layouts() {
    let (store, work, home, main, spare) = configuration();
    let payload = sealed(&payload::export(&store, &modules(), None, OPTIONS, 1, "test").unwrap());
    assert_eq!(payload.version, PAYLOAD_VERSION);
    assert_eq!(payload.library.layouts, vec![spare.clone()]);
    let preview = preview(&payload, &store, &modules(), Vec::new()).unwrap();
    assert_eq!(preview.unassigned_layouts, ["Spare"]);

    let target = SqliteStore::open_in_memory().unwrap();
    import(&target, &payload, ImportMode::Replace);
    assert_eq!(target.load_profile(work.id).unwrap().unwrap(), work);
    assert_eq!(target.load_profile(home.id).unwrap().unwrap(), home);
    // Main is one layout of both profiles again; Spare is there, unused.
    assert_eq!(target.layout_users(main.id).unwrap().len(), 2);
    assert_eq!(target.shared_layouts().unwrap().len(), 2);
    assert_eq!(
        target
            .shared_layout(spare.id)
            .unwrap()
            .unwrap()
            .profiles
            .len(),
        0
    );

    // Replacing everything again also removes local layouts no profile
    // uses and that the export does not have.
    let stray = DeviceLayout::new("Stray", mk2());
    target.create_shared_layout(&stray, None).unwrap();
    import(&target, &payload, ImportMode::Replace);
    assert!(target.shared_layout(stray.id).unwrap().is_none());
    assert!(target.shared_layout(spare.id).unwrap().is_some());
}

#[test]
fn an_export_of_profiles_stays_readable_by_earlier_releases() {
    let (store, work, _, main, _) = configuration();
    let payload =
        payload::export(&store, &modules(), Some(&[work.id]), OPTIONS, 1, "test").unwrap();
    // Version 2, the profiles with their layouts inline: what 0.6.0 to
    // 0.8.1 read.
    assert_eq!(payload.version, PAYLOAD_VERSION_WITHOUT_LIBRARY);
    let json = serde_json::to_value(&payload).unwrap();
    assert!(json.get("library").is_none());
    assert_eq!(json["profiles"][0]["layouts"][0]["id"], main.id.to_string());
    assert_eq!(
        json["profiles"][0]["activeLayouts"][mk2().as_str()],
        main.id.to_string()
    );
}

#[test]
fn an_import_never_overwrites_a_layout_profiles_keep_using() {
    let (store, work, home, main, _) = configuration();
    let payload =
        sealed(&payload::export(&store, &modules(), Some(&[home.id]), OPTIONS, 1, "test").unwrap());
    // Home is gone here, and Main (Work's) was changed since.
    store.delete_profile(home.id).unwrap();
    let mut edited = store.load_profile(work.id).unwrap().unwrap();
    edited.layouts[0].name = "Main, edited".into();
    edited.actions[0].name = "Hello, edited".into();
    store.save_profile(&edited).unwrap();

    import(&store, &payload, ImportMode::Merge);
    let imported = store.load_profile(home.id).unwrap().unwrap();
    // Home gets its own copy of what it had, with its references following.
    assert_eq!(imported.layouts.len(), 1);
    assert_ne!(imported.layouts[0].id, main.id);
    assert_eq!(imported.layouts[0].name, "Main");
    let bound = imported.layouts[0].bound_actions();
    assert_eq!(imported.action(bound[0]).unwrap().name, "Hello");
    assert_eq!(imported.active_layouts.len(), 0);
    // Work's layout and action are untouched.
    assert_eq!(store.load_profile(work.id).unwrap().unwrap(), edited);
    assert_eq!(store.layout_users(main.id).unwrap().len(), 1);
}

#[test]
fn an_unchanged_layout_is_shared_again_on_import() {
    let (store, _, home, main, _) = configuration();
    let payload =
        payload::export(&store, &modules(), Some(&[home.id]), OPTIONS, 1, "test").unwrap();
    store.delete_profile(home.id).unwrap();
    import(&store, &payload, ImportMode::Merge);
    // Main is the same as here: Home uses the one record again.
    assert_eq!(store.layout_users(main.id).unwrap().len(), 2);
    assert_eq!(store.shared_layouts().unwrap().len(), 2);
}

#[test]
fn layouts_of_no_profile_are_checked_like_a_profiles() {
    let (store, _, _, _, _) = configuration();
    let mut payload = payload::export(&store, &modules(), None, OPTIONS, 1, "test").unwrap();
    // A key that runs an action the export does not have.
    let home_page = payload.library.layouts[0].home_page;
    payload.library.layouts[0]
        .page_mut(home_page)
        .unwrap()
        .bindings
        .push(ControlBinding::new(
            ControlId::Key { index: 1 },
            Trigger::Press,
            BindingTarget::action(ActionInstanceId::new()),
        ));
    assert!(validate(&payload, &ModelCatalog::builtin()).is_err());
    let twice = Payload {
        library: deckpilot_portability::LibraryExport {
            layouts: vec![
                payload.library.layouts[0].clone(),
                payload.library.layouts[0].clone(),
            ],
            ..payload.library.clone()
        },
        ..payload.clone()
    };
    assert!(validate(&twice, &ModelCatalog::builtin()).is_err());
}
