//! Update providers: who updates an installation, what the app may do
//! about it, and the cross-channel rules (docs/distribution/README.md,
//! sections 3 to 5).
//!
//! Pure logic only. The input is a build's compiled
//! [`Distribution`] record, never anything read at run time;
//! [`select_provider`] maps it onto a [`ProviderKind`]. The
//! [`UpdateProvider`] trait is what the desktop app implements per kind.
//! Nothing in DeckPilot uses this module yet.
//!
//! | `distribution_channel` | [`ProviderKind`] |
//! | --- | --- |
//! | `github` | `GithubRelease` (or `None` without an updater key or a known format) |
//! | `homebrew`, `winget`, `apt` | `ExternalPackageManager` |
//! | `flathub`, `mac_app_store`, `microsoft_store` | `StoreManaged` |
//!
//! The rules R1 to R9 of README section 5 appear here as functions that
//! the providers call; the ones that are enforced elsewhere (the build, the
//! release pipeline, documentation) are noted on the function.

use std::future::Future;
use std::pin::Pin;

use semver::Version;
use thiserror::Error;

use crate::channel::{Channel, ReleaseKind};

pub use deckpilot_distribution::{
    Architecture, Distribution, DistributionChannel, PackageFormat, Platform, ReleaseChannel,
    UpdateProviderKind as ProviderKind,
};

/// The [`ReleaseKind`] of a build's [`ReleaseChannel`]: alpha, beta and rc
/// are all pre-releases. This keeps `channel.rs` the only version rule.
pub const fn release_kind(release_channel: ReleaseChannel) -> ReleaseKind {
    match release_channel {
        ReleaseChannel::Stable => ReleaseKind::Stable,
        ReleaseChannel::Beta | ReleaseChannel::Alpha => ReleaseKind::Prerelease,
    }
}

/// The update [`Channel`] a fresh installation of this build subscribes to.
/// The user's choice (`policy.rs`) still decides afterwards.
pub const fn default_update_channel(release_channel: ReleaseChannel) -> Channel {
    match release_kind(release_channel) {
        ReleaseKind::Stable => Channel::Stable,
        ReleaseKind::Prerelease => Channel::Prerelease,
    }
}

/// What the UI shows and which operations exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::struct_excessive_bools)]
pub struct ProviderCapabilities {
    /// The app can download and install updates itself.
    pub self_update: bool,
    /// `UpdateMode::DownloadAndAsk` and `InstallWhenIdle` are offered.
    pub automatic_updates: bool,
    /// The user may choose Stable or Prerelease in the app.
    pub channel_choice: bool,
    /// The app may look up a newer version (for a notice only, without
    /// `self_update`).
    pub version_check: bool,
}

impl ProviderCapabilities {
    /// The capabilities of `kind`. `version_notice` is the user's setting
    /// for external package managers (off by default); other kinds ignore
    /// it.
    pub const fn of(kind: ProviderKind, version_notice: bool) -> Self {
        match kind {
            ProviderKind::GithubRelease => Self {
                self_update: true,
                automatic_updates: true,
                channel_choice: true,
                version_check: true,
            },
            ProviderKind::ExternalPackageManager => Self {
                self_update: false,
                automatic_updates: false,
                channel_choice: false,
                version_check: version_notice,
            },
            ProviderKind::StoreManaged | ProviderKind::None => Self {
                self_update: false,
                automatic_updates: false,
                channel_choice: false,
                version_check: false,
            },
        }
    }
}

/// An update a provider found: today's `UpdateInfo` of the desktop app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableVersion {
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

/// The step of an update that failed: today's `Stage` of the desktop app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    Check,
    Download,
    Install,
}

impl Stage {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Download => "download",
            Self::Install => "installation",
        }
    }
}

/// What a provider is doing; the phases map onto today's `Phase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    Idle,
    Checking,
    Available(AvailableVersion),
    Downloading {
        downloaded: u64,
        total: Option<u64>,
    },
    Ready(AvailableVersion),
    Installing,
    /// Updates come from elsewhere; `instruction` tells the user how.
    Managed {
        by: DistributionChannel,
        instruction: Option<String>,
    },
    NotConfigured {
        reason: String,
    },
    Failed {
        stage: Stage,
        message: String,
    },
}

/// Why a provider operation did not run.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UpdateError {
    /// The provider has no such operation (R2): only `GithubRelease`
    /// downloads or installs.
    #[error("{operation} is not available: updates of this installation are {}", managed_by(*.kind))]
    Unsupported {
        kind: ProviderKind,
        operation: &'static str,
    },
    #[error("updates are not available in this build: {0}")]
    NotConfigured(NotConfigured),
    #[error("the update {} failed: {message}", .stage.name())]
    Failed { stage: Stage, message: String },
}

fn managed_by(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::GithubRelease => "installed by the app",
        ProviderKind::ExternalPackageManager => "installed by the package manager",
        ProviderKind::StoreManaged => "installed by the store",
        ProviderKind::None => "not available in this build",
    }
}

/// A boxed future, so that [`UpdateProvider`] stays usable as
/// `dyn UpdateProvider` (what `#[async_trait]` would generate, without the
/// dependency).
pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, UpdateError>> + Send + 'a>>;

/// Download progress: bytes so far and the total, if known.
pub type ProgressFn<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);

/// One way of updating an installation (README section 3). The desktop app
/// implements it per [`ProviderKind`] and chooses the implementation once
/// at start-up with [`select_provider`].
pub trait UpdateProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn capabilities(&self) -> ProviderCapabilities;
    /// Look for an update the channel accepts. Never downloads a package.
    fn check(&self) -> ProviderFuture<'_, Option<AvailableVersion>>;
    /// The result of the last check.
    fn available_version(&self) -> Option<AvailableVersion>;
    /// Download and verify; `GithubRelease` only, every other provider
    /// returns [`UpdateError::Unsupported`] ([`require_self_update`]).
    fn prepare<'a>(&'a self, progress: ProgressFn<'a>) -> ProviderFuture<'a, ()>;
    /// Replace the installation; `GithubRelease` only.
    fn install(&self) -> ProviderFuture<'_, ()>;
    fn status(&self) -> ProviderStatus;
    /// Whether `capabilities().automatic_updates`; the background task asks
    /// this.
    fn supports_automatic_updates(&self) -> bool {
        self.capabilities().automatic_updates
    }
}

/// Why a build gets [`ProviderKind::None`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum NotConfigured {
    /// A GitHub build without the compiled updater public key.
    #[error("no update key is compiled into this build")]
    NoUpdaterKey,
    /// A GitHub build whose package format is unknown: neither compiled in
    /// nor embedded by the bundler (a development build run from `target/`).
    #[error("the package format of this build is unknown")]
    UnknownPackageFormat,
    /// The compiled format and the bundler's disagree (R3).
    #[error("the compiled package format {compiled} differs from the bundled {bundled}")]
    FormatMismatch {
        compiled: PackageFormat,
        bundled: PackageFormat,
    },
    /// The format is not one the GitHub updater installs, or not one of
    /// this platform (R3).
    #[error("GitHub updates do not install {0} packages on this platform")]
    FormatNotUpdatable(PackageFormat),
}

/// The provider a build gets, and for a GitHub build the format it updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Selection {
    pub kind: ProviderKind,
    /// The installed format, where known.
    pub package_format: Option<PackageFormat>,
    /// Set exactly when `kind` is [`ProviderKind::None`].
    pub not_configured: Option<NotConfigured>,
}

/// The inputs of [`select_provider`]. All of them are build-time facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildFacts<'a> {
    /// The compiled record (`deckpilot_distribution::current()`).
    pub distribution: &'a Distribution,
    /// The format the Tauri bundler embedded into this package
    /// (`tauri::utils::platform::bundle_type()`), if any. Like the record,
    /// it is patched in before signing.
    pub bundled_format: Option<PackageFormat>,
    /// Whether a non-empty updater public key is compiled in.
    pub updater_key: bool,
}

/// Chooses the provider (README sections 2.2 and 4, rules R1 and R3). A
/// pure function of build-time facts, so the provider cannot change while
/// the app runs.
pub const fn select_provider(facts: BuildFacts<'_>) -> Selection {
    let distribution = facts.distribution;
    let kind = distribution.update_provider();
    let compiled = distribution.package_format();
    let format = match (compiled, facts.bundled_format) {
        (Some(compiled), Some(bundled)) if compiled as u8 != bundled as u8 => {
            return none(
                Some(compiled),
                NotConfigured::FormatMismatch { compiled, bundled },
            );
        }
        (Some(format), _) | (None, Some(format)) => Some(format),
        (None, None) => None,
    };
    if !matches!(kind, ProviderKind::GithubRelease) {
        return Selection {
            kind,
            package_format: format,
            not_configured: None,
        };
    }
    let Some(format) = format else {
        return none(None, NotConfigured::UnknownPackageFormat);
    };
    if format.updater_suffix().is_none()
        || format.platform() as u8 != distribution.platform() as u8
        || !distribution.distribution_channel().allows_format(format)
    {
        return none(Some(format), NotConfigured::FormatNotUpdatable(format));
    }
    if !facts.updater_key {
        return none(Some(format), NotConfigured::NoUpdaterKey);
    }
    Selection {
        kind: ProviderKind::GithubRelease,
        package_format: Some(format),
        not_configured: None,
    }
}

const fn none(package_format: Option<PackageFormat>, reason: NotConfigured) -> Selection {
    Selection {
        kind: ProviderKind::None,
        package_format,
        not_configured: Some(reason),
    }
}

/// The status a provider starts in.
pub fn initial_status(distribution: &Distribution, selection: &Selection) -> ProviderStatus {
    match (selection.kind, selection.not_configured) {
        (ProviderKind::GithubRelease, _) => ProviderStatus::Idle,
        (ProviderKind::ExternalPackageManager | ProviderKind::StoreManaged, _) => {
            ProviderStatus::Managed {
                by: distribution.distribution_channel(),
                instruction: update_instruction(distribution.distribution_channel())
                    .map(str::to_owned),
            }
        }
        (ProviderKind::None, reason) => ProviderStatus::NotConfigured {
            reason: reason.map_or_else(|| "not configured".to_owned(), |r| r.to_string()),
        },
    }
}

/// How the user updates an installation the app does not update itself.
pub const fn update_instruction(channel: DistributionChannel) -> Option<&'static str> {
    match channel {
        DistributionChannel::Github => None,
        DistributionChannel::Apt => Some("sudo apt update && sudo apt upgrade"),
        DistributionChannel::Homebrew => Some("brew upgrade --cask deckpilot"),
        // The identifier is decided when the manifest is first submitted.
        DistributionChannel::Winget => Some("winget upgrade KnittingCode.DeckPilot"),
        DistributionChannel::Flathub => {
            Some("Updates are installed by `flatpak update` or your software center")
        }
        DistributionChannel::MacAppStore => Some("Updates are installed by the App Store"),
        DistributionChannel::MicrosoftStore => Some("Updates are installed by the Microsoft Store"),
    }
}

/// **R1.** Only a `github` build may construct the Tauri updater, and only
/// when its provider was selected as `GithubRelease`. Every other channel
/// has no code path to it. (The build configuration, PR C and D, also
/// leaves the updater key, and for stores the updater plugin, out.)
pub const fn may_construct_tauri_updater(
    distribution: &Distribution,
    selection: &Selection,
) -> bool {
    matches!(
        distribution.distribution_channel(),
        DistributionChannel::Github
    ) && matches!(distribution.update_provider(), ProviderKind::GithubRelease)
        && matches!(selection.kind, ProviderKind::GithubRelease)
}

/// **R2.** The guard at the start of `prepare` and `install`: a store or
/// package-manager build never downloads or installs a GitHub update, even
/// if a manifest is reachable and the user asks.
pub fn require_self_update(kind: ProviderKind, operation: &'static str) -> Result<(), UpdateError> {
    if ProviderCapabilities::of(kind, false).self_update {
        Ok(())
    } else {
        Err(UpdateError::Unsupported { kind, operation })
    }
}

/// The exact GitHub manifest key of a selection: `{os}-{arch}-{format}`,
/// for example `darwin-aarch64-app` or `linux-x86_64-appimage`. `None`
/// unless the provider is `GithubRelease`.
pub fn manifest_key(distribution: &Distribution, selection: &Selection) -> Option<String> {
    if !matches!(selection.kind, ProviderKind::GithubRelease) {
        return None;
    }
    let suffix = selection.package_format?.updater_suffix()?;
    Some(format!(
        "{}-{}-{suffix}",
        distribution.platform().updater_os(),
        distribution.architecture().name()
    ))
}

/// A manifest or agent the rules refuse.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Rejection {
    #[error("this installation is not updated from GitHub Releases")]
    NotGithubRelease,
    #[error("the update manifest has no `{0}` entry for this installation's format")]
    MissingManifestEntry(String),
    #[error("the update is a `{offered}` package, but this installation is `{expected}`")]
    OtherFormat { expected: String, offered: String },
    #[error("version {offered} is not an update for {current} on the {} channel", .channel.name())]
    NotAnUpdate {
        channel: Channel,
        current: Version,
        offered: Version,
    },
    #[error("the agent does not report its distribution")]
    AgentWithoutDistribution,
    #[error("the agent was installed from {agent}, the app from {app}")]
    AgentFromOtherChannel {
        app: DistributionChannel,
        agent: DistributionChannel,
    },
    #[error("the agent is version {agent}, the app {app}")]
    AgentOtherVersion {
        app: &'static str,
        agent: &'static str,
    },
    #[error("the agent was built for another platform or architecture")]
    AgentOtherTarget,
    #[error("changing from {from} to {to} is a reinstall: install DeckPilot from {to} instead")]
    ChannelChangeIsReinstall {
        from: DistributionChannel,
        to: DistributionChannel,
    },
}

/// **R3.** A GitHub build self-updates only if the manifest has the exact
/// `{os}-{arch}-{format}` key of its format. Returns that key.
pub fn require_manifest_entry<'k>(
    distribution: &Distribution,
    selection: &Selection,
    manifest_keys: impl IntoIterator<Item = &'k str>,
) -> Result<String, Rejection> {
    let key = manifest_key(distribution, selection).ok_or(Rejection::NotGithubRelease)?;
    if manifest_keys.into_iter().any(|k| k == key) {
        Ok(key)
    } else {
        Err(Rejection::MissingManifestEntry(key))
    }
}

/// **R4.** A GitHub update installs the format the build was installed as:
/// the artifact's manifest key must be the exact key, never a plain
/// `{os}-{arch}` fallback or another format's key (no `.deb` over an
/// AppImage, no NSIS over MSI).
pub fn require_same_format(
    distribution: &Distribution,
    selection: &Selection,
    artifact_key: &str,
) -> Result<(), Rejection> {
    let expected = manifest_key(distribution, selection).ok_or(Rejection::NotGithubRelease)?;
    if artifact_key == expected {
        Ok(())
    } else {
        Err(Rejection::OtherFormat {
            expected,
            offered: artifact_key.to_owned(),
        })
    }
}

/// **R5.** Never an older version, never a pre-release on the stable
/// channel: [`Channel::accepts`], unchanged, and only for a provider that
/// installs updates at all.
pub fn require_update(
    kind: ProviderKind,
    channel: Channel,
    current: &Version,
    offered: &Version,
) -> Result<(), Rejection> {
    if !matches!(kind, ProviderKind::GithubRelease) {
        return Err(Rejection::NotGithubRelease);
    }
    if channel.accepts(current, offered) {
        Ok(())
    } else {
        Err(Rejection::NotAnUpdate {
            channel,
            current: current.clone(),
            offered: offered.clone(),
        })
    }
}

/// **R6.** The app works only with an agent of its own channel, version
/// and target; otherwise it restarts the agent from its own bundle. An
/// agent that does not report its record is refused as well (PR E adds the
/// record to `GetStatus`).
pub fn check_agent(app: &Distribution, agent: Option<&Distribution>) -> Result<(), Rejection> {
    let agent = agent.ok_or(Rejection::AgentWithoutDistribution)?;
    if agent.distribution_channel() != app.distribution_channel() {
        return Err(Rejection::AgentFromOtherChannel {
            app: app.distribution_channel(),
            agent: agent.distribution_channel(),
        });
    }
    if agent.application_version() != app.application_version() {
        return Err(Rejection::AgentOtherVersion {
            app: app.application_version(),
            agent: agent.application_version(),
        });
    }
    if agent.platform() != app.platform() || agent.architecture() != app.architecture() {
        return Err(Rejection::AgentOtherTarget);
    }
    Ok(())
}

/// **R7.** Only the `apt` variant of the `.deb`, identified by its embedded
/// record, is published into the APT repository; the GitHub `.deb` never
/// is. (Enforced by the downstream `distribute-apt` job, PR D and G.)
pub fn may_publish_to_apt_repository(
    channel: DistributionChannel,
    format: Option<PackageFormat>,
) -> bool {
    channel == DistributionChannel::Apt && format == Some(PackageFormat::Deb)
}

/// **R8.** Changing channels is a reinstall by the user, never an in-app
/// action: the installed variant decides. Staying on the same channel is
/// the only thing this allows.
pub fn change_channel(from: DistributionChannel, to: DistributionChannel) -> Result<(), Rejection> {
    if from == to {
        Ok(())
    } else {
        Err(Rejection::ChannelChangeIsReinstall { from, to })
    }
}

// **R9** (no release-build input can change the record) holds by
// construction: every input of `select_provider` is a build-time fact, and
// `deckpilot_distribution` reads the record only while compiling. Its
// `tests/release_build.rs` checks a release binary; PR D checks the
// shipped ones.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::task::{Context, Poll, Waker};

    const VERSION: &str = "0.6.0-alpha.3";

    fn record(
        channel: DistributionChannel,
        format: Option<PackageFormat>,
        platform: Platform,
    ) -> Distribution {
        Distribution::new(channel, format, VERSION, platform, Architecture::X86_64).unwrap()
    }

    fn select(distribution: &Distribution, bundled: Option<PackageFormat>, key: bool) -> Selection {
        select_provider(BuildFacts {
            distribution,
            bundled_format: bundled,
            updater_key: key,
        })
    }

    /// Polls a future that never waits (the fakes below).
    fn ready<T>(future: impl Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("the fake provider never waits"),
        }
    }

    #[test]
    fn release_channels_map_onto_the_existing_channel_semantics() {
        // Exactly the versions that form release tags have a release
        // channel, and its kind is the tag's kind.
        for version in [
            "0.6.0",
            "1.2.3",
            "0.6.0-alpha.3",
            "0.6.0-beta.1",
            "1.0.0-rc.10",
            "0.6.0-alpha.0",
            "0.6.0-dev.1",
            "0.6.0+build",
            "0.5.0-1",
            "01.0.0",
            "0.6",
            "99999999999999999999.0.0",
        ] {
            let tag = crate::channel::ReleaseTag::for_version(version).ok();
            let channel = ReleaseChannel::for_version(version);
            assert_eq!(channel.map(release_kind), tag.map(|t| t.kind), "{version}");
        }
        assert_eq!(release_kind(ReleaseChannel::Stable), ReleaseKind::Stable);
        assert_eq!(release_kind(ReleaseChannel::Beta), ReleaseKind::Prerelease);
        assert_eq!(release_kind(ReleaseChannel::Alpha), ReleaseKind::Prerelease);
        assert_eq!(
            ReleaseChannel::for_version("1.0.0-rc.1"),
            Some(ReleaseChannel::Beta)
        );
        assert_eq!(
            default_update_channel(ReleaseChannel::Stable),
            Channel::Stable
        );
        assert_eq!(
            default_update_channel(ReleaseChannel::Alpha),
            Channel::Prerelease
        );
        // The installed 0.6.0-alpha.3 is a pre-release on the prerelease
        // channel, as today.
        assert_eq!(
            ReleaseChannel::for_version(VERSION).map(default_update_channel),
            Some(Channel::Prerelease)
        );
    }

    #[test]
    fn selection_follows_the_channel_table() {
        use DistributionChannel as C;
        use PackageFormat as F;
        let cases = [
            (
                C::Apt,
                Platform::Linux,
                ProviderKind::ExternalPackageManager,
            ),
            (
                C::Homebrew,
                Platform::Macos,
                ProviderKind::ExternalPackageManager,
            ),
            (
                C::Winget,
                Platform::Windows,
                ProviderKind::ExternalPackageManager,
            ),
            (C::Flathub, Platform::Linux, ProviderKind::StoreManaged),
            (C::MacAppStore, Platform::Macos, ProviderKind::StoreManaged),
            (
                C::MicrosoftStore,
                Platform::Windows,
                ProviderKind::StoreManaged,
            ),
        ];
        for (channel, platform, kind) in cases {
            let distribution = record(channel, None, platform);
            // With or without an updater key: never GitHub.
            for key in [false, true] {
                let selection = select(&distribution, None, key);
                assert_eq!(selection.kind, kind, "{channel}");
                assert_eq!(selection.not_configured, None);
                assert!(!may_construct_tauri_updater(&distribution, &selection));
                assert_eq!(manifest_key(&distribution, &selection), None);
            }
        }
        let mac = record(C::Github, None, Platform::Macos);
        let selection = select(&mac, None, true);
        assert_eq!(selection.kind, ProviderKind::GithubRelease);
        assert_eq!(selection.package_format, Some(F::App));
        assert_eq!(
            manifest_key(&mac, &selection).as_deref(),
            Some("darwin-x86_64-app")
        );
    }

    #[test]
    fn r1_only_github_builds_reach_the_tauri_updater() {
        use DistributionChannel as C;
        for &channel in C::ALL {
            for &platform in Platform::ALL {
                let Ok(distribution) =
                    Distribution::new(channel, None, VERSION, platform, Architecture::Aarch64)
                else {
                    continue;
                };
                for &bundled in PackageFormat::ALL {
                    for key in [false, true] {
                        let selection = select(&distribution, Some(bundled), key);
                        let allowed = may_construct_tauri_updater(&distribution, &selection);
                        assert_eq!(
                            allowed,
                            channel == C::Github && selection.kind == ProviderKind::GithubRelease,
                            "{channel} {platform} {bundled} key={key}"
                        );
                        if allowed {
                            assert!(key);
                        }
                    }
                }
            }
        }
        // A GitHub build without an updater key gets no updater either.
        let linux = record(C::Github, None, Platform::Linux);
        let selection = select(&linux, Some(PackageFormat::Deb), false);
        assert_eq!(selection.kind, ProviderKind::None);
        assert_eq!(selection.not_configured, Some(NotConfigured::NoUpdaterKey));
        assert!(!may_construct_tauri_updater(&linux, &selection));
    }

    #[test]
    fn r2_managed_providers_never_prepare_or_install() {
        for kind in [
            ProviderKind::ExternalPackageManager,
            ProviderKind::StoreManaged,
            ProviderKind::None,
        ] {
            for operation in ["download", "install"] {
                assert_eq!(
                    require_self_update(kind, operation),
                    Err(UpdateError::Unsupported { kind, operation })
                );
            }
            let capabilities = ProviderCapabilities::of(kind, true);
            assert!(!capabilities.self_update);
            assert!(!capabilities.automatic_updates);
            assert!(!capabilities.channel_choice);
        }
        assert_eq!(
            require_self_update(ProviderKind::GithubRelease, "install"),
            Ok(())
        );
        assert_eq!(
            ProviderCapabilities::of(ProviderKind::GithubRelease, false),
            ProviderCapabilities {
                self_update: true,
                automatic_updates: true,
                channel_choice: true,
                version_check: true,
            }
        );
        // The version notice of package managers is the user's setting,
        // off by default; stores never check.
        assert!(
            !ProviderCapabilities::of(ProviderKind::ExternalPackageManager, false).version_check
        );
        assert!(ProviderCapabilities::of(ProviderKind::ExternalPackageManager, true).version_check);
        assert!(!ProviderCapabilities::of(ProviderKind::StoreManaged, true).version_check);
    }

    #[test]
    fn r3_the_package_format_must_be_known_agree_and_be_offered() {
        use PackageFormat as F;
        let linux = record(DistributionChannel::Github, None, Platform::Linux);
        // A development build run from target/: no bundle type.
        let selection = select(&linux, None, true);
        assert_eq!(selection.kind, ProviderKind::None);
        assert_eq!(
            selection.not_configured,
            Some(NotConfigured::UnknownPackageFormat)
        );
        for (format, key) in [
            (F::Deb, "linux-x86_64-deb"),
            (F::Rpm, "linux-x86_64-rpm"),
            (F::AppImage, "linux-x86_64-appimage"),
        ] {
            let selection = select(&linux, Some(format), true);
            assert_eq!(selection.kind, ProviderKind::GithubRelease);
            assert_eq!(
                require_manifest_entry(&linux, &selection, ["linux-x86_64", key]),
                Ok(key.to_owned())
            );
            // The plain `{os}-{arch}` key alone is not enough.
            assert_eq!(
                require_manifest_entry(&linux, &selection, ["linux-x86_64"]),
                Err(Rejection::MissingManifestEntry(key.to_owned()))
            );
        }
        // A bundle type of another platform, or one the updater never
        // installs.
        for format in [F::Msi, F::Flatpak, F::App] {
            let selection = select(&linux, Some(format), true);
            assert_eq!(
                selection.not_configured,
                Some(NotConfigured::FormatNotUpdatable(format)),
                "{format}"
            );
        }
        // Compiled and bundled formats must agree.
        let mac = record(DistributionChannel::Github, Some(F::App), Platform::Macos);
        assert_eq!(
            select(&mac, Some(F::App), true).kind,
            ProviderKind::GithubRelease
        );
        assert_eq!(
            select(&mac, Some(F::Deb), true).not_configured,
            Some(NotConfigured::FormatMismatch {
                compiled: F::App,
                bundled: F::Deb
            })
        );
        // Mismatches also disable the managed providers: fail closed.
        let apt = record(DistributionChannel::Apt, None, Platform::Linux);
        assert_eq!(
            select(&apt, Some(F::Deb), false).kind,
            ProviderKind::ExternalPackageManager
        );
        assert_eq!(
            select(&apt, Some(F::AppImage), false).kind,
            ProviderKind::None
        );
        // Not a GitHub provider: no manifest entry is ever good enough.
        let selection = select(&apt, None, false);
        assert_eq!(
            require_manifest_entry(&apt, &selection, ["linux-x86_64-deb"]),
            Err(Rejection::NotGithubRelease)
        );
    }

    #[test]
    fn r4_updates_keep_the_installed_format() {
        use PackageFormat as F;
        let windows = record(DistributionChannel::Github, None, Platform::Windows);
        let msi = select(&windows, Some(F::Msi), true);
        assert_eq!(
            require_same_format(&windows, &msi, "windows-x86_64-msi"),
            Ok(())
        );
        for other in ["windows-x86_64-nsis", "windows-x86_64", "linux-x86_64-msi"] {
            assert!(
                require_same_format(&windows, &msi, other).is_err(),
                "{other}"
            );
        }
        let linux = record(DistributionChannel::Github, None, Platform::Linux);
        let appimage = select(&linux, Some(F::AppImage), true);
        assert_eq!(
            require_same_format(&linux, &appimage, "linux-x86_64-deb"),
            Err(Rejection::OtherFormat {
                expected: "linux-x86_64-appimage".to_owned(),
                offered: "linux-x86_64-deb".to_owned(),
            })
        );
    }

    #[test]
    fn r5_never_older_never_a_prerelease_on_stable() {
        let v = |s: &str| Version::parse(s).unwrap();
        let github = ProviderKind::GithubRelease;
        assert!(
            require_update(
                github,
                Channel::Prerelease,
                &v(VERSION),
                &v("0.6.0-alpha.4")
            )
            .is_ok()
        );
        assert!(require_update(github, Channel::Stable, &v("0.6.0"), &v("0.7.0-beta.1")).is_err());
        assert!(require_update(github, Channel::Prerelease, &v("0.6.0"), &v("0.5.9")).is_err());
        assert!(require_update(github, Channel::Prerelease, &v("0.6.0"), &v("0.6.0")).is_err());
        assert_eq!(
            require_update(
                ProviderKind::StoreManaged,
                Channel::Stable,
                &v("0.6.0"),
                &v("0.7.0")
            ),
            Err(Rejection::NotGithubRelease)
        );
    }

    #[test]
    fn r6_an_agent_from_another_channel_is_refused() {
        let app = record(DistributionChannel::Github, None, Platform::Linux);
        assert_eq!(check_agent(&app, Some(&app)), Ok(()));
        assert_eq!(
            check_agent(&app, None),
            Err(Rejection::AgentWithoutDistribution)
        );
        let apt = record(DistributionChannel::Apt, None, Platform::Linux);
        assert_eq!(
            check_agent(&app, Some(&apt)),
            Err(Rejection::AgentFromOtherChannel {
                app: DistributionChannel::Github,
                agent: DistributionChannel::Apt,
            })
        );
        let older = Distribution::new(
            DistributionChannel::Github,
            None,
            "0.6.0-alpha.2",
            Platform::Linux,
            Architecture::X86_64,
        )
        .unwrap();
        assert!(matches!(
            check_agent(&app, Some(&older)),
            Err(Rejection::AgentOtherVersion { .. })
        ));
        let arm = Distribution::new(
            DistributionChannel::Github,
            None,
            VERSION,
            Platform::Linux,
            Architecture::Aarch64,
        )
        .unwrap();
        assert_eq!(
            check_agent(&app, Some(&arm)),
            Err(Rejection::AgentOtherTarget)
        );
    }

    #[test]
    fn r7_only_the_apt_variant_enters_the_apt_repository() {
        use DistributionChannel as C;
        assert!(may_publish_to_apt_repository(
            C::Apt,
            Some(PackageFormat::Deb)
        ));
        assert!(!may_publish_to_apt_repository(
            C::Github,
            Some(PackageFormat::Deb)
        ));
        assert!(!may_publish_to_apt_repository(C::Apt, None));
        for &channel in C::ALL {
            for &format in PackageFormat::ALL {
                assert_eq!(
                    may_publish_to_apt_repository(channel, Some(format)),
                    channel == C::Apt && format == PackageFormat::Deb
                );
            }
        }
    }

    #[test]
    fn r8_changing_channels_is_a_reinstall() {
        use DistributionChannel as C;
        for &from in C::ALL {
            for &to in C::ALL {
                assert_eq!(
                    change_channel(from, to).is_ok(),
                    from == to,
                    "{from} -> {to}"
                );
            }
        }
    }

    #[test]
    fn r9_selection_depends_only_on_build_facts() {
        // The compiled record of this test build selects the same provider
        // every time; there is no other input.
        let current = deckpilot_distribution::DISTRIBUTION;
        let facts = BuildFacts {
            distribution: &current,
            bundled_format: None,
            updater_key: false,
        };
        assert_eq!(select_provider(facts), select_provider(facts));
        assert!(!current.is_development_override());
    }

    #[test]
    fn statuses_and_instructions() {
        let apt = record(DistributionChannel::Apt, None, Platform::Linux);
        assert_eq!(
            initial_status(&apt, &select(&apt, None, false)),
            ProviderStatus::Managed {
                by: DistributionChannel::Apt,
                instruction: Some("sudo apt update && sudo apt upgrade".to_owned()),
            }
        );
        let linux = record(DistributionChannel::Github, None, Platform::Linux);
        assert_eq!(
            initial_status(&linux, &select(&linux, Some(PackageFormat::Deb), true)),
            ProviderStatus::Idle
        );
        assert_eq!(
            initial_status(&linux, &select(&linux, Some(PackageFormat::Deb), false)),
            ProviderStatus::NotConfigured {
                reason: "no update key is compiled into this build".to_owned()
            }
        );
        for &channel in DistributionChannel::ALL {
            assert_eq!(
                update_instruction(channel).is_none(),
                channel == DistributionChannel::Github
            );
        }
        assert_eq!(
            UpdateError::Unsupported {
                kind: ProviderKind::StoreManaged,
                operation: "install"
            }
            .to_string(),
            "install is not available: updates of this installation are installed by the store"
        );
    }

    /// A store provider as PR C would write it: no I/O, no updater.
    #[derive(Debug)]
    struct Store {
        channel: DistributionChannel,
        calls: Mutex<Vec<&'static str>>,
    }

    impl UpdateProvider for Store {
        fn kind(&self) -> ProviderKind {
            ProviderKind::StoreManaged
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::of(self.kind(), false)
        }
        fn check(&self) -> ProviderFuture<'_, Option<AvailableVersion>> {
            self.calls.lock().unwrap().push("check");
            Box::pin(async { Ok(None) })
        }
        fn available_version(&self) -> Option<AvailableVersion> {
            None
        }
        fn prepare<'a>(&'a self, _progress: ProgressFn<'a>) -> ProviderFuture<'a, ()> {
            let result = require_self_update(self.kind(), "download");
            Box::pin(async move { result })
        }
        fn install(&self) -> ProviderFuture<'_, ()> {
            let result = require_self_update(self.kind(), "install");
            Box::pin(async move { result })
        }
        fn status(&self) -> ProviderStatus {
            ProviderStatus::Managed {
                by: self.channel,
                instruction: update_instruction(self.channel).map(str::to_owned),
            }
        }
    }

    #[test]
    fn the_trait_is_object_safe_and_managed_providers_refuse_installs() {
        let provider: Box<dyn UpdateProvider> = Box::new(Store {
            channel: DistributionChannel::MacAppStore,
            calls: Mutex::new(Vec::new()),
        });
        assert_eq!(ready(provider.check()), Ok(None));
        assert_eq!(provider.available_version(), None);
        let progress = |_: u64, _: Option<u64>| panic!("nothing is downloaded");
        assert!(matches!(
            ready(provider.prepare(&progress)),
            Err(UpdateError::Unsupported { .. })
        ));
        assert!(matches!(
            ready(provider.install()),
            Err(UpdateError::Unsupported { .. })
        ));
        assert!(!provider.supports_automatic_updates());
        assert!(matches!(
            provider.status(),
            ProviderStatus::Managed {
                by: DistributionChannel::MacAppStore,
                ..
            }
        ));
    }
}
