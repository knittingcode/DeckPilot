//! Opening SQLite connections, and making sure SQLCipher is complete on
//! each of them.
//!
//! SQLCipher registers its `sqlcipher_export()` SQL function (used for the
//! encrypted backup before a migration, re-encryption and backups) as an
//! *auto extension* from `SQLITE_EXTRA_INIT`. SQLite runs that hook at the
//! end of the first `sqlite3_initialize()`, **after** it has marked the
//! library as initialized and released its init mutex. A second thread that
//! opens a connection in that window finds SQLite initialized, skips the
//! initialization, and gets a connection without `sqlcipher_export` —
//! permanently, since auto extensions are only applied when a connection is
//! opened. Its next backup then fails with `no such function:
//! sqlcipher_export`. (Seen in CI, where test threads open their first
//! connections at the same time.)
//!
//! Every connection is therefore opened here:
//!
//! 1. SQLite is initialized once per process, behind a lock, by opening a
//!    throwaway in-memory connection; nothing of ours opens a connection
//!    before that has returned, i.e. before the auto extension exists.
//! 2. Each new connection is checked: `PRAGMA cipher_version` must answer
//!    (otherwise plain SQLite was linked and nothing may be encrypted,
//!    backed up or migrated), and `sqlcipher_export` must be present. A
//!    connection that missed the function (opened during another, foreign
//!    initialization) is replaced by a new one; if that keeps failing the
//!    open is refused before anything is read or written.

use std::path::Path;
use std::sync::OnceLock;

use rusqlite::Connection;

use crate::{Result, StorageError};

/// How often a connection that missed `sqlcipher_export` is opened again,
/// and how long to wait in between (the window lasts microseconds).
#[cfg(feature = "sqlcipher")]
const REOPEN_ATTEMPTS: u32 = 50;
#[cfg(feature = "sqlcipher")]
const REOPEN_DELAY: std::time::Duration = std::time::Duration::from_millis(10);

/// Initializes SQLite (and SQLCipher's extra initialization) once, before
/// any connection is handed out.
fn initialize() -> Result<()> {
    static INITIALIZED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    INITIALIZED
        .get_or_init(|| {
            // `sqlite3_open` runs `sqlite3_initialize`, which returns only
            // after SQLCipher's extra initialization.
            Connection::open_in_memory()
                .map(drop)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(StorageError::Invalid)
}

/// The SQLCipher version this connection runs, `None` for plain SQLite.
#[cfg(feature = "sqlcipher")]
pub(crate) fn cipher_version(connection: &Connection) -> Result<Option<String>> {
    use rusqlite::OptionalExtension;
    Ok(connection
        .query_row("PRAGMA cipher_version", [], |row| row.get::<_, String>(0))
        .optional()?
        .filter(|version| !version.is_empty()))
}

/// Whether `sqlcipher_export()` can be called on this connection.
#[cfg(feature = "sqlcipher")]
pub(crate) fn has_export(connection: &Connection) -> Result<bool> {
    // The pragma, not the `pragma_function_list` table: it does not read
    // the schema, so it works before the key of an encrypted file is set.
    let mut statement = connection.prepare("PRAGMA function_list")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(0)? == "sqlcipher_export" {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Refuses a connection that cannot encrypt, back up or migrate.
#[cfg(feature = "sqlcipher")]
pub(crate) fn check_complete(connection: &Connection) -> Result<()> {
    if cipher_version(connection)?.is_none() {
        return Err(StorageError::SqlCipherIncomplete(
            "this build is linked against SQLite without SQLCipher".into(),
        ));
    }
    if !has_export(connection)? {
        return Err(StorageError::SqlCipherIncomplete(
            "sqlcipher_export() is not available on the connection".into(),
        ));
    }
    Ok(())
}

#[cfg(not(feature = "sqlcipher"))]
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn check_complete(_connection: &Connection) -> Result<()> {
    Ok(())
}

/// Opens a connection with `open` once SQLite is initialized, and checks
/// that SQLCipher is complete on it; see the module documentation.
fn checked(open: impl Fn() -> rusqlite::Result<Connection>) -> Result<Connection> {
    initialize()?;
    #[cfg(feature = "sqlcipher")]
    {
        let mut attempt = 1;
        loop {
            let connection = open()?;
            if cipher_version(&connection)?.is_some()
                && !has_export(&connection)?
                && attempt < REOPEN_ATTEMPTS
            {
                drop(connection);
                attempt += 1;
                std::thread::sleep(REOPEN_DELAY);
                continue;
            }
            check_complete(&connection)?;
            return Ok(connection);
        }
    }
    #[cfg(not(feature = "sqlcipher"))]
    Ok(open()?)
}

/// Opens (or creates) the database file at `path`.
pub(crate) fn open(path: &Path) -> Result<Connection> {
    checked(|| Connection::open(path))
}

/// Opens a private in-memory database.
pub(crate) fn open_in_memory() -> Result<Connection> {
    checked(Connection::open_in_memory)
}

#[cfg(all(test, feature = "sqlcipher"))]
mod tests;
