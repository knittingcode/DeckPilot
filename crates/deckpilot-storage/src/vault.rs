//! Lifecycle of the encrypted configuration database.
//!
//! The database is always encrypted with SQLCipher using a 256-bit key. How
//! that key is protected is the database's [`Protection`]:
//!
//! - **System-managed** (default): a random key kept in the operating
//!   system's credential store (`deckpilot_keystore`), unlocked by the
//!   user's login. Nothing has to be typed.
//! - **Password**: the key is derived from the user's password with
//!   Argon2id. The password is never stored; a wrong password yields a key
//!   that SQLCipher rejects, which is how it is verified.
//!
//! `deckpilot.vault.json` next to the database records the protection: the
//! Argon2id parameters (algorithm, cost, random salt) or the identifier of
//! the system key. It contains nothing secret.
//!
//! Operations that replace the database file (changing the password,
//! switching the protection, encrypting a legacy database) write a complete
//! encrypted copy with `sqlcipher_export`, verify it, and then swap files
//! with renames. They need exclusive access: the caller makes sure no other
//! connection is open. Interrupted swaps are completed or rolled back on the
//! next unlock; at no point is an unencrypted copy written. A key in the
//! credential store is never replaced or deleted automatically.

use std::fs;
use std::path::{Path, PathBuf};

use deckpilot_keystore::{KeyId, KeyStore, Secret};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::crypto::{KEY_LEN, KdfParams, Password, SecretKey, random_bytes};
use crate::sqlite::{apply_key, check_integrity, export_database, verify_copy};
use crate::{DatabaseKey, Encryption, Result, SqliteStore, StorageError};

/// File with the key protection parameters.
pub const VAULT_FILE: &str = "deckpilot.vault.json";
/// Format version of [`VAULT_FILE`]. Format 1 (0.4 development builds)
/// only knew password protection and is still read.
const VAULT_FORMAT: u32 = 2;
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// How the database key is protected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Protection {
    /// A random key in the operating system's credential store.
    SystemManaged { key_id: String },
    /// A key derived from the user's password.
    Password { kdf: KdfParams },
}

impl Protection {
    pub fn mode(&self) -> ProtectionMode {
        match self {
            Self::SystemManaged { .. } => ProtectionMode::SystemManaged,
            Self::Password { .. } => ProtectionMode::Password,
        }
    }
}

/// [`Protection`] without its parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProtectionMode {
    SystemManaged,
    Password,
}

/// Contents of [`VAULT_FILE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultFile {
    format: u32,
    protection: Protection,
}

/// Format 1: password protection only.
#[derive(Deserialize)]
struct VaultFileV1 {
    kdf: KdfParams,
}

/// What can open the database: a password the user typed, the credential
/// store, or both.
#[derive(Debug, Clone, Copy, Default)]
pub struct Credentials<'a> {
    pub password: Option<&'a Password>,
    pub keys: Option<&'a dyn KeyStore>,
}

impl<'a> Credentials<'a> {
    pub fn password(password: &'a Password) -> Self {
        Self {
            password: Some(password),
            keys: None,
        }
    }

    pub fn keys(keys: &'a dyn KeyStore) -> Self {
        Self {
            password: None,
            keys: Some(keys),
        }
    }
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

    /// Serializes changes to the files between processes (the agent and the
    /// configuration app both use the vault).
    fn lock_file(&self) -> PathBuf {
        self.database.with_file_name("deckpilot.vault.lock")
    }

    /// Takes the exclusive cross-process lock for changing the files. Held
    /// only while files are written, renamed or repaired — never while a key
    /// is derived or the credential store is asked, which may take long or
    /// show a prompt. Released when the returned file is dropped.
    fn lock(&self) -> Result<fs::File> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.lock_file())
            .map_err(io)?;
        file.lock().map_err(io)?;
        Ok(file)
    }

    /// Parameters of a first setup until its database exists.
    fn setup_file(&self) -> PathBuf {
        sibling(&self.vault_file(), ".init")
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
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io(error)),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        match value.get("format").and_then(serde_json::Value::as_u64) {
            Some(1) => {
                let file: VaultFileV1 = serde_json::from_value(value)?;
                Ok(Some(VaultFile {
                    format: 1,
                    protection: Protection::Password { kdf: file.kdf },
                }))
            }
            Some(2) => Ok(Some(serde_json::from_value(value)?)),
            other => Err(StorageError::Vault(format!(
                "unsupported key file format {}",
                other.map_or_else(|| "(none)".to_owned(), |v| v.to_string())
            ))),
        }
    }

    fn write_vault(path: &Path, protection: &Protection) -> Result<()> {
        let file = VaultFile {
            format: VAULT_FORMAT,
            protection: protection.clone(),
        };
        write_atomically(path, &serde_json::to_vec_pretty(&file)?)
    }

    /// The current protection, if the database has been set up.
    pub fn protection(&self) -> Result<Option<Protection>> {
        Ok(Self::read_vault(&self.vault_file())?.map(|file| file.protection))
    }

    /// Completes a swap that was interrupted after the replacement had been
    /// verified: the database is missing, but the replacement exists.
    fn recover_swap(&self) -> Result<()> {
        if !self.database.exists() && self.replacement().exists() {
            fs::rename(self.replacement(), &self.database).map_err(io)?;
        }
        Ok(())
    }

    /// Completes or discards a first setup that was interrupted: with the
    /// database created, its parameters are moved into place; without it,
    /// nothing was set up and the staged parameters are removed.
    fn recover_setup(&self) -> Result<()> {
        let staged = self.setup_file();
        if !staged.exists() {
            return Ok(());
        }
        let created = fs::metadata(&self.database).is_ok_and(|m| m.len() > 0);
        if created && !self.vault_file().exists() {
            fs::rename(&staged, self.vault_file()).map_err(io)
        } else {
            fs::remove_file(&staged).map_err(io)
        }
    }

    /// Whether the key file exists, i.e. a configuration was set up here.
    pub fn has_key_file(&self) -> bool {
        self.vault_file().exists()
    }

    /// Detects what is on disk, finishing an interrupted file swap or setup
    /// first. Waits while another process changes the files.
    pub fn state(&self) -> Result<DatabaseState> {
        let _lock = self.lock()?;
        self.detect()
    }

    /// [`Vault::state`] for callers that hold the lock.
    fn detect(&self) -> Result<DatabaseState> {
        self.recover_swap()?;
        self.recover_setup()?;
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
        self.require_missing()?;
        let kdf = self.new_kdf()?;
        let key = kdf.derive(password)?;
        self.create(&key, &Protection::Password { kdf })?;
        Ok(key)
    }

    /// Creates the database with `key`. The parameters are staged first and
    /// moved into place once the database exists, so an interruption never
    /// leaves a key file without a database (see [`Vault::recover_setup`]).
    fn create(&self, key: &DatabaseKey, protection: &Protection) -> Result<()> {
        let _lock = self.lock()?;
        // Checked again under the lock: another process may have set up
        // meanwhile.
        if self.detect()? != DatabaseState::Missing {
            return Err(StorageError::Vault("a database already exists".into()));
        }
        if self.has_key_file() {
            return Err(StorageError::DatabaseMissing);
        }
        Self::write_vault(&self.setup_file(), protection)?;
        let created = SqliteStore::open(&self.database, &Encryption::Key(key.clone()))
            .map(drop)
            .and_then(|()| fs::rename(self.setup_file(), self.vault_file()).map_err(io));
        if let Err(error) = created {
            let _ = fs::remove_file(self.setup_file());
            remove_database(&self.database);
            return Err(error);
        }
        Ok(())
    }

    /// Creates a new encrypted database with a random key kept in `keys`.
    /// Fails without writing anything if the store cannot hold the key; the
    /// caller must not fall back to anything weaker on its own.
    pub fn initialize_system(&self, keys: &dyn KeyStore) -> Result<DatabaseKey> {
        self.require_missing()?;
        let (key_id, key) = new_system_key(keys)?;
        let protection = Protection::SystemManaged {
            key_id: key_id.to_string(),
        };
        if let Err(error) = self.create(&key, &protection) {
            // The key was never used for anything; removing it is safe.
            let _ = keys.delete(&key_id);
            return Err(error);
        }
        Ok(key)
    }

    /// Setting up is only allowed where nothing was set up before: a key
    /// file without its database means the database went missing (moved,
    /// deleted, an unmounted drive), and silently creating a new, empty
    /// configuration would hide that. The user decides with
    /// [`Vault::discard_orphaned_key_file`].
    fn require_missing(&self) -> Result<()> {
        if self.state()? != DatabaseState::Missing {
            return Err(StorageError::Vault("a database already exists".into()));
        }
        if self.has_key_file() {
            return Err(StorageError::DatabaseMissing);
        }
        // An empty file left by an aborted start is not a database.
        remove_database(&self.database);
        Ok(())
    }

    /// Moves the key file of a missing database aside (never deletes it), so
    /// that a new configuration can be set up. Only on the user's explicit
    /// request. Returns where the file was moved.
    pub fn discard_orphaned_key_file(&self) -> Result<PathBuf> {
        let _lock = self.lock()?;
        if self.detect()? != DatabaseState::Missing || !self.has_key_file() {
            return Err(StorageError::Vault(
                "there is no key file without a database".into(),
            ));
        }
        let aside = self
            .vault_file()
            .with_file_name(format!("deckpilot.vault.orphaned-{}.json", timestamp()));
        fs::rename(self.vault_file(), &aside).map_err(io)?;
        Ok(aside)
    }

    /// Opens a raw connection with `key` and checks that the key is right.
    fn try_key(&self, key: &DatabaseKey) -> Result<()> {
        let connection = Connection::open(&self.database)?;
        apply_key(&connection, &Encryption::Key(key.clone()))
    }

    /// The key for `protection` from `credentials`, or
    /// [`StorageError::CredentialsRequired`] if they do not apply.
    fn key_for(protection: &Protection, credentials: Credentials<'_>) -> Result<DatabaseKey> {
        match protection {
            Protection::Password { kdf } => {
                let password = credentials
                    .password
                    .ok_or(StorageError::CredentialsRequired("password"))?;
                Ok(kdf.derive(password)?)
            }
            Protection::SystemManaged { key_id } => {
                let keys = credentials
                    .keys
                    .ok_or(StorageError::CredentialsRequired("system key"))?;
                let secret = keys.load(&KeyId::parse(key_id)?)?;
                key_from_secret(&secret)
            }
        }
    }

    /// Derives the key from `password` and verifies it against the database.
    pub fn unlock(&self, password: &Password) -> Result<DatabaseKey> {
        self.unlock_with(Credentials::password(password))
    }

    /// Loads the key from the credential store and verifies it.
    pub fn unlock_system(&self, keys: &dyn KeyStore) -> Result<DatabaseKey> {
        self.unlock_with(Credentials::keys(keys))
    }

    /// Opens the database with whatever `credentials` apply to its
    /// protection. Completes or rolls back an interrupted change of the
    /// password or protection.
    pub fn unlock_with(&self, credentials: Credentials<'_>) -> Result<DatabaseKey> {
        if self.state()? != DatabaseState::Encrypted {
            return Err(StorageError::Vault("there is no encrypted database".into()));
        }
        let current = Self::read_vault(&self.vault_file())?
            .ok_or_else(|| StorageError::Vault(format!("{VAULT_FILE} is missing")))?;
        let failure = match Self::key_for(&current.protection, credentials) {
            Ok(key) => match self.try_key(&key) {
                Ok(()) => {
                    // A change that did not reach the swap: discard it.
                    // A key it may have put into the credential store is
                    // left there (never delete keys automatically). A change
                    // running right now holds the lock; afterwards there is
                    // nothing left to discard.
                    let _lock = self.lock()?;
                    let _ = fs::remove_file(self.pending_vault_file());
                    remove_database(&self.replacement());
                    remove_database(&self.previous());
                    return Ok(key);
                }
                Err(error @ StorageError::InvalidKey) => error,
                Err(error) => return Err(error),
            },
            Err(error) => error,
        };
        // The swap of a change happened, but the new parameters were not
        // moved into place yet.
        if let Some(pending) = Self::read_vault(&self.pending_vault_file())?
            && let Ok(key) = Self::key_for(&pending.protection, credentials)
            && self.try_key(&key).is_ok()
        {
            let _lock = self.lock()?;
            fs::rename(self.pending_vault_file(), self.vault_file()).map_err(io)?;
            remove_database(&self.previous());
            return Ok(key);
        }
        Err(failure)
    }

    /// Changes the password: writes a copy encrypted with the new key,
    /// verifies it and swaps it in. Requires exclusive access.
    pub fn change_password(&self, current: &Password, new: &Password) -> Result<DatabaseKey> {
        let old_key = self.unlock(current)?;
        self.use_password(&old_key, new)
    }

    /// Switches to (or renews) password protection. `current` is the key
    /// that opens the database now. A system key that protected the
    /// database before stays in the credential store: encrypted backups
    /// made before the switch still need it. Requires exclusive access.
    pub fn use_password(&self, current: &DatabaseKey, password: &Password) -> Result<DatabaseKey> {
        self.try_key(current)?;
        let kdf = self.new_kdf()?;
        let new_key = kdf.derive(password)?;
        self.rekey(current, &new_key, &Protection::Password { kdf })?;
        Ok(new_key)
    }

    /// Switches to system-managed protection with a new random key. The key
    /// is stored and read back before the database is re-encrypted; if any
    /// step fails, the database and its previous protection are unchanged.
    /// Requires exclusive access.
    pub fn use_system_key(
        &self,
        current: &DatabaseKey,
        keys: &dyn KeyStore,
    ) -> Result<DatabaseKey> {
        self.try_key(current)?;
        let (key_id, new_key) = new_system_key(keys)?;
        let protection = Protection::SystemManaged {
            key_id: key_id.to_string(),
        };
        if let Err(error) = self.rekey(current, &new_key, &protection) {
            // Only remove the new key if the swap certainly did not happen.
            if !self.pending_vault_file().exists() {
                let _ = keys.delete(&key_id);
            }
            return Err(error);
        }
        Ok(new_key)
    }

    /// Re-encrypts the database from `old` to `new` and records
    /// `protection`.
    fn rekey(&self, old: &DatabaseKey, new: &DatabaseKey, protection: &Protection) -> Result<()> {
        let _lock = self.lock()?;
        let source = Connection::open(&self.database)?;
        apply_key(&source, &Encryption::Key(old.clone()))?;
        let version: u32 = source.pragma_query_value(None, "user_version", |row| row.get(0))?;
        export_database(&source, &self.replacement(), Some(new))?;
        drop(source);
        if let Err(error) = verify_copy(&self.replacement(), &Encryption::Key(new.clone()), version)
        {
            remove_database(&self.replacement());
            return Err(error);
        }
        Self::write_vault(&self.pending_vault_file(), protection)?;
        if let Err(error) = self.swap_in_replacement() {
            let _ = fs::remove_file(self.pending_vault_file());
            remove_database(&self.replacement());
            return Err(error);
        }
        fs::rename(self.pending_vault_file(), self.vault_file()).map_err(io)?;
        remove_database(&self.previous());
        Ok(())
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

    /// Encrypts a database written by an earlier, unencrypted DeckPilot with
    /// a key derived from `password`. See [`Vault::encrypt_plaintext_with`].
    pub fn encrypt_plaintext(&self, password: &Password) -> Result<(DatabaseKey, PathBuf)> {
        self.require_plaintext()?;
        let kdf = self.new_kdf()?;
        let key = kdf.derive(password)?;
        self.encrypt_plaintext_with(&key, &Protection::Password { kdf })
            .map(|legacy| (key, legacy))
    }

    /// Encrypts a legacy database with a new system-managed key.
    pub fn encrypt_plaintext_system(&self, keys: &dyn KeyStore) -> Result<(DatabaseKey, PathBuf)> {
        self.require_plaintext()?;
        let (key_id, key) = new_system_key(keys)?;
        let protection = Protection::SystemManaged {
            key_id: key_id.to_string(),
        };
        match self.encrypt_plaintext_with(&key, &protection) {
            Ok(legacy) => Ok((key, legacy)),
            Err(error) => {
                if self.state().ok() == Some(DatabaseState::Plaintext) {
                    let _ = keys.delete(&key_id);
                }
                Err(error)
            }
        }
    }

    fn require_plaintext(&self) -> Result<()> {
        if self.state()? == DatabaseState::Plaintext {
            Ok(())
        } else {
            Err(StorageError::Vault(
                "there is no unencrypted database to migrate".into(),
            ))
        }
    }

    /// The encrypted copy is written and verified (integrity, schema version
    /// and the number of rows in every table) before anything is switched.
    /// The original is kept at [`Vault::legacy_copy_path`] and is never
    /// deleted here; the user decides when to remove it. Requires exclusive
    /// access.
    fn encrypt_plaintext_with(
        &self,
        key: &DatabaseKey,
        protection: &Protection,
    ) -> Result<PathBuf> {
        let _lock = self.lock()?;
        if self.detect()? != DatabaseState::Plaintext {
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
        export_database(&source, &self.replacement(), Some(key))?;
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
        Self::write_vault(&self.vault_file(), protection)?;
        let legacy = self.legacy_copy_path();
        if legacy.exists() {
            let older =
                legacy.with_file_name(format!("deckpilot.unencrypted-backup-{}.db", timestamp()));
            fs::rename(&legacy, older).map_err(io)?;
        }
        fs::rename(&self.database, &legacy).map_err(io)?;
        move_side_files(&self.database, &legacy)?;
        fs::rename(self.replacement(), &self.database).map_err(io)?;
        Ok(legacy)
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

/// Creates a random key, stores it under a new random identifier and reads
/// it back, so that a store that silently drops writes is noticed before
/// anything depends on the key.
fn new_system_key(keys: &dyn KeyStore) -> Result<(KeyId, DatabaseKey)> {
    let mut id = [0u8; 16];
    random_bytes(&mut id)?;
    let key_id = KeyId::parse(&hex(&id))?;
    let key = SecretKey::random()?;
    keys.store(&key_id, &Secret::new(key.as_bytes().to_vec()))?;
    let stored = keys.load(&key_id).and_then(|secret| {
        key_from_secret(&secret)
            .map_err(|error| deckpilot_keystore::KeyStoreError::Failed(error.to_string()))
    });
    match stored {
        Ok(stored) if stored.matches(&key) => Ok((key_id, key)),
        Ok(_) => {
            let _ = keys.delete(&key_id);
            Err(StorageError::Vault(format!(
                "the {} returned a different key than was stored",
                keys.name()
            )))
        }
        Err(error) => {
            let _ = keys.delete(&key_id);
            Err(error.into())
        }
    }
}

fn key_from_secret(secret: &Secret) -> Result<DatabaseKey> {
    let bytes: [u8; KEY_LEN] = secret
        .expose()
        .try_into()
        .map_err(|_| StorageError::Vault("the stored key has the wrong length".into()))?;
    Ok(SecretKey::from_bytes(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
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
