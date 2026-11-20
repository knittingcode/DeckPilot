//! Text appearances of key titles (`docs/key-text.md`) are part of a
//! binding's appearance: stored with the layout (no schema change), kept by
//! layouts shared between profiles and by duplicates, in encrypted backups,
//! and absent from layouts written before they existed, which keep their
//! wrapped titles.

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget,
    ControlBinding, DeviceLayout, FitLines, KeyAppearance, ParameterValue, Profile, TextAlign,
    TextAppearance, TextMode, TextOverflow, Trigger,
};
use deckpilot_device::{ControlId, DeviceModelId, models};
use deckpilot_storage::{ConfigStore, SqliteStore};

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

/// Every text mode with its settings, one per key.
fn texts() -> Vec<TextAppearance> {
    vec![
        TextAppearance {
            mode: TextMode::Wrap,
            max_lines: Some(2),
            align: Some(TextAlign::Left),
            ..TextAppearance::default()
        },
        TextAppearance {
            mode: TextMode::SingleLine,
            size: Some(13),
            overflow: Some(TextOverflow::Clip),
            ..TextAppearance::default()
        },
        TextAppearance {
            mode: TextMode::AutoFit,
            size: Some(16),
            min_size: Some(8),
            fit: Some(FitLines::Multiline),
            max_lines: Some(3),
            ..TextAppearance::default()
        },
        TextAppearance {
            mode: TextMode::Marquee,
            speed: Some(60),
            pause: Some(400),
            align: Some(TextAlign::Right),
            ..TextAppearance::default()
        },
    ]
}

fn work() -> (Profile, DeviceLayout) {
    let mut profile = Profile::new("Work");
    let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
    let home = layout.home_page;
    for (index, text) in (0_u16..).zip(texts()) {
        let hello = action(&format!("Hello {index}"));
        let mut binding = ControlBinding::new(
            ControlId::Key { index },
            Trigger::Press,
            BindingTarget::action(hello.id),
        );
        binding.label = Some("Deploy the staging environment".into());
        binding.appearance = Some(KeyAppearance {
            text: Some(text),
            ..KeyAppearance::default()
        });
        layout.page_mut(home).unwrap().bindings.push(binding);
        profile.actions.push(hello);
    }
    profile.layouts.push(layout.clone());
    profile.activate_layout(layout.id).unwrap();
    (profile, layout)
}

fn stored_texts(layout: &DeviceLayout) -> Vec<TextAppearance> {
    layout.pages[0]
        .bindings
        .iter()
        .filter_map(|b| b.appearance.as_ref()?.text)
        .collect()
}

#[test]
fn text_appearances_round_trip_with_the_profile_and_shared_layouts() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (work, main) = work();
    store.save_profile(&work).unwrap();
    let loaded = store.load_profile(work.id).unwrap().unwrap();
    assert_eq!(loaded, work);
    assert_eq!(stored_texts(&loaded.layouts[0]), texts());

    // Shared with another profile: the same settings there.
    let home = Profile::new("Home");
    store.save_profile(&home).unwrap();
    store.assign_layout(home.id, main.id).unwrap();
    let shared = store.load_profile(home.id).unwrap().unwrap();
    assert_eq!(stored_texts(shared.layout(main.id).unwrap()), texts());
    assert_eq!(
        stored_texts(&store.shared_layout(main.id).unwrap().unwrap().layout),
        texts()
    );

    // Edited through one profile, seen by the other.
    let mut edited = shared.clone();
    let layout = edited.layouts.iter_mut().find(|l| l.id == main.id).unwrap();
    layout.pages[0].bindings[0]
        .appearance
        .as_mut()
        .unwrap()
        .text
        .as_mut()
        .unwrap()
        .max_lines = Some(4);
    store.save_profile(&edited).unwrap();
    let seen = store.load_profile(work.id).unwrap().unwrap();
    assert_eq!(stored_texts(&seen.layouts[0])[0].max_lines, Some(4));

    // A duplicate keeps them.
    let mut owner = store.load_profile(work.id).unwrap().unwrap();
    let copy = owner.duplicate_layout(main.id, "Main copy").unwrap();
    store.save_profile(&owner).unwrap();
    let copied = store.load_profile(work.id).unwrap().unwrap();
    let mut expected = texts();
    expected[0].max_lines = Some(4);
    assert_eq!(stored_texts(copied.layout(copy).unwrap()), expected);
}

/// A layout written before text modes (no `text` in its appearances) reads
/// as before: wrapped titles. Nothing is migrated or rewritten.
#[test]
fn layouts_without_text_appearances_keep_wrapping() {
    let (_, mut main) = work();
    for binding in &mut main.pages[0].bindings {
        binding.appearance = Some(KeyAppearance {
            font_size: Some(deckpilot_core::FontSize::Large),
            ..KeyAppearance::default()
        });
    }
    let mut document = serde_json::to_value(&main).unwrap();
    // The document as an earlier version wrote it: no `text` anywhere.
    assert!(!document.to_string().contains("\"text\""));
    document["pages"][0]["bindings"][0]["appearance"] = serde_json::json!({ "fontSize": "large" });
    let old: DeviceLayout = serde_json::from_value(document).unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    let mut profile = Profile::new("Old");
    profile.layouts.push(old.clone());
    store.save_profile(&profile).unwrap();
    let loaded = store.load_profile(profile.id).unwrap().unwrap();
    for binding in &loaded.layouts[0].pages[0].bindings {
        let look = binding.appearance.as_ref().unwrap();
        assert_eq!(look.text, None);
        assert_eq!(look.text().mode, TextMode::Wrap);
    }
    assert_eq!(loaded.layouts[0], old);
}

#[cfg(feature = "sqlcipher")]
struct Dir(std::path::PathBuf);

#[cfg(feature = "sqlcipher")]
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(feature = "sqlcipher")]
#[test]
fn text_appearances_survive_an_encrypted_backup() {
    use deckpilot_storage::Encryption;

    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let dir = Dir(std::env::temp_dir().join(format!(
        "deckpilot-text-backup-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    )));
    std::fs::create_dir_all(&dir.0).unwrap();
    let key = Encryption::Key(deckpilot_storage::DatabaseKey::random().unwrap());
    let (work, _) = work();
    let store = SqliteStore::open(dir.0.join("deckpilot.db"), &key).unwrap();
    store.save_profile(&work).unwrap();
    let backup = dir.0.join("backup.db");
    store.backup_to(&backup).unwrap();
    drop(store);
    let restored = SqliteStore::open(&backup, &key).unwrap();
    let loaded = restored.load_profile(work.id).unwrap().unwrap();
    assert_eq!(stored_texts(&loaded.layouts[0]), texts());
    assert_eq!(loaded, work);
}
