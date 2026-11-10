//! Application updates from GitHub Releases.
//!
//! The Tauri updater downloads the platform artifact named in the release's
//! `latest.json` and verifies its minisign signature against the public key
//! compiled into this build (`DECKPILOT_UPDATER_PUBKEY`). Builds without a
//! key, such as development builds, report updates as not configured and
//! never download anything.
//!
//! Installing replaces both executables, because the agent ships inside the
//! application bundle. The sequence is:
//!
//! 1. download and verify the update;
//! 2. ask the agent to finish its current request, release the devices and
//!    exit (`prepareForUpdate`);
//! 3. install; on failure, start the previous agent again and report;
//! 4. restart the application, which starts the new agent.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use deckpilot_ipc::{AgentPaths, Request, Response};
use deckpilot_storage::ConfigStore;
use deckpilot_update::policy::{InstallContext, InstallDecision, UpdatePreferences, decide};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Base64 minisign public key of the release signing key, set at build time.
pub const PUBLIC_KEY: Option<&str> = option_env!("DECKPILOT_UPDATER_PUBKEY");
pub const STATE_EVENT: &str = "update-state";
const PREFERENCES_KEY: &str = "update.preferences";
/// How often the background task wakes up to see whether a check is due.
const TICK: Duration = Duration::from_secs(30 * 60);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn configured() -> bool {
    PUBLIC_KEY.is_some_and(|key| !key.trim().is_empty())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Downloading,
    Ready,
    Installing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    notes: Option<String>,
    date: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

#[derive(Default)]
struct Inner {
    phase: Phase,
    available: Option<UpdateInfo>,
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
    progress: Option<Progress>,
    error: Option<String>,
}

/// Shared updater state.
#[derive(Default)]
pub struct UpdateManager {
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for UpdateManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateManager").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    current_version: &'static str,
    configured: bool,
    target: String,
    endpoint: &'static str,
    preferences: UpdatePreferences,
    phase: Phase,
    available: Option<UpdateInfo>,
    progress: Option<Progress>,
    error: Option<String>,
}

impl UpdateManager {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn preferences(state: &AppState) -> UpdatePreferences {
    state
        .store()
        .setting(PREFERENCES_KEY)
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<UpdatePreferences>(&text).ok())
        .unwrap_or_default()
        .sanitized()
}

fn save_preferences(state: &AppState, preferences: &UpdatePreferences) -> CommandResult<()> {
    let text =
        serde_json::to_string(preferences).map_err(|e| CommandError::Invalid(e.to_string()))?;
    state.store().set_setting(PREFERENCES_KEY, &text)?;
    Ok(())
}

fn snapshot<R: Runtime>(app: &AppHandle<R>) -> UpdateState {
    let state = app.state::<AppState>();
    let manager = app.state::<UpdateManager>();
    let inner = manager.lock();
    UpdateState {
        current_version: env!("CARGO_PKG_VERSION"),
        configured: configured(),
        target: deckpilot_update::manifest::current_target(),
        endpoint: deckpilot_update::STABLE_ENDPOINT,
        preferences: preferences(&state),
        phase: inner.phase,
        available: inner.available.clone(),
        progress: inner.progress,
        error: inner.error.clone(),
    }
}

fn publish<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit(STATE_EVENT, snapshot(app));
}

fn set_phase<R: Runtime>(app: &AppHandle<R>, phase: Phase, error: Option<String>) {
    {
        let manager = app.state::<UpdateManager>();
        let mut inner = manager.lock();
        inner.phase = phase;
        inner.error = error;
    }
    publish(app);
}

async fn check<R: Runtime>(app: &AppHandle<R>) -> CommandResult<Option<UpdateInfo>> {
    let key = PUBLIC_KEY
        .filter(|_| configured())
        .ok_or_else(|| CommandError::Invalid("updates are not configured in this build".into()))?;
    set_phase(app, Phase::Checking, None);
    let endpoint = deckpilot_update::STABLE_ENDPOINT
        .parse()
        .map_err(|_| CommandError::Invalid("invalid update endpoint".into()))?;
    let result = async {
        app.updater_builder()
            .pubkey(key)
            .endpoints(vec![endpoint])?
            .build()?
            .check()
            .await
    }
    .await;
    let state = app.state::<AppState>();
    let mut prefs = preferences(&state);
    prefs.last_check = Some(now());
    save_preferences(&state, &prefs)?;
    match result {
        Ok(found) => {
            let info = found.as_ref().map(|update| UpdateInfo {
                version: update.version.clone(),
                notes: update.body.clone(),
                date: update.date.map(|d| d.to_string()),
            });
            {
                let manager = app.state::<UpdateManager>();
                let mut inner = manager.lock();
                inner.available.clone_from(&info);
                inner.update = found;
                inner.bytes = None;
                inner.progress = None;
                inner.phase = Phase::Idle;
                inner.error = None;
            }
            publish(app);
            Ok(info)
        }
        Err(error) => {
            let message = format!("update check failed: {error}");
            set_phase(app, Phase::Idle, Some(message.clone()));
            Err(CommandError::Invalid(message))
        }
    }
}

async fn download<R: Runtime>(app: &AppHandle<R>) -> CommandResult<()> {
    let update = app
        .state::<UpdateManager>()
        .lock()
        .update
        .clone()
        .ok_or_else(|| CommandError::Invalid("no update is available".into()))?;
    set_phase(app, Phase::Downloading, None);
    let progress_app = app.clone();
    let mut downloaded = 0u64;
    let result = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                progress_app.state::<UpdateManager>().lock().progress =
                    Some(Progress { downloaded, total });
                publish(&progress_app);
            },
            || {},
        )
        .await;
    match result {
        // The signature has been verified by the updater at this point.
        Ok(bytes) => {
            app.state::<UpdateManager>().lock().bytes = Some(bytes);
            set_phase(app, Phase::Ready, None);
            Ok(())
        }
        Err(error) => {
            let message = format!("download failed: {error}");
            set_phase(app, Phase::Idle, Some(message.clone()));
            Err(CommandError::Invalid(message))
        }
    }
}

/// Stops the agent, installs, and restarts the application.
async fn install<R: Runtime>(app: &AppHandle<R>) -> CommandResult<()> {
    let (update, bytes) = {
        let manager = app.state::<UpdateManager>();
        let inner = manager.lock();
        match (inner.update.clone(), inner.bytes.clone()) {
            (Some(update), Some(bytes)) => (update, bytes),
            _ => return Err(CommandError::Invalid("download the update first".into())),
        }
    };
    set_phase(app, Phase::Installing, None);
    let paths = app.state::<AppState>().paths.clone();
    let agent_was_running = matches!(crate::agent::connection(&paths).await, Ok(c) if c.running);
    if agent_was_running {
        crate::agent::stop_any_version(&paths, Request::PrepareForUpdate).await?;
        if !crate::agent::wait_until_stopped(&paths, Duration::from_secs(15)).await {
            let message = "the agent did not stop; the update was not installed".to_owned();
            set_phase(app, Phase::Ready, Some(message.clone()));
            return Err(CommandError::Invalid(message));
        }
    }
    if let Err(error) = update.install(bytes) {
        let message = format!("installation failed: {error}");
        if agent_was_running && let Err(start) = crate::agent::start_agent() {
            eprintln!("could not restart the agent after a failed update: {start}");
        }
        set_phase(app, Phase::Ready, Some(message.clone()));
        return Err(CommandError::Invalid(message));
    }
    app.restart();
}

// Tauri passes the app handle and state to commands by value.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn update_state<R: Runtime>(app: AppHandle<R>) -> UpdateState {
    snapshot(&app)
}

#[tauri::command]
pub async fn check_for_updates<R: Runtime>(app: AppHandle<R>) -> CommandResult<Option<UpdateInfo>> {
    check(&app).await
}

#[tauri::command]
pub async fn download_update<R: Runtime>(app: AppHandle<R>) -> CommandResult<()> {
    download(&app).await
}

#[tauri::command]
pub async fn install_update<R: Runtime>(app: AppHandle<R>) -> CommandResult<()> {
    install(&app).await
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn set_update_preferences<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    preferences: UpdatePreferences,
) -> CommandResult<UpdateState> {
    let mut preferences = preferences.sanitized();
    preferences.last_check = self::preferences(&state).last_check;
    save_preferences(&state, &preferences)?;
    Ok(snapshot(&app))
}

/// Seconds since the agent's last device input, if it runs.
async fn agent_last_input(paths: &AgentPaths) -> Option<u64> {
    match crate::agent::request(paths, Request::GetStatus).await {
        Ok(Response::Status { status }) => status.last_input_at,
        _ => None,
    }
}

/// Periodically checks for updates and applies the configured mode.
pub fn spawn_background<R: Runtime>(app: AppHandle<R>) {
    if !configured() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let prefs = preferences(&app.state::<AppState>());
            if prefs.is_check_due(now()) {
                let _ = check(&app).await;
            }
            let (available, downloaded) = {
                let manager = app.state::<UpdateManager>();
                let inner = manager.lock();
                (inner.update.is_some(), inner.bytes.is_some())
            };
            if available {
                let paths = app.state::<AppState>().paths.clone();
                let context = InstallContext {
                    downloaded,
                    user_confirmed: false,
                    agent_busy: false,
                    agent_last_input_at: agent_last_input(&paths).await,
                    now: now(),
                };
                match decide(prefs.mode, &context) {
                    InstallDecision::Download => {
                        let _ = download(&app).await;
                    }
                    InstallDecision::InstallNow => {
                        let _ = install(&app).await;
                    }
                    InstallDecision::Notify
                    | InstallDecision::AskUser
                    | InstallDecision::WaitForIdle => {}
                }
            }
            tokio::time::sleep(TICK).await;
        }
    });
}
