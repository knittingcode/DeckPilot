use serde::Serialize;
use tauri::State;

use deckpilot_core::{ActionType, ActiveProjectContext, Os, Project};
use deckpilot_device::DeviceModel;
use deckpilot_storage::{ConfigStore, LaunchInfo, ProfileSummary};

use crate::error::CommandResult;
use crate::state::{AppState, load_profile, machine_config};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    app_version: &'static str,
    os: Os,
    database_path: String,
    development: bool,
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
        database_path: state.paths.database.display().to_string(),
        development: std::env::var_os("DECKPILOT_DEV").is_some(),
        profile_count: profiles.len(),
        active_profile,
        active_project,
        action_type_count: state.registry.action_types().count(),
        device_model_count: state.catalog.iter().count(),
        resource_binding_count: machine_config(&store)?.bindings.len(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionState {
    /// The database is encrypted. Not available yet; see the roadmap.
    enabled: bool,
    /// A password must be entered before the configuration can be read.
    password_required: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchView {
    launch: Option<LaunchInfo>,
    encryption: EncryptionState,
}

/// How this start relates to the previous one. After an update, an
/// encrypted database would require the password here; encryption is not
/// enabled yet, so no prompt is needed.
#[tauri::command]
pub fn launch_info(state: State<'_, AppState>) -> LaunchView {
    LaunchView {
        launch: state.launch.clone(),
        encryption: EncryptionState {
            enabled: false,
            password_required: false,
        },
    }
}

#[tauri::command]
pub fn list_action_types(state: State<'_, AppState>) -> Vec<ActionType> {
    state.registry.action_types().cloned().collect()
}

#[tauri::command]
pub fn list_device_models(state: State<'_, AppState>) -> Vec<DeviceModel> {
    state.catalog.iter().cloned().collect()
}
