//! The reserved Back key of folder pages: the rule, migration of folders
//! from before it, moving Back to the first key, validation and the
//! stored shape.

use deckpilot_device::{
    DeviceModel, DeviceModelId, EncoderSpec, HardwareSupport, KeyGrid, ModelCatalog,
    ModelVerification, TouchSurface, models,
};

use super::*;
use crate::gesture::KeyGestureSettings;
use crate::{
    ActionInstance, ActionInstanceId, ActionParameters, ControlBinding, LayoutIssue, LayoutPage,
    Profile, Trigger,
};

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn bind(index: u16, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, target)
}

fn mk2() -> DeviceModel {
    models::stream_deck_mk2()
}

/// Keys without a display, a dial and a touch strip with one.
fn blind_model() -> DeviceModel {
    DeviceModel {
        id: DeviceModelId::new("test.blind"),
        vendor: "Test".into(),
        name: "Keys without displays".into(),
        usb: None,
        capabilities: DeviceCapabilities {
            keys: Some(KeyGrid {
                rows: 1,
                columns: 3,
                image: None,
            }),
            encoders: Some(EncoderSpec {
                count: 1,
                pressable: true,
            }),
            touch: Some(TouchSurface {
                zones: 1,
                image: mk2().capabilities.keys.and_then(|k| k.image),
                gestures: vec![deckpilot_device::TouchGesture::Tap],
            }),
            brightness: false,
        },
        support: HardwareSupport::DescriptorOnly,
        verification: ModelVerification::default(),
        default_appearance: None,
    }
}

fn catalog() -> ModelCatalog {
    let mut catalog = ModelCatalog::builtin();
    catalog.register(blind_model());
    catalog
}

fn profile_with_action() -> (Profile, ActionInstanceId) {
    let mut profile = Profile::new("Work");
    let id = ActionInstanceId::new();
    profile.actions.push(ActionInstance {
        id,
        name: "Run".into(),
        action_type: crate::testing::echo_type(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    });
    (profile, id)
}

/// Home opens Tools (key 1); Tools opens Docker (key 1). The pages are
/// stored without a Back setting, as folders from before it.
struct Tree {
    layout: DeviceLayout,
    tools: PageId,
    docker: PageId,
}

fn tree() -> Tree {
    let mut layout = DeviceLayout::new("Main", mk2().id);
    let home = layout.home_page;
    let mut tools = LayoutPage::new("Tools");
    tools.parent = Some(home);
    let mut docker = LayoutPage::new("Docker");
    docker.parent = Some(tools.id);
    let (tools_id, docker_id) = (tools.id, docker.id);
    tools
        .bindings
        .push(bind(1, BindingTarget::OpenPage { page: docker_id }));
    layout.pages.extend([tools, docker]);
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(bind(1, BindingTarget::OpenPage { page: tools_id }));
    layout.explicit_tree = true;
    Tree {
        layout,
        tools: tools_id,
        docker: docker_id,
    }
}

#[test]
fn the_reserved_key_is_the_first_key_with_a_display() {
    assert_eq!(reserved_back_control(&mk2().capabilities), Some(key(0)));
    assert_eq!(
        reserved_back_control(&models::stream_deck_plus().capabilities),
        Some(key(0))
    );
    // Never a dial or a touch strip, also when the keys have no display.
    assert_eq!(reserved_back_control(&blind_model().capabilities), None);
}

#[test]
fn only_folder_pages_with_the_reserved_setting_have_a_system_back() {
    let mut t = tree();
    let caps = mk2().capabilities;
    let home = t.layout.home_page;
    // Unset: the behaviour from before (legacy).
    assert_eq!(t.layout.back_position(t.tools), Some(BackPosition::Legacy));
    assert_eq!(t.layout.system_back(t.tools, &caps), None);
    assert_eq!(t.layout.back_position(home), None);
    for page in [home, t.tools, t.docker] {
        t.layout.page_mut(page).unwrap().back = Some(FolderBack::RESERVED);
    }
    // Never on the home (root) page, on every page inside a folder.
    assert_eq!(t.layout.system_back(home, &caps), None);
    assert_eq!(t.layout.system_back(t.tools, &caps), Some(key(0)));
    assert_eq!(t.layout.system_back(t.docker, &caps), Some(key(0)));
    // The reserved key is the way back: no automatic one, no warning.
    assert!(t.layout.has_way_back(t.docker));
    assert_eq!(t.layout.automatic_back(t.docker, &caps.controls()), None);
    assert_eq!(t.layout.warnings(), []);
    // A model without keys with a display has none.
    assert_eq!(
        t.layout.system_back(t.tools, &blind_model().capabilities),
        None
    );
}

#[test]
fn migration_reserves_free_first_keys_and_keeps_occupied_ones_legacy() {
    let (_, action) = profile_with_action();
    let mut t = tree();
    let mut other = LayoutPage::new("Other");
    other.parent = Some(t.tools);
    // Taken by the user's action: kept, and the folder stays legacy.
    other.bindings.push(bind(0, BindingTarget::action(action)));
    let other_id = other.id;
    let mut backs = LayoutPage::new("Backs");
    backs.parent = Some(t.tools);
    // Already a Back on the first key, and an explicit one elsewhere.
    backs.bindings.push(bind(0, BindingTarget::Back));
    backs.bindings.push(ControlBinding::new(
        key(0),
        Trigger::Release,
        BindingTarget::Back,
    ));
    backs.bindings.push(bind(5, BindingTarget::Back));
    backs.gestures.push(KeyGestureSettings {
        control: key(0),
        double_click_ms: Some(400),
        long_press_ms: None,
    });
    let backs_id = backs.id;
    t.layout.pages.extend([other, backs]);
    let home = t.layout.home_page;
    let home_before = t.layout.page(home).unwrap().clone();

    let report = t.layout.assign_folder_backs(&mk2().capabilities);
    assert_eq!(report.reserved, [t.tools, t.docker, backs_id]);
    assert_eq!(report.legacy, [other_id]);
    assert_eq!(report.kept_backs, 2);
    assert!(report.changed());
    // The home page is not a folder page: untouched.
    assert_eq!(t.layout.page(home).unwrap(), &home_before);
    let backs = t.layout.page(backs_id).unwrap();
    assert_eq!(backs.back, Some(FolderBack::RESERVED));
    // Nothing is lost: the Back bindings and timings stay as they were.
    assert_eq!(backs.bindings.len(), 3);
    assert_eq!(backs.gestures.len(), 1);
    let other = t.layout.page(other_id).unwrap();
    assert_eq!(other.back, Some(FolderBack::LEGACY));
    assert_eq!(other.bindings, [bind(0, BindingTarget::action(action))]);
    // Legacy: today's automatic Back on the first free key.
    assert_eq!(
        t.layout
            .automatic_back(other_id, &mk2().capabilities.controls()),
        Some(key(1))
    );
    assert_eq!(t.layout.system_back(other_id, &mk2().capabilities), None);
    // Nested folders: the reservation is per page.
    assert_eq!(
        t.layout.system_back(t.docker, &mk2().capabilities),
        Some(key(0))
    );
    // Idempotent.
    let again = t.layout.clone();
    assert!(!t.layout.assign_folder_backs(&mk2().capabilities).changed());
    assert_eq!(t.layout, again);
}

#[test]
fn migration_of_a_model_without_display_keys_reserves_nothing_but_stays_stable() {
    let mut layout = DeviceLayout::new("Pedal", blind_model().id);
    let mut folder = LayoutPage::new("Folder");
    folder.parent = Some(layout.home_page);
    folder.bindings.push(bind(0, BindingTarget::Back));
    let id = folder.id;
    layout.pages.push(folder);
    let report = layout.assign_folder_backs(&blind_model().capabilities);
    assert_eq!(report.reserved, [id]);
    // No reserved key: no key is the system Back.
    assert_eq!(report.kept_backs, 0);
    assert_eq!(layout.page(id).unwrap().bindings.len(), 1);
    assert_eq!(layout.system_back(id, &blind_model().capabilities), None);
}

#[test]
fn moving_back_to_the_first_key_moves_the_occupant_to_the_first_free_key() {
    let (_, action) = profile_with_action();
    let mut t = tree();
    let caps = mk2().capabilities;
    {
        let tools = t.layout.page_mut(t.tools).unwrap();
        tools.back = Some(FolderBack::LEGACY);
        tools.bindings.push(bind(0, BindingTarget::action(action)));
        tools.bindings.push(ControlBinding::new(
            key(0),
            Trigger::Release,
            BindingTarget::Back,
        ));
        tools.gestures.push(KeyGestureSettings {
            control: key(0),
            double_click_ms: None,
            long_press_ms: Some(900),
        });
    }
    // Key 1 opens Docker: the first free key is key 2.
    assert_eq!(
        t.layout.move_back_to_reserved(t.tools, &caps),
        Ok(Some(key(2)))
    );
    let tools = t.layout.page(t.tools).unwrap();
    assert_eq!(tools.back, Some(FolderBack::RESERVED));
    assert_eq!(
        tools
            .bindings
            .iter()
            .find(|b| b.control == key(2))
            .map(|b| &b.target),
        Some(&BindingTarget::action(action))
    );
    // The Back binding stays on the reserved key, nothing else does.
    assert!(
        tools
            .bindings
            .iter()
            .any(|b| b.target == BindingTarget::Back && b.control == key(0))
    );
    assert!(
        tools
            .bindings
            .iter()
            .all(|b| b.control != key(0) || b.target == BindingTarget::Back)
    );
    assert_eq!(tools.gestures[0].control, key(2));
    assert_eq!(t.layout.system_back(t.tools, &caps), Some(key(0)));
    // A free first key needs no move.
    assert_eq!(t.layout.move_back_to_reserved(t.docker, &caps), Ok(None));
    // Not a folder.
    let home = t.layout.home_page;
    assert_eq!(
        t.layout.move_back_to_reserved(home, &caps),
        Err(FolderBackError::NotAFolder)
    );
    assert_eq!(
        t.layout
            .move_back_to_reserved(t.tools, &blind_model().capabilities),
        Err(FolderBackError::NoReservedKey)
    );
}

#[test]
fn moving_back_is_refused_without_a_free_key_and_changes_nothing() {
    let (_, action) = profile_with_action();
    let mut t = tree();
    let tools = t.layout.page_mut(t.tools).unwrap();
    tools.back = Some(FolderBack::LEGACY);
    tools.bindings.clear();
    for index in 0..15 {
        tools
            .bindings
            .push(bind(index, BindingTarget::action(action)));
    }
    tools.bindings.push(ControlBinding::new(
        key(0),
        Trigger::Release,
        BindingTarget::Back,
    ));
    let before = t.layout.clone();
    assert_eq!(
        t.layout.move_back_to_reserved(t.tools, &mk2().capabilities),
        Err(FolderBackError::NoFreeKey)
    );
    assert_eq!(t.layout, before);
}

#[test]
fn saving_refuses_bindings_on_the_reserved_key_and_invalid_auto_exit() {
    let (profile, action) = profile_with_action();
    let mut t = tree();
    t.layout.assign_folder_backs(&mk2().capabilities);
    assert_eq!(t.layout.validate(&catalog(), &profile), []);
    let tools = t.layout.page_mut(t.tools).unwrap();
    // A Back kept from before is fine; a plugin's (or any) action is not.
    tools.bindings.push(ControlBinding::new(
        key(0),
        Trigger::Release,
        BindingTarget::Back,
    ));
    assert_eq!(t.layout.validate(&catalog(), &profile), []);
    let tools = t.layout.page_mut(t.tools).unwrap();
    tools.bindings.push(bind(0, BindingTarget::action(action)));
    tools.auto_exit = Some(AutoExit {
        enabled: false,
        seconds: 0,
    });
    let issues = t.layout.validate(&catalog(), &profile);
    assert!(
        issues.contains(&LayoutIssue::ReservedBack {
            page: "Tools".into(),
            control: key(0),
        }),
        "{issues:?}"
    );
    assert!(issues.contains(&LayoutIssue::InvalidAutoExit {
        page: "Tools".into(),
        reason: AutoExitError::Seconds(0),
    }));
    // The same binding on a legacy folder is the user's.
    let tools = t.layout.page_mut(t.tools).unwrap();
    tools.back = Some(FolderBack::LEGACY);
    tools.auto_exit = Some(AutoExit {
        enabled: true,
        seconds: AutoExit::MAX_SECONDS,
    });
    assert_eq!(t.layout.validate(&catalog(), &profile), []);
}

#[test]
fn auto_exit_takes_whole_seconds_from_one_to_an_hour() {
    for seconds in [1, 10, 3600] {
        let on = AutoExit {
            enabled: true,
            seconds,
        };
        assert_eq!(on.check(), Ok(()));
        assert_eq!(on.duration(), Some(Duration::from_secs(seconds.into())));
    }
    for seconds in [0, 3601] {
        let bad = AutoExit {
            enabled: true,
            seconds,
        };
        assert_eq!(bad.check(), Err(AutoExitError::Seconds(seconds)));
        assert_eq!(bad.duration(), None);
    }
    assert_eq!(AutoExit::disabled().duration(), None);
    assert_eq!(AutoExit::disabled().check(), Ok(()));
    // Only folder pages exit.
    let mut t = tree();
    let home = t.layout.home_page;
    for page in [home, t.tools] {
        t.layout.page_mut(page).unwrap().auto_exit = Some(AutoExit {
            enabled: true,
            seconds: 5,
        });
    }
    assert_eq!(t.layout.auto_exit(home), None);
    assert_eq!(t.layout.auto_exit(t.tools), Some(Duration::from_secs(5)));
}

#[test]
fn folder_settings_are_stored_in_camel_case_and_left_out_when_unset() {
    let home = PageId::new();
    let folder = LayoutPage::folder("Tools", home);
    let json = serde_json::to_value(&folder).unwrap();
    assert_eq!(json["back"], serde_json::json!({ "position": "reserved" }));
    assert_eq!(
        json["autoExit"],
        serde_json::json!({ "enabled": false, "seconds": 10 })
    );
    // Pages from before the settings read and write as before.
    let old = LayoutPage::new("Old");
    let json = serde_json::to_value(&old).unwrap();
    assert!(json.get("back").is_none() && json.get("autoExit").is_none());
    let read: LayoutPage = serde_json::from_value(json).unwrap();
    assert_eq!(read, old);
    let legacy: FolderBack =
        serde_json::from_value(serde_json::json!({"position": "legacy"})).unwrap();
    assert_eq!(legacy, FolderBack::LEGACY);
    assert!(
        serde_json::from_value::<AutoExit>(
            serde_json::json!({"enabled": true, "seconds": 5, "deadline": 1})
        )
        .is_err()
    );
}
