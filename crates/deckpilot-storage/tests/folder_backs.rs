//! Migration 11: folder pages of stored layouts get their Back setting
//! (`docs/data-model.md`, "Pages and folders"), after the encrypted backup
//! every upgrade starts with, without losing a binding: free or Back-only
//! first keys become the reserved Back, occupied ones stay `legacy` with a
//! migration note. Shared layouts are migrated once for every profile that
//! uses them; nested folders page by page; unknown models not at all.

#![cfg(feature = "sqlcipher")]

use std::path::{Path, PathBuf};

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, AutoExit, BackPosition,
    BindingTarget, ControlBinding, DeviceLayout, FolderBack, LayoutPage, PageId, ParameterValue,
    Profile, Trigger,
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
        "deckpilot-folder-backs-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Dir(dir)
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn press(index: u16, target: BindingTarget) -> ControlBinding {
    ControlBinding::new(key(index), Trigger::Press, target)
}

fn folder(name: &str, parent: PageId) -> LayoutPage {
    // As stored before migration 11: no Back setting.
    let mut page = LayoutPage::new(name);
    page.parent = Some(parent);
    page
}

struct Fixture {
    work: Profile,
    home: Profile,
    main: DeviceLayout,
    shared: DeviceLayout,
    unknown: DeviceLayout,
    tools: PageId,
    docker: PageId,
    git: PageId,
    other: PageId,
    shared_folder: PageId,
    unknown_folder: PageId,
}

/// Main: Home opens Tools (empty first key); Tools holds Docker (Back on
/// key 0, an explicit Back on key 5) and Git (its key 0 runs an action);
/// Other is a top-level page. Shared (used by Work and Home) has an empty
/// folder. A layout of a model this build does not know has a folder too.
fn fixture() -> Fixture {
    let mk2 = DeviceModelId::new(models::STREAM_DECK_MK2_ID);
    let run = ActionInstance {
        id: ActionInstanceId::new(),
        name: "Run".into(),
        action_type: ActionTypeId::new("general.log").unwrap(),
        parameters: ActionParameters::new().with("message", ParameterValue::Text("run".into())),
        description: String::new(),
        icon: None,
        label: None,
    };
    let mut main = DeviceLayout::new("Main", mk2.clone());
    let root = main.home_page;
    let tools = folder("Tools", root);
    let mut docker = folder("Docker", tools.id);
    docker
        .bindings
        .extend([press(0, BindingTarget::Back), press(5, BindingTarget::Back)]);
    let mut git = folder("Git", tools.id);
    git.bindings.push(press(0, BindingTarget::action(run.id)));
    let other = LayoutPage::new("Other");
    let ids = (tools.id, docker.id, git.id, other.id);
    main.page_mut(root)
        .unwrap()
        .bindings
        .push(press(1, BindingTarget::OpenPage { page: ids.0 }));
    main.pages.extend([tools, docker, git, other]);
    main.explicit_tree = true;

    let mut shared = DeviceLayout::new("Shared", mk2);
    let shared_folder = folder("Shared folder", shared.home_page);
    let shared_folder_id = shared_folder.id;
    shared.pages.push(shared_folder);

    let mut unknown = DeviceLayout::new("Unknown", DeviceModelId::new("test.unknown"));
    let unknown_folder = folder("Elsewhere", unknown.home_page);
    let unknown_folder_id = unknown_folder.id;
    unknown.pages.push(unknown_folder);

    let mut work = Profile::new("Work");
    work.actions.push(run);
    work.layouts.extend([main.clone(), unknown.clone()]);
    Fixture {
        work,
        home: Profile::new("Home"),
        main,
        shared,
        unknown,
        tools: ids.0,
        docker: ids.1,
        git: ids.2,
        other: ids.3,
        shared_folder: shared_folder_id,
        unknown_folder: unknown_folder_id,
    }
}

/// A schema 10 database (the release before migration 11) with the
/// fixture stored as that release stored it.
fn schema_10(path: &Path, encryption: &Encryption, f: &Fixture) {
    let store = SqliteStore::open(path, encryption).unwrap();
    store.save_profile(&f.work).unwrap();
    store.save_profile(&f.home).unwrap();
    store
        .create_shared_layout(&f.shared, Some(f.work.id))
        .unwrap();
    store.assign_layout(f.home.id, f.shared.id).unwrap();
    store
        .connection_for_tests()
        .execute_batch("DELETE FROM migration_history; PRAGMA user_version = 10;")
        .unwrap();
}

fn position(layout: &DeviceLayout, page: PageId) -> Option<BackPosition> {
    layout.page(page).unwrap().back.map(|b| b.position)
}

#[test]
fn the_upgrade_reserves_back_keys_without_losing_bindings() {
    let dir = temp_dir();
    let path = dir.0.join("deckpilot.db");
    let encryption = Encryption::Key(DatabaseKey::random().unwrap());
    let f = fixture();
    schema_10(&path, &encryption, &f);

    let store = SqliteStore::open(&path, &encryption).unwrap();
    assert_eq!(store.schema_version().unwrap(), 13);
    let history = store.migration_history().unwrap();
    assert_eq!(
        (
            history[0].from_version,
            history[0].to_version,
            history[0].result.as_str()
        ),
        (10, 13, "ok")
    );
    let backup = history[0].backup.clone().unwrap();
    assert!(backup.contains("deckpilot-schema10-"), "{backup}");

    let work = store.load_profile(f.work.id).unwrap().unwrap();
    let main = work.layout(f.main.id).unwrap();
    // Folders with a free first key, or Back on it: the reserved Back.
    assert_eq!(position(main, f.tools), Some(BackPosition::Reserved));
    assert_eq!(position(main, f.docker), Some(BackPosition::Reserved));
    // An occupied first key: legacy, the action stays.
    assert_eq!(position(main, f.git), Some(BackPosition::Legacy));
    // Not folders: nothing.
    assert_eq!(position(main, main.home_page), None);
    assert_eq!(position(main, f.other), None);
    // No binding changed anywhere (Docker keeps both Back bindings).
    for (page, before) in main.pages.iter().zip(&f.main.pages) {
        assert_eq!(page.bindings, before.bindings, "{}", page.name);
        assert_eq!(page.auto_exit, None);
    }
    let capabilities = models::stream_deck_mk2().capabilities;
    assert_eq!(main.system_back(f.docker, &capabilities), Some(key(0)));
    assert_eq!(main.system_back(f.git, &capabilities), None);
    // The unknown model's layout is left as it was.
    let unknown = work.layout(f.unknown.id).unwrap();
    assert_eq!(unknown, &f.unknown);
    assert_eq!(position(unknown, f.unknown_folder), None);
    // The shared layout once, the same in both profiles.
    let home = store.load_profile(f.home.id).unwrap().unwrap();
    for profile in [&work, &home] {
        let shared = profile.layout(f.shared.id).unwrap();
        assert_eq!(
            position(shared, f.shared_folder),
            Some(BackPosition::Reserved)
        );
    }
    // One note: how to move Git's Back.
    let notes: Vec<_> = store
        .migration_notes()
        .unwrap()
        .into_iter()
        .filter(|n| n.schema_version == 11)
        .collect();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].message.contains("“Git”"), "{}", notes[0].message);
    assert!(notes[0].message.contains("Move Back to the first key"));
    assert_eq!(notes[0].profile, Some(f.work.id));
    drop(store);

    // Opening again changes nothing.
    let store = SqliteStore::open(&path, &encryption).unwrap();
    assert_eq!(store.migration_history().unwrap().len(), 1);
    assert_eq!(store.load_profile(f.work.id).unwrap().unwrap(), work);
    drop(store);

    // The backup is the schema 10 state, without the settings.
    let raw = rusqlite::Connection::open(&backup).unwrap();
    let Encryption::Key(secret) = &encryption else {
        unreachable!()
    };
    raw.pragma_update(None, "key", format!("x'{}'", secret.to_hex().as_str()))
        .unwrap();
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 10);
    let document: String = raw
        .query_row(
            "SELECT document FROM layouts WHERE id = ?1",
            [f.main.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!document.contains("\"back\":{"), "{document}");
}

#[test]
fn settings_survive_saving_reloading_and_sharing() {
    let dir = temp_dir();
    let path = dir.0.join("deckpilot.db");
    let encryption = Encryption::Key(DatabaseKey::random().unwrap());
    let store = SqliteStore::open(&path, &encryption).unwrap();
    let f = fixture();
    let mut work = f.work.clone();
    let main = work.layouts.iter_mut().find(|l| l.id == f.main.id).unwrap();
    let tools = main.page_mut(f.tools).unwrap();
    tools.back = Some(FolderBack::RESERVED);
    tools.auto_exit = Some(AutoExit {
        enabled: true,
        seconds: 42,
    });
    main.page_mut(f.git).unwrap().back = Some(FolderBack::LEGACY);
    store.save_profile(&work).unwrap();
    store.save_profile(&f.home).unwrap();
    store.assign_layout(f.home.id, f.main.id).unwrap();
    drop(store);
    let store = SqliteStore::open(&path, &encryption).unwrap();
    let loaded = store.load_profile(work.id).unwrap().unwrap();
    assert_eq!(loaded.layout(f.main.id), work.layout(f.main.id));
    // The profile that shares the layout sees the same settings.
    let home = store.load_profile(f.home.id).unwrap().unwrap();
    assert_eq!(home.layout(f.main.id), work.layout(f.main.id));
    let tools = home.layout(f.main.id).unwrap().page(f.tools).unwrap();
    assert_eq!(
        tools.auto_exit,
        Some(AutoExit {
            enabled: true,
            seconds: 42
        })
    );
}
