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
const PREFERENCES_KEY: &str = deckpilot_update::policy::PREFERENCES_SETTING;
/// How often the background task wakes up to see whether a check is due.
const TICK: Duration = Duration::from_secs(30 * 60);
const STARTUP_DELAY: Duration = Duration::from_secs(60);

/// Build-time only: a loopback update endpoint for the reproducible update
/// test (`docs/release-validation.md`). Official builds never set it. It
/// cannot come from the runtime environment, and only `http://127.0.0.1:`
/// URLs are accepted, so a build that has it can only update from this
/// computer. Plain HTTP must also be allowed in that build's configuration;
/// signatures are verified exactly as in release builds.
const TEST_ENDPOINT: Option<&str> = option_env!("DECKPILOT_UPDATE_ENDPOINT");

fn test_endpoint() -> Option<&'static str> {
    TEST_ENDPOINT.filter(|endpoint| is_loopback_test_endpoint(endpoint))
}

fn is_loopback_test_endpoint(endpoint: &str) -> bool {
    endpoint
        .strip_prefix("http://127.0.0.1:")
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(['@', '#', '?']))
}

/// The update manifest URL: the stable channel, or the test endpoint.
fn endpoint() -> &'static str {
    test_endpoint().unwrap_or(deckpilot_update::STABLE_ENDPOINT)
}

/// Background timing; the update test build checks quickly.
fn timing() -> (Duration, Duration) {
    if test_endpoint().is_some() {
        (Duration::from_secs(10), Duration::from_secs(15))
    } else {
        (STARTUP_DELAY, TICK)
    }
}

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

/// Stored preferences; the defaults while the configuration is locked.
fn preferences(state: &AppState) -> UpdatePreferences {
    state
        .store()
        .ok()
        .and_then(|store| store.setting(PREFERENCES_KEY).ok().flatten())
        .and_then(|text| serde_json::from_str::<UpdatePreferences>(&text).ok())
        .unwrap_or_default()
        .sanitized()
}

fn save_preferences(state: &AppState, preferences: &UpdatePreferences) -> CommandResult<()> {
    let text =
        serde_json::to_string(preferences).map_err(|e| CommandError::Invalid(e.to_string()))?;
    state.store()?.set_setting(PREFERENCES_KEY, &text)?;
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
        endpoint: endpoint(),
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

/// Whether an operation may start while the updater is in `current`.
/// Only one check, download or installation runs at a time.
fn may_start(current: Phase) -> bool {
    !matches!(
        current,
        Phase::Checking | Phase::Downloading | Phase::Installing
    )
}

/// Moves to `phase` unless another update operation is running; the check
/// and the change happen under one lock, so two attempts cannot both start.
fn begin<R: Runtime>(app: &AppHandle<R>, phase: Phase) -> CommandResult<()> {
    {
        let manager = app.state::<UpdateManager>();
        let mut inner = manager.lock();
        if !may_start(inner.phase) {
            return Err(CommandError::Invalid(
                "another update operation is in progress".into(),
            ));
        }
        inner.phase = phase;
        inner.error = None;
    }
    publish(app);
    Ok(())
}

async fn check<R: Runtime>(app: &AppHandle<R>) -> CommandResult<Option<UpdateInfo>> {
    let key = PUBLIC_KEY
        .filter(|_| configured())
        .ok_or_else(|| CommandError::Invalid("updates are not configured in this build".into()))?;
    begin(app, Phase::Checking)?;
    let endpoint = endpoint()
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
    // While locked, checks still work; the time is not stored.
    if state.is_unlocked() {
        save_preferences(&state, &prefs)?;
    }
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
    begin(app, Phase::Downloading)?;
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
    let paths = app.state::<AppState>().paths.clone();
    // An update never ends running workflows: it waits until they finished
    // (automatic installation tries again later) or the user cancels them.
    let current = crate::agent::connection(&paths).await;
    if let Ok(connection) = &current
        && let Some(running) = connection.running_executions().filter(|n| *n > 0)
    {
        let message = crate::agent::running_workflows_message(running);
        set_phase(app, Phase::Ready, Some(message.clone()));
        return Err(CommandError::Invalid(message));
    }
    begin(app, Phase::Installing)?;
    let agent_was_running = matches!(current, Ok(c) if c.running);
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

/// The agent's last device input and whether workflows are running in it.
async fn agent_activity(paths: &AgentPaths) -> (Option<u64>, bool) {
    match crate::agent::request(paths, Request::GetStatus).await {
        Ok(Response::Status { status }) => (
            status.last_input_at,
            status.running_executions.is_some_and(|n| n > 0),
        ),
        _ => (None, false),
    }
}

/// Periodically checks for updates and applies the configured mode.
pub fn spawn_background<R: Runtime>(app: AppHandle<R>) {
    if !configured() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let (startup, tick) = timing();
        tokio::time::sleep(startup).await;
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
                let (agent_last_input_at, workflows_running) = agent_activity(&paths).await;
                let context = InstallContext {
                    downloaded,
                    user_confirmed: false,
                    // Running workflows are never ended by an update.
                    agent_busy: workflows_running,
                    agent_last_input_at,
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
            tokio::time::sleep(tick).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_update_operation_runs_at_a_time() {
        assert!(may_start(Phase::Idle));
        assert!(may_start(Phase::Ready));
        for busy in [Phase::Checking, Phase::Downloading, Phase::Installing] {
            assert!(!may_start(busy), "{busy:?}");
        }
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;

    #[test]
    fn only_loopback_test_endpoints_are_accepted() {
        assert!(is_loopback_test_endpoint(
            "http://127.0.0.1:8765/latest.json"
        ));
        for rejected in [
            "https://example.com/latest.json",
            "http://127.0.0.1.example.com/latest.json",
            "http://127.0.0.1:8765@evil.example/latest.json",
            "http://localhost:8765/latest.json",
            "http://127.0.0.1:",
        ] {
            assert!(!is_loopback_test_endpoint(rejected), "{rejected}");
        }
        // Normal builds use the stable channel.
        if TEST_ENDPOINT.is_none() {
            assert_eq!(endpoint(), deckpilot_update::STABLE_ENDPOINT);
            assert_eq!(timing(), (STARTUP_DELAY, TICK));
        }
    }
}
