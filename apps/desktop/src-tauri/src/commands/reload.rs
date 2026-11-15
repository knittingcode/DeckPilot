//! Saves the agent has read before the command returns.
//!
//! Most configuration commands ask the agent to reload without waiting
//! (`crate::agent::notify_reload`). A save that the frontend, or a test,
//! may act on right away (start the workflow it just saved, press the key
//! of the layout it just saved) waits instead until the agent has
//! acknowledged `ReloadConfiguration`, which it answers only after it has
//! read the configuration again. The wait is bounded: the configuration is
//! already written, so a save never fails because of the agent, and the
//! result says what became of the reload.

use std::time::Duration;

use serde::Serialize;

use deckpilot_ipc::{AgentPaths, ClientError, Request};

/// How long a save waits for the agent's acknowledgement. A reload reads
/// the configuration from the local database: well under a second, also on
/// a busy computer.
pub(crate) const RELOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// What became of the agent's reload after a save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AgentReload {
    /// The agent has read the saved configuration.
    Applied,
    /// No agent runs (or it is stopping); it reads the configuration when
    /// it starts.
    NotRunning,
    /// The agent did not acknowledge in time; it may still read the
    /// configuration later.
    TimedOut,
    /// The agent could not be asked or could not read the configuration.
    Failed { message: String },
}

/// A saved value with the agent's reload. Serialized as the value with an
/// `agentReload` field, so the frontend reads it as before.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved<T> {
    #[serde(flatten)]
    pub value: T,
    pub agent_reload: AgentReload,
}

/// Asks the agent to reload and waits at most `timeout` for it.
pub(crate) async fn reload_agent(paths: &AgentPaths, timeout: Duration) -> AgentReload {
    let request = crate::agent::request(paths, Request::ReloadConfiguration);
    let reload = match tokio::time::timeout(timeout, request).await {
        Ok(Ok(_)) => AgentReload::Applied,
        // A connection closed by the agent means it is stopping.
        Ok(Err(ClientError::NotRunning | ClientError::Closed)) => AgentReload::NotRunning,
        Ok(Err(error)) => AgentReload::Failed {
            message: error.to_string(),
        },
        Err(_) => AgentReload::TimedOut,
    };
    match &reload {
        AgentReload::TimedOut => eprintln!(
            "the agent did not confirm the reload within {} s",
            timeout.as_secs_f32()
        ),
        AgentReload::Failed { message } => eprintln!("the agent did not reload: {message}"),
        AgentReload::Applied | AgentReload::NotRunning => {}
    }
    reload
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths() -> AgentPaths {
        AgentPaths::in_dir(std::env::temp_dir().join(format!(
            "deckpilot-reload-test-{}",
            deckpilot_core::ProfileId::new()
        )))
    }

    #[test]
    fn no_agent_is_not_running() {
        let paths = temp_paths();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let reload = runtime.block_on(reload_agent(&paths, RELOAD_TIMEOUT));
        assert_eq!(reload, AgentReload::NotRunning);
    }

    /// An endpoint that takes the connection and never answers stands in
    /// for a hung agent: the wait ends at the timeout.
    #[cfg(unix)]
    #[test]
    fn a_silent_agent_times_out() {
        let paths = temp_paths();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        deckpilot_ipc::token::create(&paths.token_file).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let reload = runtime.block_on(async {
            let _listener = paths.endpoint().listen().unwrap();
            reload_agent(&paths, Duration::from_millis(200)).await
        });
        assert_eq!(reload, AgentReload::TimedOut);
        let _ = std::fs::remove_dir_all(&paths.data_dir);
    }

    #[test]
    fn saved_is_the_value_with_the_reload() {
        let saved = Saved {
            value: serde_json::json!({ "id": "x", "name": "Flow" }),
            agent_reload: AgentReload::Failed {
                message: "no".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            serde_json::json!({
                "id": "x", "name": "Flow",
                "agentReload": { "state": "failed", "message": "no" }
            })
        );
    }
}
