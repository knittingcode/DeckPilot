//! Encrypted configuration export and import.
//!
//! An export contains the portable configuration: profiles (with their
//! projects, actions, workflows and layouts), the portable settings of
//! feature modules as declared by each module's [`Portability`] contract, and
//! the logical resource keys the profiles use, with their kinds. It never
//! contains machine-specific values (paths, application locations), the
//! database key, IPC tokens, passwords or runtime state.
//!
//! Imports are untrusted: the archive is size-limited, authenticated and
//! validated (structure, identifiers, references, limits) before anything is
//! written, and everything is written in one transaction.
//!
//! [`Portability`]: deckpilot_core::Portability

pub mod archive;
pub mod import;
pub mod payload;

use thiserror::Error;

pub use import::{
    ConflictResolution, ImportMode, ImportOptions, ImportPreview, ImportSummary, plan, preview,
};
pub use payload::{
    AppSettings, ExportOptions, ModuleExport, PAYLOAD_VERSION, Payload, UpdateSettings, export,
};

#[derive(Debug, Error)]
pub enum PortabilityError {
    #[error("the file is not a DeckPilot configuration export")]
    NotAnArchive,
    #[error("the export uses format version {0}, which this DeckPilot cannot read")]
    UnsupportedVersion(u32),
    #[error("the password is wrong or the file was modified")]
    WrongPasswordOrDamaged,
    #[error("the file is too large")]
    TooLarge,
    #[error("encryption failed")]
    Crypto,
    #[error(transparent)]
    Kdf(#[from] deckpilot_storage::crypto::CryptoError),
    #[error("the export is invalid: {0}")]
    Invalid(String),
    #[error(transparent)]
    Storage(#[from] deckpilot_storage::StorageError),
    #[error("the export is malformed: {0}")]
    Json(#[from] serde_json::Error),
}
