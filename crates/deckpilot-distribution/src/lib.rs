//! The distribution record of a DeckPilot build: who delivered this artifact
//! (and therefore owns its installation), how it is updated, its package
//! format, its release channel, version, platform and architecture
//! (docs/distribution/README.md, sections 1 and 2).
//!
//! The record is **compiled in and read-only**. `build.rs` reads
//! `DECKPILOT_DISTRIBUTION_CHANNEL` (default `github`) and
//! `DECKPILOT_PACKAGE_FORMAT` while compiling and fails the build on any
//! other value; [`DISTRIBUTION`] is then a constant, validated a second time
//! during constant evaluation. The version is `CARGO_PKG_VERSION`, the
//! platform and architecture are `cfg!(target_os)` / `cfg!(target_arch)`.
//! A release build never consults the operating system, the environment,
//! command-line arguments, settings or the database for it (rule R9).
//!
//! Debug builds (`cfg(debug_assertions)`) alone may replace the channel with
//! a development override, so that the update UI of every channel can be
//! tried without building each variant ([`current`]). The code that reads
//! it is not compiled into release builds; `tests/release_build.rs` builds
//! the crate in the release profile and shows that the override's variable
//! name is absent from the binary and that setting it changes nothing.
//!
//! Nothing uses this crate yet. The update providers
//! (`deckpilot_update::provider`) take a [`Distribution`] as their input.

use std::fmt;
use std::str::FromStr;

#[cfg(test)]
mod names;

/// Who delivered this artifact and therefore owns its installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DistributionChannel {
    /// Downloaded from GitHub Releases; updates itself.
    Github,
    /// A dedicated Homebrew build (reserved; phase 1 ships the GitHub build).
    Homebrew,
    /// A dedicated `WinGet` build (reserved; phase 1 ships the GitHub build).
    Winget,
    /// The APT repository's `.deb` variant.
    Apt,
    /// Flathub.
    Flathub,
    /// The Mac App Store.
    MacAppStore,
    /// The Microsoft Store (MSIX).
    MicrosoftStore,
}

/// How a build is updated: a fixed function of its [`DistributionChannel`]
/// ([`DistributionChannel::update_provider`]), never chosen separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpdateProviderKind {
    /// GitHub Releases and the Tauri updater.
    GithubRelease,
    /// A package manager (APT, Homebrew, `WinGet`) updates the installation.
    ExternalPackageManager,
    /// A store (Flathub, Mac App Store, Microsoft Store) updates it.
    StoreManaged,
    /// Updates are not available in this build.
    None,
}

/// The installed package format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageFormat {
    /// macOS `.app` (in a `.dmg`, or the updater's `.app.tar.gz`).
    App,
    Msi,
    Nsis,
    Deb,
    Rpm,
    AppImage,
    Flatpak,
    /// The Mac App Store's `.pkg` of a sandboxed `.app`.
    MasPkg,
    Msix,
}

/// The class of a build's own version. It is a label: the user's update
/// subscription stays `deckpilot_update::channel::Channel`, where alpha,
/// beta and rc are all pre-releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReleaseChannel {
    /// `X.Y.Z`.
    Stable,
    /// `X.Y.Z-beta.N` and `X.Y.Z-rc.N`.
    Beta,
    /// `X.Y.Z-alpha.N`.
    Alpha,
}

/// The compilation target's operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

/// The compilation target's architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Architecture {
    Aarch64,
    X86_64,
}

/// Why a combination of values is not a distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidDistribution {
    /// The channel has no build for the platform (for example `apt` on macOS).
    ChannelNotForPlatform,
    /// The format does not belong to the channel (for example `apt` and `msi`).
    FormatNotForChannel,
    /// The format does not belong to the platform (for example `deb` on Windows).
    FormatNotForPlatform,
    /// The version is not `X.Y.Z` or `X.Y.Z-(alpha|beta|rc).N`.
    InvalidVersion,
}

impl fmt::Display for InvalidDistribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ChannelNotForPlatform => "the distribution channel has no build for this platform",
            Self::FormatNotForChannel => "the package format does not belong to the channel",
            Self::FormatNotForPlatform => "the package format does not belong to the platform",
            Self::InvalidVersion => {
                "the version is not X.Y.Z or X.Y.Z-alpha.N, -beta.N, -rc.N (no leading zeros, N >= 1)"
            }
        })
    }
}

impl std::error::Error for InvalidDistribution {}

/// A name that is not one of an enum's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownName {
    pub kind: &'static str,
    pub name: String,
}

impl fmt::Display for UnknownName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a {}", self.name, self.kind)
    }
}

impl std::error::Error for UnknownName {}

/// `a == b`, usable in constant evaluation.
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `name()`, `from_name()` (a `const fn`), `ALL`, `Display` and `FromStr`
/// for a fieldless enum and its snake-case names.
macro_rules! named {
    ($ty:ident, $kind:literal, { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl $ty {
            /// Every value, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The value's name in `targets.toml` and the build variables.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            /// The value named `name` exactly (case-sensitive).
            pub const fn from_name(name: &str) -> Option<Self> {
                $(if str_eq(name, $name) {
                    return Some(Self::$variant);
                })+
                None
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.name())
            }
        }

        impl FromStr for $ty {
            type Err = UnknownName;

            fn from_str(name: &str) -> Result<Self, UnknownName> {
                Self::from_name(name).ok_or_else(|| UnknownName {
                    kind: $kind,
                    name: name.to_owned(),
                })
            }
        }
    };
}

named!(DistributionChannel, "distribution channel", {
    Github => "github",
    Homebrew => "homebrew",
    Winget => "winget",
    Apt => "apt",
    Flathub => "flathub",
    MacAppStore => "mac_app_store",
    MicrosoftStore => "microsoft_store",
});

named!(UpdateProviderKind, "update provider", {
    GithubRelease => "github_release",
    ExternalPackageManager => "external_package_manager",
    StoreManaged => "store_managed",
    None => "none",
});

named!(PackageFormat, "package format", {
    App => "app",
    Msi => "msi",
    Nsis => "nsis",
    Deb => "deb",
    Rpm => "rpm",
    AppImage => "appimage",
    Flatpak => "flatpak",
    MasPkg => "mas_pkg",
    Msix => "msix",
});

named!(ReleaseChannel, "release channel", {
    Stable => "stable",
    Beta => "beta",
    Alpha => "alpha",
});

named!(Platform, "platform", {
    Macos => "macos",
    Windows => "windows",
    Linux => "linux",
});

named!(Architecture, "architecture", {
    Aarch64 => "aarch64",
    X86_64 => "x86_64",
});

impl DistributionChannel {
    /// The fixed channel → provider table (README section 1).
    pub const fn update_provider(self) -> UpdateProviderKind {
        match self {
            Self::Github => UpdateProviderKind::GithubRelease,
            Self::Homebrew | Self::Winget | Self::Apt => UpdateProviderKind::ExternalPackageManager,
            Self::Flathub | Self::MacAppStore | Self::MicrosoftStore => {
                UpdateProviderKind::StoreManaged
            }
        }
    }

    /// Whether the channel has a build for `platform`.
    pub const fn builds_for(self, platform: Platform) -> bool {
        match self {
            Self::Github => true,
            Self::Homebrew | Self::MacAppStore => matches!(platform, Platform::Macos),
            Self::Winget | Self::MicrosoftStore => matches!(platform, Platform::Windows),
            Self::Apt | Self::Flathub => matches!(platform, Platform::Linux),
        }
    }

    /// Whether `format` is a format this channel installs.
    pub const fn allows_format(self, format: PackageFormat) -> bool {
        use PackageFormat as F;
        match self {
            Self::Github => matches!(
                format,
                F::App | F::Msi | F::Nsis | F::Deb | F::Rpm | F::AppImage
            ),
            Self::Homebrew => matches!(format, F::App),
            Self::Winget => matches!(format, F::Msi | F::Nsis),
            Self::Apt => matches!(format, F::Deb),
            Self::Flathub => matches!(format, F::Flatpak),
            Self::MacAppStore => matches!(format, F::MasPkg),
            Self::MicrosoftStore => matches!(format, F::Msix),
        }
    }

    /// The format a build of this channel for `platform` is known to have
    /// without it being compiled in: the only format the pair allows. `None`
    /// when there are several (a GitHub build for Windows or Linux, whose
    /// format the Tauri bundler embeds per package).
    pub const fn implied_format(self, platform: Platform) -> Option<PackageFormat> {
        let mut found = None;
        let mut i = 0;
        while i < PackageFormat::ALL.len() {
            let format = PackageFormat::ALL[i];
            if self.allows_format(format) && format.platform() as u8 == platform as u8 {
                if found.is_some() {
                    return None;
                }
                found = Some(format);
            }
            i += 1;
        }
        found
    }
}

impl PackageFormat {
    /// The platform the format installs on.
    pub const fn platform(self) -> Platform {
        match self {
            Self::App | Self::MasPkg => Platform::Macos,
            Self::Msi | Self::Nsis | Self::Msix => Platform::Windows,
            Self::Deb | Self::Rpm | Self::AppImage | Self::Flatpak => Platform::Linux,
        }
    }

    /// The suffix of the format's key in a GitHub update manifest
    /// (`{os}-{arch}-{suffix}`); `None` for formats the Tauri updater never
    /// installs.
    pub const fn updater_suffix(self) -> Option<&'static str> {
        match self {
            Self::App => Some("app"),
            Self::Msi => Some("msi"),
            Self::Nsis => Some("nsis"),
            Self::Deb => Some("deb"),
            Self::Rpm => Some("rpm"),
            Self::AppImage => Some("appimage"),
            Self::Flatpak | Self::MasPkg | Self::Msix => None,
        }
    }
}

/// A decimal number without a leading zero that fits a `u64`, as release
/// tags require (`deckpilot_update::channel`).
const fn is_number(bytes: &[u8], start: usize, end: usize) -> bool {
    if start >= end || (bytes[start] == b'0' && end - start > 1) {
        return false;
    }
    let mut value: u64 = 0;
    let mut i = start;
    while i < end {
        let digit = bytes[i];
        if !digit.is_ascii_digit() {
            return false;
        }
        value = match value.checked_mul(10) {
            Some(v) => match v.checked_add((digit - b'0') as u64) {
                Some(v) => v,
                None => return false,
            },
            None => return false,
        };
        i += 1;
    }
    true
}

/// Whether `bytes[start..end]` is `prefix` followed by a number of at least 1.
const fn is_stage(bytes: &[u8], start: usize, end: usize, prefix: &str) -> bool {
    let prefix = prefix.as_bytes();
    if end - start <= prefix.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if bytes[start + i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    let number = start + prefix.len();
    is_number(bytes, number, end) && !(end - number == 1 && bytes[number] == b'0')
}

impl ReleaseChannel {
    /// The release channel of `version`: exactly the versions that form a
    /// release tag (`vX.Y.Z`, `vX.Y.Z-alpha.N`, `-beta.N`, `-rc.N`), with rc
    /// counted as beta. `None` for anything else.
    pub const fn for_version(version: &str) -> Option<Self> {
        let bytes = version.as_bytes();
        let len = bytes.len();
        // The end of the version core: the first `-`, or the end.
        let mut core_end = 0;
        while core_end < len && bytes[core_end] != b'-' {
            core_end += 1;
        }
        // Three dot-separated numbers.
        let mut start = 0;
        let mut parts = 0;
        let mut i = 0;
        while i <= core_end {
            if i == core_end || bytes[i] == b'.' {
                if !is_number(bytes, start, i) {
                    return None;
                }
                parts += 1;
                start = i + 1;
            }
            i += 1;
        }
        if parts != 3 {
            return None;
        }
        if core_end == len {
            return Some(Self::Stable);
        }
        let pre = core_end + 1;
        if is_stage(bytes, pre, len, "alpha.") {
            Some(Self::Alpha)
        } else if is_stage(bytes, pre, len, "beta.") || is_stage(bytes, pre, len, "rc.") {
            Some(Self::Beta)
        } else {
            None
        }
    }
}

impl Platform {
    /// The compilation target's platform.
    pub const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::Macos
    } else if cfg!(target_os = "windows") {
        Self::Windows
    } else if cfg!(target_os = "linux") {
        Self::Linux
    } else {
        // build.rs rejects other targets with a readable message first.
        panic!("deckpilot-distribution: unsupported target OS")
    };

    /// The OS part of a GitHub update manifest key
    /// (`deckpilot_update::manifest::current_target`).
    pub const fn updater_os(self) -> &'static str {
        match self {
            Self::Macos => "darwin",
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }
}

impl Architecture {
    /// The compilation target's architecture.
    pub const CURRENT: Self = if cfg!(target_arch = "aarch64") {
        Self::Aarch64
    } else if cfg!(target_arch = "x86_64") {
        Self::X86_64
    } else {
        panic!("deckpilot-distribution: unsupported target architecture")
    };
}

/// The distribution record of a build (README section 1).
///
/// Fields are private so that a record is always consistent: the provider
/// follows from the channel, the release channel from the version, and the
/// format belongs to both the channel and the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
// The field names are the record's documented names (targets.toml).
#[allow(clippy::struct_field_names)]
pub struct Distribution {
    distribution_channel: DistributionChannel,
    update_provider: UpdateProviderKind,
    package_format: Option<PackageFormat>,
    release_channel: ReleaseChannel,
    application_version: &'static str,
    platform: Platform,
    architecture: Architecture,
    development_override: bool,
}

impl Distribution {
    /// A validated record. `package_format` may be left out when the channel
    /// and platform imply it ([`DistributionChannel::implied_format`]); it
    /// stays `None` for a GitHub build for Windows or Linux, whose format
    /// comes from the bundler (`tauri::utils::platform::bundle_type`).
    ///
    /// Tests construct records with this and pass them to the providers.
    pub const fn new(
        distribution_channel: DistributionChannel,
        package_format: Option<PackageFormat>,
        application_version: &'static str,
        platform: Platform,
        architecture: Architecture,
    ) -> Result<Self, InvalidDistribution> {
        if !distribution_channel.builds_for(platform) {
            return Err(InvalidDistribution::ChannelNotForPlatform);
        }
        let package_format = match package_format {
            Some(format) => {
                if !distribution_channel.allows_format(format) {
                    return Err(InvalidDistribution::FormatNotForChannel);
                }
                if format.platform() as u8 != platform as u8 {
                    return Err(InvalidDistribution::FormatNotForPlatform);
                }
                Some(format)
            }
            None => distribution_channel.implied_format(platform),
        };
        let Some(release_channel) = ReleaseChannel::for_version(application_version) else {
            return Err(InvalidDistribution::InvalidVersion);
        };
        Ok(Self {
            distribution_channel,
            update_provider: distribution_channel.update_provider(),
            package_format,
            release_channel,
            application_version,
            platform,
            architecture,
            development_override: false,
        })
    }

    pub const fn distribution_channel(&self) -> DistributionChannel {
        self.distribution_channel
    }

    pub const fn update_provider(&self) -> UpdateProviderKind {
        self.update_provider
    }

    /// The compiled-in or implied package format; `None` when only the
    /// bundler knows it (a GitHub build for Windows or Linux).
    pub const fn package_format(&self) -> Option<PackageFormat> {
        self.package_format
    }

    pub const fn release_channel(&self) -> ReleaseChannel {
        self.release_channel
    }

    pub const fn application_version(&self) -> &'static str {
        self.application_version
    }

    pub const fn platform(&self) -> Platform {
        self.platform
    }

    pub const fn architecture(&self) -> Architecture {
        self.architecture
    }

    /// Whether a debug build's development override replaced the compiled
    /// channel; the UI marks such a record "Development override". Always
    /// `false` in release builds.
    pub const fn is_development_override(&self) -> bool {
        self.development_override
    }
}

const fn compiled_channel() -> DistributionChannel {
    match DistributionChannel::from_name(env!("DECKPILOT_BUILD_CHANNEL")) {
        Some(channel) => channel,
        None => panic!("deckpilot-distribution: invalid DECKPILOT_DISTRIBUTION_CHANNEL"),
    }
}

const fn compiled_format() -> Option<PackageFormat> {
    let name = env!("DECKPILOT_BUILD_PACKAGE_FORMAT");
    if name.is_empty() {
        return None;
    }
    match PackageFormat::from_name(name) {
        Some(format) => Some(format),
        None => panic!("deckpilot-distribution: invalid DECKPILOT_PACKAGE_FORMAT"),
    }
}

/// The record compiled into this build. An invalid combination fails
/// constant evaluation, so it fails the build.
pub const DISTRIBUTION: Distribution = match Distribution::new(
    compiled_channel(),
    compiled_format(),
    env!("CARGO_PKG_VERSION"),
    Platform::CURRENT,
    Architecture::CURRENT,
) {
    Ok(distribution) => distribution,
    Err(InvalidDistribution::ChannelNotForPlatform) => {
        panic!("deckpilot-distribution: the channel has no build for the target platform")
    }
    Err(InvalidDistribution::FormatNotForChannel) => {
        panic!("deckpilot-distribution: the package format does not belong to the channel")
    }
    Err(InvalidDistribution::FormatNotForPlatform) => {
        panic!("deckpilot-distribution: the package format does not belong to the target")
    }
    Err(InvalidDistribution::InvalidVersion) => {
        panic!("deckpilot-distribution: CARGO_PKG_VERSION is not a release version")
    }
};

/// The record of the running build.
///
/// Release builds: always [`DISTRIBUTION`]. Debug builds: the same, unless
/// the development override is set when this is first called (see
/// `development` below); the result never changes afterwards.
pub fn current() -> &'static Distribution {
    #[cfg(debug_assertions)]
    {
        development::current()
    }
    #[cfg(not(debug_assertions))]
    {
        &DISTRIBUTION
    }
}

/// The development override. Compiled only with `debug_assertions`, so
/// neither the code nor the variable's name is in a release build
/// (`tests/release_build.rs`).
#[cfg(debug_assertions)]
mod development {
    use super::{DISTRIBUTION, Distribution, DistributionChannel, PackageFormat};
    use std::sync::OnceLock;

    /// `channel` or `channel:format`, for example `apt` or
    /// `microsoft_store:msix`.
    pub const VARIABLE: &str = "DECKPILOT_DISTRIBUTION_OVERRIDE";

    pub fn current() -> &'static Distribution {
        static CURRENT: OnceLock<Distribution> = OnceLock::new();
        CURRENT.get_or_init(|| match std::env::var(VARIABLE) {
            Ok(value) if !value.is_empty() => apply(&DISTRIBUTION, &value)
                .unwrap_or_else(|message| panic!("{VARIABLE}={value:?}: {message}")),
            Ok(_) | Err(std::env::VarError::NotPresent) => DISTRIBUTION,
            Err(std::env::VarError::NotUnicode(_)) => panic!("{VARIABLE} is not valid UTF-8"),
        })
    }

    /// `base` with the channel (and format) of `value`. The platform,
    /// architecture and version stay the compiled ones; the channel need not
    /// build for the platform, so that, say, the APT UI can be tried on
    /// macOS.
    pub fn apply(base: &Distribution, value: &str) -> Result<Distribution, String> {
        let (channel, format) = match value.split_once(':') {
            Some((channel, format)) => (channel, Some(format)),
            None => (value, None),
        };
        let channel: DistributionChannel = channel.parse().map_err(|e| format!("{e}"))?;
        let format = match format {
            Some(format) => {
                let format: PackageFormat = format.parse().map_err(|e| format!("{e}"))?;
                if !channel.allows_format(format) {
                    return Err(format!("{format} is not a format of {channel}"));
                }
                Some(format)
            }
            None => channel
                .implied_format(base.platform)
                .or_else(|| channel.implied_format(only_platform(channel)?)),
        };
        Ok(Distribution {
            distribution_channel: channel,
            update_provider: channel.update_provider(),
            package_format: format,
            development_override: true,
            ..*base
        })
    }

    fn only_platform(channel: DistributionChannel) -> Option<super::Platform> {
        let mut platforms = super::Platform::ALL
            .iter()
            .copied()
            .filter(|p| channel.builds_for(*p));
        let first = platforms.next()?;
        platforms.next().is_none().then_some(first)
    }
}

#[cfg(debug_assertions)]
pub use development::VARIABLE as DEVELOPMENT_OVERRIDE_VARIABLE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        fn check<T: Copy + PartialEq + fmt::Debug + FromStr>(
            all: &[T],
            name: fn(T) -> &'static str,
        ) {
            for &value in all {
                assert_eq!(name(value).parse::<T>().ok(), Some(value));
                assert!(name(value).to_uppercase().parse::<T>().is_err());
            }
            let mut names: Vec<_> = all.iter().map(|v| name(*v)).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), all.len());
        }
        check(DistributionChannel::ALL, DistributionChannel::name);
        check(UpdateProviderKind::ALL, UpdateProviderKind::name);
        check(PackageFormat::ALL, PackageFormat::name);
        check(ReleaseChannel::ALL, ReleaseChannel::name);
        check(Platform::ALL, Platform::name);
        check(Architecture::ALL, Architecture::name);
        assert_eq!(DistributionChannel::ALL.len(), 7);
        assert!("unknown".parse::<DistributionChannel>().is_err());
        assert!("".parse::<DistributionChannel>().is_err());
        assert!(" github".parse::<DistributionChannel>().is_err());
    }

    /// `build.rs` validates with `names.rs`; it must accept exactly what the
    /// enums define.
    #[test]
    fn build_script_tables_match_the_enums() {
        let channels: Vec<&str> = names::CHANNELS.iter().map(|(name, ..)| *name).collect();
        let expected: Vec<&str> = DistributionChannel::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(channels, expected);
        for &(name, platforms, formats) in names::CHANNELS {
            let channel = DistributionChannel::from_name(name).unwrap();
            for &platform in Platform::ALL {
                assert_eq!(
                    platforms.contains(&platform.name()),
                    channel.builds_for(platform),
                    "{name} on {platform}"
                );
            }
            for &format in PackageFormat::ALL {
                assert_eq!(
                    formats.contains(&format.name()),
                    channel.allows_format(format),
                    "{name} with {format}"
                );
            }
        }
        let formats: Vec<(&str, &str)> = PackageFormat::ALL
            .iter()
            .map(|f| (f.name(), f.platform().name()))
            .collect();
        assert_eq!(names::FORMATS, formats.as_slice());
        let platforms: Vec<&str> = names::PLATFORMS.iter().map(|(_, p)| *p).collect();
        let expected: Vec<&str> = Platform::ALL.iter().map(|p| p.name()).collect();
        assert_eq!(platforms, expected);
        let arches: Vec<&str> = Architecture::ALL.iter().map(|a| a.name()).collect();
        assert_eq!(names::ARCHITECTURES, arches.as_slice());
    }

    #[test]
    fn providers_follow_the_channel() {
        use DistributionChannel as C;
        use UpdateProviderKind as P;
        let table = [
            (C::Github, P::GithubRelease),
            (C::Homebrew, P::ExternalPackageManager),
            (C::Winget, P::ExternalPackageManager),
            (C::Apt, P::ExternalPackageManager),
            (C::Flathub, P::StoreManaged),
            (C::MacAppStore, P::StoreManaged),
            (C::MicrosoftStore, P::StoreManaged),
        ];
        assert_eq!(table.len(), C::ALL.len());
        for (channel, provider) in table {
            assert_eq!(channel.update_provider(), provider, "{channel}");
        }
    }

    #[test]
    fn release_channels_follow_the_version() {
        let rc = ReleaseChannel::for_version;
        assert_eq!(rc("0.6.0"), Some(ReleaseChannel::Stable));
        assert_eq!(rc("10.0.12"), Some(ReleaseChannel::Stable));
        assert_eq!(rc("0.0.0"), Some(ReleaseChannel::Stable));
        assert_eq!(rc("0.6.0-alpha.3"), Some(ReleaseChannel::Alpha));
        assert_eq!(rc("0.6.0-beta.2"), Some(ReleaseChannel::Beta));
        assert_eq!(rc("1.0.0-rc.10"), Some(ReleaseChannel::Beta));
        for invalid in [
            "",
            "0.6",
            "0.6.0.1",
            "00.6.0",
            "0.06.0",
            "0.6.0-",
            "0.6.0-alpha",
            "0.6.0-alpha.",
            "0.6.0-alpha.0",
            "0.6.0-alpha.01",
            "0.6.0-Alpha.1",
            "0.6.0-alpha1",
            "0.6.0-alpha.1.2",
            "0.6.0-dev.1",
            "0.6.0+build.1",
            "0.6.0-rc.1+build",
            "0.6.0 ",
            "v0.6.0",
            "99999999999999999999.0.0",
            "0.5.0-1",
            "0..0",
            ".0.0",
            "0.0.",
        ] {
            assert_eq!(rc(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn records_are_validated() {
        use DistributionChannel as C;
        use PackageFormat as F;
        let new = |c, f, p| Distribution::new(c, f, "0.6.0-alpha.3", p, Architecture::X86_64);
        let apt = new(C::Apt, None, Platform::Linux).unwrap();
        assert_eq!(apt.package_format(), Some(F::Deb));
        assert_eq!(
            apt.update_provider(),
            UpdateProviderKind::ExternalPackageManager
        );
        assert_eq!(apt.release_channel(), ReleaseChannel::Alpha);
        assert!(!apt.is_development_override());
        assert_eq!(
            new(C::Github, None, Platform::Macos)
                .unwrap()
                .package_format(),
            Some(F::App)
        );
        assert_eq!(
            new(C::Github, None, Platform::Linux)
                .unwrap()
                .package_format(),
            None
        );
        assert_eq!(
            new(C::Github, None, Platform::Windows)
                .unwrap()
                .package_format(),
            None
        );
        assert_eq!(
            new(C::Winget, None, Platform::Windows)
                .unwrap()
                .package_format(),
            None
        );
        assert_eq!(
            new(C::Apt, None, Platform::Macos),
            Err(InvalidDistribution::ChannelNotForPlatform)
        );
        assert_eq!(
            new(C::Apt, Some(F::Rpm), Platform::Linux),
            Err(InvalidDistribution::FormatNotForChannel)
        );
        assert_eq!(
            new(C::Github, Some(F::Deb), Platform::Windows),
            Err(InvalidDistribution::FormatNotForPlatform)
        );
        assert_eq!(
            Distribution::new(
                C::Github,
                None,
                "0.6.0-dev.1",
                Platform::Linux,
                Architecture::X86_64
            ),
            Err(InvalidDistribution::InvalidVersion)
        );
    }

    #[test]
    fn the_compiled_record_describes_this_build() {
        // Tests are built without DECKPILOT_DISTRIBUTION_CHANNEL: the
        // default is a GitHub build of the current target.
        if option_env!("DECKPILOT_DISTRIBUTION_CHANNEL").is_none() {
            assert_eq!(
                DISTRIBUTION.distribution_channel(),
                DistributionChannel::Github
            );
            assert_eq!(
                DISTRIBUTION.update_provider(),
                UpdateProviderKind::GithubRelease
            );
        }
        assert_eq!(
            DISTRIBUTION.application_version(),
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(DISTRIBUTION.platform().name(), std::env::consts::OS);
        assert_eq!(DISTRIBUTION.architecture().name(), std::env::consts::ARCH);
        assert!(!DISTRIBUTION.is_development_override());
        assert_eq!(
            Some(DISTRIBUTION.release_channel()),
            ReleaseChannel::for_version(env!("CARGO_PKG_VERSION"))
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_development_override_replaces_only_the_channel() {
        let base = Distribution::new(
            DistributionChannel::Github,
            None,
            "0.6.0",
            Platform::Macos,
            Architecture::Aarch64,
        )
        .unwrap();
        let apt = development::apply(&base, "apt").unwrap();
        assert_eq!(apt.distribution_channel(), DistributionChannel::Apt);
        assert_eq!(
            apt.update_provider(),
            UpdateProviderKind::ExternalPackageManager
        );
        assert_eq!(apt.package_format(), Some(PackageFormat::Deb));
        assert_eq!(apt.platform(), Platform::Macos);
        assert_eq!(apt.application_version(), "0.6.0");
        assert!(apt.is_development_override());
        let store = development::apply(&base, "microsoft_store:msix").unwrap();
        assert_eq!(store.update_provider(), UpdateProviderKind::StoreManaged);
        assert_eq!(store.package_format(), Some(PackageFormat::Msix));
        assert_eq!(
            development::apply(&base, "winget")
                .unwrap()
                .package_format(),
            None
        );
        for invalid in ["unknown", "Apt", "apt:msi", "apt:", ":deb"] {
            assert!(development::apply(&base, invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            DEVELOPMENT_OVERRIDE_VARIABLE,
            "DECKPILOT_DISTRIBUTION_OVERRIDE"
        );
    }
}
