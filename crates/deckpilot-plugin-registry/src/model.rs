//! The registry documents, field for field as `schema/registry-*.schema.json`
//! (#191). Every struct refuses unknown fields (`additionalProperties:
//! false`), and each document has a `validate` method for the constraints
//! serde cannot express (lengths, patterns, ranges, uniqueness) plus the
//! semantic rules of section 10 that need nothing but the document itself.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use deckpilot_plugin_manifest::{
    Engines, MAX_PACKAGE_BYTES, Permission, Platform, PluginId, Sha256Digest, check_relative_path,
    is_publisher_id,
};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::sig::key_identity;

/// The only `spec_version` this crate reads.
pub const SPEC_VERSION: u64 = 1;

/// Every package URL starts with this (`index.download_prefix`, a schema
/// constant).
pub const DOWNLOAD_PREFIX: &str =
    "https://github.com/knittingcode/deckpilot-plugins/releases/download/";

/// Largest index document (`timestamp.index.length`).
pub const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;
/// Largest detail document (`fileRef.length`).
pub const MAX_DETAIL_BYTES: u64 = 256 * 1024;

macro_rules! doc_type {
    ($(#[$meta:meta])* $name:ident = $value:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            #[default]
            #[serde(rename = $value)]
            Value,
        }
    };
}

doc_type!(
    /// `_type` of `keys/<n>.json`.
    KeysetType = "deckpilot-registry-keys"
);
doc_type!(
    /// `_type` of `timestamp.json`.
    TimestampType = "deckpilot-registry-timestamp"
);
doc_type!(
    /// `_type` of `index/<seq>.json`.
    IndexType = "deckpilot-registry-index"
);
doc_type!(
    /// `_type` of `plugins/<id>/<sha256>.json`.
    DetailType = "deckpilot-registry-plugin"
);

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's signature
fn is_false(value: &bool) -> bool {
    !*value
}

// -------------------------------------------------------------- key set

/// A signing role: base64-wrapped minisign public keys and how many of them
/// must sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub keys: Vec<String>,
    pub threshold: u64,
}

impl Role {
    /// `$defs/role`: 1-4 well-formed keys, all different, threshold 1-4
    /// and at most the number of keys.
    pub fn validate(&self) -> Result<(), String> {
        if self.keys.is_empty() || self.keys.len() > 4 {
            return Err(format!("has {} keys; 1 to 4 are allowed", self.keys.len()));
        }
        let mut seen = BTreeSet::new();
        for key in &self.keys {
            if key.len() < 40
                || key.len() > 512
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
            {
                return Err("has a key that is not base64 (40-512 characters)".into());
            }
            let id = key_identity(key).map_err(|_| "has a malformed minisign public key")?;
            if !seen.insert(id) {
                return Err("lists the same key twice".into());
            }
        }
        let keys = u64::try_from(self.keys.len()).unwrap_or(u64::MAX);
        if self.threshold == 0 || self.threshold > 4 || self.threshold > keys {
            return Err(format!("has threshold {} for {keys} keys", self.threshold));
        }
        Ok(())
    }

    pub(crate) fn identities(&self) -> BTreeSet<[u8; 32]> {
        self.keys
            .iter()
            .filter_map(|k| key_identity(k).ok())
            .collect()
    }
}

/// `keys/<n>.json`: one link of the key set chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyset {
    #[serde(rename = "_type")]
    pub kind: KeysetType,
    pub spec_version: u64,
    pub version: u64,
    pub root: Role,
    pub index: Role,
    pub timestamp: Role,
    /// Lowest seq accepted under this key set.
    pub seq_floor: u64,
    /// Recovery from an index-key compromise: replace the stored seq with
    /// `seq_floor` (and, in this crate, allow the next index to lift
    /// revocations; see the crate documentation).
    #[serde(default, skip_serializing_if = "is_false")]
    pub seq_floor_reset: bool,
}

impl Keyset {
    pub fn validate(&self) -> Result<(), String> {
        if self.spec_version != SPEC_VERSION {
            return Err(format!("spec_version {} is not 1", self.spec_version));
        }
        if self.version == 0 {
            return Err("version must be at least 1".into());
        }
        let mut all = BTreeSet::new();
        for (name, role) in [
            ("root", &self.root),
            ("index", &self.index),
            ("timestamp", &self.timestamp),
        ] {
            role.validate().map_err(|e| format!("{name} role {e}"))?;
            // Separation of privilege: the unattended timestamp key must
            // never be able to act as the reviewer-gated index key or the
            // offline root.
            for id in role.identities() {
                if !all.insert(id) {
                    return Err(format!("{name} role shares a key with another role"));
                }
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------- timestamp

/// A pinned document: path, sha256 and length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRef {
    pub path: String,
    pub sha256: Sha256Digest,
    pub length: u64,
}

/// `timestamp.json`: the only mutable document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timestamp {
    #[serde(rename = "_type")]
    pub kind: TimestampType,
    pub spec_version: u64,
    pub seq: u64,
    pub keys_version: u64,
    #[serde(with = "crate::datetime")]
    pub signed_at: DateTime<Utc>,
    #[serde(with = "crate::datetime")]
    pub expires: DateTime<Utc>,
    pub index: FileRef,
}

fn is_positive_decimal(text: &str) -> bool {
    !text.is_empty() && !text.starts_with('0') && text.bytes().all(|b| b.is_ascii_digit())
}

impl Timestamp {
    pub fn validate(&self) -> Result<(), String> {
        if self.spec_version != SPEC_VERSION {
            return Err(format!("spec_version {} is not 1", self.spec_version));
        }
        if self.seq == 0 || self.keys_version == 0 {
            return Err("seq and keys_version must be at least 1".into());
        }
        let number = self
            .index
            .path
            .strip_prefix("index/")
            .and_then(|rest| rest.strip_suffix(".json"));
        if !number.is_some_and(is_positive_decimal) {
            return Err(format!(
                "index.path `{}` is not index/<seq>.json",
                self.index.path
            ));
        }
        if self.index.length == 0 || self.index.length > MAX_INDEX_BYTES {
            return Err(format!(
                "index.length {} is out of range",
                self.index.length
            ));
        }
        if self.expires <= self.signed_at {
            return Err("expires is not after signed_at".into());
        }
        Ok(())
    }
}

// ----------------------------------------------------------------- index

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexPublisher {
    pub namespace: String,
    pub github_id: u64,
    pub login: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Latest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable: Option<Version>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerelease: Option<Version>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deprecated {
    #[serde(with = "crate::datetime")]
    pub at: DateTime<Utc>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<PluginId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexPlugin {
    pub id: PluginId,
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

impl IndexPlugin {
    fn validate(&self, at: &str) -> Result<(), String> {
        if self.publisher != self.id.publisher() {
            return Err(format!("{at}.publisher does not match the id"));
        }
        chars(&format!("{at}.name"), &self.name, 1, 48)?;
        chars(&format!("{at}.summary"), &self.summary, 1, 300)?;
        if let Some(categories) = &self.categories {
            let unique: BTreeSet<_> = categories.iter().collect();
            if categories.len() > 3 || unique.len() != categories.len() {
                return Err(format!("{at}.categories: at most 3, all different"));
            }
            if !categories.iter().all(|c| {
                (2..=24).contains(&c.len())
                    && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
            }) {
                return Err(format!("{at}.categories has a malformed category"));
            }
        }
        match (&self.latest.stable, &self.latest.prerelease) {
            (None, None) => return Err(format!("{at}.latest is empty")),
            (Some(stable), _) if !stable.pre.is_empty() => {
                return Err(format!("{at}.latest.stable is a pre-release"));
            }
            (_, Some(pre)) if pre.pre.is_empty() => {
                return Err(format!("{at}.latest.prerelease is not a pre-release"));
            }
            _ => {}
        }
        if let Some(deprecated) = &self.deprecated {
            chars(
                &format!("{at}.deprecated.reason"),
                &deprecated.reason,
                1,
                500,
            )?;
        }
        // Content-addressed: the file name is the sha256 of its bytes.
        let path = format!("plugins/{}/{}.json", self.id, self.detail.sha256);
        if self.detail.path != path {
            return Err(format!("{at}.detail.path is not {path}"));
        }
        if self.detail.length == 0 || self.detail.length > MAX_DETAIL_BYTES {
            return Err(format!("{at}.detail.length is out of range"));
        }
        Ok(())
    }
}

/// `yank` < `disable`: a disable also covers a yank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RevocationAction {
    /// Not offered for install or update; installed copies keep running.
    Yank,
    /// Installed copies are stopped and refused; installs refused.
    Disable,
}

/// `"*"` or a list of versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawVersions", into = "RawVersions")]
pub enum RevokedVersions {
    All,
    List(Vec<Version>),
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum RawVersions {
    Star(String),
    List(Vec<Version>),
}

impl TryFrom<RawVersions> for RevokedVersions {
    type Error = String;

    fn try_from(raw: RawVersions) -> Result<Self, String> {
        match raw {
            RawVersions::Star(star) if star == "*" => Ok(Self::All),
            RawVersions::Star(other) => Err(format!("`{other}` is neither \"*\" nor a list")),
            RawVersions::List(list) => Ok(Self::List(list)),
        }
    }
}

impl From<RevokedVersions> for RawVersions {
    fn from(versions: RevokedVersions) -> Self {
        match versions {
            RevokedVersions::All => Self::Star("*".into()),
            RevokedVersions::List(list) => Self::List(list),
        }
    }
}

impl RevokedVersions {
    pub fn contains(&self, version: &Version) -> bool {
        match self {
            Self::All => true,
            Self::List(list) => list.contains(version),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revocation {
    pub id: PluginId,
    pub versions: RevokedVersions,
    pub action: RevocationAction,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisory: Option<String>,
    #[serde(with = "crate::datetime")]
    pub at: DateTime<Utc>,
}

impl Revocation {
    pub fn matches(&self, id: &PluginId, version: &Version) -> bool {
        self.id == *id && self.versions.contains(version)
    }

    /// Whether `later` still revokes everything this revocation revokes,
    /// at least as strongly (section 10: revocations are cumulative).
    /// Reason, advisory and date may change; scope and strength may not
    /// shrink.
    pub fn is_covered_by(&self, later: &[Revocation]) -> bool {
        let mut candidates = later
            .iter()
            .filter(|r| r.id == self.id && r.action >= self.action);
        match &self.versions {
            RevokedVersions::All => candidates.any(|r| r.versions == RevokedVersions::All),
            RevokedVersions::List(list) => {
                let candidates: Vec<_> = candidates.collect();
                list.iter()
                    .all(|v| candidates.iter().any(|r| r.versions.contains(v)))
            }
        }
    }
}

/// `index/<seq>.json`: an immutable snapshot of the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    #[serde(rename = "_type")]
    pub kind: IndexType,
    pub spec_version: u64,
    pub seq: u64,
    pub keys_version: u64,
    #[serde(with = "crate::datetime")]
    pub generated_at: DateTime<Utc>,
    /// #191: `generated_at` + 90 days. Not checked by default; the
    /// timestamp governs freshness (see [`crate::Policy`]).
    #[serde(with = "crate::datetime")]
    pub expires: DateTime<Utc>,
    pub source_commit: String,
    pub download_prefix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_app_version: Option<Version>,
    pub publishers: Vec<IndexPublisher>,
    pub plugins: Vec<IndexPlugin>,
    pub revocations: Vec<Revocation>,
}

fn chars(field: &str, text: &str, min: usize, max: usize) -> Result<(), String> {
    let count = text.chars().count();
    if count < min || count > max {
        return Err(format!(
            "{field} has {count} characters; {min} to {max} are allowed"
        ));
    }
    Ok(())
}

fn is_hex40(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_https(text: &str) -> bool {
    text.strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(char::is_whitespace))
}

/// `^[A-Za-z0-9-]{1,39}/[A-Za-z0-9._-]{1,100}$`
fn is_repository(text: &str) -> bool {
    text.split_once('/').is_some_and(|(owner, name)| {
        (1..=39).contains(&owner.len())
            && owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && (1..=100).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    })
}

impl Index {
    pub fn validate(&self) -> Result<(), String> {
        if self.spec_version != SPEC_VERSION {
            return Err(format!("spec_version {} is not 1", self.spec_version));
        }
        if self.seq == 0 || self.keys_version == 0 {
            return Err("seq and keys_version must be at least 1".into());
        }
        if self.expires <= self.generated_at {
            return Err("expires is not after generated_at".into());
        }
        if !is_hex40(&self.source_commit) {
            return Err("source_commit is not a 40-digit commit hash".into());
        }
        if self.download_prefix != DOWNLOAD_PREFIX {
            return Err(format!("download_prefix is not {DOWNLOAD_PREFIX}"));
        }
        let mut namespaces = BTreeSet::new();
        for (i, publisher) in self.publishers.iter().enumerate() {
            let at = format!("publishers[{i}]");
            if !is_publisher_id(&publisher.namespace) {
                return Err(format!("{at}.namespace is not a publisher id"));
            }
            if !namespaces.insert(publisher.namespace.as_str()) {
                return Err(format!("{at}.namespace appears twice"));
            }
            if publisher.github_id == 0 {
                return Err(format!("{at}.github_id must be at least 1"));
            }
            if !(1..=39).contains(&publisher.login.len())
                || !publisher
                    .login
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(format!("{at}.login is not a GitHub login"));
            }
            if let Some(name) = &publisher.display_name {
                chars(&format!("{at}.display_name"), name, 1, 64)?;
            }
        }
        let mut ids = BTreeSet::new();
        for (i, plugin) in self.plugins.iter().enumerate() {
            let at = format!("plugins[{i}]");
            if !ids.insert(&plugin.id) {
                return Err(format!("{at}.id {} appears twice", plugin.id));
            }
            plugin.validate(&at)?;
        }
        for (i, revocation) in self.revocations.iter().enumerate() {
            let at = format!("revocations[{i}]");
            if let RevokedVersions::List(list) = &revocation.versions {
                let unique: BTreeSet<_> = list.iter().collect();
                if list.is_empty() || unique.len() != list.len() {
                    return Err(format!("{at}.versions: at least one, all different"));
                }
            }
            chars(&format!("{at}.reason"), &revocation.reason, 1, 500)?;
            if let Some(advisory) = &revocation.advisory
                && !is_https(advisory)
            {
                return Err(format!("{at}.advisory is not an https URL"));
            }
        }
        Ok(())
    }

    /// The entry of plugin `id`.
    pub fn plugin(&self, id: &PluginId) -> Option<&IndexPlugin> {
        self.plugins.iter().find(|p| p.id == *id)
    }
}

// ---------------------------------------------------------------- detail

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionSource {
    pub repository: String,
    pub tag: String,
    pub commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageRef {
    pub url: String,
    pub sha256: Sha256Digest,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Yanked {
    #[serde(with = "crate::datetime")]
    pub at: DateTime<Utc>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailVersion {
    pub version: Version,
    #[serde(with = "crate::datetime")]
    pub published_at: DateTime<Utc>,
    pub engines: Engines,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<Platform>>,
    /// Kept as JSON values: the manifest's `permissions` must be the same
    /// value (shown for consent before the download).
    pub permissions: Vec<serde_json::Value>,
    pub manifest_sha256: Sha256Digest,
    pub source: VersionSource,
    pub package: PackageRef,
    pub builder: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yanked: Option<Yanked>,
}

/// `plugins/<id>/<sha256>.json`: every published version of one plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDetail {
    #[serde(rename = "_type")]
    pub kind: DetailType,
    pub spec_version: u64,
    pub id: PluginId,
    pub publisher: String,
    pub name: String,
    pub description: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repository: Option<String>,
    pub versions: Vec<DetailVersion>,
}

/// The release URL prefix of one version: `<prefix><id>--v<version>/`.
pub fn release_prefix(id: &PluginId, version: &Version) -> String {
    format!("{DOWNLOAD_PREFIX}{id}--v{version}/")
}

impl PluginDetail {
    pub fn validate(&self) -> Result<(), String> {
        if self.spec_version != SPEC_VERSION {
            return Err(format!("spec_version {} is not 1", self.spec_version));
        }
        if self.publisher != self.id.publisher() {
            return Err("publisher does not match the id".into());
        }
        chars("name", &self.name, 1, 48)?;
        chars("description", &self.description, 1, 4000)?;
        chars("license", &self.license, 1, 64)?;
        if let Some(homepage) = &self.homepage
            && (!is_https(homepage) || homepage.len() > 512)
        {
            return Err("homepage is not an https URL of at most 512 characters".into());
        }
        if let Some(repository) = &self.source_repository
            && !is_repository(repository)
        {
            return Err("source_repository is not owner/name".into());
        }
        if self.versions.is_empty() {
            return Err("versions is empty".into());
        }
        for pair in self.versions.windows(2) {
            if pair[0].version >= pair[1].version {
                return Err(format!(
                    "versions are not strictly increasing ({} then {})",
                    pair[0].version, pair[1].version
                ));
            }
        }
        for entry in &self.versions {
            let at = format!("versions[{}]", entry.version);
            if entry.permissions.len() > 16 {
                return Err(format!("{at}.permissions has more than 16 entries"));
            }
            for permission in &entry.permissions {
                serde_json::from_value::<Permission>(permission.clone())
                    .map_err(|e| format!("{at}.permissions: {e}"))?;
            }
            let source = &entry.source;
            if !is_repository(&source.repository) {
                return Err(format!("{at}.source.repository is not owner/name"));
            }
            chars(&format!("{at}.source.tag"), &source.tag, 1, 128)?;
            if !is_hex40(&source.commit) {
                return Err(format!("{at}.source.commit is not a commit hash"));
            }
            if let Some(path) = &source.path {
                check_relative_path(path).map_err(|e| format!("{at}.source.path {e}"))?;
            }
            if let Some(archive) = &source.archive {
                let prefix = release_prefix(&self.id, &entry.version);
                if !archive
                    .strip_prefix(&prefix)
                    .and_then(|name| name.strip_suffix(".source.tar.gz"))
                    .is_some_and(is_asset_name)
                {
                    return Err(format!("{at}.source.archive is not a release asset"));
                }
            }
            let prefix = release_prefix(&self.id, &entry.version);
            if !entry
                .package
                .url
                .strip_prefix(&prefix)
                .and_then(|name| name.strip_suffix(".dpplugin"))
                .is_some_and(is_asset_name)
            {
                return Err(format!(
                    "{at}.package.url is not a release asset under {prefix}"
                ));
            }
            if entry.package.size == 0 || entry.package.size > MAX_PACKAGE_BYTES {
                return Err(format!("{at}.package.size is out of range"));
            }
            if let Some(attestation) = &entry.attestation
                && !is_https(attestation)
            {
                return Err(format!("{at}.attestation is not an https URL"));
            }
            if let Some(yanked) = &entry.yanked {
                chars(&format!("{at}.yanked.reason"), &yanked.reason, 1, 500)?;
            }
        }
        Ok(())
    }

    /// The entry of `version`.
    pub fn version(&self, version: &Version) -> Option<&DetailVersion> {
        self.versions.iter().find(|v| v.version == *version)
    }
}

/// `[a-z0-9.-]+` (a release asset file name without its extension).
fn is_asset_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
}
