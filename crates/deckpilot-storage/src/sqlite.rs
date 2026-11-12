use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use deckpilot_core::{
    ExecutionId, ExecutionRecord, MachineConfig, ModuleId, ModuleMigration, ModuleSettings,
    ModuleStates, Profile, ProfileId, Project, ProjectId, ProjectKey,
};

use crate::executions::{ExecutionQuery, Retention};
use crate::{
    ConfigStore, DOCUMENT_VERSION, LaunchInfo, MigrationNote, ProfileSummary, Result, StorageError,
};

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
    crate::legacy::SCHEMA,
    // Migration 4 only rewrites documents.
    "",
    r"
    CREATE TABLE migration_history (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        from_version INTEGER NOT NULL,
        to_version   INTEGER NOT NULL,
        backup       TEXT,
        result       TEXT NOT NULL,
        applied_at   INTEGER NOT NULL
    );
    CREATE TABLE module_schemas (
        module_id  TEXT PRIMARY KEY NOT NULL,
        version    INTEGER NOT NULL CHECK (version >= 0),
        updated_at INTEGER NOT NULL
    );
",
    crate::executions::SCHEMA,
    crate::credentials::SCHEMA,
    // Shared variables and provenance (with `provenance::SCHEMA` below).
    // Additive: no document is rewritten and no provenance is invented for
    // existing data.
    crate::variables::SCHEMA,
];

/// Runs `step` again while SQLite reports the database busy, for up to
/// [`BUSY_TIMEOUT`].
fn retry_busy(mut step: impl FnMut() -> rusqlite::Result<()>) -> Result<()> {
    let deadline = std::time::Instant::now() + BUSY_TIMEOUT;
    loop {
        match step() {
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::DatabaseBusy
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            other => return Ok(other?),
        }
    }
}

/// Backups kept per database; older ones are removed after a new one is
/// verified.
const KEPT_BACKUPS: usize = 5;

const ACTIVE_PROFILE: &str = "active_profile";
const DEVICE_BRIGHTNESS: &str = "device_brightness";
/// How long a writer waits for another process holding the database lock.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Raw 256-bit key of an encrypted database, derived from the user's
/// password by [`crate::vault`]. SQLCipher receives it as a raw key, so its
/// own passphrase derivation is skipped.
pub type DatabaseKey = crate::crypto::SecretKey;

/// Encryption mode for opening a database.
#[derive(Debug, Clone)]
pub enum Encryption {
    /// Plain SQLite. Only for tests and for reading legacy databases.
    None,
    /// Requires the `sqlcipher` feature (enabled by default).
    Key(DatabaseKey),
}

/// SQLite-backed [`ConfigStore`].
#[derive(Debug)]
pub struct SqliteStore {
    connection: Connection,
    /// File and key, for encrypted backups before migrations.
    path: Option<PathBuf>,
    encryption: Encryption,
}

/// One schema upgrade, as recorded in `migration_history`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationRecord {
    pub from_version: u32,
    pub to_version: u32,
    pub backup: Option<String>,
    pub result: String,
    pub applied_at: i64,
}

/// Changes applied together in one transaction, e.g. by an import.
#[derive(Debug, Default)]
pub struct ConfigChanges {
    pub delete_profiles: Vec<ProfileId>,
    pub save_profiles: Vec<Profile>,
    pub module_states: Vec<(ModuleId, ModuleSettings)>,
    /// Application settings (`key`, `value`) written with the rest.
    pub settings: Vec<(String, String)>,
}

impl SqliteStore {
    /// Opens (or creates) the database at `path` and applies migrations.
    ///
    /// Before an existing database is upgraded, its integrity is checked and
    /// an encrypted backup is written and verified next to it (in
    /// `backups/`). All migrations run in one transaction; the result is
    /// verified and recorded in `migration_history`.
    pub fn open(path: impl AsRef<Path>, encryption: &Encryption) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let connection = crate::engine::open(&path)?;
        Self::init(connection, Some(path), encryption)
    }

    /// Opens a private in-memory database. Useful for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(crate::engine::open_in_memory()?, None, &Encryption::None)
    }

    fn init(
        connection: Connection,
        path: Option<PathBuf>,
        encryption: &Encryption,
    ) -> Result<Self> {
        apply_key(&connection, encryption)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        // A database written by a newer DeckPilot is refused before anything
        // could write to it: switching the journal mode below rewrites the
        // file header. `migrate` checks again inside its transaction.
        let found: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let supported = Self::supported_schema_version();
        if found > supported {
            return Err(StorageError::UnsupportedSchema { found, supported });
        }
        // Changing the journal mode needs a lock that SQLite may refuse at
        // once (without the busy handler) while another connection — the
        // agent or the app — writes. The mode persists in the file, so it is
        // only changed when needed, and a busy answer is retried.
        retry_busy(|| {
            let mode: String =
                connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
            if !mode.eq_ignore_ascii_case("wal") {
                connection.pragma_update(None, "journal_mode", "WAL")?;
            }
            Ok(())
        })?;
        let store = Self {
            connection,
            path,
            encryption: encryption.clone(),
        };
        store.migrate()?;
        Ok(store)
    }

    /// A transaction that takes the write lock at once (`IMMEDIATE`). The
    /// agent and the configuration app write to the same database; a
    /// deferred transaction that reads before it writes cannot wait for the
    /// other's lock and fails with "database is locked" instead.
    fn write_transaction(&self) -> Result<rusqlite::Transaction<'_>> {
        Ok(rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?)
    }

    /// The raw connection, for tests that need to hold locks or inspect the
    /// file. Not part of the API.
    #[doc(hidden)]
    pub fn connection_for_tests(&self) -> &Connection {
        &self.connection
    }

    /// Current schema version of the open database.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    /// Schema version this build creates and understands.
    ///
    /// # Panics
    ///
    /// Never: there are far fewer migrations than `u32::MAX`.
    pub fn supported_schema_version() -> u32 {
        u32::try_from(MIGRATIONS.len()).expect("migration count fits in u32")
    }

    /// `PRAGMA quick_check` must report `ok`.
    pub fn check_integrity(&self) -> Result<()> {
        check_integrity(&self.connection)
    }

    fn migrate(&self) -> Result<()> {
        let supported = Self::supported_schema_version();
        let current = self.schema_version()?;
        if current > supported {
            // Never touch a database written by a newer DeckPilot.
            return Err(StorageError::UnsupportedSchema {
                found: current,
                supported,
            });
        }
        if current == supported {
            return self.verify_unverified_upgrade();
        }
        let backup = if current > 0 {
            self.check_integrity()?;
            self.backup_before_migration(current)?
        } else {
            None
        };
        let backup_name = backup.as_ref().map(|path| path.display().to_string());
        // IMMEDIATE takes the write lock now, so a second process that opened
        // the same database concurrently waits and then sees the new version.
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let current: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current > supported {
            return Err(StorageError::UnsupportedSchema {
                found: current,
                supported,
            });
        }
        for version in current..supported {
            tx.execute_batch(MIGRATIONS[version as usize])?;
            match version + 1 {
                2 => upgrade_documents(&tx)?,
                3 => crate::legacy::migrate(&tx)?,
                4 => crate::project_actions::migrate(&tx)?,
                8 => tx.execute_batch(crate::provenance::SCHEMA)?,
                _ => {}
            }
            failpoint(Failpoint::Migration(version + 1))?;
        }
        tx.pragma_update(None, "user_version", supported)?;
        // Recorded as unverified until the committed result has passed the
        // integrity check below; a database whose check failed, or was
        // never run because the process stopped, is checked again on every
        // start until it passes (`verify_unverified_upgrade`).
        let upgrade = if current < supported {
            tx.execute(
                "INSERT INTO migration_history (from_version, to_version, backup, result, applied_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![current, supported, backup_name, UNVERIFIED, now()],
            )?;
            Some(tx.last_insert_rowid())
        } else {
            None
        };
        failpoint(Failpoint::Commit)?;
        tx.commit()?;
        match upgrade {
            Some(id) => self.verify_upgrade(id, backup_name),
            // Another process upgraded meanwhile and verifies its upgrade.
            None => Ok(()),
        }
    }

    /// Checks the integrity of the database after the upgrade recorded as
    /// row `id`, and records the result as `ok` only if it passes.
    ///
    /// A failure is reported with the verified backup, which still holds
    /// the previous consistent state; the database is left as it is (it is
    /// already upgraded) and stays unverified, so it is never used until a
    /// later check passes or the user restores the backup.
    fn verify_upgrade(&self, id: i64, backup: Option<String>) -> Result<()> {
        let checked = failpoint(Failpoint::Verify).and_then(|()| self.check_integrity());
        if let Err(error) = checked {
            return Err(StorageError::MigrationFailed {
                reason: error.to_string(),
                backup,
            });
        }
        retry_busy(|| {
            self.connection
                .execute(
                    "UPDATE migration_history SET result = 'ok' WHERE id = ?1 AND result = ?2",
                    params![id, UNVERIFIED],
                )
                .map(drop)
        })
    }

    /// Repeats the verification of an upgrade that was committed but not
    /// verified (the check failed, or the process stopped before it).
    fn verify_unverified_upgrade(&self) -> Result<()> {
        let pending: Option<(i64, Option<String>)> = self
            .connection
            .query_row(
                "SELECT id, backup FROM migration_history WHERE result = ?1
                 ORDER BY id DESC LIMIT 1",
                [UNVERIFIED],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match pending {
            Some((id, backup)) => self.verify_upgrade(id, backup),
            None => Ok(()),
        }
    }

    /// Writes an encrypted backup of the current state before an upgrade
    /// from `version`. Plain (test) databases and in-memory databases are not
    /// backed up, so no unencrypted copy is ever written.
    fn backup_before_migration(&self, version: u32) -> Result<Option<PathBuf>> {
        let (Some(path), Encryption::Key(key)) = (&self.path, &self.encryption) else {
            return Ok(None);
        };
        let dir = backup_dir(path);
        std::fs::create_dir_all(&dir).map_err(StorageError::Io)?;
        // Copies left by an interrupted earlier attempt were never verified.
        remove_partial_backups(&dir);
        let target = dir.join(format!("deckpilot-schema{version}-{}.db", now_millis()));
        // Written under a temporary name and renamed once verified, so that
        // an incomplete copy is never taken for a backup (or counted towards
        // the kept ones), even if the process is killed while writing it.
        let partial = partial_backup(&target);
        let written = export_database(&self.connection, &partial, Some(key))
            .and_then(|()| failpoint(Failpoint::Backup))
            .and_then(|()| verify_copy(&partial, &Encryption::Key(key.clone()), version))
            .and_then(|()| std::fs::rename(&partial, &target).map_err(StorageError::Io));
        if let Err(error) = written {
            remove_partial_backups(&dir);
            return Err(error);
        }
        prune_backups(&dir);
        Ok(Some(target))
    }

    /// Writes an encrypted, verified copy of the database to `target`.
    pub fn backup_to(&self, target: &Path) -> Result<()> {
        let Encryption::Key(key) = &self.encryption else {
            return Err(StorageError::EncryptionUnavailable);
        };
        let version = self.schema_version()?;
        export_database(&self.connection, target, Some(key))?;
        verify_copy(target, &self.encryption, version)
    }

    /// Recorded schema upgrades, oldest first.
    pub fn migration_history(&self) -> Result<Vec<MigrationRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT from_version, to_version, backup, result, applied_at
             FROM migration_history ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(MigrationRecord {
                from_version: row.get(0)?,
                to_version: row.get(1)?,
                backup: row.get(2)?,
                result: row.get(3)?,
                applied_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Applies the declared schema migrations of feature modules, each module
    /// in its own transaction. Versions are tracked in `module_schemas`.
    /// Migrations are SQL declared by the module; no module code runs.
    pub fn apply_module_schemas(&self, modules: &[(ModuleId, Vec<ModuleMigration>)]) -> Result<()> {
        for (module, migrations) in modules {
            let tx = rusqlite::Transaction::new_unchecked(
                &self.connection,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            let current: u32 = tx
                .query_row(
                    "SELECT version FROM module_schemas WHERE module_id = ?1",
                    [module.as_str()],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(0);
            let supported = u32::try_from(migrations.len()).unwrap_or(u32::MAX);
            if current > supported {
                return Err(StorageError::UnsupportedModuleSchema {
                    module: module.to_string(),
                    found: current,
                    supported,
                });
            }
            for migration in migrations.iter().skip(current as usize) {
                if !migration.sql.trim().is_empty() {
                    tx.execute_batch(&migration.sql)?;
                }
            }
            tx.execute(
                "INSERT INTO module_schemas (module_id, version, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (module_id) DO UPDATE SET version = excluded.version,
                    updated_at = excluded.updated_at",
                params![module.as_str(), supported, now()],
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    /// Schema version of a module's own tables, `0` if none were created.
    pub fn module_schema_version(&self, module: &ModuleId) -> Result<u32> {
        Ok(self
            .connection
            .query_row(
                "SELECT version FROM module_schemas WHERE module_id = ?1",
                [module.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    /// Writes (or updates) the record of a workflow run.
    /// Stored credentials, without their secrets.
    pub fn credentials(&self) -> Result<Vec<deckpilot_core::CredentialInfo>> {
        crate::credentials::list(&self.connection)
    }

    /// Stores or replaces a credential; with `secret: None` only the
    /// description of an existing one changes.
    pub fn set_credential(
        &self,
        id: &deckpilot_core::CredentialId,
        description: &str,
        secret: Option<&deckpilot_core::Secret>,
    ) -> Result<()> {
        if let Some(secret) = secret
            && (secret.expose().is_empty()
                || secret.expose().len() > crate::credentials::MAX_SECRET_BYTES)
        {
            return Err(StorageError::Invalid(format!(
                "a secret must have 1 to {} bytes",
                crate::credentials::MAX_SECRET_BYTES
            )));
        }
        if secret.is_none() && !crate::credentials::exists(&self.connection, id)? {
            return Err(StorageError::Invalid(format!("no credential named `{id}`")));
        }
        crate::credentials::set(&self.connection, id, description, secret)
    }

    pub fn delete_credential(&self, id: &deckpilot_core::CredentialId) -> Result<bool> {
        crate::credentials::delete(&self.connection, id)
    }

    /// Every credential with its secret, for the engine.
    pub fn load_credentials(&self) -> Result<deckpilot_core::MemoryCredentials> {
        crate::credentials::load(&self.connection)
    }

    pub fn save_execution(&self, record: &ExecutionRecord) -> Result<()> {
        crate::executions::save(&self.connection, record)
    }

    pub fn execution(&self, id: ExecutionId) -> Result<Option<ExecutionRecord>> {
        crate::executions::get(&self.connection, id)
    }

    /// Records of a profile's runs, newest first.
    pub fn executions(&self, query: ExecutionQuery) -> Result<Vec<ExecutionRecord>> {
        crate::executions::list(&self.connection, query)
    }

    /// Marks runs left running by an agent that stopped, other than
    /// `except`, as interrupted.
    pub fn interrupt_stale_executions(&self, except: &[ExecutionId], now_ms: u64) -> Result<usize> {
        crate::executions::interrupt_stale(&self.connection, except, now_ms)
    }

    /// Removes the oldest finished records beyond `retention`.
    pub fn prune_executions(&self, retention: Retention) -> Result<()> {
        crate::executions::prune(&self.connection, retention)
    }

    /// Deletes the finished records of a profile, or of one of its workflows.
    pub fn clear_executions(
        &self,
        profile: ProfileId,
        workflow: Option<deckpilot_core::WorkflowId>,
    ) -> Result<usize> {
        crate::executions::clear(&self.connection, profile, workflow)
    }

    /// Applies `changes` in one transaction: everything or nothing.
    pub fn apply_changes(&self, changes: &ConfigChanges) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        for id in &changes.delete_profiles {
            tx.execute("DELETE FROM profiles WHERE id = ?1", [id.to_string()])?;
            tx.execute(
                "DELETE FROM workflow_executions WHERE profile_id = ?1",
                [id.to_string()],
            )?;
            tx.execute(
                "DELETE FROM settings WHERE key = ?1 AND value = ?2",
                params![ACTIVE_PROFILE, id.to_string()],
            )?;
        }
        for profile in &changes.save_profiles {
            write_profile(&tx, profile)?;
        }
        for (id, settings) in &changes.module_states {
            tx.execute(
                "INSERT INTO modules (id, enabled, config) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET enabled = excluded.enabled, config = excluded.config",
                params![id.as_str(), settings.enabled, encode(&settings.config)?],
            )?;
        }
        for (key, value) in &changes.settings {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// Shared variables (schema 8). See [`crate::variables`].
impl SqliteStore {
    /// Every definition this build can read, by key.
    pub fn shared_variables(&self) -> Result<Vec<crate::variables::StoredVariable>> {
        crate::variables::list(&self.connection)
    }

    pub fn shared_variable(
        &self,
        key: &deckpilot_core::variables::VariableKey,
    ) -> Result<Option<crate::variables::StoredVariable>> {
        crate::variables::get(&self.connection, key)
    }

    /// Creates or changes a user variable. Refused for a key a module
    /// declares, for an invalid definition, and when stored values would no
    /// longer fit (they are never dropped silently). Reserved namespaces
    /// (`steps`, `system`, `workflow`) cannot be part of a
    /// [`VariableKey`](deckpilot_core::variables::VariableKey) at all.
    pub fn save_shared_variable(
        &self,
        definition: &deckpilot_core::variables::VariableDefinition,
    ) -> Result<()> {
        self.save_shared_variable_from(definition, crate::variables::VariableOrigin::User)
    }

    /// [`Self::save_shared_variable`] recording where a new definition came
    /// from (the seed, an import).
    pub fn save_shared_variable_from(
        &self,
        definition: &deckpilot_core::variables::VariableDefinition,
        origin: crate::variables::VariableOrigin,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        crate::variables::save(&tx, definition, origin)?;
        tx.commit()?;
        Ok(())
    }

    /// Declares the variables of a module. Keys already held by a user
    /// variable or another module are skipped and returned; nothing of the
    /// holder changes.
    pub fn declare_module_variables(
        &self,
        module: &ModuleId,
        definitions: &[deckpilot_core::variables::VariableDefinition],
    ) -> Result<Vec<crate::variables::DeclarationConflict>> {
        let tx = self.write_transaction()?;
        let conflicts = crate::variables::declare(&tx, module, definitions)?;
        tx.commit()?;
        Ok(conflicts)
    }

    /// Deletes a user variable with all its values. Returns whether it
    /// existed.
    pub fn delete_shared_variable(
        &self,
        key: &deckpilot_core::variables::VariableKey,
    ) -> Result<bool> {
        let tx = self.write_transaction()?;
        let deleted = crate::variables::delete(&tx, key)?;
        tx.commit()?;
        Ok(deleted)
    }

    /// Sets the value of `key` at `scope` (global, a profile or a project
    /// of that profile), replacing the value there.
    pub fn set_shared_variable_value(
        &self,
        key: &deckpilot_core::variables::VariableKey,
        scope: &deckpilot_core::variables::ScopeKey,
        value: &deckpilot_core::variables::VariableValue,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        crate::variables::set_value(&tx, key, scope, value)?;
        tx.commit()?;
        Ok(())
    }

    /// Removes the value at `scope`. Returns whether there was one.
    pub fn clear_shared_variable_value(
        &self,
        key: &deckpilot_core::variables::VariableKey,
        scope: &deckpilot_core::variables::ScopeKey,
    ) -> Result<bool> {
        let tx = self.write_transaction()?;
        let cleared = crate::variables::clear_value(&tx, key, scope)?;
        tx.commit()?;
        Ok(cleared)
    }

    /// Every stored value of `key`, of all profiles.
    pub fn shared_variable_values(
        &self,
        key: &deckpilot_core::variables::VariableKey,
    ) -> Result<Vec<crate::variables::StoredValue>> {
        crate::variables::values_of(&self.connection, key)
    }

    /// Values of a profile and its projects (no global values).
    pub fn profile_variable_values(
        &self,
        profile: ProfileId,
    ) -> Result<Vec<crate::variables::StoredValue>> {
        crate::variables::values_of_profile(&self.connection, profile)
    }

    pub fn global_variable_values(&self) -> Result<Vec<crate::variables::StoredValue>> {
        crate::variables::global_values(&self.connection)
    }

    /// A snapshot of the variables as `profile` sees them, for resolving
    /// `shared.` references.
    pub fn shared_variables_for(
        &self,
        profile: ProfileId,
    ) -> Result<crate::variables::SharedVariables> {
        crate::variables::SharedVariables::load(&self.connection, profile)
    }
}

/// Provenance (schema 8). See [`crate::provenance`].
impl SqliteStore {
    /// Records (or replaces) provenance rows; all or none.
    pub fn record_provenance(&self, rows: &[crate::provenance::Provenance]) -> Result<()> {
        let tx = self.write_transaction()?;
        crate::provenance::record(&tx, rows)?;
        tx.commit()?;
        Ok(())
    }

    /// Provenance rows of a profile.
    pub fn provenance(&self, profile: ProfileId) -> Result<Vec<crate::provenance::Provenance>> {
        crate::provenance::list(&self.connection, profile)
    }

    /// Each item of `profile` with provenance, whether it still exists and
    /// whether it was edited since it was created.
    pub fn provenance_status(
        &self,
        profile: ProfileId,
    ) -> Result<Vec<crate::provenance::ProvenanceStatus>> {
        use crate::provenance::{ProvenanceEntity, ProvenanceStatus, item_snapshot};
        let Some(document) = self.load_profile(profile)? else {
            return Err(StorageError::UnknownProfile);
        };
        let mut statuses = Vec::new();
        for row in crate::provenance::list(&self.connection, profile)? {
            let current = if row.entity == ProvenanceEntity::Variable {
                match deckpilot_core::variables::VariableKey::new(row.entity_id.clone()) {
                    Ok(key) => crate::variables::get(&self.connection, &key)?
                        .map(|stored| crate::provenance::variable_snapshot(&stored.definition))
                        .transpose()?,
                    Err(_) => None,
                }
            } else {
                item_snapshot(&document, row.entity, &row.entity_id)?
            };
            statuses.push(ProvenanceStatus {
                exists: current.is_some(),
                edited: current.is_some_and(|current| current != row.snapshot),
                provenance: row,
            });
        }
        Ok(statuses)
    }
}

/// Safe deletion of actions. See [`crate::references`].
impl SqliteStore {
    /// Loads `profile`, applies `change` and saves the result, all in one
    /// transaction; nothing is written when `change` fails.
    fn change_profile<T>(
        &self,
        profile: ProfileId,
        change: impl FnOnce(&mut Profile) -> Result<T>,
    ) -> Result<T> {
        let tx = self.write_transaction()?;
        let mut document = self
            .load_profile(profile)?
            .ok_or(StorageError::UnknownProfile)?;
        let result = change(&mut document)?;
        write_profile(&tx, &document)?;
        tx.commit()?;
        Ok(result)
    }

    /// Every place in `profile` that uses `action`: key bindings, workflow
    /// steps and nodes, and the triggers of those workflows.
    pub fn action_references(
        &self,
        profile: ProfileId,
        action: deckpilot_core::ActionInstanceId,
    ) -> Result<Vec<crate::references::ActionReference>> {
        let document = self
            .load_profile(profile)?
            .ok_or(StorageError::UnknownProfile)?;
        Ok(crate::references::action_references(&document, action))
    }

    /// Deletes `action` if nothing uses it; otherwise
    /// [`StorageError::ActionInUse`] lists the uses and nothing changes.
    pub fn delete_action(
        &self,
        profile: ProfileId,
        action: deckpilot_core::ActionInstanceId,
    ) -> Result<deckpilot_core::ActionInstance> {
        self.change_profile(profile, |document| {
            crate::references::delete_action(document, action)
        })
    }

    /// Points the selected uses of `from` at `to`, in one transaction.
    pub fn replace_action_references(
        &self,
        profile: ProfileId,
        from: deckpilot_core::ActionInstanceId,
        to: deckpilot_core::ActionInstanceId,
        selection: &crate::references::Selection,
    ) -> Result<crate::references::ReferenceChange> {
        self.change_profile(profile, |document| {
            crate::references::replace_action(document, from, to, selection)
        })
    }

    /// Removes the selected key bindings and steps that use `action`, in
    /// one transaction. Graph nodes are refused.
    pub fn unbind_action(
        &self,
        profile: ProfileId,
        action: deckpilot_core::ActionInstanceId,
        selection: &crate::references::Selection,
    ) -> Result<crate::references::ReferenceChange> {
        self.change_profile(profile, |document| {
            crate::references::unbind_action(document, action, selection)
        })
    }
}

/// `backups/` next to the database file.
pub fn backup_dir(database: &Path) -> PathBuf {
    database
        .parent()
        .map_or_else(|| PathBuf::from("backups"), |dir| dir.join("backups"))
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

/// `migration_history.result` of an upgrade that is committed but has not
/// passed its integrity check yet.
const UNVERIFIED: &str = "unverified";

/// Points of an upgrade where tests make it fail or kill the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failpoint {
    /// The backup is written but not yet verified.
    Backup,
    /// The migration to this version ran, inside the open transaction.
    Migration(u32),
    /// Everything ran; the transaction is about to be committed.
    Commit,
    /// The upgrade is committed; its integrity check fails.
    Verify,
}

#[cfg(test)]
thread_local! {
    /// Makes the upgrade on this thread fail at the given point.
    pub(crate) static FAIL_AT: std::cell::Cell<Option<Failpoint>> =
        const { std::cell::Cell::new(None) };
}

/// Environment variable that makes a test process abort at a failpoint
/// (its `Debug` form, e.g. `Migration(7)`), as if it had been killed.
#[cfg(test)]
pub(crate) const ABORT_AT_ENV: &str = "DECKPILOT_TEST_ABORT_AT";

/// Test builds only: fails or aborts at `point` when asked to.
#[cfg(test)]
fn failpoint(point: Failpoint) -> Result<()> {
    if std::env::var(ABORT_AT_ENV).is_ok_and(|value| value == format!("{point:?}")) {
        std::process::abort();
    }
    if FAIL_AT.with(std::cell::Cell::get) == Some(point) {
        return Err(StorageError::Invalid(format!(
            "injected failure at {point:?}"
        )));
    }
    Ok(())
}

#[cfg(not(test))]
#[allow(clippy::unnecessary_wraps)]
#[inline]
fn failpoint(_point: Failpoint) -> Result<()> {
    Ok(())
}

const PARTIAL_SUFFIX: &str = ".partial";

fn partial_backup(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(PARTIAL_SUFFIX);
    PathBuf::from(name)
}

/// Removes unverified copies (and their side files) from `dir`. Best effort.
fn remove_partial_backups(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        // `….db.partial` and its side files (`….db.partial-journal`).
        if name.starts_with("deckpilot-schema") && name.contains(&format!(".db{PARTIAL_SUFFIX}")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Removes the oldest backups beyond [`KEPT_BACKUPS`]. Best effort.
fn prune_backups(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut backups: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Not `….db.partial`: an unverified copy is not a backup.
            name.starts_with("deckpilot-schema")
                && Path::new(&name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("db"))
        })
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    backups.sort();
    let excess = backups.len().saturating_sub(KEPT_BACKUPS);
    for (_, path) in backups.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

/// Writes a profile and its projects inside the caller's transaction.
fn write_profile(tx: &Connection, profile: &Profile) -> Result<()> {
    // Projects and the default project live in their own tables.
    let mut document = serde_json::to_value(profile)?;
    if let Some(fields) = document.as_object_mut() {
        fields.remove("projects");
        fields.remove("defaultProject");
    }
    tx.execute(
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
            document.to_string(),
            now()
        ],
    )?;
    SqliteStore::save_projects(tx, profile)?;
    crate::provenance::prune(tx, profile)?;
    Ok(())
}

pub(crate) fn check_integrity(connection: &Connection) -> Result<()> {
    let result: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(StorageError::Corrupt(result))
    }
}

/// SQLCipher's raw-key literal for `key` (`x'…'`), in a zeroizing buffer.
pub(crate) fn raw_key_literal(key: &DatabaseKey) -> zeroize::Zeroizing<String> {
    let hex = key.to_hex();
    let mut literal = zeroize::Zeroizing::new(String::with_capacity(hex.len() + 3));
    literal.push_str("x'");
    literal.push_str(&hex);
    literal.push('\'');
    literal
}

/// Copies the database behind `connection` into a new file at `target`,
/// encrypted with `key` (or plain when `None`), with the same schema version.
/// Uses `sqlcipher_export`, which writes straight into the target, so no
/// intermediate copy is created.
pub(crate) fn export_database(
    connection: &Connection,
    target: &Path,
    key: Option<&DatabaseKey>,
) -> Result<()> {
    // Never attempt the export on a connection that cannot do it: refused
    // before the target is touched.
    crate::engine::check_complete(connection)?;
    if target.exists() {
        std::fs::remove_file(target).map_err(StorageError::Io)?;
    }
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let target_name = target.to_string_lossy().into_owned();
    let literal = key.map(raw_key_literal);
    let empty = zeroize::Zeroizing::new(String::new());
    connection.execute(
        "ATTACH DATABASE ?1 AS deckpilot_export KEY ?2",
        params![target_name, literal.as_ref().unwrap_or(&empty).as_str()],
    )?;
    let exported = connection
        .query_row(
            "SELECT sqlcipher_export('deckpilot_export')",
            [],
            |_| Ok(()),
        )
        .and_then(|()| {
            connection.execute_batch(&format!(
                "PRAGMA deckpilot_export.user_version = {version};"
            ))
        });
    let detached = connection.execute_batch("DETACH DATABASE deckpilot_export;");
    exported?;
    detached?;
    Ok(())
}

/// Opens `path` with `encryption` and checks integrity and schema version.
pub(crate) fn verify_copy(path: &Path, encryption: &Encryption, version: u32) -> Result<()> {
    let connection = crate::engine::open(path)?;
    apply_key(&connection, encryption)?;
    check_integrity(&connection)?;
    let found: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if found == version {
        Ok(())
    } else {
        Err(StorageError::Corrupt(format!(
            "copy has schema version {found}, expected {version}"
        )))
    }
}

/// Rewrites stored documents in document version 2, which only adds optional
/// fields, so older documents deserialize with defaults and are written back
/// unchanged otherwise.
fn upgrade_documents(connection: &Connection) -> Result<()> {
    let profiles: Vec<(String, u32, String)> = connection
        .prepare("SELECT id, document_version, document FROM profiles")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, version, document) in profiles {
        let profile: Profile = decode(version, &document)?;
        connection.execute(
            "UPDATE profiles SET document_version = 2, document = ?1 WHERE id = ?2",
            params![encode(&profile)?, id],
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
            "UPDATE machine_config SET document_version = 2, document = ?1 WHERE id = 1",
            params![encode(&config)?],
        )?;
    }
    Ok(())
}

#[cfg(feature = "sqlcipher")]
pub(crate) fn apply_key(connection: &Connection, encryption: &Encryption) -> Result<()> {
    if let Encryption::Key(key) = encryption {
        // A raw key: SQLCipher skips its own passphrase derivation.
        let literal = raw_key_literal(key);
        connection.pragma_update(None, "key", literal.as_str())?;
        // Reading the schema fails if the key is wrong: the first page does
        // not authenticate, which SQLite reports as "not a database". A file
        // whose first page authenticates but whose schema cannot be read
        // (truncated, damaged pages) was opened with the right key and is
        // reported as damaged, not as a wrong password.
        connection
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| match &error {
                rusqlite::Error::SqliteFailure(failure, message)
                    if failure.code == rusqlite::ErrorCode::DatabaseCorrupt =>
                {
                    StorageError::Corrupt(
                        message
                            .clone()
                            .unwrap_or_else(|| "the database file is damaged".into()),
                    )
                }
                _ => StorageError::InvalidKey,
            })?;
    }
    Ok(())
}

#[cfg(not(feature = "sqlcipher"))]
pub(crate) fn apply_key(_connection: &Connection, encryption: &Encryption) -> Result<()> {
    match encryption {
        Encryption::None => Ok(()),
        Encryption::Key(_) => Err(StorageError::EncryptionUnavailable),
    }
}

pub(crate) fn now() -> i64 {
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

pub(crate) fn constraint_violation(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(failure, _)
        if failure.code == rusqlite::ErrorCode::ConstraintViolation)
}

impl SqliteStore {
    fn profile_exists(&self, id: ProfileId) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM profiles WHERE id = ?1)",
            [id.to_string()],
            |row| row.get(0),
        )?)
    }

    fn load_projects(&self, profile: ProfileId) -> Result<Vec<Project>> {
        let mut statement = self.connection.prepare(
            "SELECT id, key, name, resources FROM projects WHERE profile_id = ?1
             ORDER BY position, id",
        )?;
        let rows = statement.query_map([profile.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut projects = Vec::new();
        for row in rows {
            let (id, key, name, resources) = row?;
            projects.push(Project {
                id: parse_id(&id)?,
                key: ProjectKey::new(key).map_err(|error| {
                    StorageError::Document(serde::de::Error::custom(error.to_string()))
                })?,
                name,
                resources: serde_json::from_str(&resources)?,
            });
        }
        Ok(projects)
    }

    /// Writes the projects of `profile` to the `projects` table inside the
    /// caller's transaction. The table, not the document, is the source of
    /// truth; its constraints keep every project in exactly one profile.
    fn save_projects(tx: &Connection, profile: &Profile) -> Result<()> {
        let profile_id = profile.id.to_string();
        let stored: Vec<String> = tx
            .prepare("SELECT id FROM projects WHERE profile_id = ?1")?
            .query_map([&profile_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in stored {
            if !profile.projects.iter().any(|p| p.id.to_string() == id) {
                // Also removes default and active selections of the project.
                tx.execute("DELETE FROM projects WHERE id = ?1", [&id])?;
            }
        }
        // Park existing keys on unique values so that keys can be swapped.
        tx.execute(
            "UPDATE projects SET key = id WHERE profile_id = ?1",
            [&profile_id],
        )?;
        for (position, project) in profile.projects.iter().enumerate() {
            let changed = tx
                .execute(
                    "INSERT INTO projects (id, profile_id, key, name, position, resources)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (id) DO UPDATE SET
                        key = excluded.key,
                        name = excluded.name,
                        position = excluded.position,
                        resources = excluded.resources
                     WHERE projects.profile_id = excluded.profile_id",
                    params![
                        project.id.to_string(),
                        profile_id,
                        project.key.as_str(),
                        project.name,
                        i64::try_from(position).unwrap_or(i64::MAX),
                        encode(&project.resources)?
                    ],
                )
                .map_err(|error| {
                    if constraint_violation(&error) {
                        StorageError::DuplicateProjectKey(project.key.to_string())
                    } else {
                        error.into()
                    }
                })?;
            if changed == 0 {
                return Err(StorageError::ProjectOfAnotherProfile(project.id));
            }
        }
        tx.execute(
            "DELETE FROM default_projects WHERE profile_id = ?1",
            [&profile_id],
        )?;
        if let Some(default) = profile.default_project {
            tx.execute(
                "INSERT INTO default_projects (profile_id, project_id) VALUES (?1, ?2)",
                params![profile_id, default.to_string()],
            )
            .map_err(|error| {
                if constraint_violation(&error) {
                    StorageError::ProjectOfAnotherProfile(default)
                } else {
                    error.into()
                }
            })?;
        }
        Ok(())
    }
}

fn decode<T: DeserializeOwned>(version: u32, document: &str) -> Result<T> {
    if !(MIN_DOCUMENT_VERSION..=DOCUMENT_VERSION).contains(&version) {
        return Err(StorageError::UnsupportedDocument(version));
    }
    Ok(serde_json::from_str(document)?)
}

pub(crate) fn parse_id<T: std::str::FromStr>(value: &str) -> rusqlite::Result<T> {
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
        let Some((version, document)) = row else {
            return Ok(None);
        };
        let mut profile: Profile = decode(version, &document)?;
        profile.projects = self.load_projects(id)?;
        profile.default_project = self
            .connection
            .query_row(
                "SELECT project_id FROM default_projects WHERE profile_id = ?1",
                [id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|value| parse_id(&value))
            .transpose()?;
        Ok(Some(profile))
    }

    fn save_profile(&self, profile: &Profile) -> Result<()> {
        let tx = self.write_transaction()?;
        write_profile(&tx, profile)?;
        tx.commit()?;
        Ok(())
    }

    fn delete_profile(&self, id: ProfileId) -> Result<bool> {
        let tx = self.write_transaction()?;
        let deleted = tx.execute("DELETE FROM profiles WHERE id = ?1", [id.to_string()])? > 0;
        tx.execute(
            "DELETE FROM workflow_executions WHERE profile_id = ?1",
            [id.to_string()],
        )?;
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
        let tx = self.write_transaction()?;
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
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT project_id FROM active_projects WHERE profile_id = ?1",
                [profile.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.map(|value| parse_id(&value)).transpose()?)
    }

    fn set_active_project(&self, profile: ProfileId, project: Option<ProjectId>) -> Result<()> {
        if !self.profile_exists(profile)? {
            return Err(StorageError::UnknownProfile);
        }
        let Some(project) = project else {
            self.connection.execute(
                "DELETE FROM active_projects WHERE profile_id = ?1",
                [profile.to_string()],
            )?;
            return Ok(());
        };
        // The composite foreign key rejects projects of other profiles.
        self.connection
            .execute(
                "INSERT INTO active_projects (profile_id, project_id) VALUES (?1, ?2)
                 ON CONFLICT (profile_id) DO UPDATE SET project_id = excluded.project_id",
                params![profile.to_string(), project.to_string()],
            )
            .map_err(|error| {
                if constraint_violation(&error) {
                    StorageError::ProjectOfAnotherProfile(project)
                } else {
                    error.into()
                }
            })?;
        Ok(())
    }

    fn module_states(&self) -> Result<ModuleStates> {
        let mut statement = self
            .connection
            .prepare("SELECT id, enabled, config FROM modules ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut states = ModuleStates::new();
        for row in rows {
            let (id, enabled, config) = row?;
            // Rows this build cannot read (an unknown id format, a damaged
            // configuration) are kept in the table but ignored here, so they
            // can never prevent DeckPilot from starting. The module then uses
            // its defaults, keeping the stored enabled flag.
            let Ok(id) = ModuleId::new(id) else { continue };
            let config = serde_json::from_str(&config).unwrap_or_default();
            states.insert(id, ModuleSettings { enabled, config });
        }
        Ok(states)
    }

    fn set_module_state(&self, id: &ModuleId, settings: &ModuleSettings) -> Result<()> {
        self.connection.execute(
            "INSERT INTO modules (id, enabled, config) VALUES (?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET enabled = excluded.enabled, config = excluded.config",
            params![id.as_str(), settings.enabled, encode(&settings.config)?],
        )?;
        Ok(())
    }

    fn migration_notes(&self) -> Result<Vec<MigrationNote>> {
        let mut statement = self.connection.prepare(
            "SELECT schema_version, profile_id, message, created_at FROM migration_notes ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut notes = Vec::new();
        for row in rows {
            let (schema_version, profile, message, created_at) = row?;
            notes.push(MigrationNote {
                schema_version,
                profile: profile.map(|p| parse_id(&p)).transpose()?,
                message,
                created_at,
            });
        }
        Ok(notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::{
        ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, MachineResourceBinding,
        Os, ParameterValue, ResourceKey, ResourceKind,
    };

    fn sample_profile(name: &str) -> Profile {
        let mut profile = Profile::new(name);
        let mut shop = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
        shop.resources
            .insert("repository".into(), ResourceKey::new("shop.repo").unwrap());
        profile.projects.push(shop);
        profile.actions.push(ActionInstance {
            id: ActionInstanceId::new(),
            name: "Hello".into(),
            action_type: ActionTypeId::new("general.log").unwrap(),
            parameters: ActionParameters::new().with("message", ParameterValue::Text("hi".into())),
            description: String::new(),
            icon: None,
            label: None,
        });
        profile
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("deckpilot-test-{}", ProfileId::new()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
        assert_eq!(profile.actions[0].action_type.as_str(), "general.log");
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
        // Migration 4 is the last one that rewrites documents; schema 8
        // leaves them at version 4, which decodes unchanged.
        assert!(versions.iter().all(|v| *v == 4), "{versions:?}");
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
    fn fresh_database_is_empty() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.list_profiles().unwrap(), Vec::new());
        assert_eq!(store.active_profile().unwrap(), None);
        assert_eq!(store.module_states().unwrap(), ModuleStates::new());
        assert_eq!(store.migration_notes().unwrap(), Vec::new());
        let count = |table: &str| -> i64 {
            store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        };
        // No demonstration data, no shared variables and no provenance:
        // only the development tools seed data (`tools/dev`).
        for table in [
            "projects",
            "shared_variables",
            "shared_variable_values",
            "provenance",
            "credentials",
            "workflow_executions",
        ] {
            assert_eq!(count(table), 0, "{table}");
        }
        assert_eq!(store.shared_variables().unwrap(), Vec::new());
    }

    #[test]
    fn projects_keep_order_and_live_in_their_own_table() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut profile = sample_profile("Work");
        for key in ["CMR", "CMS"] {
            profile
                .projects
                .push(Project::new(ProjectKey::new(key).unwrap(), key));
        }
        profile.default_project = Some(profile.projects[1].id);
        store.save_profile(&profile).unwrap();
        let document: String = store
            .connection
            .query_row("SELECT document FROM profiles", [], |row| row.get(0))
            .unwrap();
        assert!(
            !document.contains("SHOP"),
            "projects are not in the document"
        );

        // Reorder and swap two keys in one save.
        profile.projects.rotate_left(1);
        profile.projects[0].key = ProjectKey::new("CMS").unwrap();
        profile.projects[1].key = ProjectKey::new("CMR").unwrap();
        store.save_profile(&profile).unwrap();
        assert_eq!(
            store.load_profile(profile.id).unwrap(),
            Some(profile.clone())
        );

        // A removed project takes its default and active selection with it.
        let removed = profile.projects.remove(0);
        store
            .set_active_project(profile.id, Some(removed.id))
            .unwrap();
        profile.default_project = Some(removed.id);
        assert!(matches!(
            store.save_profile(&profile),
            Err(StorageError::ProjectOfAnotherProfile(_))
        ));
        profile.default_project = None;
        store.save_profile(&profile).unwrap();
        assert_eq!(store.active_project(profile.id).unwrap(), None);
        assert_eq!(store.load_profile(profile.id).unwrap(), Some(profile));
    }

    #[test]
    fn projects_cannot_move_between_profiles() {
        let store = SqliteStore::open_in_memory().unwrap();
        let work = sample_profile("Work");
        store.save_profile(&work).unwrap();

        // A second profile that claims the first profile's project.
        let mut home = Profile::new("Home");
        home.projects.push(work.projects[0].clone());
        assert!(matches!(
            store.save_profile(&home),
            Err(StorageError::ProjectOfAnotherProfile(_))
        ));
        // Nothing of the failed save was written.
        assert_eq!(store.load_profile(home.id).unwrap(), None);
        assert_eq!(store.load_profile(work.id).unwrap(), Some(work.clone()));

        // The same key in another profile is fine.
        let home = sample_profile("Home");
        store.save_profile(&home).unwrap();

        // Active and default project must be the profile's own.
        assert!(matches!(
            store.set_active_project(home.id, Some(work.projects[0].id)),
            Err(StorageError::ProjectOfAnotherProfile(_))
        ));
        let mut foreign_default = home.clone();
        foreign_default.default_project = Some(work.projects[0].id);
        assert!(store.save_profile(&foreign_default).is_err());

        // Each profile keeps its own active project.
        store
            .set_active_project(work.id, Some(work.projects[0].id))
            .unwrap();
        store
            .set_active_project(home.id, Some(home.projects[0].id))
            .unwrap();
        assert_eq!(
            store.active_project(work.id).unwrap(),
            Some(work.projects[0].id)
        );
        assert_eq!(
            store.active_project(home.id).unwrap(),
            Some(home.projects[0].id)
        );

        // Duplicate keys within one profile are rejected by the database.
        let mut duplicate = work.clone();
        duplicate
            .projects
            .push(Project::new(ProjectKey::new("shop").unwrap(), "Again"));
        assert!(matches!(
            store.save_profile(&duplicate),
            Err(StorageError::DuplicateProjectKey(_))
        ));
    }

    #[test]
    fn damaged_module_rows_do_not_fail_loading() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .connection
            .execute_batch(
                "INSERT INTO modules VALUES ('projects', 0, 'not json');
                 INSERT INTO modules VALUES ('Bad Id', 1, '{}');",
            )
            .unwrap();
        let states = store.module_states().unwrap();
        assert_eq!(states.len(), 1);
        let projects = &states[&ModuleId::new("projects").unwrap()];
        assert!(!projects.enabled);
        assert_eq!(projects.config, ActionParameters::new());
    }

    #[test]
    fn module_states_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let id = ModuleId::new("projects").unwrap();
        let settings = ModuleSettings {
            enabled: false,
            config: ActionParameters::new().with("wrapAround", ParameterValue::Boolean(false)),
        };
        store.set_module_state(&id, &settings).unwrap();
        assert_eq!(store.module_states().unwrap().get(&id), Some(&settings));
        let enabled = ModuleSettings {
            enabled: true,
            ..settings
        };
        store.set_module_state(&id, &enabled).unwrap();
        assert_eq!(store.module_states().unwrap()[&id], enabled);
    }

    /// Writes a schema 2 database with the given profile documents and
    /// active projects and opens it, which runs migration 3.
    fn migrate_v2(
        profiles: &[(ProfileId, serde_json::Value)],
        active: &[(ProfileId, ProjectId)],
    ) -> (SqliteStore, std::path::PathBuf) {
        let dir = temp_dir();
        let path = dir.join("v2.db");
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(MIGRATIONS[0]).unwrap();
            connection.execute_batch(MIGRATIONS[1]).unwrap();
            connection.pragma_update(None, "user_version", 2).unwrap();
            for (id, document) in profiles {
                connection
                    .execute(
                        "INSERT INTO profiles VALUES (?1, ?2, 2, ?3, 0)",
                        params![
                            id.to_string(),
                            document["name"].as_str(),
                            document.to_string()
                        ],
                    )
                    .unwrap();
            }
            for (profile, project) in active {
                connection
                    .execute(
                        "INSERT INTO profile_state VALUES (?1, ?2)",
                        params![profile.to_string(), project.to_string()],
                    )
                    .unwrap();
            }
        }
        (SqliteStore::open(&path, &Encryption::None).unwrap(), dir)
    }

    fn legacy_document(
        id: ProfileId,
        name: &str,
        projects: &[(ProjectId, &str)],
        actions: &serde_json::Value,
        workflows: &serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name": name, "description": "",
            "actions": actions, "workflows": workflows, "layouts": [],
            "projects": projects.iter().map(|(id, key)| serde_json::json!({
                "id": id, "key": key, "name": key,
                "resources": {"repository": format!("{}.repo", key.to_lowercase())}
            })).collect::<Vec<_>>(),
            "defaultProject": projects.first().map(|(id, _)| *id),
        })
    }

    #[test]
    fn migration_3_moves_projects_and_upgrades_actions() {
        let work = ProfileId::new();
        let (shop, cmr) = (ProjectId::new(), ProjectId::new());
        let (cycle, back, select, missing, log) = (
            ActionInstanceId::new(),
            ActionInstanceId::new(),
            ActionInstanceId::new(),
            ActionInstanceId::new(),
            ActionInstanceId::new(),
        );
        let text = |v: &str| serde_json::json!({"type": "text", "value": v});
        let choice = |v: &str| serde_json::json!({"type": "choice", "value": v});
        let actions = serde_json::json!([
            {"id": cycle, "name": "Next", "actionType": "deckpilot.project.cycle",
             "parameters": {"direction": choice("next")}},
            {"id": back, "name": "Back", "actionType": "deckpilot.project.cycle",
             "parameters": {"direction": choice("previous")}},
            {"id": select, "name": "CMR", "actionType": "deckpilot.project.select",
             "parameters": {"project": text("cmr")}},
            {"id": missing, "name": "Gone", "actionType": "deckpilot.project.select",
             "parameters": {"project": text("GONE")}},
            {"id": log, "name": "Log", "actionType": "deckpilot.core.log",
             "parameters": {"message": text("{project.key}")}},
        ]);
        let workflows = serde_json::json!([{
            "id": deckpilot_core::WorkflowId::new(), "name": "Flow",
            "steps": [
                {"action": cycle, "onFailure": "stop", "overrides": {"direction": choice("previous")}},
                {"action": select, "onFailure": "stop", "overrides": {"project": text("SHOP")},
                 "condition": {"type": "activeProject", "project": cmr}},
            ]
        }]);
        let document = legacy_document(
            work,
            "Work",
            &[(shop, "SHOP"), (cmr, "CMR")],
            &actions,
            &workflows,
        );
        let (store, dir) = migrate_v2(&[(work, document)], &[(work, cmr)]);

        let profile = store.load_profile(work).unwrap().unwrap();
        let keys: Vec<_> = profile.projects.iter().map(|p| p.key.to_string()).collect();
        assert_eq!(keys, ["SHOP", "CMR"]);
        assert_eq!(profile.projects[0].id, shop);
        assert_eq!(
            profile.projects[0].resources["repository"].as_str(),
            "shop.repo"
        );
        assert_eq!(profile.default_project, Some(shop));
        assert_eq!(store.active_project(work).unwrap(), Some(cmr));

        let types: Vec<_> = profile
            .actions
            .iter()
            .map(|a| a.action_type.to_string())
            .collect();
        assert_eq!(
            types,
            [
                "projects.next",
                "projects.previous",
                "projects.select",
                "projects.select",
                "general.log"
            ]
        );
        assert_eq!(profile.actions[0].parameters, ActionParameters::new());
        assert_eq!(
            profile.actions[2].parameters.project("project_id"),
            Some(cmr)
        );
        assert!(
            profile.actions[3].parameters.is_empty(),
            "unknown keys are not guessed"
        );
        let steps = &profile.workflows[0].steps;
        assert_eq!(steps[0].overrides, ActionParameters::new());
        assert_eq!(steps[1].overrides.project("project_id"), Some(shop));

        let notes: Vec<_> = store
            .migration_notes()
            .unwrap()
            .into_iter()
            .map(|n| n.message)
            .collect();
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(notes[2].contains("reports the current project"));
        assert!(notes[0].contains("\"Gone\""));
        assert!(notes[1].contains("direction override"));
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migration_4_converts_resource_actions_and_keeps_bindings() {
        use deckpilot_core::{BindingTarget, ControlBinding, DeviceLayout, Trigger};
        use deckpilot_device::{ControlId, DeviceModelId, models};

        let work = ProfileId::new();
        let (shop, cmr) = (ProjectId::new(), ProjectId::new());
        let ids: Vec<ActionInstanceId> = (0..4).map(|_| ActionInstanceId::new()).collect();
        let resource = |reference: serde_json::Value| serde_json::json!({"resource": {"type": "resource", "value": reference}});
        let actions = serde_json::json!([
            {"id": ids[0], "name": "Repo (active)", "actionType": "deckpilot.resource.resolve",
             "parameters": resource(serde_json::json!({"type": "projectRole", "role": "repository"}))},
            {"id": ids[1], "name": "Repo SHOP", "actionType": "deckpilot.resource.resolve",
             "parameters": resource(serde_json::json!({"type": "fixed", "key": "shop.repo"}))},
            {"id": ids[2], "name": "Elsewhere", "actionType": "deckpilot.resource.resolve",
             "parameters": resource(serde_json::json!({"type": "fixed", "key": "other.thing"}))},
            {"id": ids[3], "name": "Pick SHOP", "actionType": "deckpilot.project.select",
             "parameters": {"project": {"type": "text", "value": "SHOP"}}},
        ]);
        let workflows = serde_json::json!([{
            "id": deckpilot_core::WorkflowId::new(), "name": "Flow",
            "steps": [{"action": ids[0], "onFailure": "stop",
                       "overrides": resource(serde_json::json!({"type": "fixed", "key": "cmr.repo"}))}]
        }]);
        let mut document = legacy_document(
            work,
            "Work",
            &[(shop, "SHOP"), (cmr, "CMR")],
            &actions,
            &workflows,
        );
        let mut layout = DeviceLayout::new("Main", DeviceModelId::new(models::STREAM_DECK_MK2_ID));
        for (index, action) in ids.iter().enumerate() {
            layout.pages[0].bindings.push(ControlBinding {
                control: ControlId::Key {
                    index: u16::try_from(index).unwrap(),
                },
                trigger: Trigger::Press,
                target: BindingTarget::Action { action: *action },
                label: None,
                appearance: None,
            });
        }
        document["layouts"] = serde_json::to_value([&layout]).unwrap();
        let (store, dir) = migrate_v2(&[(work, document)], &[]);
        assert_eq!(
            store.schema_version().unwrap(),
            SqliteStore::supported_schema_version()
        );

        let profile = store.load_profile(work).unwrap().unwrap();
        // Same actions, same ids, same order; layouts untouched.
        let stored: Vec<_> = profile.actions.iter().map(|a| a.id).collect();
        assert_eq!(stored, ids);
        assert_eq!(profile.layouts, vec![layout]);
        let text = |value: &str| ParameterValue::Text(value.into());
        let p = |index: usize| &profile.actions[index].parameters;
        assert_eq!(
            p(0),
            &ActionParameters::new().with("resource", text("repository"))
        );
        assert_eq!(
            p(1),
            &ActionParameters::new()
                .with("project_id", ParameterValue::Project(shop))
                .with("resource", text("repository"))
        );
        // Unresolvable: kept, but without a resource to pick.
        assert_eq!(p(2), &ActionParameters::new());
        assert_eq!(
            profile.actions[2].action_type.as_str(),
            "projects.resolve-resource"
        );
        assert_eq!(
            p(3),
            &ActionParameters::new().with("project_id", ParameterValue::Project(shop))
        );
        assert_eq!(
            profile.workflows[0].steps[0].overrides,
            ActionParameters::new()
                .with("project_id", ParameterValue::Project(cmr))
                .with("resource", text("repository"))
        );
        let notes: Vec<_> = store
            .migration_notes()
            .unwrap()
            .into_iter()
            .map(|n| (n.schema_version, n.message))
            .collect();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].0, 4);
        assert!(notes[0].1.contains("\"other.thing\""), "{notes:?}");
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migration_3_resolves_ambiguous_legacy_data_deterministically() {
        // Two profiles share a project id (a hand-copied document), one of
        // them uses a key twice, and an active project points elsewhere.
        let (first, second) = {
            let (a, b) = (ProfileId::new(), ProfileId::new());
            if a.to_string() < b.to_string() {
                (a, b)
            } else {
                (b, a)
            }
        };
        let shared = ProjectId::new();
        let other = ProjectId::new();
        let select = ActionInstanceId::new();
        let actions = serde_json::json!([
            {"id": select, "name": "Shop", "actionType": "deckpilot.project.select",
             "parameters": {"project": {"type": "text", "value": "SHOP"}}}
        ]);
        let first_doc = legacy_document(
            first,
            "First",
            &[(shared, "SHOP")],
            &serde_json::json!([]),
            &serde_json::json!([]),
        );
        let second_doc = legacy_document(
            second,
            "Second",
            &[(shared, "SHOP"), (other, "shop")],
            &actions,
            &serde_json::json!([]),
        );
        let run = || {
            migrate_v2(
                &[(first, first_doc.clone()), (second, second_doc.clone())],
                &[(first, other), (second, shared)],
            )
        };
        let (store, dir) = run();
        let a = store.load_profile(first).unwrap().unwrap();
        let b = store.load_profile(second).unwrap().unwrap();

        // The profile processed first keeps the id; the second gets a new one
        // and its references follow.
        assert_eq!(a.projects[0].id, shared);
        assert_ne!(b.projects[0].id, shared);
        assert_eq!(b.default_project, Some(b.projects[0].id));
        assert_eq!(
            b.actions[0].parameters.project("project_id"),
            Some(b.projects[0].id)
        );
        assert_eq!(
            store.active_project(second).unwrap(),
            Some(b.projects[0].id)
        );
        // Keys are unique per profile again; nothing moved between profiles.
        assert_eq!(b.projects[1].key.as_str(), "shop-2");
        assert_eq!(b.projects[1].id, other);
        assert_eq!(a.projects.len(), 1);
        // The first profile's active project belonged to the second one.
        assert_eq!(store.active_project(first).unwrap(), None);
        let notes = store.migration_notes().unwrap();
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(notes.iter().all(|n| n.profile.is_some()));
        let derived = b.projects[0].id;
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();

        // Running the same migration again produces the same ids.
        let (store, dir) = run();
        assert_eq!(
            store.load_profile(second).unwrap().unwrap().projects[0].id,
            derived
        );
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn database_key_is_redacted() {
        assert_eq!(
            format!("{:?}", DatabaseKey::random().unwrap()),
            "SecretKey(<redacted>)"
        );
    }

    #[cfg(not(feature = "sqlcipher"))]
    #[test]
    fn encryption_requires_sqlcipher_build() {
        let result =
            SqliteStore::open(":memory:", &Encryption::Key(DatabaseKey::random().unwrap()));
        assert!(matches!(result, Err(StorageError::EncryptionUnavailable)));
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn encrypted_database_requires_correct_key() {
        let dir = temp_dir();
        let path = dir.join("encrypted.db");
        let key = Encryption::Key(DatabaseKey::random().unwrap());
        {
            let store = SqliteStore::open(&path, &key).unwrap();
            store.save_profile(&sample_profile("Work")).unwrap();
        }
        assert!(SqliteStore::open(&path, &key).is_ok());
        let wrong = Encryption::Key(DatabaseKey::random().unwrap());
        assert!(matches!(
            SqliteStore::open(&path, &wrong),
            Err(StorageError::InvalidKey)
        ));
        assert!(SqliteStore::open(&path, &Encryption::None).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn upgrades_are_preceded_by_a_verified_encrypted_backup() {
        let dir = temp_dir();
        let path = dir.join("deckpilot.db");
        let key = DatabaseKey::random().unwrap();
        let encryption = Encryption::Key(key.clone());
        let profile = sample_profile("Work");
        {
            // A database as the previous release left it: schema 4.
            let store = SqliteStore::open(&path, &encryption).unwrap();
            store.save_profile(&profile).unwrap();
            store
                .connection
                .execute_batch(
                    "DROP TABLE migration_history; DROP TABLE module_schemas;
                     DROP TABLE workflow_executions; DROP TABLE credentials;
                     DROP TABLE shared_variable_values; DROP TABLE shared_variables;
                     DROP TABLE provenance;
                     PRAGMA user_version = 4;",
                )
                .unwrap();
        }
        let store = SqliteStore::open(&path, &encryption).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            SqliteStore::supported_schema_version()
        );
        assert_eq!(store.load_profile(profile.id).unwrap(), Some(profile));

        let history = store.migration_history().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(
            (history[0].from_version, history[0].to_version),
            (4, SqliteStore::supported_schema_version())
        );
        assert_eq!(history[0].result, "ok");
        let backup = std::path::PathBuf::from(history[0].backup.clone().unwrap());
        assert!(backup.starts_with(backup_dir(&path)));

        // The backup is encrypted with the same key and holds schema 4.
        let header = std::fs::read(&backup).unwrap();
        assert_ne!(&header[..15], b"SQLite format 3");
        verify_copy(&backup, &encryption, 4).unwrap();
        assert!(matches!(
            verify_copy(&backup, &Encryption::Key(DatabaseKey::random().unwrap()), 4),
            Err(StorageError::InvalidKey)
        ));

        // Opening again does nothing further.
        drop(store);
        let store = SqliteStore::open(&path, &encryption).unwrap();
        assert_eq!(store.migration_history().unwrap().len(), 1);
        // Windows cannot delete files that are still open.
        drop(store);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn module_schemas_are_versioned() {
        let store = SqliteStore::open_in_memory().unwrap();
        let module = ModuleId::new("notes").unwrap();
        let step = |version: u32, sql: &str| ModuleMigration {
            version,
            description: String::new(),
            sql: sql.into(),
        };
        let schema = vec![
            step(
                1,
                "CREATE TABLE notes_items (id INTEGER PRIMARY KEY, text TEXT NOT NULL);",
            ),
            step(
                2,
                "ALTER TABLE notes_items ADD COLUMN done INTEGER NOT NULL DEFAULT 0;",
            ),
        ];
        store
            .apply_module_schemas(&[(module.clone(), schema[..1].to_vec())])
            .unwrap();
        assert_eq!(store.module_schema_version(&module).unwrap(), 1);
        store
            .apply_module_schemas(&[(module.clone(), schema.clone())])
            .unwrap();
        assert_eq!(store.module_schema_version(&module).unwrap(), 2);
        store
            .connection
            .execute("INSERT INTO notes_items (text, done) VALUES ('x', 1)", [])
            .unwrap();
        // A newer module schema than this build knows is refused.
        assert!(matches!(
            store.apply_module_schemas(&[(module.clone(), schema[..1].to_vec())]),
            Err(StorageError::UnsupportedModuleSchema { found: 2, .. })
        ));
        // A failing step leaves the previous version in place.
        let broken = vec![schema[0].clone(), schema[1].clone(), step(3, "NOT SQL")];
        assert!(
            store
                .apply_module_schemas(&[(module.clone(), broken)])
                .is_err()
        );
        assert_eq!(store.module_schema_version(&module).unwrap(), 2);
    }

    #[test]
    fn concurrent_writers_wait_for_each_other() {
        // The agent and the app each have a connection to the same file.
        let dir = std::env::temp_dir().join(format!("deckpilot-concurrent-{}", ProfileId::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("deckpilot.db");
        SqliteStore::open(&path, &Encryption::None).unwrap();
        // Two connections (the agent and the app), interleaving writes that
        // read first. Kept small: on slow disks (CI) a larger load can make a
        // writer wait longer than the busy timeout, which is not what this
        // test is about — it catches the immediate "database is locked" of a
        // deferred transaction that cannot upgrade its lock.
        let writers: Vec<_> = (0..2)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let store = SqliteStore::open(&path, &Encryption::None).unwrap();
                    for i in 0..10 {
                        store
                            .record_launch(&format!("component-{n}"), "0.4.0")
                            .unwrap();
                        store
                            .save_profile(&Profile::new(format!("P{n}-{i}")))
                            .unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let store = SqliteStore::open(&path, &Encryption::None).unwrap();
        assert_eq!(store.list_profiles().unwrap().len(), 20);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changes_are_applied_atomically() {
        let store = SqliteStore::open_in_memory().unwrap();
        let existing = sample_profile("Existing");
        store.save_profile(&existing).unwrap();
        let fresh = sample_profile("Fresh");
        // The second profile claims a project of the existing one: the whole
        // change set must be rejected, including the deletion.
        let mut thief = Profile::new("Thief");
        thief.projects.push(existing.projects[0].clone());
        let changes = ConfigChanges {
            delete_profiles: vec![existing.id],
            save_profiles: vec![fresh.clone(), thief],
            module_states: Vec::new(),
            settings: Vec::new(),
        };
        // The deletion makes the project free again, so this succeeds…
        store.apply_changes(&changes).unwrap();
        assert_eq!(store.load_profile(existing.id).unwrap(), None);
        // …while a real conflict rolls everything back.
        let mut thief = Profile::new("Thief 2");
        thief.projects.push(fresh.projects[0].clone());
        let other = sample_profile("Other");
        let failing = ConfigChanges {
            delete_profiles: Vec::new(),
            save_profiles: vec![other.clone(), thief],
            module_states: vec![(
                ModuleId::new("projects").unwrap(),
                ModuleSettings {
                    enabled: false,
                    config: ActionParameters::new(),
                },
            )],
            settings: vec![("update.preferences".into(), "{}".into())],
        };
        assert!(store.apply_changes(&failing).is_err());
        assert_eq!(store.load_profile(other.id).unwrap(), None);
        assert_eq!(store.module_states().unwrap(), ModuleStates::new());
        assert_eq!(store.setting("update.preferences").unwrap(), None);
        assert_eq!(store.load_profile(fresh.id).unwrap(), Some(fresh));
    }
}
