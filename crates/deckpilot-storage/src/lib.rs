//! Local storage for DeckPilot configuration.
//!
//! Storage is split into three areas that are never mixed:
//!
//! * **Portable configuration** – profiles (actions, workflows, projects,
//!   layouts). Safe to export and move to another machine.
//! * **Machine configuration** – resource bindings for this computer.
//! * **Runtime state** – which profile and project are active, device
//!   brightness. Written by the background agent while it is running.
//!
//! Projects are stored in their own table with the profile as owner. Database
//! constraints guarantee that a project belongs to exactly one profile and
//! that the default and active project of a profile are its own projects.
//!
//! The database may be opened by the agent and the configuration app at the
//! same time; SQLite's WAL mode and a busy timeout serialize their writes.
//!
//! Secrets are not stored here yet. When they are introduced they will live in
//! the operating system's credential store or an encrypted database, never in
//! portable documents.

pub mod credentials;
pub mod crypto;
pub mod executions;
mod legacy;
pub mod policy;
mod project_actions;
pub mod provenance;
pub mod references;
mod sqlite;
#[cfg(all(test, feature = "sqlcipher"))]
mod upgrade_tests;
pub mod variables;
pub mod vault;

use deckpilot_core::{
    MachineConfig, ModuleId, ModuleSettings, ModuleStates, Profile, ProfileId, ProjectId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use sqlite::{
    ConfigChanges, DatabaseKey, Encryption, MigrationRecord, SqliteStore, backup_dir,
};
pub use variables::{SharedVariables, StoredValue, StoredVariable, VariableOrigin};

/// Version of the JSON document format written for profiles and machine
/// configuration. Bumped when the serialized shape changes incompatibly.
///
/// Version 3 no longer contains projects and the default project; they are
/// stored in their own tables. Version 4 uses the generic projects action
/// parameters (`project_id`, a resource name for `projects.resolve-resource`).
/// Version 5 marks documents that may contain the shapes of the action
/// architecture (references and templates in parameters, inline
/// invocations, graph version 2); version 4 documents decode unchanged, and
/// a DeckPilot that reads at most version 4 refuses them.
pub const DOCUMENT_VERSION: u32 = 5;

/// A change a schema migration made to user data beyond a plain format
/// upgrade, for example a project that needed a new id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationNote {
    pub schema_version: u32,
    pub profile: Option<ProfileId>,
    pub message: String,
    /// Seconds since the Unix epoch.
    pub created_at: i64,
}

/// Result of recording an application start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchInfo {
    pub previous_version: Option<String>,
    pub current_version: String,
    /// The component started with a different version than last time.
    pub first_after_update: bool,
}

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
    #[error("file system error: {0}")]
    Io(#[source] std::io::Error),
    #[error("the database failed its integrity check: {0}")]
    Corrupt(String),
    #[error("the database upgrade could not be verified ({reason}); the previous state is in the backup {}", .backup.as_deref().unwrap_or("(none)"))]
    MigrationFailed {
        reason: String,
        backup: Option<String>,
    },
    #[error("module `{module}` has schema version {found}; this build supports up to {supported}")]
    UnsupportedModuleSchema {
        module: String,
        found: u32,
        supported: u32,
    },
    #[error(transparent)]
    Crypto(#[from] crypto::CryptoError),
    #[error("{0}")]
    Vault(String),
    /// A value was refused, e.g. an empty secret.
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    KeyStore(#[from] deckpilot_keystore::KeyStoreError),
    /// The key file shows that a configuration was set up here, but its
    /// database is missing. Nothing is created until the user decides.
    #[error(
        "the configuration database is missing, although DeckPilot was set up in this folder; nothing was created"
    )]
    DatabaseMissing,
    /// The database is protected by a password (or a system key) and none
    /// was supplied.
    #[error("the {0} is needed to open the database")]
    CredentialsRequired(&'static str),
    #[error("project {0} belongs to another profile")]
    ProjectOfAnotherProfile(ProjectId),
    #[error("project key `{0}` is already used in this profile")]
    DuplicateProjectKey(String),
    #[error("there is no shared variable `{0}`")]
    UnknownVariable(String),
    /// The key is declared by a module; users cannot create or change it.
    #[error("`{key}` is a variable of the module `{module}`")]
    VariableKeyTaken { key: String, module: String },
    /// Changing the definition would make stored values invalid. They are
    /// never dropped silently.
    #[error(
        "`{key}` has {count} value(s) that the new definition does not accept; clear them first"
    )]
    VariableValuesConflict { key: String, count: usize },
    #[error(transparent)]
    Variable(#[from] deckpilot_core::variables::DefinitionIssue),
    /// The action is still used; nothing was deleted. Replace or unbind
    /// the listed uses first.
    #[error("the action is still used in {} place(s); replace or unbind those first", .0.len())]
    ActionInUse(Vec<references::ActionReference>),
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

    /// Display brightness for connected devices in percent, if configured.
    fn device_brightness(&self) -> Result<Option<u8>>;
    fn set_device_brightness(&self, percent: u8) -> Result<()>;

    /// Free-form, machine-local preferences (for example update settings).
    fn setting(&self, key: &str) -> Result<Option<String>>;
    fn set_setting(&self, key: &str, value: &str) -> Result<()>;

    /// Records that `component` started with `version` and reports whether
    /// this is the first start after a version change.
    fn record_launch(&self, component: &str, version: &str) -> Result<LaunchInfo>;

    /// Stored enabled state and configuration of feature modules.
    fn module_states(&self) -> Result<ModuleStates>;
    fn set_module_state(&self, id: &ModuleId, settings: &ModuleSettings) -> Result<()>;

    /// Changes made by schema migrations that the user may want to review.
    fn migration_notes(&self) -> Result<Vec<MigrationNote>>;
}
