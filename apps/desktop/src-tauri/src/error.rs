use serde::{Serialize, Serializer};

/// Error returned to the frontend. Serialized as a plain message string.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Storage(#[from] deckpilot_storage::StorageError),
    #[error(transparent)]
    Registry(#[from] deckpilot_core::registry::RegistryError),
    #[error(transparent)]
    Engine(#[from] deckpilot_core::EngineError),
    #[error(transparent)]
    Module(#[from] deckpilot_core::ModuleError),
    #[error(transparent)]
    Catalog(#[from] deckpilot_modules::projects::CatalogError),
    #[error(transparent)]
    Profile(#[from] deckpilot_core::ProfileIssue),
    #[error(transparent)]
    ProjectContext(#[from] deckpilot_core::project::ProjectContextError),
    #[error(transparent)]
    Identifier(#[from] deckpilot_core::IdentifierError),
    #[error(transparent)]
    Agent(#[from] deckpilot_ipc::ClientError),
    #[error(transparent)]
    Updater(#[from] tauri_plugin_updater::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("{0} not found")]
    NotFound(&'static str),
}

impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
