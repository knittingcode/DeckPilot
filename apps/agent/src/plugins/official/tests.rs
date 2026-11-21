//! Official releases against a real plugin store in a temporary data
//! directory, with throwaway publisher keys made per test and the bundled
//! test fixture `plugins/first-party/test-fixtures/example-default` (a
//! minimal ABI 1 module stands in for its `plugin.wasm`).

use std::path::Path;
use std::sync::Arc;

use deckpilot_plugin_bundled::{BUNDLED_DIR, BundleClass, Pins};
use deckpilot_plugin_manifest::{ApiLevel, Platform};
use deckpilot_plugin_package::{MANIFEST, PackageBuilder};
use deckpilot_plugin_registry::RevokedVersions;
use deckpilot_plugin_registry::publish::{KeyRole, SigningKey, generate_key};
use deckpilot_plugin_store::{
    Consent, CrashOnce, Failpoint, GrantKey, GrantSet, GrantStatus, ManagerOptions, Operation,
    PluginManager, PluginState, StoreKey, UpdateOutcome,
};
use serde_json::{Value, json};

use super::super::bundled::{self, Bundle, Seeded};
use super::*;

const ID: &str = "deckpilot.example-default";
const FIXTURE: &str = "../../plugins/first-party/test-fixtures/example-default";
const REGISTRY: &str = "https://registry.example/";

fn id() -> PluginId {
    PluginId::parse(ID).unwrap()
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURE)
            .join(name),
    )
    .unwrap()
}

fn host() -> HostEnvironment {
    HostEnvironment {
        app_version: v("0.10.0"),
        plugin_api: ApiLevel::new(1, 9),
        platform: Platform::current().unwrap_or(Platform::Macos),
    }
}

fn notifications() -> Value {
    json!([{ "kind": "notifications", "justification": "Test fixture: low tier." }])
}

fn with_network() -> Value {
    json!([
        { "kind": "notifications", "justification": "Test fixture: low tier." },
        {
            "kind": "network",
            "hosts": ["api.example.com"],
            "justification": "Test fixture: medium tier, needs consent."
        }
    ])
}

fn with_setting(manifest: &mut Value) {
    manifest["contributes"]["settings"] = json!({
        "portabilityVersion": 1,
        "fields": [{
            "key": "greeting",
            "label": "Greeting",
            "description": "Test fixture: a setting kept through the transition.",
            "required": false,
            "kind": { "type": "text", "maxLength": 32 },
            "portability": "portable"
        }]
    });
}

/// The plugin's files at `version` (the module carries the version, so
/// every version has other bytes; `note` changes them within a version).
fn files(version: &str, permissions: &Value, note: &str, edit: impl FnOnce(&mut Value)) -> Files {
    let mut manifest: Value = serde_json::from_slice(&fixture("manifest.json")).unwrap();
    manifest["version"] = version.into();
    manifest["permissions"] = permissions.clone();
    with_setting(&mut manifest);
    edit(&mut manifest);
    let mut files = Files::new();
    files.insert(
        MANIFEST.into(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    );
    files.insert(
        "plugin.wasm".into(),
        deckpilot_plugin_package::wasm::minimal_abi_module_with_note(&format!("{version}{note}")),
    );
    files.insert("LICENSE".into(), fixture("LICENSE"));
    files.insert("assets/icon-128.png".into(), fixture("assets/icon-128.png"));
    files
}

fn package_of(files: &Files) -> Vec<u8> {
    let mut builder = PackageBuilder::new();
    builder.first_party();
    for (path, bytes) in files {
        builder.add(path, bytes.clone()).unwrap();
    }
    builder.build().unwrap().bytes
}

/// A `.dpplugin` written directly (no content checks): for manifests the
/// builder refuses, such as the privileged kinds.
fn raw_package(files: &Files) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, bytes) in files {
        let mut header = tar::Header::new_ustar();
        header.set_path(path).unwrap();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_cksum();
        tar.append(&header, bytes.as_slice()).unwrap();
    }
    let tar = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}

/// What a verified registry chain would pin for `package`.
fn expected_for(package: &[u8], version: &str) -> ExpectedPackage {
    let files = read_archive(package, &Limits::default()).unwrap();
    let manifest: Value = serde_json::from_slice(&files[MANIFEST]).unwrap();
    ExpectedPackage {
        id: id(),
        version: v(version),
        package: digest(package),
        manifest_sha256: sha256(&files[MANIFEST]),
        permissions: Some(manifest["permissions"].as_array().unwrap().clone()),
        first_party: true,
    }
}

struct Key {
    signing: SigningKey,
}

impl Key {
    fn new(dir: &Path, name: &str) -> Self {
        let path = dir.join(name);
        generate_key(KeyRole::Publisher, &path, None).unwrap();
        Self {
            signing: SigningKey::load(&path, None).unwrap(),
        }
    }

    fn public(&self) -> String {
        self.signing.public().to_owned()
    }

    /// The `.minisig` bytes for `package` as `id` `version`.
    fn sign(&self, package: &[u8], version: &str) -> Vec<u8> {
        let comment = publisher::release_comment(&id(), &v(version), &digest(package));
        self.signing.sign(package, &comment).unwrap().into_bytes()
    }
}

fn keys(current: &[&Key], threshold: u64, revoked: &[&Key]) -> PublisherKeys {
    PublisherKeys {
        publisher_keys_version: 1,
        publisher: "deckpilot".into(),
        keys: current.iter().map(|k| k.public()).collect(),
        threshold,
        revoked_keys: revoked.iter().map(|k| k.public()).collect(),
    }
}

/// A temporary data directory, application resources and keys.
struct Env {
    root: tempfile::TempDir,
    key: Key,
}

impl Env {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let key = Key::new(root.path(), "publisher.key");
        Self { root, key }
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn state(&self) -> PathBuf {
        self.data().join(STATE_DIR)
    }

    fn open(&self) -> PluginManager {
        self.open_with(ManagerOptions::default())
    }

    fn open_with(&self, options: ManagerOptions) -> PluginManager {
        PluginManager::open(&self.data(), &StoreKey::from_bytes([0x42; 32]), options).unwrap()
    }

    fn official(&self) -> Official {
        self.official_with(keys(&[&self.key], 1, &[]), Vec::new())
    }

    fn official_with(&self, keys: PublisherKeys, revoked: Vec<RevokedRelease>) -> Official {
        self.official_compiled(
            keys,
            Compiled {
                revoked,
                ..Compiled::default()
            },
        )
    }

    fn official_compiled(&self, keys: PublisherKeys, compiled: Compiled) -> Official {
        Official::with(&self.state(), Ok(keys), Ok(compiled))
    }

    /// The bundle at `version` with `permissions`, pinned to its files.
    fn bundle(&self, version: &str, permissions: &Value) -> Bundle {
        let dir = self.root.path().join("resources").join(BUNDLED_DIR);
        let plugin = dir.join(ID);
        let _ = std::fs::remove_dir_all(&plugin);
        for (path, bytes) in files(version, permissions, "", |_| {}) {
            let target = plugin.join(&path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }
        let pins = Pins::compute(&dir, &[(id(), v(version), BundleClass::Default)]).unwrap();
        Bundle {
            dir: Some(dir),
            pins,
            trust: Ok(()),
        }
    }

    fn audit(&self) -> Vec<ChainedLine> {
        std::fs::read_to_string(self.state().join(AUDIT_FILE))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

/// One release: its package, what the registry pins and its signature.
struct Release {
    package: Vec<u8>,
    expected: ExpectedPackage,
    signature: Vec<u8>,
}

impl Release {
    fn new(env: &Env, version: &str, permissions: &Value) -> Self {
        Self::with(env, version, permissions, "", |_| {})
    }

    fn with(
        env: &Env,
        version: &str,
        permissions: &Value,
        note: &str,
        edit: impl FnOnce(&mut Value),
    ) -> Self {
        let package = package_of(&files(version, permissions, note, edit));
        let expected = expected_for(&package, version);
        let signature = env.key.sign(&package, version);
        Self {
            package,
            expected,
            signature,
        }
    }

    fn delivery<'a>(
        &'a self,
        bundle: Option<&'a Bundle>,
        host: &'a HostEnvironment,
    ) -> Delivery<'a> {
        Delivery {
            expected: &self.expected,
            package: &self.package,
            signatures: std::slice::from_ref(&self.signature),
            pin: bundle.and_then(|b| b.pins.get(&id())),
            host: Some(host),
            now: Utc::now(),
        }
    }
}

fn registry_source() -> InstallSource {
    InstallSource::Registry {
        registry: REGISTRY.into(),
    }
}

fn refusal(result: Result<VerifiedPackage, Refusal>) -> Refusal {
    match result {
        Ok(package) => panic!("verified {}", package.manifest().version),
        Err(refusal) => refusal,
    }
}

/// The bundled copy seeded (enabled where plugins run), with a setting
/// and a credential the transition must keep.
fn seeded(env: &Env, bundle: &Bundle) -> PluginManager {
    let mut manager = env.open();
    bundled::seed(&mut manager, bundle);
    manager
        .set_settings(
            &id(),
            &std::iter::once(("greeting".to_owned(), json!("Yo"))).collect(),
        )
        .unwrap();
    manager
}

fn state_of(manager: &PluginManager) -> PluginRecord {
    manager.store().plugin(&id()).unwrap().unwrap()
}

/// A registry that revokes nothing.
struct NoVerdicts;

impl VerdictSource for NoVerdicts {
    fn verdict(&self, _id: &PluginId, _version: &Version) -> InstalledVerdict {
        InstalledVerdict::Run
    }
}

// ------------------------------------------------------------ the keys

#[test]
fn this_build_cannot_verify_any_official_release() {
    // No publisher key is pinned (the owner provisions it): every official
    // install is refused before any download, and a verification attempt
    // is refused and audited.
    let env = Env::new();
    let official = Official::open(&env.state());
    assert!(
        official
            .ready()
            .unwrap_err()
            .contains("no first-party publisher key"),
        "{:?}",
        official.ready()
    );
    let release = Release::new(&env, "0.1.1", &notifications());
    let host = host();
    let refused = refusal(official.verify(&release.delivery(None, &host)));
    assert_eq!(refused, Refusal::Publisher(PublisherError::NotProvisioned));
    let audit = env.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].record.outcome, publisher::AuditOutcome::Refused);
    assert!(official.floor(&id()).is_none());
}

#[test]
fn a_release_verifies_and_raises_the_floor() {
    let env = Env::new();
    let official = env.official();
    official.ready().unwrap();
    let release = Release::new(&env, "0.1.1", &notifications());
    let host = host();
    let package = official.verify(&release.delivery(None, &host)).unwrap();
    assert_eq!(package.manifest().version, v("0.1.1"));
    let floor = official.floor(&id()).unwrap();
    assert_eq!(floor.version, v("0.1.1"));
    assert_eq!(floor.package, digest(&release.package));
    assert_eq!(floor.signers.len(), 1);
    // Persisted: a new agent sees it.
    assert_eq!(env.official().floor(&id()), Some(floor));
    let audit = env.audit();
    assert_eq!(audit[0].record.outcome, publisher::AuditOutcome::Verified);
    assert_eq!(audit[0].prev, GENESIS);
}

// ----------------------------------------------------------- tampering

#[test]
fn tampered_packages_and_swapped_signatures_are_refused() {
    let env = Env::new();
    let official = env.official();
    let host = host();
    let genuine = Release::new(&env, "0.1.1", &notifications());

    // A flipped byte (the registry's pin still names the genuine bytes).
    let mut tampered = Release::new(&env, "0.1.1", &notifications());
    let last = tampered.package.len() - 1;
    tampered.package[last] ^= 1;
    assert!(matches!(
        refusal(official.verify(&tampered.delivery(None, &host))),
        Refusal::Publisher(PublisherError::DigestMismatch { .. })
    ));

    // A compromised registry pins other bytes (its own package): the
    // publisher signature of the genuine release does not cover them.
    let mut swapped = Release::with(&env, "0.1.1", &notifications(), "-evil", |_| {});
    swapped.signature = genuine.signature.clone();
    assert!(matches!(
        refusal(official.verify(&swapped.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Signature(_))
    ));

    // The signature of another version, replayed.
    let other = Release::new(&env, "0.1.2", &notifications());
    let mut replayed = Release::new(&env, "0.1.1", &notifications());
    replayed.signature = other.signature.clone();
    assert!(matches!(
        refusal(official.verify(&replayed.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Signature(_))
    ));

    // A key this agent does not pin.
    let stranger = Key::new(env.root.path(), "stranger.key");
    let mut foreign = Release::new(&env, "0.1.1", &notifications());
    foreign.signature = stranger.sign(&foreign.package, "0.1.1");
    assert!(matches!(
        refusal(official.verify(&foreign.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Signature(_))
    ));

    // Garbage instead of a signature.
    let mut garbage = Release::new(&env, "0.1.1", &notifications());
    garbage.signature = b"not base64 !".to_vec();
    assert!(matches!(
        refusal(official.verify(&garbage.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Signature(_))
    ));

    // The registry's permissions (the consent dialog) must be the
    // manifest's.
    let mut relabeled = Release::new(&env, "0.1.1", &notifications());
    relabeled.expected.permissions = Some(Vec::new());
    assert!(matches!(
        refusal(official.verify(&relabeled.delivery(None, &host))),
        Refusal::Package(_)
    ));

    // A registry that does not authorize the namespace.
    let mut unauthorized = Release::new(&env, "0.1.1", &notifications());
    unauthorized.expected.first_party = false;
    assert!(matches!(
        refusal(official.verify(&unauthorized.delivery(None, &host))),
        Refusal::Package(_)
    ));

    // Nothing above moved the floor; every refusal is audited.
    assert!(official.floor(&id()).is_none());
    let audit = env.audit();
    assert_eq!(audit.len(), 7);
    assert!(
        audit
            .iter()
            .all(|l| l.record.outcome == publisher::AuditOutcome::Refused)
    );
}

#[test]
fn the_audit_log_is_hash_chained() {
    let env = Env::new();
    let official = env.official();
    let host = host();
    for version in ["0.1.1", "0.1.2", "0.1.0"] {
        let release = Release::new(&env, version, &notifications());
        let _ = official.verify(&release.delivery(None, &host));
    }
    let path = env.state().join(AUDIT_FILE);
    let text = std::fs::read_to_string(&path).unwrap();
    check_audit_chain(&text, GENESIS).unwrap();
    // Removing a line, or editing one, breaks the chain.
    let lines: Vec<&str> = text.lines().collect();
    let removed = format!("{}\n{}\n", lines[0], lines[2]);
    assert_eq!(check_audit_chain(&removed, GENESIS), Err(2));
    let edited = text.replacen("\"verified\"", "\"refused\"", 1);
    assert_eq!(check_audit_chain(&edited, GENESIS), Err(2));
    // The last line chains to nothing yet: its hash is in the agent's log
    // (`official audit line`), the anchor to compare a log against.
}

// ------------------------------------------------------------ rollback

#[test]
fn never_below_the_floor_and_the_same_version_only_with_the_same_files() {
    let env = Env::new();
    let official = env.official();
    let host = host();
    let release = Release::new(&env, "0.1.2", &notifications());
    official.verify(&release.delivery(None, &host)).unwrap();

    let older = Release::new(&env, "0.1.1", &notifications());
    assert!(matches!(
        refusal(official.verify(&older.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
    let other_files = Release::with(&env, "0.1.2", &notifications(), "-rebuilt", |_| {});
    assert!(matches!(
        refusal(official.verify(&other_files.delivery(None, &host))),
        Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
    ));
    // The same release again (a retried install) verifies.
    official.verify(&release.delivery(None, &host)).unwrap();
    // A new agent (a restart) keeps the floor.
    let restarted = env.official();
    assert!(matches!(
        refusal(restarted.verify(&older.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
}

#[test]
fn the_bundled_copy_is_a_floor_too() {
    let env = Env::new();
    let official = env.official();
    let host = host();
    let bundle = env.bundle("0.2.0", &notifications());
    let older = Release::new(&env, "0.1.9", &notifications());
    assert!(matches!(
        refusal(official.verify(&older.delivery(Some(&bundle), &host))),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
    // The bundled version with other files.
    let same = Release::with(&env, "0.2.0", &notifications(), "-artifact", |_| {});
    assert!(matches!(
        refusal(official.verify(&same.delivery(Some(&bundle), &host))),
        Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
    ));
    official
        .verify(&Release::new(&env, "0.2.1", &notifications()).delivery(Some(&bundle), &host))
        .unwrap();
}

#[test]
fn unreadable_floors_refuse_and_are_never_reset() {
    let env = Env::new();
    std::fs::create_dir_all(env.state()).unwrap();
    std::fs::write(env.state().join(FLOORS_FILE), b"{ not json").unwrap();
    let official = env.official();
    assert!(official.ready().is_err());
    let host = host();
    let release = Release::new(&env, "0.1.1", &notifications());
    assert!(matches!(
        refusal(official.verify(&release.delivery(None, &host))),
        Refusal::Unavailable(_)
    ));
    assert_eq!(
        std::fs::read(env.state().join(FLOORS_FILE)).unwrap(),
        b"{ not json"
    );
}

// ---------------------------------------------------------- revocation

#[test]
fn revoked_keys_and_releases_are_refused() {
    let env = Env::new();
    let host = host();
    let release = Release::new(&env, "0.1.1", &notifications());

    let revoked_key = env.official_with(keys(&[], 1, &[&env.key]), Vec::new());
    // A key set whose only key is revoked is not provisioned at all.
    assert!(revoked_key.ready().is_err());
    let second = Key::new(env.root.path(), "second.key");
    let revoked_key = env.official_with(keys(&[&second], 1, &[&env.key]), Vec::new());
    assert!(matches!(
        refusal(revoked_key.verify(&release.delivery(None, &host))),
        Refusal::Publisher(PublisherError::RevokedKey { .. })
    ));

    let lock = vec![RevokedRelease {
        id: id(),
        versions: RevokedVersions::List(vec![v("0.1.1")]),
        reason: "test".into(),
    }];
    let revoked_release = env.official_with(keys(&[&env.key], 1, &[]), lock);
    let refused = refusal(revoked_release.verify(&release.delivery(None, &host)));
    assert!(matches!(
        refused,
        Refusal::Publisher(PublisherError::Revoked { .. })
    ));
    assert_eq!(
        refused.to_ipc(&id()).reason.as_deref(),
        Some(reason::REVOKED)
    );
}

#[test]
fn an_installed_release_is_revoked_by_a_later_agent() {
    let env = Env::new();
    let host = host();
    let official = env.official();
    let release = Release::new(&env, "0.1.1", &notifications());
    let package = official.verify(&release.delivery(None, &host)).unwrap();
    let mut manager = env.open();
    manager
        .install(&package, &registry_source(), &Consent::of(vec![], "user"))
        .unwrap();
    let records = manager.store().plugins().unwrap();
    assert!(official.revoked_installed(&records).is_empty());

    // An agent whose pinned keys revoke the key that signed it.
    let second = Key::new(env.root.path(), "second.key");
    let later = env.official_with(keys(&[&second], 1, &[&env.key]), Vec::new());
    let revoked = later.revoked_installed(&records);
    assert_eq!(revoked.len(), 1, "{revoked:?}");
    let verdicts = Verdicts {
        official: &revoked,
        registry: &NoVerdicts,
    };
    let changes = manager.apply_verdicts(&verdicts).unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(state_of(&manager).state, PluginState::Quarantined);

    // An agent whose lock revokes the release.
    let lock = vec![RevokedRelease {
        id: id(),
        versions: RevokedVersions::All,
        reason: "withdrawn".into(),
    }];
    let locked = env.official_with(keys(&[&env.key], 1, &[]), lock);
    assert_eq!(locked.revoked_installed(&records).len(), 1);
    // A bundled copy is never affected (the pins vouch for it).
    let other = Env::new();
    let bundle = other.bundle("0.1.0", &notifications());
    let bundled_manager = seeded(&other, &bundle);
    let lock = vec![RevokedRelease {
        id: id(),
        versions: RevokedVersions::All,
        reason: "withdrawn".into(),
    }];
    let locked = other.official_with(keys(&[&other.key], 1, &[]), lock);
    assert!(
        locked
            .revoked_installed(&bundled_manager.store().plugins().unwrap())
            .is_empty()
    );
}

// ------------------------------------------------------- compatibility

#[test]
fn an_incompatible_release_is_refused() {
    let env = Env::new();
    let official = env.official();
    let release = Release::with(&env, "0.1.1", &notifications(), "", |m| {
        m["engines"]["deckpilot"] = json!(">=9.0.0");
    });
    let host = host();
    let refused = refusal(official.verify(&release.delivery(None, &host)));
    assert!(matches!(
        refused,
        Refusal::Publisher(PublisherError::Incompatible(_))
    ));
    assert_eq!(
        refused.to_ipc(&id()).reason.as_deref(),
        Some(reason::INCOMPATIBLE)
    );
    let old_api = HostEnvironment {
        plugin_api: ApiLevel::new(1, 0),
        ..host.clone()
    };
    let newer_api = Release::with(&env, "0.1.1", &notifications(), "", |m| {
        m["engines"]["pluginApi"] = json!("^1.9");
    });
    assert!(matches!(
        refusal(official.verify(&newer_api.delivery(None, &old_api))),
        Refusal::Publisher(PublisherError::Incompatible(_))
    ));
    // No known host: refused, never checked without one.
    let mut unknown = release.delivery(None, &host);
    unknown.host = None;
    assert!(matches!(
        refusal(official.verify(&unknown)),
        Refusal::Unavailable(_)
    ));
}

// ---------------------------------------------------------------- gate

fn privileged_files(version: &str) -> Files {
    files(
        version,
        &json!([{ "kind": "hostViews", "justification": "Test fixture: privileged." }]),
        "",
        |m| m["engines"]["pluginApi"] = json!("^1.9"),
    )
}

#[test]
fn the_privileged_gate_is_closed() {
    // The compiled gate. Changing this assertion is the review's job.
    assert_eq!(PRIVILEGED_GATE, PrivilegedGate::Closed);
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    // Even with two distinct keys and a perfect release.
    let official = env.official_with(keys(&[&env.key, &second], 2, &[]), Vec::new());
    let package = raw_package(&privileged_files("0.1.1"));
    let expected = expected_for(&package, "0.1.1");
    let signatures = [
        env.key.sign(&package, "0.1.1"),
        second.sign(&package, "0.1.1"),
    ];
    let host = host();
    let delivery = Delivery {
        expected: &expected,
        package: &package,
        signatures: &signatures,
        pin: None,
        host: Some(&host),
        now: Utc::now(),
    };
    assert!(matches!(
        refusal(official.verify(&delivery)),
        Refusal::Publisher(PublisherError::PrivilegedGateClosed { .. })
    ));
    assert!(official.floor(&id()).is_none());
}

static REVIEW: GateReview = GateReview {
    reviewer: "test",
    date: "2026-11-21",
    min_threshold: 2,
    plugins: &[(ID, &["hostViews"])],
};

static ONE_KEY_REVIEW: GateReview = GateReview {
    reviewer: "test",
    date: "2026-11-21",
    min_threshold: 1,
    plugins: &[(ID, &["hostViews"])],
};

static OTHER_KEYS_REVIEW: GateReview = GateReview {
    reviewer: "test",
    date: "2026-11-21",
    min_threshold: 2,
    plugins: &[(ID, &["tools:git"])],
};

/// The opening path, tested behind a test-only gate: its own conditions
/// refuse, and when they hold the package passes the bundled reader. The
/// store still refuses to install it from a registry (X12's second lock).
#[test]
fn the_opening_path_needs_its_review_conditions_and_meets_the_store_lock() {
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let package = raw_package(&privileged_files("0.1.1"));
    let expected = expected_for(&package, "0.1.1");
    let signatures = [
        env.key.sign(&package, "0.1.1"),
        second.sign(&package, "0.1.1"),
    ];
    let host = host();
    let delivery = Delivery {
        expected: &expected,
        package: &package,
        signatures: &signatures,
        pin: None,
        host: Some(&host),
        now: Utc::now(),
    };
    let gated = |keys: PublisherKeys, gate| env.official_with(keys, Vec::new()).with_gate(gate);
    let closed = |r: Result<VerifiedPackage, Refusal>| {
        matches!(
            r,
            Err(Refusal::Publisher(
                PublisherError::PrivilegedGateClosed { .. }
            ))
        )
    };
    // One key is never enough, whatever the review says.
    let one = gated(
        keys(&[&env.key], 1, &[]),
        PrivilegedGate::Open(&ONE_KEY_REVIEW),
    );
    assert!(closed(one.verify(&delivery)));
    let one = gated(keys(&[&env.key], 1, &[]), PrivilegedGate::Open(&REVIEW));
    assert!(closed(one.verify(&delivery)));
    // A kind the review does not name.
    let other = gated(
        keys(&[&env.key, &second], 2, &[]),
        PrivilegedGate::Open(&OTHER_KEYS_REVIEW),
    );
    assert!(closed(other.verify(&delivery)));
    // Everything holds: the bundled reader accepts it.
    let open = gated(
        keys(&[&env.key, &second], 2, &[]),
        PrivilegedGate::Open(&REVIEW),
    );
    let admitted = open.verify(&delivery).unwrap();
    assert!(
        GrantSet::from_manifest(admitted.manifest())
            .keys()
            .any(GrantKey::is_first_party_only)
    );
    // ... and the store refuses it from a registry all the same.
    let mut manager = env.open();
    let refused = manager
        .install(&admitted, &registry_source(), &Consent::of(vec![], "user"))
        .unwrap_err();
    assert!(
        matches!(
            refused,
            deckpilot_plugin_store::StoreError::SourceNotAllowed { .. }
        ),
        "{refused}"
    );
}

// ---------------------------------------------------------- transition

/// Bundled 0.1.0 → official 0.1.1 through the store's update: the id, the
/// setting and the grants stay; the copy becomes registry-managed.
#[test]
fn the_transition_keeps_the_configuration_and_the_grants() {
    let env = Env::new();
    let bundle = env.bundle("0.1.0", &notifications());
    let mut manager = seeded(&env, &bundle);
    let before = state_of(&manager);
    let official = env.official();
    let host = host();
    let release = Release::new(&env, "0.1.1", &notifications());
    let package = official
        .verify(&release.delivery(Some(&bundle), &host))
        .unwrap();
    check_transition(&before.active().unwrap().manifest, package.manifest()).unwrap();
    let outcome = manager.update(&package, &registry_source()).unwrap();
    assert!(
        matches!(outcome, UpdateOutcome::Applied { .. }),
        "{outcome:?}"
    );
    let after = state_of(&manager);
    assert_eq!(after.source, registry_source());
    assert_eq!(after.state, before.state);
    assert_eq!(after.active().unwrap().version, v("0.1.1"));
    assert_eq!(
        manager.store().settings(&id()).unwrap()["greeting"],
        json!("Yo")
    );
    let grants = manager.store().grants(&id()).unwrap();
    assert_eq!(
        grants.iter().map(|g| g.key.to_string()).collect::<Vec<_>>(),
        ["notifications"]
    );
    assert!(grants.iter().all(|g| g.status == GrantStatus::Granted));

    // Idempotent: the same release again changes nothing.
    official
        .verify(&release.delivery(Some(&bundle), &host))
        .unwrap();
    assert!(manager.update(&package, &registry_source()).is_err());
    assert_eq!(state_of(&manager), after);

    // Offline start (no registry): seeding leaves the managed copy alone,
    // and it stays as it was.
    let report = bundled::seed(&mut manager, &bundle);
    assert!(
        matches!(&report[0].1, Seeded::Skipped(why) if why.contains("registry")),
        "{report:?}"
    );
    drop(manager);
    let mut reopened = env.open();
    reopened.recover().unwrap();
    bundled::seed(&mut reopened, &bundle);
    assert_eq!(state_of(&reopened), after);
}

#[test]
fn new_privileges_wait_for_consent_and_the_bundled_copy_keeps_running() {
    let env = Env::new();
    let bundle = env.bundle("0.1.0", &notifications());
    let mut manager = seeded(&env, &bundle);
    let before = state_of(&manager);
    let official = env.official();
    let host = host();
    let release = Release::new(&env, "0.1.1", &with_network());
    let package = official
        .verify(&release.delivery(Some(&bundle), &host))
        .unwrap();
    let outcome = manager.update(&package, &registry_source()).unwrap();
    let UpdateOutcome::AwaitingConsent { diff, .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert!(
        diff.escalations()
            .iter()
            .any(|key| key.to_string().starts_with("network")),
        "{diff:?}"
    );
    let staged = state_of(&manager);
    assert_eq!(staged.active().unwrap().version, v("0.1.0"));
    assert_eq!(staged.state, before.state);
    let granted: Vec<String> = manager
        .store()
        .grants(&id())
        .unwrap()
        .iter()
        .filter(|g| g.status == GrantStatus::Granted)
        .map(|g| g.key.to_string())
        .collect();
    assert_eq!(granted, ["notifications"], "nothing new is granted");
}

#[test]
fn a_transition_that_drops_an_action_or_setting_is_refused() {
    let bundled: Manifest = {
        let f = files("0.1.0", &notifications(), "", |_| {});
        Manifest::from_slice(&f[MANIFEST]).unwrap()
    };
    let renamed: Manifest = {
        let f = files("0.1.1", &notifications(), "", |m| {
            m["contributes"]["actions"][0]["id"] = json!("hello");
        });
        Manifest::from_slice(&f[MANIFEST]).unwrap()
    };
    let problem = check_transition(&bundled, &renamed).unwrap_err();
    assert!(problem.contains("actions greet"), "{problem}");
    let no_setting: Manifest = {
        let f = files("0.1.1", &notifications(), "", |m| {
            m["contributes"].as_object_mut().unwrap().remove("settings");
        });
        Manifest::from_slice(&f[MANIFEST]).unwrap()
    };
    assert!(
        check_transition(&bundled, &no_setting)
            .unwrap_err()
            .contains("settings greeting")
    );
}

/// A crash in the middle of the transition: recovery leaves the bundled
/// copy active and intact, the floor stays raised, and the retry with the
/// same bytes completes; other bytes for that version stay refused.
#[test]
fn an_interrupted_transition_recovers_and_retries() {
    for point in [
        Failpoint::JournalWritten,
        Failpoint::FilesUnpacked,
        Failpoint::StepRecorded,
    ] {
        let env = Env::new();
        let bundle = env.bundle("0.1.0", &notifications());
        let manager = seeded(&env, &bundle);
        let before = state_of(&manager);
        drop(manager);
        let official = env.official();
        let host = host();
        let release = Release::new(&env, "0.1.1", &notifications());
        let package = official
            .verify(&release.delivery(Some(&bundle), &host))
            .unwrap();
        let crash = Arc::new(CrashOnce::new(Operation::Update, point));
        let mut crashing = env.open_with(ManagerOptions {
            failpoints: crash.clone(),
            ..ManagerOptions::default()
        });
        assert!(crashing.update(&package, &registry_source()).is_err());
        assert!(crash.fired(), "{point:?}");
        drop(crashing);

        let mut manager = env.open();
        manager.recover().unwrap();
        bundled::seed(&mut manager, &bundle);
        let recovered = state_of(&manager);
        assert_eq!(recovered.state, before.state, "{point:?}");
        let rolled_forward = recovered.source == registry_source();
        if rolled_forward {
            // Recovery completed the verified update: all of it.
            assert_eq!(recovered.active().unwrap().version, v("0.1.1"));
        } else {
            // Recovery undid it: the bundled copy, active and intact.
            assert_eq!(recovered.source, InstallSource::Bundled, "{point:?}");
            assert_eq!(recovered.active().unwrap().version, v("0.1.0"));
            deckpilot_plugin_bundled::verify_installed(
                &manager
                    .layout()
                    .resolve(&recovered.active().unwrap().install_dir),
                &bundle.pins.plugins[0],
            )
            .unwrap();
        }
        assert_eq!(
            manager.store().settings(&id()).unwrap()["greeting"],
            json!("Yo")
        );

        // The floor held through the crash.
        let restarted = env.official();
        assert_eq!(restarted.floor(&id()).unwrap().version, v("0.1.1"));
        let rebuilt = Release::with(&env, "0.1.1", &notifications(), "-rebuilt", |_| {});
        assert!(matches!(
            refusal(restarted.verify(&rebuilt.delivery(Some(&bundle), &host))),
            Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
        ));
        let again = restarted
            .verify(&release.delivery(Some(&bundle), &host))
            .unwrap();
        if !rolled_forward {
            manager.update(&again, &registry_source()).unwrap();
        }
        assert_eq!(state_of(&manager).source, registry_source());
        assert_eq!(
            manager.store().settings(&id()).unwrap()["greeting"],
            json!("Yo")
        );
    }
}

#[test]
fn a_refused_release_leaves_the_bundled_copy_untouched() {
    let env = Env::new();
    let bundle = env.bundle("0.1.0", &notifications());
    let manager = seeded(&env, &bundle);
    let before = state_of(&manager);
    let official = env.official();
    let host = host();
    let mut tampered = Release::new(&env, "0.1.1", &notifications());
    let last = tampered.package.len() - 1;
    tampered.package[last] ^= 1;
    refusal(official.verify(&tampered.delivery(Some(&bundle), &host)));
    assert_eq!(state_of(&manager), before);
    assert!(official.floor(&id()).is_none());
}

// ------------------------------------------------- review fixes (2026-11-21)

/// Install `version` (signed by `key`) from the registry, or update to it.
fn install_or_update(manager: &mut PluginManager, official: &Official, release: &Release) {
    let host = host();
    let package = official.verify(&release.delivery(None, &host)).unwrap();
    if manager.store().plugin(&id()).unwrap().is_some() {
        let outcome = manager.update(&package, &registry_source()).unwrap();
        assert!(
            matches!(outcome, UpdateOutcome::Applied { .. }),
            "{outcome:?}"
        );
    } else {
        let all: Vec<GrantKey> = GrantSet::from_manifest(package.manifest())
            .keys()
            .cloned()
            .collect();
        manager
            .install(&package, &registry_source(), &Consent::of(all, "user"))
            .unwrap();
    }
}

/// Finding 1: every verified version keeps its signers, so a rollback to an
/// older version signed by a now-revoked key is refused, not only the
/// floor version.
#[test]
fn a_rollback_to_a_release_signed_by_a_revoked_key_is_refused() {
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let mut manager = env.open();
    let both = env.official_with(keys(&[&env.key, &second], 1, &[]), Vec::new());
    install_or_update(
        &mut manager,
        &both,
        &Release::new(&env, "0.1.1", &notifications()),
    );
    let mut newer = Release::new(&env, "0.1.2", &notifications());
    newer.signature = second.sign(&newer.package, "0.1.2");
    install_or_update(&mut manager, &both, &newer);
    assert_eq!(state_of(&manager).active().unwrap().version, v("0.1.2"));

    // A later agent revokes the key that signed 0.1.1 only.
    let later = env.official_with(keys(&[&second], 1, &[&env.key]), Vec::new());
    let records = manager.store().plugins().unwrap();
    let revoked = later.revoked_installed(&records);
    assert_eq!(
        revoked.keys().cloned().collect::<Vec<_>>(),
        [(id(), v("0.1.1"))],
        "{revoked:?}"
    );
    let verdicts = Verdicts {
        official: &revoked,
        registry: &NoVerdicts,
    };
    let refused = manager.rollback(&id(), &verdicts);
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(state_of(&manager).active().unwrap().version, v("0.1.2"));
    // Without the revocation the rollback goes through.
    let none = BTreeMap::new();
    manager
        .rollback(
            &id(),
            &Verdicts {
                official: &none,
                registry: &NoVerdicts,
            },
        )
        .unwrap();
    assert_eq!(state_of(&manager).active().unwrap().version, v("0.1.1"));
    // ... and then the active version is the revoked one.
    let records = manager.store().plugins().unwrap();
    assert!(
        later
            .revoked_installed(&records)
            .contains_key(&(id(), v("0.1.1")))
    );
}

/// Finding 2: the floors compiled into the agent (released pins for this
/// platform, the artifact lock) hold on a fresh install with nothing
/// recorded and no bundled copy.
#[test]
fn compiled_floors_hold_on_a_fresh_install() {
    let env = Env::new();
    let host = host();
    let shipped = files("0.3.0", &notifications(), "", |_| {});
    let released_pins = json!({
        "releasedPinsVersion": 1,
        "releases": [{
            "release": "0.9.0",
            "platform": "linux",
            "plugins": [{
                "id": ID,
                "version": "0.3.0",
                "files": shipped
                    .iter()
                    .map(|(path, bytes)| (path.clone(), json!(digest(bytes))))
                    .collect::<serde_json::Map<_, _>>()
            }]
        }, {
            "release": "0.9.0",
            "platform": "macos",
            "plugins": [{ "id": ID, "version": "9.0.0", "files": {} }]
        }]
    });
    let empty_lock = br#"{"artifactsVersion":1,"plugins":[],"revoked":[]}"#;
    let compiled = Compiled::parse(
        empty_lock,
        &serde_json::to_vec(&released_pins).unwrap(),
        Some(Platform::Linux),
    )
    .unwrap();
    // Other platforms do not count.
    assert_eq!(compiled.released[&id()].version, v("0.3.0"));
    let official = env.official_compiled(keys(&[&env.key], 1, &[]), compiled);
    assert!(matches!(
        refusal(
            official.verify(&Release::new(&env, "0.2.9", &notifications()).delivery(None, &host))
        ),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
    assert!(matches!(
        refusal(official.verify(
            &Release::with(&env, "0.3.0", &notifications(), "-other", |_| {}).delivery(None, &host)
        )),
        Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
    ));
    official
        .verify(&Release::new(&env, "0.3.1", &notifications()).delivery(None, &host))
        .unwrap();

    // The lock: never below its release, at its version only its package.
    let locked = Release::new(&env, "0.5.0", &notifications());
    let lock = json!({
        "artifactsVersion": 1,
        "plugins": [{
            "id": ID,
            "version": "0.5.0",
            "sha256": digest(&locked.package).sha256.to_string(),
            "size": locked.package.len()
        }],
        "revoked": []
    });
    let other = Env::new();
    let compiled = Compiled::parse(
        &serde_json::to_vec(&lock).unwrap(),
        br#"{"releasedPinsVersion":1,"releases":[]}"#,
        Some(Platform::Linux),
    )
    .unwrap();
    let official = other.official_compiled(keys(&[&env.key], 1, &[]), compiled);
    let older = Release::new(&env, "0.4.9", &notifications());
    assert!(matches!(
        refusal(official.verify(&older.delivery(None, &host))),
        Refusal::Publisher(PublisherError::Rollback { .. })
    ));
    let rebuilt = Release::with(&env, "0.5.0", &notifications(), "-rebuilt", |_| {});
    assert!(matches!(
        refusal(official.verify(&rebuilt.delivery(None, &host))),
        Refusal::Publisher(PublisherError::SameVersionOtherFiles { .. })
    ));
    official.verify(&locked.delivery(None, &host)).unwrap();
}

#[test]
fn this_builds_compiled_data_parses() {
    Compiled::this_build().unwrap();
    for platform in [Platform::Macos, Platform::Windows, Platform::Linux] {
        Compiled::parse(
            ARTIFACT_LOCK.as_bytes(),
            RELEASED_PINS.as_bytes(),
            Some(platform),
        )
        .unwrap();
    }
}

/// Finding 7: revocation fails closed. A registry copy of an official
/// plugin this agent has no record of verifying (the floors file deleted)
/// or whose record cannot be read is not run.
#[test]
fn revocation_fails_closed_without_the_record() {
    let env = Env::new();
    let mut manager = env.open();
    let official = env.official();
    install_or_update(
        &mut manager,
        &official,
        &Release::new(&env, "0.1.1", &notifications()),
    );
    let records = manager.store().plugins().unwrap();
    assert!(official.revoked_installed(&records).is_empty());

    std::fs::remove_file(env.state().join(FLOORS_FILE)).unwrap();
    let deleted = env.official();
    let revoked = deleted.revoked_installed(&records);
    assert!(
        revoked[&(id(), v("0.1.1"))].reason.contains("no record"),
        "{revoked:?}"
    );

    std::fs::write(env.state().join(FLOORS_FILE), b"garbage").unwrap();
    let unreadable = env.official();
    let revoked = unreadable.revoked_installed(&records);
    assert!(
        revoked[&(id(), v("0.1.1"))].reason.contains("cannot read"),
        "{revoked:?}"
    );
    // The quarantine follows.
    let changes = manager
        .apply_verdicts(&Verdicts {
            official: &revoked,
            registry: &NoVerdicts,
        })
        .unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(state_of(&manager).state, PluginState::Quarantined);
}

/// Finding 8: refusals before verification are audited, and the rotation
/// keeps several generations, chained across files.
#[test]
fn early_refusals_are_audited_and_rotation_keeps_generations() {
    let env = Env::new();
    let official = env.official().with_audit_limit(600);
    official.refused_before_verification(
        &id(),
        Some(&v("0.1.1")),
        None,
        "refused before any download: test",
        Utc::now(),
    );
    let audit = env.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].record.outcome, publisher::AuditOutcome::Refused);
    assert!(
        audit[0]
            .record
            .reason
            .as_deref()
            .unwrap()
            .contains("before any download")
    );

    for n in 0..40 {
        official.refused_before_verification(&id(), None, None, &format!("line {n}"), Utc::now());
    }
    let generation = |n: usize| env.state().join(format!("{AUDIT_FILE}.{n}"));
    for n in 1..=AUDIT_GENERATIONS {
        assert!(generation(n).exists(), "generation {n}");
    }
    assert!(!generation(AUDIT_GENERATIONS + 1).exists());
    // Oldest kept generation first, the live file last: one chain.
    let mut prev: Option<String> = None;
    for path in (1..=AUDIT_GENERATIONS)
        .rev()
        .map(generation)
        .chain(std::iter::once(env.state().join(AUDIT_FILE)))
    {
        let text = std::fs::read_to_string(&path).unwrap();
        let first: ChainedLine = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        let start = prev.clone().unwrap_or(first.prev.clone());
        prev = Some(check_audit_chain(&text, &start).unwrap());
    }
}

const STATUS_FIXTURE: &str = "Test fixture: privileged.";

fn host_views_and_status_items(version: &str) -> Files {
    files(
        version,
        &json!([{ "kind": "hostViews", "justification": STATUS_FIXTURE }]),
        "",
        |m| {
            m["engines"]["pluginApi"] = json!("^1.9");
            m["contributes"]["statusItems"] = json!(["activeProject"]);
        },
    )
}

static HOST_VIEWS_ONLY: GateReview = GateReview {
    reviewer: "test",
    date: "2026-11-21",
    min_threshold: 2,
    plugins: &[(ID, &["hostViews"])],
};

static HOST_VIEWS_AND_STATUS: GateReview = GateReview {
    reviewer: "test",
    date: "2026-11-21",
    min_threshold: 2,
    plugins: &[(ID, &["hostViews", "`statusItems`"])],
};

/// Finding 6: the opening path limits every privileged construct, keeps the
/// registry's manifest and permission pins, and records the keys that
/// actually signed.
#[test]
fn the_opening_path_limits_every_construct_and_records_the_real_signers() {
    let env = Env::new();
    let second = Key::new(env.root.path(), "second.key");
    let third = Key::new(env.root.path(), "third.key");
    let package = raw_package(&host_views_and_status_items("0.1.1"));
    let expected = expected_for(&package, "0.1.1");
    // Two of the three pinned keys sign.
    let signatures = [
        env.key.sign(&package, "0.1.1"),
        second.sign(&package, "0.1.1"),
    ];
    let host = host();
    let delivery = |expected| Delivery {
        expected,
        package: &package,
        signatures: &signatures,
        pin: None,
        host: Some(&host),
        now: Utc::now(),
    };
    let gated = |gate| {
        env.official_with(keys(&[&env.key, &second, &third], 2, &[]), Vec::new())
            .with_gate(gate)
    };
    // `statusItems` is not in the review: refused.
    assert!(matches!(
        gated(PrivilegedGate::Open(&HOST_VIEWS_ONLY)).verify(&delivery(&expected)),
        Err(Refusal::Publisher(
            PublisherError::PrivilegedGateClosed { .. }
        ))
    ));
    // The registry's permissions are still checked on this path.
    let mut relabeled = expected.clone();
    relabeled.permissions = Some(Vec::new());
    assert!(matches!(
        gated(PrivilegedGate::Open(&HOST_VIEWS_AND_STATUS)).verify(&delivery(&relabeled)),
        Err(Refusal::Publisher(PublisherError::Package(_)))
    ));
    let mut other_manifest = expected.clone();
    other_manifest.manifest_sha256 = sha256(b"another manifest");
    assert!(matches!(
        gated(PrivilegedGate::Open(&HOST_VIEWS_AND_STATUS)).verify(&delivery(&other_manifest)),
        Err(Refusal::Publisher(PublisherError::Package(_)))
    ));
    // Everything named: admitted, with exactly the two signers.
    let open = gated(PrivilegedGate::Open(&HOST_VIEWS_AND_STATUS));
    open.verify(&delivery(&expected)).unwrap();
    let audit = env.audit();
    let recorded = &audit.last().unwrap().record.signers;
    let mut real = vec![
        publisher::fingerprint(&env.key.public()).unwrap(),
        publisher::fingerprint(&second.public()).unwrap(),
    ];
    real.sort();
    let mut recorded = recorded.clone();
    recorded.sort();
    assert_eq!(recorded, real);
    assert!(!recorded.contains(&publisher::fingerprint(&third.public()).unwrap()));
}

// ------------------------------------------------- re-review (N2, floors)

fn make_writable_and_flip(path: &Path) {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).unwrap();
    let mut bytes = std::fs::read(path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(path, bytes).unwrap();
}

/// N2: after the update from the bundle to a registry release, the
/// retained bundled version has no release record but is the bundled copy
/// this agent pins: it is trusted, and the rollback to it goes through. A
/// tampered retained copy is still refused, with an accurate reason.
#[test]
fn the_retained_bundled_copy_is_trusted_and_a_tampered_one_is_not() {
    for tampered in [false, true] {
        let env = Env::new();
        let bundle = env.bundle("0.1.0", &notifications());
        let mut manager = seeded(&env, &bundle);
        let official = env.official();
        let host = host();
        let release = Release::new(&env, "0.1.1", &notifications());
        let package = official
            .verify(&release.delivery(Some(&bundle), &host))
            .unwrap();
        manager.update(&package, &registry_source()).unwrap();
        let record = state_of(&manager);
        let previous = record.previous().unwrap().clone();
        assert_eq!(previous.version, v("0.1.0"));
        if tampered {
            make_writable_and_flip(
                &manager
                    .layout()
                    .resolve(&previous.install_dir)
                    .join("plugin.wasm"),
            );
        }
        let records = manager.store().plugins().unwrap();
        let withheld = official.withheld(&records, &|record, version| {
            bundled_match(&bundle, manager.layout(), record, version)
        });
        // Without the bundle to check against, it would have no record.
        let unchecked = official.revoked_installed(&records);
        assert_eq!(
            unchecked[&(id(), v("0.1.0"))].kind,
            WithheldKind::Unverified
        );
        let verdicts = Verdicts {
            official: &withheld,
            registry: &NoVerdicts,
        };
        if tampered {
            let why = &withheld[&(id(), v("0.1.0"))];
            assert_eq!(why.kind, WithheldKind::Unverified);
            assert_eq!(why.ipc_reason(), reason::INVALID_PACKAGE);
            assert!(
                why.reason.contains("not the bundled copy this agent pins"),
                "{why:?}"
            );
            assert!(!why.state_reason().starts_with("revoked"), "{why:?}");
            assert!(manager.rollback(&id(), &verdicts).is_err());
            assert_eq!(state_of(&manager).active().unwrap().version, v("0.1.1"));
        } else {
            assert!(withheld.is_empty(), "{withheld:?}");
            manager.rollback(&id(), &verdicts).unwrap();
            let rolled = state_of(&manager);
            assert_eq!(rolled.active().unwrap().version, v("0.1.0"));
            // Still trusted where it now runs.
            let records = manager.store().plugins().unwrap();
            assert!(
                official
                    .withheld(&records, &|record, version| {
                        bundled_match(&bundle, manager.layout(), record, version)
                    })
                    .is_empty()
            );
        }
    }
}

/// Info: a floors file with fields of a newer DeckPilot is read (its
/// records keep working for revocation) but never rewritten (official
/// installs are refused); another floorsVersion is refused entirely.
#[test]
fn floors_of_a_newer_agent_are_read_but_never_rewritten() {
    let env = Env::new();
    let mut manager = env.open();
    let official = env.official();
    install_or_update(
        &mut manager,
        &official,
        &Release::new(&env, "0.1.1", &notifications()),
    );
    let path = env.state().join(FLOORS_FILE);
    let mut floors: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    floors["newerRecords"] = json!({ "something": 1 });
    floors["plugins"][ID]["newerField"] = json!(true);
    let written = serde_json::to_vec_pretty(&floors).unwrap();
    std::fs::write(&path, &written).unwrap();

    let downgraded = env.official();
    let records = manager.store().plugins().unwrap();
    assert!(downgraded.revoked_installed(&records).is_empty());
    assert!(
        downgraded.ready().unwrap_err().contains("newer DeckPilot"),
        "{:?}",
        downgraded.ready()
    );
    let host = host();
    assert!(matches!(
        refusal(
            downgraded.verify(&Release::new(&env, "0.1.2", &notifications()).delivery(None, &host))
        ),
        Refusal::Unavailable(_)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), written, "never rewritten");

    floors["floorsVersion"] = json!(2);
    std::fs::write(&path, serde_json::to_vec(&floors).unwrap()).unwrap();
    let other_format = env.official();
    let withheld = other_format.revoked_installed(&records);
    assert!(
        withheld[&(id(), v("0.1.1"))].reason.contains("cannot read"),
        "{withheld:?}"
    );
}
