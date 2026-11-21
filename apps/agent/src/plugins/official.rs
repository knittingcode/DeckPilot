//! Official plugins (`deckpilot.*`) delivered by a registry as first-party
//! release artifacts (`docs/marketplace/publisher-signature.md`, "The agent:
//! installing and updating official plugins").
//!
//! A `deckpilot.*` package from a registry is installed or updated only if,
//! on top of everything the registry chain already checked (root-signed key
//! set authorizing the `deckpilot` namespace, verified publisher record,
//! pinned size and sha256), it is a **first-party release**: a threshold of
//! the publisher keys compiled into this agent signed exactly these bytes
//! for exactly this id and version ([`publisher::verify_release`]). The two
//! signatures are independent: a registry compromise cannot sign a release,
//! and a publisher signature cannot make the registry list anything.
//!
//! On top of the verifier, the agent adds what only it knows:
//!
//! * **A rollback floor per plugin**, persisted in `<data dir>/plugin-official/
//!   floors.json`: the highest release it ever verified for the plugin, and
//!   its files. Never below it; the same version only with the same files.
//!   The bundled copy this agent pins, the highest version a DeckPilot
//!   release shipped on this platform (`released-pins.json`) and the
//!   artifact lock count as floors too, all compiled in. The floor is
//!   raised as soon as a release verified, before the store installs it, so
//!   an interrupted install can be retried with the same bytes and nothing
//!   older. An unreadable floor file refuses every official install
//!   (fail closed; it is never reset).
//! * **Revocations**: the artifact lock compiled into the agent
//!   (`plugins/first-party/artifacts.json`, `revoked`) and the pinned
//!   `revokedKeys` refuse a release before installation and quarantine an
//!   installed registry copy (with the registry's own revocations, which the
//!   service applies as before).
//! * **Compatibility** with the running host (app version, plugin API,
//!   platform), always: an official artifact is never checked without a host.
//! * **An audit log**, `<data dir>/plugin-official/audit.jsonl`: one
//!   [`AuditRecord`] per verification, accepted or refused (rotated at
//!   [`MAX_AUDIT_BYTES`]), and the agent's log.
//!
//! **Authenticity is not authorization.** Nothing here grants anything:
//! the verified package goes through the store's ordinary install or update
//! with exactly the consent the user gave, so new or broadened permissions
//! stay staged until the user approves them, and an update keeps a grant only
//! where the new version still declares it (the store's permission diff).
//!
//! **The privileged-kinds gate stays closed** ([`PRIVILEGED_GATE`]). Its
//! opening path ([`admit_privileged`]) is reviewed code behind a compiled
//! constant, not a flag, and even an opened gate would still meet the
//! store's own rule that plugin API 1.9's privileged kinds come only from a
//! bundled source (`check_privileged_source`, X12).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use deckpilot_ipc::plugins::reason;
use deckpilot_ipc::{ErrorCode, IpcError};
use deckpilot_plugin_bundled::Pin;
use deckpilot_plugin_manifest::{HostEnvironment, Manifest, PackageDigest, PluginId};
use deckpilot_plugin_package::{
    ExpectedFiles, ExpectedPackage, Files, Limits, PackageError, VerifiedPackage, read_archive,
    sha256, verify_bundled_files, verify_package,
};
use deckpilot_plugin_registry::publisher::{
    self, ArtifactLock, AuditRecord, PublisherError, PublisherKeys, ReleaseClaim, ReleaseFloor,
    ReleasePolicy, RevokedRelease, VerifiedRelease,
};
use deckpilot_plugin_registry::{InstalledVerdict, write_atomic};
use deckpilot_plugin_store::{InstallSource, PluginRecord, VerdictSource, VersionRecord};
use semver::Version;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Below the data directory (outside `plugins/`, whose unknown entries the
/// store's recovery sweeps away).
pub(super) const STATE_DIR: &str = "plugin-official";
const FLOORS_FILE: &str = "floors.json";
const AUDIT_FILE: &str = "audit.jsonl";
/// The audit log is rotated to `audit.jsonl.1` above this size.
pub(super) const MAX_AUDIT_BYTES: u64 = 1024 * 1024;
const FLOORS_VERSION: u32 = 1;
/// The reviewed artifact lock of this build; its `revoked` list refuses
/// releases at run time too.
const ARTIFACT_LOCK: &str = include_str!("../../../../plugins/first-party/artifacts.json");
/// What every DeckPilot release shipped per platform: the highest version
/// of each plugin is a floor no registry release may go below.
const RELEASED_PINS: &str = include_str!("../../../../plugins/first-party/released-pins.json");
/// Rotated audit logs kept (`audit.jsonl.1` the newest).
pub(super) const AUDIT_GENERATIONS: usize = 5;

/// Test only (feature `test-hooks`): publisher keys read from this file
/// instead of the pinned ones, so the end-to-end tests can sign releases
/// with throwaway keys. Never read by a release build.
#[cfg(feature = "test-hooks")]
pub const TEST_KEYS_ENV: &str = "DECKPILOT_TEST_PUBLISHER_KEYS";

// ------------------------------------------------------------------ gate

/// The privileged-kinds gate for official artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrivilegedGate {
    /// Every artifact declaring a plugin API 1.9 privileged kind is
    /// refused (`PublisherError::PrivilegedGateClosed`).
    Closed,
    /// Opened by a recorded review, for the plugins and privileged
    /// permissions it names, under its conditions.
    #[cfg_attr(not(test), allow(dead_code))]
    Open(&'static GateReview),
}

/// What an independent security review allowed when it opened the gate
/// (`docs/marketplace/publisher-review-request.md`, Sign-off).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct GateReview {
    pub reviewer: &'static str,
    pub date: &'static str,
    /// Distinct publisher keys a privileged release needs; at least 2.
    pub min_threshold: u64,
    /// Per plugin id, every privileged construct it may declare, as
    /// [`deckpilot_plugin_store::privileged_constructs`] names them: the
    /// privileged grant keys (`tools:git`, `resources`, `hostViews`, …) and
    /// the other gated constructs by their labels ("a \`resource\`
    /// parameter", "\`node.changesContext\`", "the host source \`…\`", "the
    /// \`projects\` view", "\`statusItems\`", "\`dashboardCards\`"). Anything
    /// else stays refused.
    pub plugins: &'static [(&'static str, &'static [&'static str])],
}

/// **Closed.** Opening it is a reviewed code change to this constant (with
/// the review recorded in `publisher-review-request.md`), never a flag,
/// setting, key or environment variable; `the_privileged_gate_is_closed`
/// fails until that review updates it too.
pub(super) const PRIVILEGED_GATE: PrivilegedGate = PrivilegedGate::Closed;

/// What the gate's opening path admitted, and the keys that signed it.
#[derive(Debug)]
pub(super) struct Admitted {
    pub package: VerifiedPackage,
    pub signers: Vec<String>,
}

/// The gate's opening path, for an artifact the verifier refused only
/// because it declares privileged kinds (so its size, sha256, signatures
/// and revocations already passed: [`publisher::verify_release`] checks
/// those first). Closed: the refusal stands. Open: the review's
/// conditions (threshold, plugin, every privileged construct), the keys
/// that actually signed (a threshold of them, counted again here), the
/// registry's pins (manifest hash, permissions), then the bundled reader
/// rules (`verify_bundled_files`, the only reader that accepts the
/// privileged kinds), compatibility and the floor.
pub(super) fn admit_privileged(
    gate: PrivilegedGate,
    refusal: PublisherError,
    keys: &PublisherKeys,
    signatures: &[String],
    expected: &ExpectedPackage,
    package: &[u8],
    policy: &ReleasePolicy<'_>,
) -> Result<Admitted, PublisherError> {
    let PublisherError::PrivilegedGateClosed { .. } = &refusal else {
        return Err(refusal);
    };
    let PrivilegedGate::Open(review) = gate else {
        return Err(refusal);
    };
    if review.min_threshold < 2 || keys.threshold < review.min_threshold {
        return Err(refusal);
    }
    let Some((_, allowed)) = review
        .plugins
        .iter()
        .find(|(id, _)| *id == expected.id.as_str())
    else {
        return Err(refusal);
    };
    let comment = publisher::release_comment(&expected.id, &expected.version, &expected.package);
    let signers: Vec<String> = keys
        .keys
        .iter()
        .filter(|key| {
            signatures.iter().any(|signature| {
                deckpilot_plugin_registry::sig::verify_exact(key, signature, package, &comment)
                    .is_ok()
            })
        })
        .filter_map(|key| publisher::fingerprint(key))
        .collect();
    if u64::try_from(signers.len()).unwrap_or(0) < review.min_threshold.max(keys.threshold) {
        return Err(refusal);
    }
    let files = read_archive(package, policy.limits)?;
    check_registry_pins(expected, &files)?;
    let pinned = ExpectedFiles {
        id: expected.id.clone(),
        version: expected.version.clone(),
        files: digests(&files),
    };
    let verified = verify_bundled_files(files, &pinned, policy.limits)?;
    let constructs = deckpilot_plugin_store::privileged_constructs(verified.manifest());
    if constructs.iter().any(|c| !allowed.contains(&c.as_str())) {
        return Err(refusal);
    }
    let host = policy.host.ok_or(refusal)?;
    verified
        .manifest()
        .check_compatibility(host)
        .map_err(PublisherError::Incompatible)?;
    if let Some(floor) = policy.floor {
        let claim = ReleaseClaim {
            id: expected.id.clone(),
            version: expected.version.clone(),
            package: expected.package.clone(),
        };
        check_floor(&claim, floor, &digests(verified.files()))?;
    }
    Ok(Admitted {
        package: verified,
        signers,
    })
}

/// The registry's pins that `verify_package` checks, for the opening path
/// (which reads with the bundled rules instead): the manifest's sha256 and
/// its permissions, exactly as the consent dialog shows them.
fn check_registry_pins(expected: &ExpectedPackage, files: &Files) -> Result<(), PublisherError> {
    let manifest = files
        .get(deckpilot_plugin_package::MANIFEST)
        .ok_or_else(|| PublisherError::Package("manifest.json is missing".into()))?;
    if sha256(manifest) != expected.manifest_sha256 {
        return Err(PublisherError::Package(
            "manifest.json is not the one the registry pins".into(),
        ));
    }
    if let Some(permissions) = &expected.permissions {
        let value: serde_json::Value = serde_json::from_slice(manifest)
            .map_err(|e| PublisherError::Package(format!("manifest.json: {e}")))?;
        let declared = value
            .get("permissions")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        if declared != serde_json::Value::Array(permissions.clone()) {
            return Err(PublisherError::Package(
                "the manifest's permissions are not the ones the registry shows".into(),
            ));
        }
    }
    Ok(())
}

fn digests(files: &Files) -> BTreeMap<String, PackageDigest> {
    files
        .iter()
        .map(|(path, bytes)| (path.clone(), publisher::digest_of(bytes)))
        .collect()
}

/// The verifier's floor rule, for the gate's opening path.
fn check_floor(
    claim: &ReleaseClaim,
    floor: &ReleaseFloor,
    files: &BTreeMap<String, PackageDigest>,
) -> Result<(), PublisherError> {
    if claim.version < floor.version {
        return Err(PublisherError::Rollback {
            id: claim.id.clone(),
            version: claim.version.clone(),
            floor: floor.version.clone(),
        });
    }
    if claim.version == floor.version && *files != floor.files {
        return Err(PublisherError::SameVersionOtherFiles {
            id: claim.id.clone(),
            version: claim.version.clone(),
            files: "the files differ".into(),
        });
    }
    Ok(())
}

// --------------------------------------------------------------- floors

/// `floors.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FloorsFile {
    floors_version: u32,
    plugins: BTreeMap<PluginId, Floor>,
    /// Every release this agent verified, by plugin and version: the
    /// fingerprints of the keys that signed it. An installed registry copy
    /// of an official plugin whose version is not here is not run.
    #[serde(default)]
    releases: BTreeMap<PluginId, BTreeMap<Version, Vec<String>>>,
}

impl Default for FloorsFile {
    fn default() -> Self {
        Self {
            floors_version: FLOORS_VERSION,
            plugins: BTreeMap::new(),
            releases: BTreeMap::new(),
        }
    }
}

/// The highest release of a plugin this agent verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Floor {
    pub version: Version,
    pub package: PackageDigest,
    /// Every file by path.
    pub files: BTreeMap<String, PackageDigest>,
    /// Fingerprints of the publisher keys whose signature counted.
    pub signers: Vec<String>,
    pub verified_at: String,
}

impl Floor {
    fn release_floor(&self) -> ReleaseFloor {
        ReleaseFloor {
            version: self.version.clone(),
            files: self.files.clone(),
        }
    }
}

// -------------------------------------------------------------- refusals

/// Why an official release was refused. Nothing was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The publisher verifier (or the gate) refused it.
    Publisher(PublisherError),
    /// The package failed the registry's pins after the publisher checks.
    Package(String),
    /// This agent cannot verify official releases now (keys, lock, floors,
    /// host), with the reason.
    Unavailable(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Publisher(e) => write!(f, "{e}"),
            Self::Package(m) | Self::Unavailable(m) => f.write_str(m),
        }
    }
}

impl Refusal {
    /// As an IPC error: revocations and incompatibility keep their reasons,
    /// everything else is an invalid package.
    pub fn to_ipc(&self, id: &PluginId) -> IpcError {
        let why = match self {
            Self::Publisher(PublisherError::Revoked { .. } | PublisherError::RevokedKey { .. }) => {
                reason::REVOKED
            }
            Self::Publisher(PublisherError::Incompatible(_)) => reason::INCOMPATIBLE,
            _ => reason::INVALID_PACKAGE,
        };
        IpcError::new(
            ErrorCode::Failed,
            format!("{id} is not a verified DeckPilot release; nothing was installed: {self}"),
        )
        .with_reason(why)
    }
}

// ------------------------------------------------------------- official

/// One downloaded official release, as the registry delivered it.
#[derive(Debug)]
pub(super) struct Delivery<'a> {
    /// What the verified registry chain pins (id, version, size, sha256,
    /// manifest hash, permissions, `first_party`).
    pub expected: &'a ExpectedPackage,
    pub package: &'a [u8],
    /// The signature files' bytes (`<package>.minisig`).
    pub signatures: &'a [Vec<u8>],
    /// The bundled copy this agent pins for the id, if any (a floor).
    pub pin: Option<&'a Pin>,
    /// The running host; `None` refuses (the host is always checked).
    pub host: Option<&'a HostEnvironment>,
    pub now: DateTime<Utc>,
}

/// What the agent compiles in besides the keys: the artifact lock (its
/// releases are minimum versions, its `revoked` refuses releases) and the
/// floors of `released-pins.json` for the running platform.
#[derive(Debug, Clone, Default)]
pub(super) struct Compiled {
    pub revoked: Vec<RevokedRelease>,
    /// The highest version any DeckPilot release shipped on this platform,
    /// with its files.
    pub released: BTreeMap<PluginId, ReleaseFloor>,
    /// The lock's releases: never below them, the same version only with
    /// the same package.
    pub locked: BTreeMap<PluginId, ReleaseClaim>,
}

/// `released-pins.json`, only what the floors need.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleasedPins {
    released_pins_version: u32,
    releases: Vec<ReleasedRelease>,
}

#[derive(Deserialize)]
struct ReleasedRelease {
    platform: String,
    plugins: Vec<ReleasedPlugin>,
}

#[derive(Deserialize)]
struct ReleasedPlugin {
    id: PluginId,
    version: Version,
    files: BTreeMap<String, PackageDigest>,
}

impl Compiled {
    /// From the lock and released pins (JSON) for `platform` (none: no
    /// released floors; an unknown platform verifies nothing anyway).
    pub fn parse(
        lock: &[u8],
        released_pins: &[u8],
        platform: Option<deckpilot_plugin_manifest::Platform>,
    ) -> Result<Self, String> {
        let lock = ArtifactLock::from_json(lock)
            .map_err(|e| format!("the compiled artifact lock is invalid: {e}"))?;
        let pins: ReleasedPins = serde_json::from_slice(released_pins)
            .map_err(|e| format!("the compiled released pins are invalid: {e}"))?;
        if pins.released_pins_version != 1 {
            return Err(format!(
                "the compiled released pins have releasedPinsVersion {}",
                pins.released_pins_version
            ));
        }
        let name = platform.map(|p| match p {
            deckpilot_plugin_manifest::Platform::Macos => "macos",
            deckpilot_plugin_manifest::Platform::Windows => "windows",
            deckpilot_plugin_manifest::Platform::Linux => "linux",
        });
        let mut released: BTreeMap<PluginId, ReleaseFloor> = BTreeMap::new();
        for release in pins.releases {
            if Some(release.platform.as_str()) != name {
                continue;
            }
            for plugin in release.plugins {
                if released
                    .get(&plugin.id)
                    .is_none_or(|floor| floor.version < plugin.version)
                {
                    released.insert(
                        plugin.id,
                        ReleaseFloor {
                            version: plugin.version,
                            files: plugin.files,
                        },
                    );
                }
            }
        }
        let locked = lock
            .plugins
            .iter()
            .map(|artifact| (artifact.id.clone(), artifact.claim()))
            .collect();
        Ok(Self {
            revoked: lock.revoked,
            released,
            locked,
        })
    }

    /// This build's.
    fn this_build() -> Result<Self, String> {
        Self::parse(
            ARTIFACT_LOCK.as_bytes(),
            RELEASED_PINS.as_bytes(),
            deckpilot_plugin_manifest::Platform::current(),
        )
    }

    /// Never below the lock's release; at its version only its package.
    fn check_locked(&self, claim: &ReleaseClaim) -> Result<(), Refusal> {
        let Some(locked) = self.locked.get(&claim.id) else {
            return Ok(());
        };
        if claim.version < locked.version {
            return Err(Refusal::Publisher(PublisherError::Rollback {
                id: claim.id.clone(),
                version: claim.version.clone(),
                floor: locked.version.clone(),
            }));
        }
        if claim.version == locked.version && claim.package != locked.package {
            return Err(Refusal::Publisher(PublisherError::SameVersionOtherFiles {
                id: claim.id.clone(),
                version: claim.version.clone(),
                files: "the package is not the one the artifact lock pins".into(),
            }));
        }
        Ok(())
    }
}

/// The publisher keys, the compiled lock and floors, and the persisted
/// floors.
#[derive(Debug)]
pub(super) struct Official {
    keys: Result<PublisherKeys, String>,
    compiled: Result<Compiled, String>,
    dir: PathBuf,
    floors: Mutex<Result<FloorsFile, String>>,
    /// The floors file holds fields of a newer DeckPilot: read, never
    /// rewritten (see `read_floors`).
    floors_newer: Option<String>,
    gate: PrivilegedGate,
    /// [`MAX_AUDIT_BYTES`], smaller in tests.
    max_audit_bytes: u64,
}

impl Official {
    /// This agent's: the pinned publisher keys (in `test-hooks` builds, the
    /// file named by [`TEST_KEYS_ENV`] instead), the compiled lock's
    /// revocations, and the floors in `dir`.
    pub fn open(dir: &Path) -> Self {
        Self::with(dir, pinned_keys(), Compiled::this_build())
    }

    /// With the given keys and compiled data (tests).
    pub fn with(
        dir: &Path,
        keys: Result<PublisherKeys, String>,
        compiled: Result<Compiled, String>,
    ) -> Self {
        let (floors, floors_newer) = match read_floors(&dir.join(FLOORS_FILE)) {
            Ok((floors, newer)) => (Ok(floors), newer),
            Err(problem) => (Err(problem), None),
        };
        if let Err(problem) = &floors {
            warn!(%problem, "official plugin floors unreadable: official installs are refused");
        }
        if let Some(problem) = &floors_newer {
            warn!(%problem, "official installs are refused until a newer DeckPilot runs again");
        }
        Self {
            keys,
            compiled,
            dir: dir.to_path_buf(),
            floors: Mutex::new(floors),
            floors_newer,
            gate: PRIVILEGED_GATE,
            max_audit_bytes: MAX_AUDIT_BYTES,
        }
    }

    /// With another gate (tests of the opening path only).
    #[cfg(test)]
    pub fn with_gate(mut self, gate: PrivilegedGate) -> Self {
        self.gate = gate;
        self
    }

    /// With a smaller audit rotation size (tests of the rotation only).
    #[cfg(test)]
    pub fn with_audit_limit(mut self, bytes: u64) -> Self {
        self.max_audit_bytes = bytes;
        self
    }

    /// Whether an official release could be verified at all (before any
    /// download): valid and provisioned keys, a valid lock, readable floors.
    pub fn ready(&self) -> Result<(), String> {
        let keys = self.keys.as_ref().map_err(Clone::clone)?;
        keys.validate()
            .map_err(|e| format!("the publisher keys are invalid: {e}"))?;
        if !keys.is_provisioned() {
            return Err(PublisherError::NotProvisioned.to_string());
        }
        self.compiled.as_ref().map_err(Clone::clone)?;
        if let Some(problem) = &self.floors_newer {
            return Err(problem.clone());
        }
        self.lock_floors()
            .as_ref()
            .map(|_| ())
            .map_err(Clone::clone)
    }

    fn lock_floors(&self) -> std::sync::MutexGuard<'_, Result<FloorsFile, String>> {
        self.floors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The floor recorded for `id`, if any.
    #[cfg(test)]
    pub fn floor(&self, id: &PluginId) -> Option<Floor> {
        self.lock_floors()
            .as_ref()
            .ok()
            .and_then(|f| f.plugins.get(id).cloned())
    }

    /// The floor a release of `id` is checked against: the higher of the
    /// recorded one and the bundled pin. Equal versions must agree on the
    /// files.
    fn effective_floor(
        &self,
        id: &PluginId,
        pin: Option<&Pin>,
    ) -> Result<Option<ReleaseFloor>, Refusal> {
        let recorded = {
            let floors = self.lock_floors();
            let floors = floors
                .as_ref()
                .map_err(|e| Refusal::Unavailable(e.clone()))?;
            floors.plugins.get(id).map(Floor::release_floor)
        };
        let bundled = pin.map(|pin| ReleaseFloor {
            version: pin.version.clone(),
            files: pin.files.clone(),
        });
        let released = self
            .compiled
            .as_ref()
            .map_err(|e| Refusal::Unavailable(e.clone()))?
            .released
            .get(id)
            .cloned();
        let mut floor: Option<ReleaseFloor> = None;
        for (what, candidate) in [
            ("the recorded release", recorded),
            ("the bundled copy", bundled),
            ("a DeckPilot release (released-pins.json)", released),
        ] {
            let Some(candidate) = candidate else {
                continue;
            };
            floor = match floor {
                None => Some(candidate),
                Some(current) if current.version > candidate.version => Some(current),
                Some(current) if current.version < candidate.version => Some(candidate),
                Some(current) if current.files == candidate.files => Some(current),
                Some(current) => {
                    return Err(Refusal::Publisher(PublisherError::SameVersionOtherFiles {
                        id: id.clone(),
                        version: current.version,
                        files: format!("{what} has other files at that version"),
                    }));
                }
            };
        }
        Ok(floor)
    }

    /// Verifies `delivery` and returns the package the store may install.
    /// Every outcome is audited; a verified release raises the floor before
    /// this returns (a floor that cannot be saved refuses the release).
    pub fn verify(&self, delivery: &Delivery<'_>) -> Result<VerifiedPackage, Refusal> {
        let expected = delivery.expected;
        let claim = ReleaseClaim {
            id: expected.id.clone(),
            version: expected.version.clone(),
            package: expected.package.clone(),
        };
        let result = self.verify_inner(delivery, &claim);
        let record = match &result {
            Ok((_, signers)) => AuditRecord {
                at: delivery.now,
                outcome: publisher::AuditOutcome::Verified,
                id: claim.id.to_string(),
                version: claim.version.to_string(),
                sha256: claim.package.sha256.to_string(),
                size: claim.package.size,
                signers: signers.clone(),
                reason: None,
            },
            Err(refusal) => {
                let mut record = AuditRecord::refused(
                    &claim,
                    &PublisherError::Package(String::new()),
                    delivery.now,
                );
                record.reason = Some(refusal.to_string());
                record
            }
        };
        match &result {
            Ok(_) => info!(plugin = %claim.id, version = %claim.version,
                signers = ?record.signers, "official release verified"),
            Err(refusal) => warn!(plugin = %claim.id, version = %claim.version,
                %refusal, "official release refused"),
        }
        self.audit(&record);
        result.map(|(package, _)| package)
    }

    fn verify_inner(
        &self,
        delivery: &Delivery<'_>,
        claim: &ReleaseClaim,
    ) -> Result<(VerifiedPackage, Vec<String>), Refusal> {
        let keys = self
            .keys
            .as_ref()
            .map_err(|e| Refusal::Unavailable(e.clone()))?;
        let compiled = self
            .compiled
            .as_ref()
            .map_err(|e| Refusal::Unavailable(e.clone()))?;
        let revoked = &compiled.revoked;
        let host = delivery.host.ok_or_else(|| {
            Refusal::Unavailable("the running DeckPilot's version or platform is unknown".into())
        })?;
        if !delivery.expected.first_party {
            return Err(Refusal::Package(
                "the registry does not authorize the deckpilot namespace for this package".into(),
            ));
        }
        let signatures = delivery
            .signatures
            .iter()
            .map(|bytes| publisher::read_signature(bytes))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Refusal::Publisher)?;
        compiled.check_locked(claim)?;
        let floor = self.effective_floor(&claim.id, delivery.pin)?;
        let limits = Limits::default();
        let policy = ReleasePolicy {
            host: Some(host),
            floor: floor.as_ref(),
            revoked,
            limits: &limits,
        };
        let (package, files, signers) =
            match publisher::verify_release(delivery.package, &signatures, keys, claim, &policy) {
                Ok(release) => {
                    let package = Self::registry_package(delivery, &release)?;
                    (package, release.file_digests(), release.signers().to_vec())
                }
                Err(refusal @ PublisherError::PrivilegedGateClosed { .. }) => {
                    let admitted = admit_privileged(
                        self.gate,
                        refusal,
                        keys,
                        &signatures,
                        delivery.expected,
                        delivery.package,
                        &policy,
                    )
                    .map_err(Refusal::Publisher)?;
                    let files = digests(admitted.package.files());
                    (admitted.package, files, admitted.signers)
                }
                Err(refusal) => return Err(Refusal::Publisher(refusal)),
            };
        self.record_release(claim, files, &signers, delivery.now)?;
        Ok((package, signers))
    }

    /// The verified release checked once more as the registry pins it
    /// (manifest hash, the permissions the consent dialog shows), with the
    /// first-party rules: the store installs only a [`VerifiedPackage`].
    fn registry_package(
        delivery: &Delivery<'_>,
        release: &VerifiedRelease,
    ) -> Result<VerifiedPackage, Refusal> {
        let mut expected = delivery.expected.clone();
        expected.first_party = true;
        let package = verify_package(delivery.package, &expected, &Limits::default())
            .map_err(|e: PackageError| Refusal::Package(e.to_string()))?;
        if package.manifest() != release.manifest() {
            return Err(Refusal::Package(
                "the package read twice gave two manifests".into(),
            ));
        }
        Ok(package)
    }

    /// Records the verified release (its signers, for run-time revocation)
    /// and raises the floor if it is higher. Saved before this returns; a
    /// record that cannot be saved refuses the release.
    fn record_release(
        &self,
        claim: &ReleaseClaim,
        files: BTreeMap<String, PackageDigest>,
        signers: &[String],
        now: DateTime<Utc>,
    ) -> Result<(), Refusal> {
        if let Some(problem) = &self.floors_newer {
            return Err(Refusal::Unavailable(problem.clone()));
        }
        let mut floors = self.lock_floors();
        let floors = floors
            .as_mut()
            .map_err(|e| Refusal::Unavailable(e.clone()))?;
        let mut next = floors.clone();
        next.releases
            .entry(claim.id.clone())
            .or_default()
            .insert(claim.version.clone(), signers.to_vec());
        if next
            .plugins
            .get(&claim.id)
            .is_none_or(|floor| floor.version < claim.version)
        {
            next.plugins.insert(
                claim.id.clone(),
                Floor {
                    version: claim.version.clone(),
                    package: claim.package.clone(),
                    files,
                    signers: signers.to_vec(),
                    verified_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                },
            );
        }
        if next == *floors {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(&next)
            .map_err(|e| Refusal::Unavailable(format!("the floors: {e}")))?;
        write_atomic(&self.dir.join(FLOORS_FILE), &bytes).map_err(|e| {
            Refusal::Unavailable(format!(
                "the rollback floor of {} cannot be saved: {e}",
                claim.id
            ))
        })?;
        *floors = next;
        Ok(())
    }

    /// Audits a refusal that happened before a release could be verified
    /// (`ready` refused before the download, or the signature could not be
    /// downloaded): what is known of the release, and why.
    pub fn refused_before_verification(
        &self,
        id: &PluginId,
        version: Option<&Version>,
        package: Option<&PackageDigest>,
        why: &str,
        now: DateTime<Utc>,
    ) {
        warn!(plugin = %id, version = ?version.map(ToString::to_string), %why,
            "official release refused before verification");
        self.audit(&AuditRecord {
            at: now,
            outcome: publisher::AuditOutcome::Refused,
            id: id.to_string(),
            version: version.map(ToString::to_string).unwrap_or_default(),
            sha256: package.map(|p| p.sha256.to_string()).unwrap_or_default(),
            size: package.map_or(0, |p| p.size),
            signers: Vec::new(),
            reason: Some(why.to_owned()),
        });
    }

    /// Appends `record` to the audit log, hash-chained: each line is
    /// `{"prev": <sha256 of the previous line>, "record": AuditRecord}`, the
    /// first line of a log chains to the last line of the newest rotated one
    /// (`audit.jsonl.1`; above [`MAX_AUDIT_BYTES`] the log is rotated and
    /// [`AUDIT_GENERATIONS`] old files are kept, the oldest dropped) or to
    /// [`GENESIS`]. Editing, removing or reordering a line breaks the chain
    /// ([`check_audit_chain`]).
    ///
    /// Limits (`publisher-signature.md`): tamper-evident, not tamper-proof.
    /// Whoever can write the data directory can rewrite or truncate the
    /// whole chain, or delete the files; the last line is anchored only by
    /// its hash in the agent's log (`official audit line`), which the same
    /// user can edit; the oldest generation's first line chains to a file
    /// that is gone. A failure to write is logged, never fatal.
    fn audit(&self, record: &AuditRecord) {
        let path = self.dir.join(AUDIT_FILE);
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            let generation = |n: usize| self.dir.join(format!("{AUDIT_FILE}.{n}"));
            let rotated = generation(1);
            if std::fs::metadata(&path).is_ok_and(|m| m.len() > self.max_audit_bytes) {
                for n in (1..AUDIT_GENERATIONS).rev() {
                    if generation(n).exists() {
                        std::fs::rename(generation(n), generation(n + 1))?;
                    }
                }
                std::fs::rename(&path, &rotated)?;
            }
            let prev = last_line_hash(&path)?
                .or(last_line_hash(&rotated)?)
                .unwrap_or_else(|| GENESIS.to_owned());
            let line = ChainedLine {
                prev,
                record: record.clone(),
            };
            let text = serde_json::to_string(&line).map_err(std::io::Error::other)?;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            file.write_all(format!("{text}\n").as_bytes())?;
            file.sync_data()?;
            // The head of the chain, in the agent's log too: the anchor a
            // reviewer compares the file's last line against.
            info!(hash = %sha256(text.as_bytes()), "official audit line");
            Ok(())
        })();
        if let Err(e) = result {
            warn!(error = %e, path = %path.display(), "the official plugin audit log cannot be written");
        }
    }

    /// [`Official::withheld`] without bundled copies to check against
    /// (every retained version needs a record of its own). Tests only.
    #[cfg(test)]
    pub fn revoked_installed(
        &self,
        records: &[PluginRecord],
    ) -> BTreeMap<(PluginId, Version), Withheld> {
        self.withheld(records, &|_, _| BundledMatch::NotPinned)
    }

    /// The installed versions of registry copies of official plugins this
    /// agent must not run, with the reason:
    ///
    /// - [`WithheldKind::Revoked`]: the compiled lock revokes the release,
    ///   or the release this agent recorded for it was signed by a key that
    ///   is now revoked;
    /// - [`WithheldKind::Unverified`]: nothing vouches for it. No record of
    ///   verifying it (a deleted or reset floors file), records that cannot
    ///   be read, compiled data that is invalid, or a retained copy that
    ///   claims to be the bundled version but whose files differ from the
    ///   pins (fail closed).
    ///
    /// A version whose installed files are exactly the bundled copy this
    /// agent pins (`bundled`: typically the bundled version retained for
    /// rollback after the update to a registry release) is trusted like any
    /// bundled copy. Bundled installs are never affected.
    pub fn withheld(
        &self,
        records: &[PluginRecord],
        bundled: &dyn Fn(&PluginRecord, &VersionRecord) -> BundledMatch,
    ) -> BTreeMap<(PluginId, Version), Withheld> {
        let revoked_keys: BTreeSet<String> = self
            .keys
            .as_ref()
            .map(|k| {
                k.revoked_keys
                    .iter()
                    .filter_map(|key| publisher::fingerprint(key))
                    .collect()
            })
            .unwrap_or_default();
        let compiled = self.compiled.as_ref();
        let floors = self.lock_floors();
        let mut out = BTreeMap::new();
        for record in records {
            if !matches!(record.source, InstallSource::Registry { .. })
                || !deckpilot_plugin_store::is_first_party(&record.id)
            {
                continue;
            }
            for installed in &record.versions {
                let version = &installed.version;
                let key = (record.id.clone(), version.clone());
                let not_bundled = match bundled(record, installed) {
                    BundledMatch::Matches => continue,
                    BundledMatch::Differs(why) => Some(why),
                    BundledMatch::NotPinned => None,
                };
                let compiled = match compiled {
                    Ok(compiled) => compiled,
                    Err(problem) => {
                        out.insert(
                            key,
                            Withheld::unverified(format!("this agent cannot check it: {problem}")),
                        );
                        continue;
                    }
                };
                if let Some(r) = compiled
                    .revoked
                    .iter()
                    .find(|r| r.id == record.id && r.versions.contains(version))
                {
                    out.insert(
                        key,
                        Withheld::revoked(format!(
                            "the DeckPilot release {version} is revoked: {}",
                            r.reason
                        )),
                    );
                    continue;
                }
                let floors = match floors.as_ref() {
                    Ok(floors) => floors,
                    Err(problem) => {
                        out.insert(
                            key,
                            Withheld::unverified(format!(
                                "this agent cannot read its record of the releases: {problem}"
                            )),
                        );
                        continue;
                    }
                };
                match floors.releases.get(&record.id).and_then(|r| r.get(version)) {
                    None => {
                        let why = match not_bundled {
                            Some(why) => format!(
                                "the installed copy of {version} is not the bundled copy this \
                                 agent pins ({why}), and this agent has no record of verifying \
                                 it as a DeckPilot release"
                            ),
                            None => format!(
                                "this agent has no record of verifying {version} as a DeckPilot \
                                 release, and no bundled copy of {version} to check it against"
                            ),
                        };
                        out.insert(key, Withheld::unverified(why));
                    }
                    Some(signers) if signers.iter().any(|s| revoked_keys.contains(s)) => {
                        out.insert(
                            key,
                            Withheld::revoked(format!(
                                "the DeckPilot release {version} was signed by a revoked \
                                 publisher key"
                            )),
                        );
                    }
                    Some(_) => {}
                }
            }
        }
        out
    }
}

/// Whether an installed version is the bundled copy this agent pins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BundledMatch {
    /// This agent pins no bundled copy of that version.
    NotPinned,
    /// Its installed files are exactly the pinned ones.
    Matches,
    /// Pinned, but the installed files differ (why).
    Differs(String),
}

/// Checks the installed files of `version` against `bundle`'s pin of it.
pub(super) fn bundled_match(
    bundle: &super::bundled::Bundle,
    layout: &deckpilot_plugin_store::Layout,
    record: &PluginRecord,
    version: &VersionRecord,
) -> BundledMatch {
    let Some(pin) = bundle.pins.pinned(&record.id, &version.version) else {
        return BundledMatch::NotPinned;
    };
    match deckpilot_plugin_bundled::verify_installed(&layout.resolve(&version.install_dir), pin) {
        Ok(()) => BundledMatch::Matches,
        Err(e) => BundledMatch::Differs(e.to_string()),
    }
}

/// Why an installed version is not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WithheldKind {
    /// Revoked (by the lock or a revoked publisher key).
    Revoked,
    /// Nothing vouches for it.
    Unverified,
}

/// An installed version this agent does not run, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Withheld {
    pub kind: WithheldKind,
    pub reason: String,
}

impl Withheld {
    fn revoked(reason: String) -> Self {
        Self {
            kind: WithheldKind::Revoked,
            reason,
        }
    }

    fn unverified(reason: String) -> Self {
        Self {
            kind: WithheldKind::Unverified,
            reason,
        }
    }

    /// The IPC reason code: `revoked`, or `invalidPackage` for a copy
    /// nothing vouches for.
    pub fn ipc_reason(&self) -> &'static str {
        match self.kind {
            WithheldKind::Revoked => reason::REVOKED,
            WithheldKind::Unverified => reason::INVALID_PACKAGE,
        }
    }

    /// The state reason a quarantine records.
    pub fn state_reason(&self) -> String {
        match self.kind {
            WithheldKind::Revoked => format!("revoked: {}", self.reason),
            WithheldKind::Unverified => format!("not verified: {}", self.reason),
        }
    }
}

fn pinned_keys() -> Result<PublisherKeys, String> {
    #[cfg(feature = "test-hooks")]
    if let Some(path) = std::env::var_os(TEST_KEYS_ENV).filter(|p| !p.is_empty()) {
        return std::fs::read(&path)
            .map_err(|e| format!("{TEST_KEYS_ENV}: {e}"))
            .and_then(|bytes| PublisherKeys::from_json(&bytes).map_err(|e| e.to_string()));
    }
    PublisherKeys::pinned().map_err(|e| e.to_string())
}

/// The floors file, and whether it holds fields this agent does not know
/// (`Some(why)`: written by a newer DeckPilot).
///
/// Unknown fields are tolerated for **reading**: every floor and every
/// signer record this agent knows is used as it is, so a downgraded agent
/// keeps checking revocations and running the copies it can vouch for.
/// Such a file is never **rewritten** by this agent (official installs
/// are refused while it is), because rewriting would drop what the newer
/// agent recorded. Another `floorsVersion` is refused entirely (fail
/// closed): a newer format may change what the known fields mean.
fn read_floors(path: &Path) -> Result<(FloorsFile, Option<String>), String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((FloorsFile::default(), None));
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let invalid =
        |e: serde_json::Error| format!("{} is invalid ({e}); it is never reset", path.display());
    let raw: serde_json::Value = serde_json::from_slice(&bytes).map_err(invalid)?;
    let floors: FloorsFile = serde_json::from_value(raw.clone()).map_err(invalid)?;
    if floors.floors_version != FLOORS_VERSION {
        return Err(format!(
            "{} has floorsVersion {}; this agent reads {FLOORS_VERSION}",
            path.display(),
            floors.floors_version
        ));
    }
    let known = serde_json::to_value(&floors).map_err(|e| e.to_string())?;
    let newer = (!covered(&raw, &known)).then(|| {
        format!(
            "{} holds records of a newer DeckPilot; this agent reads it but never rewrites it",
            path.display()
        )
    });
    Ok((floors, newer))
}

/// Whether everything in `original` is in `known` (objects by key,
/// recursively; arrays and values exactly): `known` may add defaults.
fn covered(original: &serde_json::Value, known: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (original, known) {
        (Value::Object(a), Value::Object(b)) => a
            .iter()
            .all(|(key, value)| b.get(key).is_some_and(|other| covered(value, other))),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| covered(x, y))
        }
        (a, b) => a == b,
    }
}

/// The `prev` of the first line of a new audit chain.
pub(super) const GENESIS: &str = "genesis";

/// One line of `audit.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChainedLine {
    pub prev: String,
    pub record: AuditRecord,
}

/// The sha256 of the last line of `path` (without its newline), if any.
fn last_line_hash(path: &Path) -> std::io::Result<Option<String>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(text
        .lines()
        .rfind(|l| !l.is_empty())
        .map(|line| sha256(line.as_bytes()).to_string()))
}

/// Checks the hash chain of the audit log `text` (one file), starting at
/// `first_prev`; returns the hash of its last line, or the 1-based number
/// of the first line that does not chain. (For tests and reviewers; the
/// agent only appends.)
#[cfg(test)]
pub(super) fn check_audit_chain(text: &str, first_prev: &str) -> Result<String, usize> {
    let mut prev = first_prev.to_owned();
    for (n, line) in text.lines().filter(|l| !l.is_empty()).enumerate() {
        let parsed: ChainedLine = serde_json::from_str(line).map_err(|_| n + 1)?;
        if parsed.prev != prev {
            return Err(n + 1);
        }
        prev = sha256(line.as_bytes()).to_string();
    }
    Ok(prev)
}

/// The registry's verdicts with the official revocations on top.
pub(super) struct Verdicts<'a> {
    pub official: &'a BTreeMap<(PluginId, Version), Withheld>,
    pub registry: &'a dyn VerdictSource,
}

impl VerdictSource for Verdicts<'_> {
    fn verdict(&self, id: &PluginId, version: &Version) -> InstalledVerdict {
        match self.official.get(&(id.clone(), version.clone())) {
            Some(withheld) => InstalledVerdict::Disable {
                reason: withheld.reason.clone(),
                advisory: None,
            },
            None => self.registry.verdict(id, version),
        }
    }
}

// ----------------------------------------------------------- transition

/// The bundled → Marketplace-managed transition keeps what the user's
/// configuration refers to: every action id (keys, profiles, layouts,
/// projects and workflows bind them), every setting and every credential
/// slot of the bundled version must still be declared. Grants are the
/// store's diff (kept where still declared alike, new ones need consent).
pub(super) fn check_transition(bundled: &Manifest, next: &Manifest) -> Result<(), String> {
    let missing = |old: Vec<&str>, new: Vec<&str>, what: &str| -> Option<String> {
        let new: BTreeSet<&str> = new.into_iter().collect();
        let gone: Vec<&str> = old.into_iter().filter(|k| !new.contains(k)).collect();
        (!gone.is_empty()).then(|| format!("{what} {}", gone.join(", ")))
    };
    let c = (&bundled.contributes, &next.contributes);
    let problems: Vec<String> = [
        missing(
            c.0.actions().iter().map(|a| a.id.as_str()).collect(),
            c.1.actions().iter().map(|a| a.id.as_str()).collect(),
            "actions",
        ),
        missing(
            c.0.settings().iter().map(|s| s.key.as_str()).collect(),
            c.1.settings().iter().map(|s| s.key.as_str()).collect(),
            "settings",
        ),
        missing(
            c.0.credentials().iter().map(|s| s.key.as_str()).collect(),
            c.1.credentials().iter().map(|s| s.key.as_str()).collect(),
            "credential slots",
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} {} would drop what the bundled {} declares ({}); the bundled copy stays",
            next.id,
            next.version,
            bundled.version,
            problems.join("; ")
        ))
    }
}

/// The sha256 of `bytes` (tests).
#[cfg(test)]
fn digest(bytes: &[u8]) -> PackageDigest {
    PackageDigest {
        sha256: sha256(bytes),
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
    }
}

#[cfg(test)]
mod tests;
