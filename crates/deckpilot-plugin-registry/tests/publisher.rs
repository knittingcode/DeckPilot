//! The first-party publisher signature (`deckpilot_plugin_registry::publisher`)
//! and `deckpilot-registry release sign|verify`, with throwaway keys made
//! per test.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use common::{ThrowawayKey, id, package, t0, v};
use deckpilot_plugin_manifest::{ApiLevel, HostEnvironment, PackageDigest, Platform};
use deckpilot_plugin_package::Limits;
use deckpilot_plugin_registry::RevokedVersions;
use deckpilot_plugin_registry::publish::{Io, run};
use deckpilot_plugin_registry::publisher::{
    AuditOutcome, AuditRecord, PublisherError, PublisherKeys, ReleaseClaim, ReleaseFloor,
    ReleasePolicy, RevokedRelease, digest_of, release_comment, verify_release,
};
use deckpilot_plugin_registry::sig::{SigError, index_comment};
use tempfile::TempDir;

fn keys(current: &[&ThrowawayKey], threshold: u64, revoked: &[&ThrowawayKey]) -> PublisherKeys {
    let keys = PublisherKeys {
        publisher_keys_version: 1,
        publisher: "deckpilot".into(),
        keys: current.iter().map(|k| k.public().to_owned()).collect(),
        threshold,
        revoked_keys: revoked.iter().map(|k| k.public().to_owned()).collect(),
    };
    keys.validate().unwrap();
    keys
}

fn host() -> HostEnvironment {
    HostEnvironment {
        app_version: v("0.10.0"),
        plugin_api: ApiLevel::new(1, 9),
        platform: Platform::Macos,
    }
}

fn claim_for(bytes: &[u8], plugin: &str, version: &str) -> ReleaseClaim {
    ReleaseClaim {
        id: id(plugin),
        version: v(version),
        package: digest_of(bytes),
    }
}

fn sign(key: &ThrowawayKey, bytes: &[u8], claim: &ReleaseClaim) -> String {
    key.sign(
        bytes,
        &release_comment(&claim.id, &claim.version, &claim.package),
    )
}

struct Case {
    bytes: Vec<u8>,
    claim: ReleaseClaim,
    host: HostEnvironment,
    limits: Limits,
}

impl Case {
    fn new(plugin: &str, version: &str) -> Self {
        let bytes = package(plugin, version).bytes;
        let claim = claim_for(&bytes, plugin, version);
        Self {
            bytes,
            claim,
            host: host(),
            limits: Limits::default(),
        }
    }

    fn verify(
        &self,
        signature: &str,
        keys: &PublisherKeys,
        floor: Option<&ReleaseFloor>,
        revoked: &[RevokedRelease],
    ) -> Result<deckpilot_plugin_registry::publisher::VerifiedRelease, PublisherError> {
        let policy = ReleasePolicy {
            host: Some(&self.host),
            floor,
            revoked,
            limits: &self.limits,
        };
        let signatures: Vec<String> = signature.split('|').map(str::to_owned).collect();
        verify_release(&self.bytes, &signatures, keys, &self.claim, &policy)
    }
}

#[test]
fn a_signed_release_verifies_and_grants_nothing() {
    let key = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let signature = sign(&key, &case.bytes, &case.claim);
    let verified = case
        .verify(&signature, &keys(&[&key], 1, &[]), None, &[])
        .unwrap();
    assert_eq!(verified.manifest().id, id("deckpilot.alpha"));
    assert_eq!(verified.signers().len(), 1);
    let files = verified.file_digests();
    assert!(files.contains_key("plugin.wasm"));
    assert!(files.contains_key("manifest.json"));
    let audit = verified.audit(t0());
    assert_eq!(audit.outcome, AuditOutcome::Verified);
    assert_eq!(audit.signers, verified.signers());
    let line = audit.to_line();
    assert!(line.ends_with('\n'));
    let back: AuditRecord = serde_json::from_str(line.trim_end()).unwrap();
    assert_eq!(back, audit);
}

#[test]
fn nothing_verifies_without_a_provisioned_key() {
    let key = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let signature = sign(&key, &case.bytes, &case.claim);
    let pinned = PublisherKeys::pinned().unwrap();
    assert!(!pinned.is_provisioned());
    assert_eq!(
        case.verify(&signature, &pinned, None, &[]).unwrap_err(),
        PublisherError::NotProvisioned
    );
}

#[test]
fn the_signature_binds_id_version_and_bytes() {
    let key = ThrowawayKey::generate();
    let other = ThrowawayKey::generate();
    let pinned = keys(&[&key], 1, &[]);
    let case = Case::new("deckpilot.alpha", "1.2.0");

    // Another key.
    let error = case
        .verify(&sign(&other, &case.bytes, &case.claim), &pinned, None, &[])
        .unwrap_err();
    assert!(matches!(error, PublisherError::Signature(_)), "{error}");

    // The right key, another version in the comment.
    let mut older = case.claim.clone();
    older.version = v("1.1.0");
    let error = case
        .verify(&sign(&key, &case.bytes, &older), &pinned, None, &[])
        .unwrap_err();
    assert!(
        matches!(
            error,
            PublisherError::Signature(SigError::WrongComment { .. })
        ),
        "{error}"
    );

    // A registry signature of the same bytes.
    let error = case
        .verify(
            &key.sign(&case.bytes, &index_comment(1, 1)),
            &pinned,
            None,
            &[],
        )
        .unwrap_err();
    assert!(matches!(error, PublisherError::Signature(_)), "{error}");

    // Tampered bytes: refused against the claim before any signature.
    let mut tampered = Case::new("deckpilot.alpha", "1.2.0");
    let signature = sign(&key, &tampered.bytes, &tampered.claim);
    let last = tampered.bytes.len() - 1;
    tampered.bytes[last] ^= 1;
    let error = tampered.verify(&signature, &pinned, None, &[]).unwrap_err();
    assert!(
        matches!(error, PublisherError::DigestMismatch { .. }),
        "{error}"
    );
    // ... and a claim rewritten to the tampered bytes fails the signature.
    tampered.claim.package = digest_of(&tampered.bytes);
    let error = tampered.verify(&signature, &pinned, None, &[]).unwrap_err();
    assert!(matches!(error, PublisherError::Signature(_)), "{error}");

    // A package of another plugin under this claim's signature.
    let mut swapped = Case::new("deckpilot.beta", "1.2.0");
    swapped.claim.id = id("deckpilot.alpha");
    let error = swapped
        .verify(
            &sign(&key, &swapped.bytes, &swapped.claim),
            &pinned,
            None,
            &[],
        )
        .unwrap_err();
    assert!(matches!(error, PublisherError::Package(_)), "{error}");
}

#[test]
fn only_first_party_releases() {
    let key = ThrowawayKey::generate();
    let case = Case::new("acme.alpha", "1.0.0");
    let error = case
        .verify(
            &sign(&key, &case.bytes, &case.claim),
            &keys(&[&key], 1, &[]),
            None,
            &[],
        )
        .unwrap_err();
    assert!(
        matches!(error, PublisherError::NotFirstParty { .. }),
        "{error}"
    );
}

#[test]
fn revoked_keys_and_thresholds() {
    let old = ThrowawayKey::generate();
    let new = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let by_old = sign(&old, &case.bytes, &case.claim);
    let error = case
        .verify(&by_old, &keys(&[&new], 1, &[&old]), None, &[])
        .unwrap_err();
    assert!(
        matches!(error, PublisherError::RevokedKey { .. }),
        "{error}"
    );
    // A key cannot be both current and revoked.
    let both = PublisherKeys {
        publisher_keys_version: 1,
        publisher: "deckpilot".into(),
        keys: vec![old.public().into()],
        threshold: 1,
        revoked_keys: vec![old.public_with_comment("same key")],
    };
    assert!(both.validate().is_err());
    // Two keys required, one signature given.
    let error = case
        .verify(&by_old, &keys(&[&old, &new], 2, &[]), None, &[])
        .unwrap_err();
    assert!(
        matches!(
            error,
            PublisherError::Signature(SigError::BelowThreshold { .. })
        ),
        "{error}"
    );
    // Both signatures (the test helper splits signatures at `|`, which
    // base64 never contains).
    let by_new = sign(&new, &case.bytes, &case.claim);
    let both_signed = format!("{by_old}|{by_new}");
    let verified = case
        .verify(&both_signed, &keys(&[&old, &new], 2, &[]), None, &[])
        .unwrap();
    assert_eq!(verified.signers().len(), 2);
    // The same signature twice is one key.
    let twice = format!("{by_old}|{by_old}");
    assert!(
        case.verify(&twice, &keys(&[&old, &new], 2, &[]), None, &[])
            .is_err()
    );
    // A valid current signature does not outweigh a revoked one.
    let third = ThrowawayKey::generate();
    let error = case
        .verify(&both_signed, &keys(&[&new, &third], 1, &[&old]), None, &[])
        .unwrap_err();
    assert!(
        matches!(error, PublisherError::RevokedKey { .. }),
        "{error}"
    );
}

#[test]
fn revoked_releases_are_refused() {
    let key = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let signature = sign(&key, &case.bytes, &case.claim);
    let pinned = keys(&[&key], 1, &[]);
    let revoked = [RevokedRelease {
        id: id("deckpilot.alpha"),
        versions: RevokedVersions::List(vec![v("1.2.0")]),
        reason: "leaks a token".into(),
    }];
    let error = case
        .verify(&signature, &pinned, None, &revoked)
        .unwrap_err();
    assert!(error.to_string().contains("leaks a token"), "{error}");
    let other = [RevokedRelease {
        id: id("deckpilot.alpha"),
        versions: RevokedVersions::List(vec![v("1.1.0")]),
        reason: "old".into(),
    }];
    case.verify(&signature, &pinned, None, &other).unwrap();
    let all = [RevokedRelease {
        id: id("deckpilot.alpha"),
        versions: RevokedVersions::All,
        reason: "withdrawn".into(),
    }];
    assert!(case.verify(&signature, &pinned, None, &all).is_err());
}

#[test]
fn compatibility_is_enforced() {
    let key = ThrowawayKey::generate();
    let mut case = Case::new("deckpilot.alpha", "1.2.0");
    let signature = sign(&key, &case.bytes, &case.claim);
    let pinned = keys(&[&key], 1, &[]);
    // The example manifest needs DeckPilot >=0.8.0 <1.0.0 and plugin API ^1.0.
    case.host.app_version = v("1.0.0");
    let error = case.verify(&signature, &pinned, None, &[]).unwrap_err();
    assert!(matches!(error, PublisherError::Incompatible(_)), "{error}");
    case.host.app_version = v("0.10.0");
    case.host.plugin_api = ApiLevel::new(2, 0);
    let error = case.verify(&signature, &pinned, None, &[]).unwrap_err();
    assert!(matches!(error, PublisherError::Incompatible(_)), "{error}");
}

#[test]
fn rollback_and_same_version_other_files_are_refused() {
    let key = ThrowawayKey::generate();
    let case = Case::new("deckpilot.alpha", "1.2.0");
    let signature = sign(&key, &case.bytes, &case.claim);
    let pinned = keys(&[&key], 1, &[]);
    let files = case
        .verify(&signature, &pinned, None, &[])
        .unwrap()
        .file_digests();

    let higher = ReleaseFloor {
        version: v("1.3.0"),
        files: BTreeMap::new(),
    };
    let error = case
        .verify(&signature, &pinned, Some(&higher), &[])
        .unwrap_err();
    assert!(matches!(error, PublisherError::Rollback { .. }), "{error}");

    let mut changed = files.clone();
    changed.insert(
        "plugin.wasm".into(),
        PackageDigest {
            sha256: digest_of(b"other").sha256,
            size: 5,
        },
    );
    let same_other = ReleaseFloor {
        version: v("1.2.0"),
        files: changed,
    };
    let error = case
        .verify(&signature, &pinned, Some(&same_other), &[])
        .unwrap_err();
    assert!(
        error.to_string().contains("plugin.wasm"),
        "names the changed file: {error}"
    );

    let same = ReleaseFloor {
        version: v("1.2.0"),
        files,
    };
    case.verify(&signature, &pinned, Some(&same), &[]).unwrap();
    let lower = ReleaseFloor {
        version: v("1.1.9"),
        files: BTreeMap::new(),
    };
    case.verify(&signature, &pinned, Some(&lower), &[]).unwrap();
}

// ------------------------------------------------------------------ CLI

struct Cli {
    dir: TempDir,
    env: HashMap<String, String>,
}

impl Cli {
    fn new() -> Self {
        Self {
            dir: TempDir::new().unwrap(),
            env: HashMap::new(),
        }
    }

    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).display().to_string()
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let env = |name: &str| self.env.get(name).cloned();
        let mut stdin = std::io::empty();
        let mut stdin = std::io::BufReader::new(&mut stdin);
        let mut out = Vec::new();
        let mut io = Io {
            env: &env,
            stdin: &mut stdin,
            out: &mut out,
        };
        run(&args, &mut io)
            .map(|()| String::from_utf8(out).unwrap())
            .map_err(|e| e.to_string())
    }
}

fn keys_file(path: &str, public: &str) {
    let doc = serde_json::json!({
        "publisherKeysVersion": 1,
        "publisher": "deckpilot",
        "keys": [public],
        "threshold": 1,
        "revokedKeys": [],
    });
    std::fs::write(path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
}

fn verify_args<'a>(cli: &'a Cli, built: &'a PackageDigest, extra: &[&'a str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "release",
        "verify",
        "--package",
        &cli.path("alpha.dpplugin"),
        "--signature",
        &cli.path("alpha.dpplugin.minisig"),
        "--id",
        "deckpilot.alpha",
        "--version",
        "1.2.0",
        "--sha256",
        &built.sha256.to_string(),
        "--size",
        &built.size.to_string(),
        "--app-version",
        "0.10.0",
        "--plugin-api",
        "1.9",
        "--platform",
        "macos",
        "--audit",
        &cli.path("audit.jsonl"),
        "--now",
        "2026-11-21T10:00:00Z",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    args
}

#[test]
fn cli_signs_verifies_audits_and_unpacks() {
    let mut cli = Cli::new();
    let bytes = package("deckpilot.alpha", "1.2.0").bytes;
    std::fs::write(cli.path("alpha.dpplugin"), &bytes).unwrap();
    let digest = digest_of(&bytes);
    cli.run(&[
        "keys",
        "generate",
        "--role",
        "publisher",
        "--out",
        &cli.path("publisher.key"),
        "--no-password",
    ])
    .unwrap();

    // The key from an environment variable, as a CI secret.
    let secret = std::fs::read_to_string(cli.path("publisher.key")).unwrap();
    cli.env.insert("PUBLISHER_KEY".into(), secret);
    let out = cli
        .run(&[
            "release",
            "sign",
            "--package",
            &cli.path("alpha.dpplugin"),
            "--key-env",
            "PUBLISHER_KEY",
            "--out",
            &cli.path("alpha.dpplugin.minisig"),
        ])
        .unwrap();
    assert!(out.contains("signed deckpilot.alpha 1.2.0"), "{out}");
    assert!(!out.contains("secret"), "{out}");

    // The pinned keys are not provisioned: refused, and audited.
    let args = verify_args(&cli, &digest, &[]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let error = cli.run(&args).unwrap_err();
    assert!(error.contains("no first-party publisher key"), "{error}");

    // With the test key and unpacking.
    let public = std::fs::read_to_string(cli.path("publisher.key.pub")).unwrap();
    keys_file(&cli.path("keys.json"), public.trim());
    let keys = cli.path("keys.json");
    let unpack = cli.path("unpacked");
    let args = verify_args(&cli, &digest, &["--keys", &keys, "--unpack", &unpack]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = cli.run(&args).unwrap();
    assert!(out.contains("deckpilot.alpha 1.2.0 verified"), "{out}");
    assert!(out.contains("not the pinned keys"), "{out}");
    assert!(out.contains("no permission is granted"), "{out}");
    assert!(Path::new(&unpack).join("plugin.wasm").is_file());
    assert!(Path::new(&unpack).join("manifest.json").is_file());

    let audit = std::fs::read_to_string(cli.path("audit.jsonl")).unwrap();
    let records: Vec<AuditRecord> = audit
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].outcome, AuditOutcome::Refused);
    assert!(
        records[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("publisher key")
    );
    assert_eq!(records[1].outcome, AuditOutcome::Verified);
    assert_eq!(records[1].sha256, digest.sha256.to_string());

    // A claim that names other bytes.
    let wrong = PackageDigest {
        sha256: digest_of(b"x").sha256,
        size: digest.size,
    };
    let args = verify_args(&cli, &wrong, &["--keys", &keys]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    assert!(cli.run(&args).unwrap_err().contains("the claim says"));

    // A higher version already shipped (released pins).
    let pins = serde_json::json!({
        "releasedPinsVersion": 1,
        "releases": [{ "release": "0.11.0", "platform": "macos", "source": "test",
                       "plugins": [{ "id": "deckpilot.alpha", "version": "1.3.0", "files": {} }] }]
    });
    std::fs::write(cli.path("released-pins.json"), pins.to_string()).unwrap();
    let released = cli.path("released-pins.json");
    let args = verify_args(
        &cli,
        &digest,
        &["--keys", &keys, "--released-pins", &released],
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let error = cli.run(&args).unwrap_err();
    assert!(error.contains("below 1.3.0"), "{error}");
    // Other platforms' pins do not count.
    let args = verify_args(
        &cli,
        &digest,
        &["--keys", &keys, "--released-pins", &released],
    );
    let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
    let at = args.iter().position(|a| *a == "macos").unwrap();
    args[at] = "linux";
    cli.run(&args).unwrap();
}

#[test]
fn cli_refuses_to_sign_what_could_never_verify() {
    let mut cli = Cli::new();
    cli.run(&[
        "keys",
        "generate",
        "--role",
        "publisher",
        "--out",
        &cli.path("publisher.key"),
        "--no-password",
    ])
    .unwrap();
    let bytes = package("acme.alpha", "1.0.0").bytes;
    std::fs::write(cli.path("acme.dpplugin"), &bytes).unwrap();
    let error = cli
        .run(&[
            "release",
            "sign",
            "--package",
            &cli.path("acme.dpplugin"),
            "--key",
            &cli.path("publisher.key"),
            "--out",
            &cli.path("acme.minisig"),
        ])
        .unwrap_err();
    assert!(error.contains("refusing to sign"), "{error}");
    assert!(!Path::new(&cli.path("acme.minisig")).exists());
    cli.env.clear();
    let error = cli
        .run(&[
            "release",
            "sign",
            "--package",
            &cli.path("acme.dpplugin"),
            "--key-env",
            "MISSING",
            "--out",
            &cli.path("acme.minisig"),
        ])
        .unwrap_err();
    assert!(error.contains("MISSING is not set"), "{error}");
}

// ------------------------------------------------------------ the app's files

const ARTIFACTS: &str = include_str!("../../../plugins/first-party/artifacts.json");
const RELEASED_PINS: &str = include_str!("../../../plugins/first-party/released-pins.json");

#[test]
fn the_artifact_lock_parses_and_locks_nothing_yet() {
    use deckpilot_plugin_registry::publisher::ArtifactLock;
    let lock = ArtifactLock::from_json(ARTIFACTS.as_bytes()).unwrap();
    // Wave A: no plugin is consumed from a release artifact yet.
    assert_eq!(lock.plugins, Vec::new());
    assert_eq!(lock.revoked, Vec::new());

    let entry = |id: &str, version: &str| {
        serde_json::json!({ "id": id, "version": version,
                            "sha256": digest_of(b"x").sha256.to_string(), "size": 1 })
    };
    let doc = |plugins: Vec<serde_json::Value>, revoked: serde_json::Value| {
        serde_json::json!({ "artifactsVersion": 1, "plugins": plugins, "revoked": revoked })
            .to_string()
    };
    let one = ArtifactLock::from_json(
        doc(
            vec![entry("deckpilot.github", "0.2.1")],
            serde_json::json!([]),
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        one.plugins[0].package_url(),
        "https://github.com/knittingcode/deckpilot-plugins/releases/download/\
         deckpilot.github--v0.2.1/deckpilot.github-0.2.1.dpplugin"
    );
    assert!(
        one.plugins[0]
            .signature_url()
            .ends_with(".dpplugin.minisig")
    );
    assert_eq!(one.plugins[0].claim().version, v("0.2.1"));
    for bad in [
        doc(vec![entry("acme.github", "0.2.1")], serde_json::json!([])),
        doc(
            vec![
                entry("deckpilot.github", "0.2.1"),
                entry("deckpilot.github", "0.2.2"),
            ],
            serde_json::json!([]),
        ),
        doc(
            vec![entry("deckpilot.github", "0.2.1")],
            serde_json::json!([{ "id": "deckpilot.github", "versions": "*", "reason": "x" }]),
        ),
        doc(vec![], serde_json::json!([]))
            .replace("\"artifactsVersion\":1", "\"artifactsVersion\":2"),
    ] {
        assert!(ArtifactLock::from_json(bad.as_bytes()).is_err(), "{bad}");
    }
}

#[test]
fn the_released_pins_give_the_rollback_floor() {
    use deckpilot_plugin_registry::publish::floor_from_released_pins;
    let pins = RELEASED_PINS.as_bytes();
    let floor = floor_from_released_pins(pins, &id("deckpilot.github"), Platform::Macos)
        .unwrap()
        .expect("0.9.0 shipped deckpilot.github on macOS");
    assert!(floor.version >= v("0.1.0"));
    assert!(floor.files.contains_key("plugin.wasm"));
    let unknown = floor_from_released_pins(pins, &id("deckpilot.unknown"), Platform::Macos);
    assert!(unknown.unwrap().is_none());
}
