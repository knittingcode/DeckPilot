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
//! 2. refuse while workflows run in the agent;
//! 3. record the intent (`update-intent.json` in the data directory,
//!    `deckpilot_update::intent`);
//! 4. ask the agent to finish its current request, release the devices and
//!    exit (`prepareForUpdate`), and wait until its endpoint is closed
//!    **and** its instance lock is free, so the old and the new agent never
//!    run at the same time;
//! 5. install; on failure, start the previous agent again, remove the
//!    intent and report;
//! 6. restart the application, which starts the new agent;
//! 7. at the next start, [`recover`] reads the intent: the new version runs
//!    with its agent (the intent is removed), the previous version is still
//!    installed (its agent is started again), or the new agent does not run
//!    (reported, with what is safe about going back).
//!
//! docs/distribution/README.md, section 7, has the full lifecycle and
//! docs/updates.md when rolling back is safe.
//!
//! All of this is the `GithubRelease` provider (docs/distribution/README.md,
//! section 4.1), which only a GitHub build of a known package format with a
//! compiled key gets ([`crate::update_provider`]). Package-manager and store
//! builds never reach the Tauri updater (R1): the commands answer through
//! their provider, which reports who updates the installation and refuses
//! to download or install (R2). Builds without the `self-update` feature
//! do not contain the updater at all.
//!
//! Before anything is downloaded, a check also refuses an update whose
//! manifest names any artifact outside this repository's GitHub release
//! downloads (`Manifest::validate_artifacts`), including the URL the updater
//! actually requests as it parsed it, and an update that is not the exact
//! `{os}-{arch}-{format}` entry of the installed format (R3, R4): a `.deb`
//! never updates to an AppImage, an MSI never to NSIS. The minisign
//! verification of the package is unchanged.

// The Tauri updater parts are left out without `self-update`; what they
// share with the rest then has no user.
#![cfg_attr(not(feature = "self-update"), allow(dead_code, unused_imports))]

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
#[cfg(feature = "self-update")]
use tauri_plugin_updater::{Update, UpdaterExt};

use deckpilot_ipc::{AgentPaths, Request, Response};
use deckpilot_storage::ConfigStore;
use deckpilot_update::channel::{Channel, ListedRelease, newest_release};
use deckpilot_update::intent::{self, Observation, Recovery, UpdateIntent};
use deckpilot_update::manifest::{Manifest, ManifestError, PlatformArtifact};
use deckpilot_update::policy::{InstallContext, InstallDecision, UpdatePreferences, decide};
use deckpilot_update::provider::{AvailableVersion, ProviderStatus};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;
use crate::update_provider::{self, ActiveProvider, ProviderInfo};

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

/// Whether this build can verify and install updates itself: the updater
/// is compiled in and so is its key. An input of the provider selection.
pub(crate) fn self_update_compiled() -> bool {
    cfg!(feature = "self-update") && configured()
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

impl UpdateInfo {
    pub(crate) fn into_available(self) -> AvailableVersion {
        AvailableVersion {
            version: self.version,
            notes: self.notes,
            date: self.date,
        }
    }

    fn from_available(available: AvailableVersion) -> Self {
        Self {
            version: available.version,
            notes: available.notes,
            date: available.date,
        }
    }
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
    #[cfg(feature = "self-update")]
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
    progress: Option<Progress>,
    error: Option<String>,
}

/// Shared updater state, with the provider chosen when it is created.
pub struct UpdateManager {
    inner: Mutex<Inner>,
    provider: ActiveProvider,
}

impl Default for UpdateManager {
    /// The provider of the running build (`ActiveProvider::current`).
    fn default() -> Self {
        Self::with_provider(ActiveProvider::current())
    }
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
    /// Who updates this installation and what the app may do about it
    /// (`distributionChannel`, `updateProvider`, `releaseChannel`,
    /// `supportsAutomaticUpdates`, …). Additive: the fields above keep
    /// their meaning.
    #[serde(flatten)]
    distribution: ProviderInfo,
}

impl UpdateManager {
    /// Tests choose the distribution they need.
    pub(crate) fn with_provider(provider: ActiveProvider) -> Self {
        Self {
            inner: Mutex::default(),
            provider,
        }
    }

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

/// The provider chosen for `app`'s build.
pub(crate) fn active_provider<R: Runtime>(app: &AppHandle<R>) -> ActiveProvider {
    app.state::<UpdateManager>().provider
}

/// The update found by the last check.
pub(crate) fn available<R: Runtime>(app: &AppHandle<R>) -> Option<UpdateInfo> {
    app.state::<UpdateManager>().lock().available.clone()
}

/// The GitHub provider's status, from the phase.
pub(crate) fn provider_status<R: Runtime>(app: &AppHandle<R>) -> ProviderStatus {
    let manager = app.state::<UpdateManager>();
    let inner = manager.lock();
    let available = || inner.available.clone().map(UpdateInfo::into_available);
    match inner.phase {
        Phase::Idle => available().map_or(ProviderStatus::Idle, ProviderStatus::Available),
        Phase::Checking => ProviderStatus::Checking,
        Phase::Downloading => ProviderStatus::Downloading {
            downloaded: inner.progress.map_or(0, |p| p.downloaded),
            total: inner.progress.and_then(|p| p.total),
        },
        Phase::Ready => available().map_or(ProviderStatus::Idle, ProviderStatus::Ready),
        Phase::Installing => ProviderStatus::Installing,
    }
}

pub(crate) fn snapshot<R: Runtime>(app: &AppHandle<R>) -> UpdateState {
    let state = app.state::<AppState>();
    let manager = app.state::<UpdateManager>();
    let inner = manager.lock();
    let preferences = preferences(&state);
    UpdateState {
        current_version: env!("CARGO_PKG_VERSION"),
        // As before for GitHub builds; false for every build that does not
        // update itself.
        configured: manager.provider.may_self_update(),
        target: deckpilot_update::manifest::current_target(),
        endpoint: endpoint(preferences.channel),
        preferences,
        phase: inner.phase,
        available: inner.available.clone(),
        progress: inner.progress,
        error: inner.error.clone(),
        distribution: manager.provider.info(),
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
    /// The exact manifest key of the installed format
    /// (`{os}-{arch}-{format}`): the only entry the updater may use (R3,
    /// R4), never the plain `{os}-{arch}` fallback.
    pub target: String,
    /// Which artifact URLs a manifest may name.
    pub artifacts: ArtifactPolicy,
}

/// Which artifact URLs an update manifest may name (audit G10). Checked on
/// every artifact of a manifest before anything is downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArtifactPolicy {
    /// Release builds: release downloads of this repository only,
    /// [`Manifest::validate_artifacts`] (pinned to
    /// [`deckpilot_update::DOWNLOAD_PREFIX`], owner and repository in any
    /// case).
    GithubRelease,
    /// The loopback update test build and the tests: URLs under this
    /// `http://127.0.0.1:<port>/` prefix stand for release downloads, and
    /// the same rule checks the rest of the URL and the signature.
    Loopback(String),
}

impl ArtifactPolicy {
    /// The policy of a build: loopback only with the compiled loopback
    /// test endpoint.
    fn of_build() -> Self {
        test_endpoint().map_or(Self::GithubRelease, |endpoint| {
            Self::Loopback(loopback_origin(endpoint))
        })
    }

    /// Every artifact of `manifest`.
    pub(crate) fn validate(&self, manifest: &Manifest) -> Result<(), ManifestError> {
        manifest
            .platforms
            .values()
            .try_for_each(|artifact| self.validate_artifact(artifact))
    }

    pub(crate) fn validate_artifact(
        &self,
        artifact: &PlatformArtifact,
    ) -> Result<(), ManifestError> {
        let url = match self {
            Self::GithubRelease => artifact.url.clone(),
            Self::Loopback(prefix) => artifact.url.strip_prefix(prefix.as_str()).map_or_else(
                || artifact.url.clone(),
                |rest| format!("{}{rest}", deckpilot_update::DOWNLOAD_PREFIX),
            ),
        };
        let single = Manifest {
            version: String::new(),
            notes: None,
            pub_date: None,
            platforms: BTreeMap::from([(
                String::new(),
                PlatformArtifact {
                    signature: artifact.signature.clone(),
                    url,
                },
            )]),
        };
        single.validate_artifacts().map_err(|error| match error {
            ManifestError::UntrustedUrl(_) => ManifestError::UntrustedUrl(artifact.url.clone()),
            other => other,
        })
    }
}

/// `http://127.0.0.1:<port>/` of a loopback test endpoint.
fn loopback_origin(endpoint: &str) -> String {
    let rest = endpoint
        .strip_prefix("http://127.0.0.1:")
        .unwrap_or_default();
    let port = rest.split('/').next().unwrap_or_default();
    format!("http://127.0.0.1:{port}/")
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

impl Source {
    /// The release source of a build whose provider is `active`.
    fn release(channel: Channel, active: &ActiveProvider) -> CommandResult<Self> {
        let not_configured =
            || CommandError::Invalid("updates are not configured in this build".into());
        let key = PUBLIC_KEY
            .filter(|_| configured())
            .ok_or_else(not_configured)?;
        // R1 and R3: only a GitHub build of a known format has a key to
        // update from.
        let target = active.manifest_key().ok_or_else(not_configured)?;
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
            target,
            artifacts: ArtifactPolicy::of_build(),
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

    #[cfg(feature = "self-update")]
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
            // Exactly the installed format's entry (R3, R4). The release
            // manifests always have it; the endpoints have no `{{target}}`.
            .target(self.target.clone())
            // Never a pre-release on the stable channel, never an older
            // version: whatever a manifest announces.
            .version_comparator(move |current, remote| channel.accepts(&current, &remote.version))
            .configure_client(move |client| client.connect_timeout(connect).read_timeout(read));
        if let Some(executable) = &self.executable {
            builder = builder.executable_path(executable);
        }
        builder.build()
    }

    /// Refuses an update before anything is downloaded unless it is the
    /// installed format's entry (R4) and every artifact URL of its manifest
    /// passes the policy (G10). The URL the updater will request is checked
    /// as well, as it parsed it: parsing resolves `%2e%2e` segments, which
    /// could otherwise lead out of the release downloads.
    #[cfg(feature = "self-update")]
    fn verify(&self, update: &Update) -> Result<(), CheckError> {
        if update.target != self.target {
            return Err(CheckError::OtherFormat {
                expected: self.target.clone(),
                offered: update.target.clone(),
            });
        }
        let manifest: Manifest = serde_json::from_value(update.raw_json.clone())
            .map_err(|e| CheckError::Manifest(ManifestError::Json(e.to_string())))?;
        self.artifacts
            .validate(&manifest)
            .and_then(|()| {
                self.artifacts.validate_artifact(&PlatformArtifact {
                    signature: update.signature.clone(),
                    url: update.download_url.to_string(),
                })
            })
            .map_err(CheckError::Manifest)
    }
}

/// Why a check failed: the updater, reading the release list, or a manifest
/// this build refuses.
#[derive(Debug)]
pub(crate) enum CheckError {
    #[cfg(feature = "self-update")]
    Updater(tauri_plugin_updater::Error),
    Releases(String),
    Manifest(ManifestError),
    OtherFormat {
        expected: String,
        offered: String,
    },
}

impl CheckError {
    /// The message for a refused manifest; nothing was downloaded.
    fn refused(&self) -> Option<String> {
        let (summary, detail) = match self {
            Self::Manifest(error @ ManifestError::UntrustedUrl(_)) => (
                "the update description names a download outside DeckPilot's releases on \
                 GitHub, so it was refused; nothing was downloaded. Do not install DeckPilot \
                 from another source; report the problem if it persists",
                error.to_string(),
            ),
            Self::Manifest(error @ ManifestError::InvalidSignature) => (
                "the update description has a missing or malformed signature, so it was \
                 refused; nothing was downloaded. Try again later and report the problem if it \
                 persists",
                error.to_string(),
            ),
            Self::Manifest(error) => (
                "the update server did not return a valid update description; nothing was \
                 downloaded. Try again later",
                error.to_string(),
            ),
            Self::OtherFormat { expected, offered } => (
                "the update is not a package of the format DeckPilot was installed as; nothing \
                 was downloaded. Install the update from the releases page",
                format!("the update is `{offered}`, this installation `{expected}`"),
            ),
            #[cfg(feature = "self-update")]
            Self::Updater(_) => return None,
            Self::Releases(_) => return None,
        };
        Some(format!("update check failed: {summary} ({detail})"))
    }
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

#[cfg(feature = "self-update")]
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
#[cfg(feature = "self-update")]
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

/// A check of the GitHub provider, from the build's release source.
#[cfg(feature = "self-update")]
pub(crate) async fn check<R: Runtime>(app: &AppHandle<R>) -> CommandResult<Option<UpdateInfo>> {
    let channel = preferences(&app.state::<AppState>()).channel;
    let source = Source::release(channel, &active_provider(app))?;
    check_from(app, &source).await
}

#[cfg(feature = "self-update")]
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
        let found = updater.check().await.map_err(CheckError::Updater)?;
        if let Some(update) = &found {
            source.verify(update)?;
        }
        Ok(found)
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
        Err(error @ (CheckError::Manifest(_) | CheckError::OtherFormat { .. })) => {
            // Nothing from this manifest is kept.
            {
                let manager = app.state::<UpdateManager>();
                let mut inner = manager.lock();
                inner.available = None;
                inner.update = None;
                inner.bytes = None;
                inner.progress = None;
            }
            Err(fail(app, Phase::Idle, error.refused().unwrap_or_default()))
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

#[cfg(feature = "self-update")]
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
#[cfg(feature = "self-update")]
pub(crate) async fn install<R: Runtime>(app: &AppHandle<R>) -> CommandResult<()> {
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
#[cfg(feature = "self-update")]
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
    // Recorded before anything is stopped, so that the next start can tell
    // what happened even if this process does not survive.
    let intent = UpdateIntent {
        format: intent::FORMAT,
        from_version: env!("CARGO_PKG_VERSION").to_owned(),
        to_version: update.version.clone(),
        distribution_channel: deckpilot_distribution::current()
            .distribution_channel()
            .name()
            .to_owned(),
        schema_version: schema_version(app),
        created_at: now(),
        agent_was_running,
    };
    if let Err(error) = intent.write(&paths.data_dir) {
        return Err(fail(
            app,
            Phase::Ready,
            format!(
                "update installation failed: the update could not be recorded in the data \
                 directory ({error}); nothing was installed and the agent keeps running. Check \
                 that the disk has free space and try again"
            ),
        ));
    }
    if agent_was_running
        && let Err(error) = crate::agent::stop_any_version(&paths, Request::PrepareForUpdate).await
    {
        let _ = UpdateIntent::remove(&paths.data_dir);
        return Err(fail(
            app,
            Phase::Ready,
            format!(
                "update installation failed: the agent could not be asked to stop \
                 ({error}); nothing was installed and the agent keeps running. Try again"
            ),
        ));
    }
    // Also without a running agent: one that is starting holds the lock
    // before it accepts connections, and would run the old executable.
    if let Err(reason) = crate::agent::wait_until_released(&paths, STOP_TIMEOUT).await {
        let _ = UpdateIntent::remove(&paths.data_dir);
        return Err(fail(
            app,
            Phase::Ready,
            format!(
                "update installation failed: {reason}; nothing was installed. Try again, or \
                 quit the agent from its menu first"
            ),
        ));
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
        // Handled here: nothing is left for the next start to recover.
        let _ = UpdateIntent::remove(&paths.data_dir);
        return Err(fail(app, phase, message));
    }
    Ok(())
}

/// How long the agent has to stop (endpoint closed, lock released) before
/// an installation is abandoned.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long recovery waits for the agent at start-up.
const RECOVERY_AGENT_WAIT: Duration = Duration::from_secs(15);
/// How long recovery gives an agent the app has just started before it
/// starts one itself.
const AGENT_STARTING: Duration = Duration::from_secs(5);

/// The configuration database's schema version, while it is open here.
fn schema_version<R: Runtime>(app: &AppHandle<R>) -> Option<u32> {
    let state = app.state::<AppState>();
    let store = state.store().ok()?;
    store.schema_version().ok()
}

/// At start-up: finishes or reports an update that a previous run of the
/// application recorded (`update-intent.json`) and did not complete
/// (docs/distribution/README.md, section 7, step 11). Without an intent it
/// does nothing.
pub async fn recover<R: Runtime>(app: &AppHandle<R>, start_agent: bool) {
    let paths = app.state::<AppState>().paths.clone();
    let start = || {
        if start_agent {
            crate::agent::start_agent()
        } else {
            Err("the agent is managed by a development session".to_owned())
        }
    };
    recover_with(app, &paths, start, RECOVERY_AGENT_WAIT).await;
}

/// [`recover`] with the agent start and the time to wait for the agent
/// given, for tests.
pub(crate) async fn recover_with<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AgentPaths,
    start_agent: impl FnOnce() -> Result<(), String>,
    wait: Duration,
) -> Option<Recovery> {
    let intent = match UpdateIntent::read(&paths.data_dir) {
        Ok(Some(intent)) => intent,
        Ok(None) => return None,
        Err(error) => {
            log(app, &format!("{error}; it is removed"));
            let _ = UpdateIntent::remove(&paths.data_dir);
            return None;
        }
    };
    // The previous or the new agent must run. The app has just started it
    // if none ran: give it a moment, then start it here.
    let mut connection = crate::agent::wait_until_running(paths, wait.min(AGENT_STARTING)).await;
    if !connection.running {
        if let Err(error) = start_agent() {
            log(
                app,
                &format!("update recovery: cannot start the agent: {error}"),
            );
        }
        connection = crate::agent::wait_until_running(paths, wait).await;
    }
    let agent_version = Some(&connection)
        .filter(|c| c.running)
        .and_then(|c| c.status.as_ref())
        .map(|status| status.version.clone());
    let observation = Observation {
        app_version: env!("CARGO_PKG_VERSION"),
        agent_version: agent_version.as_deref(),
        supported_schema: deckpilot_storage::SqliteStore::supported_schema_version(),
        current_schema: schema_version(app),
    };
    let recovery = intent::assess(&intent, &observation);
    let message = intent::describe(&intent, &recovery);
    log(app, &message);
    if !recovery.keeps_intent()
        && let Err(error) = UpdateIntent::remove(&paths.data_dir)
    {
        log(
            app,
            &format!("the update intent could not be removed: {error}"),
        );
    }
    match recovery {
        Recovery::Completed | Recovery::Superseded => {}
        Recovery::NotInstalled { .. } | Recovery::AgentNotRunning { .. } => {
            set_phase(app, Phase::Idle, Some(message));
        }
    }
    Some(recovery)
}

// Tauri passes the app handle and state to commands by value.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn update_state<R: Runtime>(app: AppHandle<R>) -> UpdateState {
    snapshot(&app)
}

/// Checks through the build's provider: the Tauri updater for GitHub
/// builds, nothing at all for package-manager and store builds.
#[tauri::command]
pub async fn check_for_updates<R: Runtime>(app: AppHandle<R>) -> CommandResult<Option<UpdateInfo>> {
    update_provider::for_app(&app)
        .check()
        .await
        .map(|found| found.map(UpdateInfo::from_available))
        .map_err(update_provider::command_error)
}

/// Downloads through the build's provider; only GitHub builds can (R2).
#[tauri::command]
pub async fn download_update<R: Runtime>(app: AppHandle<R>) -> CommandResult<()> {
    update_provider::for_app(&app)
        .prepare(&|_, _| {})
        .await
        .map_err(update_provider::command_error)
}

/// Installs through the build's provider; only GitHub builds can (R2).
#[tauri::command]
pub async fn install_update<R: Runtime>(app: AppHandle<R>) -> CommandResult<()> {
    update_provider::for_app(&app)
        .install()
        .await
        .map_err(update_provider::command_error)
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
        #[cfg(feature = "self-update")]
        {
            inner.update = None;
        }
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

/// Without the updater no build updates itself, so there is no background
/// task.
#[cfg(not(feature = "self-update"))]
#[allow(clippy::needless_pass_by_value)]
pub fn spawn_background<R: Runtime>(_app: AppHandle<R>) {}

/// Periodically checks for updates and applies the configured mode. Only a
/// build that updates itself has this task; package-manager and store
/// builds never check in the background.
#[cfg(feature = "self-update")]
pub fn spawn_background<R: Runtime>(app: AppHandle<R>) {
    if !update_provider::for_app(&app).supports_automatic_updates() {
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
            assert_eq!(ArtifactPolicy::of_build(), ArtifactPolicy::GithubRelease);
        }
        assert_eq!(
            loopback_origin("http://127.0.0.1:8765/latest.json"),
            "http://127.0.0.1:8765/"
        );
        assert_eq!(
            loopback_origin("http://127.0.0.1:8765"),
            "http://127.0.0.1:8765/"
        );
    }

    /// The published `v0.6.0-alpha.3` manifest (`prerelease.json`, audit
    /// section 5): every key and asset name as released. The new client
    /// check accepts it, and every installed format finds its exact entry.
    #[test]
    fn the_published_manifest_shape_passes_the_client_check() {
        use deckpilot_update::provider::{
            Architecture, Distribution, DistributionChannel, PackageFormat as F, Platform,
        };
        const SIGNATURE: &str =
            include_str!("../../../../crates/deckpilot-update/tests/fixtures/artifact.bin.sig");
        let tag = "v0.6.0-alpha.3";
        let entries = [
            ("darwin-aarch64", "DeckPilot_aarch64.app.tar.gz"),
            ("darwin-aarch64-app", "DeckPilot_aarch64.app.tar.gz"),
            ("darwin-x86_64", "DeckPilot_x64.app.tar.gz"),
            ("darwin-x86_64-app", "DeckPilot_x64.app.tar.gz"),
            ("windows-x86_64", "DeckPilot_0.6.0-alpha.3_x64_en-US.msi"),
            (
                "windows-x86_64-msi",
                "DeckPilot_0.6.0-alpha.3_x64_en-US.msi",
            ),
            (
                "windows-x86_64-nsis",
                "DeckPilot_0.6.0-alpha.3_x64-setup.exe",
            ),
            ("linux-x86_64", "DeckPilot_0.6.0-alpha.3_amd64.AppImage"),
            (
                "linux-x86_64-appimage",
                "DeckPilot_0.6.0-alpha.3_amd64.AppImage",
            ),
            ("linux-x86_64-deb", "DeckPilot_0.6.0-alpha.3_amd64.deb"),
        ];
        let manifest = Manifest {
            version: "0.6.0-alpha.3".into(),
            notes: None,
            pub_date: None,
            platforms: entries
                .iter()
                .map(|(key, asset)| {
                    (
                        (*key).to_owned(),
                        PlatformArtifact {
                            signature: SIGNATURE.trim().to_owned(),
                            url: format!("{}{tag}/{asset}", deckpilot_update::DOWNLOAD_PREFIX),
                        },
                    )
                })
                .collect(),
        };
        let policy = ArtifactPolicy::GithubRelease;
        assert_eq!(policy.validate(&manifest), Ok(()));
        // As the updater parses each URL before downloading it.
        for artifact in manifest.platforms.values() {
            let parsed = artifact.url.parse::<reqwest::Url>().unwrap().to_string();
            assert_eq!(parsed, artifact.url);
            assert_eq!(
                policy.validate_artifact(&PlatformArtifact {
                    signature: artifact.signature.clone(),
                    url: parsed,
                }),
                Ok(())
            );
        }
        for (platform, architecture, format) in [
            (Platform::Macos, Architecture::Aarch64, F::App),
            (Platform::Macos, Architecture::X86_64, F::App),
            (Platform::Windows, Architecture::X86_64, F::Msi),
            (Platform::Windows, Architecture::X86_64, F::Nsis),
            (Platform::Linux, Architecture::X86_64, F::AppImage),
            (Platform::Linux, Architecture::X86_64, F::Deb),
        ] {
            let distribution = Distribution::new(
                DistributionChannel::Github,
                None,
                "0.6.0-alpha.3",
                platform,
                architecture,
            )
            .unwrap();
            let key = ActiveProvider::select(distribution, Some(format), true)
                .manifest_key()
                .unwrap();
            assert!(manifest.platforms.contains_key(&key), "{key}");
        }
    }

    #[test]
    fn loopback_urls_stand_for_release_downloads_only_under_their_prefix() {
        const SIGNATURE: &str =
            include_str!("../../../../crates/deckpilot-update/tests/fixtures/artifact.bin.sig");
        let policy = ArtifactPolicy::Loopback("http://127.0.0.1:8765/".into());
        let artifact = |url: &str| PlatformArtifact {
            signature: SIGNATURE.trim().to_owned(),
            url: url.to_owned(),
        };
        assert_eq!(
            policy.validate_artifact(&artifact("http://127.0.0.1:8765/DeckPilot.app.tar.gz")),
            Ok(())
        );
        for bad in [
            "http://127.0.0.1:87650/DeckPilot.app.tar.gz",
            "http://127.0.0.1:8766/DeckPilot.app.tar.gz",
            "http://localhost:8765/DeckPilot.app.tar.gz",
            "http://127.0.0.1:8765/../x",
            "http://127.0.0.1:8765/a?b",
            "http://127.0.0.1:8765/",
            "https://evil.example/DeckPilot.app.tar.gz",
        ] {
            assert_eq!(
                policy.validate_artifact(&artifact(bad)),
                Err(ManifestError::UntrustedUrl(bad.to_owned())),
                "{bad}"
            );
        }
    }
}

#[cfg(all(test, feature = "self-update"))]
#[path = "updates_tests.rs"]
mod flow_tests;
