//! Commands invoked by the frontend.
//!
//! Configuration commands load a profile, apply one change through the domain
//! model, save it in a single write and ask the agent to reload (saves the
//! user may act on right away wait for it, see `reload`). Runtime
//! state (active profile and project) and execution go through the agent
//! while it runs; when it does not, runtime state is written to storage and
//! the agent reads it on its next start.

// Tauri injects `State` and deserialized arguments by value.
#![allow(clippy::needless_pass_by_value)]

mod actions;
pub(crate) mod agent;
mod credentials;
mod icons;
pub(crate) mod key_states;
mod layouts;
mod logs;
mod machine;
mod modules;
pub(crate) mod plugins;
pub(crate) mod portability;
mod profiles;
pub(crate) mod reload;
pub(crate) mod security;
mod shared_layouts;
mod shared_variables;
mod system;
mod variables;
pub(crate) mod windows;
mod work_session;
mod workflows;

use crate::modules::projects;

use deckpilot_core::{Profile, ProfileId};

use crate::error::CommandResult;
use crate::state::{AppState, load_profile};

/// Invoke handler with every command exposed to the frontend.
#[allow(clippy::too_many_lines)] // One line per command.
pub fn handler<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
{
    tauri::generate_handler![
        security::security_status,
        security::setup_password,
        security::unlock,
        security::confirm_identity,
        security::system_unlock,
        security::authenticate,
        security::use_password,
        security::use_system_key,
        security::set_security_policy,
        security::start_new_configuration,
        security::encrypt_legacy_database,
        security::change_password,
        security::lock,
        security::remove_unencrypted_copies,
        windows::open_window,
        windows::current_window,
        windows::fit_window,
        windows::close_window,
        windows::show_main_window,
        windows::titlebar_double_click,
        windows::finish_security_prompt,
        agent::last_workflow_report,
        portability::export_configuration,
        portability::preview_import,
        portability::apply_import,
        portability::cancel_import,
        system::get_overview,
        system::launch_info,
        logs::read_agent_log,
        system::list_action_types,
        system::list_device_models,
        system::migration_notes,
        modules::list_modules,
        modules::set_module_enabled,
        modules::set_module_config,
        credentials::list_credentials,
        credentials::set_credential,
        credentials::delete_credential,
        shared_variables::list_shared_variables,
        shared_variables::save_shared_variable,
        shared_variables::delete_shared_variable,
        shared_variables::set_shared_variable_value,
        shared_variables::clear_shared_variable_value,
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
        actions::action_references,
        actions::replace_action_references,
        actions::unbind_action_references,
        workflows::save_workflow,
        workflows::duplicate_workflow,
        workflows::delete_workflow,
        workflows::validate_workflow_graph,
        workflows::convert_workflow,
        projects::create_project,
        projects::update_project,
        projects::move_project,
        projects::delete_project,
        projects::set_default_project,
        projects::copy_project,
        projects::cycle_project,
        projects::select_project,
        variables::list_variables,
        variables::preview_references,
        machine::get_machine_config,
        machine::set_resource_binding,
        machine::remove_resource_binding,
        machine::get_gesture_defaults,
        machine::set_gesture_defaults,
        projects::check_resources,
        layouts::create_layout,
        layouts::save_layout,
        layouts::duplicate_layout,
        layouts::delete_layout,
        layouts::activate_layout,
        layouts::preview_layout,
        key_states::watch_key_states,
        layouts::page_tree,
        layouts::reorder_pages,
        layouts::move_page,
        layouts::copy_page,
        shared_layouts::list_shared_layouts,
        shared_layouts::create_shared_layout,
        shared_layouts::rename_shared_layout,
        shared_layouts::duplicate_shared_layout,
        shared_layouts::delete_shared_layout,
        shared_layouts::assign_layout,
        shared_layouts::unassign_layout,
        shared_layouts::reorder_profile_layouts,
        shared_layouts::set_default_layout,
        shared_layouts::profile_layouts,
        icons::list_icons,
        icons::import_icon,
        icons::delete_icon,
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
        agent::start_workflow,
        agent::cancel_execution,
        agent::list_executions,
        agent::get_execution,
        agent::clear_execution_history,
        work_session::get_work_session,
        work_session::list_work_sessions,
        work_session::resolve_work_session_review,
        work_session::clear_work_session_notices,
        work_session::get_work_session_settings,
        work_session::save_work_session_settings,
        plugins::get_plugin_runtime,
        plugins::list_plugins,
        plugins::list_plugin_navigation,
        plugins::get_plugin_navigation_icon,
        plugins::get_plugin,
        plugins::list_available_plugins,
        plugins::install_plugin,
        plugins::enable_plugin,
        plugins::disable_plugin,
        plugins::grant_plugin_permissions,
        plugins::revoke_plugin_permission,
        plugins::update_plugin,
        plugins::approve_plugin_update,
        plugins::decline_plugin_update,
        plugins::rollback_plugin,
        plugins::uninstall_plugin,
        plugins::plan_plugin_install,
        plugins::install_plugin_plan,
        plugins::plan_plugin_removal,
        plugins::remove_plugin,
        plugins::invoke_plugin_action,
        plugins::list_known_plugins,
        plugins::get_plugin_settings,
        plugins::set_plugin_settings,
        plugins::grant_plugin_user_origin,
        plugins::revoke_plugin_user_origin,
        plugins::list_plugin_credentials,
        plugins::set_plugin_credential,
        plugins::delete_plugin_credential,
        plugins::list_module_migrations,
        plugins::decide_module_migration,
        plugins::retry_module_migration,
        plugins::pick_plugin_package,
        plugins::watch_plugin_events,
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
    let result = save_profile_change(state, id, change)?;
    crate::agent::notify_reload(&state.paths);
    Ok(result)
}

/// As `modify_profile`, and returns once the agent has read the change (at
/// most `reload::RELOAD_TIMEOUT` later), so that a request right after the
/// save, such as starting the workflow just saved, sees it. The change is
/// saved whatever the agent answers; the result says what it did.
pub(crate) async fn modify_profile_and_reload<T>(
    state: &AppState,
    id: ProfileId,
    change: impl FnOnce(&mut Profile) -> CommandResult<T>,
) -> CommandResult<reload::Saved<T>> {
    let value = save_profile_change(state, id, change)?;
    let agent_reload = reload::reload_agent(&state.paths, reload::RELOAD_TIMEOUT).await;
    Ok(reload::Saved {
        value,
        agent_reload,
    })
}

fn save_profile_change<T>(
    state: &AppState,
    id: ProfileId,
    change: impl FnOnce(&mut Profile) -> CommandResult<T>,
) -> CommandResult<T> {
    let store = state.store()?;
    let mut profile = load_profile(&store, id)?;
    let result = change(&mut profile)?;
    deckpilot_storage::ConfigStore::save_profile(&*store, &profile)?;
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
