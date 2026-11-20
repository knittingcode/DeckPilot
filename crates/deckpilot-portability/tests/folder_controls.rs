//! Folder keys in exports and imports (`docs/data-model.md`, "Folder
//! controls"): an imported folder key with more than its folder is
//! normalized as migration 12 normalized stored ones (a warning, not a
//! refusal), for profiles and for layouts of no profile, and the recovered
//! list travels through an encrypted export and back unchanged.

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, LayoutPage, Os, PageId, ParameterValue, Profile, Trigger,
};
use deckpilot_device::{ControlId, DeviceModelId, ModelCatalog, models};
use deckpilot_portability::import::validate;
use deckpilot_portability::payload::{self, ExportOptions, Payload};
use deckpilot_portability::{ImportMode, ImportOptions, plan};
use deckpilot_storage::crypto::{KdfParams, Password};
use deckpilot_storage::{ConfigStore, SqliteStore};

const OPTIONS: ExportOptions = ExportOptions {
    module_settings: false,
    app_settings: false,
};

fn modules() -> deckpilot_core::ModuleRegistry {
    deckpilot_modules::builtin(Os::Linux)
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

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

/// Work: Main's home key 1 opens the folder Tools on the press, and runs
/// Hello on a double click.
fn profile() -> (Profile, DeviceLayout, PageId) {
    let hello = ActionInstance {
        id: ActionInstanceId::new(),
        name: "Hello".into(),
        action_type: ActionTypeId::new("general.log").unwrap(),
        parameters: ActionParameters::new().with("message", ParameterValue::Text("hi".into())),
        description: String::new(),
        icon: None,
        label: None,
    };
    let mut main = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = main.home_page;
    let tools = LayoutPage::folder("Tools", home);
    let tools_id = tools.id;
    main.pages.push(tools);
    main.page_mut(home).unwrap().bindings.extend([
        ControlBinding::new(
            key(1),
            Trigger::Press,
            BindingTarget::OpenPage { page: tools_id },
        ),
        ControlBinding::new(
            key(1),
            Trigger::DoubleClick,
            BindingTarget::action(hello.id),
        ),
    ]);
    main.explicit_tree = true;
    let mut work = Profile::new("Work");
    work.actions.push(hello);
    work.layouts.push(main.clone());
    (work, main, tools_id)
}

fn import(store: &SqliteStore, payload: &Payload) -> Vec<String> {
    let warnings = validate(payload, &ModelCatalog::builtin()).unwrap();
    let options = ImportOptions {
        mode: ImportMode::Merge,
        profiles: payload.profiles.iter().map(|p| p.id).collect(),
        ..ImportOptions::default()
    };
    let (changes, _) = plan(payload, &options, store, &modules()).unwrap();
    store.apply_changes(&changes).unwrap();
    warnings
}

#[test]
fn an_imported_folder_key_with_more_is_normalized_with_a_warning() {
    // An export from before the rule: written by hand here, as an older
    // build stored it.
    let source = SqliteStore::open_in_memory().unwrap();
    let (work, main, tools) = profile();
    source.save_profile(&work).unwrap();
    let payload = sealed(
        &payload::export(&source, &modules(), Some(&[work.id]), OPTIONS, 1, "test").unwrap(),
    );
    let target = SqliteStore::open_in_memory().unwrap();
    let warnings = import(&target, &payload);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("“Tools”") && w.contains("recovered")),
        "{warnings:?}"
    );
    let imported = target.load_profile(work.id).unwrap().unwrap();
    let layout = imported.layout(main.id).unwrap();
    let home = layout.page(layout.home_page).unwrap();
    assert_eq!(home.bindings.len(), 1);
    assert_eq!(layout.folder_control(layout.home_page, key(1)), Some(tools));
    assert_eq!(layout.recovered.len(), 1);
    assert_eq!(
        layout.recovered[0].bindings[0].trigger,
        Trigger::DoubleClick
    );
    // Its action came along.
    let action = layout.recovered[0].bindings[0].target.action_id().unwrap();
    assert_eq!(imported.action(action).unwrap().name, "Hello");
}

#[test]
fn the_recovered_list_round_trips_through_an_export() {
    let source = SqliteStore::open_in_memory().unwrap();
    let (mut work, main, _) = profile();
    work.layouts[0].normalize_folder_controls();
    let normalized = work.layouts[0].clone();
    source.save_profile(&work).unwrap();
    // Everything: the profile, and a copy of the layout no profile uses.
    let spare = normalized.duplicated("Spare");
    source.create_shared_layout(&spare, None).unwrap();
    let payload = sealed(&payload::export(&source, &modules(), None, OPTIONS, 1, "test").unwrap());
    let json = serde_json::to_value(&payload).unwrap();
    assert!(json.to_string().contains("\"recovered\""));
    let target = SqliteStore::open_in_memory().unwrap();
    let warnings = import(&target, &payload);
    assert!(
        !warnings.iter().any(|w| w.contains("recovered")),
        "{warnings:?}"
    );
    let imported = target.load_profile(work.id).unwrap().unwrap();
    assert_eq!(imported.layout(main.id).unwrap(), &normalized);
    let spare_back = target.shared_layout(spare.id).unwrap().unwrap();
    assert_eq!(spare_back.layout.recovered, spare.recovered);
}

/// An export from before schema 13 whose folder key had a title of its own:
/// imported, the key shows the folder's name, the differing title is kept
/// (recoverable) and a warning says so, as migration 13 does.
#[test]
fn an_imported_folder_key_title_becomes_the_folders_name_with_a_warning() {
    let source = SqliteStore::open_in_memory().unwrap();
    let (mut work, main, tools) = profile();
    let home = main.home_page;
    let layout = &mut work.layouts[0];
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .retain(|b| b.trigger == Trigger::Press);
    layout.page_mut(home).unwrap().bindings[0].label = Some("My tools".into());
    source.save_profile(&work).unwrap();
    let payload = sealed(
        &payload::export(&source, &modules(), Some(&[work.id]), OPTIONS, 1, "test").unwrap(),
    );
    let target = SqliteStore::open_in_memory().unwrap();
    let warnings = import(&target, &payload);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("“Tools”") && w.contains("“My tools”")),
        "{warnings:?}"
    );
    let imported = target.load_profile(work.id).unwrap().unwrap();
    let layout = imported.layout(main.id).unwrap();
    assert_eq!(layout.page(home).unwrap().bindings[0].label, None);
    assert_eq!(layout.page(tools).unwrap().name, "Tools");
    assert_eq!(
        layout.recovered_folder_label(home, key(1)),
        Some("My tools")
    );
}
