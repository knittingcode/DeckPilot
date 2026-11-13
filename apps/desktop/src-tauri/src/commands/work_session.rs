//! The work session (protocol 8, docs/gestures-and-work-session.md §9). The
//! agent owns the session; these commands read it, its history and
//! settings, clear notices and decide flagged gaps. There is deliberately
//! no command that starts, verifies or finishes a session: only its key
//! does (D11). With an agent below protocol 8 every command answers "not
//! supported", which the page explains.

use serde::Deserialize;
use tauri::State;

use deckpilot_core::work_session::WorkSessionSnapshot;
use deckpilot_core::{ProfileId, WorkSessionId, WorkflowId};
use deckpilot_ipc::work_session::{ElapsedFormat, WorkSessionSettings, WorkSessionSummary};
use deckpilot_ipc::{ClientError, Request, Response};

use super::agent::unexpected;
use crate::agent;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Asks the agent; an agent that is not running or too old gets a message
/// the page can show (it recognises "not supported").
async fn ask(state: &AppState, request: Request) -> CommandResult<Response> {
    state.store()?;
    agent::request(&state.paths, request)
        .await
        .map_err(agent_error)
}

fn agent_error(error: ClientError) -> CommandError {
    match error {
        ClientError::NotRunning => {
            CommandError::Invalid("start the DeckPilot agent to see the work session".into())
        }
        ClientError::Unsupported { agent, .. } => CommandError::Invalid(format!(
            "work sessions are not supported by the running agent ({agent}); restart the \
             agent to use the one of this version"
        )),
        other => other.into(),
    }
}

/// The work session as it is now. Elapsed time is `elapsedMs` at `asOfMs`;
/// the page counts on from there and the `agent-work-session` event
/// brings every change.
#[tauri::command]
pub async fn get_work_session(state: State<'_, AppState>) -> CommandResult<WorkSessionSnapshot> {
    match ask(&state, Request::GetWorkSession).await? {
        Response::WorkSession { snapshot } => Ok(*snapshot),
        other => Err(unexpected(&other)),
    }
}

/// Sessions of every profile, newest first; `before` (ms) pages back.
#[tauri::command]
pub async fn list_work_sessions(
    state: State<'_, AppState>,
    limit: u32,
    before: Option<i64>,
) -> CommandResult<Vec<WorkSessionSummary>> {
    match ask(&state, Request::ListWorkSessions { limit, before }).await? {
        Response::WorkSessions { entries } => Ok(entries),
        other => Err(unexpected(&other)),
    }
}

/// Keeps a flagged gap counted (`count`) or excludes it (D8); either can be
/// changed again. Returns the session afterwards.
#[tauri::command]
pub async fn resolve_work_session_review(
    state: State<'_, AppState>,
    session: WorkSessionId,
    review: i64,
    count: bool,
) -> CommandResult<WorkSessionSummary> {
    let request = Request::ResolveWorkSessionReview {
        session,
        review,
        count,
    };
    match ask(&state, request).await? {
        Response::WorkSessionEntry { entry } => Ok(*entry),
        other => Err(unexpected(&other)),
    }
}

/// Clears the notice of `session`, or the one an idle session shows.
#[tauri::command]
pub async fn clear_work_session_notices(
    state: State<'_, AppState>,
    session: Option<WorkSessionId>,
) -> CommandResult<WorkSessionSnapshot> {
    match ask(&state, Request::ClearWorkSessionNotices { session }).await? {
        Response::WorkSession { snapshot } => Ok(*snapshot),
        other => Err(unexpected(&other)),
    }
}

/// The work session settings of a profile (defaults if never saved).
#[tauri::command]
pub async fn get_work_session_settings(
    state: State<'_, AppState>,
    profile_id: ProfileId,
) -> CommandResult<WorkSessionSettings> {
    let request = Request::GetWorkSessionSettings {
        profile: profile_id,
    };
    match ask(&state, request).await? {
        Response::WorkSessionSettings { settings } => Ok(*settings),
        other => Err(unexpected(&other)),
    }
}

/// The settings as the page sends them. Unknown fields are refused, so a
/// setting the agent does not store is never silently dropped.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsInput {
    #[serde(default)]
    start_workflow: Option<WorkflowId>,
    #[serde(default)]
    verification_workflow: Option<WorkflowId>,
    approval_ttl_seconds: u32,
    review_gap_seconds: u32,
    #[serde(default)]
    elapsed_format: ElapsedFormat,
    /// Echoed from an earlier answer; `profileId` decides.
    #[serde(default, rename = "profile")]
    _profile: Option<ProfileId>,
    /// Echoed from an earlier answer; set by the agent.
    #[serde(default, rename = "updatedAtMs")]
    _updated_at_ms: Option<i64>,
}

/// Saves the settings of a profile. The agent checks the workflows against
/// the profile (start: no inputs a session cannot give; verification: the
/// `canFinish` contract) and the limits before it writes, and drops an
/// approval when they changed (D9).
#[tauri::command]
pub async fn save_work_session_settings(
    state: State<'_, AppState>,
    profile_id: ProfileId,
    settings: SettingsInput,
) -> CommandResult<WorkSessionSettings> {
    let settings = WorkSessionSettings {
        profile: profile_id,
        start_workflow: settings.start_workflow,
        verification_workflow: settings.verification_workflow,
        approval_ttl_seconds: settings.approval_ttl_seconds,
        review_gap_seconds: settings.review_gap_seconds,
        elapsed_format: settings.elapsed_format,
        updated_at_ms: None,
    };
    let request = Request::SaveWorkSessionSettings {
        settings: Box::new(settings),
    };
    match ask(&state, request).await? {
        Response::WorkSessionSettings { settings } => Ok(*settings),
        other => Err(unexpected(&other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_agent_is_reported_as_not_supported() {
        let error = agent_error(ClientError::Unsupported {
            request: "getWorkSession".into(),
            needs: 8,
            protocol: 7,
            agent: "0.6.0".into(),
        });
        // The page recognises this wording (`isNotSupported`).
        assert!(error.to_string().contains("not supported"), "{error}");
        assert!(error.to_string().contains("0.6.0"), "{error}");
        let error = agent_error(ClientError::NotRunning);
        assert!(!error.to_string().contains("not supported"), "{error}");
    }

    #[test]
    fn settings_fields_the_agent_does_not_store_are_refused() {
        let input = |extra: serde_json::Value| {
            let mut json = serde_json::json!({
                "approvalTtlSeconds": 600,
                "reviewGapSeconds": 900,
            });
            json.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value::<SettingsInput>(json)
        };
        let plain = input(serde_json::json!({})).unwrap();
        assert_eq!(plain.start_workflow, None);
        assert_eq!(plain.elapsed_format, ElapsedFormat::Minutes);
        assert!(input(serde_json::json!({"profile": ProfileId::new(), "updatedAtMs": 5})).is_ok());
        assert!(input(serde_json::json!({"longPressMs": 800})).is_err());
    }
}
