//! Migration 12: folder keys of stored layouts keep only what opens their
//! folder (`docs/data-model.md`, "Folder controls"), after the encrypted
//! backup every upgrade starts with. Their other bindings move to the
//! layout's `recovered` list with a migration note; nothing is lost. Clean
//! folder keys, other keys and layouts without folder keys stay as they
//! were. Shared layouts are migrated once; nested folders page by page; a
//! second start changes nothing.

#![cfg(feature = "sqlcipher")]

use std::path::{Path, PathBuf};

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, LayoutPage, PageId, Profile, RecoveryReason, Trigger, WorkflowId,
};
use deckpilot_device::{ControlId, DeviceModelId, models};
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore};

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir() -> Dir {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-folder-controls-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Dir(dir)
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn on(index: u16, trigger: Trigger, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), trigger, target)
}

fn open(page: PageId) -> BindingTarget {
    BindingTarget::OpenPage { page }
}

fn action(name: &str, action_type: &str) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: ActionTypeId::new(action_type).unwrap(),
        parameters: ActionParameters::new(),
        description: String::new(),
        icon: None,
        label: None,
    }
}

struct Fixture {
    work: Profile,
    home: Profile,
    main: DeviceLayout,
    shared: DeviceLayout,
    clean: DeviceLayout,
    tools: PageId,
    docker: PageId,
    shared_folder: PageId,
}

/// Main: Home key 1 opens Tools on a click, with a double click and a
/// long press that run an action; key 2 runs the action. Tools key 1 opens
/// the nested folder Docker on the press, with a release that runs a
/// plugin action. Shared (Work and Home): key 1 opens its folder, with a
/// release that starts a workflow. Clean: a folder key with nothing else.
fn fixture() -> Fixture {
    let mk2 = DeviceModelId::new(models::STREAM_DECK_MK2_ID);
    let run = action("Run", "general.log");
    let plugin = action("Who am I", "legacy-fixture.whoami");
    let mut main = DeviceLayout::new("Main", mk2.clone());
    let root = main.home_page;
    let mut tools = LayoutPage::folder("Tools", root);
    let docker = LayoutPage::folder("Docker", tools.id);
    let (tools_id, docker_id) = (tools.id, docker.id);
    tools.bindings.extend([
        on(1, Trigger::Press, open(docker_id)),
        on(1, Trigger::Release, BindingTarget::action(plugin.id)),
    ]);
    main.pages.extend([tools, docker]);
    main.page_mut(root).unwrap().bindings.extend([
        on(1, Trigger::Click, open(tools_id)),
        on(1, Trigger::DoubleClick, BindingTarget::action(run.id)),
        on(1, Trigger::LongPress, BindingTarget::action(run.id)),
        on(2, Trigger::Press, BindingTarget::action(run.id)),
    ]);
    main.explicit_tree = true;

    let mut shared = DeviceLayout::new("Shared", mk2.clone());
    let shared_folder = LayoutPage::folder("Shared folder", shared.home_page);
    let shared_folder_id = shared_folder.id;
    shared.pages.push(shared_folder);
    let home_page = shared.home_page;
    shared.page_mut(home_page).unwrap().bindings.extend([
        on(1, Trigger::Press, open(shared_folder_id)),
        on(
            1,
            Trigger::Release,
            BindingTarget::Workflow {
                workflow: WorkflowId::new(),
            },
        ),
    ]);
    shared.explicit_tree = true;

    let mut clean = DeviceLayout::new("Clean", mk2);
    let clean_folder = LayoutPage::folder("Clean folder", clean.home_page);
    let clean_home = clean.home_page;
    clean
        .page_mut(clean_home)
        .unwrap()
        .bindings
        .push(on(1, Trigger::Press, open(clean_folder.id)));
    clean.pages.push(clean_folder);
    clean.explicit_tree = true;

    let mut work = Profile::new("Work");
    work.actions.extend([run, plugin]);
    work.layouts.extend([main.clone(), clean.clone()]);
    Fixture {
        work,
        home: Profile::new("Home"),
        main,
        shared,
        clean,
        tools: tools_id,
        docker: docker_id,
        shared_folder: shared_folder_id,
    }
}

/// A schema 11 database (the schema before migration 12) with the fixture.
fn schema_11(path: &Path, encryption: &Encryption, f: &Fixture) {
    let store = SqliteStore::open(path, encryption).unwrap();
    store.save_profile(&f.work).unwrap();
    store.save_profile(&f.home).unwrap();
    store
        .create_shared_layout(&f.shared, Some(f.work.id))
        .unwrap();
    store.assign_layout(f.home.id, f.shared.id).unwrap();
    store
        .connection_for_tests()
        .execute_batch("DELETE FROM migration_history; PRAGMA user_version = 11;")
        .unwrap();
}

fn own(layout: &DeviceLayout, page: PageId, control: ControlId) -> Vec<&ControlBinding> {
    layout
        .page(page)
        .unwrap()
        .bindings
        .iter()
        .filter(|b| b.control == control)
        .collect()
}

/// The backup holds the schema 11 state, without the recovered list.
fn assert_backup_is_schema_11(backup: &str, encryption: &Encryption, f: &Fixture) {
    let raw = rusqlite::Connection::open(backup).unwrap();
    let Encryption::Key(secret) = encryption else {
        unreachable!()
    };
    raw.pragma_update(None, "key", format!("x'{}'", secret.to_hex().as_str()))
        .unwrap();
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 11);
    let document: String = raw
        .query_row(
            "SELECT document FROM layouts WHERE id = ?1",
            [f.main.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!document.contains("recovered"), "{document}");
    assert!(document.contains("doubleClick"), "{document}");
}

#[test]
fn the_upgrade_keeps_folder_keys_opening_and_recovers_the_rest() {
    let dir = temp_dir();
    let path = dir.0.join("deckpilot.db");
    let encryption = Encryption::Key(DatabaseKey::random().unwrap());
    let f = fixture();
    schema_11(&path, &encryption, &f);

    let store = SqliteStore::open(&path, &encryption).unwrap();
    assert_eq!(store.schema_version().unwrap(), 13);
    let history = store.migration_history().unwrap();
    assert_eq!(
        (
            history[0].from_version,
            history[0].to_version,
            history[0].result.as_str()
        ),
        (11, 13, "ok")
    );
    let backup = history[0].backup.clone().unwrap();
    assert!(backup.contains("deckpilot-schema11-"), "{backup}");

    let work = store.load_profile(f.work.id).unwrap().unwrap();
    let main = work.layout(f.main.id).unwrap();
    let root = main.home_page;
    // Home key 1: the open alone, moved to the press.
    let folder_key = own(main, root, key(1));
    assert_eq!(folder_key.len(), 1);
    assert_eq!(
        (folder_key[0].trigger, &folder_key[0].target),
        (Trigger::Press, &open(f.tools))
    );
    // Key 2 is no folder key: untouched.
    assert_eq!(own(main, root, key(2)).len(), 1);
    // Nested: Tools key 1 keeps its open, the plugin action is recovered.
    assert_eq!(own(main, f.tools, key(1)).len(), 1);
    assert_eq!(main.recovered.len(), 2);
    let home_entry = &main.recovered[0];
    assert_eq!(
        (home_entry.page, home_entry.control, home_entry.folder),
        (root, key(1), f.tools)
    );
    assert_eq!(home_entry.reason, RecoveryReason::FolderControl);
    assert_eq!(
        home_entry
            .bindings
            .iter()
            .map(|b| b.trigger)
            .collect::<Vec<_>>(),
        [Trigger::DoubleClick, Trigger::LongPress]
    );
    let nested = &main.recovered[1];
    assert_eq!((nested.page, nested.folder), (f.tools, f.docker));
    assert_eq!(
        nested.bindings[0].target.action_id(),
        Some(f.work.actions[1].id)
    );
    // The recovered actions stay available to the layout.
    assert_eq!(work.actions.len(), 2);
    // Nothing is lost: every binding is on a key or recovered.
    let count = |layout: &DeviceLayout| {
        layout.pages.iter().map(|p| p.bindings.len()).sum::<usize>()
            + layout.recovered_bindings().count()
    };
    assert_eq!(count(main), count(&f.main));
    // A clean folder key and its layout are untouched.
    assert_eq!(work.layout(f.clean.id).unwrap(), &f.clean);
    // The shared layout once, the same in both profiles.
    let home = store.load_profile(f.home.id).unwrap().unwrap();
    for profile in [&work, &home] {
        let shared = profile.layout(f.shared.id).unwrap();
        assert_eq!(own(shared, shared.home_page, key(1)).len(), 1);
        assert_eq!(shared.recovered.len(), 1);
        assert_eq!(shared.recovered[0].folder, f.shared_folder);
    }
    // One note per folder key that had more.
    let notes: Vec<_> = store
        .migration_notes()
        .unwrap()
        .into_iter()
        .filter(|n| n.schema_version == 12)
        .collect();
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(
        notes
            .iter()
            .any(|n| n.message.contains("“Tools”") && n.message.contains("Recovered gestures")),
        "{notes:?}"
    );
    drop(store);

    // Opening again changes nothing.
    let store = SqliteStore::open(&path, &encryption).unwrap();
    assert_eq!(store.migration_history().unwrap().len(), 1);
    assert_eq!(store.load_profile(f.work.id).unwrap().unwrap(), work);
    assert_eq!(
        store
            .migration_notes()
            .unwrap()
            .iter()
            .filter(|n| n.schema_version == 12)
            .count(),
        3
    );
    drop(store);

    assert_backup_is_schema_11(&backup, &encryption, &f);
}

#[test]
fn the_recovered_list_survives_saving_reloading_and_sharing() {
    let dir = temp_dir();
    let path = dir.0.join("deckpilot.db");
    let encryption = Encryption::Key(DatabaseKey::random().unwrap());
    let store = SqliteStore::open(&path, &encryption).unwrap();
    let f = fixture();
    let mut work = f.work.clone();
    let main = work.layouts.iter_mut().find(|l| l.id == f.main.id).unwrap();
    assert_eq!(main.normalize_folder_controls().len(), 2);
    store.save_profile(&work).unwrap();
    store.save_profile(&f.home).unwrap();
    store.assign_layout(f.home.id, f.main.id).unwrap();
    drop(store);
    let store = SqliteStore::open(&path, &encryption).unwrap();
    let loaded = store.load_profile(work.id).unwrap().unwrap();
    assert_eq!(loaded.layout(f.main.id), work.layout(f.main.id));
    let home = store.load_profile(f.home.id).unwrap().unwrap();
    let shared = home.layout(f.main.id).unwrap();
    assert_eq!(shared.recovered.len(), 2);
    // The profile that only shares the layout gets the recovered actions
    // with it.
    assert!(
        shared
            .bound_actions()
            .iter()
            .all(|id| home.action(*id).is_some())
    );
}
