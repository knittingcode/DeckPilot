//! Credentials stored on this computer, used by modules by their id.
//!
//! The app can add, replace and remove them; it never reads a secret back.
//! After a change the agent reloads, so running workflows use the new value
//! from their next request on.

use tauri::State;

use deckpilot_core::{CredentialId, CredentialInfo, Secret};

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub fn list_credentials(state: State<'_, AppState>) -> CommandResult<Vec<CredentialInfo>> {
    Ok(state.store()?.credentials()?)
}

/// Stores a credential. Without `secret`, only the description of an
/// existing credential changes.
#[tauri::command]
pub async fn set_credential(
    state: State<'_, AppState>,
    id: CredentialId,
    description: String,
    secret: Option<String>,
) -> CommandResult<Vec<CredentialInfo>> {
    let secret = secret.filter(|s| !s.is_empty()).map(Secret::new);
    state
        .store()?
        .set_credential(&id, description.trim(), secret.as_ref())?;
    crate::agent::reload(&state.paths).await?;
    Ok(state.store()?.credentials()?)
}

#[tauri::command]
pub async fn delete_credential(
    state: State<'_, AppState>,
    id: CredentialId,
) -> CommandResult<Vec<CredentialInfo>> {
    state.store()?.delete_credential(&id)?;
    crate::agent::reload(&state.paths).await?;
    Ok(state.store()?.credentials()?)
}
