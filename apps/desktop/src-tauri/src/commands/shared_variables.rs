//! Shared variables of the Shared Variables settings page, over the
//! variable store (schema 8).
//!
//! Sensitive values never leave this module: they are written, never
//! read back, and their defaults are left out of the definitions sent to
//! the app. A `secretRef` value is the name of a stored credential, never
//! the secret.

use serde::Serialize;
use tauri::State;

use deckpilot_core::ProfileId;
use deckpilot_core::variables::{ScopeKey, VariableDefinition, VariableKey, VariableValue};
use deckpilot_storage::variable_references::VariableReference;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// A value at one scope; `value` is left out for sensitive variables.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueView {
    scope: ScopeKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<VariableValue>,
}

/// A variable as the settings page shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedVariableView {
    definition: VariableDefinition,
    /// The values that apply to the profile asked for: global ones, the
    /// profile's and its projects'.
    values: Vec<ValueView>,
    /// Every use, in any profile.
    references: Vec<VariableReference>,
}

/// Every shared variable with the values that apply to `profile_id`
/// (only global values without a profile) and where it is used.
#[tauri::command]
pub fn list_shared_variables(
    state: State<'_, AppState>,
    profile_id: Option<ProfileId>,
) -> CommandResult<Vec<SharedVariableView>> {
    let store = state.store()?;
    let mut values = store.global_variable_values()?;
    if let Some(profile) = profile_id {
        values.extend(store.profile_variable_values(profile)?);
    }
    store
        .shared_variables()?
        .into_iter()
        .map(|stored| {
            let mut definition = stored.definition;
            let sensitive = definition.sensitive;
            if sensitive {
                definition.default = None;
            }
            let values = values
                .iter()
                .filter(|v| v.key == definition.key)
                .map(|v| ValueView {
                    scope: v.scope,
                    value: (!sensitive).then(|| v.value.clone()),
                })
                .collect();
            let references = store.shared_variable_references(&definition.key)?;
            Ok(SharedVariableView {
                definition,
                values,
                references,
            })
        })
        .collect()
}

/// Creates or changes a user variable. Refused for keys of modules, for
/// invalid definitions, for a type change while the variable is used, and
/// when stored values would no longer fit.
#[tauri::command]
pub fn save_shared_variable(
    state: State<'_, AppState>,
    definition: VariableDefinition,
) -> CommandResult<()> {
    state.store()?.save_shared_variable(&definition)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

/// Deletes a variable and its values; refused while anything uses it.
#[tauri::command]
pub fn delete_shared_variable(state: State<'_, AppState>, key: VariableKey) -> CommandResult<()> {
    if !state.store()?.delete_shared_variable(&key)? {
        return Err(CommandError::NotFound("shared variable"));
    }
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

/// Sets the value at `scope`, checked against the definition (type,
/// allowed scope) and, for project values, the project's profile.
#[tauri::command]
pub fn set_shared_variable_value(
    state: State<'_, AppState>,
    key: VariableKey,
    scope: ScopeKey,
    value: VariableValue,
) -> CommandResult<()> {
    if matches!(scope, ScopeKey::Execution { .. }) {
        return Err(CommandError::Invalid(
            "execution values exist only during a run".into(),
        ));
    }
    state
        .store()?
        .set_shared_variable_value(&key, &scope, &value)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

/// Removes the value at `scope`; nothing to remove is fine.
#[tauri::command]
pub fn clear_shared_variable_value(
    state: State<'_, AppState>,
    key: VariableKey,
    scope: ScopeKey,
) -> CommandResult<()> {
    state.store()?.clear_shared_variable_value(&key, &scope)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}
