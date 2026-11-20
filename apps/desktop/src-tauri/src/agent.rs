//! Communication with the background agent.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use deckpilot_ipc::client::request_once;
use deckpilot_ipc::paths::RUN_AGENT_ARG;
use deckpilot_ipc::plugins::PluginEvent;
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, AgentStatus, Capability, ClientError, ClientKind,
    ErrorCode, KeyMaterial, Request, Response, SecurityState,
};
use deckpilot_storage::DatabaseKey;
use deckpilot_update::compatibility::{self, AgentCompatibility};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Event emitted to the frontend whenever the agent's state is known to change.
pub const STATUS_EVENT: &str = "agent-status";
/// Event with the progress of workflows running in the agent: the agent's
/// execution events as they are (`executionStarted`, `executionProgress`,
/// `executionFinished`).
pub const EXECUTION_EVENT: &str = "workflow-execution";
/// Event with a notification an action asked the agent to show, for the
/// status bar.
pub const NOTIFICATION_EVENT: &str = "agent-notification";
/// Event with the agent's context changes as they are (`projectChanged`,
/// `profileChanged`, `sharedVariablesChanged`, `resourceBindingsChanged`,
/// `deviceContextChanged`, `executionContextChanged`), so the variable
/// picker refreshes its previews. Agents before protocol 7 send none.
pub const VARIABLES_EVENT: &str = "agent-variables";
/// Event with the work session's snapshot whenever it changes. Agents
/// before protocol 8 send none.
pub const WORK_SESSION_EVENT: &str = "agent-work-session";
/// Event with a plugin operation's progress or a plugin's new state: the
/// agent's `pluginEvent` payload as it is (`progress`, `changed`,
/// `migrationChanged`; `dialogOpened` and `dialogClosed` to the main
/// window only). Only forwarded once the plugin screen asked for it
/// (`watch_plugin_events`, `plugin_dialogs_ready`) and only from agents
/// that answer plugin requests.
pub const PLUGIN_EVENT: &str = "plugin-event";

const RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// How often an agent whose plugins are off is asked again (they come on
/// when the configuration is unlocked or the agent restarts with them).
const PLUGIN_RETRY_DELAY: Duration = Duration::from_secs(5);

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
    /// The protocol of the connection to the agent: the lower of this
    /// app's and the agent's.
    pub protocol: Option<u32>,
    /// What the agent can do over this connection. Features outside it are
    /// unavailable until the agent is restarted with this app's agent.
    pub capabilities: Vec<Capability>,
}

impl AgentConnection {
    pub fn stopped() -> Self {
        Self {
            running: false,
            status: None,
            can_start: agent_executable().is_some(),
            compatibility: None,
            app_version: APP_VERSION,
            protocol: None,
            capabilities: Vec::new(),
        }
    }

    fn running(status: AgentStatus, protocol: u32, capabilities: Vec<Capability>) -> Self {
        Self {
            running: true,
            compatibility: Some(agent_compatibility(&status)),
            status: Some(status),
            can_start: agent_executable().is_some(),
            app_version: APP_VERSION,
            protocol: Some(protocol),
            capabilities,
        }
    }

    /// An agent too old for any protocol this app speaks (DeckPilot 0.3 or
    /// earlier): it can only be stopped and replaced.
    fn update_required() -> Self {
        Self {
            running: true,
            status: None,
            can_start: agent_executable().is_some(),
            compatibility: Some(AgentCompatibility::UpdateRequired {
                agent: "unknown (older protocol)".into(),
                app: APP_VERSION.into(),
            }),
            app_version: APP_VERSION,
            protocol: None,
            capabilities: Vec::new(),
        }
    }

    /// Workflows running in the agent; `None` when it cannot say.
    pub fn running_executions(&self) -> Option<u32> {
        self.status.as_ref().and_then(|s| s.running_executions)
    }
}

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether the agent of `status` belongs to this app: its own distribution
/// channel (rule R6; agents without a record count as GitHub builds), then
/// the same version. Anything else is restarted from this app's bundle.
fn agent_compatibility(status: &AgentStatus) -> AgentCompatibility {
    compatibility::check_agent(
        deckpilot_distribution::current(),
        &status.version,
        status
            .distribution
            .as_ref()
            .map(|record| record.distribution_channel.as_str()),
    )
}

/// Why the agent is not stopped now.
pub fn running_workflows_error(running: u32) -> crate::error::CommandError {
    crate::error::CommandError::invalid_with(
        "workflowsRunning",
        running_workflows_message(running),
        [("count", running.into())],
    )
}

/// The English text of [`running_workflows_error`].
pub fn running_workflows_message(running: u32) -> String {
    format!(
        "{running} workflow{} running in the background; wait until {} finished or cancel {}",
        if running == 1 { " is" } else { "s are" },
        if running == 1 { "it has" } else { "they have" },
        if running == 1 { "it" } else { "them" },
    )
}

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

/// Waits until the agent is gone: its endpoint closed **and** its instance
/// lock released. The endpoint closes before the agent has finished (and
/// released the lock), so the endpoint alone does not prove that a next
/// agent can start, nor that no two agents run at once
/// (docs/distribution/README.md, section 7.1, step 6). The lock is taken
/// and released again at once, so an agent started right after still gets
/// it.
pub async fn wait_until_released(paths: &AgentPaths, timeout: Duration) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    if !wait_until_stopped(paths, timeout).await {
        return Err(format!(
            "the agent did not stop within {} seconds",
            timeout.as_secs()
        ));
    }
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    match deckpilot_ipc::instance::wait_until_free(&paths.lock_file, remaining).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "the agent closed its connection but still holds its instance lock after {} \
             seconds",
            timeout.as_secs()
        )),
        Err(error) => Err(format!(
            "the agent's instance lock cannot be checked: {error}"
        )),
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
    let mut client = match AgentClient::connect(paths, ClientKind::Desktop).await {
        Ok(client) => client,
        // A connection closed during the handshake means the agent is stopping.
        Err(ClientError::NotRunning | ClientError::Closed) => {
            return Ok(AgentConnection::stopped());
        }
        Err(ClientError::Agent(error)) if error.code == ErrorCode::UnsupportedProtocol => {
            return Ok(AgentConnection::update_required());
        }
        Err(error) => return Err(error),
    };
    match client.request(Request::GetStatus).await {
        Ok(Response::Status { status }) => Ok(AgentConnection::running(
            *status,
            client.protocol(),
            client.capabilities().to_vec(),
        )),
        Ok(other) => Err(ClientError::Protocol(format!("{other:?}"))),
        Err(ClientError::NotRunning | ClientError::Closed) => Ok(AgentConnection::stopped()),
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

/// `deckpilot-agent` next to this executable: the agent of this app's own
/// bundle. Debug builds may name another one with `DECKPILOT_AGENT_BIN`
/// (development, the end-to-end tests); release builds do not read that
/// variable at all (`deckpilot_distribution::agent_executable_override`), so
/// nothing in the environment can make them start another program as the
/// agent.
pub fn agent_executable() -> Option<PathBuf> {
    if let Some(path) = deckpilot_distribution::agent_executable_override().map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let path = dir.join(format!("deckpilot-agent{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

/// How to start the agent: inside an AppImage through the AppImage file
/// with [`RUN_AGENT_ARG`], so the agent runs as that AppImage's main process
/// and its files stay mounted as long as it runs (the mount of this app goes
/// away when the app closes); otherwise the executable itself.
fn agent_command(appimage: Option<PathBuf>, executable: PathBuf) -> (PathBuf, Vec<&'static str>) {
    match appimage {
        Some(file) => (file, vec![RUN_AGENT_ARG]),
        None => (executable, Vec::new()),
    }
}

/// Started as `<app> --agent` (by an AppImage's autostart entry, or by
/// [`start_agent`] inside an AppImage): runs the bundled agent in this
/// process's place with the remaining arguments, and does not return.
/// Returns at once for any other start.
pub fn run_agent_if_requested() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(RUN_AGENT_ARG)) {
        return;
    }
    let Some(path) = agent_executable() else {
        eprintln!("the DeckPilot agent executable was not found");
        std::process::exit(1);
    };
    let mut command = Command::new(&path);
    command.args(args);
    #[cfg(unix)]
    {
        // The same process (and so the same AppImage mount) becomes the
        // agent.
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        eprintln!("cannot start {}: {error}", path.display());
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    {
        let code = command.status().ok().and_then(|s| s.code()).unwrap_or(1);
        std::process::exit(code);
    }
}

/// Starts the agent detached from this process, so closing the window does
/// not stop it. The agent itself refuses to run twice.
pub fn start_agent() -> Result<(), String> {
    let executable = agent_executable().ok_or("the DeckPilot agent executable was not found")?;
    let (path, args) = agent_command(deckpilot_ipc::paths::appimage(), executable);
    let mut command = Command::new(&path);
    command
        .args(args)
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
    let (protocol, capabilities) = (client.protocol(), client.capabilities().to_vec());
    let running = |status| AgentConnection::running(status, protocol, capabilities.clone());
    // The app's language (protocol 13), before anything is shown: also an
    // agent that started after the app hears it. An older agent is not
    // told and keeps English.
    if let Some(locale) = app
        .try_state::<crate::commands::locale::AgentLocale>()
        .and_then(|locale| locale.get())
        && let Err(error) = client.set_locale(&locale).await
    {
        eprintln!("the agent was not told the language: {error}");
    }
    if let Response::Status { status } = client.request(Request::GetStatus).await? {
        unlock_if_needed(app, paths, &status).await;
        let _ = app.emit(STATUS_EVENT, running(*status));
    }
    client.subscribe().await?;
    loop {
        match client.next_event().await? {
            AgentEvent::State { status } => {
                unlock_if_needed(app, paths, &status).await;
                let _ = app.emit(STATUS_EVENT, running(*status));
            }
            AgentEvent::ShowWindow => show_window(app),
            AgentEvent::OpenWindow { window } => {
                // Only an identifier arrives; the catalog decides.
                if let Some(state) = app.try_state::<crate::state::AppState>()
                    && let Err(error) =
                        crate::commands::windows::open_by_id(app, &state, &window, None)
                {
                    eprintln!("the agent asked for window `{window}`: {error}");
                }
            }
            AgentEvent::ShuttingDown { reason } => {
                // On macOS the tray's "Quit DeckPilot" ends this app too; a
                // restart or an update does not. Elsewhere the app stays.
                if crate::lifecycle::on_agent_stop(reason, crate::lifecycle::MACOS)
                    == crate::lifecycle::OnAgentStop::Exit
                {
                    app.exit(0);
                }
                return Ok(());
            }
            AgentEvent::ActionCompleted { .. } => {}
            event @ (AgentEvent::ExecutionStarted { .. }
            | AgentEvent::ExecutionProgress { .. }
            | AgentEvent::ExecutionFinished { .. }) => {
                let _ = app.emit(EXECUTION_EVENT, event);
            }
            AgentEvent::Notification { notification } => {
                let _ = app.emit(NOTIFICATION_EVENT, notification);
            }
            event @ (AgentEvent::ProjectChanged { .. }
            | AgentEvent::ProfileChanged { .. }
            | AgentEvent::SharedVariablesChanged { .. }
            | AgentEvent::ResourceBindingsChanged
            | AgentEvent::DeviceContextChanged { .. }
            | AgentEvent::ExecutionContextChanged { .. }) => {
                let _ = app.emit(VARIABLES_EVENT, event);
            }
            AgentEvent::WorkSessionChanged { snapshot } => {
                let _ = app.emit(WORK_SESSION_EVENT, snapshot);
            }
        }
    }
}

/// Keeps a plugin event subscription to the agent and forwards its events
/// to the windows. Agents before protocol 10, and agents whose plugins are
/// not available, are asked again later; nothing is subscribed to then.
/// While the main window renders plugin dialogs
/// ([`crate::commands::plugins::PluginEvents::watch_dialogs`]) and the
/// agent has them, the subscription announces
/// [`deckpilot_ipc::plugins::feature::PLUGIN_DIALOGS`]; when that changes
/// it subscribes again at once.
pub fn spawn_plugin_bridge<R: Runtime>(app: AppHandle<R>, paths: AgentPaths) {
    tauri::async_runtime::spawn(async move {
        loop {
            let delay = match forward_plugin_events(&app, &paths).await {
                Ok(Bridge::Resubscribe) => Duration::ZERO,
                Ok(Bridge::Retry) => PLUGIN_RETRY_DELAY,
                Err(ClientError::Closed | ClientError::NotRunning) => RECONNECT_DELAY,
                Err(error) => {
                    eprintln!("plugin event subscription failed: {error}");
                    PLUGIN_RETRY_DELAY
                }
            };
            tokio::time::sleep(delay).await;
        }
    });
}

/// How a plugin event subscription ended.
enum Bridge {
    /// The agent has no plugins (now): ask again later.
    Retry,
    /// Whether the main window renders plugin dialogs changed.
    Resubscribe,
}

/// Whether a plugin event goes to the main window only: plugin dialogs
/// are shown there and nowhere else.
pub(crate) fn main_window_only(event: &PluginEvent) -> bool {
    matches!(
        event,
        PluginEvent::DialogOpened { .. } | PluginEvent::DialogClosed { .. }
    )
}

/// Returns `Ok(Bridge::Retry)` without subscribing when the agent has no
/// plugins.
async fn forward_plugin_events<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AgentPaths,
) -> Result<Bridge, ClientError> {
    let mut ready = app
        .try_state::<crate::commands::plugins::PluginEvents>()
        .map(|events| events.watch_dialogs());
    let mut client = AgentClient::connect(paths, ClientKind::Desktop).await?;
    if !client.supports(Capability::Plugins) {
        return Ok(Bridge::Retry);
    }
    let dialogs = match client.request(Request::GetPluginRuntime).await? {
        Response::PluginRuntime { status } if status.available => {
            status.has_feature(deckpilot_ipc::plugins::feature::PLUGIN_DIALOGS)
        }
        _ => return Ok(Bridge::Retry),
    };
    let announced = dialogs
        && ready
            .as_mut()
            .is_some_and(|ready| *ready.borrow_and_update());
    let features = if announced {
        vec![deckpilot_ipc::plugins::feature::PLUGIN_DIALOGS.to_owned()]
    } else {
        Vec::new()
    };
    // With module migration events: this app shows the migration notice.
    client
        .subscribe_plugin_events_announcing(true, features)
        .await?;
    loop {
        // Watched only when the agent has plugin dialogs. A change always
        // ends this connection: the read it interrupts is not cancel safe.
        let changed = async {
            match ready.as_mut().filter(|_| dialogs) {
                Some(ready) => ready.changed().await.is_ok(),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            event = client.next_plugin_event() => {
                let event = event?;
                if main_window_only(&event) {
                    if let PluginEvent::DialogOpened { .. } = &event {
                        reveal_main_window(app);
                    }
                    let _ = app.emit_to(crate::windows::MAIN, PLUGIN_EVENT, event);
                } else {
                    let _ = app.emit(PLUGIN_EVENT, event);
                }
            }
            alive = changed => {
                if !alive {
                    // The state is gone (the app is ending).
                    return Ok(Bridge::Retry);
                }
                // A new connection announces it (or not); closing this one
                // tells the agent this app stopped showing them.
                return Ok(Bridge::Resubscribe);
            }
        }
    }
}

/// A plugin dialog arrived: the main window comes forward if it was
/// hidden (closed to the menu bar), since the user started what asks.
fn reveal_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(main) = app.get_webview_window(crate::windows::MAIN)
        && !main.is_visible().unwrap_or(true)
    {
        crate::windows::show_main(app);
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

/// Brings the app to the front (the agent's "Open DeckPilot" while this app
/// runs): the main window when unlocked, otherwise the password prompt. A
/// process started for one window only creates its main window on macOS,
/// so the request is never lost and no second app is started.
pub(crate) fn show_window<R: Runtime>(app: &AppHandle<R>) {
    use crate::lifecycle::{MACOS, ShowTarget, show_target};
    let state = app.try_state::<crate::state::AppState>();
    let unlocked = state.as_ref().is_none_or(|state| state.is_unlocked());
    let main = app.get_webview_window(crate::windows::MAIN);
    match show_target(unlocked, main.is_some(), MACOS) {
        ShowTarget::Prompt => {
            if let Some(state) = state {
                let id = crate::commands::security::prompt_window(&state);
                if let Err(error) = crate::commands::windows::open_by_id(app, &state, id, None) {
                    eprintln!("cannot open the password prompt: {error}");
                }
            }
        }
        ShowTarget::Main => {
            crate::windows::show_main(app);
        }
        // macOS: shown by its page once it has loaded (the configuration
        // is open).
        ShowTarget::CreateMain => {
            if let Err(error) = crate::create_main_window(app) {
                eprintln!("cannot open the main window: {error}");
            }
        }
        // Windows and Linux: unchanged, an app without a main window
        // ignores the request.
        ShowTarget::Nothing => {}
    }
}

#[cfg(test)]
mod launch_tests {
    use super::*;

    fn status(version: &str, channel: Option<&str>) -> AgentStatus {
        let mut json = serde_json::json!({
            "version": version, "protocol": 10, "pid": 1, "startedAt": 0,
            "dataDir": "", "logDir": "", "endpoint": "", "backend": "mock",
            "profiles": [], "activeProfile": null, "activeProject": null,
            "devices": [], "brightness": 50, "lastInputAt": null
        });
        if let Some(channel) = channel {
            let mut record =
                serde_json::to_value(deckpilot_ipc::DistributionRecord::current()).unwrap();
            record["distributionChannel"] = channel.into();
            json["distribution"] = record;
        }
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn an_agent_of_another_channel_is_refused() {
        let own = deckpilot_distribution::current()
            .distribution_channel()
            .name();
        assert_eq!(
            agent_compatibility(&status(APP_VERSION, Some(own))),
            AgentCompatibility::Compatible
        );
        let other = if own == "apt" { "github" } else { "apt" };
        assert_eq!(
            agent_compatibility(&status(APP_VERSION, Some(other))),
            AgentCompatibility::OtherDistribution {
                agent: other.into(),
                app: own.into()
            }
        );
        // An agent without a record (0.6.0-alpha.3 and earlier) was a
        // GitHub build; it is checked by its version as before.
        let without = agent_compatibility(&status("0.5.0", None));
        if own == "github" {
            assert!(
                matches!(without, AgentCompatibility::AgentOutdated { .. }),
                "{without:?}"
            );
        } else {
            assert!(
                matches!(without, AgentCompatibility::OtherDistribution { .. }),
                "{without:?}"
            );
        }
    }

    /// Debug builds (these tests) honour the override; release builds do
    /// not read it at all, which `deckpilot-distribution`'s
    /// `tests/release_build.rs` shows on a release binary.
    #[cfg(debug_assertions)]
    #[test]
    fn debug_builds_honour_the_agent_executable_override() {
        assert_eq!(
            deckpilot_distribution::AGENT_EXECUTABLE_OVERRIDE_VARIABLE,
            "DECKPILOT_AGENT_BIN"
        );
        match std::env::var_os("DECKPILOT_AGENT_BIN") {
            Some(path) if !path.is_empty() => {
                let path = PathBuf::from(path);
                assert_eq!(agent_executable(), path.is_file().then_some(path));
            }
            _ => assert_eq!(deckpilot_distribution::agent_executable_override(), None),
        }
    }

    #[test]
    fn inside_an_appimage_the_agent_is_started_through_the_appimage() {
        let (program, args) = agent_command(
            Some(PathBuf::from("/home/u/Apps/DeckPilot.AppImage")),
            PathBuf::from("/tmp/.mount_X/usr/bin/deckpilot-agent"),
        );
        assert_eq!(program, PathBuf::from("/home/u/Apps/DeckPilot.AppImage"));
        assert_eq!(args, [RUN_AGENT_ARG]);
        // Installed packages (.deb, .rpm: /usr/bin) start the executable.
        let (program, args) = agent_command(None, PathBuf::from("/usr/bin/deckpilot-agent"));
        assert_eq!(program, PathBuf::from("/usr/bin/deckpilot-agent"));
        assert_eq!(args, Vec::<&str>::new());
    }
}
