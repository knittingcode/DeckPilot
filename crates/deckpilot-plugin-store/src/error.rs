//! Errors of the plugin store and its lifecycle operations.

use std::path::Path;

use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_package::PackageError;

use crate::failpoint::{Failpoint, Operation};
use crate::grants::GrantKey;
use crate::state::PluginState;

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The key does not open the file (or the file is not a database).
    #[error("the plugin database key is wrong, or the file is not a plugin database")]
    InvalidKey,
    /// This build links plain SQLite; nothing may be stored unencrypted.
    #[error("this build has no SQLCipher, so the plugin database cannot be opened")]
    EncryptionUnavailable,
    #[error("the plugin database is damaged: {0}")]
    Corrupt(String),
    #[error("the plugin database has schema {found}; this build supports up to {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
    /// The path names the configuration database, which this crate never
    /// opens.
    #[error("{0} is the configuration database, not the plugin database")]
    ConfigDatabase(String),
    /// Another manager holds the plugin directory.
    #[error("the plugin directory {0} is in use by another process")]
    Locked(String),
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{path}: {message}")]
    Io { path: String, message: String },
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("a stored record is invalid: {0}")]
    InvalidRecord(String),

    #[error("plugin {0} is not installed")]
    NotInstalled(PluginId),
    #[error("plugin {id} is already installed (version {version})")]
    AlreadyInstalled { id: PluginId, version: String },
    #[error("plugin {id} cannot go from {from} to {to}")]
    IllegalTransition {
        id: PluginId,
        from: PluginState,
        to: PluginState,
    },
    #[error("plugin {id} has an unfinished {op} operation; run recovery first")]
    OperationInProgress { id: PluginId, op: Operation },
    #[error("plugin {id} needs consent for: {}", list(missing))]
    ConsentRequired {
        id: PluginId,
        missing: Vec<GrantKey>,
    },
    #[error("plugin {id} does not declare the permission {key}")]
    UnknownGrant { id: PluginId, key: GrantKey },
    #[error("plugin {0} has no previous version to roll back to")]
    NoPreviousVersion(PluginId),
    #[error("plugin {0} has no update waiting for approval")]
    NoStagedUpdate(PluginId),
    #[error("plugin {id} already has version {version} waiting for approval")]
    UpdatePending { id: PluginId, version: String },
    #[error("plugin {id} version {version} is already installed")]
    VersionInstalled { id: PluginId, version: String },
    #[error("plugin {id} version {version} is not newer than the active version {active}")]
    NotNewer {
        id: PluginId,
        version: String,
        active: String,
    },
    #[error("plugin {id} version {version} is revoked: {reason}")]
    Revoked {
        id: PluginId,
        version: String,
        reason: String,
    },
    #[error("plugin {id} version {version} failed its integrity check: {problem}")]
    Integrity {
        id: PluginId,
        version: String,
        problem: String,
    },
    #[error("the secret cleanup of plugin {id} failed: {message}")]
    Hook { id: PluginId, message: String },
    /// An injected crash (tests): the operation stopped exactly here, with
    /// no clean-up, as if the process had died.
    #[error("simulated crash during {op} at {point:?}")]
    Crashed { op: Operation, point: Failpoint },
}

impl StoreError {
    pub(crate) fn io(path: &Path, error: &std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        }
    }

    pub(crate) fn record(message: impl std::fmt::Display) -> Self {
        Self::InvalidRecord(message.to_string())
    }

    /// Whether this is an injected crash (nothing may clean up after it).
    pub fn is_crash(&self) -> bool {
        matches!(self, Self::Crashed { .. })
    }
}

fn list(keys: &[GrantKey]) -> String {
    keys.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
