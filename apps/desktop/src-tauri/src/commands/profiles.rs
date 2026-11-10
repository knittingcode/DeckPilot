use std::collections::BTreeMap;

use serde::Serialize;
use tauri::State;

use deckpilot_core::{
    ActionInstanceId, ActiveProjectContext, Profile, ProfileId, ProjectId, Usage, WorkflowId,
};
use deckpilot_storage::{ConfigStore, ProfileSummary};

use super::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesView {
    profiles: Vec<ProfileSummary>,
    active_profile_id: Option<ProfileId>,
}

#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> CommandResult<ProfilesView> {
    let store = state.store();
    Ok(ProfilesView {
        profiles: store.list_profiles()?,
        active_profile_id: store.active_profile()?,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    profile: Profile,
    active_project_id: Option<ProjectId>,
    issues: Vec<String>,
    action_usages: BTreeMap<ActionInstanceId, Vec<Usage>>,
    workflow_usages: BTreeMap<WorkflowId, Vec<Usage>>,
}

#[tauri::command]
pub fn get_profile(
    state: State<'_, AppState>,
    profile_id: ProfileId,
) -> CommandResult<ProfileView> {
    let store = state.store();
    let profile = load_profile(&store, profile_id)?;
    let context = ActiveProjectContext::restore(&profile, store.active_project(profile_id)?);
    let issues = profile
        .validate(&state.catalog)
        .iter()
        .map(ToString::to_string)
        .collect();
    let action_usages = profile
        .actions
        .iter()
        .map(|a| (a.id, profile.action_usages(a.id)))
        .collect();
    let workflow_usages = profile
        .workflows
        .iter()
        .map(|w| (w.id, profile.workflow_usages(w.id)))
        .collect();
    Ok(ProfileView {
        active_project_id: context.project_id(),
        issues,
        action_usages,
        workflow_usages,
        profile,
    })
}

fn summary(store: &deckpilot_storage::SqliteStore, id: ProfileId) -> CommandResult<ProfileSummary> {
    store
        .list_profiles()?
        .into_iter()
        .find(|summary| summary.id == id)
        .ok_or(CommandError::NotFound("profile"))
}

fn checked_name(name: &str) -> CommandResult<String> {
    let name = name.trim();
    if name.is_empty() {
        Err(CommandError::Invalid(
            "profile name must not be empty".into(),
        ))
    } else {
        Ok(name.to_owned())
    }
}

#[tauri::command]
pub fn create_profile(
    state: State<'_, AppState>,
    name: String,
    description: String,
) -> CommandResult<ProfileSummary> {
    let profile = Profile {
        description: description.trim().to_owned(),
        ..Profile::new(checked_name(&name)?)
    };
    let store = state.store();
    store.save_profile(&profile)?;
    if store.active_profile()?.is_none() {
        store.set_active_profile(Some(profile.id))?;
    }
    crate::agent::notify_reload(&state.paths);
    summary(&store, profile.id)
}

#[tauri::command]
pub fn update_profile_details(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    name: String,
    description: String,
) -> CommandResult<()> {
    let name = checked_name(&name)?;
    modify_profile(&state, profile_id, |profile| {
        profile.name = name;
        description.trim().clone_into(&mut profile.description);
        Ok(())
    })
}

/// Copies a profile with new identifiers for everything in it, so editing
/// the copy never affects the original.
#[tauri::command]
pub fn duplicate_profile(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    name: String,
) -> CommandResult<ProfileSummary> {
    let name = checked_name(&name)?;
    let store = state.store();
    let copy = load_profile(&store, profile_id)?.duplicate(&name);
    store.save_profile(&copy)?;
    summary(&store, copy.id)
}

#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, profile_id: ProfileId) -> CommandResult<()> {
    if state.store().delete_profile(profile_id)? {
        crate::agent::notify_reload(&state.paths);
        Ok(())
    } else {
        Err(CommandError::NotFound("profile"))
    }
}

#[tauri::command]
pub fn set_default_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: Option<ProjectId>,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        if let Some(id) = project_id
            && profile.project(id).is_none()
        {
            return Err(CommandError::NotFound("project"));
        }
        profile.default_project = project_id;
        Ok(())
    })
}
