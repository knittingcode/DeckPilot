//! Commands invoked by the frontend.
//!
//! Every command loads the current state from storage, applies one change,
//! and writes it back, so the database remains the single source of truth.

// Tauri injects `State` and deserialized arguments by value.
#![allow(clippy::needless_pass_by_value)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::State;

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionResult, ActionType, ActionTypeId,
    ActiveProjectContext, CycleDirection, MachineConfig, MachineResourceBinding, Os, Profile,
    ProfileId, Project, ProjectId, ProjectKey, ResourceKey, ResourceKind,
};
use deckpilot_device::DeviceModel;
use deckpilot_storage::{ConfigStore, ProfileSummary};

use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile, machine_config};

/// Invoke handler with every command exposed to the frontend.
pub fn handler<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
{
    tauri::generate_handler![
        get_overview,
        list_action_types,
        list_device_models,
        list_profiles,
        get_profile,
        create_profile,
        delete_profile,
        set_active_profile,
        create_action,
        delete_action,
        run_action,
        create_project,
        delete_project,
        cycle_project,
        select_project,
        get_machine_config,
        set_resource_binding,
        remove_resource_binding,
    ]
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    app_version: &'static str,
    os: Os,
    database_path: String,
    profile_count: usize,
    active_profile: Option<ProfileSummary>,
    active_project: Option<Project>,
    action_type_count: usize,
    device_model_count: usize,
    resource_binding_count: usize,
}

#[tauri::command]
pub fn get_overview(state: State<'_, AppState>) -> CommandResult<Overview> {
    let store = state.store();
    let profiles = store.list_profiles()?;
    let active_id = store.active_profile()?;
    let active_profile = profiles.iter().find(|p| Some(p.id) == active_id).cloned();
    let active_project = match active_id {
        Some(id) => {
            let profile = load_profile(&store, id)?;
            let context = ActiveProjectContext::restore(&profile, store.active_project(id)?);
            context.project(&profile)?.cloned()
        }
        None => None,
    };
    Ok(Overview {
        app_version: env!("CARGO_PKG_VERSION"),
        os: Os::current(),
        database_path: state.database_path.display().to_string(),
        profile_count: profiles.len(),
        active_profile,
        active_project,
        action_type_count: state.engine.registry().action_types().count(),
        device_model_count: state.catalog.iter().count(),
        resource_binding_count: machine_config(&store)?.bindings.len(),
    })
}

#[tauri::command]
pub fn list_action_types(state: State<'_, AppState>) -> Vec<ActionType> {
    state.engine.registry().action_types().cloned().collect()
}

#[tauri::command]
pub fn list_device_models(state: State<'_, AppState>) -> Vec<DeviceModel> {
    state.catalog.iter().cloned().collect()
}

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
    Ok(ProfileView {
        active_project_id: context.project_id(),
        issues,
        profile,
    })
}

#[tauri::command]
pub fn create_profile(
    state: State<'_, AppState>,
    name: String,
    description: String,
) -> CommandResult<ProfileSummary> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CommandError::Invalid(
            "profile name must not be empty".into(),
        ));
    }
    let profile = Profile {
        description: description.trim().to_owned(),
        ..Profile::new(name)
    };

    let store = state.store();
    store.save_profile(&profile)?;
    if store.active_profile()?.is_none() {
        store.set_active_profile(Some(profile.id))?;
    }
    store
        .list_profiles()?
        .into_iter()
        .find(|summary| summary.id == profile.id)
        .ok_or(CommandError::NotFound("profile"))
}

#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, profile_id: ProfileId) -> CommandResult<()> {
    if state.store().delete_profile(profile_id)? {
        Ok(())
    } else {
        Err(CommandError::NotFound("profile"))
    }
}

#[tauri::command]
pub fn set_active_profile(state: State<'_, AppState>, profile_id: ProfileId) -> CommandResult<()> {
    Ok(state.store().set_active_profile(Some(profile_id))?)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAction {
    name: String,
    action_type: ActionTypeId,
    parameters: ActionParameters,
}

#[tauri::command]
pub fn create_action(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action: NewAction,
) -> CommandResult<ActionInstance> {
    let instance = state.engine.registry().create_instance(
        &action.name,
        action.action_type,
        &action.parameters,
    )?;
    let store = state.store();
    let mut profile = load_profile(&store, profile_id)?;
    profile.actions.push(instance.clone());
    store.save_profile(&profile)?;
    Ok(instance)
}

#[tauri::command]
pub fn delete_action(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action_id: ActionInstanceId,
) -> CommandResult<()> {
    let store = state.store();
    let mut profile = load_profile(&store, profile_id)?;
    profile
        .remove_action(action_id)
        .ok_or(CommandError::NotFound("action"))?;
    Ok(store.save_profile(&profile)?)
}

/// Runs an action in this process. Only hardware-independent built-in actions
/// exist at the moment, so this is safe to do from the configuration UI.
#[tauri::command]
pub fn run_action(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action_id: ActionInstanceId,
) -> CommandResult<ActionResult> {
    let store = state.store();
    let profile = load_profile(&store, profile_id)?;
    let machine = machine_config(&store)?;
    let mut context = ActiveProjectContext::restore(&profile, store.active_project(profile_id)?);
    let result = state
        .engine
        .run_action(&profile, &machine, &mut context, action_id)?;
    store.set_active_project(profile_id, context.project_id())?;
    Ok(result)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProject {
    key: String,
    name: String,
    #[serde(default)]
    resources: BTreeMap<String, String>,
}

#[tauri::command]
pub fn create_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project: NewProject,
) -> CommandResult<Project> {
    let key = ProjectKey::new(project.key.trim())?;
    let name = match project.name.trim() {
        "" => key.to_string(),
        name => name.to_owned(),
    };
    let mut new_project = Project::new(key, name);
    for (role, key) in project.resources {
        let role = role.trim();
        if role.is_empty() {
            return Err(CommandError::Invalid(
                "resource role must not be empty".into(),
            ));
        }
        new_project
            .resources
            .insert(role.to_owned(), ResourceKey::new(key.trim())?);
    }

    let store = state.store();
    let mut profile = load_profile(&store, profile_id)?;
    profile.add_project(new_project.clone())?;
    store.save_profile(&profile)?;
    Ok(new_project)
}

#[tauri::command]
pub fn delete_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
) -> CommandResult<()> {
    let store = state.store();
    let mut profile = load_profile(&store, profile_id)?;
    let before = profile.projects.len();
    profile.projects.retain(|project| project.id != project_id);
    if profile.projects.len() == before {
        return Err(CommandError::NotFound("project"));
    }
    store.save_profile(&profile)?;
    let context = ActiveProjectContext::restore(&profile, store.active_project(profile_id)?);
    Ok(store.set_active_project(profile_id, context.project_id())?)
}

#[tauri::command]
pub fn cycle_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    direction: CycleDirection,
) -> CommandResult<Option<ProjectId>> {
    let store = state.store();
    let profile = load_profile(&store, profile_id)?;
    let mut context = ActiveProjectContext::restore(&profile, store.active_project(profile_id)?);
    let active = context.cycle(&profile, direction)?;
    store.set_active_project(profile_id, active)?;
    Ok(active)
}

#[tauri::command]
pub fn select_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
) -> CommandResult<()> {
    let store = state.store();
    let profile = load_profile(&store, profile_id)?;
    let mut context = ActiveProjectContext::new(&profile);
    context.select(&profile, project_id)?;
    Ok(store.set_active_project(profile_id, Some(project_id))?)
}

#[tauri::command]
pub fn get_machine_config(state: State<'_, AppState>) -> CommandResult<MachineConfig> {
    Ok(machine_config(&state.store())?)
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
    let store = state.store();
    let mut config = machine_config(&store)?;
    config.set_binding(MachineResourceBinding {
        key: ResourceKey::new(key.trim())?,
        kind,
        value: value.to_owned(),
    });
    store.save_machine_config(&config)?;
    Ok(config)
}

#[tauri::command]
pub fn remove_resource_binding(
    state: State<'_, AppState>,
    key: ResourceKey,
) -> CommandResult<MachineConfig> {
    let store = state.store();
    let mut config = machine_config(&store)?;
    config
        .remove_binding(&key)
        .ok_or(CommandError::NotFound("resource binding"))?;
    store.save_machine_config(&config)?;
    Ok(config)
}
