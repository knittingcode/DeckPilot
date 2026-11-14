//! Registry documents, following `docs/marketplace/schemas/registry-*.schema.json`
//! of PR #191. `deny_unknown_fields` mirrors `additionalProperties: false`.
//! Field-level patterns of the schemas are not re-implemented here (the
//! registry CI validates them); the client checks what matters for trust.
//!
//! One extension beyond #191 is marked `EXTENSION (U-2)`: an optional
//! publisher signature, to test the deferred publisher-key option.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub const DOWNLOAD_PREFIX: &str =
    "https://github.com/knittingcode/deckpilot-plugins/releases/download/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub keys: Vec<String>,
    pub threshold: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyset {
    #[serde(rename = "_type")]
    pub kind: String,
    pub spec_version: u32,
    pub version: u64,
    pub root: Role,
    pub index: Role,
    pub timestamp: Role,
    pub seq_floor: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub seq_floor_reset: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRef {
    pub path: String,
    pub sha256: String,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timestamp {
    #[serde(rename = "_type")]
    pub kind: String,
    pub spec_version: u32,
    pub seq: u64,
    pub keys_version: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub signed_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires: OffsetDateTime,
    pub index: FileRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publisher {
    pub namespace: String,
    pub github_id: u64,
    pub login: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub verified: bool,
    /// EXTENSION (U-2): publisher's minisign public key, pinned by the
    /// reviewer-gated index (trust on first use at publisher onboarding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Latest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerelease: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deprecated {
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginEntry {
    pub id: String,
    pub publisher: String,
    pub name: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    pub latest: Latest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<Deprecated>,
    pub detail: FileRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RevocationAction {
    Yank,
    Disable,
}

/// `"*"` or a list of versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RevokedVersions {
    All(AllMarker),
    List(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AllMarker {
    #[serde(rename = "*")]
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revocation {
    pub id: String,
    pub versions: RevokedVersions,
    pub action: RevocationAction,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisory: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

impl Revocation {
    pub fn matches(&self, id: &str, version: &str) -> bool {
        self.id == id
            && match &self.versions {
                RevokedVersions::All(_) => true,
                RevokedVersions::List(list) => list.iter().any(|v| v == version),
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    #[serde(rename = "_type")]
    pub kind: String,
    pub spec_version: u32,
    pub seq: u64,
    pub keys_version: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires: OffsetDateTime,
    pub source_commit: String,
    pub download_prefix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_app_version: Option<String>,
    pub publishers: Vec<Publisher>,
    pub plugins: Vec<PluginEntry>,
    pub revocations: Vec<Revocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageRef {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Yanked {
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionEntry {
    pub version: String,
    #[serde(with = "time::serde::rfc3339")]
    pub published_at: OffsetDateTime,
    pub engines: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<serde_json::Value>,
    pub permissions: Vec<serde_json::Value>,
    pub manifest_sha256: String,
    pub source: serde_json::Value,
    pub package: PackageRef,
    pub builder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yanked: Option<Yanked>,
    /// EXTENSION (U-2): publisher minisign signature over the package bytes
    /// with the comment `sig::package_comment(id, version, sha256)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detail {
    #[serde(rename = "_type")]
    pub kind: String,
    pub spec_version: u32,
    pub id: String,
    pub publisher: String,
    pub name: String,
    pub description: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repository: Option<String>,
    pub versions: Vec<VersionEntry>,
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
