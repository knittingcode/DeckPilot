//! Commands of the projects module: managing the projects of a profile,
//! switching the active project and checking project resources.
//!
//! Every command names the profile it works on, and every project id is
//! looked up inside that profile, so a command can never change a project of
//! another profile. Copying a project to another profile is an explicit
//! command that creates an independent copy.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tauri::State;

use deckpilot_core::{
    ActiveProjectContext, CycleDirection, MachineResourceBinding, ProfileId, Project, ProjectId,
    ProjectKey, ResourceKey, ResourceKind,
};
use deckpilot_ipc::{ClientError, Request, Response};
use deckpilot_modules::projects::{ProjectCatalog, module_id, switching};
use deckpilot_storage::ConfigStore;

use crate::agent;
use crate::commands::agent::unexpected;
use crate::commands::modify_profile;
use crate::error::{CommandError, CommandResult};
use crate::state::{AppState, load_profile, machine_config};

/// Changes a profile's projects after checking that the module is enabled.
fn modify_projects<T>(
    state: &AppState,
    profile_id: ProfileId,
    change: impl FnOnce(&mut deckpilot_core::Profile) -> CommandResult<T>,
) -> CommandResult<T> {
    state.require_module(&module_id())?;
    modify_profile(state, profile_id, change)
}

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
    modify_projects(&state, profile_id, |profile| {
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
    modify_projects(&state, profile_id, |profile| {
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
    modify_projects(&state, profile_id, |profile| {
        Ok(profile.move_project(project_id, index)?)
    })
}

#[tauri::command]
pub fn delete_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
) -> CommandResult<()> {
    modify_projects(&state, profile_id, |profile| {
        profile.delete_project(project_id)?;
        Ok(())
    })
}

#[tauri::command]
pub fn set_default_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: Option<ProjectId>,
) -> CommandResult<()> {
    modify_projects(&state, profile_id, |profile| {
        Ok(profile.set_default_project(project_id)?)
    })
}

/// Copies a project into another profile. The copy gets a new id and keeps
/// key, name and resource roles; nothing is shared afterwards.
#[tauri::command]
pub fn copy_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
    target_profile_id: ProfileId,
) -> CommandResult<Project> {
    state.require_module(&module_id())?;
    let source = load_profile(&state.store(), profile_id)?;
    modify_profile(&state, target_profile_id, |target| {
        let id = source.copy_project_to(project_id, target)?;
        target
            .project(id)
            .cloned()
            .ok_or(CommandError::NotFound("project"))
    })
}

/// Switching acts on the active profile, which is what the agent runs.
fn ensure_active(state: &AppState, profile_id: ProfileId) -> CommandResult<()> {
    state.require_module(&module_id())?;
    if state.store().active_profile()? == Some(profile_id) {
        Ok(())
    } else {
        Err(CommandError::Invalid("the profile is not active".into()))
    }
}

#[tauri::command]
pub async fn cycle_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    direction: CycleDirection,
) -> CommandResult<Option<ProjectId>> {
    ensure_active(&state, profile_id)?;
    match agent::request(&state.paths, Request::CycleProject { direction }).await {
        Ok(Response::ActiveProject { project }) => Ok(project.map(|p| p.id)),
        Ok(other) => Err(unexpected(&other)),
        Err(ClientError::NotRunning) => {
            let wrap = switching::wrap_around(&state.modules());
            let store = state.store();
            let profile = load_profile(&store, profile_id)?;
            let context =
                ActiveProjectContext::restore(&profile, store.active_project(profile_id)?);
            let active = switching::step(&profile, context.project_id(), direction, wrap)
                .map_err(|e| CommandError::Invalid(e.to_string()))?;
            store.set_active_project(profile_id, Some(active))?;
            Ok(Some(active))
        }
        Err(error) => Err(error.into()),
    }
}

#[tauri::command]
pub async fn select_project(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    project_id: ProjectId,
) -> CommandResult<()> {
    ensure_active(&state, profile_id)?;
    // Ownership is checked here as well as in the agent.
    let profile = load_profile(&state.store(), profile_id)?;
    if profile.project(project_id).is_none() {
        return Err(CommandError::NotFound("project"));
    }
    match agent::request(
        &state.paths,
        Request::SelectProject {
            project: project_id,
        },
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(ClientError::NotRunning) => Ok(state
            .store()
            .set_active_project(profile_id, Some(project_id))?),
        Err(error) => Err(error.into()),
    }
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
    state.require_module(&module_id())?;
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
