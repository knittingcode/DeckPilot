//! The plugin database: its own SQLCipher file, its own migrations.
//!
//! The open sequence mirrors `deckpilot-storage` (raw 256-bit key, schema
//! read to prove the key, busy timeout, foreign keys, WAL, `IMMEDIATE`
//! write transactions) without depending on it: that crate pulls the app's
//! core, device and keystore crates. The configuration database and its
//! `MIGRATIONS` are never opened or changed here.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, Transaction, TransactionBehavior};
use zeroize::Zeroizing;

use crate::error::{Result, StoreError};

/// The configuration database's file name (`deckpilot-ipc` paths). The
/// plugin store refuses to open a file of that name.
pub const CONFIG_DATABASE_FILE: &str = "deckpilot.db";

/// The busy timeout of every connection, as in `deckpilot-storage`.
pub const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Schema migrations of the plugin database, in order. Version `n` is
/// `MIGRATIONS[n - 1]`; `PRAGMA user_version` holds the applied version.
/// Never edit a released entry: append a new one.
pub const MIGRATIONS: &[&str] = &[
    // 1: installs, versions, grants, audit and the operation journal.
    r"
    CREATE TABLE plugins (
        plugin_id    TEXT PRIMARY KEY,
        state        TEXT NOT NULL CHECK (state IN ('installing', 'disabled', 'enabled',
                         'updating', 'rolling_back', 'uninstalling', 'broken', 'quarantined')),
        state_reason TEXT,
        source       TEXT NOT NULL CHECK (source IN ('registry', 'sideload', 'dev')),
        registry     TEXT,
        installed_at INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL,
        CHECK ((source = 'registry') = (registry IS NOT NULL))
    ) STRICT;

    CREATE TABLE plugin_versions (
        plugin_id      TEXT NOT NULL REFERENCES plugins (plugin_id) ON DELETE CASCADE,
        version        TEXT NOT NULL,
        role           TEXT NOT NULL CHECK (role IN ('active', 'previous', 'staged')),
        install_dir    TEXT NOT NULL,
        package_sha256 TEXT NOT NULL,
        integrity_json TEXT NOT NULL,
        manifest_json  TEXT NOT NULL,
        installed_at   INTEGER NOT NULL,
        activated_at   INTEGER,
        PRIMARY KEY (plugin_id, version)
    ) STRICT;
    CREATE UNIQUE INDEX plugin_versions_role ON plugin_versions (plugin_id, role);

    CREATE TABLE plugin_grants (
        plugin_id           TEXT NOT NULL REFERENCES plugins (plugin_id) ON DELETE CASCADE,
        kind                TEXT NOT NULL,
        grant_key           TEXT NOT NULL,
        grant_json          TEXT NOT NULL,
        status              TEXT NOT NULL CHECK (status IN ('granted', 'revoked')),
        source              TEXT NOT NULL CHECK (source IN ('install', 'update', 'user', 'derived')),
        decided_by          TEXT NOT NULL,
        decided_at          INTEGER NOT NULL,
        granted_for_version TEXT NOT NULL,
        PRIMARY KEY (plugin_id, kind, grant_key)
    ) STRICT;

    -- No foreign key: the audit outlives an uninstall. Targets are grant
    -- keys (a host, a variable id, a credential key), never values.
    CREATE TABLE plugin_audit (
        id        INTEGER PRIMARY KEY AUTOINCREMENT,
        at        INTEGER NOT NULL,
        plugin_id TEXT NOT NULL,
        version   TEXT,
        op        TEXT NOT NULL,
        target    TEXT,
        decision  TEXT NOT NULL,
        reason    TEXT
    ) STRICT;
    CREATE INDEX plugin_audit_plugin ON plugin_audit (plugin_id, at);

    -- At most one unfinished operation per plugin.
    CREATE TABLE plugin_journal (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        plugin_id    TEXT NOT NULL UNIQUE,
        op           TEXT NOT NULL CHECK (op IN ('install', 'update', 'uninstall')),
        step         TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        started_at   INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE schema_migrations (
        version    INTEGER PRIMARY KEY,
        applied_at INTEGER NOT NULL
    ) STRICT;
    ",
    // 2: per-plugin data (design §2.3 items 4-6, §12.4) and the plugins
    // ever installed.
    r"
    -- The plugin's key-value storage: deleted with the plugin.
    CREATE TABLE plugin_storage (
        plugin_id  TEXT NOT NULL REFERENCES plugins (plugin_id) ON DELETE CASCADE,
        key        TEXT NOT NULL,
        value      TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (plugin_id, key)
    ) STRICT;

    -- Credential secrets by slot. No foreign key: an uninstall may keep
    -- them for a later install. Never exported, never logged.
    CREATE TABLE plugin_credentials (
        plugin_id  TEXT NOT NULL,
        key        TEXT NOT NULL,
        secret     TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (plugin_id, key)
    ) STRICT;

    -- Settings values the user saved (defaults are not stored). No
    -- foreign key: an uninstall may keep them.
    CREATE TABLE plugin_settings (
        plugin_id   TEXT PRIMARY KEY,
        values_json TEXT NOT NULL,
        updated_at  INTEGER NOT NULL
    ) STRICT;

    -- Every plugin ever installed; grows only.
    CREATE TABLE plugin_known (
        plugin_id     TEXT PRIMARY KEY,
        namespace     TEXT NOT NULL UNIQUE,
        publisher     TEXT NOT NULL,
        display_name  TEXT NOT NULL,
        last_version  TEXT NOT NULL,
        actions_json  TEXT NOT NULL,
        first_seen_at INTEGER NOT NULL,
        last_seen_at  INTEGER NOT NULL
    ) STRICT;
    ",
    // 3: bundled plugins, the module migration journal and tombstones
    // (`docs/marketplace/module-migration.md` §4.2, §5.6, §5.9).
    //
    // `plugins.source` gains 'bundled'. SQLite cannot alter a CHECK
    // constraint, so the table is rebuilt: a new table, a copy, a drop and
    // a rename, with foreign keys off for the transaction
    // (`Database::migrate`) so dropping the old table does not cascade into
    // its children, and `PRAGMA foreign_key_check` before the commit. The
    // children's `REFERENCES plugins` name the new table after the rename.
    //
    // First-party ids (`deckpilot.*`) come only from the application
    // bundle or the registry, and a bundled plugin is always first-party
    // (module-migration.md §4.4, §4.5); the store
    // checks the same rule before every write (`store::check_source`).
    r"
    CREATE TABLE plugins_v3 (
        plugin_id    TEXT PRIMARY KEY,
        state        TEXT NOT NULL CHECK (state IN ('installing', 'disabled', 'enabled',
                         'updating', 'rolling_back', 'uninstalling', 'broken', 'quarantined')),
        state_reason TEXT,
        source       TEXT NOT NULL CHECK (source IN ('registry', 'sideload', 'dev', 'bundled')),
        registry     TEXT,
        installed_at INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL,
        CHECK ((source = 'registry') = (registry IS NOT NULL)),
        CHECK ((substr(plugin_id, 1, 10) = 'deckpilot.') OR source <> 'bundled'),
        CHECK ((substr(plugin_id, 1, 10) <> 'deckpilot.') OR source IN ('registry', 'bundled'))
    ) STRICT;
    INSERT INTO plugins_v3 (plugin_id, state, state_reason, source, registry, installed_at,
                            updated_at)
        SELECT plugin_id, state, state_reason, source, registry, installed_at, updated_at
        FROM plugins;
    DROP TABLE plugins;
    ALTER TABLE plugins_v3 RENAME TO plugins;

    -- One record per built-in module moving to its plugin. Reasons and
    -- digests never contain values or secrets. No foreign key: the record
    -- outlives an uninstall of the plugin.
    CREATE TABLE module_migrations (
        module_id     TEXT PRIMARY KEY,
        plugin_id     TEXT NOT NULL CHECK (substr(plugin_id, 1, 10) = 'deckpilot.'),
        phase         TEXT NOT NULL CHECK (phase IN ('pending', 'not_eligible',
                          'installed', 'settings_copied', 'awaiting_consent',
                          'cut_over', 'failed')),
        reason        TEXT,
        attempts      INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
        source_digest TEXT,
        app_version   TEXT NOT NULL,
        updated_at    INTEGER NOT NULL
    ) STRICT;

    -- The user's explicit uninstall or disable of a first-party plugin.
    -- No foreign key: a tombstone outlives the plugin it removed.
    CREATE TABLE plugin_tombstones (
        plugin_id  TEXT PRIMARY KEY CHECK (substr(plugin_id, 1, 10) = 'deckpilot.'),
        decision   TEXT NOT NULL CHECK (decision IN ('uninstalled', 'disabled')),
        decided_at INTEGER NOT NULL
    ) STRICT;
    ",
];

/// The raw 256-bit SQLCipher key of the plugin database, supplied by the
/// caller (the agent derives it next to the configuration key). Zeroized
/// when dropped, never printed.
#[derive(Clone)]
pub struct StoreKey(Zeroizing<[u8; 32]>);

impl StoreKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// SQLCipher's raw-key literal (`x'…'`), as `deckpilot-storage` builds it.
    fn literal(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut literal = Zeroizing::new(String::with_capacity(67));
        literal.push_str("x'");
        for byte in self.0.iter() {
            literal.push(char::from(HEX[usize::from(byte >> 4)]));
            literal.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        literal.push('\'');
        literal
    }
}

impl fmt::Debug for StoreKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StoreKey(..)")
    }
}

/// The schema version this build writes.
pub fn supported_schema_version() -> u32 {
    schema_version_of(MIGRATIONS)
}

fn schema_version_of(migrations: &[&str]) -> u32 {
    u32::try_from(migrations.len()).unwrap_or(u32::MAX)
}

/// An open, keyed and migrated plugin database connection.
#[derive(Debug)]
pub struct Database {
    connection: Connection,
    path: PathBuf,
}

impl Database {
    pub fn open(path: &Path, key: &StoreKey, busy_timeout: Duration) -> Result<Self> {
        Self::open_as(path, key, busy_timeout, MIGRATIONS)
    }

    /// [`Database::open`] as a build that knows only `migrations` (tests
    /// open with a prefix of [`MIGRATIONS`] to act as an older build).
    pub(crate) fn open_as(
        path: &Path,
        key: &StoreKey,
        busy_timeout: Duration,
        migrations: &[&str],
    ) -> Result<Self> {
        if path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(CONFIG_DATABASE_FILE))
        {
            return Err(StoreError::ConfigDatabase(path.display().to_string()));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::io(parent, &e))?;
        }
        let connection = Connection::open(path)?;
        apply_key(&connection, key)?;
        connection.busy_timeout(busy_timeout)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let found = user_version(&connection)?;
        let supported = schema_version_of(migrations);
        if found > supported {
            return Err(StoreError::UnsupportedSchema { found, supported });
        }
        retry_busy(busy_timeout, || {
            let mode: String =
                connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
            if !mode.eq_ignore_ascii_case("wal") {
                connection.pragma_update(None, "journal_mode", "WAL")?;
            }
            Ok(())
        })?;
        let mut database = Self {
            connection,
            path: path.to_path_buf(),
        };
        database.migrate(migrations)?;
        Ok(database)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Re-encrypts the database at `path` from `old` to `new` in place
    /// (SQLCipher `PRAGMA rekey`), for when the key it is derived from
    /// changes. Nothing else may have it open. A wrong `old` key fails with
    /// [`StoreError::InvalidKey`] and changes nothing. The journal is
    /// switched out of WAL for the rekey; the next [`Database::open`]
    /// switches it back.
    pub fn rekey(path: &Path, old: &StoreKey, new: &StoreKey) -> Result<()> {
        let connection = Connection::open(path)?;
        apply_key(&connection, old)?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        let literal = new.literal();
        connection.pragma_update(None, "rekey", literal.as_str())?;
        drop(connection);
        // The new key must open it.
        let check = Connection::open(path)?;
        apply_key(&check, new)
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// A transaction holding the write lock from its first statement
    /// (`BEGIN IMMEDIATE`): a second writer waits for the busy timeout
    /// instead of failing half-way.
    pub fn write(&mut self) -> Result<Transaction<'_>> {
        Ok(self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?)
    }

    pub fn schema_version(&self) -> Result<u32> {
        user_version(&self.connection)
    }

    /// Applies the missing `migrations` forward, all in one transaction:
    /// either every step commits or the file stays at its old version.
    ///
    /// A migration may rebuild a table other tables reference (schema 3
    /// rebuilds `plugins`). Foreign keys are therefore off while
    /// migrations run (the pragma is a no-op inside a transaction, so it is
    /// set around it), every reference is checked with
    /// `PRAGMA foreign_key_check` before the commit, and foreign keys are
    /// switched back on whatever the outcome.
    fn migrate(&mut self, migrations: &[&str]) -> Result<()> {
        let supported = schema_version_of(migrations);
        if user_version(&self.connection)? >= supported {
            // Nothing to apply; a newer version is refused inside the
            // transaction below.
            return self.apply(migrations);
        }
        self.connection.pragma_update(None, "foreign_keys", false)?;
        let result = self.apply(migrations);
        let restored = self.connection.pragma_update(None, "foreign_keys", true);
        result?;
        restored?;
        Ok(())
    }

    fn apply(&mut self, migrations: &[&str]) -> Result<()> {
        let tx = self.write()?;
        let current = user_version(&tx)?;
        let supported = schema_version_of(migrations);
        if current > supported {
            return Err(StoreError::UnsupportedSchema {
                found: current,
                supported,
            });
        }
        if current == supported {
            return Ok(());
        }
        for (index, sql) in migrations.iter().enumerate().skip(current as usize) {
            let version = u32::try_from(index + 1).unwrap_or(u32::MAX);
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![version, crate::system_now()],
            )?;
            tx.pragma_update(None, "user_version", version)?;
        }
        let dangling = tx
            .prepare("PRAGMA foreign_key_check")?
            .query([])?
            .next()?
            .is_some();
        if dangling {
            // Dropping the transaction rolls every step back.
            return Err(StoreError::Corrupt(
                "a migration left a reference to a missing row".into(),
            ));
        }
        tx.commit()?;
        Ok(())
    }
}

fn user_version(connection: &Connection) -> Result<u32> {
    Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn retry_busy(timeout: Duration, mut f: impl FnMut() -> Result<()>) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match f() {
            Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(failure, _)))
                if failure.code == rusqlite::ErrorCode::DatabaseBusy
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            other => return other,
        }
    }
}

#[cfg(feature = "sqlcipher")]
fn apply_key(connection: &Connection, key: &StoreKey) -> Result<()> {
    use rusqlite::OptionalExtension as _;
    // Plain SQLite linked in by accident must not silently write clear text.
    let cipher = connection
        .query_row("PRAGMA cipher_version", [], |row| row.get::<_, String>(0))
        .optional()?
        .filter(|version| !version.is_empty());
    if cipher.is_none() {
        return Err(StoreError::EncryptionUnavailable);
    }
    let literal = key.literal();
    connection.pragma_update(None, "key", literal.as_str())?;
    // A wrong key fails here: the first page does not authenticate.
    connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| match &error {
            rusqlite::Error::SqliteFailure(failure, message)
                if failure.code == rusqlite::ErrorCode::DatabaseCorrupt =>
            {
                StoreError::Corrupt(
                    message
                        .clone()
                        .unwrap_or_else(|| "the database file is damaged".into()),
                )
            }
            _ => StoreError::InvalidKey,
        })?;
    Ok(())
}

#[cfg(not(feature = "sqlcipher"))]
fn apply_key(_connection: &Connection, _key: &StoreKey) -> Result<()> {
    Err(StoreError::EncryptionUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn rekey_replaces_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        let (old, new) = (StoreKey::from_bytes([1; 32]), StoreKey::from_bytes([2; 32]));
        let database = Database::open(&path, &old, DEFAULT_BUSY_TIMEOUT).unwrap();
        let version = database.schema_version().unwrap();
        drop(database);
        assert!(matches!(
            Database::rekey(&path, &new, &old),
            Err(StoreError::InvalidKey)
        ));
        Database::rekey(&path, &old, &new).unwrap();
        assert!(matches!(
            Database::open(&path, &old, DEFAULT_BUSY_TIMEOUT),
            Err(StoreError::InvalidKey)
        ));
        let reopened = Database::open(&path, &new, DEFAULT_BUSY_TIMEOUT).unwrap();
        assert_eq!(reopened.schema_version().unwrap(), version);
    }

    #[test]
    fn key_literal_is_raw_hex_and_debug_hides_it() {
        let key = StoreKey::from_bytes([0xab; 32]);
        let literal = key.literal();
        assert_eq!(literal.len(), 67);
        assert!(literal.starts_with("x'abab") && literal.ends_with("ab'"));
        assert_eq!(format!("{key:?}"), "StoreKey(..)");
    }

    #[test]
    fn refuses_the_configuration_database_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let result = Database::open(
            &dir.path().join("deckpilot.db"),
            &StoreKey::from_bytes([1; 32]),
            DEFAULT_BUSY_TIMEOUT,
        );
        assert!(matches!(result, Err(StoreError::ConfigDatabase(_))));
        assert!(!dir.path().join("deckpilot.db").exists());
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn migrates_and_refuses_newer_schemas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        let key = StoreKey::from_bytes([7; 32]);
        let db = Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT).unwrap();
        assert_eq!(db.schema_version().unwrap(), supported_schema_version());
        let applied: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(applied, i64::from(supported_schema_version()));
        db.connection()
            .pragma_update(None, "user_version", 99)
            .unwrap();
        drop(db);
        // A newer schema is refused before anything could write to it.
        let result = Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT);
        assert!(matches!(
            result,
            Err(StoreError::UnsupportedSchema { found: 99, .. })
        ));
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn reopening_does_not_reapply_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        let key = StoreKey::from_bytes([8; 32]);
        drop(Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT).unwrap());
        let db = Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT).unwrap();
        let applied: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(applied, i64::from(supported_schema_version()));
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn a_schema_1_database_gains_the_data_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        let key = StoreKey::from_bytes([9; 32]);
        {
            // What a build with only the first migration wrote.
            let connection = Connection::open(&path).unwrap();
            apply_key(&connection, &key).unwrap();
            connection.execute_batch(MIGRATIONS[0]).unwrap();
            connection.pragma_update(None, "user_version", 1).unwrap();
        }
        let db = Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT).unwrap();
        assert_eq!(db.schema_version().unwrap(), supported_schema_version());
        for table in [
            "plugin_storage",
            "plugin_credentials",
            "plugin_settings",
            "plugin_known",
            "module_migrations",
            "plugin_tombstones",
        ] {
            assert!(has_table(&db, table), "{table}");
        }
    }

    fn has_table(db: &Database, table: &str) -> bool {
        db.connection()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    }

    fn count(db: &Database, sql: &str) -> i64 {
        db.connection().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// The schema-2 build: this build's first two migrations only.
    const SCHEMA_2: &[&str] = &[MIGRATIONS[0], MIGRATIONS[1]];

    /// What a schema-2 build wrote: two plugins with versions, grants,
    /// storage, settings and an audit row.
    #[cfg(feature = "sqlcipher")]
    fn schema_2_database(path: &Path, key: &StoreKey) {
        let db = Database::open_as(path, key, DEFAULT_BUSY_TIMEOUT, SCHEMA_2).unwrap();
        assert_eq!(db.schema_version().unwrap(), 2);
        db.connection()
            .execute_batch(
                r"
                INSERT INTO plugins (plugin_id, state, source, registry, installed_at, updated_at)
                VALUES ('acme.one', 'enabled', 'registry', 'https://registry.example', 1, 2),
                       ('acme.two', 'disabled', 'sideload', NULL, 3, 4);
                INSERT INTO plugin_versions (plugin_id, version, role, install_dir,
                    package_sha256, integrity_json, manifest_json, installed_at, activated_at)
                VALUES ('acme.one', '1.0.0', 'active', 'acme.one/1.0.0', 'aa', '{}', '{}', 1, 1),
                       ('acme.one', '0.9.0', 'previous', 'acme.one/0.9.0', 'bb', '{}', '{}', 1, 1),
                       ('acme.two', '2.0.0', 'active', 'acme.two/2.0.0', 'cc', '{}', '{}', 3, 3);
                INSERT INTO plugin_grants (plugin_id, kind, grant_key, grant_json, status,
                    source, decided_by, decided_at, granted_for_version)
                VALUES ('acme.one', 'network', 'api.example.com', '{}', 'granted', 'install',
                        'user', 1, '1.0.0');
                INSERT INTO plugin_storage (plugin_id, key, value, updated_at)
                VALUES ('acme.one', 'cursor', '1', 1);
                INSERT INTO plugin_settings (plugin_id, values_json, updated_at)
                VALUES ('acme.two', '{}', 3);
                INSERT INTO plugin_audit (at, plugin_id, op, decision)
                VALUES (1, 'acme.one', 'install', 'ok');
                ",
            )
            .unwrap();
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn a_schema_2_database_migrates_to_schema_3_keeping_every_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        let key = StoreKey::from_bytes([10; 32]);
        schema_2_database(&path, &key);

        let db = Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT).unwrap();
        assert_eq!(db.schema_version().unwrap(), 3);
        assert!(has_table(&db, "module_migrations"));
        assert!(has_table(&db, "plugin_tombstones"));
        assert!(!has_table(&db, "plugins_v3"));
        assert_eq!(count(&db, "SELECT max(version) FROM schema_migrations"), 3);
        // The rebuild cascaded into nothing.
        assert_eq!(count(&db, "SELECT count(*) FROM plugins"), 2);
        assert_eq!(count(&db, "SELECT count(*) FROM plugin_versions"), 3);
        assert_eq!(count(&db, "SELECT count(*) FROM plugin_grants"), 1);
        assert_eq!(count(&db, "SELECT count(*) FROM plugin_storage"), 1);
        assert_eq!(count(&db, "SELECT count(*) FROM plugin_settings"), 1);
        assert_eq!(count(&db, "SELECT count(*) FROM plugin_audit"), 1);
        let row: (String, String, Option<String>, i64, i64) = db
            .connection()
            .query_row(
                "SELECT state, source, registry, installed_at, updated_at
                 FROM plugins WHERE plugin_id = 'acme.one'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "enabled".into(),
                "registry".into(),
                Some("https://registry.example".into()),
                1,
                2
            )
        );

        // Foreign keys are back on and the children reference the rebuilt
        // table: a delete still cascades, a dangling version is refused.
        assert_eq!(count(&db, "PRAGMA foreign_keys"), 1);
        assert!(
            db.connection()
                .execute(
                    "INSERT INTO plugin_versions (plugin_id, version, role, install_dir,
                         package_sha256, integrity_json, manifest_json, installed_at)
                     VALUES ('acme.none', '1.0.0', 'active', 'x', 'x', '{}', '{}', 1)",
                    [],
                )
                .is_err()
        );
        db.connection()
            .execute("DELETE FROM plugins WHERE plugin_id = 'acme.one'", [])
            .unwrap();
        assert_eq!(count(&db, "SELECT count(*) FROM plugin_versions"), 1);
        assert_eq!(count(&db, "SELECT count(*) FROM plugin_grants"), 0);

        // The new source, and the first-party rule in the schema itself.
        let insert = |id: &str, source: &str| {
            db.connection().execute(
                "INSERT INTO plugins (plugin_id, state, source, installed_at, updated_at)
                 VALUES (?1, 'disabled', ?2, 1, 1)",
                [id, source],
            )
        };
        insert("deckpilot.notifications", "bundled").unwrap();
        assert!(insert("deckpilot.github", "sideload").is_err());
        assert!(insert("deckpilot.github", "dev").is_err());
        assert!(insert("acme.bundled", "bundled").is_err());
        insert("acme.three", "dev").unwrap();
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn older_builds_refuse_a_schema_3_database_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        let key = StoreKey::from_bytes([11; 32]);
        schema_2_database(&path, &key);
        // Upgraded by this build, then closed (the WAL is checkpointed).
        let db = Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT).unwrap();
        db.connection()
            .execute(
                "INSERT INTO plugin_tombstones (plugin_id, decision, decided_at)
                 VALUES ('deckpilot.notifications', 'uninstalled', 5)",
                [],
            )
            .unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();

        // A schema-2 build refuses it exactly as it refuses any newer
        // schema, before switching the journal or migrating anything.
        for (build, supported) in [(SCHEMA_2, 2), (&MIGRATIONS[..1], 1)] {
            let result = Database::open_as(&path, &key, DEFAULT_BUSY_TIMEOUT, build);
            assert!(
                matches!(
                    result,
                    Err(StoreError::UnsupportedSchema { found: 3, supported: s }) if s == supported
                ),
                "{result:?}"
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), before, "the file changed");

        // This build still opens it, unchanged.
        let db = Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT).unwrap();
        assert_eq!(db.schema_version().unwrap(), 3);
        assert_eq!(count(&db, "SELECT count(*) FROM plugin_tombstones"), 1);
        assert_eq!(count(&db, "SELECT count(*) FROM plugins"), 2);
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn a_failed_migration_leaves_the_database_at_its_old_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.db");
        let key = StoreKey::from_bytes([12; 32]);
        schema_2_database(&path, &key);
        {
            // A row schema 3 does not allow (a sideloaded first-party id;
            // schema 2 did not check ids).
            let db = Database::open_as(&path, &key, DEFAULT_BUSY_TIMEOUT, SCHEMA_2).unwrap();
            db.connection()
                .execute(
                    "INSERT INTO plugins (plugin_id, state, source, installed_at, updated_at)
                     VALUES ('deckpilot.rogue', 'disabled', 'sideload', 1, 1)",
                    [],
                )
                .unwrap();
        }
        assert!(Database::open(&path, &key, DEFAULT_BUSY_TIMEOUT).is_err());

        // Nothing of the migration was committed.
        let db = Database::open_as(&path, &key, DEFAULT_BUSY_TIMEOUT, SCHEMA_2).unwrap();
        assert_eq!(db.schema_version().unwrap(), 2);
        assert!(!has_table(&db, "module_migrations"));
        assert!(!has_table(&db, "plugin_tombstones"));
        assert!(!has_table(&db, "plugins_v3"));
        assert_eq!(count(&db, "SELECT count(*) FROM plugins"), 3);
        assert_eq!(count(&db, "SELECT count(*) FROM plugin_versions"), 3);
        assert_eq!(count(&db, "SELECT max(version) FROM schema_migrations"), 2);
        assert_eq!(count(&db, "PRAGMA foreign_keys"), 1);
    }
}
