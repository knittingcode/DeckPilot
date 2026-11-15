//! The update provider of this build (docs/distribution/README.md, sections
//! 3 and 4): who updates the installation, and what the app may do about it.
//!
//! The provider is chosen once, from build-time facts only: the compiled
//! distribution record (`deckpilot_distribution::current()`), the package
//! format the Tauri bundler patched into this binary
//! (`tauri::utils::platform::bundle_type()`), and whether an updater key is
//! compiled in. Nothing read at run time can change it (rule R9).
//!
//! | Provider | Check | Download / install |
//! | --- | --- | --- |
//! | `GithubRelease` | The Tauri updater, as before ([`crate::updates`]) | The Tauri updater, minisign verified |
//! | `ExternalPackageManager` (`apt`, `homebrew`, `winget`) | Nothing (no I/O) | Refused (R2) |
//! | `StoreManaged` (`flathub`, `mac_app_store`, `microsoft_store`) | Nothing (no I/O) | Refused (R2) |
//! | `None` (no key, unknown or mismatched format) | Refused: not configured | Refused |
//!
//! Only [`for_app`] constructs a provider, and it constructs the GitHub one,
//! the only code path to the Tauri updater, exactly when
//! [`deckpilot_update::provider::may_construct_tauri_updater`] allows it
//! (R1). Builds without the `self-update` feature have no such code path at
//! all.

use serde::Serialize;
use tauri::{AppHandle, Runtime};

use deckpilot_update::provider::{
    self, AvailableVersion, BuildFacts, Distribution, PackageFormat, ProgressFn,
    ProviderCapabilities, ProviderFuture, ProviderKind, ProviderStatus, Selection, UpdateError,
    UpdateProvider,
};

/// The provider this build gets, chosen once (at start-up, when the
/// [`crate::updates::UpdateManager`] is created).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveProvider {
    pub distribution: Distribution,
    pub selection: Selection,
}

impl ActiveProvider {
    /// The provider for these build facts; tests pass their own record.
    pub const fn select(
        distribution: Distribution,
        bundled_format: Option<PackageFormat>,
        updater_key: bool,
    ) -> Self {
        let selection = provider::select_provider(BuildFacts {
            distribution: &distribution,
            bundled_format,
            updater_key,
        });
        Self {
            distribution,
            selection,
        }
    }

    /// The provider of the running build.
    pub fn current() -> Self {
        Self::select(
            *deckpilot_distribution::current(),
            bundled_format(),
            crate::updates::self_update_compiled(),
        )
    }

    pub const fn kind(&self) -> ProviderKind {
        self.selection.kind
    }

    /// The capabilities of the selected kind. The version notice of package
    /// managers is a setting that does not exist yet (PR F); it is off.
    pub const fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::of(self.selection.kind, false)
    }

    /// R1: whether this build may construct the Tauri updater at all.
    pub const fn may_self_update(&self) -> bool {
        provider::may_construct_tauri_updater(&self.distribution, &self.selection)
    }

    /// The exact manifest key this installation updates from
    /// (`{os}-{arch}-{format}`, R3 and R4); `None` unless it self-updates.
    pub fn manifest_key(&self) -> Option<String> {
        if self.may_self_update() {
            provider::manifest_key(&self.distribution, &self.selection)
        } else {
            None
        }
    }

    /// The status the provider has before anything happened: idle, managed
    /// by a package manager or store (with the instruction), or not
    /// configured (with the reason).
    pub fn initial_status(&self) -> ProviderStatus {
        provider::initial_status(&self.distribution, &self.selection)
    }

    /// The distribution fields of `UpdateState`.
    pub fn info(&self) -> ProviderInfo {
        let capabilities = self.capabilities();
        let status = self.initial_status();
        let (managed_by, update_instruction) = match &status {
            ProviderStatus::Managed { by, .. } => {
                (Some(by.name()), provider::update_instruction(*by))
            }
            _ => (None, None),
        };
        ProviderInfo {
            distribution_channel: self.distribution.distribution_channel().name(),
            update_provider: self.kind().name(),
            release_channel: self.distribution.release_channel().name(),
            supports_automatic_updates: capabilities.automatic_updates,
            package_format: self.selection.package_format.map(PackageFormat::name),
            capabilities: Capabilities {
                self_update: capabilities.self_update,
                automatic_updates: capabilities.automatic_updates,
                channel_choice: capabilities.channel_choice,
                version_check: capabilities.version_check,
            },
            managed_by,
            update_instruction,
            not_configured_reason: self.selection.not_configured.map(|r| r.to_string()),
            development_override: self.distribution.is_development_override(),
        }
    }
}

/// The package format the Tauri bundler embedded into this binary; `None`
/// for a binary run from `target/` on Windows and Linux (macOS is always an
/// `.app`).
pub fn bundled_format() -> Option<PackageFormat> {
    use tauri::utils::config::BundleType;
    Some(match tauri::utils::platform::bundle_type()? {
        BundleType::Deb => PackageFormat::Deb,
        BundleType::Rpm => PackageFormat::Rpm,
        BundleType::AppImage => PackageFormat::AppImage,
        BundleType::Msi => PackageFormat::Msi,
        BundleType::Nsis => PackageFormat::Nsis,
        BundleType::App | BundleType::Dmg => PackageFormat::App,
    })
}

/// What `UpdateState` adds for the distribution-aware UI (PR F). Names are
/// the snake-case names of `docs/distribution/README.md`, section 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    /// Who delivered this build: `github`, `apt`, `mac_app_store`, ….
    pub distribution_channel: &'static str,
    /// The selected provider: `github_release`, `external_package_manager`,
    /// `store_managed`, or `none` (also for a GitHub build that cannot
    /// update itself, see `not_configured_reason`).
    pub update_provider: &'static str,
    /// The class of this build's own version: `stable`, `beta`, `alpha`.
    pub release_channel: &'static str,
    /// Whether the background task checks, downloads and installs.
    pub supports_automatic_updates: bool,
    /// The installed format, where known: `app`, `msi`, `deb`, ….
    pub package_format: Option<&'static str>,
    pub capabilities: Capabilities,
    /// The channel that updates this installation, when it is not the app.
    pub managed_by: Option<&'static str>,
    /// How the user updates it then: a command or where the store does it.
    pub update_instruction: Option<&'static str>,
    /// Why this build has no updates, for the `none` provider.
    pub not_configured_reason: Option<String>,
    /// A debug build's development override replaced the compiled channel.
    pub development_override: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct Capabilities {
    pub self_update: bool,
    pub automatic_updates: bool,
    pub channel_choice: bool,
    pub version_check: bool,
}

/// The provider of `app`'s build, as an [`UpdateProvider`]. The commands
/// and the background task go through this.
pub fn for_app<R: Runtime>(app: &AppHandle<R>) -> Box<dyn UpdateProvider + '_> {
    let active = crate::updates::active_provider(app);
    #[cfg(feature = "self-update")]
    if active.may_self_update() {
        return Box::new(GithubReleaseProvider { app, source: None });
    }
    Box::new(NotSelfUpdating { active })
}

/// `ExternalPackageManager`, `StoreManaged` and `None`: no Tauri updater, no
/// network, no package (R1, R2).
#[derive(Debug, Clone, Copy)]
pub struct NotSelfUpdating {
    pub active: ActiveProvider,
}

impl UpdateProvider for NotSelfUpdating {
    fn kind(&self) -> ProviderKind {
        self.active.kind()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.active.capabilities()
    }

    fn check(&self) -> ProviderFuture<'_, Option<AvailableVersion>> {
        let result = match (self.kind(), self.active.selection.not_configured) {
            // Stores notify on their own; the version notice of package
            // managers is off (and does not exist yet). Never any I/O.
            (ProviderKind::ExternalPackageManager | ProviderKind::StoreManaged, _) => Ok(None),
            (_, Some(reason)) => Err(UpdateError::NotConfigured(reason)),
            (kind, None) => Err(UpdateError::Unsupported {
                kind,
                operation: "checking for updates",
            }),
        };
        Box::pin(async move { result })
    }

    fn available_version(&self) -> Option<AvailableVersion> {
        None
    }

    fn prepare<'a>(&'a self, _progress: ProgressFn<'a>) -> ProviderFuture<'a, ()> {
        let result = self.refuse("downloading an update");
        Box::pin(async move { result })
    }

    fn install(&self) -> ProviderFuture<'_, ()> {
        let result = self.refuse("installing an update");
        Box::pin(async move { result })
    }

    fn status(&self) -> ProviderStatus {
        self.active.initial_status()
    }
}

impl NotSelfUpdating {
    /// R2, and for `None` the reason it has no updates.
    fn refuse(&self, operation: &'static str) -> Result<(), UpdateError> {
        provider::require_self_update(self.kind(), operation)?;
        // Not reached: only the GitHub provider self-updates.
        Err(UpdateError::Unsupported {
            kind: self.kind(),
            operation,
        })
    }
}

/// The GitHub Releases provider: today's updater in [`crate::updates`],
/// unchanged, behind the interface.
#[cfg(feature = "self-update")]
pub struct GithubReleaseProvider<'a, R: Runtime> {
    pub(crate) app: &'a AppHandle<R>,
    /// Where updates come from; `None` is the release source of the
    /// build. Tests pass a loopback server.
    pub(crate) source: Option<crate::updates::Source>,
}

#[cfg(feature = "self-update")]
impl<R: Runtime> UpdateProvider for GithubReleaseProvider<'_, R> {
    fn kind(&self) -> ProviderKind {
        ProviderKind::GithubRelease
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::of(ProviderKind::GithubRelease, false)
    }

    fn check(&self) -> ProviderFuture<'_, Option<AvailableVersion>> {
        Box::pin(async move {
            let found = match &self.source {
                Some(source) => crate::updates::check_from(self.app, source).await,
                None => crate::updates::check(self.app).await,
            };
            found
                .map(|info| info.map(crate::updates::UpdateInfo::into_available))
                .map_err(|error| failed(provider::Stage::Check, &error))
        })
    }

    fn available_version(&self) -> Option<AvailableVersion> {
        crate::updates::available(self.app).map(crate::updates::UpdateInfo::into_available)
    }

    fn prepare<'a>(&'a self, _progress: ProgressFn<'a>) -> ProviderFuture<'a, ()> {
        // The download publishes its progress in the update state itself.
        Box::pin(async move {
            crate::updates::download(self.app)
                .await
                .map_err(|error| failed(provider::Stage::Download, &error))
        })
    }

    fn install(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            crate::updates::install(self.app)
                .await
                .map_err(|error| failed(provider::Stage::Install, &error))
        })
    }

    fn status(&self) -> ProviderStatus {
        crate::updates::provider_status(self.app)
    }
}

#[cfg(feature = "self-update")]
fn failed(stage: provider::Stage, error: &crate::error::CommandError) -> UpdateError {
    UpdateError::Failed {
        stage,
        message: error.to_string(),
    }
}

/// The message a command returns for a provider error. The GitHub
/// provider's messages are today's, word for word.
pub fn command_error(error: UpdateError) -> crate::error::CommandError {
    crate::error::CommandError::Invalid(match error {
        UpdateError::Failed { message, .. } => message,
        UpdateError::NotConfigured(provider::NotConfigured::NoUpdaterKey) => {
            "updates are not configured in this build".to_owned()
        }
        // Today's wording too, with the reason (a build run from `target/`
        // on Linux or Windows has no package format).
        UpdateError::NotConfigured(reason) => {
            format!("updates are not configured in this build: {reason}")
        }
        other @ UpdateError::Unsupported { .. } => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_update::provider::{Architecture, DistributionChannel, NotConfigured, Platform};

    const VERSION: &str = "0.6.0-alpha.3";

    pub(crate) fn record(channel: DistributionChannel, platform: Platform) -> Distribution {
        Distribution::new(channel, None, VERSION, platform, Architecture::Aarch64).unwrap()
    }

    fn ready<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    #[test]
    fn each_channel_gets_its_provider() {
        use DistributionChannel as C;
        use PackageFormat as F;
        let cases = [
            (C::Github, Platform::Macos, Some(F::App), "github_release"),
            (C::Github, Platform::Linux, Some(F::Deb), "github_release"),
            (
                C::Github,
                Platform::Linux,
                Some(F::AppImage),
                "github_release",
            ),
            (C::Github, Platform::Windows, Some(F::Msi), "github_release"),
            (
                C::Github,
                Platform::Windows,
                Some(F::Nsis),
                "github_release",
            ),
            (
                C::Apt,
                Platform::Linux,
                Some(F::Deb),
                "external_package_manager",
            ),
            (
                C::Homebrew,
                Platform::Macos,
                Some(F::App),
                "external_package_manager",
            ),
            (
                C::Winget,
                Platform::Windows,
                Some(F::Msi),
                "external_package_manager",
            ),
            (C::Flathub, Platform::Linux, None, "store_managed"),
            (C::MacAppStore, Platform::Macos, None, "store_managed"),
            (C::MicrosoftStore, Platform::Windows, None, "store_managed"),
        ];
        for (channel, platform, bundled, provider) in cases {
            // Every build of these channels is made with the key in this
            // test; only GitHub may use it.
            let active = ActiveProvider::select(record(channel, platform), bundled, true);
            let info = active.info();
            assert_eq!(info.update_provider, provider, "{channel}");
            assert_eq!(info.distribution_channel, channel.name());
            assert_eq!(info.release_channel, "alpha");
            let github = channel == C::Github;
            assert_eq!(active.may_self_update(), github, "{channel}");
            assert_eq!(info.supports_automatic_updates, github, "{channel}");
            assert_eq!(info.capabilities.self_update, github, "{channel}");
            assert_eq!(info.capabilities.channel_choice, github, "{channel}");
            assert_eq!(active.manifest_key().is_some(), github, "{channel}");
            assert_eq!(info.managed_by.is_some(), !github, "{channel}");
            assert_eq!(info.update_instruction.is_some(), !github, "{channel}");
            assert_eq!(info.not_configured_reason, None);
        }
        let apt = ActiveProvider::select(record(C::Apt, Platform::Linux), Some(F::Deb), false);
        assert_eq!(apt.info().managed_by, Some("apt"));
        assert_eq!(
            apt.info().update_instruction,
            Some("sudo apt update && sudo apt upgrade")
        );
        let brew = ActiveProvider::select(record(C::Homebrew, Platform::Macos), None, false);
        assert_eq!(
            brew.info().update_instruction,
            Some("brew upgrade --cask deckpilot")
        );
        let mas = ActiveProvider::select(record(C::MacAppStore, Platform::Macos), None, false);
        assert_eq!(
            mas.info().update_instruction,
            Some("Updates are installed by the App Store")
        );
    }

    #[test]
    fn github_builds_update_the_exact_format_they_were_installed_as() {
        use PackageFormat as F;
        let github = |platform, bundled| {
            ActiveProvider::select(record(DistributionChannel::Github, platform), bundled, true)
        };
        for (platform, format, key) in [
            (Platform::Macos, Some(F::App), "darwin-aarch64-app"),
            // macOS is always an `.app`, bundled or not.
            (Platform::Macos, None, "darwin-aarch64-app"),
            (Platform::Linux, Some(F::Deb), "linux-aarch64-deb"),
            (Platform::Linux, Some(F::Rpm), "linux-aarch64-rpm"),
            (Platform::Linux, Some(F::AppImage), "linux-aarch64-appimage"),
            (Platform::Windows, Some(F::Msi), "windows-aarch64-msi"),
            (Platform::Windows, Some(F::Nsis), "windows-aarch64-nsis"),
        ] {
            let active = github(platform, format);
            assert_eq!(active.manifest_key().as_deref(), Some(key));
            assert_eq!(active.info().update_provider, "github_release");
        }
        // Without a known format there is no key to update from: not the
        // plain `{os}-{arch}` entry (which is the AppImage on Linux and the
        // MSI on Windows).
        for platform in [Platform::Linux, Platform::Windows] {
            let active = github(platform, None);
            assert_eq!(active.kind(), ProviderKind::None);
            assert_eq!(active.manifest_key(), None);
            assert_eq!(
                active.info().not_configured_reason.as_deref(),
                Some("the package format of this build is unknown")
            );
        }
    }

    #[test]
    fn a_format_mismatch_is_refused() {
        use PackageFormat as F;
        // An APT build whose binary the bundler marked as an AppImage, and a
        // GitHub build with a bundle type of another platform: no provider
        // that updates anything.
        for (active, reason) in [
            (
                ActiveProvider::select(
                    record(DistributionChannel::Apt, Platform::Linux),
                    Some(F::AppImage),
                    false,
                ),
                NotConfigured::FormatMismatch {
                    compiled: F::Deb,
                    bundled: F::AppImage,
                },
            ),
            (
                ActiveProvider::select(
                    record(DistributionChannel::Github, Platform::Linux),
                    Some(F::Msi),
                    true,
                ),
                NotConfigured::FormatNotUpdatable(F::Msi),
            ),
        ] {
            assert_eq!(active.kind(), ProviderKind::None);
            assert!(!active.may_self_update());
            assert_eq!(active.manifest_key(), None);
            let provider = NotSelfUpdating { active };
            assert_eq!(
                ready(provider.check()),
                Err(UpdateError::NotConfigured(reason))
            );
            assert!(ready(provider.prepare(&|_, _| ())).is_err());
            assert!(ready(provider.install()).is_err());
        }
    }

    #[test]
    fn package_manager_and_store_builds_refuse_to_self_update() {
        use DistributionChannel as C;
        for (channel, platform) in [
            (C::Apt, Platform::Linux),
            (C::Homebrew, Platform::Macos),
            (C::Winget, Platform::Windows),
            (C::Flathub, Platform::Linux),
            (C::MacAppStore, Platform::Macos),
            (C::MicrosoftStore, Platform::Windows),
        ] {
            let active = ActiveProvider::select(record(channel, platform), None, true);
            let provider = NotSelfUpdating { active };
            // A check is a no-op that finds nothing.
            assert_eq!(ready(provider.check()), Ok(None), "{channel}");
            for result in [
                ready(provider.prepare(&|_, _| panic!("nothing is downloaded"))),
                ready(provider.install()),
            ] {
                let error = result.unwrap_err();
                assert!(
                    matches!(error, UpdateError::Unsupported { kind, .. } if kind == active.kind()),
                    "{channel}: {error}"
                );
                let message = command_error(error).to_string();
                assert!(message.contains("is not available"), "{message}");
            }
            assert!(!provider.supports_automatic_updates());
            assert!(matches!(
                provider.status(),
                ProviderStatus::Managed { by, instruction: Some(_) } if by == channel
            ));
        }
    }

    #[test]
    fn builds_without_updates_say_why() {
        let active = ActiveProvider::select(
            record(DistributionChannel::Github, Platform::Macos),
            None,
            false,
        );
        assert_eq!(active.kind(), ProviderKind::None);
        let provider = NotSelfUpdating { active };
        // Today's message for a build without a key.
        assert_eq!(
            command_error(ready(provider.check()).unwrap_err()).to_string(),
            "updates are not configured in this build"
        );
        assert_eq!(
            active.info().not_configured_reason.as_deref(),
            Some("no update key is compiled into this build")
        );
        assert!(matches!(
            provider.status(),
            ProviderStatus::NotConfigured { .. }
        ));
    }

    #[test]
    fn the_running_test_build_is_a_github_build_without_a_key() {
        // Test builds compile no updater key, as development builds.
        let active = ActiveProvider::current();
        assert!(!active.distribution.is_development_override());
        if crate::updates::PUBLIC_KEY.is_none() {
            assert_eq!(active.kind(), ProviderKind::None);
            assert!(!active.may_self_update());
        }
    }
}
