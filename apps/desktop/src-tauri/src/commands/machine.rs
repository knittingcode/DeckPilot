//! This computer's resource bindings. They belong to the core: any module
//! may resolve them if it holds the `machine-resources` capability.

use tauri::State;

use deckpilot_core::gesture::GestureTimings;
use deckpilot_core::{MachineConfig, MachineResourceBinding, ResourceKey, ResourceKind};
use deckpilot_storage::ConfigStore;
use deckpilot_storage::gestures;

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, machine_config};

#[tauri::command]
pub fn get_machine_config(state: State<'_, AppState>) -> CommandResult<MachineConfig> {
    Ok(machine_config(&*state.store()?)?)
}

#[tauri::command]
pub fn set_resource_binding(
    state: State<'_, AppState>,
    key: String,
    kind: ResourceKind,
    value: String,
) -> CommandResult<MachineConfig> {
    let value = value.trim();
    if value.is_empty() {
        return Err(CommandError::Invalid(
            "resource value must not be empty".into(),
        ));
    }
    let store = state.store()?;
    let mut config = machine_config(&store)?;
    config.set_binding(MachineResourceBinding {
        key: ResourceKey::new(key.trim())?,
        kind,
        value: value.to_owned(),
    });
    store.save_machine_config(&config)?;
    crate::agent::notify_reload(&state.paths);
    Ok(config)
}

#[tauri::command]
pub fn remove_resource_binding(
    state: State<'_, AppState>,
    key: ResourceKey,
) -> CommandResult<MachineConfig> {
    let store = state.store()?;
    let mut config = machine_config(&store)?;
    config
        .remove_binding(&key)
        .ok_or(CommandError::NotFound("resource binding"))?;
    store.save_machine_config(&config)?;
    crate::agent::notify_reload(&state.paths);
    Ok(config)
}

/// The double-click window and long-press time of every key whose page sets
/// none (the `gestures.defaults` setting of this computer).
#[tauri::command]
pub fn get_gesture_defaults(state: State<'_, AppState>) -> CommandResult<GestureTimings> {
    Ok(gestures::gesture_defaults(&*state.store()?)?)
}

/// Stores the default gesture timings; values outside the allowed ranges
/// are refused. The agent reads them on its next configuration reload.
#[tauri::command]
pub fn set_gesture_defaults(
    state: State<'_, AppState>,
    timings: GestureTimings,
) -> CommandResult<GestureTimings> {
    let store = state.store()?;
    gestures::set_gesture_defaults(&*store, timings)?;
    let stored = gestures::gesture_defaults(&*store)?;
    crate::agent::notify_reload(&state.paths);
    Ok(stored)
}
