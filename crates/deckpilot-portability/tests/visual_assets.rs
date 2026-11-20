//! Keys refer to the shared visual asset library by id (`docs/key-text.md`,
//! "Icons and backgrounds"): an export carries the references as they are
//! and the profile images (My Icons) they use, as icon or background;
//! DeckPilot's own assets need nothing; a plugin's travel as references and
//! draw once the plugin is installed where the export is imported.

use deckpilot_core::{
    AssetRef, Background, BindingTarget, ControlBinding, DeviceLayout, IconFit, IconRef,
    KeyAppearance, LayoutPage, Os, Profile, Trigger,
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

fn sealed(payload: &Payload) -> Payload {
    let password = Password::new("test-export-visual-assets-password");
    let bytes = payload::seal(payload, &password, KdfParams::for_tests()).unwrap();
    payload::open(&bytes, &password).unwrap()
}

fn look(icon: &str, background: &str) -> KeyAppearance {
    KeyAppearance {
        icon: Some(IconRef {
            id: AssetRef::parse(icon).unwrap(),
            fit: IconFit::Fit,
        }),
        background: Some(Background::parse(background).unwrap()),
        ..KeyAppearance::default()
    }
}

#[test]
fn asset_references_and_the_images_they_use_round_trip_through_an_export() {
    let modules = deckpilot_modules::builtin(Os::Linux);
    let image = deckpilot_render::icons::import(
        "Photo",
        &deckpilot_render::frame_png(&deckpilot_device::Frame::filled(8, 8, [9, 90, 200])).unwrap(),
    )
    .unwrap();
    let mut work = Profile::new("Work");
    let photo = image.id;
    work.icons.push(image);
    let mut main = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = main.home_page;
    let folder = LayoutPage::folder("Tools", home);
    let tools = folder.id;
    main.pages.push(folder);
    let key = |index| ControlId::Key { index };
    let mut first = ControlBinding::new(
        key(0),
        Trigger::Press,
        BindingTarget::OpenPage { page: tools },
    );
    // A built-in icon on a My Icons image background.
    first.appearance = Some(look("deckpilot.house", &format!("asset:{photo}")));
    let mut second = ControlBinding::new(key(1), Trigger::Press, BindingTarget::Back);
    // A plugin's icon and gradient: references only.
    second.appearance = Some(look("acme.weather.sunny", "asset:acme.weather.dusk"));
    let mut third = ControlBinding::new(key(2), Trigger::Press, BindingTarget::Back);
    third.appearance = Some(look("deckpilot.star", "asset:deckpilot.gradient-ocean"));
    main.page_mut(home).unwrap().bindings = vec![first, second, third];
    main.explicit_tree = true;
    work.layouts.push(main.clone());

    let source = SqliteStore::open_in_memory().unwrap();
    source.save_profile(&work).unwrap();
    let payload =
        sealed(&payload::export(&source, &modules, Some(&[work.id]), OPTIONS, 1, "test").unwrap());
    // The image a key uses only as its background travels with it.
    let exported = &payload.profiles[0];
    assert!(exported.icons.iter().any(|i| i.id == photo));
    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("asset:acme.weather.dusk"));

    let target = SqliteStore::open_in_memory().unwrap();
    validate(&payload, &ModelCatalog::builtin()).unwrap();
    let options = ImportOptions {
        mode: ImportMode::Merge,
        profiles: payload.profiles.iter().map(|p| p.id).collect(),
        ..ImportOptions::default()
    };
    let (changes, _) = plan(&payload, &options, &target, &modules).unwrap();
    target.apply_changes(&changes).unwrap();
    let imported = target.load_profile(work.id).unwrap().unwrap();
    let layout = imported.layout(main.id).unwrap();
    let looks: Vec<_> = layout
        .page(home)
        .unwrap()
        .bindings
        .iter()
        .map(|b| b.appearance.clone())
        .collect();
    let expected: Vec<_> = main
        .page(home)
        .unwrap()
        .bindings
        .iter()
        .map(|b| b.appearance.clone())
        .collect();
    assert_eq!(looks, expected, "references are kept as they are");
    assert!(
        imported.icon(photo).is_some(),
        "the background image came along"
    );
    // The image is in use: it cannot be deleted while a key shows it.
    assert_eq!(imported.icon_usages(photo).len(), 1);
}
