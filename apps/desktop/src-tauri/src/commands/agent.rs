use std::time::Duration;

use tauri::State;

use deckpilot_core::{
    ActionInstanceId, ActionResult, ExecutionId, ExecutionRecord, ExecutionSummary, ProfileId,
    WorkflowId, WorkflowReport,
};
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

/// Stops the running agent (of any version or channel) and starts the one
/// shipped with this application, e.g. after an update, when versions
/// differ, or when the agent came from another installation (rule R6).
///
/// Workflows running in the agent would be interrupted, so the restart is
/// refused while any run unless `interrupt_running` confirms it; the
/// interrupted runs are then recorded as such in the history.
#[tauri::command]
pub async fn restart_agent(
    state: State<'_, AppState>,
    interrupt_running: Option<bool>,
) -> CommandResult<AgentConnection> {
    let current = agent::connection(&state.paths).await?;
    if let Some(running) = current.running_executions().filter(|n| *n > 0)
        && !interrupt_running.unwrap_or(false)
    {
        return Err(agent::running_workflows_error(running));
    }
    agent::stop_any_version(&state.paths, Request::Shutdown).await?;
    // Started only once the old agent has released its instance lock, so
    // the two never run at the same time.
    agent::wait_until_released(&state.paths, Duration::from_secs(10))
        .await
        .map_err(CommandError::Invalid)?;
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
        Err(ClientError::NotRunning) => Ok(state.store()?.set_active_profile(Some(profile_id))?),
        Err(error) => Err(error.into()),
    }
}

/// Actions and workflows run in the active profile, which is what the agent
/// runs.
fn ensure_active(state: &AppState, profile_id: ProfileId) -> CommandResult<()> {
    if state.store()?.active_profile()? == Some(profile_id) {
        Ok(())
    } else {
        Err(CommandError::invalid(
            "profileNotActive",
            "the profile is not active",
        ))
    }
}

fn agent_required(error: ClientError) -> CommandError {
    match error {
        ClientError::NotRunning => CommandError::invalid(
            "actionsAgentNotRunning",
            "start the DeckPilot agent to run actions",
        ),
        other => other.into(),
    }
}

/// What a simulated press did, for the editor's interactive preview
/// (`docs/editor-navigation-and-simulation.md`): the agent's answer, or a
/// refusal of this app's own (`agentNotRunning`; `unsupported` for an agent
/// before protocol 14, which is never asked).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SimulatePressResult {
    /// The press went through the agent's gesture engine and dispatch;
    /// `dispatched`: the triggers that ran, in order.
    Ran {
        dispatched: Vec<deckpilot_ipc::simulation::SimulatedTrigger>,
    },
    /// Nothing ran: ask the user, then send it again with `confirmed`.
    #[serde(rename_all = "camelCase")]
    ConfirmationRequired {
        action_names: Vec<String>,
        reason: String,
    },
    /// Nothing ran. `code`: the agent's
    /// [`SimulationRefusal`](deckpilot_ipc::simulation::SimulationRefusal)
    /// name, `agentNotRunning` or `unsupported`.
    Refused { code: String, message: String },
}

impl From<deckpilot_ipc::simulation::SimulationOutcome> for SimulatePressResult {
    fn from(outcome: deckpilot_ipc::simulation::SimulationOutcome) -> Self {
        use deckpilot_ipc::simulation::SimulationOutcome as O;
        match outcome {
            O::Ran { dispatched } => Self::Ran { dispatched },
            O::ConfirmationRequired {
                action_names,
                reason,
            } => Self::ConfirmationRequired {
                action_names,
                reason,
            },
            O::Refused { code, message } => Self::Refused {
                code: code.as_str().to_owned(),
                message,
            },
        }
    }
}

/// The answer to a simulated press, from what the agent said.
pub(crate) fn simulate_press_result(
    answer: Result<Response, ClientError>,
) -> CommandResult<SimulatePressResult> {
    let refused = |code: &str, message: String| {
        Ok(SimulatePressResult::Refused {
            code: code.to_owned(),
            message,
        })
    };
    match answer {
        Ok(Response::SimulatedPress { outcome }) => Ok(outcome.into()),
        Ok(other) => Err(unexpected(&other)),
        Err(ClientError::NotRunning) => refused(
            "agentNotRunning",
            "start the DeckPilot agent to try keys".into(),
        ),
        Err(error @ ClientError::Unsupported { .. }) => refused("unsupported", error.to_string()),
        Err(error) => Err(error.into()),
    }
}

/// Presses `control` of the connected `device` in the agent as the user
/// would (`kind`: single, double or long), while the device shows `page` of
/// `layout` in `profile_id`; see [`SimulatePressResult`]. The agent runs it
/// through its own gesture engine and dispatch and never writes to the
/// device but the key images the binding changes. A press that may change
/// something outside DeckPilot answers `confirmationRequired` unless
/// `confirmed`; the agent's settings still gate those actions.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri passes each argument by name.
pub async fn simulate_press(
    state: State<'_, AppState>,
    device: deckpilot_device::DeviceId,
    profile: ProfileId,
    layout: deckpilot_core::LayoutId,
    page: deckpilot_core::PageId,
    control: deckpilot_device::ControlId,
    kind: deckpilot_ipc::simulation::PressKind,
    confirmed: Option<bool>,
) -> CommandResult<SimulatePressResult> {
    let request = Request::SimulatePress {
        device,
        profile,
        layout,
        page,
        control,
        kind,
        confirmed: confirmed.unwrap_or(false),
    };
    simulate_press_result(agent::request(&state.paths, request).await)
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

/// Starts a workflow of the active profile in the agent. It keeps running
/// when the app closes; progress arrives as execution events.
#[tauri::command]
pub async fn start_workflow(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    workflow_id: WorkflowId,
) -> CommandResult<ExecutionSummary> {
    ensure_active(&state, profile_id)?;
    match agent::request(
        &state.paths,
        Request::StartWorkflow {
            workflow: workflow_id,
        },
    )
    .await
    .map_err(agent_required)?
    {
        Response::ExecutionStarted { execution } => Ok(execution),
        other => Err(unexpected(&other)),
    }
}

#[tauri::command]
pub async fn cancel_execution(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
) -> CommandResult<()> {
    match agent::request(
        &state.paths,
        Request::CancelExecution {
            execution: execution_id,
        },
    )
    .await
    .map_err(agent_required)?
    {
        Response::Ok => Ok(()),
        other => Err(unexpected(&other)),
    }
}

/// Running and recorded runs of the active profile, newest first. Empty
/// while the agent is not running.
#[tauri::command]
pub async fn list_executions(
    state: State<'_, AppState>,
    workflow_id: Option<WorkflowId>,
    limit: Option<u32>,
) -> CommandResult<Vec<ExecutionSummary>> {
    state.store()?;
    match agent::request(
        &state.paths,
        Request::ListExecutions {
            workflow: workflow_id,
            limit,
        },
    )
    .await
    {
        Ok(Response::Executions { executions }) => Ok(executions),
        Ok(other) => Err(unexpected(&other)),
        Err(ClientError::NotRunning) => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

#[tauri::command]
pub async fn get_execution(
    state: State<'_, AppState>,
    execution_id: ExecutionId,
) -> CommandResult<ExecutionRecord> {
    state.store()?;
    match agent::request(
        &state.paths,
        Request::GetExecution {
            execution: execution_id,
        },
    )
    .await
    .map_err(agent_required)?
    {
        Response::Execution { execution } => Ok(*execution),
        other => Err(unexpected(&other)),
    }
}

#[tauri::command]
pub async fn clear_execution_history(
    state: State<'_, AppState>,
    workflow_id: Option<WorkflowId>,
) -> CommandResult<()> {
    state.store()?;
    match agent::request(
        &state.paths,
        Request::ClearExecutionHistory {
            workflow: workflow_id,
        },
    )
    .await
    .map_err(agent_required)?
    {
        Response::Ok => Ok(()),
        other => Err(unexpected(&other)),
    }
}

/// The report of the last workflow the agent ran, for the summary window.
#[tauri::command]
pub async fn last_workflow_report(
    state: State<'_, AppState>,
) -> CommandResult<Option<WorkflowReport>> {
    state.store()?;
    match agent::request(&state.paths, Request::GetLastWorkflowReport)
        .await
        .map_err(agent_required)?
    {
        Response::LastWorkflow { report } => Ok(report),
        other => Err(unexpected(&other)),
    }
}

#[cfg(test)]
mod simulate_tests {
    use deckpilot_ipc::simulation::{SimulatedTrigger, SimulationOutcome, SimulationRefusal};

    use super::*;

    #[test]
    fn simulated_presses_answer_ran_confirmation_or_refused() {
        let ran = simulate_press_result(Ok(Response::SimulatedPress {
            outcome: SimulationOutcome::Ran {
                dispatched: vec![SimulatedTrigger::DoubleClick],
            },
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&ran).unwrap(),
            serde_json::json!({"status": "ran", "dispatched": ["doubleClick"]})
        );
        let ask = simulate_press_result(Ok(Response::SimulatedPress {
            outcome: SimulationOutcome::ConfirmationRequired {
                action_names: vec!["Deploy".into()],
                reason: "changes".into(),
            },
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&ask).unwrap(),
            serde_json::json!({"status": "confirmationRequired", "actionNames": ["Deploy"],
                "reason": "changes"})
        );
        let refused = simulate_press_result(Ok(Response::SimulatedPress {
            outcome: SimulationOutcome::refused(SimulationRefusal::WrongPage, "another page"),
        }))
        .unwrap();
        assert_eq!(
            refused,
            SimulatePressResult::Refused {
                code: "wrongPage".into(),
                message: "another page".into()
            }
        );
        // An agent before protocol 14 is never asked; no agent at all.
        let older = simulate_press_result(Err(ClientError::Unsupported {
            request: "simulatePress".into(),
            needs: 14,
            protocol: 13,
            agent: "0.9.0".into(),
        }))
        .unwrap();
        assert!(
            matches!(older, SimulatePressResult::Refused { code, .. } if code == "unsupported")
        );
        let none = simulate_press_result(Err(ClientError::NotRunning)).unwrap();
        assert!(
            matches!(none, SimulatePressResult::Refused { code, .. } if code == "agentNotRunning")
        );
        assert!(simulate_press_result(Ok(Response::Ok)).is_err());
    }
}
