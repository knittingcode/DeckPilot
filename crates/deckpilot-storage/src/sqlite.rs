use std::fmt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use deckpilot_core::{MachineConfig, Profile, ProfileId, ProjectId};

use crate::{ConfigStore, DOCUMENT_VERSION, LaunchInfo, ProfileSummary, Result, StorageError};

/// Ordered schema migrations. Index `n` upgrades from version `n` to `n + 1`.
const MIGRATIONS: &[&str] = &[
    r"
    CREATE TABLE profiles (
        id               TEXT PRIMARY KEY NOT NULL,
        name             TEXT NOT NULL,
        document_version INTEGER NOT NULL,
        document         TEXT NOT NULL,
        updated_at       INTEGER NOT NULL
    );
    CREATE TABLE machine_config (
        id               INTEGER PRIMARY KEY CHECK (id = 1),
        document_version INTEGER NOT NULL,
        document         TEXT NOT NULL
    );
    CREATE TABLE profile_state (
        profile_id     TEXT PRIMARY KEY NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
        active_project TEXT
    );
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    );
",
    r"
    CREATE TABLE app_launches (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        component   TEXT NOT NULL,
        app_version TEXT NOT NULL,
        launched_at INTEGER NOT NULL
    );
    CREATE INDEX app_launches_component ON app_launches (component, id);
",
];

const ACTIVE_PROFILE: &str = "active_profile";
const DEVICE_BRIGHTNESS: &str = "device_brightness";
/// How long a writer waits for another process holding the database lock.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Key used to unlock an encrypted database.
///
/// The key is derived by SQLCipher (PBKDF2); DeckPilot does not implement any
/// cryptography itself. The value is redacted from `Debug` output.
#[derive(Clone)]
pub struct DatabaseKey(#[cfg_attr(not(feature = "sqlcipher"), allow(dead_code))] String);

impl DatabaseKey {
    pub fn new(passphrase: impl Into<String>) -> Self {
        Self(passphrase.into())
    }
}

impl fmt::Debug for DatabaseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DatabaseKey(<redacted>)")
    }
}

/// Encryption mode for opening a database.
#[derive(Debug, Clone)]
pub enum Encryption {
    None,
    /// Requires the `sqlcipher` feature.
    Key(DatabaseKey),
}

/// SQLite-backed [`ConfigStore`].
#[derive(Debug)]
pub struct SqliteStore {
    connection: Connection,
}

impl SqliteStore {
    /// Opens (or creates) the database at `path` and applies migrations.
    pub fn open(path: impl AsRef<Path>, encryption: &Encryption) -> Result<Self> {
        Self::init(Connection::open(path)?, encryption)
    }

    /// Opens a private in-memory database. Useful for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, &Encryption::None)
    }

    fn init(connection: Connection, encryption: &Encryption) -> Result<Self> {
        apply_key(&connection, encryption)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    /// Current schema version of the open database.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    fn migrate(&self) -> Result<()> {
        let supported = u32::try_from(MIGRATIONS.len()).expect("migration count fits in u32");
        let current = self.schema_version()?;
        if current > supported {
            return Err(StorageError::UnsupportedSchema {
                found: current,
                supported,
            });
        }
        for version in current..supported {
            // Schema change and document upgrade commit together or not at all.
            let tx = self.connection.unchecked_transaction()?;
            tx.execute_batch(MIGRATIONS[version as usize])?;
            if version + 1 == 2 {
                upgrade_documents(&tx)?;
            }
            tx.pragma_update(None, "user_version", version + 1)?;
            tx.commit()?;
        }
        Ok(())
    }
}

/// Rewrites stored documents in the current [`DOCUMENT_VERSION`]. Document
/// version 2 only adds optional fields, so older documents deserialize with
/// defaults and are written back unchanged otherwise.
fn upgrade_documents(connection: &Connection) -> Result<()> {
    let profiles: Vec<(String, u32, String)> = connection
        .prepare("SELECT id, document_version, document FROM profiles")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, version, document) in profiles {
        let profile: Profile = decode(version, &document)?;
        connection.execute(
            "UPDATE profiles SET document_version = ?1, document = ?2 WHERE id = ?3",
            params![DOCUMENT_VERSION, encode(&profile)?, id],
        )?;
    }
    let machine: Option<(u32, String)> = connection
        .query_row(
            "SELECT document_version, document FROM machine_config WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((version, document)) = machine {
        let config: MachineConfig = decode(version, &document)?;
        connection.execute(
            "UPDATE machine_config SET document_version = ?1, document = ?2 WHERE id = 1",
            params![DOCUMENT_VERSION, encode(&config)?],
        )?;
    }
    Ok(())
}

#[cfg(feature = "sqlcipher")]
fn apply_key(connection: &Connection, encryption: &Encryption) -> Result<()> {
    if let Encryption::Key(key) = encryption {
        connection.pragma_update(None, "key", &key.0)?;
        // Reading the schema fails if the key is wrong.
        connection
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|_| StorageError::InvalidKey)?;
    }
    Ok(())
}

#[cfg(not(feature = "sqlcipher"))]
fn apply_key(_connection: &Connection, encryption: &Encryption) -> Result<()> {
    match encryption {
        Encryption::None => Ok(()),
        Encryption::Key(_) => Err(StorageError::EncryptionUnavailable),
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or_default()
}

fn encode<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

/// Oldest document version that can still be read (and upgraded).
const MIN_DOCUMENT_VERSION: u32 = 1;

fn decode<T: DeserializeOwned>(version: u32, document: &str) -> Result<T> {
    if !(MIN_DOCUMENT_VERSION..=DOCUMENT_VERSION).contains(&version) {
        return Err(StorageError::UnsupportedDocument(version));
    }
    Ok(serde_json::from_str(document)?)
}

fn parse_id<T: std::str::FromStr>(value: &str) -> rusqlite::Result<T> {
    value.parse().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("invalid identifier `{value}`").into(),
        )
    })
}

impl ConfigStore for SqliteStore {
    fn list_profiles(&self) -> Result<Vec<ProfileSummary>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, updated_at FROM profiles ORDER BY name COLLATE NOCASE, id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProfileSummary {
                id: parse_id(&row.get::<_, String>(0)?)?,
                name: row.get(1)?,
                updated_at: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn load_profile(&self, id: ProfileId) -> Result<Option<Profile>> {
        let row: Option<(u32, String)> = self
            .connection
            .query_row(
                "SELECT document_version, document FROM profiles WHERE id = ?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(version, document)| decode(version, &document))
            .transpose()
    }

    fn save_profile(&self, profile: &Profile) -> Result<()> {
        self.connection.execute(
            "INSERT INTO profiles (id, name, document_version, document, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET
                name = excluded.name,
                document_version = excluded.document_version,
                document = excluded.document,
                updated_at = excluded.updated_at",
            params![
                profile.id.to_string(),
                profile.name,
                DOCUMENT_VERSION,
                encode(profile)?,
                now()
            ],
        )?;
        Ok(())
    }

    fn delete_profile(&self, id: ProfileId) -> Result<bool> {
        let tx = self.connection.unchecked_transaction()?;
        let deleted = tx.execute("DELETE FROM profiles WHERE id = ?1", [id.to_string()])? > 0;
        tx.execute(
            "DELETE FROM settings WHERE key = ?1 AND value = ?2",
            params![ACTIVE_PROFILE, id.to_string()],
        )?;
        tx.commit()?;
        Ok(deleted)
    }

    fn load_machine_config(&self) -> Result<Option<MachineConfig>> {
        let row: Option<(u32, String)> = self
            .connection
            .query_row(
                "SELECT document_version, document FROM machine_config WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(version, document)| decode(version, &document))
            .transpose()
    }

    fn save_machine_config(&self, config: &MachineConfig) -> Result<()> {
        self.connection.execute(
            "INSERT INTO machine_config (id, document_version, document) VALUES (1, ?1, ?2)
             ON CONFLICT (id) DO UPDATE SET
                document_version = excluded.document_version,
                document = excluded.document",
            params![DOCUMENT_VERSION, encode(config)?],
        )?;
        Ok(())
    }

    fn active_profile(&self) -> Result<Option<ProfileId>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [ACTIVE_PROFILE],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.map(|value| parse_id(&value)).transpose()?)
    }

    fn set_active_profile(&self, id: Option<ProfileId>) -> Result<()> {
        match id {
            Some(id) => {
                let exists: bool = self.connection.query_row(
                    "SELECT EXISTS (SELECT 1 FROM profiles WHERE id = ?1)",
                    [id.to_string()],
                    |row| row.get(0),
                )?;
                if !exists {
                    return Err(StorageError::UnknownProfile);
                }
                self.connection.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    params![ACTIVE_PROFILE, id.to_string()],
                )?;
            }
            None => {
                self.connection
                    .execute("DELETE FROM settings WHERE key = ?1", [ACTIVE_PROFILE])?;
            }
        }
        Ok(())
    }

    fn device_brightness(&self) -> Result<Option<u8>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [DEVICE_BRIGHTNESS],
                |row| row.get(0),
            )
            .optional()?;
        // Ignore malformed values instead of failing; the default applies.
        Ok(value
            .and_then(|value| value.parse::<u8>().ok())
            .map(|value| value.min(100)))
    }

    fn set_device_brightness(&self, percent: u8) -> Result<()> {
        self.connection.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![DEVICE_BRIGHTNESS, percent.min(100).to_string()],
        )?;
        Ok(())
    }

    fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    fn record_launch(&self, component: &str, version: &str) -> Result<LaunchInfo> {
        let tx = self.connection.unchecked_transaction()?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT app_version FROM app_launches WHERE component = ?1 ORDER BY id DESC LIMIT 1",
                [component],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute(
            "INSERT INTO app_launches (component, app_version, launched_at) VALUES (?1, ?2, ?3)",
            params![component, version, now()],
        )?;
        // Keep a short history only.
        tx.execute(
            "DELETE FROM app_launches WHERE component = ?1 AND id NOT IN
               (SELECT id FROM app_launches WHERE component = ?1 ORDER BY id DESC LIMIT 20)",
            [component],
        )?;
        tx.commit()?;
        let first_after_update = previous
            .as_deref()
            .is_some_and(|previous| previous != version);
        Ok(LaunchInfo {
            previous_version: previous,
            current_version: version.to_owned(),
            first_after_update,
        })
    }

    fn active_project(&self, profile: ProfileId) -> Result<Option<ProjectId>> {
        let value: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT active_project FROM profile_state WHERE profile_id = ?1",
                [profile.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.flatten().map(|value| parse_id(&value)).transpose()?)
    }

    fn set_active_project(&self, profile: ProfileId, project: Option<ProjectId>) -> Result<()> {
        let result = self.connection.execute(
            "INSERT INTO profile_state (profile_id, active_project) VALUES (?1, ?2)
             ON CONFLICT (profile_id) DO UPDATE SET active_project = excluded.active_project",
            params![profile.to_string(), project.map(|id| id.to_string())],
        );
        match result {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(StorageError::UnknownProfile)
            }
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::{
        ActionInstanceId, ActionParameters, ActionRegistry, MachineResourceBinding, Os,
        ParameterValue, Project, ProjectKey, ResourceKey, ResourceKind, builtin,
    };

    fn sample_profile(name: &str) -> Profile {
        let registry = ActionRegistry::with_builtins();
        let mut profile = Profile::new(name);
        profile
            .add_project(Project::new(ProjectKey::new("SHOP").unwrap(), "Shop"))
            .unwrap();
        profile.actions.push(
            registry
                .create_instance(
                    "Hello",
                    builtin::log_message_id(),
                    &ActionParameters::new().with("message", ParameterValue::Text("hi".into())),
                )
                .unwrap(),
        );
        profile
    }

    #[test]
    fn migrations_are_applied_once() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
        store.migrate().unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    }

    #[test]
    fn refuses_newer_schema() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .connection
            .pragma_update(None, "user_version", 99)
            .unwrap();
        assert!(matches!(
            store.migrate(),
            Err(StorageError::UnsupportedSchema { found: 99, .. })
        ));
    }

    #[test]
    fn profiles_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut profile = sample_profile("Work");
        store.save_profile(&profile).unwrap();
        assert_eq!(
            store.load_profile(profile.id).unwrap(),
            Some(profile.clone())
        );

        profile.name = "Office".into();
        store.save_profile(&profile).unwrap();
        store.save_profile(&sample_profile("Home")).unwrap();
        let names: Vec<_> = store
            .list_profiles()
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["Home", "Office"]);

        assert!(store.delete_profile(profile.id).unwrap());
        assert!(!store.delete_profile(profile.id).unwrap());
        assert_eq!(store.load_profile(profile.id).unwrap(), None);
    }

    #[test]
    fn machine_config_is_stored_separately() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.load_machine_config().unwrap(), None);
        let mut config = MachineConfig::new(Os::current());
        config.set_binding(MachineResourceBinding {
            key: ResourceKey::new("shop.repo").unwrap(),
            kind: ResourceKind::Directory,
            value: "/src/shop".into(),
        });
        store.save_machine_config(&config).unwrap();
        assert_eq!(store.load_machine_config().unwrap(), Some(config));

        let profile = sample_profile("Work");
        store.save_profile(&profile).unwrap();
        let stored: String = store
            .connection
            .query_row("SELECT document FROM profiles", [], |row| row.get(0))
            .unwrap();
        assert!(!stored.contains("/src/shop"));
    }

    #[test]
    fn runtime_state_is_scoped_per_profile_and_cleaned_up() {
        let store = SqliteStore::open_in_memory().unwrap();
        let work = sample_profile("Work");
        let home = sample_profile("Home");
        store.save_profile(&work).unwrap();
        store.save_profile(&home).unwrap();

        store.set_active_profile(Some(work.id)).unwrap();
        store
            .set_active_project(work.id, Some(work.projects[0].id))
            .unwrap();
        assert_eq!(store.active_profile().unwrap(), Some(work.id));
        assert_eq!(
            store.active_project(work.id).unwrap(),
            Some(work.projects[0].id)
        );
        assert_eq!(store.active_project(home.id).unwrap(), None);

        assert!(matches!(
            store.set_active_profile(Some(ProfileId::new())),
            Err(StorageError::UnknownProfile)
        ));
        assert!(matches!(
            store.set_active_project(ProfileId::new(), None),
            Err(StorageError::UnknownProfile)
        ));

        store.delete_profile(work.id).unwrap();
        assert_eq!(store.active_profile().unwrap(), None);
        assert_eq!(store.active_project(work.id).unwrap(), None);
    }

    #[test]
    fn persists_to_disk() {
        let dir = std::env::temp_dir().join(format!("deckpilot-test-{}", ProfileId::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.db");
        let profile = sample_profile("Work");
        {
            let store = SqliteStore::open(&path, &Encryption::None).unwrap();
            store.save_profile(&profile).unwrap();
        }
        let store = SqliteStore::open(&path, &Encryption::None).unwrap();
        assert_eq!(store.load_profile(profile.id).unwrap(), Some(profile));
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn brightness_is_clamped_and_persisted() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.device_brightness().unwrap(), None);
        store.set_device_brightness(40).unwrap();
        assert_eq!(store.device_brightness().unwrap(), Some(40));
        store.set_device_brightness(250).unwrap();
        assert_eq!(store.device_brightness().unwrap(), Some(100));
    }

    #[test]
    fn concurrent_connections_share_the_database() {
        let dir = std::env::temp_dir().join(format!("deckpilot-test-{}", ProfileId::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shared.db");
        let first = SqliteStore::open(&path, &Encryption::None).unwrap();
        let second = SqliteStore::open(&path, &Encryption::None).unwrap();
        let profile = sample_profile("Work");
        first.save_profile(&profile).unwrap();
        second.set_active_profile(Some(profile.id)).unwrap();
        assert_eq!(first.active_profile().unwrap(), Some(profile.id));
        drop((first, second));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migrates_a_version_one_database_without_losing_data() {
        let dir = std::env::temp_dir().join(format!("deckpilot-test-{}", ProfileId::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v1.db");
        let profile_id = ProfileId::new();
        let project_id = ProjectId::new();
        {
            // A database as written by the first release: schema 1, documents
            // version 1, without the fields added later.
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(MIGRATIONS[0]).unwrap();
            connection.pragma_update(None, "user_version", 1).unwrap();
            let document = serde_json::json!({
                "id": profile_id, "name": "Legacy", "description": "",
                "actions": [{
                    "id": ActionInstanceId::new(), "name": "Hello",
                    "actionType": "deckpilot.core.log",
                    "parameters": {"message": {"type": "text", "value": "hi"}}
                }],
                "workflows": [], "layouts": [],
                "projects": [{"id": project_id, "key": "SHOP", "name": "Shop", "resources": {}}]
            });
            connection
                .execute(
                    "INSERT INTO profiles VALUES (?1, 'Legacy', 1, ?2, 0)",
                    params![profile_id.to_string(), document.to_string()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO machine_config VALUES (1, 1, ?1)",
                    [r#"{"os":"linux","bindings":[]}"#],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO settings VALUES ('active_profile', ?1)",
                    [profile_id.to_string()],
                )
                .unwrap();
        }

        let store = SqliteStore::open(&path, &Encryption::None).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
        let profile = store.load_profile(profile_id).unwrap().unwrap();
        assert_eq!(profile.name, "Legacy");
        assert_eq!(profile.actions[0].label, None);
        assert_eq!(profile.projects[0].id, project_id);
        assert_eq!(profile.default_project, None);
        assert_eq!(store.active_profile().unwrap(), Some(profile_id));
        assert!(store.load_machine_config().unwrap().is_some());
        let versions: Vec<u32> = store
            .connection
            .prepare("SELECT document_version FROM profiles UNION ALL SELECT document_version FROM machine_config")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(versions.iter().all(|v| *v == DOCUMENT_VERSION));
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn records_launches_and_detects_updates() {
        let store = SqliteStore::open_in_memory().unwrap();
        let first = store.record_launch("desktop", "0.2.0").unwrap();
        assert_eq!(first.previous_version, None);
        assert!(!first.first_after_update);
        assert!(
            !store
                .record_launch("desktop", "0.2.0")
                .unwrap()
                .first_after_update
        );
        let updated = store.record_launch("desktop", "0.3.0").unwrap();
        assert!(updated.first_after_update);
        assert_eq!(updated.previous_version.as_deref(), Some("0.2.0"));
        // Components are tracked separately.
        assert!(
            !store
                .record_launch("agent", "0.3.0")
                .unwrap()
                .first_after_update
        );
        for _ in 0..30 {
            store.record_launch("desktop", "0.3.0").unwrap();
        }
        let rows: i64 = store
            .connection
            .query_row(
                "SELECT count(*) FROM app_launches WHERE component = 'desktop'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 20);
    }

    #[test]
    fn settings_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.setting("update.mode").unwrap(), None);
        store.set_setting("update.mode", "notify").unwrap();
        store.set_setting("update.mode", "download").unwrap();
        assert_eq!(
            store.setting("update.mode").unwrap().as_deref(),
            Some("download")
        );
    }

    #[test]
    fn database_key_is_redacted() {
        assert_eq!(
            format!("{:?}", DatabaseKey::new("secret")),
            "DatabaseKey(<redacted>)"
        );
    }

    #[cfg(not(feature = "sqlcipher"))]
    #[test]
    fn encryption_requires_sqlcipher_build() {
        let result = SqliteStore::open(":memory:", &Encryption::Key(DatabaseKey::new("secret")));
        assert!(matches!(result, Err(StorageError::EncryptionUnavailable)));
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn encrypted_database_requires_correct_key() {
        let dir = std::env::temp_dir().join(format!("deckpilot-test-{}", ProfileId::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("encrypted.db");
        let key = Encryption::Key(DatabaseKey::new("correct horse"));
        {
            let store = SqliteStore::open(&path, &key).unwrap();
            store.save_profile(&sample_profile("Work")).unwrap();
        }
        assert!(SqliteStore::open(&path, &key).is_ok());
        let wrong = Encryption::Key(DatabaseKey::new("wrong"));
        assert!(matches!(
            SqliteStore::open(&path, &wrong),
            Err(StorageError::InvalidKey)
        ));
        assert!(SqliteStore::open(&path, &Encryption::None).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
