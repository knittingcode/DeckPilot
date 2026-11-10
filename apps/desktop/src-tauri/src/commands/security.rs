//! Password setup, unlocking and password changes.
//!
//! The configuration app is the only place where the password is typed. It
//! derives the database key (Argon2id, see `deckpilot_storage::vault`),
//! opens the database and hands the key to the background agent over the
//! authenticated local socket. The password itself never leaves this
//! process and is never stored or logged.

use serde::{Deserialize, Serialize};
use tauri::State;

use deckpilot_storage::StorageError;
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::DatabaseState;

use crate::agent;
use crate::error::{CommandError, CommandResult};
use crate::state::{APP_VERSION, AppState};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityView {
    database: DatabaseState,
    unlocked: bool,
    /// The password must be entered once after an update.
    confirmation_required: bool,
    app_version: &'static str,
    /// Unencrypted copies kept after migrating a legacy database.
    legacy_copies: Vec<String>,
    /// Encrypted backups made before schema upgrades.
    backup_dir: String,
}

fn view(state: &AppState) -> CommandResult<SecurityView> {
    Ok(SecurityView {
        database: state.vault.state()?,
        unlocked: state.is_unlocked() && !state.confirmation_pending(),
        confirmation_required: state.confirmation_pending(),
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

/// A wrong password is a normal outcome, not a failure.
fn unlock_error(error: StorageError) -> CommandError {
    match error {
        StorageError::InvalidKey => CommandError::Invalid("the password is not correct".into()),
        other => other.into(),
    }
}

/// The current state. If an already unlocked agent runs, its key opens the
/// database without asking for the password (a restart of this app).
#[tauri::command]
pub async fn security_status(state: State<'_, AppState>) -> CommandResult<SecurityView> {
    if !state.is_unlocked()
        && state.vault.state()? == DatabaseState::Encrypted
        && let Some(key) = agent::fetch_key(&state.paths).await
    {
        // A key that does not open the database is ignored: the user then
        // sees the normal unlock screen.
        let _ = state.attach(key, false);
    }
    view(&state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPassword {
    password: Password,
    confirmation: Password,
}

/// First start: creates the encrypted database.
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
    view(&state)
}

#[tauri::command]
pub async fn unlock(state: State<'_, AppState>, password: Password) -> CommandResult<SecurityView> {
    let key = state.vault.unlock(&password).map_err(unlock_error)?;
    state.attach(key.clone(), true)?;
    agent::unlock(&state.paths, &key).await?;
    view(&state)
}

/// The password confirmation after an update.
#[tauri::command]
pub async fn confirm_password(
    state: State<'_, AppState>,
    password: Password,
) -> CommandResult<SecurityView> {
    let current = state.key().ok_or(CommandError::Locked)?;
    let key = state.vault.unlock(&password).map_err(unlock_error)?;
    if !key.matches(&current) {
        return Err(CommandError::Invalid("the password is not correct".into()));
    }
    state.confirm()?;
    view(&state)
}

/// Encrypts a database written by an earlier, unencrypted DeckPilot. The
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
    if std::env::var_os("DECKPILOT_NO_AGENT_AUTOSTART").is_none_or(|v| v.is_empty()) {
        let _ = agent::start_agent();
    }
    agent::unlock(&state.paths, &key).await?;
    view(&state)
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
    agent::lock(&state.paths).await?;
    state.detach();
    let result = state.vault.change_password(&input.current, &input.password);
    // On failure the old database is untouched: reopen it with the old key.
    let key = match result {
        Ok(key) => key,
        Err(error) => {
            state.attach(current_key.clone(), true)?;
            agent::unlock(&state.paths, &current_key).await?;
            return Err(error.into());
        }
    };
    state.attach(key.clone(), true)?;
    agent::unlock(&state.paths, &key).await?;
    view(&state)
}

/// Locks the app and the agent. The password is needed to continue.
#[tauri::command]
pub async fn lock(state: State<'_, AppState>) -> CommandResult<SecurityView> {
    agent::lock(&state.paths).await?;
    state.detach();
    view(&state)
}

/// Deletes the unencrypted copies kept after a legacy migration, after the
/// user confirmed it in the UI. Ordinary deletion does not guarantee that
/// the data cannot be recovered (see `docs/security.md`).
#[tauri::command]
pub fn remove_unencrypted_copies(state: State<'_, AppState>) -> CommandResult<SecurityView> {
    state.store()?;
    state.vault.remove_legacy_copies()?;
    view(&state)
}
