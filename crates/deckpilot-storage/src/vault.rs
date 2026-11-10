//! Lifecycle of the encrypted configuration database.
//!
//! The database is encrypted with SQLCipher. Its 256-bit key is derived from
//! the user's password with Argon2id; the parameters (algorithm, cost and a
//! random salt — nothing secret) are stored next to the database in
//! `deckpilot.vault.json`. The password is never stored. A wrong password
//! yields a key that SQLCipher rejects, which is how passwords are verified;
//! nothing is written in that case.
//!
//! Operations that replace the database file (changing the password,
//! encrypting a legacy database) write a complete encrypted copy with
//! `sqlcipher_export`, verify it, and then swap files with renames. They need
//! exclusive access: the caller makes sure no other connection is open.
//! Interrupted swaps are completed or rolled back on the next unlock; at no
//! point is an unencrypted copy written.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::crypto::{KdfParams, Password};
use crate::sqlite::{apply_key, check_integrity, export_database, verify_copy};
use crate::{DatabaseKey, Encryption, Result, SqliteStore, StorageError};

/// File with the key derivation parameters.
pub const VAULT_FILE: &str = "deckpilot.vault.json";
/// Format version of [`VAULT_FILE`].
const VAULT_FORMAT: u32 = 1;
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// Contents of [`VAULT_FILE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultFile {
    format: u32,
    kdf: KdfParams,
}

/// What is on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DatabaseState {
    /// No database yet: a password must be set up.
    Missing,
    /// An unencrypted database from an earlier DeckPilot.
    Plaintext,
    /// An encrypted database.
    Encrypted,
}

/// The encrypted database of one data directory.
#[derive(Debug, Clone)]
pub struct Vault {
    database: PathBuf,
    /// Cost parameters for new keys; production uses the defaults.
    kdf_cost: Option<(u32, u32)>,
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn io(error: std::io::Error) -> StorageError {
    StorageError::Io(error)
}

/// Replaces `path` atomically with `contents` (write, sync, rename).
fn write_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = sibling(path, ".tmp");
    {
        use std::io::Write as _;
        let mut file = fs::File::create(&temporary).map_err(io)?;
        file.write_all(contents).map_err(io)?;
        file.sync_all().map_err(io)?;
    }
    fs::rename(&temporary, path).map_err(io)
}

/// Moves the SQLite side files (`-wal`, `-shm`) of `from` next to `to`.
fn move_side_files(from: &Path, to: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm"] {
        let side = sibling(from, suffix);
        if side.exists() {
            fs::rename(&side, sibling(to, suffix)).map_err(io)?;
        }
    }
    Ok(())
}

fn remove_side_files(path: &Path) {
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(sibling(path, suffix));
    }
}

fn remove_database(path: &Path) {
    let _ = fs::remove_file(path);
    remove_side_files(path);
}

fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl Vault {
    /// The vault for the database file `database`.
    pub fn new(database: impl Into<PathBuf>) -> Self {
        Self {
            database: database.into(),
            kdf_cost: None,
        }
    }

    /// A vault that derives new keys with the cheapest accepted cost.
    /// Only for tests.
    #[doc(hidden)]
    #[must_use]
    pub fn with_test_cost(mut self) -> Self {
        self.kdf_cost = Some((8 * 1024, 1));
        self
    }

    pub fn database(&self) -> &Path {
        &self.database
    }

    fn vault_file(&self) -> PathBuf {
        self.database.with_file_name(VAULT_FILE)
    }

    /// Parameters of a password change in progress.
    fn pending_vault_file(&self) -> PathBuf {
        sibling(&self.vault_file(), ".next")
    }

    /// Encrypted replacement being prepared.
    fn replacement(&self) -> PathBuf {
        sibling(&self.database, ".new")
    }

    /// The previous database while a replacement is swapped in.
    fn previous(&self) -> PathBuf {
        sibling(&self.database, ".previous")
    }

    /// Where an encrypted legacy database's original is kept.
    pub fn legacy_copy_path(&self) -> PathBuf {
        self.database
            .with_file_name("deckpilot.unencrypted-backup.db")
    }

    fn new_kdf(&self) -> Result<KdfParams> {
        Ok(match self.kdf_cost {
            Some((memory, iterations)) => KdfParams::generate_with(memory, iterations)?,
            None => KdfParams::generate()?,
        })
    }

    fn read_vault(path: &Path) -> Result<Option<VaultFile>> {
        match fs::read(path) {
            Ok(bytes) => {
                let file: VaultFile = serde_json::from_slice(&bytes)?;
                if file.format != VAULT_FORMAT {
                    return Err(StorageError::Vault(format!(
                        "unsupported key file format {}",
                        file.format
                    )));
                }
                Ok(Some(file))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io(error)),
        }
    }

    fn write_vault(path: &Path, kdf: &KdfParams) -> Result<()> {
        let file = VaultFile {
            format: VAULT_FORMAT,
            kdf: kdf.clone(),
        };
        write_atomically(path, &serde_json::to_vec_pretty(&file)?)
    }

    /// Completes a swap that was interrupted after the replacement had been
    /// verified: the database is missing, but the replacement exists.
    fn recover_swap(&self) -> Result<()> {
        if !self.database.exists() && self.replacement().exists() {
            fs::rename(self.replacement(), &self.database).map_err(io)?;
        }
        Ok(())
    }

    /// Detects what is on disk, finishing an interrupted file swap first.
    pub fn state(&self) -> Result<DatabaseState> {
        self.recover_swap()?;
        let mut header = [0u8; 16];
        match fs::File::open(&self.database) {
            Ok(mut file) => {
                use std::io::Read as _;
                let read = file.read(&mut header).map_err(io)?;
                if read == 0 {
                    Ok(DatabaseState::Missing)
                } else if read == header.len() && &header == SQLITE_HEADER {
                    Ok(DatabaseState::Plaintext)
                } else {
                    Ok(DatabaseState::Encrypted)
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(DatabaseState::Missing)
            }
            Err(error) => Err(io(error)),
        }
    }

    /// Creates a new encrypted database protected by `password`.
    pub fn initialize(&self, password: &Password) -> Result<DatabaseKey> {
        if self.state()? != DatabaseState::Missing {
            return Err(StorageError::Vault("a database already exists".into()));
        }
        // An empty file left by an aborted start is not a database.
        remove_database(&self.database);
        let kdf = self.new_kdf()?;
        let key = kdf.derive(password)?;
        // Parameters first: a crash before the database exists leaves the
        // state `Missing`, and the next setup simply overwrites them.
        Self::write_vault(&self.vault_file(), &kdf)?;
        SqliteStore::open(&self.database, &Encryption::Key(key.clone()))?;
        Ok(key)
    }

    /// Opens a raw connection with `key` and checks that the key is right.
    fn try_key(&self, key: &DatabaseKey) -> Result<()> {
        let connection = Connection::open(&self.database)?;
        apply_key(&connection, &Encryption::Key(key.clone()))
    }

    /// Derives the key from `password` and verifies it against the database.
    /// Completes or rolls back an interrupted password change.
    pub fn unlock(&self, password: &Password) -> Result<DatabaseKey> {
        if self.state()? != DatabaseState::Encrypted {
            return Err(StorageError::Vault("there is no encrypted database".into()));
        }
        let current = Self::read_vault(&self.vault_file())?
            .ok_or_else(|| StorageError::Vault(format!("{VAULT_FILE} is missing")))?;
        let key = current.kdf.derive(password)?;
        match self.try_key(&key) {
            Ok(()) => {
                // A password change that did not reach the swap: discard it.
                let _ = fs::remove_file(self.pending_vault_file());
                remove_database(&self.replacement());
                remove_database(&self.previous());
                Ok(key)
            }
            Err(StorageError::InvalidKey) => {
                // The swap of a password change happened, but the new
                // parameters were not moved into place yet.
                let Some(pending) = Self::read_vault(&self.pending_vault_file())? else {
                    return Err(StorageError::InvalidKey);
                };
                let key = pending.kdf.derive(password)?;
                self.try_key(&key)?;
                fs::rename(self.pending_vault_file(), self.vault_file()).map_err(io)?;
                remove_database(&self.previous());
                Ok(key)
            }
            Err(error) => Err(error),
        }
    }

    /// Changes the password: writes a copy encrypted with the new key,
    /// verifies it and swaps it in. Requires exclusive access.
    pub fn change_password(&self, current: &Password, new: &Password) -> Result<DatabaseKey> {
        let old_key = self.unlock(current)?;
        let kdf = self.new_kdf()?;
        let new_key = kdf.derive(new)?;
        let source = Connection::open(&self.database)?;
        apply_key(&source, &Encryption::Key(old_key))?;
        let version: u32 = source.pragma_query_value(None, "user_version", |row| row.get(0))?;
        export_database(&source, &self.replacement(), Some(&new_key))?;
        drop(source);
        if let Err(error) = verify_copy(
            &self.replacement(),
            &Encryption::Key(new_key.clone()),
            version,
        ) {
            remove_database(&self.replacement());
            return Err(error);
        }
        Self::write_vault(&self.pending_vault_file(), &kdf)?;
        self.swap_in_replacement()?;
        fs::rename(self.pending_vault_file(), self.vault_file()).map_err(io)?;
        remove_database(&self.previous());
        Ok(new_key)
    }

    /// Moves the verified replacement into place, keeping the current
    /// database as `.previous` until the caller has finished.
    fn swap_in_replacement(&self) -> Result<()> {
        remove_database(&self.previous());
        fs::rename(&self.database, self.previous()).map_err(io)?;
        // Side files belong to the old database; never let them be applied
        // to the new one.
        move_side_files(&self.database, &self.previous())?;
        fs::rename(self.replacement(), &self.database).map_err(io)
    }

    /// Encrypts a database written by an earlier, unencrypted DeckPilot.
    ///
    /// The encrypted copy is written and verified (integrity, schema version
    /// and the number of rows in every table) before anything is switched.
    /// The original is kept at [`Vault::legacy_copy_path`] and is never
    /// deleted here; the user decides when to remove it. Requires exclusive
    /// access.
    pub fn encrypt_plaintext(&self, password: &Password) -> Result<(DatabaseKey, PathBuf)> {
        if self.state()? != DatabaseState::Plaintext {
            return Err(StorageError::Vault(
                "there is no unencrypted database to migrate".into(),
            ));
        }
        let source = Connection::open(&self.database)?;
        check_integrity(&source)?;
        // Fold the write-ahead log into the main file so that nothing is left
        // behind in side files.
        source.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        let version: u32 = source.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let counts = row_counts(&source)?;
        let kdf = self.new_kdf()?;
        let key = kdf.derive(password)?;
        export_database(&source, &self.replacement(), Some(&key))?;
        drop(source);

        let verified = verify_copy(&self.replacement(), &Encryption::Key(key.clone()), version)
            .and_then(|()| {
                let copy = Connection::open(self.replacement())?;
                apply_key(&copy, &Encryption::Key(key.clone()))?;
                if row_counts(&copy)? == counts {
                    Ok(())
                } else {
                    Err(StorageError::Corrupt(
                        "the encrypted copy does not contain all rows".into(),
                    ))
                }
            });
        if let Err(error) = verified {
            remove_database(&self.replacement());
            return Err(error);
        }

        // The parameters can be written now: the state is decided by the
        // database header, which is still plain until the swap.
        Self::write_vault(&self.vault_file(), &kdf)?;
        let legacy = self.legacy_copy_path();
        if legacy.exists() {
            let older =
                legacy.with_file_name(format!("deckpilot.unencrypted-backup-{}.db", timestamp()));
            fs::rename(&legacy, older).map_err(io)?;
        }
        fs::rename(&self.database, &legacy).map_err(io)?;
        move_side_files(&self.database, &legacy)?;
        fs::rename(self.replacement(), &self.database).map_err(io)?;
        Ok((key, legacy))
    }

    /// Unencrypted copies left by [`Vault::encrypt_plaintext`].
    pub fn legacy_copies(&self) -> Vec<PathBuf> {
        let Some(dir) = self.database.parent() else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut copies: Vec<PathBuf> = entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with("deckpilot.unencrypted-backup")
                            && std::path::Path::new(name)
                                .extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("db"))
                    })
            })
            .collect();
        copies.sort();
        copies
    }

    /// Removes the unencrypted copies through the file system. This does not
    /// guarantee that the data is unrecoverable, especially on SSDs; see the
    /// documentation for a secure cleanup procedure.
    pub fn remove_legacy_copies(&self) -> Result<usize> {
        let copies = self.legacy_copies();
        for copy in &copies {
            fs::remove_file(copy).map_err(io)?;
            remove_side_files(copy);
        }
        Ok(copies.len())
    }
}

/// Rows per table, to compare a copy with its source.
fn row_counts(connection: &Connection) -> Result<Vec<(String, i64)>> {
    let tables: Vec<String> = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut counts = Vec::new();
    for table in tables {
        let count: i64 =
            connection.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| {
                row.get(0)
            })?;
        counts.push((table, count));
    }
    Ok(counts)
}

#[cfg(test)]
mod tests;
