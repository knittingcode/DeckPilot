use std::sync::{Mutex, MutexGuard, PoisonError};

use deckpilot_core::{
    ActionRegistry, MachineConfig, ModuleId, ModuleRegistry, Os, Profile, ProfileId,
};
use deckpilot_device::ModelCatalog;
use deckpilot_ipc::AgentPaths;
use deckpilot_storage::{ConfigStore, Encryption, LaunchInfo, SqliteStore, StorageError};

use crate::error::{CommandError, CommandResult};

/// Shared application state.
///
/// The configuration app writes configuration documents (profiles, machine
/// bindings). Runtime state (active profile and project, brightness) belongs
/// to the background agent while it runs and is changed through IPC.
#[derive(Debug)]
pub struct AppState {
    store: Mutex<SqliteStore>,
    /// Feature modules with their stored state. Action parameters are
    /// validated against the enabled modules; actions run in the agent.
    modules: Mutex<ModuleRegistry>,
    pub catalog: ModelCatalog,
    pub paths: AgentPaths,
    /// This start compared with the previous one.
    pub launch: Option<LaunchInfo>,
}

impl AppState {
    pub fn open(paths: AgentPaths) -> Result<Self, Box<dyn std::error::Error>> {
        paths.create_dirs()?;
        let store = SqliteStore::open(&paths.database, &Encryption::None)?;
        let launch = store
            .record_launch("desktop", env!("CARGO_PKG_VERSION"))
            .ok();
        let (modules, problems) = deckpilot_modules::configured(&store.module_states()?);
        for problem in problems {
            eprintln!("stored module state was not applied: {problem}");
        }
        Ok(Self {
            store: Mutex::new(store),
            modules: Mutex::new(modules),
            catalog: ModelCatalog::builtin(),
            paths,
            launch,
        })
    }

    pub fn modules(&self) -> MutexGuard<'_, ModuleRegistry> {
        self.modules.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The action types of the enabled modules.
    pub fn actions(&self) -> ActionRegistry {
        self.modules().action_registry()
    }

    /// Fails unless the module is enabled. Used by commands that belong to a
    /// feature module.
    pub fn require_module(&self, id: &ModuleId) -> CommandResult<()> {
        let modules = self.modules();
        if modules.is_enabled(id) {
            return Ok(());
        }
        let name = modules
            .manifest(id)
            .map_or_else(|| id.to_string(), |m| m.name.clone());
        Err(CommandError::Invalid(format!(
            "the {name} module is disabled"
        )))
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
