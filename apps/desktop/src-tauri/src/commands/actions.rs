use serde::Deserialize;
use tauri::State;

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, Profile, ProfileId,
};

use deckpilot_storage::references::{ActionReference, ReferenceChange, Selection};

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

/// Rejects parameters that refer to another profile, e.g. a project.
fn check_in_profile(
    state: &AppState,
    action: &ActionInstance,
    profile: &Profile,
) -> CommandResult<()> {
    let problems = state.actions().check_in_profile(action, profile);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(CommandError::Invalid(problems.join("; ")))
    }
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
    state.actions().validate_instance(&mut instance)?;
    modify_profile(&state, profile_id, |profile| {
        check_in_profile(&state, &instance, profile)?;
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
    state.actions().validate_instance(&mut action)?;
    modify_profile(&state, profile_id, |profile| {
        check_in_profile(&state, &action, profile)?;
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
        state.actions().validate_instance(&mut copy)?;
        profile.actions.push(copy.clone());
        Ok(copy)
    })
}

/// Deletes an action that nothing uses: no key binding, workflow step,
/// graph node (or trigger of such a workflow). While anything does, it is
/// refused and nothing changes; the uses are removed first with
/// [`replace_action_references`] or [`unbind_action_references`].
#[tauri::command]
pub fn delete_action(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action_id: ActionInstanceId,
) -> CommandResult<()> {
    state.store()?.delete_action(profile_id, action_id)?;
    crate::agent::notify_reload(&state.paths);
    Ok(())
}

/// Every place that uses an action, with names for display.
#[tauri::command]
pub fn action_references(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action_id: ActionInstanceId,
) -> CommandResult<Vec<ActionReference>> {
    Ok(state.store()?.action_references(profile_id, action_id)?)
}

/// Points the selected uses of an action at `replacement`, in one change.
/// Overrides and mappings stay only when both actions have the same type;
/// the result says where they were removed.
#[tauri::command]
pub fn replace_action_references(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action_id: ActionInstanceId,
    replacement: ActionInstanceId,
    selection: Selection,
) -> CommandResult<ReferenceChange> {
    let change =
        state
            .store()?
            .replace_action_references(profile_id, action_id, replacement, &selection)?;
    crate::agent::notify_reload(&state.paths);
    Ok(change)
}

/// Clears the selected key bindings and removes the selected steps that use
/// an action, in one change. Graph nodes are refused: they are replaced or
/// removed in the workflow editor.
#[tauri::command]
pub fn unbind_action_references(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action_id: ActionInstanceId,
    selection: Selection,
) -> CommandResult<ReferenceChange> {
    let change = state
        .store()?
        .unbind_action(profile_id, action_id, &selection)?;
    crate::agent::notify_reload(&state.paths);
    Ok(change)
}
