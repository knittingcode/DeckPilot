//! Commands invoked by the frontend.
//!
//! Configuration commands load a profile, apply one change through the domain
//! model, save it in a single write and ask the agent to reload. Runtime
//! state (active profile and project) and execution go through the agent
//! while it runs; when it does not, runtime state is written to storage and
//! the agent reads it on its next start.

// Tauri injects `State` and deserialized arguments by value.
#![allow(clippy::needless_pass_by_value)]

mod actions;
pub(crate) mod agent;
mod layouts;
mod machine;
mod modules;
pub(crate) mod portability;
mod profiles;
pub(crate) mod security;
mod system;
pub(crate) mod windows;
mod workflows;

use crate::modules::projects;

use deckpilot_core::{Profile, ProfileId};

use crate::error::CommandResult;
use crate::state::{AppState, load_profile};

/// Invoke handler with every command exposed to the frontend.
pub fn handler<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
{
    tauri::generate_handler![
        security::security_status,
        security::setup_password,
        security::unlock,
        security::confirm_password,
        security::encrypt_legacy_database,
        security::change_password,
        security::lock,
        security::remove_unencrypted_copies,
        windows::open_window,
        windows::current_window,
        windows::fit_window,
        windows::close_window,
        windows::show_main_window,
        windows::finish_security_prompt,
        agent::last_workflow_report,
        portability::export_configuration,
        portability::preview_import,
        portability::apply_import,
        portability::cancel_import,
        system::get_overview,
        system::launch_info,
        system::list_action_types,
        system::list_device_models,
        system::migration_notes,
        modules::list_modules,
        modules::set_module_enabled,
        modules::set_module_config,
        profiles::list_profiles,
        profiles::get_profile,
        profiles::create_profile,
        profiles::update_profile_details,
        profiles::duplicate_profile,
        profiles::delete_profile,
        actions::create_action,
        actions::update_action,
        actions::duplicate_action,
        actions::delete_action,
        workflows::save_workflow,
        workflows::duplicate_workflow,
        workflows::delete_workflow,
        projects::create_project,
        projects::update_project,
        projects::move_project,
        projects::delete_project,
        projects::set_default_project,
        projects::copy_project,
        projects::cycle_project,
        projects::select_project,
        machine::get_machine_config,
        machine::set_resource_binding,
        machine::remove_resource_binding,
        projects::check_resources,
        layouts::create_layout,
        layouts::save_layout,
        layouts::duplicate_layout,
        layouts::delete_layout,
        layouts::activate_layout,
        layouts::preview_layout,
        agent::agent_status,
        agent::start_agent,
        agent::stop_agent,
        agent::restart_agent,
        agent::refresh_devices,
        agent::set_brightness,
        agent::get_autostart,
        agent::set_autostart,
        agent::set_active_profile,
        agent::run_action,
        agent::run_workflow,
        crate::updates::update_state,
        crate::updates::check_for_updates,
        crate::updates::download_update,
        crate::updates::install_update,
        crate::updates::set_update_preferences,
    ]
}

/// Removes surrounding whitespace without reallocating.
fn trim_in_place(value: &mut String) {
    let end = value.trim_end().len();
    value.truncate(end);
    let start = value.len() - value.trim_start().len();
    value.drain(..start);
}

/// Loads a profile, applies `change`, saves it and notifies the agent.
pub(crate) fn modify_profile<T>(
    state: &AppState,
    id: ProfileId,
    change: impl FnOnce(&mut Profile) -> CommandResult<T>,
) -> CommandResult<T> {
    let store = state.store()?;
    let mut profile = load_profile(&store, id)?;
    let result = change(&mut profile)?;
    deckpilot_storage::ConfigStore::save_profile(&*store, &profile)?;
    drop(store);
    crate::agent::notify_reload(&state.paths);
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn trims_in_place() {
        let mut value = String::from("  Start Work \n");
        super::trim_in_place(&mut value);
        assert_eq!(value, "Start Work");
    }
}
