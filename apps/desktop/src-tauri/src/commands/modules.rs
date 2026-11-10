//! Listing, enabling and configuring feature modules.

use tauri::State;

use deckpilot_core::{ActionParameters, ModuleId, ModuleInfo};
use deckpilot_storage::ConfigStore;

use crate::error::CommandResult;
use crate::state::AppState;

#[tauri::command]
pub fn list_modules(state: State<'_, AppState>) -> Vec<ModuleInfo> {
    state.modules().modules()
}

/// Saves the state of one module and lets the agent pick it up. Only the
/// enabled flag changes: actions, workflows and layouts that use the module
/// are kept and reported as unavailable while it is disabled.
fn persist(state: &AppState, id: &ModuleId) -> CommandResult<Vec<ModuleInfo>> {
    let modules = state.modules();
    let settings = modules.states().remove(id).unwrap_or_default();
    state.store().set_module_state(id, &settings)?;
    let list = modules.modules();
    drop(modules);
    crate::agent::notify_reload(&state.paths);
    Ok(list)
}

#[tauri::command]
pub fn set_module_enabled(
    state: State<'_, AppState>,
    module_id: ModuleId,
    enabled: bool,
) -> CommandResult<Vec<ModuleInfo>> {
    state.modules().set_enabled(&module_id, enabled)?;
    persist(&state, &module_id)
}

#[tauri::command]
pub fn set_module_config(
    state: State<'_, AppState>,
    module_id: ModuleId,
    config: ActionParameters,
) -> CommandResult<Vec<ModuleInfo>> {
    state.modules().set_config(&module_id, &config)?;
    persist(&state, &module_id)
}
