//! Upgrading a real DeckPilot 0.5.0 database (schema 5) to this build.
//!
//! `fixtures/v0.5.0-2/` was created by the 0.5.0-2 tools
//! (`deckpilot-dev validation seed`, from the tag `v0.5.0-2`): the
//! demonstration profiles, encrypted with SQLCipher under a key protected by
//! the development password. Every test works on a copy.

#![cfg(feature = "sqlcipher")]

use std::path::{Path, PathBuf};

use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore, StorageError};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v0.5.0-2");
/// The development password the fixture was created with (a test fixture,
/// protecting demonstration data only).
const PASSWORD: &str = "deckpilot-dev";

struct Copy(PathBuf);

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Copy {
    fn database(&self) -> PathBuf {
        self.0.join("deckpilot.db")
    }
}

fn copy() -> Copy {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-upgrade-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["deckpilot.db", "deckpilot.vault.json"] {
        std::fs::copy(Path::new(FIXTURE).join(file), dir.join(file)).unwrap();
    }
    Copy(dir)
}

fn key(copy: &Copy) -> DatabaseKey {
    Vault::new(copy.database())
        .unlock(&Password::new(PASSWORD))
        .unwrap()
}

fn digest(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn open(copy: &Copy) -> SqliteStore {
    SqliteStore::open(copy.database(), &Encryption::Key(key(copy))).unwrap()
}

fn backups(copy: &Copy) -> Vec<PathBuf> {
    std::fs::read_dir(copy.0.join("backups"))
        .map(|dir| dir.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// A database file never starts with SQLite's plaintext header.
fn is_encrypted(path: &Path) -> bool {
    !digest(path).starts_with(b"SQLite format 3\0")
}

#[test]
fn the_fixture_is_a_schema_5_database() {
    let copy = copy();
    assert!(is_encrypted(&copy.database()));
    // Opening it at all upgrades it, so the version is read from the
    // backup the upgrade makes.
    let store = open(&copy);
    let history = store.migration_history().unwrap();
    let upgrade = history.last().unwrap();
    assert_eq!((upgrade.from_version, upgrade.to_version), (5, 7));
    assert_eq!(upgrade.result, "ok");
}

#[test]
fn upgrading_keeps_every_kind_of_data() {
    let copy = copy();
    let store = open(&copy);
    assert_eq!(store.schema_version().unwrap(), 7);
    assert_eq!(
        store.schema_version().unwrap(),
        SqliteStore::supported_schema_version()
    );
    store.check_integrity().unwrap();

    let mut profiles: Vec<_> = store
        .list_profiles()
        .unwrap()
        .into_iter()
        .map(|s| store.load_profile(s.id).unwrap().unwrap())
        .collect();
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<_> = profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Empty", "Personal", "Work"]);
    let work = &profiles[2];
    let keys: Vec<_> = work.projects.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["SHOP", "CMR", "CMS"]);
    assert_eq!(
        work.projects[0].resources["repository"].as_str(),
        "shop.repository"
    );
    assert_eq!(work.actions.len(), 9);
    assert_eq!(profiles[1].actions.len(), 3);
    assert_eq!(work.layouts.len(), 3);
    let start = &work.workflows[0];
    assert_eq!(start.name, "Start Work");
    assert_eq!(start.steps.len(), 4);
    // Action types moved to module namespaces by migration 3 stay there.
    assert!(
        work.actions
            .iter()
            .all(|a| a.action_type.as_str().contains('.'))
    );
    // Runtime state: the active profile; the default project.
    assert_eq!(store.active_profile().unwrap(), Some(work.id));
    assert_eq!(work.default_project, Some(work.projects[0].id));
    // Machine resources and settings.
    let machine = store.load_machine_config().unwrap().unwrap();
    assert_eq!(machine.bindings.len(), 6);
    assert!(
        machine
            .bindings
            .iter()
            .any(|b| b.value == "https://example.com/docs/shop")
    );
    assert!(
        store
            .setting("update.preferences")
            .unwrap()
            .is_some_and(|p| p.contains("installWhenIdle"))
    );
    // Tables added after 0.5.0 start empty.
    assert_eq!(store.credentials().unwrap(), []);
    let history = store
        .executions(deckpilot_storage::executions::ExecutionQuery {
            profile: work.id,
            workflow: None,
            limit: 100,
        })
        .unwrap();
    assert_eq!(history.len(), 0);
}

#[test]
fn the_upgrade_is_preceded_by_an_encrypted_restorable_backup() {
    let copy = copy();
    let store = open(&copy);
    let history = store.migration_history().unwrap();
    let backup = PathBuf::from(history.last().unwrap().backup.clone().unwrap());
    assert_eq!(backups(&copy), std::slice::from_ref(&backup));
    // The backup is the database as it was (an SQLCipher copy with the
    // same key): encrypted, never plaintext; restoring it is shown below.
    assert!(is_encrypted(&backup));
    drop(store);
    // Reopening does not upgrade again or make another backup.
    let store = open(&copy);
    assert_eq!(backups(&copy).len(), 1);
    assert_eq!(store.migration_history().unwrap().len(), history.len());
    // Restoring is copying the backup back: it opens (and upgrades) again.
    drop(store);
    std::fs::copy(&backup, copy.database()).unwrap();
    for wal in ["deckpilot.db-wal", "deckpilot.db-shm"] {
        let _ = std::fs::remove_file(copy.0.join(wal));
    }
    let restored = open(&copy);
    assert_eq!(restored.list_profiles().unwrap().len(), 3);
}

#[test]
fn data_added_after_the_upgrade_is_kept() {
    let copy = copy();
    let store = open(&copy);
    let work_id = store
        .list_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.name == "Work")
        .unwrap()
        .id;
    let mut work = store.load_profile(work_id).unwrap().unwrap();
    let mut workflow = deckpilot_core::Workflow::new("Graph");
    workflow.graph = Some(
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "nodes": [
                {"id": "start", "type": "trigger"},
                {"id": "a", "type": "action", "action": work.actions[0].id},
            ],
            "edges": [{"from": "start", "to": "a"}],
        }))
        .unwrap(),
    );
    let graph_id = workflow.id;
    work.workflows.push(workflow);
    store.save_profile(&work).unwrap();
    let record = deckpilot_core::ExecutionRecord::start(
        deckpilot_core::ExecutionId::new(),
        work.workflow(graph_id).unwrap(),
        &work,
        deckpilot_core::ExecutionTrigger::Device,
        1,
    );
    store.save_execution(&record).unwrap();
    let credential = deckpilot_core::CredentialId::new("api.token").unwrap();
    store
        .set_credential(
            &credential,
            "test",
            Some(&deckpilot_core::Secret::new("s3cret")),
        )
        .unwrap();
    drop(store);

    let store = open(&copy);
    let work = store.load_profile(work_id).unwrap().unwrap();
    assert!(work.workflow(graph_id).unwrap().graph.is_some());
    assert!(store.execution(record.summary.id).unwrap().is_some());
    assert_eq!(store.credentials().unwrap()[0].id, credential);
    // The secret is in the encrypted file only.
    assert!(!digest(&copy.database()).windows(6).any(|w| w == b"s3cret"));
}

#[test]
fn a_wrong_key_changes_nothing() {
    let copy = copy();
    let original = digest(&copy.database());
    let wrong = DatabaseKey::random().unwrap();
    let refused = SqliteStore::open(copy.database(), &Encryption::Key(wrong));
    assert!(
        matches!(refused, Err(StorageError::InvalidKey)),
        "{refused:?}"
    );
    assert!(
        Vault::new(copy.database())
            .unlock(&Password::new("wrong password"))
            .is_err()
    );
    // No reinitialization, no backup, no upgrade.
    assert!(digest(&copy.database()) == original, "the file changed");
    assert_eq!(backups(&copy), Vec::<PathBuf>::new());
    assert_eq!(open(&copy).list_profiles().unwrap().len(), 3);
}

#[test]
fn a_newer_schema_is_refused_and_left_alone() {
    let copy = copy();
    let store = open(&copy);
    store
        .connection_for_tests()
        .pragma_update(None, "user_version", 8)
        .unwrap();
    drop(store);
    let before = digest(&copy.database());
    let backups_before = backups(&copy);
    let refused = SqliteStore::open(copy.database(), &Encryption::Key(key(&copy)));
    assert!(
        matches!(
            refused,
            Err(StorageError::UnsupportedSchema {
                found: 8,
                supported: 7
            })
        ),
        "{refused:?}"
    );
    assert!(digest(&copy.database()) == before, "the file changed");
    assert_eq!(backups(&copy), backups_before);
}
