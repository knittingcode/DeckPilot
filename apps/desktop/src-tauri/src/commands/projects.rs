use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tauri::State;

use deckpilot_core::{
    MachineConfig, MachineResourceBinding, ProfileId, Project, ProjectId, ProjectKey, ResourceKey,
    ResourceKind,
};
use deckpilot_storage::ConfigStore;

use super::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile, machine_config};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInput {
    key: String,
    name: String,
    #[serde(default)]
    resources: BTreeMap<String, String>,
}

fn build_project(id: ProjectId, input: ProjectInput) -> CommandResult<Project> {
    let key = ProjectKey::new(input.key.trim())?;
    let name = match input.name.trim() {
        "" => key.to_string(),
        name => name.to_owned(),
    };
    let mut project = Project {
        id,
        key,
        name,
        resources: BTreeMap::new(),
    };
    for (role, resource) in input.resources {
        let role = role.trim();
        if role.is_empty() {
            return Err(CommandError::Invalid(
                "resource role must not be empty".into(),
            ));
        }
        project
            .resources
            .insert(role.to_owned(), ResourceKey::new(resource.trim())?);
    }
    Ok(project)
}

#[tauri::command]
pub fn create_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project: ProjectInput,
) -> CommandResult<Project> {
    let project = build_project(ProjectId::new(), project)?;
    modify_profile(&state, profile_id, |profile| {
        profile.add_project(project.clone())?;
        Ok(project)
    })
}

#[tauri::command]
pub fn update_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    project: ProjectInput,
) -> CommandResult<Project> {
    let project = build_project(project_id, project)?;
    modify_profile(&state, profile_id, |profile| {
        profile.update_project(project.clone())?;
        Ok(project)
    })
}

/// Moves a project within the cycle order.
#[tauri::command]
pub fn move_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    index: usize,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        Ok(profile.move_project(project_id, index)?)
    })
}

#[tauri::command]
pub fn delete_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
) -> CommandResult<()> {
    modify_profile(&state, profile_id, |profile| {
        profile.delete_project(project_id)?;
        Ok(())
    })
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
    crate::agent::notify_reload(&state.paths);
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
    crate::agent::notify_reload(&state.paths);
    Ok(config)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceStatus {
    /// Bound and found on this machine.
    Ok,
    /// Bound, but the path does not exist or the URL is malformed.
    Missing,
    /// Used by a project, but not bound on this machine.
    Unbound,
    /// Bound to something DeckPilot cannot check (services, containers).
    Unchecked,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCheck {
    key: ResourceKey,
    kind: Option<ResourceKind>,
    value: Option<String>,
    status: ResourceStatus,
    /// `PROJECT role` pairs of the profile that use this resource.
    used_by: Vec<String>,
}

fn status_of(binding: &MachineResourceBinding) -> ResourceStatus {
    let path = std::path::Path::new(&binding.value);
    match binding.kind {
        ResourceKind::Directory if path.is_dir() => ResourceStatus::Ok,
        ResourceKind::Application | ResourceKind::Terminal if path.exists() => ResourceStatus::Ok,
        ResourceKind::Directory | ResourceKind::Application | ResourceKind::Terminal => {
            ResourceStatus::Missing
        }
        ResourceKind::Url
            if binding.value.starts_with("https://") || binding.value.starts_with("http://") =>
        {
            ResourceStatus::Ok
        }
        ResourceKind::Url => ResourceStatus::Missing,
        ResourceKind::DockerEnvironment | ResourceKind::Service | ResourceKind::Other => {
            ResourceStatus::Unchecked
        }
    }
}

/// Checks every machine binding and every resource the profile's projects
/// refer to.
#[tauri::command]
pub fn check_resources(
    state: State<'_, AppState>,
    profile_id: ProfileId,
) -> CommandResult<Vec<ResourceCheck>> {
    let store = state.store();
    let profile = load_profile(&store, profile_id)?;
    let config = machine_config(&store)?;
    let mut used: BTreeMap<ResourceKey, Vec<String>> = BTreeMap::new();
    for project in &profile.projects {
        for (role, key) in &project.resources {
            used.entry(key.clone())
                .or_default()
                .push(format!("{} {role}", project.key));
        }
    }
    let keys: BTreeSet<ResourceKey> = config
        .bindings
        .iter()
        .map(|b| b.key.clone())
        .chain(used.keys().cloned())
        .collect();
    Ok(keys
        .into_iter()
        .map(|key| {
            let binding = config.binding(&key);
            ResourceCheck {
                kind: binding.map(|b| b.kind),
                value: binding.map(|b| b.value.clone()),
                status: binding.map_or(ResourceStatus::Unbound, status_of),
                used_by: used.remove(&key).unwrap_or_default(),
                key,
            }
        })
        .collect())
}
