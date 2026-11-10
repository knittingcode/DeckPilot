use std::ops::Deref;
use std::sync::{Mutex, MutexGuard, PoisonError};

use deckpilot_core::{
    ActionRegistry, MachineConfig, ModuleId, ModuleRegistry, Os, Profile, ProfileId,
};
use deckpilot_device::ModelCatalog;
use deckpilot_ipc::AgentPaths;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{
    ConfigStore, DatabaseKey, Encryption, LaunchInfo, SqliteStore, StorageError,
};

use crate::error::{CommandError, CommandResult};

/// Setting with the app version for which the password was last entered.
/// After an update the password is asked for once, then not again.
const CONFIRMED_VERSION: &str = "security.confirmedVersion";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The open database while the configuration is unlocked.
#[derive(Debug)]
struct Session {
    store: SqliteStore,
    key: DatabaseKey,
    /// The password must be confirmed once after an update before the
    /// configuration can be used.
    confirmation_pending: bool,
}

/// Shared application state.
///
/// The configuration database is encrypted. Until it is unlocked (by the
/// user's password, or with the key of an already unlocked agent) no
/// configuration command works. Runtime state (active profile and project,
/// brightness) belongs to the background agent while it runs and is changed
/// through IPC.
#[derive(Debug)]
pub struct AppState {
    session: Mutex<Option<Session>>,
    /// Feature modules with their stored state. Action parameters are
    /// validated against the enabled modules; actions run in the agent.
    modules: Mutex<ModuleRegistry>,
    pub catalog: ModelCatalog,
    pub paths: AgentPaths,
    pub vault: Vault,
    /// This start compared with the previous one, once unlocked.
    launch: Mutex<Option<LaunchInfo>>,
}

/// Access to the open store; derefs to [`SqliteStore`].
pub struct StoreGuard<'a>(MutexGuard<'a, Option<Session>>);

impl Deref for StoreGuard<'_> {
    type Target = SqliteStore;

    fn deref(&self) -> &SqliteStore {
        // Only constructed when a usable session exists.
        &self
            .0
            .as_ref()
            .expect("checked when the guard was created")
            .store
    }
}

impl AppState {
    /// Prepares the state without opening the database: that needs the key.
    pub fn open(paths: AgentPaths) -> Result<Self, Box<dyn std::error::Error>> {
        paths.create_dirs()?;
        Ok(Self {
            session: Mutex::new(None),
            modules: Mutex::new(deckpilot_modules::builtin(Os::current())),
            catalog: ModelCatalog::builtin(),
            vault: Vault::new(&paths.database),
            paths,
            launch: Mutex::new(None),
        })
    }

    fn session(&self) -> MutexGuard<'_, Option<Session>> {
        // A poisoned lock only means another command panicked; SQLite itself
        // keeps the data consistent, so continue with the inner value.
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The open store, or [`CommandError::Locked`].
    pub fn store(&self) -> CommandResult<StoreGuard<'_>> {
        let session = self.session();
        match session.as_ref() {
            Some(open) if !open.confirmation_pending => Ok(StoreGuard(session)),
            Some(_) => Err(CommandError::ConfirmationRequired),
            None => Err(CommandError::Locked),
        }
    }

    pub fn is_unlocked(&self) -> bool {
        self.session().is_some()
    }

    pub fn confirmation_pending(&self) -> bool {
        self.session()
            .as_ref()
            .is_some_and(|session| session.confirmation_pending)
    }

    /// The key of the open database.
    pub fn key(&self) -> Option<DatabaseKey> {
        self.session().as_ref().map(|session| session.key.clone())
    }

    /// Opens the database with `key`. `password_entered`: the user just
    /// typed the password, which counts as the confirmation after an update.
    pub fn attach(&self, key: DatabaseKey, password_entered: bool) -> CommandResult<()> {
        let store = SqliteStore::open(&self.paths.database, &Encryption::Key(key.clone()))?;
        let (modules, problems) = deckpilot_modules::configured(&store.module_states()?);
        for problem in problems {
            eprintln!("stored module state was not applied: {problem}");
        }
        store.apply_module_schemas(&modules.schemas())?;
        let launch = store.record_launch("desktop", APP_VERSION).ok();
        let confirmed = store.setting(CONFIRMED_VERSION)?;
        let confirmation_pending = if password_entered {
            store.set_setting(CONFIRMED_VERSION, APP_VERSION)?;
            false
        } else {
            confirmed.as_deref() != Some(APP_VERSION)
        };
        *self.modules() = modules;
        *self.launch.lock().unwrap_or_else(PoisonError::into_inner) = launch;
        *self.session() = Some(Session {
            store,
            key,
            confirmation_pending,
        });
        Ok(())
    }

    /// Records that the password was confirmed for this version.
    pub fn confirm(&self) -> CommandResult<()> {
        let mut session = self.session();
        let open = session.as_mut().ok_or(CommandError::Locked)?;
        open.store.set_setting(CONFIRMED_VERSION, APP_VERSION)?;
        open.confirmation_pending = false;
        Ok(())
    }

    /// Closes the database and forgets the key.
    pub fn detach(&self) {
        *self.session() = None;
    }

    pub fn launch(&self) -> Option<LaunchInfo> {
        self.launch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
