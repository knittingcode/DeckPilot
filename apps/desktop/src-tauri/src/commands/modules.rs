//! Listing, enabling and configuring feature modules.

use tauri::State;

use deckpilot_core::{ActionParameters, ModuleId, ModuleInfo, ModuleSettings};
use deckpilot_storage::ConfigStore;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

#[tauri::command]
pub fn list_modules(state: State<'_, AppState>) -> Vec<ModuleInfo> {
    state.modules().modules()
}

/// Saves the state of one module and waits until a running agent applied
/// it, so that no action of a disabled module runs after this returns. Only
/// the enabled flag and settings change: actions, workflows and layouts that
/// use the module are kept and reported as unavailable while it is disabled.
///
/// `previous` is restored in memory if saving fails, so the app never acts
/// on a state that is not stored. The modules lock is never held while the
/// store is locked (other commands lock the store first).
async fn persist(
    state: &AppState,
    id: &ModuleId,
    previous: ModuleSettings,
) -> CommandResult<Vec<ModuleInfo>> {
    let settings = state.modules().states().remove(id).unwrap_or_default();
    let saved = state.store()?.set_module_state(id, &settings);
    if let Err(error) = saved {
        let mut modules = state.modules();
        // Restoring a state that was valid before cannot fail.
        let _ = modules.set_config(id, &previous.config);
        let _ = modules.set_enabled(id, previous.enabled);
        return Err(error.into());
    }
    crate::agent::reload(&state.paths).await?;
    Ok(state.modules().modules())
}

fn current(state: &AppState, id: &ModuleId) -> CommandResult<ModuleSettings> {
    state
        .modules()
        .states()
        .remove(id)
        .ok_or_else(|| deckpilot_core::ModuleError::Unknown(id.clone()).into())
}

#[tauri::command]
pub async fn set_module_enabled(
    state: State<'_, AppState>,
    module_id: ModuleId,
    enabled: bool,
) -> CommandResult<Vec<ModuleInfo>> {
    let previous = current(&state, &module_id)?;
    if !enabled
        && module_id == deckpilot_modules::work_session::module_id()
        && state.store()?.active_work_session()?.is_some()
    {
        return Err(CommandError::Invalid(
            "a work session is active; finish it from its key before turning work sessions off"
                .into(),
        ));
    }
    state.modules().set_enabled(&module_id, enabled)?;
    persist(&state, &module_id, previous).await
}

#[tauri::command]
pub async fn set_module_config(
    state: State<'_, AppState>,
    module_id: ModuleId,
    config: ActionParameters,
) -> CommandResult<Vec<ModuleInfo>> {
    let previous = current(&state, &module_id)?;
    state.modules().set_config(&module_id, &config)?;
    persist(&state, &module_id, previous).await
}
