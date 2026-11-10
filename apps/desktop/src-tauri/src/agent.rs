//! Communication with the background agent.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use deckpilot_ipc::client::request_once;
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, AgentStatus, ClientError, ClientKind, ErrorCode,
    KeyMaterial, Request, Response, SecurityState,
};
use deckpilot_storage::DatabaseKey;
use deckpilot_update::compatibility::{self, AgentCompatibility};
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
    /// Whether the running agent belongs to this application version.
    pub compatibility: Option<AgentCompatibility>,
    /// Version of this application.
    pub app_version: &'static str,
}

impl AgentConnection {
    pub fn stopped() -> Self {
        Self {
            running: false,
            status: None,
            can_start: agent_executable().is_some(),
            compatibility: None,
            app_version: APP_VERSION,
        }
    }

    fn running(status: AgentStatus) -> Self {
        Self {
            running: true,
            compatibility: Some(compatibility::check(APP_VERSION, &status.version)),
            status: Some(status),
            can_start: agent_executable().is_some(),
            app_version: APP_VERSION,
        }
    }

    /// An agent that only speaks an older protocol (from a previous release).
    fn outdated() -> Self {
        Self {
            running: true,
            status: None,
            can_start: agent_executable().is_some(),
            compatibility: Some(AgentCompatibility::AgentOutdated {
                agent: "unknown (older protocol)".into(),
                app: APP_VERSION.into(),
            }),
            app_version: APP_VERSION,
        }
    }
}

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Stops a running agent of this or an older release. Older agents reject
/// the current protocol, so the request is repeated with protocol 1.
pub async fn stop_any_version(paths: &AgentPaths, request: Request) -> Result<(), ClientError> {
    let outcome = match self::request(paths, request.clone()).await {
        Err(ClientError::Agent(error)) if error.code == ErrorCode::UnsupportedProtocol => {
            let mut client =
                AgentClient::connect_with_protocol(paths, ClientKind::Desktop, 1).await?;
            // Protocol 1 has no PrepareForUpdate; Shutdown has the same effect there.
            client.request(Request::Shutdown).await.map(|_| ())
        }
        other => other.map(|_| ()),
    };
    match outcome {
        Ok(()) | Err(ClientError::NotRunning | ClientError::Closed) => Ok(()),
        Err(error) => Err(error),
    }
}

pub async fn wait_until_stopped(paths: &AgentPaths, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if matches!(connection(paths).await, Ok(c) if !c.running) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn request(paths: &AgentPaths, request: Request) -> Result<Response, ClientError> {
    request_once(paths, ClientKind::Desktop, request).await
}

/// Unlocks a running agent with `key`. No agent is fine: the bridge unlocks
/// it when it starts, as long as this app is unlocked.
pub async fn unlock(paths: &AgentPaths, key: &DatabaseKey) -> Result<(), ClientError> {
    match request(
        paths,
        Request::Unlock {
            key: KeyMaterial::new(key.to_hex()),
        },
    )
    .await
    {
        Ok(_) | Err(ClientError::NotRunning) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Locks a running agent, e.g. before the database file is replaced.
pub async fn lock(paths: &AgentPaths) -> Result<(), ClientError> {
    match request(paths, Request::Lock).await {
        Ok(_) | Err(ClientError::NotRunning) => Ok(()),
        // Agents before protocol 4 hold no key.
        Err(ClientError::Agent(error)) if error.code == ErrorCode::InvalidRequest => Ok(()),
        Err(error) => Err(error),
    }
}

/// The database key of an unlocked agent, so that restarting the app does
/// not ask for the password again.
pub async fn fetch_key(paths: &AgentPaths) -> Option<DatabaseKey> {
    match request(paths, Request::GetDatabaseKey).await {
        Ok(Response::DatabaseKey { key }) => DatabaseKey::from_hex(key.expose()).ok(),
        _ => None,
    }
}

pub async fn connection(paths: &AgentPaths) -> Result<AgentConnection, ClientError> {
    match request(paths, Request::GetStatus).await {
        Ok(Response::Status { status }) => Ok(AgentConnection::running(*status)),
        Ok(other) => Err(ClientError::Protocol(format!("{other:?}"))),
        // A connection closed during the handshake means the agent is stopping.
        Err(ClientError::NotRunning | ClientError::Closed) => Ok(AgentConnection::stopped()),
        Err(ClientError::Agent(error)) if error.code == ErrorCode::UnsupportedProtocol => {
            Ok(AgentConnection::outdated())
        }
        Err(error) => Err(error),
    }
}

/// Asks a running agent to re-read configuration. Best effort: when the agent
/// is not running it reads the configuration on its next start.
/// Asks a running agent to reload and waits until it has. Used where acting
/// on stale configuration would be wrong, e.g. after disabling a module.
pub async fn reload(paths: &AgentPaths) -> Result<(), ClientError> {
    match request(paths, Request::ReloadConfiguration).await {
        Ok(_) | Err(ClientError::NotRunning) => Ok(()),
        Err(error) => Err(error),
    }
}

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
        unlock_if_needed(app, paths, &status).await;
        let _ = app.emit(STATUS_EVENT, AgentConnection::running(*status));
    }
    client.subscribe().await?;
    loop {
        match client.next_event().await? {
            AgentEvent::State { status } => {
                unlock_if_needed(app, paths, &status).await;
                let _ = app.emit(STATUS_EVENT, AgentConnection::running(*status));
            }
            AgentEvent::ShowWindow => show_window(app),
            AgentEvent::OpenWindow { window } => {
                // Only an identifier arrives; the catalog decides.
                if let Some(state) = app.try_state::<crate::state::AppState>()
                    && let Err(error) = crate::commands::windows::open_by_id(app, &state, &window)
                {
                    eprintln!("the agent asked for window `{window}`: {error}");
                }
            }
            AgentEvent::ShuttingDown => return Ok(()),
            AgentEvent::ActionCompleted { .. } => {}
        }
    }
}

/// Keeps this app's lock state in line with the agent's:
///
/// - An agent that (re)started locked while this app is unlocked gets the
///   key (password protection), e.g. after a restart from the tray.
/// - An agent that locked itself (auto-lock) or was locked elsewhere locks
///   this app too; the windows then show the prompt.
async fn unlock_if_needed<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AgentPaths,
    status: &AgentStatus,
) {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    // Opened elsewhere (the password entered in another DeckPilot window,
    // the system key read again): follow the agent. The security policy
    // still applies — `attach` asks for confirmation where it requires it.
    if status.security == SecurityState::Unlocked && !state.is_unlocked() {
        if let Some(key) = fetch_key(paths).await
            && state.attach(key, false).is_ok()
        {
            if !state.confirmation_pending() {
                crate::windows::close_security_prompts(app);
            }
            let _ = app.emit(crate::commands::windows::SECURITY_CHANGED, ());
        }
        return;
    }
    if status.security != SecurityState::Locked || !state.is_unlocked() {
        return;
    }
    // Only an agent that (re)started gets the key back. A lock by the user
    // or a policy — including reasons added later — locks this app too, so
    // a lock is never undone silently.
    if crate::commands::security::lock_needs_authentication(status.lock_reason) {
        state.detach();
        let _ = app.emit(crate::commands::windows::SECURITY_CHANGED, ());
    } else if let Some(key) = state.key()
        && let Err(error) = unlock(paths, &key).await
    {
        eprintln!("could not unlock the agent: {error}");
    }
}

/// Brings the app to the front: the main window when unlocked, otherwise
/// the password prompt.
fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<crate::state::AppState>()
        && !state.is_unlocked()
    {
        let id = crate::commands::security::prompt_window(&state);
        if let Err(error) = crate::commands::windows::open_by_id(app, &state, id) {
            eprintln!("cannot open the password prompt: {error}");
        }
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
