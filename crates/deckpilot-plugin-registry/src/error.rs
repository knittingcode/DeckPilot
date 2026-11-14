//! Errors of the registry client.

use chrono::{DateTime, Utc};
use deckpilot_plugin_manifest::{Incompatibility, PluginId};
use semver::Version;

use crate::client::StaleReason;
use crate::sig::SigError;
use crate::state::StoreError;

/// A fetch failure reported by a [`crate::RegistrySource`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The document does not exist (HTTP 404). For `keys/<n+1>.json` this
    /// ends the key set chain; anywhere else it is an error.
    #[error("not found")]
    NotFound,
    /// No answer (offline, DNS, timeout, 5xx). The client keeps its stored
    /// state and reports [`crate::Refresh::Offline`].
    #[error("unavailable: {0}")]
    Unavailable(String),
}

/// Why a registry document was refused. A refused document never changes
/// the stored state; the client keeps using what it verified before.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("{path}: {error}")]
    Fetch { path: String, error: FetchError },
    #[error("{document} is larger than {limit} bytes")]
    TooLarge { document: String, limit: usize },
    #[error("{document} is malformed: {message}")]
    Malformed { document: String, message: String },
    #[error("the embedded root keys are invalid: {0}")]
    InvalidRoots(String),
    #[error("no key set has been verified (keys/1.json is missing)")]
    NoKeyset,
    #[error("keys/{version}.json: {error}")]
    KeysetSignature { version: u64, error: SigError },
    #[error("timestamp signature: {0}")]
    TimestampSignature(SigError),
    #[error("index signature: {0}")]
    IndexSignature(SigError),
    #[error("{document} names key set {found}, the current key set is {current}")]
    KeysVersion {
        document: &'static str,
        found: u64,
        current: u64,
    },
    #[error("the timestamp expired at {expires} (freeze attack or stale mirror)")]
    TimestampExpired { expires: DateTime<Utc> },
    #[error("the timestamp is valid from {signed_at} to {expires}, longer than allowed")]
    TimestampValidity {
        signed_at: DateTime<Utc>,
        expires: DateTime<Utc>,
    },
    #[error("the index expired at {expires}")]
    IndexExpired { expires: DateTime<Utc> },
    #[error("rollback: seq {offered} is below the stored seq {stored}")]
    Rollback { stored: u64, offered: u64 },
    #[error("two different indexes were signed for seq {seq}")]
    Equivocation { seq: u64 },
    #[error("the index does not match the timestamp: {0}")]
    IndexMismatch(String),
    #[error("the timestamp names a new index; it must be supplied")]
    IndexRequired,
    #[error("the index download prefix {0:?} is not the registry's")]
    BadDownloadPrefix(String),
    #[error("the index drops a revocation of {id} (revocations are cumulative)")]
    RevocationDropped { id: PluginId },
    #[error("no verified index is available")]
    NoIndex,
    #[error("{0} is not in the registry")]
    UnknownPlugin(PluginId),
    #[error("the detail document does not match the index: {0}")]
    DetailMismatch(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Why a version may not be installed or updated to (section 12.3 steps 1
/// and 2).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    #[error("the registry has never been verified on this machine")]
    NeverVerified,
    #[error("the registry is stale ({0}); installs and updates are blocked")]
    Stale(StaleReason),
    #[error("DeckPilot {actual} is too old for this registry; update to {required} or later")]
    AppTooOld { required: Version, actual: Version },
    #[error("{0} is not in the registry")]
    UnknownPlugin(PluginId),
    #[error("{id} {version} is not in the registry")]
    UnknownVersion { id: PluginId, version: Version },
    #[error("{id} {version} was revoked: {reason}")]
    Revoked {
        id: PluginId,
        version: Version,
        reason: String,
        advisory: Option<String>,
    },
    #[error("{id} {version} was withdrawn: {reason}")]
    Yanked {
        id: PluginId,
        version: Version,
        reason: String,
    },
    #[error("{id} {version} cannot run here: {}", list(.reasons))]
    Incompatible {
        id: PluginId,
        version: Version,
        reasons: Vec<Incompatibility>,
    },
    #[error(transparent)]
    Registry(#[from] RegistryError),
}

fn list(reasons: &[Incompatibility]) -> String {
    reasons
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
