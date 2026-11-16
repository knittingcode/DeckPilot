//! Plugin marketplace commands (experimental; IPC protocol 10, capability
//! `plugins`, `docs/marketplace/agent-integration.md`).
//!
//! Every command goes to the agent, which owns the plugin store, the
//! registry and the sandboxed hosts; nothing about plugins is decided or
//! stored here. Each command opens one connection and, unless it only asks
//! for the runtime status, first checks with `getPluginRuntime` that the
//! agent answers plugin requests at all: an agent without the `plugins`
//! feature, without `--enable-plugins`, or older than protocol 10 is
//! refused here with a typed error and never receives the request.
//!
//! Errors reach the frontend as `{ reason, message }` ([`PluginError`]):
//! the agent's `IpcError.reason` (`deckpilot_ipc::plugins::reason`) as it
//! is, or one of this app's own [`reason`]s.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use deckpilot_ipc::plugins::{
    self, AvailablePlugin, KnownPlugin, ModuleMigrationInfo, PluginActionOutcome,
    PluginCredentialSlot, PluginDetails, PluginInstallSource, PluginRetention, PluginRuntimeStatus,
    PluginSettings, PluginSummary, PluginUpdateOutcome, RegistryStatus,
};
use deckpilot_ipc::{
    AgentClient, Capability, ClientError, ClientKind, ErrorCode, Request, Response, plugin_answers,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State, WebviewWindow};

use crate::state::AppState;

/// Reasons this app adds to the agent's (`deckpilot_ipc::plugins::reason`).
pub mod reason {
    pub use deckpilot_ipc::plugins::reason::{LOCKED, PLUGINS_UNAVAILABLE};
    /// No agent is running.
    pub const AGENT_NOT_RUNNING: &str = "agentNotRunning";
    /// The agent speaks a protocol before 10: it has no plugin requests.
    pub const AGENT_TOO_OLD: &str = "agentTooOld";
    /// The connection to the agent failed or the agent answered nonsense.
    pub const COMMUNICATION: &str = "communication";
    /// The agent refused the request without a reason (older agent or
    /// an error outside the plugin backend).
    pub const FAILED: &str = "failed";
    /// No file dialog is available in this window.
    pub const NO_DIALOG: &str = "noDialog";
}

/// A refused plugin command, as the frontend receives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct PluginError {
    /// Stable and machine-readable (see [`reason`]).
    pub reason: String,
    pub message: String,
}

impl PluginError {
    fn new(reason: &str, message: impl Into<String>) -> Self {
        Self {
            reason: reason.to_owned(),
            message: message.into(),
        }
    }

    fn too_old(protocol: u32) -> Self {
        Self::new(
            reason::AGENT_TOO_OLD,
            format!(
                "the running agent speaks protocol {protocol} and has no plugin support \
                 (it needs protocol 10); restart the agent with the one of this release"
            ),
        )
    }

    fn unexpected(response: &Response) -> Self {
        Self::new(
            reason::COMMUNICATION,
            format!("unexpected agent response: {response:?}"),
        )
    }

    /// Plugins are off in the agent: not built, not enabled, or locked.
    fn unavailable(status: &PluginRuntimeStatus) -> Self {
        Self::new(
            reason::PLUGINS_UNAVAILABLE,
            status
                .reason
                .clone()
                .unwrap_or_else(|| "plugins are not available in the running agent".into()),
        )
    }
}

impl From<ClientError> for PluginError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::NotRunning | ClientError::Closed => Self::new(
                reason::AGENT_NOT_RUNNING,
                "the DeckPilot agent is not running; start it in Settings › Background Agent",
            ),
            ClientError::Unsupported { protocol, .. } => Self::too_old(protocol),
            ClientError::Agent(error) => {
                let reason = error.reason.unwrap_or_else(|| {
                    match error.code {
                        ErrorCode::UnsupportedProtocol => reason::AGENT_TOO_OLD,
                        ErrorCode::Locked => reason::LOCKED,
                        _ => reason::FAILED,
                    }
                    .to_owned()
                });
                Self {
                    reason,
                    message: error.message,
                }
            }
            error @ (ClientError::Io(_) | ClientError::Protocol(_)) => {
                Self::new(reason::COMMUNICATION, error.to_string())
            }
        }
    }
}

pub type PluginResult<T> = Result<T, PluginError>;

/// A connection to an agent that answers plugin requests.
struct Connection {
    client: AgentClient,
    /// The agent pages plugin answers ([`plugins::feature::PAGED_ANSWERS`]):
    /// a plugin's actions and icon are read with requests of their own, so
    /// any number of them fits ([`deckpilot_ipc::plugin_answers`]). An
    /// agent without it answers in one message, which it shortens or
    /// refuses when the answer does not fit.
    paged: bool,
}

/// Opens a connection for plugin requests. Unless `gated` is false (only
/// `getPluginRuntime` itself), it first asks `getPluginRuntime` and refuses
/// unless the agent reports plugins as available.
async fn connect(state: &AppState, gated: bool) -> PluginResult<Connection> {
    if gated && !state.is_unlocked() {
        return Err(PluginError::new(
            reason::LOCKED,
            "the configuration is locked",
        ));
    }
    let mut client = AgentClient::connect(&state.paths, ClientKind::Desktop).await?;
    if !client.supports(Capability::Plugins) {
        return Err(PluginError::too_old(client.protocol()));
    }
    let mut paged = false;
    if gated {
        match client.request(Request::GetPluginRuntime).await? {
            Response::PluginRuntime { status } if status.available => {
                paged = status.has_feature(plugins::feature::PAGED_ANSWERS);
            }
            Response::PluginRuntime { status } => return Err(PluginError::unavailable(&status)),
            other => return Err(PluginError::unexpected(&other)),
        }
    }
    Ok(Connection { client, paged })
}

/// Sends one plugin request over a connection of its own. Every request
/// but `getPluginRuntime` is preceded by `getPluginRuntime` on the same
/// connection and refused unless the agent reports plugins as available.
async fn plugin_request(state: &AppState, request: Request) -> PluginResult<Response> {
    let gated = !matches!(request, Request::GetPluginRuntime);
    let mut connection = connect(state, gated).await?;
    Ok(connection.client.request(request).await?)
}

/// Every answer with one plugin's details also updates the action types
/// the editors validate against ([`crate::plugin_actions`]). With an agent
/// that pages, the details are read in full after the request (a change
/// answers with them too, possibly shortened to fit).
async fn plugin_details(state: &AppState, request: Request) -> PluginResult<PluginDetails> {
    let mut connection = connect(state, true).await?;
    let details = if connection.paged {
        let id = match request {
            Request::GetPlugin { plugin } => plugin,
            request => match connection.client.request(request).await? {
                Response::Plugin { plugin } => plugin.summary.id,
                other => return Err(PluginError::unexpected(&other)),
            },
        };
        plugin_answers::plugin_details(&mut connection.client, &id).await?
    } else {
        match connection.client.request(request).await? {
            Response::Plugin { plugin } => *plugin,
            other => return Err(PluginError::unexpected(&other)),
        }
    };
    state.plugin_actions().upsert(&details.summary);
    Ok(details)
}

/// The installed plugins, every action with its definition (paged when
/// the agent pages).
async fn installed_plugins(state: &AppState) -> PluginResult<Vec<PluginSummary>> {
    let mut connection = connect(state, true).await?;
    if connection.paged {
        return Ok(plugin_answers::list_plugins(&mut connection.client).await?);
    }
    match connection.client.request(Request::ListPlugins).await? {
        Response::Plugins { plugins } => Ok(plugins),
        other => Err(PluginError::unexpected(&other)),
    }
}

/// `listAvailablePlugins`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailablePlugins {
    pub registry: RegistryStatus,
    pub plugins: Vec<AvailablePlugin>,
}

/// `updatePlugin` and `approvePluginUpdate`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginUpdate {
    pub plugin: String,
    pub outcome: PluginUpdateOutcome,
}

async fn plugin_update(state: &AppState, request: Request) -> PluginResult<PluginUpdate> {
    match plugin_request(state, request).await? {
        Response::PluginUpdate { plugin, outcome } => Ok(PluginUpdate { plugin, outcome }),
        other => Err(PluginError::unexpected(&other)),
    }
}

/// Whether plugins can be used, and why not. Answered by every agent of
/// protocol 10, whatever its plugin state.
#[tauri::command]
pub async fn get_plugin_runtime(state: State<'_, AppState>) -> PluginResult<PluginRuntimeStatus> {
    match plugin_request(&state, Request::GetPluginRuntime).await? {
        Response::PluginRuntime { status } => Ok(*status),
        other => Err(PluginError::unexpected(&other)),
    }
}

/// The installed plugins. The answer is also what the editors validate
/// plugin actions against ([`crate::plugin_actions`]); when the agent
/// cannot answer, no plugin action type is known until it can.
#[tauri::command]
pub async fn list_plugins(state: State<'_, AppState>) -> PluginResult<Vec<PluginSummary>> {
    let answer = installed_plugins(&state).await;
    match &answer {
        Ok(plugins) => state.plugin_actions().replace(plugins),
        Err(_) => state.plugin_actions().clear(),
    }
    answer
}

#[tauri::command]
pub async fn get_plugin(state: State<'_, AppState>, plugin: String) -> PluginResult<PluginDetails> {
    plugin_details(&state, Request::GetPlugin { plugin }).await
}

#[tauri::command]
pub async fn list_available_plugins(
    state: State<'_, AppState>,
    refresh: Option<bool>,
) -> PluginResult<AvailablePlugins> {
    match plugin_request(
        &state,
        Request::ListAvailablePlugins {
            refresh: refresh.unwrap_or(false),
        },
    )
    .await?
    {
        Response::AvailablePlugins { registry, plugins } => {
            Ok(AvailablePlugins { registry, plugins })
        }
        other => Err(PluginError::unexpected(&other)),
    }
}

/// Installs from the registry or from a package file; `consent` lists the
/// grant keys the user checked.
#[tauri::command]
pub async fn install_plugin(
    state: State<'_, AppState>,
    source: PluginInstallSource,
    consent: Vec<String>,
) -> PluginResult<PluginDetails> {
    plugin_details(&state, Request::InstallPlugin { source, consent }).await
}

#[tauri::command]
pub async fn enable_plugin(
    state: State<'_, AppState>,
    plugin: String,
    consent: Vec<String>,
) -> PluginResult<PluginDetails> {
    plugin_details(&state, Request::EnablePlugin { plugin, consent }).await
}

#[tauri::command]
pub async fn disable_plugin(
    state: State<'_, AppState>,
    plugin: String,
) -> PluginResult<PluginDetails> {
    plugin_details(&state, Request::DisablePlugin { plugin }).await
}

#[tauri::command]
pub async fn grant_plugin_permissions(
    state: State<'_, AppState>,
    plugin: String,
    permissions: Vec<String>,
) -> PluginResult<PluginDetails> {
    plugin_details(
        &state,
        Request::GrantPluginPermissions {
            plugin,
            permissions,
        },
    )
    .await
}

#[tauri::command]
pub async fn revoke_plugin_permission(
    state: State<'_, AppState>,
    plugin: String,
    permission: String,
) -> PluginResult<PluginDetails> {
    plugin_details(
        &state,
        Request::RevokePluginPermission { plugin, permission },
    )
    .await
}

#[tauri::command]
pub async fn update_plugin(
    state: State<'_, AppState>,
    plugin: String,
    version: Option<String>,
    consent: Vec<String>,
) -> PluginResult<PluginUpdate> {
    plugin_update(
        &state,
        Request::UpdatePlugin {
            plugin,
            version,
            consent,
        },
    )
    .await
}

#[tauri::command]
pub async fn approve_plugin_update(
    state: State<'_, AppState>,
    plugin: String,
    consent: Vec<String>,
) -> PluginResult<PluginUpdate> {
    plugin_update(&state, Request::ApprovePluginUpdate { plugin, consent }).await
}

#[tauri::command]
pub async fn decline_plugin_update(
    state: State<'_, AppState>,
    plugin: String,
) -> PluginResult<PluginDetails> {
    plugin_details(&state, Request::DeclinePluginUpdate { plugin }).await
}

#[tauri::command]
pub async fn rollback_plugin(
    state: State<'_, AppState>,
    plugin: String,
) -> PluginResult<PluginDetails> {
    plugin_details(&state, Request::RollbackPlugin { plugin }).await
}

/// Uninstalls; by default its storage, settings and credentials go too,
/// `keep` keeps settings and/or credentials for a later install.
#[tauri::command]
pub async fn uninstall_plugin(
    state: State<'_, AppState>,
    plugin: String,
    keep: Option<PluginRetention>,
) -> PluginResult<()> {
    match plugin_request(
        &state,
        Request::UninstallPlugin {
            plugin: plugin.clone(),
            keep: keep.unwrap_or_default(),
        },
    )
    .await?
    {
        Response::Ok => {
            state.plugin_actions().remove(&plugin);
            Ok(())
        }
        other => Err(PluginError::unexpected(&other)),
    }
}

#[tauri::command]
pub async fn invoke_plugin_action(
    state: State<'_, AppState>,
    plugin: String,
    action: String,
    params: Option<BTreeMap<String, serde_json::Value>>,
) -> PluginResult<PluginActionOutcome> {
    match plugin_request(
        &state,
        Request::InvokePluginAction {
            plugin,
            action,
            params: params.unwrap_or_default(),
        },
    )
    .await?
    {
        Response::PluginActionResult { outcome } => Ok(outcome),
        other => Err(PluginError::unexpected(&other)),
    }
}

/// Every plugin installed here at some point (`plugin_known`).
#[tauri::command]
pub async fn list_known_plugins(state: State<'_, AppState>) -> PluginResult<Vec<KnownPlugin>> {
    match plugin_request(&state, Request::ListKnownPlugins).await? {
        Response::KnownPlugins { plugins } => Ok(plugins),
        other => Err(PluginError::unexpected(&other)),
    }
}

async fn plugin_settings(state: &AppState, request: Request) -> PluginResult<PluginSettings> {
    match plugin_request(state, request).await? {
        Response::PluginSettings { settings } => Ok(*settings),
        other => Err(PluginError::unexpected(&other)),
    }
}

#[tauri::command]
pub async fn get_plugin_settings(
    state: State<'_, AppState>,
    plugin: String,
) -> PluginResult<PluginSettings> {
    plugin_settings(&state, Request::GetPluginSettings { plugin }).await
}

/// Merges `values` into the saved settings (`null` resets one); the agent
/// validates them against the manifest.
#[tauri::command]
pub async fn set_plugin_settings(
    state: State<'_, AppState>,
    plugin: String,
    values: BTreeMap<String, serde_json::Value>,
) -> PluginResult<PluginSettings> {
    plugin_settings(&state, Request::SetPluginSettings { plugin, values }).await
}

async fn plugin_credentials(
    state: &AppState,
    request: Request,
) -> PluginResult<Vec<PluginCredentialSlot>> {
    match plugin_request(state, request).await? {
        Response::PluginCredentials { credentials, .. } => Ok(credentials),
        other => Err(PluginError::unexpected(&other)),
    }
}

/// The plugin's credential slots and whether each has a secret.
#[tauri::command]
pub async fn list_plugin_credentials(
    state: State<'_, AppState>,
    plugin: String,
) -> PluginResult<Vec<PluginCredentialSlot>> {
    plugin_credentials(&state, Request::ListPluginCredentials { plugin }).await
}

/// Stores a secret the user typed. It goes to the agent only; nothing
/// here keeps, logs or returns it.
#[tauri::command]
pub async fn set_plugin_credential(
    state: State<'_, AppState>,
    plugin: String,
    key: String,
    secret: String,
) -> PluginResult<Vec<PluginCredentialSlot>> {
    plugin_credentials(
        &state,
        Request::SetPluginCredential {
            plugin,
            key,
            secret: plugins::PluginSecret::new(secret),
        },
    )
    .await
}

#[tauri::command]
pub async fn delete_plugin_credential(
    state: State<'_, AppState>,
    plugin: String,
    key: String,
) -> PluginResult<Vec<PluginCredentialSlot>> {
    plugin_credentials(&state, Request::DeletePluginCredential { plugin, key }).await
}

/// The built-in modules moving to their first-party plugins
/// (`listModuleMigrations`): phase, reason, attempts, route, aliases and
/// what waits for consent. Names only; no answer carries a secret.
#[tauri::command]
pub async fn list_module_migrations(
    state: State<'_, AppState>,
) -> PluginResult<Vec<ModuleMigrationInfo>> {
    match plugin_request(&state, Request::ListModuleMigrations).await? {
        Response::ModuleMigrations { migrations } => Ok(migrations),
        other => Err(PluginError::unexpected(&other)),
    }
}

async fn module_migration(state: &AppState, request: Request) -> PluginResult<ModuleMigrationInfo> {
    match plugin_request(state, request).await? {
        Response::ModuleMigration { migration } => Ok(*migration),
        other => Err(PluginError::unexpected(&other)),
    }
}

/// The consent dialog of a module migration: the grant keys the user
/// approved and declined, item by item (`decideModuleMigration`).
#[tauri::command]
pub async fn decide_module_migration(
    state: State<'_, AppState>,
    module: String,
    approve: Vec<String>,
    decline: Vec<String>,
) -> PluginResult<ModuleMigrationInfo> {
    module_migration(
        &state,
        Request::DecideModuleMigration {
            module,
            approve,
            decline,
        },
    )
    .await
}

/// Retry in the migration notice (`retryModuleMigration`).
#[tauri::command]
pub async fn retry_module_migration(
    state: State<'_, AppState>,
    module: String,
) -> PluginResult<ModuleMigrationInfo> {
    module_migration(&state, Request::RetryModuleMigration { module }).await
}

/// Asks for a `.dpplugin` file in a native open dialog. `None` when the
/// user cancelled. The path is only passed to `installPlugin`, which
/// accepts the file only if its bytes are a version the registry pins.
#[tauri::command]
pub async fn pick_plugin_package<R: Runtime>(
    window: WebviewWindow<R>,
) -> PluginResult<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    if window
        .try_state::<tauri_plugin_dialog::Dialog<R>>()
        .is_none()
    {
        return Err(PluginError::new(
            reason::NO_DIALOG,
            "no file dialog is available",
        ));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Install a plugin package")
        .add_filter("DeckPilot plugin package", &["dpplugin"])
        .pick_file(move |file| {
            let _ = tx.send(file);
        });
    let file = rx.await.ok().flatten();
    Ok(file
        .and_then(|file| file.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .filter(|path| !path.is_empty() && path.len() <= plugins::MAX_PACKAGE_PATH))
}

/// Whether the plugin event bridge runs.
#[derive(Debug, Default)]
pub struct PluginEvents {
    started: AtomicBool,
}

/// Starts forwarding the agent's `pluginEvent` messages to the windows as
/// [`crate::agent::PLUGIN_EVENT`]. Only builds with plugins ask for it
/// (the plugin screen, and the editors to follow the installed plugins),
/// so other builds never subscribe. Idempotent.
#[tauri::command]
pub fn watch_plugin_events<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) {
    let Some(events) = app.try_state::<PluginEvents>() else {
        return;
    };
    if !events.started.swap(true, Ordering::SeqCst) {
        crate::agent::spawn_plugin_bridge(app.clone(), state.paths.clone());
    }
}

#[cfg(test)]
mod tests {
    use deckpilot_ipc::IpcError;

    use super::*;

    #[test]
    fn agent_errors_keep_their_reason() {
        let error = PluginError::from(ClientError::Agent(
            IpcError::new(ErrorCode::Failed, "consent is missing")
                .with_reason(plugins::reason::CONSENT_REQUIRED),
        ));
        assert_eq!(error.reason, "consentRequired");
        assert_eq!(error.message, "consent is missing");
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({ "reason": "consentRequired", "message": "consent is missing" })
        );
    }

    #[test]
    fn errors_without_a_reason_get_one() {
        let reason = |error| PluginError::from(error).reason;
        assert_eq!(reason(ClientError::NotRunning), "agentNotRunning");
        assert_eq!(reason(ClientError::Closed), "agentNotRunning");
        assert_eq!(
            reason(ClientError::Unsupported {
                request: "listPlugins".into(),
                needs: 10,
                protocol: 9,
                agent: "0.6.0".into(),
            }),
            "agentTooOld"
        );
        assert_eq!(
            reason(ClientError::Agent(IpcError::new(
                ErrorCode::UnsupportedProtocol,
                "old"
            ))),
            "agentTooOld"
        );
        assert_eq!(
            reason(ClientError::Agent(IpcError::new(
                ErrorCode::Locked,
                "locked"
            ))),
            "locked"
        );
        assert_eq!(
            reason(ClientError::Agent(IpcError::new(ErrorCode::Failed, "no"))),
            "failed"
        );
        assert_eq!(
            reason(ClientError::Protocol("garbage".into())),
            "communication"
        );
    }

    #[test]
    fn an_agent_without_plugins_says_why() {
        let status = PluginRuntimeStatus {
            available: false,
            built: false,
            enabled: false,
            execution_supported: true,
            platform: "macos".into(),
            reason: Some("built without plugin support".into()),
            registry: None,
            features: vec![],
        };
        let error = PluginError::unavailable(&status);
        assert_eq!(error.reason, "pluginsUnavailable");
        assert_eq!(error.message, "built without plugin support");
        assert_eq!(
            PluginError::too_old(9).reason,
            "agentTooOld",
            "protocol 9 agents have no plugin requests"
        );
    }
}
