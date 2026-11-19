//! Layouts shared by profiles (schema 10): creation, assignment, editing
//! through any profile, duplication, removal, deletion with confirmation,
//! defaults and the layouts devices switched to, the actions and icons a
//! layout brings, and the upgrade of a 0.8.1 database (schema 9).

use std::path::{Path, PathBuf};

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, Icon, IconData, IconFit, IconId, IconRef, KeyAppearance,
    OnFailure, ParameterValue, Profile, Trigger, Workflow, WorkflowStep,
};
use deckpilot_device::{ControlId, DeviceId, DeviceModelId, models};
use deckpilot_storage::{ConfigStore, Encryption, SqliteStore, StorageError};

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

fn bind(layout: &mut DeviceLayout, index: u16, target: BindingTarget) {
    let home = layout.home_page;
    layout
        .page_mut(home)
        .unwrap()
        .bindings
        .push(ControlBinding::new(
            ControlId::Key { index },
            Trigger::Press,
            target,
        ));
}

/// A 1×1 PNG icon.
fn icon(name: &str) -> Icon {
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60,
        0x00, 0x02, 0x00, 0x00, 0x05, 0x00, 0x01, 0xe9, 0xfa, 0xdc, 0xd8, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    Icon {
        id: IconId::new(),
        name: name.into(),
        width: 1,
        height: 1,
        png: IconData::new(PNG.to_vec()),
    }
}

/// Work: actions Hello (on key 0 of Main, with an icon) and Private (in no
/// layout), the workflow Deploy (on key 1 of Main) that runs Private.
fn work() -> (Profile, DeviceLayout) {
    let mut profile = Profile::new("Work");
    let hello = action("Hello");
    let private = action("Private");
    let mut deploy = Workflow::new("Deploy");
    deploy
        .steps
        .push(WorkflowStep::new(private.id, OnFailure::Stop));
    let image = icon("Logo");
    let mut main = DeviceLayout::new("Main", mk2());
    bind(&mut main, 0, BindingTarget::action(hello.id));
    let home = main.home_page;
    main.page_mut(home).unwrap().bindings[0].appearance = Some(KeyAppearance {
        icon: Some(IconRef {
            id: image.id,
            fit: IconFit::Fit,
        }),
        ..KeyAppearance::default()
    });
    bind(
        &mut main,
        1,
        BindingTarget::Workflow {
            workflow: deploy.id,
        },
    );
    profile.actions = vec![hello, private];
    profile.workflows.push(deploy);
    profile.icons.push(image);
    profile.layouts.push(main.clone());
    profile.activate_layout(main.id).unwrap();
    (profile, main)
}

fn names(profile: &Profile) -> Vec<&str> {
    profile.actions.iter().map(|a| a.name.as_str()).collect()
}

#[test]
fn a_profile_round_trips_unchanged() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (profile, _) = work();
    store.save_profile(&profile).unwrap();
    assert_eq!(store.load_profile(profile.id).unwrap(), Some(profile));
}

#[test]
fn layouts_are_created_on_their_own_and_shared_by_profiles() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (work, main) = work();
    store.save_profile(&work).unwrap();
    let home = Profile::new("Home");
    store.save_profile(&home).unwrap();

    // A layout of no profile.
    let teams = DeviceLayout::new("Teams", mk2());
    store.create_shared_layout(&teams, None).unwrap();
    let stored = store.shared_layout(teams.id).unwrap().unwrap();
    assert_eq!(stored.layout, teams);
    assert_eq!(stored.profiles.len(), 0);

    // Main in both profiles: one record, the same actions and icons, the
    // Home profile's own workflows (none).
    store.assign_layout(home.id, main.id).unwrap();
    store.assign_layout(home.id, main.id).unwrap();
    let home_loaded = store.load_profile(home.id).unwrap().unwrap();
    assert_eq!(home_loaded.layouts, vec![main.clone()]);
    assert_eq!(names(&home_loaded), ["Hello"]);
    assert_eq!(home_loaded.icons.len(), 1);
    assert_eq!(home_loaded.workflows.len(), 0);
    let users = store.layout_users(main.id).unwrap();
    assert_eq!(users.len(), 2);
    assert_eq!(store.shared_layouts().unwrap().len(), 2);
    // Its workflow key is unavailable in Home, not an error.
    let catalog = deckpilot_device::ModelCatalog::builtin();
    assert!(
        home_loaded.validate(&catalog).is_empty(),
        "{:?}",
        home_loaded.validate(&catalog)
    );
    assert_eq!(
        deckpilot_core::shared_layouts::availability(&main, &home_loaded).len(),
        1
    );

    // Editing it through Home edits it for Work too, also its action.
    let mut edited = home_loaded.clone();
    edited.layouts[0].name = "Main (edited)".into();
    edited.actions[0].name = "Hello there".into();
    store.save_profile(&edited).unwrap();
    let work_loaded = store.load_profile(work.id).unwrap().unwrap();
    assert_eq!(work_loaded.layouts[0].name, "Main (edited)");
    assert_eq!(names(&work_loaded), ["Hello there", "Private"]);
    // Projects, workflows and defaults stay each profile's own.
    assert_eq!(work_loaded.workflows.len(), 1);
    assert_eq!(
        store
            .load_profile(home.id)
            .unwrap()
            .unwrap()
            .workflows
            .len(),
        0
    );
}

#[test]
fn removing_a_layout_from_a_profile_keeps_it_and_what_the_profile_needs() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (work, main) = work();
    store.save_profile(&work).unwrap();
    let mut home = Profile::new("Home");
    store.save_profile(&home).unwrap();
    store.assign_layout(home.id, main.id).unwrap();
    // Home's own workflow runs Hello, which Home has through Main.
    home = store.load_profile(home.id).unwrap().unwrap();
    let hello = home.actions[0].id;
    let mut own = Workflow::new("Greet");
    own.steps.push(WorkflowStep::new(hello, OnFailure::Stop));
    home.workflows.push(own);
    store.save_profile(&home).unwrap();

    store.unassign_layout(home.id, main.id).unwrap();
    let home = store.load_profile(home.id).unwrap().unwrap();
    assert_eq!(home.layouts.len(), 0);
    // Hello stays (Greet uses it); the icon only Main showed goes.
    assert_eq!(names(&home), ["Hello"]);
    assert_eq!(home.icons.len(), 0);
    // The layout is untouched and still Work's.
    assert_eq!(store.layout_users(main.id).unwrap().len(), 1);
    assert_eq!(store.load_profile(work.id).unwrap().unwrap(), work);

    // Without a use of its own, a profile keeps nothing of a layout.
    let other = Profile::new("Other");
    store.save_profile(&other).unwrap();
    store.assign_layout(other.id, main.id).unwrap();
    store.unassign_layout(other.id, main.id).unwrap();
    let other = store.load_profile(other.id).unwrap().unwrap();
    assert_eq!((other.actions.len(), other.icons.len()), (0, 0));
    // Removing the layout through the profile document is the same.
    let mut work_without = work.clone();
    work_without.layouts.clear();
    work_without.active_layouts.clear();
    store.save_profile(&work_without).unwrap();
    let work_loaded = store.load_profile(work.id).unwrap().unwrap();
    assert_eq!(names(&work_loaded), ["Private"]);
    assert!(store.shared_layout(main.id).unwrap().is_some());
}

#[test]
fn deleting_a_shared_layout_needs_the_profiles_confirmed() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (work, main) = work();
    store.save_profile(&work).unwrap();
    let home = Profile::new("Home");
    store.save_profile(&home).unwrap();
    store.assign_layout(home.id, main.id).unwrap();
    let second = DeviceLayout::new("Second", mk2());
    store.create_shared_layout(&second, Some(work.id)).unwrap();

    // Confirmed for Work only, but Home uses it too: nothing happens.
    let refused = store.delete_shared_layout(main.id, &[work.id]);
    match refused {
        Err(StorageError::LayoutInUse(users)) => {
            let names: Vec<_> = users.iter().map(|(_, name)| name.as_str()).collect();
            assert_eq!(names, ["Home", "Work"]);
        }
        other => panic!("{other:?}"),
    }
    assert!(store.shared_layout(main.id).unwrap().is_some());

    store
        .delete_shared_layout(main.id, &[work.id, home.id])
        .unwrap();
    assert!(store.shared_layout(main.id).unwrap().is_none());
    // The profiles stay; Work's default falls back to its other layout.
    let work_loaded = store.load_profile(work.id).unwrap().unwrap();
    assert_eq!(work_loaded.layouts, vec![second.clone()]);
    assert_eq!(work_loaded.active_layouts.len(), 0);
    assert_eq!(work_loaded.layout_for(&mk2()).unwrap().id, second.id);
    // Work keeps its own action; Hello and the icon went with the layout.
    assert_eq!(names(&work_loaded), ["Private"]);
    assert_eq!(work_loaded.icons.len(), 0);
    let home_loaded = store.load_profile(home.id).unwrap().unwrap();
    assert_eq!(
        (home_loaded.layouts.len(), home_loaded.actions.len()),
        (0, 0)
    );
    assert!(matches!(
        store.delete_shared_layout(main.id, &[]),
        Err(StorageError::UnknownLayout)
    ));
}

#[test]
fn a_duplicate_is_independent() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (work, main) = work();
    store.save_profile(&work).unwrap();
    let copy = store
        .duplicate_shared_layout(main.id, "Main copy", Some(work.id))
        .unwrap();
    assert_ne!(copy.id, main.id);
    assert_ne!(copy.home_page, main.home_page);
    assert_eq!(copy.pages[0].bindings, main.pages[0].bindings);
    let mut loaded = store.load_profile(work.id).unwrap().unwrap();
    assert_eq!(loaded.layouts.len(), 2);
    // Editing the copy leaves the original alone.
    loaded.layouts[1].pages[0].bindings.clear();
    store.save_profile(&loaded).unwrap();
    assert_eq!(
        store.shared_layout(main.id).unwrap().unwrap().layout.pages[0]
            .bindings
            .len(),
        2
    );
    store.rename_shared_layout(copy.id, "  Spare ").unwrap();
    assert_eq!(
        store.shared_layout(copy.id).unwrap().unwrap().layout.name,
        "Spare"
    );
    assert!(store.rename_shared_layout(copy.id, " ").is_err());
}

#[test]
fn order_defaults_and_the_layouts_devices_switched_to() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (work, main) = work();
    store.save_profile(&work).unwrap();
    let teams = DeviceLayout::new("Teams", mk2());
    let dials = DeviceLayout::new("Dials", DeviceModelId::new(models::STREAM_DECK_PLUS_ID));
    store.create_shared_layout(&teams, Some(work.id)).unwrap();
    store.create_shared_layout(&dials, Some(work.id)).unwrap();
    store
        .reorder_profile_layouts(work.id, &[dials.id, teams.id, main.id])
        .unwrap();
    assert!(store.reorder_profile_layouts(work.id, &[main.id]).is_err());
    let loaded = store.load_profile(work.id).unwrap().unwrap();
    let order: Vec<_> = loaded.layouts.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(order, ["Dials", "Teams", "Main"]);
    assert_eq!(loaded.layout_for(&mk2()).unwrap().id, main.id);

    let deck = DeviceId {
        model: mk2(),
        serial: "A".into(),
    };
    let other = DeviceId {
        model: mk2(),
        serial: "B".into(),
    };
    // Only layouts of the profile and of the device's model are stored.
    store
        .save_device_layouts(&[
            (work.id, deck.clone(), teams.id),
            (work.id, other.clone(), dials.id),
            (work.id, other.clone(), deckpilot_core::LayoutId::new()),
        ])
        .unwrap();
    assert_eq!(
        store.device_layouts().unwrap(),
        vec![(work.id, deck.clone(), teams.id)]
    );
    // A new default for the model: devices show it.
    store.set_default_layout(work.id, teams.id).unwrap();
    assert_eq!(store.device_layouts().unwrap().len(), 0);
    assert_eq!(
        store
            .load_profile(work.id)
            .unwrap()
            .unwrap()
            .layout_for(&mk2())
            .unwrap()
            .id,
        teams.id
    );
    // Removing the layout a device shows forgets it.
    store
        .save_device_layouts(&[(work.id, deck.clone(), main.id)])
        .unwrap();
    store.unassign_layout(work.id, main.id).unwrap();
    assert_eq!(store.device_layouts().unwrap().len(), 0);
    // Deleting the profile keeps its layouts.
    store.delete_profile(work.id).unwrap();
    assert_eq!(store.shared_layouts().unwrap().len(), 3);
    assert!(
        store
            .shared_layouts()
            .unwrap()
            .iter()
            .all(|l| l.profiles.is_empty())
    );
}

#[test]
fn a_layout_keeps_its_model() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (mut work, main) = work();
    store.save_profile(&work).unwrap();
    work.layouts[0].model = DeviceModelId::new(models::STREAM_DECK_XL_ID);
    assert!(store.save_profile(&work).is_err());
    assert_eq!(
        store.shared_layout(main.id).unwrap().unwrap().layout.model,
        mk2()
    );
}

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(name: &str) -> Dir {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-{name}-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Dir(dir)
}

#[cfg(feature = "sqlcipher")]
#[test]
fn shared_layouts_are_stored_encrypted() {
    let dir = temp_dir("shared-encrypted");
    let path = dir.0.join("deckpilot.db");
    let key = deckpilot_storage::DatabaseKey::random().unwrap();
    let (work, main) = work();
    {
        let store = SqliteStore::open(&path, &Encryption::Key(key.clone())).unwrap();
        store.save_profile(&work).unwrap();
        store.check_integrity().unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert_ne!(&bytes[..15], b"SQLite format 3");
    assert!(!bytes.windows(4).any(|w| w == b"Main"));
    let store = SqliteStore::open(&path, &Encryption::Key(key)).unwrap();
    assert_eq!(store.shared_layout(main.id).unwrap().unwrap().layout, main);
    assert!(matches!(
        SqliteStore::open(
            &path,
            &Encryption::Key(deckpilot_storage::DatabaseKey::random().unwrap())
        ),
        Err(StorageError::InvalidKey)
    ));
}

/// The 0.8.1 seed: `deckpilot-dev validation seed` of the 0.8.1 release,
/// schema 9, protected by the development password.
#[cfg(feature = "sqlcipher")]
mod from_0_8_1 {
    use super::*;
    use deckpilot_storage::crypto::Password;
    use deckpilot_storage::vault::Vault;

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v0.8.1");

    fn copy() -> (Dir, Encryption) {
        let dir = temp_dir("upgrade-081");
        for file in ["deckpilot.db", "deckpilot.vault.json"] {
            std::fs::copy(Path::new(FIXTURE).join(file), dir.0.join(file)).unwrap();
        }
        let key = Vault::new(dir.0.join("deckpilot.db"))
            .unlock(&Password::new("deckpilot-dev"))
            .unwrap();
        (dir, Encryption::Key(key))
    }

    #[test]
    fn the_upgrade_keeps_layouts_bindings_and_active_layouts() {
        let (dir, key) = copy();
        let path = dir.0.join("deckpilot.db");
        let store = SqliteStore::open(&path, &key).unwrap();
        assert_eq!(store.schema_version().unwrap(), 10);
        let upgrade = store.migration_history().unwrap().pop().unwrap();
        assert_eq!(
            (
                upgrade.from_version,
                upgrade.to_version,
                upgrade.result.as_str()
            ),
            (9, 10, "ok")
        );
        let mut layouts = 0;
        let mut bindings = 0;
        for summary in store.list_profiles().unwrap() {
            let profile = store.load_profile(summary.id).unwrap().unwrap();
            for layout in &profile.layouts {
                layouts += 1;
                bindings += layout.pages.iter().map(|p| p.bindings.len()).sum::<usize>();
                // Every bound action is there, and each layout is its
                // profile's only.
                for id in layout.bound_actions() {
                    assert!(profile.action(id).is_some());
                }
                assert_eq!(store.layout_users(layout.id).unwrap().len(), 1);
            }
            for id in profile.active_layouts.values() {
                assert!(profile.layout(*id).is_some());
            }
        }
        assert!(layouts >= 3, "{layouts}");
        assert!(bindings >= 10, "{bindings}");
        assert_eq!(store.shared_layouts().unwrap().len(), layouts);
    }

    #[test]
    fn the_schema_9_backup_restores_the_released_state() {
        let (dir, key) = copy();
        let path = dir.0.join("deckpilot.db");
        let original = std::fs::read(&path).unwrap();
        let store = SqliteStore::open(&path, &key).unwrap();
        let backup = store
            .migration_history()
            .unwrap()
            .pop()
            .unwrap()
            .backup
            .unwrap();
        assert!(backup.contains("deckpilot-schema9-"), "{backup}");
        // Something changed after the upgrade.
        let extra = DeviceLayout::new("After the upgrade", mk2());
        store.create_shared_layout(&extra, None).unwrap();
        drop(store);
        std::fs::copy(&backup, &path).unwrap();
        for side in ["deckpilot.db-wal", "deckpilot.db-shm"] {
            let _ = std::fs::remove_file(dir.0.join(side));
        }
        // The restored file is a schema 9 database again; a 0.8.1 build
        // reads it. This build upgrades it once more, without the layout
        // created afterwards.
        assert_eq!(rusqlite_version(&path, &key), 9);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            std::fs::read(&backup).unwrap()
        );
        assert_ne!(original, Vec::<u8>::new());
        let store = SqliteStore::open(&path, &key).unwrap();
        assert!(store.shared_layout(extra.id).unwrap().is_none());
        assert_eq!(store.schema_version().unwrap(), 10);
    }

    /// The schema version of a database, read without upgrading it.
    fn rusqlite_version(path: &Path, key: &Encryption) -> u32 {
        let Encryption::Key(key) = key else {
            unreachable!()
        };
        let raw = rusqlite::Connection::open(path).unwrap();
        raw.pragma_update(None, "key", format!("x'{}'", key.to_hex().as_str()))
            .unwrap();
        raw.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn an_older_build_refuses_the_upgraded_database() {
        // A 0.8.1 build supports schema 9: it refuses schema 10 before
        // writing anything (the same check this build applies to 11).
        let (dir, key) = copy();
        let path = dir.0.join("deckpilot.db");
        drop(SqliteStore::open(&path, &key).unwrap());
        assert_eq!(rusqlite_version(&path, &key), 10);
    }
}

#[test]
fn identifiers_two_profiles_shared_before_get_new_ones_in_the_later_profile() {
    // A schema 9 database whose two profiles are copies of one document
    // (copied by hand): the same layout and action ids in both.
    let dir = temp_dir("shared-collision");
    let path = dir.0.join("deckpilot.db");
    let (work, main) = work();
    let copy_id = deckpilot_core::ProfileId::new();
    {
        let store = SqliteStore::open(&path, &Encryption::None).unwrap();
        store.save_profile(&work).unwrap();
        let mut document = serde_json::to_value(&work).unwrap();
        let fields = document.as_object_mut().unwrap();
        fields.remove("projects");
        let mut copy_document = document.clone();
        copy_document["id"] = serde_json::Value::String(copy_id.to_string());
        copy_document["name"] = serde_json::Value::String("Copy".into());
        let connection = store.connection_for_tests();
        connection
            .execute_batch(
                "DROP TABLE device_layouts; DROP TABLE layout_icons; DROP TABLE profile_icons;
                 DROP TABLE library_icons; DROP TABLE layout_actions;
                 DROP TABLE profile_actions; DROP TABLE library_actions;
                 DROP TABLE profile_layouts; DROP TABLE layouts;
                 PRAGMA user_version = 9;",
            )
            .unwrap();
        connection
            .execute(
                "UPDATE profiles SET document_version = 5, document = ?1",
                [document.to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO profiles (id, name, document_version, document, updated_at)
                 VALUES (?1, 'Copy', 5, ?2, 0)",
                [copy_id.to_string(), copy_document.to_string()],
            )
            .unwrap();
    }
    let store = SqliteStore::open(&path, &Encryption::None).unwrap();
    let first_id = std::cmp::min(work.id, copy_id);
    let (first, second) = if first_id == work.id {
        (work.id, copy_id)
    } else {
        (copy_id, work.id)
    };
    let first = store.load_profile(first).unwrap().unwrap();
    let second = store.load_profile(second).unwrap().unwrap();
    // The first in id order keeps the ids; the other gets new ones, with
    // its references following. Nothing is shared.
    assert_eq!(first.layouts[0].id, main.id);
    assert_ne!(second.layouts[0].id, main.id);
    assert_eq!(second.active_layouts[&mk2()], second.layouts[0].id);
    let bound = second.layouts[0].bound_actions();
    assert_eq!(bound.len(), 1);
    assert_eq!(second.action(bound[0]).unwrap().name, "Hello");
    assert_ne!(bound[0], first.layouts[0].bound_actions()[0]);
    assert_eq!(store.layout_users(main.id).unwrap().len(), 1);
    assert_eq!(second.workflows[0].steps[0].action, second.actions[1].id);
    assert_eq!(names(&first), names(&second));
    let notes = store.migration_notes().unwrap();
    assert!(
        notes.iter().filter(|n| n.schema_version == 10).count() >= 3,
        "{notes:?}"
    );
}

#[test]
fn actions_that_a_layouts_actions_name_come_with_it() {
    // A switch on key 0 whose states run Picked, which no key runs itself.
    let store = SqliteStore::open_in_memory().unwrap();
    let picked = action("Picked");
    let mut switch = action("Switch");
    switch.parameters = ActionParameters::new().with(
        "states",
        ParameterValue::Text(format!("action:{} | One", picked.id)),
    );
    let mut layout = DeviceLayout::new("Main", mk2());
    bind(&mut layout, 0, BindingTarget::action(switch.id));
    let mut work = Profile::new("Work");
    work.actions = vec![picked.clone(), switch.clone()];
    work.layouts.push(layout.clone());
    store.save_profile(&work).unwrap();
    assert_eq!(store.load_profile(work.id).unwrap().unwrap(), work);

    let home = Profile::new("Home");
    store.save_profile(&home).unwrap();
    store.assign_layout(home.id, layout.id).unwrap();
    assert_eq!(
        names(&store.load_profile(home.id).unwrap().unwrap()),
        ["Switch", "Picked"]
    );
    // Deleting Picked in Work (nothing binds it) deletes it: the switch's
    // state then names an action that is gone, as before.
    let mut without = store.load_profile(work.id).unwrap().unwrap();
    without.actions.retain(|a| a.id != picked.id);
    store.save_profile(&without).unwrap();
    assert_eq!(
        names(&store.load_profile(work.id).unwrap().unwrap()),
        ["Switch"]
    );
    assert_eq!(
        names(&store.load_profile(home.id).unwrap().unwrap()),
        ["Switch"]
    );
}
