//! Per-plugin data (design §2.3 items 4-6, §5.5): the key-value storage,
//! the credential secrets, the settings values, and the record of every
//! plugin the agent ever installed (`plugin_known`).
//!
//! Every row is keyed by the plugin id, and every function takes the id
//! from its caller (the agent: the plugin whose host it spawned), never
//! from plugin data, so one plugin can never name another's rows.
//!
//! Secrets are stored only in this encrypted database. They are returned
//! only by [`PluginData::credential_secret`] (for injection into a request
//! by the agent), zeroized when dropped, and never in a `Debug` output, an
//! audit record or an error message.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use deckpilot_plugin_manifest::{Action, Manifest, PluginId};
use rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::db::{Database, StoreKey};
use crate::error::{Result, StoreError};

/// Longest storage key, in characters (design §5.2).
pub const MAX_STORAGE_KEY_CHARS: usize = 128;
/// Largest storage value, as JSON (design §5.2).
pub const MAX_STORAGE_VALUE_BYTES: usize = 64 * 1024;
/// Largest credential secret (design §12.4).
pub const MAX_SECRET_BYTES: usize = 16 * 1024;

/// How much one plugin may store. The agent takes it from the broker's
/// decision for every write, so a revoked `storage` grant applies at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageQuota {
    /// Keys and JSON values, as UTF-8 bytes.
    pub bytes: u64,
    pub keys: u64,
}

/// What a plugin's storage holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StorageUsage {
    pub bytes: u64,
    pub keys: u64,
}

/// Why a storage operation was refused. Never contains a key or a value.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("a storage key is 1 to {MAX_STORAGE_KEY_CHARS} characters without control characters")]
    InvalidKey,
    #[error("a storage value is at most {MAX_STORAGE_VALUE_BYTES} bytes of JSON")]
    ValueTooLarge,
    #[error("storage-quota: the write needs {needed} bytes; the quota is {quota} bytes")]
    QuotaExceeded { needed: u64, quota: u64 },
    #[error("storage-quota: at most {max} keys")]
    TooManyKeys { max: u64 },
    #[error("the plugin is not installed")]
    NotInstalled,
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::Sqlite(error))
    }
}

/// A secret, zeroized when dropped. `Debug` never shows it.
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// The secret itself, for the moment it is injected.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Whether one credential slot has a secret, and since when. Never the
/// secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialStatus {
    pub key: String,
    pub updated_at: i64,
}

/// What an uninstall keeps (design §5.6). By default nothing: storage,
/// credentials and settings are deleted; `plugin_known` and the audit
/// always stay. Storage is never kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Retention {
    #[serde(default)]
    pub settings: bool,
    #[serde(default)]
    pub credentials: bool,
}

/// A plugin the agent installed at some point (`plugin_known`, design
/// §2.3 item 4). The table only grows: an uninstalled plugin keeps its
/// name, so instances of its actions can say whose they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownPlugin {
    pub id: PluginId,
    /// Its variable namespace, `plugins.<publisherCamel>.<nameCamel>`.
    pub namespace: String,
    pub publisher: String,
    pub display_name: String,
    /// The last version installed or staged.
    pub last_version: String,
    /// That version's actions.
    pub actions: Vec<Action>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

// ------------------------------------------------------------- storage

fn check_key(key: &str) -> Result<(), StorageError> {
    if key.is_empty()
        || key.chars().count() > MAX_STORAGE_KEY_CHARS
        || key.chars().any(char::is_control)
    {
        Err(StorageError::InvalidKey)
    } else {
        Ok(())
    }
}

fn installed(conn: &Connection, id: &PluginId) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM plugins WHERE plugin_id = ?1",
            [id.as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

pub(crate) fn storage_get(conn: &Connection, id: &PluginId, key: &str) -> Result<Option<Value>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT value FROM plugin_storage WHERE plugin_id = ?1 AND key = ?2",
            params![id.as_str(), key],
            |row| row.get(0),
        )
        .optional()?;
    text.map(|t| serde_json::from_str(&t).map_err(StoreError::record))
        .transpose()
}

pub(crate) fn storage_usage(conn: &Connection, id: &PluginId) -> Result<StorageUsage> {
    let (bytes, keys): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(length(CAST(key AS BLOB)) + length(CAST(value AS BLOB))), 0),
                count(*)
         FROM plugin_storage WHERE plugin_id = ?1",
        [id.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(StorageUsage {
        bytes: u64::try_from(bytes).unwrap_or_default(),
        keys: u64::try_from(keys).unwrap_or_default(),
    })
}

pub(crate) fn storage_keys(
    conn: &Connection,
    id: &PluginId,
    prefix: Option<&str>,
) -> Result<Vec<String>> {
    let mut statement =
        conn.prepare("SELECT key FROM plugin_storage WHERE plugin_id = ?1 ORDER BY key")?;
    let keys = statement
        .query_map([id.as_str()], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(keys
        .into_iter()
        .filter(|k| prefix.is_none_or(|p| k.starts_with(p)))
        .collect())
}

/// Writes `key` (in the caller's `IMMEDIATE` transaction) unless that
/// would exceed `quota`. A write that does not grow the storage is
/// allowed even above the quota (after a `storage` grant was revoked, a
/// plugin can still shrink what it has).
pub(crate) fn storage_set(
    conn: &Connection,
    id: &PluginId,
    key: &str,
    value: &Value,
    quota: StorageQuota,
    now: i64,
) -> Result<StorageUsage, StorageError> {
    check_key(key)?;
    let text = serde_json::to_string(value).map_err(StoreError::record)?;
    if text.len() > MAX_STORAGE_VALUE_BYTES {
        return Err(StorageError::ValueTooLarge);
    }
    if !installed(conn, id)? {
        return Err(StorageError::NotInstalled);
    }
    let existing: Option<i64> = conn
        .query_row(
            "SELECT length(CAST(key AS BLOB)) + length(CAST(value AS BLOB))
             FROM plugin_storage WHERE plugin_id = ?1 AND key = ?2",
            params![id.as_str(), key],
            |row| row.get(0),
        )
        .optional()?;
    let usage = storage_usage(conn, id)?;
    let old = existing.map_or(0, |n| u64::try_from(n).unwrap_or_default());
    let new = (key.len() + text.len()) as u64;
    let bytes = usage.bytes - old.min(usage.bytes) + new;
    let keys = usage.keys + u64::from(existing.is_none());
    if existing.is_none() && keys > quota.keys {
        return Err(StorageError::TooManyKeys { max: quota.keys });
    }
    if new > old && bytes > quota.bytes {
        return Err(StorageError::QuotaExceeded {
            needed: bytes,
            quota: quota.bytes,
        });
    }
    conn.execute(
        "INSERT INTO plugin_storage (plugin_id, key, value, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (plugin_id, key) DO UPDATE SET value = excluded.value,
             updated_at = excluded.updated_at",
        params![id.as_str(), key, text, now],
    )?;
    Ok(StorageUsage { bytes, keys })
}

pub(crate) fn storage_delete(conn: &Connection, id: &PluginId, key: &str) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM plugin_storage WHERE plugin_id = ?1 AND key = ?2",
        params![id.as_str(), key],
    )? > 0)
}

// --------------------------------------------------------- credentials

pub(crate) fn credential_secret(
    conn: &Connection,
    id: &PluginId,
    key: &str,
) -> Result<Option<Secret>> {
    let secret: Option<Zeroizing<String>> = conn
        .query_row(
            "SELECT secret FROM plugin_credentials WHERE plugin_id = ?1 AND key = ?2",
            params![id.as_str(), key],
            |row| row.get::<_, String>(0).map(Zeroizing::new),
        )
        .optional()?;
    Ok(secret.map(Secret))
}

pub(crate) fn credentials(conn: &Connection, id: &PluginId) -> Result<Vec<CredentialStatus>> {
    let mut statement = conn.prepare(
        "SELECT key, updated_at FROM plugin_credentials WHERE plugin_id = ?1 ORDER BY key",
    )?;
    let rows = statement
        .query_map([id.as_str()], |row| {
            Ok(CredentialStatus {
                key: row.get(0)?,
                updated_at: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub(crate) fn set_credential(
    conn: &Connection,
    id: &PluginId,
    key: &str,
    secret: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO plugin_credentials (plugin_id, key, secret, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (plugin_id, key) DO UPDATE SET secret = excluded.secret,
             updated_at = excluded.updated_at",
        params![id.as_str(), key, secret, now],
    )?;
    Ok(())
}

pub(crate) fn delete_credential(conn: &Connection, id: &PluginId, key: &str) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM plugin_credentials WHERE plugin_id = ?1 AND key = ?2",
        params![id.as_str(), key],
    )? > 0)
}

// ------------------------------------------------------------ settings

pub(crate) fn settings(conn: &Connection, id: &PluginId) -> Result<BTreeMap<String, Value>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT values_json FROM plugin_settings WHERE plugin_id = ?1",
            [id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    text.map_or_else(
        || Ok(BTreeMap::new()),
        |t| serde_json::from_str(&t).map_err(StoreError::record),
    )
}

pub(crate) fn set_settings(
    conn: &Connection,
    id: &PluginId,
    values: &BTreeMap<String, Value>,
    now: i64,
) -> Result<()> {
    if values.is_empty() {
        conn.execute(
            "DELETE FROM plugin_settings WHERE plugin_id = ?1",
            [id.as_str()],
        )?;
        return Ok(());
    }
    let json = serde_json::to_string(values).map_err(StoreError::record)?;
    conn.execute(
        "INSERT INTO plugin_settings (plugin_id, values_json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (plugin_id) DO UPDATE SET values_json = excluded.values_json,
             updated_at = excluded.updated_at",
        params![id.as_str(), json, now],
    )?;
    Ok(())
}

// --------------------------------------------------------------- known

/// Records (or refreshes) `manifest`'s plugin in `plugin_known`.
pub(crate) fn upsert_known(conn: &Connection, manifest: &Manifest, now: i64) -> Result<()> {
    let actions =
        serde_json::to_string(manifest.contributes.actions()).map_err(StoreError::record)?;
    conn.execute(
        "INSERT INTO plugin_known (plugin_id, namespace, publisher, display_name, last_version,
             actions_json, first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT (plugin_id) DO UPDATE SET publisher = excluded.publisher,
             display_name = excluded.display_name, last_version = excluded.last_version,
             actions_json = excluded.actions_json, last_seen_at = excluded.last_seen_at",
        params![
            manifest.id.as_str(),
            manifest.id.variable_namespace(),
            manifest.publisher.name,
            manifest.name,
            manifest.version.to_string(),
            actions,
            now
        ],
    )?;
    Ok(())
}

pub(crate) fn known(conn: &Connection) -> Result<Vec<KnownPlugin>> {
    let mut statement = conn.prepare(
        "SELECT plugin_id, namespace, publisher, display_name, last_version, actions_json,
                first_seen_at, last_seen_at
         FROM plugin_known ORDER BY plugin_id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(
            |(id, namespace, publisher, display_name, last_version, actions, first, last)| {
                Ok(KnownPlugin {
                    id: crate::store::parse_id(&id)?,
                    namespace,
                    publisher,
                    display_name,
                    last_version,
                    actions: serde_json::from_str(&actions).map_err(StoreError::record)?,
                    first_seen_at: first,
                    last_seen_at: last,
                })
            },
        )
        .collect()
}

/// Deletes what an uninstall does not keep (in the uninstall's final
/// transaction).
pub(crate) fn delete_plugin_data(conn: &Connection, id: &PluginId, keep: Retention) -> Result<()> {
    conn.execute(
        "DELETE FROM plugin_storage WHERE plugin_id = ?1",
        [id.as_str()],
    )?;
    if !keep.settings {
        conn.execute(
            "DELETE FROM plugin_settings WHERE plugin_id = ?1",
            [id.as_str()],
        )?;
    }
    if !keep.credentials {
        conn.execute(
            "DELETE FROM plugin_credentials WHERE plugin_id = ?1",
            [id.as_str()],
        )?;
    }
    Ok(())
}

// --------------------------------------------------------- PluginData

/// A second connection to the plugin database for the capability
/// executors (the agent's lane threads), next to the
/// [`PluginManager`](crate::PluginManager)'s. It reads and writes only
/// per-plugin data: storage, and the secrets and settings it hands to the
/// executor. Writes are `IMMEDIATE` transactions with the busy timeout.
#[derive(Debug)]
pub struct PluginData {
    db: Database,
    clock: fn() -> i64,
}

impl PluginData {
    pub fn open(path: &Path, key: &StoreKey, busy_timeout: Duration) -> Result<Self> {
        Ok(Self {
            db: Database::open(path, key, busy_timeout)?,
            clock: crate::system_now,
        })
    }

    pub fn storage_get(&self, id: &PluginId, key: &str) -> Result<Option<Value>, StorageError> {
        check_key(key)?;
        Ok(storage_get(self.db.connection(), id, key)?)
    }

    /// Writes `key` unless that exceeds `quota`; returns the new usage.
    pub fn storage_set(
        &mut self,
        id: &PluginId,
        key: &str,
        value: &Value,
        quota: StorageQuota,
    ) -> Result<StorageUsage, StorageError> {
        let now = (self.clock)();
        let tx = self.db.write()?;
        let usage = storage_set(&tx, id, key, value, quota, now)?;
        tx.commit()?;
        Ok(usage)
    }

    pub fn storage_delete(&mut self, id: &PluginId, key: &str) -> Result<bool, StorageError> {
        check_key(key)?;
        let tx = self.db.write()?;
        let deleted = storage_delete(&tx, id, key)?;
        tx.commit()?;
        Ok(deleted)
    }

    pub fn storage_keys(&self, id: &PluginId, prefix: Option<&str>) -> Result<Vec<String>> {
        storage_keys(self.db.connection(), id, prefix)
    }

    pub fn storage_usage(&self, id: &PluginId) -> Result<StorageUsage> {
        storage_usage(self.db.connection(), id)
    }

    /// The secret of `key`, for injection into one request.
    pub fn credential_secret(&self, id: &PluginId, key: &str) -> Result<Option<Secret>> {
        credential_secret(self.db.connection(), id, key)
    }

    /// The values the user saved (without defaults).
    pub fn settings(&self, id: &PluginId) -> Result<BTreeMap<String, Value>> {
        settings(self.db.connection(), id)
    }
}
