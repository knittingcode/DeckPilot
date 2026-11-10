//! Communication with the background agent.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use deckpilot_ipc::client::request_once;
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, AgentStatus, ClientError, ClientKind, Request, Response,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Event emitted to the frontend whenever the agent's state is known to change.
pub const STATUS_EVENT: &str = "agent-status";

/// Overrides the agent executable.
const AGENT_BIN_ENV: &str = "DECKPILOT_AGENT_BIN";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConnection {
    pub running: bool,
    pub status: Option<AgentStatus>,
    /// Whether an agent executable was found to start.
    pub can_start: bool,
}

impl AgentConnection {
    pub fn stopped() -> Self {
        Self {
            running: false,
            status: None,
            can_start: agent_executable().is_some(),
        }
    }

    fn running(status: AgentStatus) -> Self {
        Self {
            running: true,
            status: Some(status),
            can_start: agent_executable().is_some(),
        }
    }
}

pub async fn request(paths: &AgentPaths, request: Request) -> Result<Response, ClientError> {
    request_once(paths, ClientKind::Desktop, request).await
}

pub async fn connection(paths: &AgentPaths) -> Result<AgentConnection, ClientError> {
    match request(paths, Request::GetStatus).await {
        Ok(Response::Status { status }) => Ok(AgentConnection::running(*status)),
        Ok(other) => Err(ClientError::Protocol(format!("{other:?}"))),
        // A connection closed during the handshake means the agent is stopping.
        Err(ClientError::NotRunning | ClientError::Closed) => Ok(AgentConnection::stopped()),
        Err(error) => Err(error),
    }
}

/// Asks a running agent to re-read configuration. Best effort: when the agent
/// is not running it reads the configuration on its next start.
pub fn notify_reload(paths: &AgentPaths) {
    let paths = paths.clone();
    tauri::async_runtime::spawn(async move {
        match request(&paths, Request::ReloadConfiguration).await {
            Ok(_) | Err(ClientError::NotRunning) => {}
            Err(error) => eprintln!("could not notify the agent: {error}"),
        }
    });
}

/// `DECKPILOT_AGENT_BIN`, or `deckpilot-agent` next to this executable.
pub fn agent_executable() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(AGENT_BIN_ENV).map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let path = dir.join(format!("deckpilot-agent{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

/// Starts the agent detached from this process, so closing the window does
/// not stop it. The agent itself refuses to run twice.
pub fn start_agent() -> Result<(), String> {
    let path = agent_executable().ok_or("the DeckPilot agent executable was not found")?;
    let mut command = Command::new(&path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start {}: {e}", path.display()))
}

/// Waits until a freshly started agent accepts connections.
pub async fn wait_until_running(paths: &AgentPaths, timeout: Duration) -> AgentConnection {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(connection) = connection(paths).await
            && connection.running
        {
            return connection;
        }
        if tokio::time::Instant::now() >= deadline {
            return AgentConnection::stopped();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Keeps a subscription to the agent and forwards its state to the window.
/// Reconnects automatically when the agent restarts.
pub fn spawn_bridge<R: Runtime>(app: AppHandle<R>, paths: AgentPaths) {
    tauri::async_runtime::spawn(async move {
        let mut announced_stopped = false;
        loop {
            match subscribe(&app, &paths).await {
                Ok(()) | Err(ClientError::Closed | ClientError::NotRunning) => {}
                Err(error) => eprintln!("agent connection failed: {error}"),
            }
            if !announced_stopped {
                let _ = app.emit(STATUS_EVENT, AgentConnection::stopped());
            }
            announced_stopped = true;
            tokio::time::sleep(RECONNECT_DELAY).await;
            if let Ok(connection) = connection(&paths).await {
                announced_stopped = !connection.running;
            }
        }
    });
}

async fn subscribe<R: Runtime>(app: &AppHandle<R>, paths: &AgentPaths) -> Result<(), ClientError> {
    let mut client = AgentClient::connect(paths, ClientKind::Desktop).await?;
    if let Response::Status { status } = client.request(Request::GetStatus).await? {
        let _ = app.emit(STATUS_EVENT, AgentConnection::running(*status));
    }
    client.subscribe().await?;
    loop {
        match client.next_event().await? {
            AgentEvent::State { status } => {
                let _ = app.emit(STATUS_EVENT, AgentConnection::running(*status));
            }
            AgentEvent::ShowWindow => show_window(app),
            AgentEvent::ShuttingDown => return Ok(()),
            AgentEvent::ActionCompleted { .. } => {}
        }
    }
}

fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
