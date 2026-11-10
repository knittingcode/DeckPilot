use std::ops::Deref;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionRegistry, MachineConfig, ModuleId, ModuleRegistry, Os, Profile, ProfileId,
};
use deckpilot_device::ModelCatalog;
use deckpilot_ipc::{AgentPaths, AgentStatus, LockReason, ProtectionMode, SecurityState};
use deckpilot_presence::PresenceCheck;
use deckpilot_storage::policy::{Reauthentication, security_policy};
use deckpilot_storage::vault::{self, Vault};
use deckpilot_storage::{
    ConfigStore, DatabaseKey, Encryption, LaunchInfo, SqliteStore, StorageError,
};

use crate::error::{CommandError, CommandResult};

/// Setting with the app version for which the user last authenticated.
/// With the "after updates" policy, authentication is asked for once after
/// an update, then not again.
const CONFIRMED_VERSION: &str = "security.confirmedVersion";
/// How long an authentication allows one sensitive operation.
const AUTHORIZATION_WINDOW: Duration = Duration::from_secs(120);
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The open database while the configuration is unlocked.
#[derive(Debug)]
struct Session {
    store: SqliteStore,
    key: DatabaseKey,
    /// The user must authenticate (after an update or at launch, as the
    /// security policy says) before the configuration can be used.
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
    /// Operating-system user verification (Touch ID, login password).
    pub presence: Arc<dyn PresenceCheck>,
    /// When the user last authenticated for a sensitive operation.
    authorized_at: Mutex<Option<Instant>>,
}

/// What the agent reported about the database's protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentSecurity {
    pub security: SecurityState,
    pub protection: Option<ProtectionMode>,
    pub lock_reason: Option<LockReason>,
}

impl AgentSecurity {
    pub fn of(status: &AgentStatus) -> Self {
        Self {
            security: status.security,
            protection: status.protection,
            lock_reason: status.lock_reason,
        }
    }
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
        Self::with_presence(paths, Arc::from(deckpilot_presence::platform()))
    }

    pub fn with_presence(
        paths: AgentPaths,
        presence: Arc<dyn PresenceCheck>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        paths.create_dirs()?;
        Ok(Self {
            session: Mutex::new(None),
            modules: Mutex::new(deckpilot_modules::builtin(Os::current())),
            catalog: ModelCatalog::builtin(),
            vault: Vault::new(&paths.database),
            paths,
            launch: Mutex::new(None),
            presence,
            authorized_at: Mutex::new(None),
        })
    }

    /// How the database key is protected, from the vault file.
    pub fn protection(&self) -> Option<vault::ProtectionMode> {
        self.vault
            .protection()
            .ok()
            .flatten()
            .map(|protection| protection.mode())
    }

    /// Whether the user can be asked to prove they are present: with their
    /// DeckPilot password, or with the operating system's verification.
    pub fn can_verify_user(&self) -> bool {
        self.protection() == Some(vault::ProtectionMode::Password)
            || self.presence.method().is_some()
    }

    /// Records a successful authentication for the next sensitive operation.
    pub fn authorize(&self) {
        *self
            .authorized_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    /// Uses up a recent authentication, or fails with
    /// [`CommandError::AuthenticationRequired`].
    pub fn take_authorization(&self) -> CommandResult<()> {
        let mut authorized = self
            .authorized_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match authorized.take() {
            Some(at) if at.elapsed() <= AUTHORIZATION_WINDOW => Ok(()),
            _ => Err(CommandError::AuthenticationRequired),
        }
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

    /// Opens the database with `key`. `authenticated`: the user just
    /// authenticated (typed the password, or passed the system check), which
    /// counts as the confirmation the security policy may ask for.
    pub fn attach(&self, key: DatabaseKey, authenticated: bool) -> CommandResult<()> {
        let store = SqliteStore::open(&self.paths.database, &Encryption::Key(key.clone()))?;
        let (modules, problems) = deckpilot_modules::configured(&store.module_states()?);
        for problem in problems {
            eprintln!("stored module state was not applied: {problem}");
        }
        store.apply_module_schemas(&modules.schemas())?;
        let launch = store.record_launch("desktop", APP_VERSION).ok();
        let confirmed = store.setting(CONFIRMED_VERSION)?;
        let confirmation_pending = if authenticated {
            store.set_setting(CONFIRMED_VERSION, APP_VERSION)?;
            false
        } else if !self.can_verify_user() {
            // Nothing to verify against (system key, no system check).
            false
        } else {
            match security_policy(&store)?.reauthentication {
                Reauthentication::SensitiveOnly => false,
                Reauthentication::AfterUpdate => confirmed.as_deref() != Some(APP_VERSION),
                Reauthentication::EveryLaunch => true,
            }
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

    /// Records that the user authenticated for this version.
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
