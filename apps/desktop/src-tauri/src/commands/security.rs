//! Opening the configuration, its protection and sensitive operations.
//!
//! Two protection modes exist (see `docs/security.md`):
//!
//! - **System-managed** (default): the background agent keeps a random key
//!   in the operating system's credential store and opens the database at
//!   login. This app receives the key from the agent; it never reads the
//!   credential store itself, so only one executable needs access to it.
//! - **Password**: the user's password is typed here. This app derives the
//!   key (Argon2id), opens the database and hands the key to the agent. The
//!   password never leaves this process and is never stored or logged.
//!
//! Sensitive operations (exports, protection changes, policy changes,
//! deleting the unencrypted copy) need a fresh authentication: the password,
//! or the operating system's verification (Touch ID / login password).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use deckpilot_ipc::{ClientError, ErrorCode, KeyMaterial, LockReason, Request, SecurityState};
use deckpilot_presence::{Method, PresenceError};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::policy::{
    self, AutoLock, Reauthentication, SecurityPolicy, security_policy,
};
use deckpilot_storage::vault::{DatabaseState, ProtectionMode};
use deckpilot_storage::{DatabaseKey, StorageError};

use crate::agent;
use crate::error::{CommandError, CommandResult};
use crate::state::{APP_VERSION, AgentSecurity, AppState};

/// How the user can be verified on this computer.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verification {
    /// The DeckPilot password.
    Password,
    /// Touch ID or the login password (macOS).
    TouchId,
    /// The login password of the operating system.
    DevicePassword,
    /// Nothing can be verified: system-managed key without a system check.
    /// Sensitive operations are only confirmed.
    ConfirmationOnly,
}

/// What the frontend needs to choose and render a prompt. The flags are
/// independent facts reported to the UI, not a state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityView {
    database: DatabaseState,
    /// `None` until a database exists.
    protection: Option<ProtectionMode>,
    unlocked: bool,
    /// The user must authenticate before continuing (security policy).
    confirmation_required: bool,
    /// Set while the agent is locked.
    lock_reason: Option<LockReason>,
    /// The agent is reading the key; the credential store may show a prompt.
    waiting_for_key_store: bool,
    /// A key file exists but its database is missing: nothing is created
    /// until the user decides (`start_new_configuration`).
    database_missing: bool,
    /// Whether the agent can detect this computer's screen lock.
    screen_lock_detection: bool,
    /// Why the configuration could not be opened automatically.
    problem: Option<String>,
    /// Name of this platform's credential store, e.g. "macOS Keychain".
    key_store: &'static str,
    /// How the user is verified for sensitive operations and after a lock.
    verification: Verification,
    /// Stored policy, while unlocked.
    policy: Option<SecurityPolicy>,
    agent_running: bool,
    app_version: &'static str,
    /// Unencrypted copies kept after migrating a legacy database.
    legacy_copies: Vec<String>,
    /// Encrypted backups made before schema upgrades.
    backup_dir: String,
}

fn verification(state: &AppState) -> Verification {
    if state.protection() == Some(ProtectionMode::Password) {
        return Verification::Password;
    }
    match state.presence.method() {
        Some(Method::TouchId) => Verification::TouchId,
        Some(Method::DevicePassword) => Verification::DevicePassword,
        None => Verification::ConfirmationOnly,
    }
}

/// The agent's current status, if it runs.
async fn refresh_agent(state: &AppState) -> Option<deckpilot_ipc::AgentStatus> {
    agent::connection(&state.paths)
        .await
        .ok()
        .and_then(|connection| connection.status)
}

async fn view(state: &AppState) -> CommandResult<SecurityView> {
    let status = refresh_agent(state).await;
    let policy = state
        .store()
        .ok()
        .and_then(|store| security_policy(&*store).ok());
    Ok(SecurityView {
        database: state.vault.state()?,
        protection: state.protection(),
        unlocked: state.is_unlocked() && !state.confirmation_pending(),
        confirmation_required: state.confirmation_pending(),
        lock_reason: status.as_ref().and_then(|s| s.lock_reason),
        waiting_for_key_store: status.as_ref().is_some_and(|s| s.waiting_for_key_store),
        database_missing: state.vault.state()? == DatabaseState::Missing
            && state.vault.has_key_file(),
        screen_lock_detection: status.as_ref().is_some_and(|s| s.screen_lock_detection),
        problem: status.as_ref().and_then(|s| s.security_problem.clone()),
        key_store: key_store_name(),
        verification: verification(state),
        policy,
        agent_running: status.is_some(),
        app_version: APP_VERSION,
        legacy_copies: state
            .vault
            .legacy_copies()
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        backup_dir: deckpilot_storage::backup_dir(&state.paths.database)
            .display()
            .to_string(),
    })
}

/// The names `deckpilot_keystore::platform_store_name` uses (this app does
/// not link the credential store; a test keeps the two in sync).
pub fn key_store_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS Keychain"
    } else if cfg!(target_os = "windows") {
        "Windows Credential Manager"
    } else if cfg!(target_os = "linux") {
        "Secret Service"
    } else {
        "system credential store"
    }
}

/// The prompt that opens the configuration in its current state. The
/// prompts adapt to the protection mode (password field or system check).
pub fn prompt_window(state: &AppState) -> &'static str {
    match state.vault.state() {
        Ok(DatabaseState::Missing) => "security.setup",
        Ok(DatabaseState::Plaintext) => "security.migrate",
        _ if state.confirmation_pending() => "security.confirm",
        _ => "security.unlock",
    }
}

/// A wrong password is a normal outcome, not a failure.
fn unlock_error(error: StorageError) -> CommandError {
    match error {
        StorageError::InvalidKey => CommandError::Invalid("the password is not correct".into()),
        StorageError::CredentialsRequired(_) => CommandError::Invalid(
            "this configuration does not use a password; it opens with the system key".into(),
        ),
        other => other.into(),
    }
}

fn agent_autostart_allowed() -> bool {
    std::env::var_os("DECKPILOT_NO_AGENT_AUTOSTART").is_none_or(|v| v.is_empty())
}

/// Starts the agent if it is not running (it opens a system-managed
/// configuration by itself) and waits until it answers.
async fn ensure_agent(state: &AppState) {
    if !agent_autostart_allowed() {
        return;
    }
    if let Ok(connection) = agent::connection(&state.paths).await
        && !connection.running
        && connection.can_start
    {
        if let Err(error) = agent::start_agent() {
            eprintln!("{error}");
            return;
        }
        agent::wait_until_running(&state.paths, Duration::from_secs(10)).await;
    }
}

/// Opens the database with the key of an unlocked agent, if there is one.
async fn attach_agent_key(state: &AppState, authenticated: bool) -> bool {
    match agent::fetch_key(&state.paths).await {
        // A key that does not open the database is ignored: the user then
        // sees the normal prompt.
        Some(key) => state.attach(key, authenticated).is_ok(),
        None => false,
    }
}

/// The current state. Starts the agent if needed and opens the
/// configuration with its key when it is unlocked: always with a
/// system-managed key, and after a restart of this app with a password.
#[tauri::command]
pub async fn security_status(state: State<'_, AppState>) -> CommandResult<SecurityView> {
    if !state.is_unlocked() {
        ensure_agent(&state).await;
        if state.vault.state()? == DatabaseState::Encrypted {
            attach_agent_key(&state, false).await;
        }
    }
    view(&state).await
}

/// Runs the operating system's verification on a blocking thread.
async fn verify_presence(state: &AppState, reason: &'static str) -> CommandResult<()> {
    if state.presence.method().is_none() {
        return Err(CommandError::Invalid(
            "this computer has no system authentication".into(),
        ));
    }
    // The check blocks until the user answered the system prompt.
    let presence = state.presence.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || presence.verify(reason))
        .await
        .map_err(|e| CommandError::Invalid(e.to_string()))?;
    outcome.map_err(|error| match error {
        PresenceError::Cancelled => CommandError::Invalid("authentication was cancelled".into()),
        other => CommandError::Invalid(other.to_string()),
    })
}

/// Whether a lock was a decision (by the user or a security policy) that
/// unlocking must respect: the user authenticates again, and an open app
/// locks too. Only a lock without such a decision — the agent just started,
/// or could not read its key — is not. Reasons added later count as
/// decisions, so the safe behaviour is the default.
pub fn lock_needs_authentication(reason: Option<LockReason>) -> bool {
    !matches!(
        reason,
        None | Some(LockReason::Startup | LockReason::KeyUnavailable)
    )
}

/// Opens a configuration with a system-managed key: after a lock (with the
/// system check, where available), or to retry when the credential store
/// was unavailable. Also sets up a new configuration, or encrypts a legacy
/// one, with a system key.
#[tauri::command]
pub async fn system_unlock(state: State<'_, AppState>) -> CommandResult<SecurityView> {
    ensure_agent(&state).await;
    let agent_state = refresh_agent(&state).await.map(|s| AgentSecurity::of(&s));
    let locked_by_user_or_policy = agent_state.is_some_and(|s| {
        s.security == SecurityState::Locked && lock_needs_authentication(s.lock_reason)
    });
    let verified = if locked_by_user_or_policy && state.presence.method().is_some() {
        verify_presence(&state, "unlock your DeckPilot configuration").await?;
        true
    } else {
        false
    };
    if agent_state.is_none() {
        return Err(CommandError::Invalid(
            "the DeckPilot background agent is not running".into(),
        ));
    }
    agent::request(&state.paths, Request::SystemUnlock).await?;
    if !attach_agent_key(&state, verified).await {
        return Err(CommandError::Locked);
    }
    view(&state).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPassword {
    password: Password,
    confirmation: Password,
}

/// Creates the encrypted database protected by a password: the user chose
/// a password at setup, e.g. because no credential store is available.
#[tauri::command]
pub async fn setup_password(
    state: State<'_, AppState>,
    input: NewPassword,
) -> CommandResult<SecurityView> {
    input
        .password
        .validate_new(&input.confirmation)
        .map_err(StorageError::from)?;
    let key = state.vault.initialize(&input.password)?;
    state.attach(key.clone(), true)?;
    agent::unlock(&state.paths, &key).await?;
    view(&state).await
}

#[tauri::command]
pub async fn unlock(state: State<'_, AppState>, password: Password) -> CommandResult<SecurityView> {
    let key = state.vault.unlock(&password).map_err(unlock_error)?;
    state.attach(key.clone(), true)?;
    agent::unlock(&state.paths, &key).await?;
    view(&state).await
}

/// Checks the user: the password in password mode, otherwise the system
/// verification. Without either (system key, no system check) the explicit
/// confirmation in the UI is all there is.
async fn verify_user(
    state: &AppState,
    password: Option<Password>,
    reason: &'static str,
) -> CommandResult<()> {
    let current = state.key().ok_or(CommandError::Locked)?;
    match state.protection() {
        Some(ProtectionMode::Password) => {
            let password = password
                .ok_or_else(|| CommandError::Invalid("enter your DeckPilot password".into()))?;
            let key = state.vault.unlock(&password).map_err(unlock_error)?;
            if key.matches(&current) {
                Ok(())
            } else {
                Err(CommandError::Invalid("the password is not correct".into()))
            }
        }
        _ if state.presence.method().is_some() => verify_presence(state, reason).await,
        _ => Ok(()),
    }
}

/// The authentication the security policy asks for after an update or at
/// launch.
#[tauri::command]
pub async fn confirm_identity(
    state: State<'_, AppState>,
    password: Option<Password>,
) -> CommandResult<SecurityView> {
    verify_user(&state, password, "open your DeckPilot configuration").await?;
    state.confirm()?;
    view(&state).await
}

/// Authenticates for the next sensitive operation (valid for two minutes,
/// used up by that operation).
#[tauri::command]
pub async fn authenticate(
    state: State<'_, AppState>,
    password: Option<Password>,
) -> CommandResult<()> {
    state.store()?;
    verify_user(&state, password, "confirm a sensitive change in DeckPilot").await?;
    state.authorize();
    Ok(())
}

/// Encrypts a database written by an earlier, unencrypted DeckPilot with a
/// password. (With a system key, the agent does this by itself.) The
/// original is kept until the user removes it.
#[tauri::command]
pub async fn encrypt_legacy_database(
    state: State<'_, AppState>,
    input: NewPassword,
) -> CommandResult<SecurityView> {
    input
        .password
        .validate_new(&input.confirmation)
        .map_err(StorageError::from)?;
    // An agent from before encryption may still hold the plain database.
    agent::stop_any_version(&state.paths, deckpilot_ipc::Request::Shutdown).await?;
    let (key, _legacy) = state.vault.encrypt_plaintext(&input.password)?;
    state.attach(key.clone(), true)?;
    if agent_autostart_allowed() {
        let _ = agent::start_agent();
    }
    agent::unlock(&state.paths, &key).await?;
    view(&state).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordChange {
    current: Password,
    password: Password,
    confirmation: Password,
}

/// Re-encrypts the database with a key derived from the new password. The
/// agent is locked while the file is replaced and unlocked again afterwards.
/// The current password is the authentication.
#[tauri::command]
pub async fn change_password(
    state: State<'_, AppState>,
    input: PasswordChange,
) -> CommandResult<SecurityView> {
    input
        .password
        .validate_new(&input.confirmation)
        .map_err(StorageError::from)?;
    // Check the current password before closing anything.
    let current_key = state.vault.unlock(&input.current).map_err(unlock_error)?;
    replace_protection(&state, current_key, |current| {
        state.vault.use_password(current, &input.password)
    })
    .await?;
    view(&state).await
}

/// Locks the agent and closes this app's connection, runs `change` with the
/// current key and reopens with the new key — or, if it failed, with the
/// old one (the database is then unchanged).
async fn replace_protection(
    state: &AppState,
    current: DatabaseKey,
    change: impl FnOnce(&DatabaseKey) -> Result<DatabaseKey, StorageError>,
) -> CommandResult<()> {
    agent::lock(&state.paths).await?;
    state.detach();
    match change(&current) {
        Ok(key) => {
            state.attach(key.clone(), true)?;
            agent::unlock(&state.paths, &key).await?;
            Ok(())
        }
        Err(error) => {
            state.attach(current.clone(), true)?;
            agent::unlock(&state.paths, &current).await?;
            Err(error.into())
        }
    }
}

/// Switches from the system-managed key to a password. Sensitive. The
/// previous key stays in the credential store (backups made before need it).
#[tauri::command]
pub async fn use_password(
    state: State<'_, AppState>,
    input: NewPassword,
) -> CommandResult<SecurityView> {
    input
        .password
        .validate_new(&input.confirmation)
        .map_err(StorageError::from)?;
    let current = state.key().ok_or(CommandError::Locked)?;
    if state.protection() == Some(ProtectionMode::Password) {
        return Err(CommandError::Invalid(
            "the configuration already uses a password; change it instead".into(),
        ));
    }
    state.take_authorization()?;
    replace_protection(&state, current, |current| {
        state.vault.use_password(current, &input.password)
    })
    .await?;
    view(&state).await
}

/// Switches from a password to a system-managed key. Sensitive. The agent
/// stores the new key in the credential store, verifies it and re-encrypts
/// the database; on failure nothing changes.
#[tauri::command]
pub async fn use_system_key(state: State<'_, AppState>) -> CommandResult<SecurityView> {
    let current = state.key().ok_or(CommandError::Locked)?;
    if state.protection() != Some(ProtectionMode::Password) {
        return Err(CommandError::Invalid(
            "the configuration already uses a system-managed key".into(),
        ));
    }
    if refresh_agent(&state).await.is_none() {
        return Err(CommandError::Invalid(
            "the DeckPilot background agent must be running to store the key".into(),
        ));
    }
    state.take_authorization()?;
    state.detach();
    let switched = agent::request(
        &state.paths,
        Request::UseSystemKey {
            key: KeyMaterial::new(current.to_hex()),
        },
    )
    .await;
    match switched {
        Ok(_) => {
            if !attach_agent_key(&state, true).await {
                return Err(CommandError::Locked);
            }
        }
        Err(error) => {
            // The agent reopened the unchanged database with the old key.
            state.attach(current, true)?;
            return Err(match error {
                ClientError::Agent(e) if e.code == ErrorCode::Unsupported => {
                    CommandError::Invalid(e.message)
                }
                other => other.into(),
            });
        }
    }
    view(&state).await
}

/// Changes the auto-lock and re-authentication settings. Sensitive.
/// Settings that need a verification this computer cannot do are refused.
#[tauri::command]
pub async fn set_security_policy(
    state: State<'_, AppState>,
    policy: SecurityPolicy,
) -> CommandResult<SecurityView> {
    let needs_verification = policy.auto_lock != AutoLock::Never
        || policy.reauthentication != Reauthentication::SensitiveOnly;
    if needs_verification && !state.can_verify_user() {
        return Err(CommandError::Invalid(
            "this setting needs a password or system authentication, which this computer does not offer"
                .into(),
        ));
    }
    if policy.auto_lock == AutoLock::ScreenLock
        && !refresh_agent(&state)
            .await
            .is_some_and(|status| status.screen_lock_detection)
    {
        return Err(CommandError::Invalid(
            "DeckPilot cannot detect the screen lock on this computer".into(),
        ));
    }
    let unchanged = security_policy(&*state.store()?)? == policy;
    if unchanged {
        return view(&state).await;
    }
    state.take_authorization()?;
    policy::set_security_policy(&*state.store()?, &policy)?;
    agent::reload(&state.paths).await?;
    view(&state).await
}

/// The database is missing although a configuration was set up here (the
/// key file exists). On the user's explicit request, the key file is moved
/// aside (not deleted) and a new, empty configuration is created by the
/// agent with a system key. Keys in the credential store are kept.
#[tauri::command]
pub async fn start_new_configuration(state: State<'_, AppState>) -> CommandResult<SecurityView> {
    state.vault.discard_orphaned_key_file()?;
    ensure_agent(&state).await;
    if refresh_agent(&state).await.is_some() {
        agent::request(&state.paths, Request::SystemUnlock).await?;
        attach_agent_key(&state, true).await;
    }
    view(&state).await
}

/// Locks the app and the agent.
#[tauri::command]
pub async fn lock(state: State<'_, AppState>) -> CommandResult<SecurityView> {
    agent::lock(&state.paths).await?;
    state.detach();
    view(&state).await
}

/// Deletes the unencrypted copies kept after a legacy migration. Sensitive.
/// Ordinary deletion does not guarantee that the data cannot be recovered
/// (see `docs/security.md`).
#[tauri::command]
pub async fn remove_unencrypted_copies(state: State<'_, AppState>) -> CommandResult<SecurityView> {
    state.store()?;
    state.take_authorization()?;
    state.vault.remove_legacy_copies()?;
    view(&state).await
}
