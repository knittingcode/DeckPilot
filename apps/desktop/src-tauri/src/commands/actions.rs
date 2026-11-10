use serde::Deserialize;
use tauri::State;

use deckpilot_core::{ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, ProfileId};

use super::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAction {
    name: String,
    action_type: ActionTypeId,
    parameters: ActionParameters,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

#[tauri::command]
pub fn create_action(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action: NewAction,
) -> CommandResult<ActionInstance> {
    let mut instance = ActionInstance {
        id: ActionInstanceId::new(),
        name: action.name,
        action_type: action.action_type,
        parameters: action.parameters,
        description: action.description,
        icon: action.icon,
        label: action.label,
    };
    state.registry.validate_instance(&mut instance)?;
    modify_profile(&state, profile_id, |profile| {
        profile.actions.push(instance.clone());
        Ok(instance)
    })
}

/// Replaces an action's configuration; its identifier and type stay the same.
#[tauri::command]
pub fn update_action(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action: ActionInstance,
) -> CommandResult<ActionInstance> {
    let mut action = action;
    state.registry.validate_instance(&mut action)?;
    modify_profile(&state, profile_id, |profile| {
        let slot = profile
            .actions
            .iter_mut()
            .find(|existing| existing.id == action.id)
            .ok_or(CommandError::NotFound("action"))?;
        if slot.action_type != action.action_type {
            return Err(CommandError::Invalid(
                "the action type cannot be changed".into(),
            ));
        }
        *slot = action.clone();
        Ok(action)
    })
}

#[tauri::command]
pub fn duplicate_action(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action_id: ActionInstanceId,
    name: String,
) -> CommandResult<ActionInstance> {
    modify_profile(&state, profile_id, |profile| {
        let mut copy = profile
            .action(action_id)
            .ok_or(CommandError::NotFound("action"))?
            .clone();
        copy.id = ActionInstanceId::new();
        copy.name = name;
        state.registry.validate_instance(&mut copy)?;
        profile.actions.push(copy.clone());
        Ok(copy)
    })
}

/// Deletes an action that no workflow or layout uses.
#[tauri::command]
pub fn delete_action(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action_id: ActionInstanceId,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        profile.delete_action(action_id)?;
        Ok(())
    })
}
