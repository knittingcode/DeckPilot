//! The update manifest (`latest.json`) produced by the Tauri bundler.

use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::DOWNLOAD_PREFIX;

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

fn validate_artifact(artifact: &PlatformArtifact) -> Result<(), ManifestError> {
    let url = &artifact.url;
    let path = url.strip_prefix(DOWNLOAD_PREFIX).unwrap_or_default();
    if path.is_empty() || path.contains("..") || url.contains(['?', '#', '\\']) {
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
