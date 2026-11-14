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
    u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX)
}

/// An open, keyed and migrated plugin database connection.
#[derive(Debug)]
pub struct Database {
    connection: Connection,
    path: PathBuf,
}

impl Database {
    pub fn open(path: &Path, key: &StoreKey, busy_timeout: Duration) -> Result<Self> {
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
        let supported = supported_schema_version();
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
        database.migrate()?;
        Ok(database)
    }

    pub fn path(&self) -> &Path {
        &self.path
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

    fn migrate(&mut self) -> Result<()> {
        let tx = self.write()?;
        let current = user_version(&tx)?;
        let supported = supported_schema_version();
        if current > supported {
            return Err(StoreError::UnsupportedSchema {
                found: current,
                supported,
            });
        }
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
            let version = u32::try_from(index + 1).unwrap_or(u32::MAX);
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![version, crate::system_now()],
            )?;
            tx.pragma_update(None, "user_version", version)?;
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
        assert_eq!(applied, 1);
    }
}
