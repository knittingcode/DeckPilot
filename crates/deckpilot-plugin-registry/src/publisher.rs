//! **Experimental.** The first-party publisher signature: how DeckPilot
//! knows that a `.dpplugin` release artifact of one of its own plugins
//! (`deckpilot.*`) was released by DeckPilot
//! (`docs/marketplace/publisher-signature.md`).
//!
//! The plugins DeckPilot bundles are built, packaged and released in
//! `knittingcode/deckpilot-plugins`. Each release artifact is an ordinary
//! `.dpplugin` (the existing format, `docs/marketplace/artifact-contract.md`)
//! with a detached minisign signature by the **first-party publisher
//! role**. Before an application build stages such an artifact as a
//! bundled plugin it calls [`verify_release`], which checks, in order:
//!
//! 1. the pinned publisher keys are provisioned ([`PublisherKeys::pinned`],
//!    `publisher/first-party.json`, compiled in; never a key from the
//!    artifact, its manifest, its id or the repository it came from);
//! 2. the package's size and sha256 against the reviewed claim
//!    ([`ReleaseClaim`], from the app repository's artifact lock);
//! 3. a threshold of distinct, non-revoked publisher keys signed exactly
//!    these bytes with exactly [`release_comment`] (id, version, sha256 and
//!    size: a valid signature of another plugin, version or file never
//!    passes, nor a registry or updater signature);
//! 4. that no revocation names the release ([`RevokedRelease`]);
//! 5. the package itself with the first-party reader rules
//!    (`deckpilot_plugin_package::verify_package`, `first_party`: archive
//!    limits, manifest rules, id and version, the module check);
//! 6. compatibility with the host ([`HostEnvironment`]: `engines.deckpilot`,
//!    `engines.pluginApi`, `platforms`);
//! 7. rollback: never below the highest version an application release
//!    shipped, and the same version only with the same files
//!    ([`ReleaseFloor`], from `plugins/first-party/released-pins.json`).
//!
//! # Authenticity is not authorization
//!
//! A [`VerifiedRelease`] says who released the bytes and nothing about
//! what they may do. It carries no grant and no tier. Permissions are
//! decided exactly as before: a bundled copy is trusted by the pins
//! compiled into the agent (`deckpilot-plugin-bundled`), seeding derives
//! only low-tier grants, and every medium- and high-tier permission
//! (credentials, private network, process execution through `tools`, …)
//! waits for the user's consent.
//!
//! **The privileged-kinds gate is closed.** A signed artifact may not
//! declare any of plugin API 1.9's privileged kinds (`tools`, which is
//! process execution, `resources`, `projectContext`, `hostViews`,
//! `openUrl.fromParams`, `resource` parameters, `node.changesContext`):
//! [`verify_release`] refuses it with [`PublisherError::PrivilegedGateClosed`]
//! whatever the signature says. Those kinds reach a plugin only from
//! sources built in this repository and pinned in the agent, as today
//! (X12). Opening the gate needs the independent security review of
//! `docs/marketplace/publisher-review-request.md` and a code change; there
//! is no configuration, flag or key that opens it.
//!
//! No signing code is in this module: signing is the `publish` feature's
//! (`deckpilot-registry publisher sign`), run only in
//! `knittingcode/deckpilot-plugins`' release environment.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use deckpilot_plugin_manifest::{
    FIRST_PARTY_PUBLISHER, HostEnvironment, Incompatibility, IssueCode, Manifest, PackageDigest,
    PluginId, Sha256Digest,
};
use deckpilot_plugin_package::{
    ExpectedPackage, Files, Limits, MANIFEST, PackageError, read_archive, sha256, verify_package,
};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::model::{RevokedVersions, Role};
use crate::sig::{self, SigError, key_identity};

/// Prefix of every publisher trusted comment. Distinct from the registry's
/// (`deckpilot-registry v1 role:`) and from the updater's
/// (`timestamp:…\tfile:…`).
pub const COMMENT_PREFIX: &str = "deckpilot-publisher v1 role:";

/// The only `publisherKeysVersion`.
pub const KEYS_VERSION: u32 = 1;

/// Largest publisher keys document.
pub const MAX_KEYS_BYTES: usize = 16 * 1024;

/// Largest signature file (the registry's limit).
pub const MAX_SIGNATURE_BYTES: usize = crate::MAX_SIGNATURE_BYTES;

/// Most keys a publisher role may have, current and revoked each.
pub const MAX_KEYS: usize = 4;

/// The pinned publisher keys: `publisher/first-party.json` of this crate,
/// compiled in. Changing them is a reviewed change to this repository and
/// reaches users with an application release; nothing at run time can
/// replace them.
pub const PINNED_KEYS: &str = include_str!("../publisher/first-party.json");

/// `deckpilot-publisher v1 role:release id:<id> version:<version>
/// sha256:<hex> size:<bytes>`: what a release signature's trusted comment
/// must be, byte for byte.
pub fn release_comment(id: &PluginId, version: &Version, package: &PackageDigest) -> String {
    format!(
        "{COMMENT_PREFIX}release id:{id} version:{version} sha256:{} size:{}",
        package.sha256, package.size
    )
}

/// `publisher/first-party.json`: the first-party publisher role.
///
/// `keys` empty means **not provisioned**: the file is valid, but every
/// verification fails with [`PublisherError::NotProvisioned`]. That is the
/// state until the owner creates the production publisher key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PublisherKeys {
    pub publisher_keys_version: u32,
    /// Always [`FIRST_PARTY_PUBLISHER`]: the role signs `deckpilot.*` only.
    pub publisher: String,
    /// Base64-wrapped minisign public keys (the registry key format).
    pub keys: Vec<String>,
    /// Distinct keys that must each have signed a release.
    pub threshold: u64,
    /// Keys that must never count again (a lost or compromised key). A
    /// release signed by one is refused even when other keys also signed.
    pub revoked_keys: Vec<String>,
}

impl PublisherKeys {
    /// The keys compiled into this build ([`PINNED_KEYS`]).
    pub fn pinned() -> Result<Self, PublisherError> {
        Self::from_json(PINNED_KEYS.as_bytes())
    }

    /// Parses and validates a publisher keys document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, PublisherError> {
        if bytes.len() > MAX_KEYS_BYTES {
            return Err(PublisherError::Keys(format!(
                "{} bytes; at most {MAX_KEYS_BYTES}",
                bytes.len()
            )));
        }
        let keys: Self =
            serde_json::from_slice(bytes).map_err(|e| PublisherError::Keys(e.to_string()))?;
        keys.validate().map_err(PublisherError::Keys)?;
        Ok(keys)
    }

    /// The document's rules: version 1, publisher `deckpilot`, at most
    /// four distinct valid keys and four revoked ones, no key both current
    /// and revoked, and a threshold from 1 to the number of keys (1 when
    /// there are none).
    pub fn validate(&self) -> Result<(), String> {
        if self.publisher_keys_version != KEYS_VERSION {
            return Err(format!(
                "publisherKeysVersion {} (only {KEYS_VERSION})",
                self.publisher_keys_version
            ));
        }
        if self.publisher != FIRST_PARTY_PUBLISHER {
            return Err(format!(
                "publisher `{}`: the role is only for `{FIRST_PARTY_PUBLISHER}`",
                self.publisher
            ));
        }
        if self.keys.len() > MAX_KEYS || self.revoked_keys.len() > MAX_KEYS {
            return Err(format!(
                "at most {MAX_KEYS} keys and {MAX_KEYS} revoked keys"
            ));
        }
        let identities = |keys: &[String], what: &str| -> Result<BTreeSet<[u8; 32]>, String> {
            let mut seen = BTreeSet::new();
            for (n, key) in keys.iter().enumerate() {
                let id = key_identity(key).map_err(|e| format!("{what}[{n}]: {e}"))?;
                if !seen.insert(id) {
                    return Err(format!("{what}[{n}] is listed twice"));
                }
            }
            Ok(seen)
        };
        let current = identities(&self.keys, "keys")?;
        let revoked = identities(&self.revoked_keys, "revokedKeys")?;
        if !current.is_disjoint(&revoked) {
            return Err("a key is both current and revoked".into());
        }
        let count = u64::try_from(current.len()).unwrap_or(u64::MAX);
        if self.threshold == 0 || self.threshold > count.max(1) {
            return Err(format!(
                "threshold {} for {} key(s)",
                self.threshold,
                current.len()
            ));
        }
        Ok(())
    }

    /// Whether any key is pinned.
    pub fn is_provisioned(&self) -> bool {
        !self.keys.is_empty()
    }

    fn role(&self) -> Role {
        Role {
            keys: self.keys.clone(),
            threshold: self.threshold,
        }
    }

    /// Short fingerprints (the first 8 bytes of each Ed25519 key, hex) of
    /// the current keys, for logs and the audit record.
    pub fn fingerprints(&self) -> Vec<String> {
        self.keys.iter().filter_map(|k| fingerprint(k)).collect()
    }
}

/// The first 8 bytes of a key's Ed25519 key, hex.
pub fn fingerprint(public_key_b64: &str) -> Option<String> {
    key_identity(public_key_b64).ok().map(|id| {
        id[..8].iter().fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
    })
}

/// What a reviewed artifact lock says a release is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseClaim {
    pub id: PluginId,
    pub version: Version,
    pub package: PackageDigest,
}

/// A release that must never be used, with the reason (an artifact lock's
/// `revoked`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokedRelease {
    pub id: PluginId,
    pub versions: RevokedVersions,
    pub reason: String,
}

/// Where release artifacts are downloaded from: the plugin repository's
/// GitHub Releases, one release per plugin version (`<id>--v<version>`),
/// like the registry's packages. Trust never comes from this URL: only
/// from the claim and the signature.
pub const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/knittingcode/deckpilot-plugins/releases/download/";

/// The only `artifactsVersion`.
pub const ARTIFACTS_VERSION: u32 = 1;

/// `plugins/first-party/artifacts.json` of the application repository: the
/// reviewed list of first-party release artifacts an application build
/// stages as bundled plugins instead of building them from source, and the
/// releases it must never use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ArtifactLock {
    #[serde(rename = "$comment", default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub artifacts_version: u32,
    pub plugins: Vec<LockedArtifact>,
    pub revoked: Vec<RevokedRelease>,
}

/// One locked artifact: the claim, and where the package and its signature
/// are downloaded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedArtifact {
    pub id: PluginId,
    pub version: Version,
    pub sha256: Sha256Digest,
    pub size: u64,
}

impl LockedArtifact {
    /// What [`verify_release`] checks the downloaded package against.
    pub fn claim(&self) -> ReleaseClaim {
        ReleaseClaim {
            id: self.id.clone(),
            version: self.version.clone(),
            package: PackageDigest {
                sha256: self.sha256,
                size: self.size,
            },
        }
    }

    /// `<prefix><id>--v<version>/<id>-<version>.dpplugin`.
    pub fn package_url(&self) -> String {
        format!(
            "{RELEASE_DOWNLOAD_PREFIX}{id}--v{v}/{id}-{v}.dpplugin",
            id = self.id,
            v = self.version
        )
    }

    /// The package URL with `.minisig`.
    pub fn signature_url(&self) -> String {
        format!("{}.minisig", self.package_url())
    }
}

impl ArtifactLock {
    /// Parses and checks a lock: format 1, first-party ids only, one entry
    /// per id, no locked release that the lock itself revokes.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let lock: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if lock.artifacts_version != ARTIFACTS_VERSION {
            return Err(format!(
                "artifactsVersion {} (only {ARTIFACTS_VERSION})",
                lock.artifacts_version
            ));
        }
        let mut ids = BTreeSet::new();
        for artifact in &lock.plugins {
            if artifact.id.publisher() != FIRST_PARTY_PUBLISHER {
                return Err(format!("{} is not a first-party plugin", artifact.id));
            }
            if !ids.insert(artifact.id.as_str()) {
                return Err(format!("{} is locked twice", artifact.id));
            }
            if let Some(r) = lock
                .revoked
                .iter()
                .find(|r| r.id == artifact.id && r.versions.contains(&artifact.version))
            {
                return Err(format!(
                    "{} {} is locked and revoked ({})",
                    artifact.id, artifact.version, r.reason
                ));
            }
        }
        Ok(lock)
    }
}

/// The highest version of a plugin an application release shipped, with
/// its pinned files (`released-pins.json`): a release below it is a
/// rollback, and the same version must have the same files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFloor {
    pub version: Version,
    /// Size and sha256 by path, as `bundled/pins.json` pins them.
    pub files: BTreeMap<String, PackageDigest>,
}

/// What [`verify_release`] checks a release against besides the keys.
#[derive(Debug, Clone, Copy)]
pub struct ReleasePolicy<'a> {
    /// The host the release is for. `None` skips the compatibility check:
    /// only `release sign`'s own check before signing, which knows no host;
    /// an application build always names its host.
    pub host: Option<&'a HostEnvironment>,
    pub floor: Option<&'a ReleaseFloor>,
    pub revoked: &'a [RevokedRelease],
    pub limits: &'a Limits,
}

/// Why a release was refused. Every variant is a refusal: there is no
/// warning-only outcome.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PublisherError {
    #[error("the publisher keys are invalid: {0}")]
    Keys(String),
    #[error(
        "no first-party publisher key is pinned in this build (publisher/first-party.json): \
         nothing can be verified until the owner provisions one"
    )]
    NotProvisioned,
    #[error("the claim is for {found}, a first-party release is `{FIRST_PARTY_PUBLISHER}.*`")]
    NotFirstParty { found: String },
    #[error("the signature is {size} bytes; at most {MAX_SIGNATURE_BYTES}")]
    SignatureTooLarge { size: usize },
    #[error("{count} signatures; at most {MAX_KEYS} (one per key)")]
    TooManySignatures { count: usize },
    #[error("the package is {actual}, the claim says {expected}")]
    DigestMismatch { expected: String, actual: String },
    #[error("signed by a revoked publisher key ({fingerprint})")]
    RevokedKey { fingerprint: String },
    #[error("the publisher signature does not verify: {0}")]
    Signature(#[from] SigError),
    #[error("{id} {version} is revoked: {reason}")]
    Revoked {
        id: PluginId,
        version: Version,
        reason: String,
    },
    #[error(
        "the manifest declares {}: plugin API 1.9's privileged kinds never come from a signed \
         artifact; the gate stays closed until the security review of \
         docs/marketplace/publisher-review-request.md",
        .usages.join(", ")
    )]
    PrivilegedGateClosed { usages: Vec<String> },
    #[error("the package is invalid: {0}")]
    Package(String),
    #[error("not compatible with this DeckPilot: {}", list(.0))]
    Incompatible(Vec<Incompatibility>),
    #[error("{id} {version} is below {floor}, which a DeckPilot release already shipped")]
    Rollback {
        id: PluginId,
        version: Version,
        floor: Version,
    },
    #[error(
        "{id} {version} was shipped by a DeckPilot release with other files ({files}); a \
         changed plugin needs a new version"
    )]
    SameVersionOtherFiles {
        id: PluginId,
        version: Version,
        files: String,
    },
}

fn list(items: &[Incompatibility]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

impl From<PackageError> for PublisherError {
    fn from(error: PackageError) -> Self {
        Self::Package(error.to_string())
    }
}

/// A release whose signature, package, compatibility and version passed
/// [`verify_release`]. **Authenticity only:** it grants nothing (see the
/// module documentation).
#[derive(Debug, Clone)]
pub struct VerifiedRelease {
    claim: ReleaseClaim,
    manifest: Manifest,
    files: Files,
    signers: Vec<String>,
}

impl VerifiedRelease {
    pub fn claim(&self) -> &ReleaseClaim {
        &self.claim
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Every file of the package by path (`plugin.wasm` included).
    pub fn files(&self) -> &Files {
        &self.files
    }

    /// Size and sha256 of every file, as the bundled pins record them.
    pub fn file_digests(&self) -> BTreeMap<String, PackageDigest> {
        digests(&self.files)
    }

    /// Fingerprints of the publisher keys whose signature counted.
    pub fn signers(&self) -> &[String] {
        &self.signers
    }

    /// The audit record of this verification.
    pub fn audit(&self, at: DateTime<Utc>) -> AuditRecord {
        AuditRecord {
            at,
            outcome: AuditOutcome::Verified,
            id: self.claim.id.to_string(),
            version: self.claim.version.to_string(),
            sha256: self.claim.package.sha256.to_string(),
            size: self.claim.package.size,
            signers: self.signers.clone(),
            reason: None,
        }
    }
}

fn digests(files: &Files) -> BTreeMap<String, PackageDigest> {
    files
        .iter()
        .map(|(path, bytes)| {
            (
                path.clone(),
                PackageDigest {
                    sha256: sha256(bytes),
                    size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                },
            )
        })
        .collect()
}

/// The outcome of one verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditOutcome {
    Verified,
    Refused,
}

/// One line of the publisher audit log (JSON Lines): every verification,
/// accepted or refused, with what was checked and why it was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditRecord {
    #[serde(with = "crate::datetime")]
    pub at: DateTime<Utc>,
    pub outcome: AuditOutcome,
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub size: u64,
    /// Fingerprints of the keys whose signature counted (empty when refused).
    pub signers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl AuditRecord {
    /// The record of a refused verification.
    pub fn refused(claim: &ReleaseClaim, error: &PublisherError, at: DateTime<Utc>) -> Self {
        Self {
            at,
            outcome: AuditOutcome::Refused,
            id: claim.id.to_string(),
            version: claim.version.to_string(),
            sha256: claim.package.sha256.to_string(),
            size: claim.package.size,
            signers: Vec::new(),
            reason: Some(error.to_string()),
        }
    }

    /// The record as one JSON line (with the newline).
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_default();
        line.push('\n');
        line
    }
}

/// Verifies a first-party release artifact (`package`, the `.dpplugin`
/// bytes, and `signatures`, its base64 minisign signatures: one per key, a
/// threshold of them needed) against the publisher `keys`, the reviewed
/// `claim` and the `policy`. See the module documentation for the order of
/// the checks.
pub fn verify_release(
    package: &[u8],
    signatures: &[String],
    keys: &PublisherKeys,
    claim: &ReleaseClaim,
    policy: &ReleasePolicy<'_>,
) -> Result<VerifiedRelease, PublisherError> {
    keys.validate().map_err(PublisherError::Keys)?;
    if !keys.is_provisioned() {
        return Err(PublisherError::NotProvisioned);
    }
    if claim.id.publisher() != FIRST_PARTY_PUBLISHER {
        return Err(PublisherError::NotFirstParty {
            found: claim.id.to_string(),
        });
    }
    if signatures.len() > MAX_KEYS {
        return Err(PublisherError::TooManySignatures {
            count: signatures.len(),
        });
    }
    if let Some(signature) = signatures.iter().find(|s| s.len() > MAX_SIGNATURE_BYTES) {
        return Err(PublisherError::SignatureTooLarge {
            size: signature.len(),
        });
    }
    check_bytes(package, claim, policy.limits)?;
    let signers = check_signers(package, signatures, keys, claim)?;
    if let Some(revoked) = policy
        .revoked
        .iter()
        .find(|r| r.id == claim.id && r.versions.contains(&claim.version))
    {
        return Err(PublisherError::Revoked {
            id: claim.id.clone(),
            version: claim.version.clone(),
            reason: revoked.reason.clone(),
        });
    }

    let verified = check_contents(package, claim, policy.limits)?;
    if let Some(host) = policy.host {
        verified
            .manifest()
            .check_compatibility(host)
            .map_err(PublisherError::Incompatible)?;
    }
    if let Some(floor) = policy.floor {
        check_floor(claim, floor, verified.files())?;
    }
    Ok(VerifiedRelease {
        claim: claim.clone(),
        manifest: verified.manifest().clone(),
        files: verified.files().clone(),
        signers,
    })
}

/// What [`verify_release`] checks of a package besides its signature,
/// revocation, host and floor: the claimed bytes, the privileged-kinds gate
/// and the first-party package rules, with the claim's id and version.
/// `release sign` runs it **before** it loads the secret key, so no
/// archive is parsed while the key is in memory.
pub fn check_release_package(
    package: &[u8],
    claim: &ReleaseClaim,
    limits: &Limits,
) -> Result<Manifest, PublisherError> {
    if claim.id.publisher() != FIRST_PARTY_PUBLISHER {
        return Err(PublisherError::NotFirstParty {
            found: claim.id.to_string(),
        });
    }
    check_bytes(package, claim, limits)?;
    Ok(check_contents(package, claim, limits)?.manifest().clone())
}

/// The package, with the first-party rules. The privileged kinds are named
/// first, so the refusal says it is the gate.
fn check_contents(
    package: &[u8],
    claim: &ReleaseClaim,
    limits: &Limits,
) -> Result<deckpilot_plugin_package::VerifiedPackage, PublisherError> {
    let files = read_archive(package, limits)?;
    let manifest_bytes = files
        .get(MANIFEST)
        .ok_or_else(|| PublisherError::Package(format!("{MANIFEST} is missing")))?;
    check_gate(manifest_bytes)?;
    let expected = ExpectedPackage {
        id: claim.id.clone(),
        version: claim.version.clone(),
        package: claim.package.clone(),
        manifest_sha256: sha256(manifest_bytes),
        permissions: None,
        first_party: true,
    };
    Ok(verify_package(package, &expected, limits)?)
}

/// The bytes are the claimed bytes, before anything parses them.
fn check_bytes(
    package: &[u8],
    claim: &ReleaseClaim,
    limits: &Limits,
) -> Result<(), PublisherError> {
    let size = u64::try_from(package.len()).unwrap_or(u64::MAX);
    if size > limits.max_archive_bytes {
        return Err(PackageError::ArchiveTooLarge {
            size,
            limit: limits.max_archive_bytes,
        }
        .into());
    }
    let actual = PackageDigest {
        sha256: sha256(package),
        size,
    };
    if actual != claim.package {
        return Err(PublisherError::DigestMismatch {
            expected: describe(&claim.package),
            actual: describe(&actual),
        });
    }
    Ok(())
}

/// A threshold of current keys signed exactly this id, version and file,
/// and no revoked key did. Returns the fingerprints of the signers.
fn check_signers(
    package: &[u8],
    signatures: &[String],
    keys: &PublisherKeys,
    claim: &ReleaseClaim,
) -> Result<Vec<String>, PublisherError> {
    let comment = release_comment(&claim.id, &claim.version, &claim.package);
    let signed_by = |key: &String| {
        signatures
            .iter()
            .any(|signature| sig::verify_exact(key, signature, package, &comment).is_ok())
    };
    if let Some(revoked) = keys.revoked_keys.iter().find(|key| signed_by(key)) {
        return Err(PublisherError::RevokedKey {
            fingerprint: fingerprint(revoked).unwrap_or_default(),
        });
    }
    sig::verify_role(&keys.role(), signatures, package, &comment)?;
    Ok(keys
        .keys
        .iter()
        .filter(|key| signed_by(key))
        .filter_map(|key| fingerprint(key))
        .collect())
}

/// The privileged-kinds gate: closed (see the module documentation). A
/// manifest that does not even parse is refused here, never waved through
/// to a later check.
fn check_gate(manifest_bytes: &[u8]) -> Result<(), PublisherError> {
    let manifest = Manifest::from_slice(manifest_bytes)
        .map_err(|e| PublisherError::Package(format!("{MANIFEST}: {e}")))?;
    if let Err(errors) = manifest.validate_first_party() {
        let usages: Vec<String> = errors
            .0
            .iter()
            .filter(|issue| issue.code == IssueCode::FirstPartyOnly)
            .map(|issue| issue.path.clone())
            .collect();
        if !usages.is_empty() {
            return Err(PublisherError::PrivilegedGateClosed { usages });
        }
    }
    Ok(())
}

/// Never below what a release shipped; the same version only with the same
/// files.
fn check_floor(
    claim: &ReleaseClaim,
    floor: &ReleaseFloor,
    files: &Files,
) -> Result<(), PublisherError> {
    if claim.version < floor.version {
        return Err(PublisherError::Rollback {
            id: claim.id.clone(),
            version: claim.version.clone(),
            floor: floor.version.clone(),
        });
    }
    if claim.version == floor.version {
        let files = digests(files);
        if files != floor.files {
            return Err(PublisherError::SameVersionOtherFiles {
                id: claim.id.clone(),
                version: claim.version.clone(),
                files: changed(&floor.files, &files),
            });
        }
    }
    Ok(())
}

fn describe(digest: &PackageDigest) -> String {
    format!("sha256 {}, {} bytes", digest.sha256, digest.size)
}

/// The paths that differ between two file sets, for the message.
fn changed(old: &BTreeMap<String, PackageDigest>, new: &BTreeMap<String, PackageDigest>) -> String {
    let paths: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    paths
        .into_iter()
        .filter(|path| old.get(*path) != new.get(*path))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Reads a base64 minisign signature file: its text, trimmed of the final
/// newline. The format is checked by the verification.
pub fn read_signature(bytes: &[u8]) -> Result<String, PublisherError> {
    if bytes.len() > MAX_SIGNATURE_BYTES {
        return Err(PublisherError::SignatureTooLarge { size: bytes.len() });
    }
    let invalid = || {
        PublisherError::Signature(SigError::Signature(
            deckpilot_update::signature::SignatureError::InvalidSignature,
        ))
    };
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?.trim();
    // A signature is base64 of the four minisign lines.
    STANDARD.decode(text).map_err(|_| invalid())?;
    Ok(text.to_owned())
}

/// The sha256 of `bytes` (re-exported for the tools that build claims).
pub fn digest_of(bytes: &[u8]) -> PackageDigest {
    PackageDigest {
        sha256: sha256(bytes),
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
    }
}

/// Parses `sha256` text for a claim.
pub fn parse_sha256(text: &str) -> Result<Sha256Digest, PublisherError> {
    Sha256Digest::parse(text).map_err(|e| PublisherError::Package(format!("sha256: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_keys_are_valid_and_not_provisioned() {
        // No production publisher key exists yet (the owner creates it).
        // When one is pinned, this test changes with it in review.
        let keys = PublisherKeys::pinned().unwrap();
        assert!(!keys.is_provisioned());
        assert_eq!(keys.fingerprints(), Vec::<String>::new());
    }

    #[test]
    fn comments_are_domain_separated() {
        let id = PluginId::parse("deckpilot.github").unwrap();
        let digest = digest_of(b"x");
        let comment = release_comment(&id, &Version::new(0, 2, 1), &digest);
        assert_eq!(
            comment,
            format!(
                "deckpilot-publisher v1 role:release id:deckpilot.github version:0.2.1 \
                 sha256:{} size:1",
                digest.sha256
            )
        );
        assert!(!comment.starts_with(sig::COMMENT_PREFIX));
    }

    /// The gate is closed for the bundled plugins that use plugin API 1.9's
    /// privileged kinds (process execution included) and open to nothing:
    /// the plugins without them pass it.
    #[test]
    fn the_privileged_kinds_gate_is_closed() {
        let closed = [
            include_str!("../../../plugins/first-party/docker/manifest.json"),
            include_str!("../../../plugins/first-party/git/manifest.json"),
            include_str!("../../../plugins/first-party/projects/manifest.json"),
            include_str!("../../../plugins/first-party/workflows/manifest.json"),
            include_str!("../../../plugins/first-party/workspace/manifest.json"),
        ];
        for manifest in closed {
            let error = check_gate(manifest.as_bytes()).unwrap_err();
            assert!(
                matches!(&error, PublisherError::PrivilegedGateClosed { usages } if !usages.is_empty()),
                "{error}"
            );
            assert!(error.to_string().contains("publisher-review-request.md"));
        }
        let git = include_str!("../../../plugins/first-party/git/manifest.json");
        let error = check_gate(git.as_bytes()).unwrap_err().to_string();
        assert!(
            error.contains("permissions["),
            "names the tools permission: {error}"
        );
        for open in [
            include_str!("../../../plugins/first-party/github/manifest.json"),
            include_str!("../../../plugins/first-party/gitlab/manifest.json"),
            include_str!("../../../plugins/first-party/jira/manifest.json"),
            include_str!("../../../plugins/first-party/notifications/manifest.json"),
        ] {
            check_gate(open.as_bytes()).unwrap();
            Manifest::from_slice(open.as_bytes())
                .unwrap()
                .validate_first_party()
                .unwrap();
        }
    }

    #[test]
    fn the_gate_refuses_a_manifest_that_does_not_parse() {
        for bytes in [&b"{ not json"[..], b"{}", b"[]", b""] {
            assert!(
                matches!(check_gate(bytes), Err(PublisherError::Package(_))),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn keys_documents_are_checked() {
        let doc = |extra: &str| {
            format!(
                "{{\"publisherKeysVersion\":1,\"publisher\":\"deckpilot\",\"keys\":[],\
                 \"threshold\":1,\"revokedKeys\":[]{extra}}}"
            )
        };
        assert!(PublisherKeys::from_json(doc("").as_bytes()).is_ok());
        assert!(PublisherKeys::from_json(doc(",\"extra\":1").as_bytes()).is_err());
        let other = doc("").replace("\"deckpilot\"", "\"acme\"");
        assert!(PublisherKeys::from_json(other.as_bytes()).is_err());
        let zero = doc("").replace("\"threshold\":1", "\"threshold\":0");
        assert!(PublisherKeys::from_json(zero.as_bytes()).is_err());
        let two = doc("").replace("\"threshold\":1", "\"threshold\":2");
        assert!(PublisherKeys::from_json(two.as_bytes()).is_err());
        let v2 = doc("").replace("Version\":1", "Version\":2");
        assert!(PublisherKeys::from_json(v2.as_bytes()).is_err());
        let bad_key = doc("").replace("\"keys\":[]", "\"keys\":[\"not a key\"]");
        assert!(PublisherKeys::from_json(bad_key.as_bytes()).is_err());
    }
}
