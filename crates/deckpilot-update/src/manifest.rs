//! The update manifest (`latest.json`) produced by the Tauri bundler.

use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::REPOSITORY;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformArtifact {
    /// Base64-encoded minisign signature of the artifact.
    pub signature: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub pub_date: Option<String>,
    /// Keyed by target, e.g. `darwin-aarch64`, `linux-x86_64`, `windows-x86_64`.
    pub platforms: BTreeMap<String, PlatformArtifact>,
}

/// An update that applies to this installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub url: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ManifestError {
    #[error("the manifest is not valid JSON: {0}")]
    Json(String),
    #[error("`{0}` is not a valid version")]
    InvalidVersion(String),
    #[error("version {0} is a pre-release, which the stable channel does not offer")]
    PreRelease(String),
    #[error("the release has no artifact for {0}")]
    MissingPlatform(String),
    #[error("artifact URL `{0}` does not point to a DeckPilot release download")]
    UntrustedUrl(String),
    #[error("the artifact signature is missing or malformed")]
    InvalidSignature,
}

impl Manifest {
    pub fn parse(json: &str) -> Result<Self, ManifestError> {
        serde_json::from_str(json).map_err(|e| ManifestError::Json(e.to_string()))
    }

    /// Validates the manifest and returns the update for `target` if it is
    /// newer than `current`. Returns `Ok(None)` when already up to date.
    pub fn update_for(
        &self,
        current: &Version,
        target: &str,
    ) -> Result<Option<AvailableUpdate>, ManifestError> {
        let version = parse_version(&self.version)?;
        if !version.pre.is_empty() {
            return Err(ManifestError::PreRelease(self.version.clone()));
        }
        // Validate every artifact so a broken manifest is noticed early.
        self.validate_artifacts()?;
        if version <= *current {
            return Ok(None);
        }
        let artifact = self
            .platforms
            .get(target)
            .ok_or_else(|| ManifestError::MissingPlatform(target.to_owned()))?;
        Ok(Some(AvailableUpdate {
            version: version.to_string(),
            notes: self.notes.clone(),
            pub_date: self.pub_date.clone(),
            url: artifact.url.clone(),
            signature: artifact.signature.clone(),
        }))
    }
}

impl Manifest {
    /// Checks every artifact: a DeckPilot release download URL and a
    /// well-formed signature. Independent of the channel: release
    /// verification also checks pre-release (test) builds, which the stable
    /// channel never offers.
    pub fn validate_artifacts(&self) -> Result<(), ManifestError> {
        for artifact in self.platforms.values() {
            validate_artifact(artifact)?;
        }
        Ok(())
    }
}

/// Accepts versions with or without a leading `v` (release tags use one).
pub fn parse_version(value: &str) -> Result<Version, ManifestError> {
    Version::parse(value.trim().trim_start_matches('v'))
        .map_err(|_| ManifestError::InvalidVersion(value.to_owned()))
}

/// The path after `https://github.com/<owner>/<repo>/releases/download/`
/// when `url` is a release download of [`REPOSITORY`], or nothing.
///
/// Scheme, host and the path segments are compared exactly; only the owner
/// and repository name are compared ASCII case-insensitively, as GitHub
/// resolves them (the repository was renamed from `knittingcode/DeckPilot`
/// to `knittingcode/deckpilot`; both name the same repository). This widens
/// nothing beyond that one repository. Builds up to 0.6.0-alpha.3 compared
/// the whole [`crate::DOWNLOAD_PREFIX`] case-sensitively, so the release
/// pipeline still writes exactly that prefix
/// (`tools/release-pipeline/common.sh`).
fn release_download_path(url: &str) -> Option<&str> {
    let (want_owner, want_repo) = REPOSITORY.split_once('/')?;
    let rest = url.strip_prefix("https://github.com/")?;
    let (owner, rest) = rest.split_once('/')?;
    let (repo, rest) = rest.split_once('/')?;
    let path = rest.strip_prefix("releases/download/")?;
    (owner.eq_ignore_ascii_case(want_owner) && repo.eq_ignore_ascii_case(want_repo)).then_some(path)
}

/// Whether `url` is a release download of [`REPOSITORY`] that every client
/// reads the same way: no query, fragment or backslash; every segment after
/// the prefix non-empty, not `.` or `..`, and without a percent-escape of
/// `.`, `/`, `\` or `%` in either case (`%2e%2e` is `..` to a URL parser,
/// `%252e` is `%2e` once decoded); and unchanged when a URL parser
/// normalizes it.
fn is_release_download(url: &str) -> bool {
    let Some(path) = release_download_path(url) else {
        return false;
    };
    !url.contains(['?', '#', '\\'])
        && path.split('/').all(plain_segment)
        && url::Url::parse(url).is_ok_and(|parsed| parsed.as_str() == url)
}

/// A path segment that names itself: not empty, not a dot segment, and no
/// percent-escape that decodes to `.`, `/`, `\` or `%` (or is malformed).
/// Other escapes (`%2B` for `+`) are kept as they are.
fn plain_segment(segment: &str) -> bool {
    if segment.is_empty() || segment == "." || segment == ".." {
        return false;
    }
    let mut escapes = segment.split('%');
    escapes.next();
    escapes.all(|rest| {
        rest.get(..2)
            .filter(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .is_some_and(|byte| !matches!(byte, b'.' | b'/' | b'\\' | b'%'))
    })
}

fn validate_artifact(artifact: &PlatformArtifact) -> Result<(), ManifestError> {
    let url = &artifact.url;
    if !is_release_download(url) {
        return Err(ManifestError::UntrustedUrl(url.clone()));
    }
    crate::signature::decode_signature(&artifact.signature)
        .map_err(|_| ManifestError::InvalidSignature)?;
    Ok(())
}

/// Updater target of the running build, matching the manifest keys.
pub fn current_target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DOWNLOAD_PREFIX;

    const SIGNATURE: &str = include_str!("../tests/fixtures/artifact.bin.sig");

    fn manifest(version: &str, url: &str) -> Manifest {
        let mut platforms = BTreeMap::new();
        for target in ["darwin-aarch64", "linux-x86_64", "windows-x86_64"] {
            platforms.insert(
                target.to_owned(),
                PlatformArtifact {
                    signature: SIGNATURE.trim().to_owned(),
                    url: url.to_owned(),
                },
            );
        }
        Manifest {
            version: version.into(),
            notes: Some("Fixes".into()),
            pub_date: None,
            platforms,
        }
    }

    fn url(name: &str) -> String {
        format!("{DOWNLOAD_PREFIX}v0.3.0/{name}")
    }

    #[test]
    fn offers_only_newer_stable_versions() {
        let current = Version::parse("0.3.0").unwrap();
        let newer = manifest("v0.3.1", &url("DeckPilot.app.tar.gz"));
        let update = newer
            .update_for(&current, "darwin-aarch64")
            .unwrap()
            .unwrap();
        assert_eq!(update.version, "0.3.1");
        assert_eq!(update.notes.as_deref(), Some("Fixes"));

        let same = manifest("0.3.0", &url("a"));
        assert_eq!(same.update_for(&current, "darwin-aarch64").unwrap(), None);
        let older = manifest("0.2.9", &url("a"));
        assert_eq!(older.update_for(&current, "darwin-aarch64").unwrap(), None);
        assert!(Version::parse("0.10.0").unwrap() > Version::parse("0.9.0").unwrap());

        let pre = manifest("0.4.0-beta.1", &url("a"));
        assert!(matches!(
            pre.update_for(&current, "darwin-aarch64"),
            Err(ManifestError::PreRelease(_))
        ));
        let garbage = manifest("latest", &url("a"));
        assert!(matches!(
            garbage.update_for(&current, "darwin-aarch64"),
            Err(ManifestError::InvalidVersion(_))
        ));
    }

    #[test]
    fn responses_that_are_not_manifests_are_errors() {
        // What arrives offline, behind a captive portal or after an
        // interrupted download: never an update.
        for body in [
            "",
            "<html><body>Sign in to the network</body></html>",
            r#"{"version": "1.0.0", "platforms": {"linux-x86_64": {"signature": "#,
            r#"{"version": "1.0.0"}"#,
        ] {
            assert!(
                matches!(Manifest::parse(body), Err(ManifestError::Json(_))),
                "{body}"
            );
        }
    }

    #[test]
    fn download_prefix_is_the_repository() {
        assert_eq!(
            DOWNLOAD_PREFIX,
            format!("https://github.com/{REPOSITORY}/releases/download/")
        );
    }

    #[test]
    fn accepts_the_repository_name_in_any_case() {
        let current = Version::parse("0.3.0").unwrap();
        for good in [
            "https://github.com/knittingcode/DeckPilot/releases/download/v1/a.msi",
            "https://github.com/knittingcode/deckpilot/releases/download/v1/a.msi",
            "https://github.com/KnittingCode/DECKPILOT/releases/download/v1/a.msi",
        ] {
            let result = manifest("1.0.0", good).update_for(&current, "linux-x86_64");
            assert!(matches!(result, Ok(Some(_))), "{good}: {result:?}");
        }
    }

    #[test]
    fn the_case_insensitive_name_widens_nothing() {
        let current = Version::parse("0.3.0").unwrap();
        for bad in [
            "https://github.com/knittingcode/deckpilot-plugins/releases/download/v1/a.msi",
            "https://github.com/knittingcode/deckpilot2/releases/download/v1/a.msi",
            "https://github.com/knittingcode/deckpilo/releases/download/v1/a.msi",
            "https://github.com/knittingcodes/deckpilot/releases/download/v1/a.msi",
            "https://github.com/knittingcode.evil/deckpilot/releases/download/v1/a.msi",
            "https://github.com/evil/knittingcode/deckpilot/releases/download/v1/a.msi",
            "https://github.com//knittingcode/deckpilot/releases/download/v1/a.msi",
            "https://github.com/knittingcode//deckpilot/releases/download/v1/a.msi",
            "https://github.com/knittingcode/deckpilot/Releases/download/v1/a.msi",
            "https://github.com/knittingcode/deckpilot/releases/Download/v1/a.msi",
            "https://github.com/knittingcode/deckpilot/archive/download/v1/a.msi",
            "https://GITHUB.com/knittingcode/deckpilot/releases/download/v1/a.msi",
            "https://github.com.evil.example/knittingcode/deckpilot/releases/download/v1/a.msi",
            "https://evil.example/https://github.com/knittingcode/deckpilot/releases/download/v1/a.msi",
            "HTTPS://github.com/knittingcode/deckpilot/releases/download/v1/a.msi",
            "http://github.com/knittingcode/deckpilot/releases/download/v1/a.msi",
            "https://github.com/knittingcode/deckpilot/releases/download/",
            "https://github.com/knittingcode/deckpilot/releases/download/../../x",
            "https://github.com/knittingcode/deckpilot/releases/download/v1/a.msi?x=1",
            "https://github.com/knittingcode/deckpilot/releases/download/v1/a.msi#x",
            "https://github.com/knittingcode/deckpilot",
            // Non-ASCII look-alikes do not fold to the ASCII name.
            "https://github.com/knittingcode/deckpiloт/releases/download/v1/a.msi",
            "https://github.com/knıttingcode/deckpilot/releases/download/v1/a.msi",
        ] {
            let result = manifest("1.0.0", bad).update_for(&current, "linux-x86_64");
            assert!(
                matches!(result, Err(ManifestError::UntrustedUrl(_))),
                "{bad}: {result:?}"
            );
        }
    }

    #[test]
    fn rejects_untrusted_or_incomplete_manifests() {
        let current = Version::parse("0.3.0").unwrap();
        for bad in [
            "https://example.com/DeckPilot.msi",
            "http://github.com/knittingcode/DeckPilot/releases/download/v1/a.msi",
            "https://github.com/someone/DeckPilot/releases/download/v1/a.msi",
            "https://github.com/knittingcode/DeckPilot/releases/download/../../x",
            "https://github.com/knittingcode/DeckPilot/releases/download/v1/a.msi?x=1",
            "https://github.com/knittingcode/DeckPilot/releases/download/",
        ] {
            let result = manifest("1.0.0", bad).update_for(&current, "linux-x86_64");
            assert!(
                matches!(result, Err(ManifestError::UntrustedUrl(_))),
                "{bad}"
            );
        }

        let mut unsigned = manifest("1.0.0", &url("a"));
        unsigned
            .platforms
            .get_mut("linux-x86_64")
            .unwrap()
            .signature = "not a signature".into();
        assert_eq!(
            unsigned.update_for(&current, "linux-x86_64"),
            Err(ManifestError::InvalidSignature)
        );

        let missing = manifest("1.0.0", &url("a"));
        assert!(matches!(
            missing.update_for(&current, "linux-riscv64"),
            Err(ManifestError::MissingPlatform(_))
        ));
        assert!(matches!(Manifest::parse("{}"), Err(ManifestError::Json(_))));
    }

    fn validate(url: &str) -> Result<(), ManifestError> {
        manifest("1.0.0", url).validate_artifacts()
    }

    #[test]
    fn rejects_encoded_and_dot_segments() {
        for rest in [
            // Percent-encoded dot segments, in any case: `..` to a URL
            // parser, which resolves them out of the release downloads.
            "v1/%2e%2e/%2e%2e/other/x",
            "v1/%2E%2E/%2E%2E/other/x",
            "v1/%2e%2E/x",
            "v1/%2E./x",
            "v1/.%2e/x",
            "v1/%2e/x",
            "%2e%2e/%2e%2e/%2e%2e/%2e%2e/attacker/deckpilot/releases/download/v1/x",
            // An encoded dot inside a name, too.
            "v1/a%2emsi",
            // Encoded separators.
            "v1%2fa.msi",
            "v1/..%2Fx",
            "v1/a%5cb.msi",
            "v1/..%5C..%5Cx",
            // Double encoding: `%252e` is `%2e` once decoded.
            "v1/%252e%252e/x",
            "v1/%252E%252E/x",
            "v1/%252f/x",
            "v1/%255c/x",
            "v1/a%25.msi",
            // Malformed escapes.
            "v1/a%",
            "v1/a%2",
            "v1/a%zz.msi",
            "v1/a%+2.msi",
            "v1/a%%2e.msi",
            // Plain dot segments and empty segments.
            "v1/../x",
            "v1/./a.msi",
            "./v1/a.msi",
            "v1//a.msi",
            "v1/a.msi/",
            "v1/",
            "/v1/a.msi",
            // What a URL parser changes: not the URL as written.
            "v1/a b.msi",
            "v1/a\tb.msi",
            "v1/a\nb.msi",
            "v1/ä.msi",
            "v1/a\"b.msi",
            "v1/a<b>.msi",
            // As before.
            "v1/a\\b.msi",
            "v1/a.msi?x=1",
            "v1/a.msi#x",
        ] {
            for prefix in [
                DOWNLOAD_PREFIX,
                "https://github.com/knittingcode/deckpilot/releases/download/",
            ] {
                let bad = format!("{prefix}{rest}");
                assert_eq!(
                    validate(&bad),
                    Err(ManifestError::UntrustedUrl(bad.clone())),
                    "{bad}"
                );
            }
        }
    }

    #[test]
    fn accepts_plain_and_encoded_plus_asset_names() {
        for rest in [
            "v0.6.0-alpha.3/DeckPilot_aarch64.app.tar.gz",
            "v0.6.0-alpha.3/DeckPilot_0.6.0-alpha.3_x64_en-US.msi",
            "v0.6.0/DeckPilot-0.6.0-1.x86_64.rpm",
            // Build metadata, as written or encoded.
            "v1.0.0/DeckPilot_1.0.0+build.1_x64.msi",
            "v1.0.0/DeckPilot_1.0.0%2Bbuild.1_x64.msi",
            "v1.0.0/DeckPilot_1.0.0%2bbuild.1_x64.msi",
            "v1.0.0/DeckPilot_1.0.0~1_amd64.deb",
        ] {
            let good = format!("{DOWNLOAD_PREFIX}{rest}");
            assert_eq!(validate(&good), Ok(()), "{good}");
        }
    }

    /// The manifest published with 0.6.0-alpha.3, byte for byte: every URL
    /// still passes, and reads the same to a URL parser.
    #[test]
    fn the_published_alpha_3_manifest_validates() {
        let manifest = Manifest::parse(include_str!(
            "../tests/fixtures/v0.6.0-alpha.3-prerelease.json"
        ))
        .unwrap();
        assert_eq!(manifest.version, "0.6.0-alpha.3");
        assert_eq!(manifest.platforms.len(), 10);
        assert_eq!(manifest.validate_artifacts(), Ok(()));
        for (key, artifact) in &manifest.platforms {
            assert!(
                artifact
                    .url
                    .starts_with(&format!("{DOWNLOAD_PREFIX}v0.6.0-alpha.3/")),
                "{key}: {}",
                artifact.url
            );
            assert_eq!(
                url::Url::parse(&artifact.url).unwrap().as_str(),
                artifact.url
            );
        }
    }

    #[test]
    fn parses_bundler_output() {
        let json = format!(
            r#"{{"version":"0.3.1","notes":"n","pub_date":"2026-11-10T00:00:00Z",
                "platforms":{{"linux-x86_64":{{"signature":"{}","url":"{}"}}}}}}"#,
            SIGNATURE.trim(),
            url("DeckPilot.AppImage")
        );
        let parsed = Manifest::parse(&json).unwrap();
        assert_eq!(parsed.platforms.len(), 1);
        assert!(current_target().contains('-'));
    }
}
