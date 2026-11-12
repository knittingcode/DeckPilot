//! Release tags and update channels.
//!
//! Only these tags make releases (`docs/releasing.md`):
//!
//! | Tag | Kind | Channel | Manifest |
//! | --- | ---- | ------- | -------- |
//! | `vX.Y.Z` | stable | stable | `latest.json` |
//! | `vX.Y.Z-alpha.N`, `-beta.N`, `-rc.N` | pre-release | prerelease | `prerelease.json` |
//!
//! `X`, `Y`, `Z` and `N` are decimal numbers without leading zeros, `N` at
//! least 1 — exactly
//! `^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$`,
//! the expression of the release workflow's tag gate. Anything else (build
//! metadata, other identifiers, uppercase, a missing `v`, and the former
//! `vX.Y.Z-N` test tags) is not a release tag.
//!
//! The channels are isolated by the manifest's *name*: the stable channel
//! reads `latest.json` of GitHub's "latest" release, and only stable
//! releases carry a `latest.json`, so even a pre-release that was marked
//! "latest" by mistake offers nothing to stable installations. The
//! prerelease channel offers the newest published pre-release **or stable**
//! release, so its users move on to the stable release once it is newer.

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Manifest of a stable release; the stable channel's endpoint.
pub const STABLE_MANIFEST: &str = "latest.json";
/// Manifest of a pre-release.
pub const PRERELEASE_MANIFEST: &str = "prerelease.json";

/// Which releases an installation is offered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Channel {
    /// Stable releases only (the default).
    #[default]
    Stable,
    /// Alpha, beta and release-candidate builds, and stable releases.
    Prerelease,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Prerelease => "prerelease",
        }
    }

    /// Whether this channel installs `offered` over `current`: always a
    /// newer version; on the stable channel never a pre-release.
    pub fn accepts(self, current: &Version, offered: &Version) -> bool {
        offered > current && (self == Self::Prerelease || offered.pre.is_empty())
    }
}

/// What a release tag builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseKind {
    /// `vX.Y.Z`: published as the latest release, stable channel.
    Stable,
    /// `vX.Y.Z-alpha.N`, `-beta.N`, `-rc.N`: published as a GitHub
    /// pre-release, prerelease channel.
    Prerelease,
}

impl ReleaseKind {
    pub fn name(self) -> &'static str {
        self.channel().name()
    }

    /// The channel the release is offered on once published.
    pub fn channel(self) -> Channel {
        match self {
            Self::Stable => Channel::Stable,
            Self::Prerelease => Channel::Prerelease,
        }
    }

    /// The name of the release's update manifest.
    pub fn manifest(self) -> &'static str {
        match self {
            Self::Stable => STABLE_MANIFEST,
            Self::Prerelease => PRERELEASE_MANIFEST,
        }
    }
}

/// A valid release tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTag {
    pub tag: String,
    pub version: Version,
    pub kind: ReleaseKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "`{0}` is not a release tag: use vX.Y.Z, vX.Y.Z-alpha.N, vX.Y.Z-beta.N or vX.Y.Z-rc.N \
     (numbers without leading zeros, N >= 1)"
)]
pub struct InvalidTag(pub String);

/// A decimal number without a leading zero (`0` itself is allowed).
fn number(text: &str) -> Option<u64> {
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    canonical.then(|| text.parse().ok()).flatten()
}

impl ReleaseTag {
    pub fn parse(tag: &str) -> Result<Self, InvalidTag> {
        let invalid = || InvalidTag(tag.to_owned());
        let rest = tag.strip_prefix('v').ok_or_else(invalid)?;
        let (core, pre) = match rest.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (rest, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 || parts.iter().any(|part| number(part).is_none()) {
            return Err(invalid());
        }
        let kind = match pre.map(|pre| pre.split_once('.')) {
            None => ReleaseKind::Stable,
            Some(Some(("alpha" | "beta" | "rc", n))) if number(n).is_some_and(|n| n >= 1) => {
                ReleaseKind::Prerelease
            }
            Some(_) => return Err(invalid()),
        };
        let version = Version::parse(rest).map_err(|_| invalid())?;
        Ok(Self {
            tag: tag.to_owned(),
            version,
            kind,
        })
    }

    /// The tag of `version` (which must form a valid tag).
    pub fn for_version(version: &str) -> Result<Self, InvalidTag> {
        Self::parse(&format!("v{version}"))
    }

    /// The MSI (Windows Installer) product version, `major.minor.patch.build` with
    /// numbers only: Windows Installer refuses `0.6.0-alpha.1`. The build
    /// field orders the pre-releases of one version — alpha.N is N,
    /// beta.N 20000 + N, rc.N 40000 + N — and the stable release has none.
    /// Windows Installer compares only the first three fields (DeckPilot's
    /// installer allows same-version upgrades), so the stable release
    /// installs over its pre-releases. Fails for versions an MSI cannot
    /// carry (major or minor above 255, patch above 65535, N above 19999).
    pub fn msi_version(&self) -> Result<String, String> {
        let v = &self.version;
        if v.major > 255 || v.minor > 255 || v.patch > 65535 {
            return Err(format!(
                "{}: an MSI version allows at most 255.255.65535",
                self.tag
            ));
        }
        let core = format!("{}.{}.{}", v.major, v.minor, v.patch);
        let Some((stage, n)) = v.pre.as_str().split_once('.') else {
            return Ok(core);
        };
        let n: u64 = n
            .parse()
            .map_err(|_| format!("{}: not a release tag", self.tag))?;
        if n > 19_999 {
            return Err(format!(
                "{}: at most {stage}.19999 fits an MSI version",
                self.tag
            ));
        }
        let offset = match stage {
            "alpha" => 0,
            "beta" => 20_000,
            _ => 40_000,
        };
        Ok(format!("{core}.{}", offset + n))
    }

    /// The URL of this release's manifest: `latest.json` of a stable
    /// release, `prerelease.json` of a pre-release.
    pub fn manifest_url(&self) -> String {
        self.manifest_url_at(crate::DOWNLOAD_PREFIX)
    }

    /// [`Self::manifest_url`] under another download prefix (tests).
    pub fn manifest_url_at(&self, downloads: &str) -> String {
        format!("{downloads}{}/{}", self.tag, self.kind.manifest())
    }

    /// `key=value` lines for the release workflow (`GITHUB_OUTPUT`).
    pub fn describe(&self) -> Result<String, String> {
        Ok(format!(
            "tag={}\nversion={}\nchannel={}\nmanifest={}\nprerelease={}\nmsi_version={}\n",
            self.tag,
            self.version,
            self.kind.channel().name(),
            self.kind.manifest(),
            self.kind == ReleaseKind::Prerelease,
            self.msi_version()?,
        ))
    }
}

/// A release as GitHub's release list describes it
/// (`GET /repos/{owner}/{repo}/releases`); other fields are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ListedRelease {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
}

/// The release the prerelease channel offers: the newest published release
/// — pre-release or stable — that the channel accepts over `current`.
/// Drafts, tags that are not release tags, and releases whose GitHub
/// pre-release flag disagrees with their tag are never offered. `None` when
/// there is nothing newer.
///
/// The list comes from the network and is not trusted beyond this choice:
/// the manifest URL is built from the validated tag
/// ([`ReleaseTag::manifest_url`]), and the updater still verifies the
/// package's signature and its signed version.
pub fn newest_release(
    releases: &[ListedRelease],
    channel: Channel,
    current: &Version,
) -> Option<ReleaseTag> {
    releases
        .iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let tag = ReleaseTag::parse(&release.tag_name).ok()?;
            (release.prerelease == (tag.kind == ReleaseKind::Prerelease)).then_some(tag)
        })
        .filter(|tag| channel.accepts(current, &tag.version))
        .max_by(|a, b| a.version.cmp(&b.version))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(tag: &str) -> Option<ReleaseKind> {
        ReleaseTag::parse(tag).ok().map(|t| t.kind)
    }

    #[test]
    fn classifies_release_tags() {
        assert_eq!(kind("v0.6.0"), Some(ReleaseKind::Stable));
        assert_eq!(kind("v10.0.12"), Some(ReleaseKind::Stable));
        assert_eq!(kind("v0.0.0"), Some(ReleaseKind::Stable));
        for tag in ["v0.6.0-alpha.1", "v0.6.0-beta.2", "v1.0.0-rc.10"] {
            assert_eq!(kind(tag), Some(ReleaseKind::Prerelease), "{tag}");
        }
        let tag = ReleaseTag::parse("v0.6.0-rc.1").unwrap();
        assert_eq!(tag.version, Version::parse("0.6.0-rc.1").unwrap());
    }

    #[test]
    fn rejects_malformed_tags() {
        for tag in [
            "",
            "v",
            "0.6.0",
            "V0.6.0",
            "testv0.6.0",
            "v0.6",
            "v0.6.0.1",
            "v00.6.0",
            "v01.0.0",
            "v0.06.0",
            "v0.6.0-",
            "v0.6.0-alpha",
            "v0.6.0-alpha.0",
            "v1.0.0-beta.0",
            "v0.6.0-alpha.01",
            "v0.6.0-Alpha.1",
            "v0.6.0-alpha1",
            "v0.6.0-alpha.1.2",
            "v0.6.0-bettta.1",
            "v0.6.0-dev.1",
            "v1.0.0-preview.1",
            "v1.0.0-extra",
            "v0.6.0+build.1",
            "v0.6.0-rc.1+build",
            "v0.6.0 ",
            " v0.6.0",
            "v0.6.0\n",
            "v-1.0.0",
            "vx.y.z",
            "v99999999999999999999.0.0",
            "release-0.6.0",
            // The former test-tag convention no longer makes releases.
            "v0.5.0-1",
            "v0.5.0-2",
            "v0.6.0-0",
        ] {
            assert!(ReleaseTag::parse(tag).is_err(), "{tag:?}");
        }
    }

    #[test]
    fn kinds_map_to_channels_and_manifests() {
        assert_eq!(ReleaseKind::Stable.channel(), Channel::Stable);
        assert_eq!(ReleaseKind::Stable.manifest(), "latest.json");
        assert_eq!(ReleaseKind::Prerelease.channel(), Channel::Prerelease);
        assert_eq!(ReleaseKind::Prerelease.manifest(), "prerelease.json");
        assert_eq!(
            ReleaseTag::parse("v0.6.0-alpha.1")
                .unwrap()
                .describe()
                .unwrap(),
            "tag=v0.6.0-alpha.1\nversion=0.6.0-alpha.1\nchannel=prerelease\n\
             manifest=prerelease.json\nprerelease=true\nmsi_version=0.6.0.1\n"
        );
        assert_eq!(
            ReleaseTag::parse("v0.6.0").unwrap().describe().unwrap(),
            "tag=v0.6.0\nversion=0.6.0\nchannel=stable\nmanifest=latest.json\n\
             prerelease=false\nmsi_version=0.6.0\n"
        );
        assert_eq!(
            ReleaseTag::for_version("0.6.0").unwrap().kind,
            ReleaseKind::Stable
        );
    }

    #[test]
    fn msi_versions_are_numeric_and_ordered() {
        let msi = |tag: &str| ReleaseTag::parse(tag).unwrap().msi_version();
        assert_eq!(msi("v0.6.0").unwrap(), "0.6.0");
        assert_eq!(msi("v0.6.0-alpha.1").unwrap(), "0.6.0.1");
        assert_eq!(msi("v0.6.0-beta.2").unwrap(), "0.6.0.20002");
        assert_eq!(msi("v0.6.0-rc.19999").unwrap(), "0.6.0.59999");
        // Ordered like the versions within 0.6.0.
        let order = [
            "v0.6.0-alpha.1",
            "v0.6.0-alpha.19999",
            "v0.6.0-beta.1",
            "v0.6.0-rc.1",
        ];
        let builds: Vec<u32> = order
            .iter()
            .map(|t| msi(t).unwrap().rsplit('.').next().unwrap().parse().unwrap())
            .collect();
        assert!(builds.windows(2).all(|w| w[0] < w[1]), "{builds:?}");
        for tag in ["v256.0.0", "v0.256.0", "v0.0.65536", "v0.6.0-rc.20000"] {
            assert!(msi(tag).is_err(), "{tag}");
            assert!(ReleaseTag::parse(tag).unwrap().describe().is_err(), "{tag}");
        }
    }

    fn listed(tag: &str, draft: bool, prerelease: bool) -> ListedRelease {
        ListedRelease {
            tag_name: tag.into(),
            draft,
            prerelease,
        }
    }

    #[test]
    fn the_prerelease_channel_offers_the_newest_release_of_either_kind() {
        let v = |s: &str| Version::parse(s).unwrap();
        let releases = [
            listed("v0.7.0-alpha.1", false, true),
            listed("v0.6.1", false, false),
            listed("v0.7.0-beta.1", true, true), // draft
            listed("v0.5.0-2", true, true),      // old test build
            listed("v0.9.0", false, true),       // flag disagrees with the tag
            listed("v0.9.0-rc.1", false, false), // flag disagrees with the tag
            listed("v1.0.0-preview.1", false, true),
            listed("nightly", false, true),
        ];
        let base = crate::DOWNLOAD_PREFIX;
        let newest = |list: &[ListedRelease], channel, current: &str| {
            newest_release(list, channel, &v(current)).map(|tag| tag.manifest_url())
        };
        assert_eq!(
            newest(&releases, Channel::Prerelease, "0.6.0"),
            Some(format!("{base}v0.7.0-alpha.1/prerelease.json"))
        );
        // Stable releases come from the stable manifest of their own tag:
        // a pre-release user moves on to the stable release once it is newer.
        let with_stable = [releases.as_slice(), &[listed("v0.7.0", false, false)]].concat();
        assert_eq!(
            newest(&with_stable, Channel::Prerelease, "0.7.0-alpha.1"),
            Some(format!("{base}v0.7.0/latest.json"))
        );
        // Never the same or an older version.
        assert_eq!(newest(&with_stable, Channel::Prerelease, "0.7.0"), None);
        // The stable channel never takes a pre-release from the list.
        assert_eq!(
            newest(&releases, Channel::Stable, "0.6.0"),
            Some(format!("{base}v0.6.1/latest.json"))
        );
        assert_eq!(newest(&[], Channel::Prerelease, "0.6.0"), None);
        assert_eq!(
            ReleaseTag::parse("v1.0.0-rc.1")
                .unwrap()
                .manifest_url_at("http://127.0.0.1:1/"),
            "http://127.0.0.1:1/v1.0.0-rc.1/prerelease.json"
        );
        let parsed: Vec<ListedRelease> = serde_json::from_str(
            r#"[{"tag_name":"v0.7.0-rc.1","draft":false,"prerelease":true,"assets":[]}]"#,
        )
        .unwrap();
        assert_eq!(parsed[0], listed("v0.7.0-rc.1", false, true));
    }

    #[test]
    fn stable_installations_never_accept_pre_releases() {
        let v = |s: &str| Version::parse(s).unwrap();
        let stable = Channel::Stable;
        assert!(stable.accepts(&v("0.6.0"), &v("0.6.1")));
        assert!(!stable.accepts(&v("0.6.0"), &v("0.7.0-alpha.1")));
        assert!(!stable.accepts(&v("0.6.0"), &v("0.7.0-rc.3")));
        assert!(!stable.accepts(&v("0.6.0"), &v("0.6.0")));
        assert!(!stable.accepts(&v("0.6.1"), &v("0.6.0")));
        // A pre-release installation switched back to stable waits until a
        // stable release is newer than what it runs: no downgrade.
        assert!(!stable.accepts(&v("0.7.0-beta.1"), &v("0.6.5")));
        assert!(stable.accepts(&v("0.7.0-beta.1"), &v("0.7.0")));

        let pre = Channel::Prerelease;
        assert!(pre.accepts(&v("0.6.0"), &v("0.7.0-alpha.1")));
        assert!(pre.accepts(&v("0.7.0-alpha.1"), &v("0.7.0-alpha.2")));
        assert!(pre.accepts(&v("0.7.0-alpha.2"), &v("0.7.0-beta.1")));
        assert!(pre.accepts(&v("0.7.0-rc.1"), &v("0.7.0")));
        assert!(!pre.accepts(&v("0.7.0-beta.1"), &v("0.7.0-alpha.9")));
        assert!(!pre.accepts(&v("0.7.0"), &v("0.7.0-rc.1")));
        assert_eq!(Channel::default(), Channel::Stable);
    }
}
