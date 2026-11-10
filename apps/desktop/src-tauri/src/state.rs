use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use deckpilot_core::{ActionRegistry, Engine, MachineConfig, Os, Profile, ProfileId};
use deckpilot_device::ModelCatalog;
use deckpilot_storage::{ConfigStore, Encryption, SqliteStore, StorageError};

use crate::error::{CommandError, CommandResult};

const DATABASE_FILE: &str = "deckpilot.db";

/// Shared application state.
#[derive(Debug)]
pub struct AppState {
    store: Mutex<SqliteStore>,
    pub engine: Engine,
    pub catalog: ModelCatalog,
    pub database_path: PathBuf,
}

impl AppState {
    pub fn open(data_dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(data_dir)?;
        let database_path = data_dir.join(DATABASE_FILE);
        let store = SqliteStore::open(&database_path, &Encryption::None)?;
        Ok(Self {
            store: Mutex::new(store),
            engine: Engine::new(ActionRegistry::with_builtins()),
            catalog: ModelCatalog::builtin(),
            database_path,
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
