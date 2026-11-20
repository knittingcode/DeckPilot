//! Migration 13: the key that opens a folder shows the folder's name, its
//! one title (`docs/data-model.md`, "Folder controls"), after the encrypted
//! backup every upgrade starts with. A title of the key's own that equals
//! the name is dropped; one that differs is kept in the layout's
//! `recovered` list with a migration note, never silently lost. Other keys
//! keep their titles (`folder_control` tests); shared layouts are migrated once; a second start
//! changes nothing.

#![cfg(feature = "sqlcipher")]

use std::path::PathBuf;

use deckpilot_core::{
    BindingTarget, ControlBinding, DeviceLayout, KeyAppearance, LayoutPage, Profile, Trigger,
};
use deckpilot_device::{ControlId, models};
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
        "deckpilot-folder-labels-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Dir(dir)
}

fn key(index: u16) -> ControlId {
    ControlId::Key { index }
}

fn titled(index: u16, target: BindingTarget, label: &str) -> ControlBinding {
    let mut binding = ControlBinding::new(key(index), Trigger::Press, target);
    binding.label = Some(label.into());
    binding
}

#[test]
fn the_upgrade_makes_the_folder_name_the_key_title_and_keeps_differing_titles() {
    let dir = temp_dir();
    let path = dir.0.join("deckpilot.db");
    let encryption = Encryption::Key(DatabaseKey::random().unwrap());
    let mut work = Profile::new("Work");
    let mut layout = DeviceLayout::new("Main", models::stream_deck_mk2().id);
    let home = layout.home_page;
    let tools = LayoutPage::folder("Tools", home);
    let media = LayoutPage::folder("Media", home);
    let (tools_id, media_id) = (tools.id, media.id);
    layout.pages.extend([tools, media]);
    let mut differing = titled(1, BindingTarget::OpenPage { page: tools_id }, "My tools");
    differing.appearance = Some(KeyAppearance {
        bold: Some(true),
        ..KeyAppearance::default()
    });
    layout.page_mut(home).unwrap().bindings = vec![
        differing,
        titled(2, BindingTarget::OpenPage { page: media_id }, "Media"),
    ];
    let original = layout.clone();
    work.layouts.push(layout);
    let store = SqliteStore::open(&path, &encryption).unwrap();
    store.save_profile(&work).unwrap();
    store
        .connection_for_tests()
        .execute_batch("DELETE FROM migration_history; PRAGMA user_version = 12;")
        .unwrap();
    drop(store);

    let store = SqliteStore::open(&path, &encryption).unwrap();
    assert_eq!(store.schema_version().unwrap(), 13);
    let history = store.migration_history().unwrap();
    assert_eq!((history[0].from_version, history[0].to_version), (12, 13));
    assert!(
        history[0]
            .backup
            .as_deref()
            .is_some_and(|b| b.contains("deckpilot-schema12-"))
    );
    let migrated = store.load_profile(work.id).unwrap().unwrap();
    let main = migrated.layout(original.id).unwrap();
    let label = |index: u16| {
        main.page(home)
            .unwrap()
            .bindings
            .iter()
            .find(|b| b.control == key(index))
            .unwrap()
            .label
            .clone()
    };
    assert_eq!(label(1), None, "the folder's name is its title");
    assert_eq!(label(2), None, "the same as the name: dropped");
    // The look and the names stay; the differing title is recoverable.
    let look = main.page(home).unwrap().bindings[0].appearance.clone();
    assert_eq!(look.and_then(|l| l.bold), Some(true));
    assert_eq!(main.page(tools_id).unwrap().name, "Tools");
    assert_eq!(main.recovered_folder_label(home, key(1)), Some("My tools"));
    let notes: Vec<_> = store
        .migration_notes()
        .unwrap()
        .into_iter()
        .filter(|n| n.schema_version == 13)
        .collect();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].message.contains("“My tools”"), "{notes:?}");
    drop(store);

    // Opening again changes nothing.
    let store = SqliteStore::open(&path, &encryption).unwrap();
    assert_eq!(store.migration_history().unwrap().len(), 1);
    assert_eq!(store.load_profile(work.id).unwrap().unwrap(), migrated);
}
