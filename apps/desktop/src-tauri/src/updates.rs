//! Application updates from GitHub Releases.
//!
//! The Tauri updater downloads the platform artifact named in the release's
//! manifest and verifies its minisign signature against the public key
//! compiled into this build (`DECKPILOT_UPDATER_PUBKEY`). Builds without a
//! key, such as development builds, report updates as not configured and
//! never download anything.
//!
//! Channels (`deckpilot_update::channel`): the stable channel (default)
//! reads `latest.json` of GitHub's latest release and never installs a
//! pre-release. The prerelease channel picks the newest published release
//! of either kind from GitHub's release list and reads that release's
//! manifest, so it moves on to a stable release once that is newer.
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
use deckpilot_update::channel::{Channel, ListedRelease, newest_release};
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

/// Where updates of `channel` are looked up: the stable manifest, the
/// release list (prerelease channel), or the test endpoint.
fn endpoint(channel: Channel) -> &'static str {
    test_endpoint().unwrap_or(match channel {
        Channel::Stable => deckpilot_update::STABLE_ENDPOINT,
        Channel::Prerelease => deckpilot_update::RELEASES_API,
    })
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
    let preferences = preferences(&state);
    UpdateState {
        current_version: env!("CARGO_PKG_VERSION"),
        configured: configured(),
        target: deckpilot_update::manifest::current_target(),
        endpoint: endpoint(preferences.channel),
        preferences,
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

/// Where the manifest of an update comes from.
#[derive(Debug, Clone)]
pub(crate) enum Endpoint {
    /// A fixed manifest URL (the stable channel, the test endpoint).
    Manifest(String),
    /// The newest release in GitHub's release list that the channel
    /// accepts; its manifest is under `downloads` (the release download
    /// prefix, a loopback URL in tests).
    Releases { list: String, downloads: String },
}

/// Where updates come from and how long the network may stall. Release
/// builds use [`Source::release`]; tests use a loopback server and a
/// throwaway key.
#[derive(Debug, Clone)]
pub(crate) struct Source {
    pub key: String,
    pub endpoint: Endpoint,
    /// Which versions may be installed; checked again on the version the
    /// manifest announces, whatever endpoint it came from.
    pub channel: Channel,
    /// Establishing a connection may take at most this long.
    pub connect_timeout: Duration,
    /// A response that sends no data for this long fails, so a dropped or
    /// stalled connection ends the check or download instead of keeping
    /// the updater busy (and every later attempt refused) forever.
    pub read_timeout: Duration,
    /// The executable whose installation is replaced; the running one by
    /// default.
    pub executable: Option<std::path::PathBuf>,
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

impl Source {
    fn release(channel: Channel) -> CommandResult<Self> {
        let key = PUBLIC_KEY.filter(|_| configured()).ok_or_else(|| {
            CommandError::Invalid("updates are not configured in this build".into())
        })?;
        let endpoint = match (test_endpoint(), channel) {
            (None, Channel::Prerelease) => Endpoint::Releases {
                list: deckpilot_update::RELEASES_API.to_owned(),
                downloads: deckpilot_update::DOWNLOAD_PREFIX.to_owned(),
            },
            _ => Endpoint::Manifest(endpoint(channel).to_owned()),
        };
        Ok(Self {
            key: key.to_owned(),
            endpoint,
            channel,
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: READ_TIMEOUT,
            executable: None,
        })
    }

    /// The manifest to check: fixed, or chosen from the release list.
    /// `None` when the list has nothing newer for the channel.
    async fn manifest(&self) -> Result<Option<String>, CheckError> {
        let (list, downloads) = match &self.endpoint {
            Endpoint::Manifest(url) => return Ok(Some(url.clone())),
            Endpoint::Releases { list, downloads } => (list, downloads),
        };
        let releases = fetch_releases(list, self.connect_timeout, self.read_timeout)
            .await
            .map_err(CheckError::Releases)?;
        let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .map_err(|e| CheckError::Releases(e.to_string()))?;
        Ok(newest_release(&releases, self.channel, &current)
            .map(|tag| tag.manifest_url_at(downloads)))
    }

    fn updater<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        manifest: &str,
    ) -> Result<tauri_plugin_updater::Updater, tauri_plugin_updater::Error> {
        let endpoint = manifest.parse()?;
        let (connect, read) = (self.connect_timeout, self.read_timeout);
        let channel = self.channel;
        let mut builder = app
            .updater_builder()
            .pubkey(self.key.clone())
            .endpoints(vec![endpoint])?
            // Never a pre-release on the stable channel, never an older
            // version: whatever a manifest announces.
            .version_comparator(move |current, remote| channel.accepts(&current, &remote.version))
            .configure_client(move |client| client.connect_timeout(connect).read_timeout(read));
        if let Some(executable) = &self.executable {
            builder = builder.executable_path(executable);
        }
        builder.build()
    }
}

/// Why a check failed: the updater, or reading the release list.
#[derive(Debug)]
pub(crate) enum CheckError {
    Updater(tauri_plugin_updater::Error),
    Releases(String),
}

/// GitHub's release list (prerelease channel). Read like the updater reads
/// a manifest: HTTPS with the same timeouts; the answer only chooses which
/// release's manifest is read, see [`newest_release`].
async fn fetch_releases(
    url: &str,
    connect: Duration,
    read: Duration,
) -> Result<Vec<ListedRelease>, String> {
    // The updater's TLS stack (rustls) needs a crypto provider, as in the
    // updater plugin.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("DeckPilot/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(connect)
        .read_timeout(read)
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| format!("not a release list: {e}"))
}

/// The step of an update that failed, for messages and the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Check,
    Download,
    Install,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Download => "download",
            Self::Install => "installation",
        }
    }
}

fn is_signature_error(error: &tauri_plugin_updater::Error) -> bool {
    use tauri_plugin_updater::Error as E;
    matches!(
        error,
        E::Minisign(_)
            | E::Base64(_)
            | E::SignatureUtf8(_)
            | E::SignedVersionMismatch { .. }
            | E::MissingSignedVersion
    )
}

/// A message for an updater error that says what happened, what state the
/// installation is in and what to do. The updater's own text follows as
/// the detail; it holds URLs, versions and paths, never key material.
pub(crate) fn describe(stage: Stage, error: &tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error as E;
    let summary = match error {
        error if is_signature_error(error) => {
            "the update failed signature verification and was discarded; nothing was \
             installed. Do not install DeckPilot from another source; try again later and \
             report the problem if it persists"
        }
        E::Reqwest(error) if error.is_timeout() => {
            "the connection to GitHub stopped responding; nothing was installed. Check the \
             internet connection and try again"
        }
        E::Reqwest(_) | E::Network(_) => {
            "GitHub Releases could not be reached or refused the request; nothing was \
             installed. Check the internet connection, proxy or firewall and try again"
        }
        E::ReleaseNotFound | E::Serialization(_) => {
            "the update server did not return a valid update description; nothing was \
             installed. Try again later"
        }
        E::TargetNotFound(_) | E::TargetsNotFound(_) => {
            "the latest release has no update for this platform; install it from the \
             releases page instead"
        }
        E::PreviousAppNotRestored(_) => {
            "installing failed and the previous version could not be put back automatically; \
             move the copy named at the end of this message back to its original location, or reinstall DeckPilot \
             from the releases page"
        }
        E::AuthenticationFailed => {
            "administrator authorization was cancelled or failed; the installed version was \
             not changed. Install again and confirm the prompt"
        }
        E::DebInstallFailed | E::PackageInstallFailed => {
            "the system package manager could not install the update; the installed version \
             was not changed. Install the package from the releases page to see its error"
        }
        E::InvalidUpdaterFormat | E::BinaryNotFoundInArchive => {
            "the downloaded package does not fit this installation; the installed version was \
             not changed. Install the update from the releases page"
        }
        E::TempDirNotOnSameMountPoint | E::TempDirNotFound | E::FailedToDetermineExtractPath => {
            "the update cannot be installed where DeckPilot is installed; the installed version \
             was not changed. Move DeckPilot to the Applications folder (macOS) or install the \
             update from the releases page"
        }
        E::Io(_) if stage == Stage::Install => {
            "installing failed and the previous version was kept. Check that DeckPilot's \
             installation folder is writable and the disk has free space, then try again"
        }
        _ => match stage {
            Stage::Check => "nothing was downloaded. Try again later",
            Stage::Download => "nothing was installed. Try again",
            Stage::Install => "the previous version was kept. Try again",
        },
    };
    format!("update {} failed: {summary} ({error})", stage.name())
}

/// Update events are appended to this file in the log directory, so the
/// cause of a failed update survives the restart and can be attached to a
/// problem report.
pub(crate) const UPDATE_LOG: &str = "updates.log";
const MAX_UPDATE_LOG: u64 = 256 * 1024;

/// Appends `message` to the update log in `dir`. Best effort; the log is
/// kept below about 256 KiB by moving a full one to `updates.log.1`.
pub(crate) fn log_event(dir: &std::path::Path, message: &str) {
    use std::io::Write as _;
    eprintln!("update: {message}");
    let path = dir.join(UPDATE_LOG);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_UPDATE_LOG) {
        let _ = std::fs::rename(&path, dir.join(format!("{UPDATE_LOG}.1")));
    }
    let _ = std::fs::create_dir_all(dir);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(
            file,
            "{} version {}: {message}",
            now(),
            env!("CARGO_PKG_VERSION")
        );
    }
}

fn log<R: Runtime>(app: &AppHandle<R>, message: &str) {
    log_event(&app.state::<AppState>().paths.log_dir, message);
}

/// Ends an operation with an error: back to `phase`, the message shown and
/// logged.
fn fail<R: Runtime>(app: &AppHandle<R>, phase: Phase, message: String) -> CommandError {
    log(app, &message);
    set_phase(app, phase, Some(message.clone()));
    CommandError::Invalid(message)
}

async fn check<R: Runtime>(app: &AppHandle<R>) -> CommandResult<Option<UpdateInfo>> {
    let channel = preferences(&app.state::<AppState>()).channel;
    let source = Source::release(channel)?;
    check_from(app, &source).await
}

pub(crate) async fn check_from<R: Runtime>(
    app: &AppHandle<R>,
    source: &Source,
) -> CommandResult<Option<UpdateInfo>> {
    begin(app, Phase::Checking)?;
    let result = async {
        let Some(manifest) = source.manifest().await? else {
            return Ok(None);
        };
        let updater = source
            .updater(app, &manifest)
            .map_err(CheckError::Updater)?;
        updater.check().await.map_err(CheckError::Updater)
    }
    .await;
    let state = app.state::<AppState>();
    let mut prefs = preferences(&state);
    prefs.last_check = Some(now());
    // While locked, checks still work; the time is not stored. Failing to
    // store it must not leave the updater in the checking phase.
    if state.is_unlocked()
        && let Err(error) = save_preferences(&state, &prefs)
    {
        log(
            app,
            &format!("the time of the update check was not stored: {error}"),
        );
    }
    match result {
        Ok(found) => {
            let info = found.as_ref().map(|update| UpdateInfo {
                version: update.version.clone(),
                notes: update.body.clone(),
                date: update.date.map(|d| d.to_string()),
            });
            if let Some(info) = &info {
                log(app, &format!("update {} is available", info.version));
            }
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
        Err(CheckError::Updater(error)) => {
            Err(fail(app, Phase::Idle, describe(Stage::Check, &error)))
        }
        Err(CheckError::Releases(detail)) => Err(fail(
            app,
            Phase::Idle,
            format!(
                "update check failed: the list of DeckPilot releases on GitHub could not be \
                 read; nothing was downloaded. Check the internet connection and try again \
                 later ({detail})"
            ),
        )),
    }
}

pub(crate) async fn download<R: Runtime>(app: &AppHandle<R>) -> CommandResult<()> {
    let update = app
        .state::<UpdateManager>()
        .lock()
        .update
        .clone()
        .ok_or_else(|| CommandError::Invalid("no update is available".into()))?;
    begin(app, Phase::Downloading)?;
    {
        // A download starts from the beginning and replaces an earlier one.
        let manager = app.state::<UpdateManager>();
        let mut inner = manager.lock();
        inner.bytes = None;
        inner.progress = None;
    }
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
            log(
                app,
                &format!(
                    "update {} downloaded ({} bytes), signature verified",
                    update.version,
                    bytes.len()
                ),
            );
            app.state::<UpdateManager>().lock().bytes = Some(bytes);
            set_phase(app, Phase::Ready, None);
            Ok(())
        }
        Err(error) => {
            // Nothing partial is kept; the update stays offered and the
            // next attempt downloads it again from the start.
            app.state::<UpdateManager>().lock().progress = None;
            Err(fail(app, Phase::Idle, describe(Stage::Download, &error)))
        }
    }
}

/// Stops the agent, installs, and restarts the application.
async fn install<R: Runtime>(app: &AppHandle<R>) -> CommandResult<()> {
    replace(app).await?;
    log(app, "update installed; restarting the application");
    app.restart();
}

/// An installation up to the restart: stops the agent and replaces the
/// installed application, which contains the agent. On failure the previous
/// version stays installed, an agent that was running is started again, and
/// the downloaded update stays ready to retry.
///
/// On Windows the installer takes over, and this process exits instead of
/// returning.
pub(crate) async fn replace<R: Runtime>(app: &AppHandle<R>) -> CommandResult<()> {
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
    log(app, &format!("installing update {}", update.version));
    let agent_was_running = matches!(current, Ok(c) if c.running);
    if agent_was_running {
        if let Err(error) = crate::agent::stop_any_version(&paths, Request::PrepareForUpdate).await
        {
            return Err(fail(
                app,
                Phase::Ready,
                format!(
                    "update installation failed: the agent could not be asked to stop \
                     ({error}); nothing was installed and the agent keeps running. Try again"
                ),
            ));
        }
        if !crate::agent::wait_until_stopped(&paths, Duration::from_secs(15)).await {
            return Err(fail(
                app,
                Phase::Ready,
                "update installation failed: the agent did not stop within 15 seconds; \
                 nothing was installed. Try again, or quit the agent from its menu first"
                    .to_owned(),
            ));
        }
    }
    if let Err(error) = update.install(bytes) {
        let mut message = describe(Stage::Install, &error);
        // A package that no longer verifies is of no use: download it again
        // instead of retrying it.
        let phase = if is_signature_error(&error) {
            app.state::<UpdateManager>().lock().bytes = None;
            Phase::Idle
        } else {
            Phase::Ready
        };
        if agent_was_running {
            match crate::agent::start_agent() {
                Ok(()) => message.push_str("; the agent was started again"),
                Err(start) => {
                    use std::fmt::Write as _;
                    let _ = write!(
                        message,
                        "; the agent could not be started again ({start}), restart DeckPilot"
                    );
                }
            }
        }
        return Err(fail(app, phase, message));
    }
    Ok(())
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
    let previous = self::preferences(&state);
    preferences.last_check = previous.last_check;
    if preferences.channel != previous.channel {
        // An update found on the other channel is forgotten; the next
        // check looks on the new one.
        let manager = app.state::<UpdateManager>();
        let mut inner = manager.lock();
        if !may_start(inner.phase) {
            return Err(CommandError::Invalid(
                "the update channel cannot change while an update is being checked, \
                 downloaded or installed"
                    .into(),
            ));
        }
        inner.available = None;
        inner.update = None;
        inner.bytes = None;
        inner.progress = None;
        inner.phase = Phase::Idle;
        inner.error = None;
    }
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
        // Normal builds use the channel's endpoint.
        if TEST_ENDPOINT.is_none() {
            assert_eq!(endpoint(Channel::Stable), deckpilot_update::STABLE_ENDPOINT);
            assert_eq!(
                endpoint(Channel::Prerelease),
                deckpilot_update::RELEASES_API
            );
            assert_eq!(timing(), (STARTUP_DELAY, TICK));
        }
    }
}

#[cfg(test)]
#[path = "updates_tests.rs"]
mod flow_tests;
