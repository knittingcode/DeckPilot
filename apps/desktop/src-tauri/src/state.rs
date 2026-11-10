use std::sync::{Mutex, MutexGuard, PoisonError};

use deckpilot_core::{ActionRegistry, MachineConfig, Os, Profile, ProfileId};
use deckpilot_device::ModelCatalog;
use deckpilot_ipc::AgentPaths;
use deckpilot_storage::{ConfigStore, Encryption, SqliteStore, StorageError};

use crate::error::{CommandError, CommandResult};

/// Shared application state.
///
/// The configuration app writes configuration documents (profiles, machine
/// bindings). Runtime state (active profile and project, brightness) belongs
/// to the background agent while it runs and is changed through IPC.
#[derive(Debug)]
pub struct AppState {
    store: Mutex<SqliteStore>,
    /// Validates action parameters; actions are executed by the agent.
    pub registry: ActionRegistry,
    pub catalog: ModelCatalog,
    pub paths: AgentPaths,
}

impl AppState {
    pub fn open(paths: AgentPaths) -> Result<Self, Box<dyn std::error::Error>> {
        paths.create_dirs()?;
        let store = SqliteStore::open(&paths.database, &Encryption::None)?;
        Ok(Self {
            store: Mutex::new(store),
            registry: ActionRegistry::with_builtins(),
            catalog: ModelCatalog::builtin(),
            paths,
        })
    }

    pub fn store(&self) -> MutexGuard<'_, SqliteStore> {
        // A poisoned lock only means another command panicked; SQLite itself
        // keeps the data consistent, so continue with the inner value.
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub fn load_profile(store: &SqliteStore, id: ProfileId) -> CommandResult<Profile> {
    store
        .load_profile(id)?
        .ok_or(CommandError::NotFound("profile"))
}

/// Loads this machine's configuration, creating it on first use.
pub fn machine_config(store: &SqliteStore) -> Result<MachineConfig, StorageError> {
    if let Some(config) = store.load_machine_config()? {
        return Ok(config);
    }
    let config = MachineConfig::new(Os::current());
    store.save_machine_config(&config)?;
    Ok(config)
}
