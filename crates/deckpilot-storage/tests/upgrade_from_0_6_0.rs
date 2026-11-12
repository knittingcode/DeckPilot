//! Upgrading a DeckPilot 0.6.0 database (schema 7) to schema 8.
//!
//! `fixtures/v0.6.0/` was created by the 0.6.0 tools (`deckpilot-dev
//! validation seed`, `main` before schema 8) with the development password,
//! plus a workflow run, a credential (a fixture secret), an active project
//! and launches, so that every schema 7 table has rows. Its machine paths
//! were replaced with neutral ones. Every test works on a copy.

#![cfg(feature = "sqlcipher")]

use std::path::{Path, PathBuf};

use deckpilot_core::variables::{
    ProjectSource, Resolution, ResolutionContext, ScopeKey, VariableDefinition, VariableKey,
    VariableScope, VariableType, VariableValue,
};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore, StorageError};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v0.6.0");
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

    fn key(&self) -> DatabaseKey {
        Vault::new(self.database())
            .unlock(&Password::new(PASSWORD))
            .unwrap()
    }

    fn open(&self) -> Result<SqliteStore, StorageError> {
        SqliteStore::open(self.database(), &Encryption::Key(self.key()))
    }

    fn backups(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.0.join("backups"))
            .map(|dir| dir.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    }
}

fn copy() -> Copy {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-upgrade-060-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["deckpilot.db", "deckpilot.vault.json"] {
        std::fs::copy(Path::new(FIXTURE).join(file), dir.join(file)).unwrap();
    }
    Copy(dir)
}

#[test]
fn upgrading_keeps_every_kind_of_data() {
    let copy = copy();
    let store = copy.open().unwrap();
    assert_eq!(store.schema_version().unwrap(), 8);
    let upgrade = store.migration_history().unwrap().pop().unwrap();
    assert_eq!((upgrade.from_version, upgrade.to_version), (7, 8));
    assert_eq!(upgrade.result, "ok");

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
    assert_eq!(work.default_project, Some(work.projects[0].id));
    assert_eq!(
        store.active_project(work.id).unwrap(),
        Some(work.projects[1].id)
    );
    assert_eq!(store.active_profile().unwrap(), Some(work.id));
    assert!(work.workflows.iter().any(|w| w.graph.is_some()));
    assert_eq!(work.layouts.len(), 3);
    assert_ne!(work.actions.len(), 0);
    let credentials = store.credentials().unwrap();
    assert_eq!(credentials[0].id.as_str(), "fixture.token");
    assert_eq!(
        store
            .executions(deckpilot_storage::executions::ExecutionQuery {
                profile: work.id,
                workflow: None,
                limit: 10,
            })
            .unwrap()
            .len(),
        1
    );
    assert!(store.load_machine_config().unwrap().is_some());
    // No variables and no provenance were made up for the existing
    // (seeded) data.
    assert_eq!(store.shared_variables().unwrap(), Vec::new());
    for profile in &profiles {
        assert_eq!(store.provenance(profile.id).unwrap(), Vec::new());
    }
}

#[test]
fn shared_variables_work_on_the_upgraded_database_and_survive_a_reopen() {
    let copy = copy();
    let store = copy.open().unwrap();
    let work = store
        .list_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.name == "Work")
        .unwrap();
    let work = store.load_profile(work.id).unwrap().unwrap();
    let key = VariableKey::new("projects.favorite").unwrap();
    store
        .save_shared_variable(&VariableDefinition::new(
            key.clone(),
            VariableType::ProjectId,
            &[VariableScope::Profile],
        ))
        .unwrap();
    store
        .set_shared_variable_value(
            &key,
            &ScopeKey::Profile { profile: work.id },
            &VariableValue::ProjectId(work.projects[2].id),
        )
        .unwrap();
    let history = store.migration_history().unwrap().len();
    drop(store);

    let store = copy.open().unwrap();
    let variables = store.shared_variables_for(work.id).unwrap();
    let context = ResolutionContext::preview(work.id).with_project(None, ProjectSource::Explicit);
    assert!(matches!(
        variables.resolve(&key, &context, &[]),
        Resolution::Value { value: VariableValue::ProjectId(id), .. } if id == work.projects[2].id
    ));
    // Reopening neither upgrades nor backs up again.
    assert_eq!(copy.backups().len(), 1);
    assert_eq!(store.migration_history().unwrap().len(), history);
}

#[test]
fn the_backup_restores_the_schema_7_database() {
    let copy = copy();
    let store = copy.open().unwrap();
    let backup = PathBuf::from(
        store
            .migration_history()
            .unwrap()
            .pop()
            .unwrap()
            .backup
            .unwrap(),
    );
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("deckpilot-schema7-")
    );
    // Something changed after the upgrade is lost by restoring.
    store
        .save_shared_variable(&VariableDefinition::new(
            VariableKey::new("paths.root").unwrap(),
            VariableType::String,
            &[VariableScope::Global],
        ))
        .unwrap();
    drop(store);
    std::fs::copy(&backup, copy.database()).unwrap();
    for side in ["deckpilot.db-wal", "deckpilot.db-shm"] {
        let _ = std::fs::remove_file(copy.0.join(side));
    }
    let raw = rusqlite::Connection::open(copy.database()).unwrap();
    raw.pragma_update(None, "key", format!("x'{}'", copy.key().to_hex().as_str()))
        .unwrap();
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 7);
    drop(raw);
    // It upgrades again and holds the 0.6.0 data, without the variable.
    let store = copy.open().unwrap();
    assert_eq!(store.list_profiles().unwrap().len(), 3);
    assert_eq!(store.shared_variables().unwrap(), Vec::new());
}

#[test]
fn a_schema_8_database_is_refused_by_a_build_that_supports_less() {
    // What 0.6.0 does with a schema 8 database is what this build does with
    // a newer one: refuse it and leave the file as it is.
    let copy = copy();
    let store = copy.open().unwrap();
    store
        .connection_for_tests()
        .pragma_update(None, "user_version", 9)
        .unwrap();
    drop(store);
    let before = std::fs::read(copy.database()).unwrap();
    assert!(matches!(
        copy.open(),
        Err(StorageError::UnsupportedSchema {
            found: 9,
            supported: 8
        })
    ));
    assert!(
        std::fs::read(copy.database()).unwrap() == before,
        "the file changed"
    );
}
