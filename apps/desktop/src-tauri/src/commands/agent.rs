use std::time::Duration;

use tauri::State;

use deckpilot_core::{ActionInstanceId, ActionResult, ProfileId, WorkflowId, WorkflowReport};
use deckpilot_ipc::{ClientError, Request, Response};
use deckpilot_storage::ConfigStore;

use crate::agent::{self, AgentConnection};
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

pub(crate) fn unexpected(response: &Response) -> CommandError {
    CommandError::Invalid(format!("unexpected agent response: {response:?}"))
}

#[tauri::command]
pub async fn agent_status(state: State<'_, AppState>) -> CommandResult<AgentConnection> {
    Ok(agent::connection(&state.paths).await?)
}

#[tauri::command]
pub async fn start_agent(state: State<'_, AppState>) -> CommandResult<AgentConnection> {
    let connection = agent::connection(&state.paths).await?;
    if connection.running {
        return Ok(connection);
    }
    agent::start_agent().map_err(CommandError::Invalid)?;
    Ok(agent::wait_until_running(&state.paths, Duration::from_secs(5)).await)
}

#[tauri::command]
pub async fn stop_agent(state: State<'_, AppState>) -> CommandResult<()> {
    agent::stop_any_version(&state.paths, Request::Shutdown).await?;
    Ok(())
}

/// Stops the running agent (of any version) and starts the one shipped with
/// this application, e.g. after an update or when versions differ.
#[tauri::command]
pub async fn restart_agent(state: State<'_, AppState>) -> CommandResult<AgentConnection> {
    agent::stop_any_version(&state.paths, Request::Shutdown).await?;
    if !agent::wait_until_stopped(&state.paths, Duration::from_secs(10)).await {
        return Err(CommandError::Invalid(
            "the running agent did not stop".into(),
        ));
    }
    agent::start_agent().map_err(CommandError::Invalid)?;
    Ok(agent::wait_until_running(&state.paths, Duration::from_secs(5)).await)
}

#[tauri::command]
pub async fn refresh_devices(state: State<'_, AppState>) -> CommandResult<()> {
    agent::request(&state.paths, Request::RefreshDevices).await?;
    Ok(())
}

#[tauri::command]
pub async fn set_brightness(state: State<'_, AppState>, percent: u8) -> CommandResult<()> {
    agent::request(&state.paths, Request::SetBrightness { percent }).await?;
    Ok(())
}

#[tauri::command]
pub async fn get_autostart(state: State<'_, AppState>) -> CommandResult<bool> {
    match agent::request(&state.paths, Request::GetAutostart).await? {
        Response::Autostart { enabled } => Ok(enabled),
        other => Err(unexpected(&other)),
    }
}

#[tauri::command]
pub async fn set_autostart(state: State<'_, AppState>, enabled: bool) -> CommandResult<bool> {
    match agent::request(&state.paths, Request::SetAutostart { enabled }).await? {
        Response::Autostart { enabled } => Ok(enabled),
        other => Err(unexpected(&other)),
    }
}

#[tauri::command]
pub async fn set_active_profile(
    state: State<'_, AppState>,
    profile_id: ProfileId,
) -> CommandResult<()> {
    match agent::request(
        &state.paths,
        Request::SetActiveProfile {
            profile: profile_id,
        },
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(ClientError::NotRunning) => Ok(state.store().set_active_profile(Some(profile_id))?),
        Err(error) => Err(error.into()),
    }
}

/// Actions and workflows run in the active profile, which is what the agent
/// runs.
fn ensure_active(state: &AppState, profile_id: ProfileId) -> CommandResult<()> {
    if state.store().active_profile()? == Some(profile_id) {
        Ok(())
    } else {
        Err(CommandError::Invalid("the profile is not active".into()))
    }
}

fn agent_required(error: ClientError) -> CommandError {
    match error {
        ClientError::NotRunning => {
            CommandError::Invalid("start the DeckPilot agent to run actions".into())
        }
        other => other.into(),
    }
}

/// Runs an action of the active profile in the agent, which owns execution.
#[tauri::command]
pub async fn run_action(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    action_id: ActionInstanceId,
) -> CommandResult<ActionResult> {
    ensure_active(&state, profile_id)?;
    match agent::request(&state.paths, Request::RunAction { action: action_id })
        .await
        .map_err(agent_required)?
    {
        Response::ActionResult { result } => Ok(result),
        other => Err(unexpected(&other)),
    }
}

#[tauri::command]
pub async fn run_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow_id: WorkflowId,
) -> CommandResult<WorkflowReport> {
    ensure_active(&state, profile_id)?;
    match agent::request(
        &state.paths,
        Request::RunWorkflow {
            workflow: workflow_id,
        },
    )
    .await
    .map_err(agent_required)?
    {
        Response::WorkflowReport { report } => Ok(report),
        other => Err(unexpected(&other)),
    }
}
