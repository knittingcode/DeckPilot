//! Local storage for DeckPilot configuration.
//!
//! Storage is split into three areas that are never mixed:
//!
//! * **Portable configuration** – profiles (actions, workflows, projects,
//!   layouts). Safe to export and move to another machine.
//! * **Machine configuration** – resource bindings for this computer.
//! * **Runtime state** – which profile and project are active.
//!
//! Secrets are not stored here yet. When they are introduced they will live in
//! the operating system's credential store or an encrypted database, never in
//! portable documents.

mod sqlite;

use deckpilot_core::{MachineConfig, Profile, ProfileId, ProjectId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use sqlite::{DatabaseKey, Encryption, SqliteStore};

/// Version of the JSON document format written for profiles and machine
/// configuration. Bumped when the serialized shape changes incompatibly.
pub const DOCUMENT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("stored document is invalid: {0}")]
    Document(#[from] serde_json::Error),
    #[error("database schema version {found} is newer than supported version {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("document version {0} is not supported")]
    UnsupportedDocument(u32),
    #[error("database encryption is not available in this build")]
    EncryptionUnavailable,
    #[error("the database could not be opened with the provided key")]
    InvalidKey,
    #[error("profile does not exist")]
    UnknownProfile,
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Lightweight listing entry for a stored profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: ProfileId,
    pub name: String,
    /// Seconds since the Unix epoch.
    pub updated_at: i64,
}

/// Persistence interface used by the application.
pub trait ConfigStore {
    fn list_profiles(&self) -> Result<Vec<ProfileSummary>>;
    fn load_profile(&self, id: ProfileId) -> Result<Option<Profile>>;
    /// Inserts or replaces a profile.
    fn save_profile(&self, profile: &Profile) -> Result<()>;
    /// Deletes a profile together with its runtime state. Returns whether it existed.
    fn delete_profile(&self, id: ProfileId) -> Result<bool>;

    fn load_machine_config(&self) -> Result<Option<MachineConfig>>;
    fn save_machine_config(&self, config: &MachineConfig) -> Result<()>;

    fn active_profile(&self) -> Result<Option<ProfileId>>;
    fn set_active_profile(&self, id: Option<ProfileId>) -> Result<()>;
    fn active_project(&self, profile: ProfileId) -> Result<Option<ProjectId>>;
    fn set_active_project(&self, profile: ProfileId, project: Option<ProjectId>) -> Result<()>;
}
