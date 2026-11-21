//! `release sign` and `release verify`: the first-party publisher signature
//! of a `.dpplugin` release artifact (`crate::publisher`,
//! `docs/marketplace/publisher-signature.md`).
//!
//! `release sign` runs only in `knittingcode/deckpilot-plugins`' release
//! environment, with the publisher key the owner provisions as a secret
//! (or with a throwaway key in CI). `release verify` is the check an
//! application build runs before it stages an artifact as a bundled
//! plugin: with the pinned keys unless `--keys` names others (CI with
//! throwaway keys; the output then says so).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;

use chrono::{DateTime, Utc};
use deckpilot_plugin_manifest::{
    ApiLevel, FIRST_PARTY_PUBLISHER, HostEnvironment, PackageDigest, Platform, PluginId,
};
use deckpilot_plugin_package::Limits;
use semver::Version;
use serde::Deserialize;

use super::keys::SigningKey;
use super::{PublishError, Result, fail};
use crate::publisher::{
    self, AuditRecord, PublisherError, PublisherKeys, ReleaseClaim, ReleaseFloor, ReleasePolicy,
    RevokedRelease,
};

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

/// Step 1 of `release sign`, before the secret key is loaded: the claim is
/// the given `id` and `version` (from the release tag, never from the
/// package) with the package's digest, and the package passes every check
/// [`publisher::verify_release`] applies without a signature (first-party
/// rules, the gate, id and version equal to the claim). Nothing that could
/// never verify is signed, and no archive is parsed while the key is in
/// memory.
pub fn preflight(package: &[u8], id: PluginId, version: Version) -> Result<ReleaseClaim> {
    let claim = ReleaseClaim {
        id,
        version,
        package: publisher::digest_of(package),
    };
    publisher::check_release_package(package, &claim, &Limits::default())
        .map_err(|e| PublishError(format!("refusing to sign: {e}")))?;
    Ok(claim)
}

/// Step 2: the signature of exactly the claimed bytes. Only hashing: the
/// package is not parsed here.
pub fn sign(package: &[u8], claim: &ReleaseClaim, key: &SigningKey) -> Result<String> {
    if publisher::digest_of(package) != claim.package {
        return fail("the package changed since the preflight");
    }
    let comment = publisher::release_comment(&claim.id, &claim.version, &claim.package);
    key.sign(package, &comment)
}

/// Step 3, after the secret key is dropped: the signature verifies with
/// the verifier's rules against `public_key` alone (the host's
/// compatibility is the application build's check: it knows the host).
pub fn check_signed(
    package: &[u8],
    signature: &str,
    public_key: &str,
    claim: &ReleaseClaim,
) -> Result<()> {
    let keys = PublisherKeys {
        publisher_keys_version: publisher::KEYS_VERSION,
        publisher: FIRST_PARTY_PUBLISHER.to_owned(),
        keys: vec![public_key.to_owned()],
        threshold: 1,
        revoked_keys: Vec::new(),
    };
    let limits = Limits::default();
    let policy = ReleasePolicy {
        host: None,
        floor: None,
        revoked: &[],
        limits: &limits,
    };
    publisher::verify_release(package, &[signature.to_owned()], &keys, claim, &policy)
        .map(|_| ())
        .map_err(|e| PublishError(format!("the new signature does not verify: {e}")))
}

/// `released-pins.json` (format 1), only what the floor needs.
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

/// The highest version of `id` any release recorded for `platform`.
pub fn floor_from_released_pins(
    bytes: &[u8],
    id: &PluginId,
    platform: Platform,
) -> Result<Option<ReleaseFloor>> {
    let pins: ReleasedPins =
        serde_json::from_slice(bytes).map_err(|e| PublishError(format!("released pins: {e}")))?;
    if pins.released_pins_version != 1 {
        return fail(format!(
            "released pins: releasedPinsVersion {} (only 1)",
            pins.released_pins_version
        ));
    }
    let platform = platform_name(platform);
    Ok(pins
        .releases
        .into_iter()
        .filter(|r| r.platform == platform)
        .flat_map(|r| r.plugins)
        .filter(|p| &p.id == id)
        .max_by(|a, b| a.version.cmp(&b.version))
        .map(|p| ReleaseFloor {
            version: p.version,
            files: p.files,
        }))
}

fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Macos => "macos",
        Platform::Windows => "windows",
        Platform::Linux => "linux",
    }
}

/// `macos`, `windows` or `linux`.
pub fn parse_platform(text: &str) -> Result<Platform> {
    match text {
        "macos" => Ok(Platform::Macos),
        "windows" => Ok(Platform::Windows),
        "linux" => Ok(Platform::Linux),
        other => fail(format!("--platform `{other}`: macos, windows or linux")),
    }
}

/// The `revoked` list of an artifact lock (any JSON object with one).
pub fn revocations_of(bytes: &[u8]) -> Result<Vec<RevokedRelease>> {
    #[derive(Deserialize)]
    struct WithRevoked {
        #[serde(default)]
        revoked: Vec<RevokedRelease>,
    }
    serde_json::from_slice::<WithRevoked>(bytes)
        .map(|w| w.revoked)
        .map_err(|e| PublishError(format!("revocations: {e}")))
}

/// What `release verify` needs besides the files.
#[derive(Debug)]
pub struct VerifyRequest<'a> {
    pub package: &'a Path,
    /// One signature file per key (a threshold of them must verify).
    pub signatures: Vec<&'a Path>,
    /// `None`: the pinned keys.
    pub keys: Option<&'a Path>,
    pub claim: ReleaseClaim,
    pub host: HostEnvironment,
    pub released_pins: Option<&'a Path>,
    pub revocations: Option<&'a Path>,
    pub audit: Option<&'a Path>,
    pub unpack: Option<&'a Path>,
    pub now: DateTime<Utc>,
}

/// Runs the verification, appends the audit record (accepted or refused)
/// when asked, unpacks the files when asked, and returns the message.
pub fn verify(request: &VerifyRequest<'_>) -> Result<String> {
    let outcome = verify_inner(request);
    if let Some(audit) = request.audit {
        let record = match &outcome {
            Ok(verified) => verified.audit(request.now),
            Err(Refusal::Publisher(error)) => {
                AuditRecord::refused(&request.claim, error, request.now)
            }
            Err(Refusal::Tool(error)) => AuditRecord::refused(
                &request.claim,
                &PublisherError::Package(error.0.clone()),
                request.now,
            ),
        };
        append(audit, record.to_line().as_bytes())?;
    }
    let verified = outcome.map_err(|refusal| match refusal {
        Refusal::Publisher(error) => PublishError(format!(
            "{} {} refused: {error}",
            request.claim.id, request.claim.version
        )),
        Refusal::Tool(error) => error,
    })?;
    if let Some(dir) = request.unpack {
        unpack(dir, verified.files())?;
    }
    let keys_note = match request.keys {
        Some(path) => format!(" (keys from {}, not the pinned keys)", path.display()),
        None => String::new(),
    };
    Ok(format!(
        "{} {} verified: sha256 {}, {} bytes, signed by {}{keys_note}; authenticity only, no \
         permission is granted by it",
        request.claim.id,
        request.claim.version,
        request.claim.package.sha256,
        request.claim.package.size,
        verified.signers().join(", ")
    ))
}

enum Refusal {
    Publisher(PublisherError),
    Tool(PublishError),
}

impl From<PublishError> for Refusal {
    fn from(error: PublishError) -> Self {
        Self::Tool(error)
    }
}

fn verify_inner(request: &VerifyRequest<'_>) -> Result<publisher::VerifiedRelease, Refusal> {
    let keys = match request.keys {
        Some(path) => PublisherKeys::from_json(&read(path)?),
        None => PublisherKeys::pinned(),
    }
    .map_err(Refusal::Publisher)?;
    let package = read(request.package)?;
    let signatures = request
        .signatures
        .iter()
        .map(|path| {
            read(path)
                .map_err(Refusal::Tool)
                .and_then(|bytes| publisher::read_signature(&bytes).map_err(Refusal::Publisher))
        })
        .collect::<Result<Vec<_>, Refusal>>()?;
    let floor = request
        .released_pins
        .map(|path| {
            floor_from_released_pins(&read(path)?, &request.claim.id, request.host.platform)
        })
        .transpose()?
        .flatten();
    let revoked = request
        .revocations
        .map(|path| revocations_of(&read(path)?))
        .transpose()?
        .unwrap_or_default();
    let limits = Limits::default();
    let policy = ReleasePolicy {
        host: Some(&request.host),
        floor: floor.as_ref(),
        revoked: &revoked,
        limits: &limits,
    };
    publisher::verify_release(&package, &signatures, &keys, &request.claim, &policy)
        .map_err(Refusal::Publisher)
}

fn append(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

/// Writes every file under `dir`, which must not exist yet.
fn unpack(dir: &Path, files: &deckpilot_plugin_package::Files) -> Result<()> {
    if dir.exists() {
        return fail(format!(
            "{} already exists; refusing to unpack into it",
            dir.display()
        ));
    }
    for (path, bytes) in files {
        // Package paths passed the reader's path rules: relative, no `..`.
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| PublishError(format!("{}: {e}", parent.display())))?;
        }
        std::fs::write(&target, bytes)
            .map_err(|e| PublishError(format!("{}: {e}", target.display())))?;
    }
    Ok(())
}

/// The host of `release verify`: `--app-version`, `--plugin-api`,
/// `--platform` (default: this computer's).
pub fn host(
    app_version: Version,
    plugin_api: &str,
    platform: Option<Platform>,
) -> Result<HostEnvironment> {
    let plugin_api = ApiLevel::parse(plugin_api)
        .ok_or_else(|| PublishError(format!("--plugin-api `{plugin_api}`: MAJOR.MINOR")))?;
    let platform = platform
        .or_else(Platform::current)
        .ok_or_else(|| PublishError("--platform is required on this system".into()))?;
    Ok(HostEnvironment {
        app_version,
        plugin_api,
        platform,
    })
}
