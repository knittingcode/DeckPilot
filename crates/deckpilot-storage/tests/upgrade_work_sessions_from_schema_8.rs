//! Upgrading a schema 8 database (`main` before work sessions) to schema 9.
//!
//! `fixtures/schema8/` is the 0.6.0 fixture upgraded by `main` at schema 8
//! (#97), plus shared variables with a global, a profile and a project
//! value and a provenance row, so that every schema 8 table has rows. It
//! uses the 0.6.0 fixture's vault (development password). The machine path
//! of its 7 → 8 backup was replaced with a neutral one. Every test works on
//! a copy.

#![cfg(feature = "sqlcipher")]

use std::path::{Path, PathBuf};

use deckpilot_core::variables::{
    ProjectSource, Resolution, ResolutionContext, ScopeKey, VariableKey, VariableValue,
};
use deckpilot_core::work_session::VerificationOutcome;
use deckpilot_core::{VerificationId, WorkSessionId};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::work_sessions::{
    NewWorkSession, SessionStatus, Transition, VerificationResult,
};
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore, StorageError};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/schema8");
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

    /// The schema version, read without DeckPilot's open (which upgrades).
    fn raw_version(&self) -> u32 {
        let raw = rusqlite::Connection::open(self.database()).unwrap();
        raw.pragma_update(None, "key", format!("x'{}'", self.key().to_hex().as_str()))
            .unwrap();
        raw.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }
}

fn copy() -> Copy {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "deckpilot-upgrade-schema8-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["deckpilot.db", "deckpilot.vault.json"] {
        std::fs::copy(Path::new(FIXTURE).join(file), dir.join(file)).unwrap();
    }
    Copy(dir)
}

fn work(store: &SqliteStore) -> deckpilot_core::Profile {
    let summary = store
        .list_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.name == "Work")
        .unwrap();
    store.load_profile(summary.id).unwrap().unwrap()
}

#[test]
fn the_fixture_is_a_schema_8_database() {
    assert_eq!(copy().raw_version(), 8);
}

#[test]
fn upgrading_keeps_variables_and_provenance_and_adds_no_session() {
    let copy = copy();
    let store = copy.open().unwrap();
    assert_eq!(store.schema_version().unwrap(), 9);
    let history = store.migration_history().unwrap();
    let upgrade = history.last().unwrap();
    assert_eq!((upgrade.from_version, upgrade.to_version), (8, 9));
    assert_eq!(upgrade.result, "ok");
    // The earlier upgrades are kept.
    assert!(
        history
            .iter()
            .any(|h| (h.from_version, h.to_version) == (7, 8))
    );

    let work = work(&store);
    assert_eq!(store.list_profiles().unwrap().len(), 3);
    assert_eq!(work.projects.len(), 3);
    let keys: Vec<_> = store
        .shared_variables()
        .unwrap()
        .into_iter()
        .map(|v| v.definition.key.to_string())
        .collect();
    assert_eq!(keys, ["paths.root", "projects.favorite"]);
    let root = VariableKey::new("paths.root").unwrap();
    assert_eq!(store.shared_variable_values(&root).unwrap().len(), 3);
    let variables = store.shared_variables_for(work.id).unwrap();
    let context = ResolutionContext::preview(work.id)
        .with_project(Some(work.projects[1].id), ProjectSource::Explicit);
    assert!(matches!(
        variables.resolve(&root, &context, &[]),
        Resolution::Value { value: VariableValue::String(text), .. } if text == "/srv/work/cmr"
    ));
    assert_eq!(store.provenance(work.id).unwrap().len(), 1);
    assert_eq!(store.credentials().unwrap().len(), 1);

    assert_eq!(store.active_work_session().unwrap(), None);
    assert_eq!(
        store.work_session_settings(work.id).unwrap(),
        deckpilot_storage::work_sessions::WorkSessionSettings::defaults(work.id)
    );
    assert_eq!(copy.backups().len(), 1);
}

#[test]
fn a_work_session_on_the_upgraded_database_survives_a_reopen() {
    let copy = copy();
    let store = copy.open().unwrap();
    let work = work(&store);
    let id = WorkSessionId::new();
    let started = store
        .begin_work_session(&NewWorkSession {
            id,
            profile: work.id,
            project: Some(work.projects[0].id),
            start_workflow: None,
            at_ms: 1_000,
        })
        .unwrap();
    assert!(matches!(started, Transition::Applied(_)));
    store.work_session_start_succeeded(id, 2_000).unwrap();
    let v = VerificationId::new();
    store
        .begin_work_session_verification(id, SessionStatus::Working, v, None, 60_000)
        .unwrap();
    store
        .record_work_session_verification(
            id,
            v,
            &VerificationResult::Outcome(VerificationOutcome {
                can_finish: true,
                details: Some("all green".into()),
            }),
            "boot",
            61_000,
        )
        .unwrap();
    drop(store);

    // Reopening neither upgrades nor backs up again, and keeps the session.
    let store = copy.open().unwrap();
    assert_eq!(copy.backups().len(), 1);
    let active = store.active_work_session().unwrap().unwrap();
    assert_eq!(active.id, id);
    assert_eq!(active.status, SessionStatus::ReadyToFinish);
    assert_eq!(active.verification_details.as_deref(), Some("all green"));
    assert_eq!(store.work_session_elapsed_ms(id, 62_000).unwrap(), 60_000);
    assert!(matches!(
        store.finish_work_session(id, v, "boot", 62_000).unwrap(),
        Transition::Applied(_)
    ));
    // The profile with the active session cannot be deleted.
    assert!(matches!(
        store.delete_profile(work.id),
        Err(StorageError::WorkSessionActive)
    ));
}

#[test]
fn the_backup_restores_the_schema_8_database() {
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
            .starts_with("deckpilot-schema8-")
    );
    // A session started after the upgrade is lost by restoring.
    let work = work(&store);
    store
        .begin_work_session(&NewWorkSession {
            id: WorkSessionId::new(),
            profile: work.id,
            project: None,
            start_workflow: None,
            at_ms: 1,
        })
        .unwrap();
    drop(store);
    std::fs::copy(&backup, copy.database()).unwrap();
    for side in ["deckpilot.db-wal", "deckpilot.db-shm"] {
        let _ = std::fs::remove_file(copy.0.join(side));
    }
    assert_eq!(copy.raw_version(), 8);
    // It upgrades again and holds the schema 8 data, without the session.
    let store = copy.open().unwrap();
    assert_eq!(store.schema_version().unwrap(), 9);
    assert_eq!(store.shared_variables().unwrap().len(), 2);
    assert_eq!(store.active_work_session().unwrap(), None);
    let value = store
        .shared_variable_values(&VariableKey::new("projects.favorite").unwrap())
        .unwrap();
    assert!(matches!(value[0].scope, ScopeKey::Profile { profile } if profile == work.id));
}

#[test]
fn a_newer_schema_is_refused_without_changes() {
    let copy = copy();
    let store = copy.open().unwrap();
    store
        .connection_for_tests()
        .pragma_update(None, "user_version", 10)
        .unwrap();
    drop(store);
    let before = std::fs::read(copy.database()).unwrap();
    let backups = copy.backups();
    assert!(matches!(
        copy.open(),
        Err(StorageError::UnsupportedSchema {
            found: 10,
            supported: 9
        })
    ));
    assert!(
        std::fs::read(copy.database()).unwrap() == before,
        "the file changed"
    );
    assert_eq!(copy.backups(), backups);
}
