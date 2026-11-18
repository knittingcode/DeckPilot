//! Action/control compatibility on databases of earlier releases
//! (`docs/device-controls.md`, "Migration"): the storage fixtures
//! (`crates/deckpilot-storage/tests/fixtures`: 0.5.0, 0.6.0 and schema 8)
//! open with this version, every profile loads and validates, every stored
//! binding is the same, byte for byte, after loading, checking and saving
//! again, and every key binding is compatible with the built-in action
//! types. Every test works on a copy.

use std::path::{Path, PathBuf};

use deckpilot_core::{ActionRegistry, LayoutIssue, Os};
use deckpilot_device::{ControlKind, ModelCatalog};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, Encryption, SqliteStore};
use serde_json::Value;

/// The development password the fixtures were created with (test
/// fixtures, protecting demonstration data only).
const PASSWORD: &str = "deckpilot-dev";

const FIXTURES: [&str; 3] = ["v0.5.0-2", "v0.6.0", "schema8"];

struct Copy(PathBuf);

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy(fixture: &str) -> Copy {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../deckpilot-storage/tests/fixtures")
        .join(fixture);
    let mut bytes = [0u8; 8];
    deckpilot_storage::crypto::random_bytes(&mut bytes).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-controls-{fixture}-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["deckpilot.db", "deckpilot.vault.json"] {
        std::fs::copy(source.join(file), dir.join(file)).unwrap();
    }
    Copy(dir)
}

fn open(copy: &Copy) -> SqliteStore {
    let database = copy.0.join("deckpilot.db");
    let key = Vault::new(database.clone())
        .unlock(&Password::new(PASSWORD))
        .unwrap();
    SqliteStore::open(database, &Encryption::Key(key)).unwrap()
}

/// Every built-in module enabled, so that every built-in type is checked.
fn registry() -> ActionRegistry {
    let mut modules = deckpilot_modules::builtin(Os::current());
    for info in modules.modules() {
        let _ = modules.set_enabled(&info.manifest.id, true);
    }
    modules.action_registry()
}

/// The stored document of a profile and its version, as they are in the
/// database.
fn stored(store: &SqliteStore, id: &str) -> (u32, String) {
    store
        .connection_for_tests()
        .query_row(
            "SELECT document_version, document FROM profiles WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

/// Every binding of a profile document, serialized, in document order.
fn bindings(document: &Value) -> Vec<String> {
    document["layouts"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|layout| layout["pages"].as_array().into_iter().flatten())
        .flat_map(|page| page["bindings"].as_array().into_iter().flatten())
        .map(|binding| serde_json::to_string(binding).unwrap())
        .collect()
}

#[test]
fn stored_bindings_are_kept_and_key_bindings_stay_compatible() {
    let catalog = ModelCatalog::builtin();
    let registry = registry();
    let mut checked_keys = 0;
    let mut checked_bindings = 0;
    let mut reported = Vec::new();
    for fixture in FIXTURES {
        let copy = copy(fixture);
        let store = open(&copy);
        let profiles = store.list_profiles().unwrap();
        assert!(!profiles.is_empty(), "{fixture} has profiles");
        for summary in profiles {
            let id = summary.id.to_string();
            let (version, document) = stored(&store, &id);
            let before: Value = serde_json::from_str(&document).unwrap();
            let profile = store.load_profile(summary.id).unwrap().unwrap();

            // Loading changes no binding.
            let loaded = serde_json::to_value(&profile).unwrap();
            assert_eq!(bindings(&loaded), bindings(&before), "{fixture}: {id}");

            for layout in &profile.layouts {
                // Nothing that validated before is new: compatibility is
                // a separate check.
                assert!(
                    layout
                        .validate(&catalog, &profile)
                        .iter()
                        .all(|issue| !matches!(issue, LayoutIssue::IncompatibleAction { .. })),
                    "{fixture}"
                );
                let incompatible = layout.check_compatibility(&catalog, &registry, &profile);
                for page in &layout.pages {
                    for binding in &page.bindings {
                        checked_bindings += 1;
                        if binding.control.kind() == ControlKind::Key
                            && binding.target.action_id().is_some()
                        {
                            checked_keys += 1;
                        }
                    }
                }
                // Every key binding is compatible. Other controls of the
                // descriptor-only Stream Deck + (never run before, no
                // driver) are compatible where the action declares them;
                // the others are reported (and kept, below).
                for issue in incompatible {
                    let LayoutIssue::IncompatibleAction {
                        control, trigger, ..
                    } = &issue
                    else {
                        panic!("{fixture}: {issue:?}");
                    };
                    assert_ne!(control.kind(), ControlKind::Key, "{fixture}: {issue}");
                    reported.push(format!("{fixture}: {}: {control} {trigger:?}", layout.name));
                }
            }

            // Saving again keeps every binding byte for byte and the
            // document version.
            store.save_profile(&profile).unwrap();
            let (saved_version, saved) = stored(&store, &id);
            let after: Value = serde_json::from_str(&saved).unwrap();
            assert_eq!(bindings(&after), bindings(&before), "{fixture}: {id}");
            assert_eq!(saved_version, deckpilot_storage::DOCUMENT_VERSION);
            assert!(saved_version >= version);
            assert_eq!(store.load_profile(summary.id).unwrap().unwrap(), profile);
        }
    }
    assert!(
        checked_bindings > 0 && checked_keys > 0,
        "the fixtures have key bindings"
    );
    // The seeded Stream Deck + layout of every fixture binds a key-only
    // action (Log) to a tap on the touch strip: reported, kept, not run.
    // Its dial rotations run Next and Previous project, which declare
    // encoders, and stay compatible.
    assert_eq!(
        reported,
        FIXTURES.map(|fixture| format!("{fixture}: Dials: touch zone 0 Touch {{ gesture: Tap }}")),
    );
}
