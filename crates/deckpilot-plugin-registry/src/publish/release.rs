//! `release sign` and `release verify`: the first-party publisher signature
//! of a `.dpplugin` release artifact (`crate::publisher`,
//! `docs/marketplace/publisher-signature.md`).
//!
//! `release sign` runs only in `knittingcode/deckpilot-plugins`' release
//! environment, with one publisher key the owner provisions as a secret
//! (or with a throwaway key in CI), once per key (each key in its own job),
//! and writes that key's file `<package>.<KEYID>.minisig`. `release verify`
//! is the check an application build runs before it stages an artifact as
//! a bundled plugin: it reads every `<package>.*.minisig`, reports which
//! keys counted, and enforces the threshold of the pinned keys unless
//! `--keys` names others (CI with throwaway keys; the output then says so).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

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
    RevokedRelease, SignatureCheck, SignatureFile,
};

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| PublishError(format!("{}: {e}", path.display())))
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| PublishError(format!("{}: no file name", path.display())))
}

/// Where `release sign` writes the signature of `key_id` for `package`:
/// `<dir>/<package file name>.<KEYID>.minisig`, `dir` the package's own
/// directory unless given.
pub fn signature_path(package: &Path, dir: Option<&Path>, key_id: &str) -> Result<PathBuf> {
    let name = publisher::signature_file_name(&file_name(package)?, key_id);
    let dir = dir.map_or_else(
        || package.parent().map(Path::to_path_buf).unwrap_or_default(),
        Path::to_path_buf,
    );
    Ok(dir.join(name))
}

/// Every signature file of `package` in `dir` (the package's directory
/// unless given): the files named `<package file name>.<KEYID>.minisig`,
/// sorted by name, each read up to one byte over the signature limit (the
/// verifier refuses what is larger). The old single name
/// `<package>.minisig` is not read.
pub fn signature_files(
    package: &Path,
    dir: Option<&Path>,
) -> Result<Vec<(PathBuf, SignatureFile)>> {
    let package_name = file_name(package)?;
    let dir = dir.map_or_else(
        || package.parent().map(Path::to_path_buf).unwrap_or_default(),
        Path::to_path_buf,
    );
    let shown = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir.as_path()
    };
    let entries =
        std::fs::read_dir(shown).map_err(|e| PublishError(format!("{}: {e}", shown.display())))?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| PublishError(format!("{}: {e}", shown.display())))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(key_id) = publisher::key_id_in_file_name(&package_name, &name) else {
            continue;
        };
        let path = entry.path();
        let limit = u64::try_from(publisher::MAX_SIGNATURE_BYTES).unwrap_or(u64::MAX) + 1;
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .and_then(|file| {
                std::io::Read::read_to_end(&mut std::io::Read::take(file, limit), &mut bytes)
            })
            .map_err(|e| PublishError(format!("{}: {e}", path.display())))?;
        found.push((
            path,
            SignatureFile {
                key_id: key_id.to_owned(),
                bytes,
            },
        ));
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
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

/// Step 3, after the secret key is dropped: the signature, as the file
/// named for `public_key`'s key id, verifies with the verifier's rules
/// against `public_key` alone (the host's compatibility is the application
/// build's check: it knows the host).
pub fn check_signed(
    package: &[u8],
    signature: &str,
    public_key: &str,
    claim: &ReleaseClaim,
) -> Result<()> {
    let key_id = publisher::key_id(public_key)
        .ok_or_else(|| PublishError("the signing key has no key id".into()))?;
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
    let file = SignatureFile {
        key_id,
        bytes: format!("{signature}\n").into_bytes(),
    };
    publisher::verify_release(package, &[file], &keys, claim, &policy)
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
    /// Where the signature files are (`<package>.<KEYID>.minisig`, one per
    /// key, a threshold of them must verify): the package's directory
    /// unless given.
    pub signatures_dir: Option<&'a Path>,
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
/// when asked, unpacks the files when asked, and returns the message: the
/// outcome, then one line per signature file found (counted or ignored,
/// with the reason), accepted or refused.
pub fn verify(request: &VerifyRequest<'_>) -> Result<String> {
    let keys_note = match request.keys {
        Some(path) => format!("the keys in {} (not the pinned keys)", path.display()),
        None => "the pinned keys".to_owned(),
    };
    let inputs = inputs(request);
    let outcome = inputs
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|inputs| verify_inputs(request, inputs));
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
    let listing = inputs
        .as_ref()
        .map(|i| listing(request, i))
        .unwrap_or_default();
    let verified = outcome.map_err(|refusal| match refusal {
        Refusal::Publisher(error) => PublishError(format!(
            "{} {} refused (threshold of {keys_note}): {error}{listing}",
            request.claim.id, request.claim.version,
        )),
        Refusal::Tool(error) => error,
    })?;
    if let Some(dir) = request.unpack {
        unpack(dir, verified.files())?;
    }
    let threshold = inputs.map(|i| i.keys.threshold).unwrap_or_default();
    Ok(format!(
        "{} {} verified: sha256 {}, {} bytes, signed by {} distinct publisher key(s), threshold \
         {threshold} of {keys_note}; authenticity only, no permission is granted by it{listing}",
        request.claim.id,
        request.claim.version,
        request.claim.package.sha256,
        request.claim.package.size,
        verified.signers().len(),
    ))
}

#[derive(Clone)]
enum Refusal {
    Publisher(PublisherError),
    Tool(PublishError),
}

impl From<PublishError> for Refusal {
    fn from(error: PublishError) -> Self {
        Self::Tool(error)
    }
}

/// The keys, the package and its signature files.
struct Inputs {
    keys: PublisherKeys,
    package: Vec<u8>,
    files: Vec<(PathBuf, SignatureFile)>,
}

fn inputs(request: &VerifyRequest<'_>) -> Result<Inputs, Refusal> {
    let keys = match request.keys {
        Some(path) => PublisherKeys::from_json(&read(path)?),
        None => PublisherKeys::pinned(),
    }
    .map_err(Refusal::Publisher)?;
    Ok(Inputs {
        keys,
        package: read(request.package)?,
        files: signature_files(request.package, request.signatures_dir)?,
    })
}

/// One line per signature file: counted (key id, fingerprint), ignored
/// (why), or only found (the keys could not judge it).
fn listing(request: &VerifyRequest<'_>, inputs: &Inputs) -> String {
    let files: Vec<SignatureFile> = inputs.files.iter().map(|(_, f)| f.clone()).collect();
    let report =
        publisher::signature_report(&inputs.package, &files, &inputs.keys, &request.claim).ok();
    if inputs.files.is_empty() {
        return "\n  no signature file <package>.<KEYID>.minisig was found".to_owned();
    }
    inputs
        .files
        .iter()
        .enumerate()
        .map(|(n, (path, _))| {
            let shown = path.display();
            match report.as_ref().and_then(|r| r.get(n)) {
                Some(SignatureCheck::Counted {
                    key_id,
                    fingerprint,
                }) => format!("\n  counted  {shown}: key {key_id} (fingerprint {fingerprint})"),
                Some(SignatureCheck::Ignored { reason, .. }) => {
                    format!("\n  ignored  {shown}: {reason}")
                }
                None => format!("\n  found    {shown}"),
            }
        })
        .collect()
}

fn verify_inputs(
    request: &VerifyRequest<'_>,
    inputs: &Inputs,
) -> Result<publisher::VerifiedRelease, Refusal> {
    let signatures: Vec<SignatureFile> = inputs.files.iter().map(|(_, f)| f.clone()).collect();
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
    publisher::verify_release(
        &inputs.package,
        &signatures,
        &inputs.keys,
        &request.claim,
        &policy,
    )
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
