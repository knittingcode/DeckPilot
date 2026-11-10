//! Opening, locking and protecting the configuration database.
//!
//! The agent is the only DeckPilot process that reads the credential store.
//! That keeps a single executable in the access list of the system key (no
//! repeated "allow access" prompts on macOS) and lets the agent open the
//! configuration at login, before anyone opened the configuration app. The
//! app gets the key from the agent over the authenticated local socket.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use deckpilot_ipc::{
    ErrorCode, IpcError, KeyMaterial, LockReason, ProtectionMode, Request, Response, SecurityState,
};
use deckpilot_keystore::{KeyStore, KeyStoreError, Scope, SystemKeyStore};
use deckpilot_storage::policy::AutoLock;
use deckpilot_storage::vault::{self, DatabaseState, Vault};
use deckpilot_storage::{DatabaseKey, Encryption, SqliteStore, StorageError};
use tracing::{info, warn};

use tokio::sync::oneshot;

use super::{Agent, Command, locked, security_state, storage_error};

/// How often auto-lock conditions (including the screen lock) are checked
/// by default.
pub const AUTO_LOCK_CHECK: Duration = Duration::from_secs(5);
/// Automatic retries when the credential store is unreachable, after 2, 4,
/// 8, 16, 32 and 64 seconds.
const KEY_RETRIES: u32 = 6;
const KEY_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Result of reading the system key (or setting up with one).
#[derive(Debug)]
pub(crate) struct KeyOutcome {
    state: DatabaseState,
    result: Result<DatabaseKey, StorageError>,
}
/// A wall-clock gap between two checks this much longer than the check
/// interval means the computer was asleep (timers do not fire during sleep).
const SLEEP_GAP: Duration = Duration::from_secs(90);

/// The wall clock; replaceable in tests.
pub type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(SystemTime::now)
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The credential store of this platform. Development builds and isolated
/// data directories use their own service name, so they never see the keys
/// of an installed DeckPilot. No store: system-managed keys are unavailable
/// (there is no file-based fallback).
pub(super) fn system_key_store() -> Option<Arc<dyn KeyStore>> {
    let custom_dir =
        std::env::var_os(deckpilot_ipc::paths::DATA_DIR_ENV).is_some_and(|v| !v.is_empty());
    let scope = if cfg!(debug_assertions) || custom_dir {
        Scope::Development
    } else {
        Scope::Production
    };
    match SystemKeyStore::open(scope) {
        Ok(store) => Some(Arc::new(store)),
        Err(error) => {
            warn!(%error, "the system credential store is unavailable");
            None
        }
    }
}

pub(super) fn protection_mode(database: &Path) -> Option<ProtectionMode> {
    match Vault::new(database).protection() {
        Ok(Some(protection)) => Some(match protection.mode() {
            vault::ProtectionMode::SystemManaged => ProtectionMode::SystemManaged,
            vault::ProtectionMode::Password => ProtectionMode::Password,
        }),
        _ => None,
    }
}

fn malformed() -> IpcError {
    IpcError::new(ErrorCode::InvalidRequest, "malformed key")
}

impl Agent {
    fn vault(&self) -> Vault {
        Vault::new(&self.options.paths.database)
    }

    fn refresh_protection(&mut self) {
        self.protection = protection_mode(&self.options.paths.database);
    }

    /// Unlock, lock and key requests, and the refusal of requests that need
    /// the configuration while it is locked. `None`: not handled here.
    pub(super) fn handle_security(
        &mut self,
        request: &Request,
    ) -> Option<Result<Response, IpcError>> {
        let result = match request {
            Request::Unlock { key } => DatabaseKey::from_hex(key.expose())
                .map_err(|_| malformed())
                .and_then(|key| self.open_with_key(key)),
            Request::UseSystemKey { key } => self.use_system_key(key),
            Request::Lock => {
                self.lock(LockReason::Requested);
                Ok(())
            }
            Request::GetDatabaseKey => {
                return Some(self.key.as_ref().ok_or_else(locked).map(|key| {
                    Response::DatabaseKey {
                        key: KeyMaterial::new(key.to_hex()),
                    }
                }));
            }
            // Devices and autostart do not touch the database.
            Request::RefreshDevices
            | Request::SetAutostart { .. }
            | Request::ShowWindow
            | Request::RequestWindow { .. }
            | Request::SimulateInput { .. } => return None,
            _ if self.security != SecurityState::Unlocked => return Some(Err(locked())),
            _ => return None,
        };
        // Security changes are published right away, also when they fail.
        self.publish();
        Some(result.map(|()| Response::Ok))
    }

    /// Opens what can be opened without the user: a database with a system
    /// key, or — with a system key — a new database or a legacy unencrypted
    /// one. Password-protected databases wait for the app. The credential
    /// store is read on a blocking thread (it may show a prompt); `reply`
    /// is answered when that finished.
    pub(super) fn open_automatically(
        &mut self,
        reply: Option<oneshot::Sender<Result<Response, IpcError>>>,
    ) {
        if self.security == SecurityState::Unlocked {
            if let Some(reply) = reply {
                let _ = reply.send(Ok(Response::Ok));
            }
            return;
        }
        self.key_waiters.extend(reply);
        if self.key_pending {
            return;
        }
        let vault = self.vault();
        let state = match vault.state() {
            Ok(state) => state,
            Err(error) => {
                self.answer_key_waiters(&Err(storage_error(&error)));
                return;
            }
        };
        let Some(keys) = self.options.key_store.clone() else {
            let result = if state == DatabaseState::Encrypted
                && self.protection == Some(ProtectionMode::Password)
            {
                Err(password_required())
            } else {
                let problem = format!(
                    "the {} is not available",
                    deckpilot_keystore::platform_store_name()
                );
                self.security_problem = Some(problem.clone());
                self.note_key_unavailable(state);
                Err(IpcError::new(ErrorCode::Unsupported, problem))
            };
            self.answer_key_waiters(&result);
            self.publish();
            return;
        };
        self.key_pending = true;
        self.publish();
        let commands = self.handle.commands.clone();
        tokio::task::spawn_blocking(move || {
            let result = match state {
                DatabaseState::Missing => vault.initialize_system(keys.as_ref()).inspect(|_| {
                    info!(store = %keys.name(), "created an encrypted configuration with a system-managed key");
                }),
                DatabaseState::Plaintext => vault
                    .encrypt_plaintext_system(keys.as_ref())
                    .map(|(key, legacy)| {
                        info!(kept = %legacy.display(), "encrypted the configuration of an earlier version with a system-managed key");
                        key
                    }),
                DatabaseState::Encrypted => vault.unlock_system(keys.as_ref()),
            };
            let _ = commands.send(Command::SystemKey(KeyOutcome { state, result }));
        });
    }

    /// Continues after the blocking read of the system key.
    pub(super) fn finish_system_key(&mut self, outcome: KeyOutcome) {
        self.key_pending = false;
        self.refresh_protection();
        let result = match outcome.result {
            Ok(key) => {
                self.security_problem = None;
                self.key_retries = 0;
                self.security =
                    security_state(&self.options.paths.database).unwrap_or(SecurityState::Locked);
                self.open_with_key(key)
            }
            Err(StorageError::CredentialsRequired(_)) => {
                self.security_problem = None;
                Err(password_required())
            }
            Err(error) => {
                warn!(%error, "the configuration could not be opened with the system key");
                self.security_problem = Some(error.to_string());
                self.note_key_unavailable(outcome.state);
                if matches!(
                    error,
                    StorageError::KeyStore(KeyStoreError::Unreachable { .. })
                ) {
                    self.schedule_key_retry();
                }
                Err(IpcError::new(ErrorCode::Failed, error.to_string()))
            }
        };
        self.answer_key_waiters(&result);
        self.publish();
    }

    /// The credential store could not be reached (e.g. the keyring daemon
    /// starts after the agent at login): try again a few times, waiting
    /// longer each time. Only for unreachable stores — a refusal or a
    /// cancelled prompt is never retried automatically.
    fn schedule_key_retry(&mut self) {
        if self.key_retries >= KEY_RETRIES {
            return;
        }
        let delay = KEY_RETRY_DELAY * 2u32.pow(self.key_retries);
        self.key_retries += 1;
        let commands = self.handle.commands.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = commands.send(Command::RetrySystemKey);
        });
    }

    fn answer_key_waiters(&mut self, result: &Result<(), IpcError>) {
        for waiter in self.key_waiters.drain(..) {
            let _ = waiter.send(result.clone().map(|()| Response::Ok));
        }
    }

    fn note_key_unavailable(&mut self, state: DatabaseState) {
        self.security = security_state(&self.options.paths.database).unwrap_or(match state {
            DatabaseState::Missing => SecurityState::Uninitialized,
            DatabaseState::Plaintext => SecurityState::LegacyPlaintext,
            DatabaseState::Encrypted => SecurityState::Locked,
        });
        if self.security == SecurityState::Locked {
            self.lock_reason = Some(LockReason::KeyUnavailable);
        }
    }

    /// Opens the database with `key`. Fails without changing anything if the
    /// key is wrong.
    pub(super) fn open_with_key(&mut self, key: DatabaseKey) -> Result<(), IpcError> {
        if self.security == SecurityState::Unlocked {
            return Ok(());
        }
        let database = self.options.paths.database.clone();
        // The app may have set up or migrated the database meanwhile.
        if security_state(&database).map_err(|e| storage_error(&e))? != SecurityState::Locked {
            return Err(IpcError::new(
                ErrorCode::Failed,
                "there is no encrypted database to unlock",
            ));
        }
        let store = match SqliteStore::open(&database, &Encryption::Key(key.clone())) {
            Ok(store) => store,
            Err(StorageError::InvalidKey) => {
                warn!("rejected an unlock attempt with a wrong key");
                return Err(IpcError::new(ErrorCode::Unauthorized, "wrong key"));
            }
            Err(error) => return Err(storage_error(&error)),
        };
        self.store = Some(store);
        self.key = Some(key);
        self.security = SecurityState::Unlocked;
        self.lock_reason = None;
        self.security_problem = None;
        self.last_activity = self.now_seconds();
        self.refresh_protection();
        self.runtime.set_placeholder(None);
        info!("configuration unlocked");
        if let Err(error) = self.load_configuration() {
            self.lock(LockReason::Requested);
            return Err(storage_error(&error));
        }
        Ok(())
    }

    /// Closes the database and forgets the key. Devices show "Locked".
    pub(super) fn lock(&mut self, reason: LockReason) {
        self.store = None;
        self.key = None;
        self.security =
            security_state(&self.options.paths.database).unwrap_or(SecurityState::Locked);
        self.lock_reason = (self.security == SecurityState::Locked).then_some(reason);
        self.refresh_protection();
        self.runtime.set_profile(None, None);
        self.runtime.set_placeholder(Some("Locked"));
        self.persisted_project = None;
        info!(?reason, "configuration locked");
        self.after_change();
    }

    /// Re-encrypts a password-protected database with a new random key in
    /// the credential store. `key` proves access. The configuration app has
    /// closed its connection; the agent closes its own for the switch and
    /// reopens the database afterwards — with the old key if the switch
    /// failed, which leaves everything as it was.
    fn use_system_key(&mut self, key: &KeyMaterial) -> Result<(), IpcError> {
        let current = DatabaseKey::from_hex(key.expose()).map_err(|_| malformed())?;
        if self.protection == Some(ProtectionMode::SystemManaged) {
            return Err(IpcError::new(
                ErrorCode::InvalidRequest,
                "the configuration already uses a system-managed key",
            ));
        }
        if let Some(open) = &self.key
            && !open.matches(&current)
        {
            return Err(IpcError::new(ErrorCode::Unauthorized, "wrong key"));
        }
        let keys = self.options.key_store.clone().ok_or_else(|| {
            IpcError::new(
                ErrorCode::Unsupported,
                format!(
                    "the {} is not available",
                    deckpilot_keystore::platform_store_name()
                ),
            )
        })?;
        self.lock(LockReason::Requested);
        let switched = self.vault().use_system_key(&current, keys.as_ref());
        self.refresh_protection();
        match switched {
            Ok(new_key) => {
                info!(store = %keys.name(), "the configuration now uses a system-managed key");
                self.open_with_key(new_key)
            }
            Err(error) => {
                warn!(%error, "switching to a system-managed key failed; nothing was changed");
                let _ = self.open_with_key(current);
                Err(match error {
                    StorageError::InvalidKey => IpcError::new(ErrorCode::Unauthorized, "wrong key"),
                    other => IpcError::new(ErrorCode::Failed, other.to_string()),
                })
            }
        }
    }

    pub(super) fn now_seconds(&self) -> u64 {
        seconds((self.options.clock)())
    }

    /// Locks after inactivity, sleep or the screen lock, as configured.
    pub(super) fn check_auto_lock(&mut self) {
        let now = (self.options.clock)();
        let gap = now.duration_since(self.last_check).unwrap_or_default();
        self.last_check = now;
        // Lock on the change to "locked", so that unlocking DeckPilot while
        // the screen is still locked does not lock it again immediately.
        let screen_locked = self
            .options
            .screen_lock
            .as_ref()
            .and_then(|monitor| monitor.is_locked())
            .unwrap_or(false);
        let screen_just_locked = screen_locked && !self.screen_was_locked;
        self.screen_was_locked = screen_locked;
        if self.security != SecurityState::Unlocked {
            return;
        }
        let slept = gap > self.options.auto_lock_check + SLEEP_GAP;
        match self.auto_lock {
            AutoLock::ScreenLock if screen_just_locked => self.lock(LockReason::ScreenLocked),
            AutoLock::Sleep | AutoLock::ScreenLock if slept => self.lock(LockReason::Sleep),
            policy => {
                if let Some(limit) = policy.idle_after()
                    && seconds(now).saturating_sub(self.last_activity) >= limit.as_secs()
                {
                    self.lock(LockReason::Idle);
                }
            }
        }
    }
}

fn password_required() -> IpcError {
    IpcError::new(
        ErrorCode::Locked,
        "the configuration is protected by a password",
    )
}
