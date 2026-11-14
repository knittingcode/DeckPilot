//! Test fixture: an in-memory static registry site signed with throwaway
//! keys generated per test, real packages built with
//! `deckpilot-plugin-package`, and the bundled schemas.
#![allow(dead_code, clippy::unused_self)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, TimeDelta, TimeZone as _, Utc};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_package::{BuiltPackage, PackageBuilder, sha256};
use deckpilot_plugin_registry::sig::{index_comment, keys_comment, timestamp_comment};
use deckpilot_plugin_registry::*;
use serde_json::Value;

pub const EXAMPLE: &str = include_str!("../fixtures/example-manifest.json");

pub fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap()
}

pub fn days(n: i64) -> TimeDelta {
    TimeDelta::days(n)
}

pub fn id(text: &str) -> PluginId {
    PluginId::parse(text).unwrap()
}

pub fn v(text: &str) -> semver::Version {
    semver::Version::parse(text).unwrap()
}

// ------------------------------------------------------------------ keys

/// A minisign key pair that only ever lives in memory.
pub struct ThrowawayKey {
    keys: minisign::KeyPair,
    public: String,
}

impl ThrowawayKey {
    pub fn generate() -> Self {
        let keys = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let public = STANDARD.encode(keys.pk.to_box().unwrap().to_string());
        Self { keys, public }
    }

    /// Base64-wrapped minisign public key (the `DECKPILOT_UPDATER_PUBKEY`
    /// format).
    pub fn public(&self) -> &str {
        &self.public
    }

    /// The same key with another untrusted comment line: a different
    /// string for the same key.
    pub fn public_with_comment(&self, comment: &str) -> String {
        let text = String::from_utf8(STANDARD.decode(&self.public).unwrap()).unwrap();
        let key_line = text.lines().nth(1).unwrap();
        STANDARD.encode(format!("untrusted comment: {comment}\n{key_line}\n"))
    }

    /// Base64-wrapped minisign signature (Tauri format).
    pub fn sign(&self, data: &[u8], trusted_comment: &str) -> String {
        let signature = minisign::sign(
            None,
            &self.keys.sk,
            std::io::Cursor::new(data),
            Some(trusted_comment),
            None,
        )
        .unwrap();
        STANDARD.encode(signature.to_string())
    }
}

pub fn role(keys: &[&ThrowawayKey], threshold: u64) -> Role {
    Role {
        keys: keys.iter().map(|k| k.public().to_owned()).collect(),
        threshold,
    }
}

// ------------------------------------------------------------------ site

#[derive(Default)]
pub struct Site {
    pub files: RefCell<HashMap<String, Vec<u8>>>,
    pub offline: Cell<bool>,
}

impl RegistrySource for Site {
    fn fetch(&self, path: &str, _max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        if self.offline.get() {
            return Err(FetchError::Unavailable("offline".into()));
        }
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or(FetchError::NotFound)
    }
}

impl Site {
    pub fn put(&self, path: &str, bytes: impl Into<Vec<u8>>) {
        self.files
            .borrow_mut()
            .insert(path.to_owned(), bytes.into());
    }

    pub fn get(&self, path: &str) -> Vec<u8> {
        self.files.borrow()[path].clone()
    }

    pub fn remove(&self, path: &str) {
        self.files.borrow_mut().remove(path);
    }

    pub fn snapshot(&self) -> HashMap<String, Vec<u8>> {
        self.files.borrow().clone()
    }

    /// A mirror or attacker serving an old, validly signed timestamp.
    pub fn replay_timestamp(&self, snapshot: &HashMap<String, Vec<u8>>) {
        for path in ["timestamp.json", "timestamp.json.minisig"] {
            self.put(path, snapshot[path].clone());
        }
    }
}

// --------------------------------------------------------------- plugins

pub fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[4]
        })
        .collect()
}

pub fn manifest_json(plugin: &str, version: &str) -> Vec<u8> {
    let mut value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["id"] = Value::from(plugin);
    value["version"] = Value::from(version);
    value["publisher"]["id"] = Value::from(plugin.split('.').next().unwrap());
    serde_json::to_vec_pretty(&value).unwrap()
}

pub fn package(plugin: &str, version: &str) -> BuiltPackage {
    let mut builder = PackageBuilder::new();
    builder
        .add("manifest.json", manifest_json(plugin, version))
        .unwrap()
        .add(
            "plugin.js",
            format!("export function activate(ctx) {{ ctx.log('{plugin} {version}'); }}\n"),
        )
        .unwrap()
        .add("LICENSE", "MIT License\n")
        .unwrap()
        .add("README.md", "# Test plugin\n")
        .unwrap()
        .add("assets/icon-128.png", noise(200, 1))
        .unwrap()
        .add("assets/icon-32.png", noise(50, 2))
        .unwrap()
        .add("assets/keys/pr.png", noise(50, 3))
        .unwrap();
    builder.build().unwrap()
}

pub struct PluginVersion {
    pub version: String,
    pub built: BuiltPackage,
    pub yanked: Option<String>,
    pub platforms: Option<Vec<deckpilot_plugin_manifest::Platform>>,
}

pub struct PluginSpec {
    pub id: String,
    pub versions: Vec<PluginVersion>,
    pub deprecated: bool,
}

pub fn plugin(plugin_id: &str, versions: &[&str]) -> PluginSpec {
    PluginSpec {
        id: plugin_id.to_owned(),
        versions: versions
            .iter()
            .map(|version| PluginVersion {
                version: (*version).to_owned(),
                built: package(plugin_id, version),
                yanked: None,
                platforms: None,
            })
            .collect(),
        deprecated: false,
    }
}

pub fn release_url(plugin_id: &str, version: &str) -> String {
    format!("{DOWNLOAD_PREFIX}{plugin_id}--v{version}/{plugin_id}-{version}.dpplugin")
}

pub fn revocation(
    plugin_id: &str,
    versions: Option<&[&str]>,
    action: RevocationAction,
) -> Revocation {
    Revocation {
        id: id(plugin_id),
        versions: match versions {
            None => RevokedVersions::All,
            Some(list) => RevokedVersions::List(list.iter().map(|s| v(s)).collect()),
        },
        action,
        reason: "test revocation".into(),
        advisory: Some("https://example.invalid/advisory".into()),
        at: t0(),
    }
}

// --------------------------------------------------------------- registry

/// The registry side: keys, documents and the published site.
pub struct Registry {
    pub roots: Vec<ThrowawayKey>,
    pub root_threshold: u64,
    pub index_key: ThrowawayKey,
    pub timestamp_key: ThrowawayKey,
    pub keys_version: Cell<u64>,
    pub seq: u64,
    pub site: Site,
    pub timestamp_validity: TimeDelta,
    /// Every document published, for the schema tests.
    pub published: RefCell<Vec<(String, Vec<u8>)>>,
}

impl Registry {
    /// Two root keys, threshold 1, and `keys/1.json`.
    pub fn new() -> Self {
        let registry = Self {
            roots: vec![ThrowawayKey::generate(), ThrowawayKey::generate()],
            root_threshold: 1,
            index_key: ThrowawayKey::generate(),
            timestamp_key: ThrowawayKey::generate(),
            keys_version: Cell::new(0),
            seq: 0,
            site: Site::default(),
            timestamp_validity: days(7),
            published: RefCell::new(Vec::new()),
        };
        let root = registry.root_role();
        let doc = registry.keyset_doc(&root, 0, false);
        registry.publish_keyset(&doc, &[&registry.roots[0]]);
        registry
    }

    /// What the app embeds.
    pub fn embedded_roots(&self) -> Role {
        self.root_role()
    }

    pub fn root_role(&self) -> Role {
        role(&self.roots.iter().collect::<Vec<_>>(), self.root_threshold)
    }

    pub fn keyset_doc(&self, root: &Role, seq_floor: u64, reset: bool) -> Keyset {
        Keyset {
            kind: KeysetType::Value,
            spec_version: 1,
            version: self.keys_version.get() + 1,
            root: root.clone(),
            index: role(&[&self.index_key], 1),
            timestamp: role(&[&self.timestamp_key], 1),
            seq_floor,
            seq_floor_reset: reset,
        }
    }

    fn record(&self, path: &str, bytes: &[u8]) {
        self.published
            .borrow_mut()
            .push((path.to_owned(), bytes.to_vec()));
    }

    /// Writes `keys/<n>.json` and its signature array, signed by `signers`.
    pub fn publish_keyset(&self, doc: &Keyset, signers: &[&ThrowawayKey]) {
        let bytes = serde_json::to_vec_pretty(doc).unwrap();
        let comment = keys_comment(doc.version);
        let signatures: Vec<String> = signers.iter().map(|k| k.sign(&bytes, &comment)).collect();
        let path = format!("keys/{}.json", doc.version);
        self.record(&path, &bytes);
        self.site.put(&path, bytes);
        self.site.put(
            &format!("{path}.minisig"),
            serde_json::to_vec(&signatures).unwrap(),
        );
        self.keys_version.set(doc.version);
    }

    pub fn detail_doc(&self, spec: &PluginSpec, now: DateTime<Utc>) -> PluginDetail {
        PluginDetail {
            kind: DetailType::Value,
            spec_version: 1,
            id: id(&spec.id),
            publisher: spec.id.split('.').next().unwrap().to_owned(),
            name: "Test plugin".into(),
            description: "A test plugin".into(),
            license: "MIT".into(),
            homepage: Some("https://acme.example/plugin".into()),
            source_repository: Some("someone/test-plugin".into()),
            versions: spec
                .versions
                .iter()
                .map(|pv| {
                    let manifest = &pv.built.manifest;
                    let raw: Value =
                        serde_json::from_slice(&manifest_json(&spec.id, &pv.version)).unwrap();
                    DetailVersion {
                        version: v(&pv.version),
                        published_at: now,
                        engines: manifest.engines.clone(),
                        platforms: pv.platforms.clone(),
                        permissions: raw["permissions"].as_array().unwrap().clone(),
                        manifest_sha256: pv.built.integrity.manifest_sha256,
                        source: VersionSource {
                            repository: "someone/test-plugin".into(),
                            tag: format!("v{}", pv.version),
                            commit: "0".repeat(40),
                            path: None,
                            archive: None,
                        },
                        package: PackageRef {
                            url: release_url(&spec.id, &pv.version),
                            sha256: pv.built.integrity.package.sha256,
                            size: pv.built.integrity.package.size,
                        },
                        builder: v("1.0.0"),
                        attestation: None,
                        yanked: pv.yanked.as_ref().map(|reason| Yanked {
                            at: now,
                            reason: reason.clone(),
                        }),
                    }
                })
                .collect(),
        }
    }

    pub fn detail_bytes(&self, spec: &PluginSpec, now: DateTime<Utc>) -> Vec<u8> {
        serde_json::to_vec_pretty(&self.detail_doc(spec, now)).unwrap()
    }

    pub fn index_doc(
        &self,
        seq: u64,
        now: DateTime<Utc>,
        plugins: &[PluginSpec],
        revocations: &[Revocation],
    ) -> Index {
        let mut namespaces: Vec<String> = plugins
            .iter()
            .map(|p| p.id.split('.').next().unwrap().to_owned())
            .collect();
        namespaces.sort();
        namespaces.dedup();
        Index {
            kind: IndexType::Value,
            spec_version: 1,
            seq,
            keys_version: self.keys_version.get(),
            generated_at: now,
            expires: now + days(90),
            source_commit: "a".repeat(40),
            download_prefix: DOWNLOAD_PREFIX.into(),
            min_app_version: None,
            publishers: namespaces
                .iter()
                .enumerate()
                .map(|(i, namespace)| IndexPublisher {
                    namespace: namespace.clone(),
                    github_id: 1000 + i as u64,
                    login: namespace.clone(),
                    display_name: None,
                    verified: true,
                })
                .collect(),
            plugins: plugins
                .iter()
                .map(|p| {
                    let detail = self.detail_bytes(p, now);
                    let digest = sha256(&detail);
                    let stable = p
                        .versions
                        .iter()
                        .rev()
                        .find(|pv| pv.yanked.is_none())
                        .map(|pv| v(&pv.version));
                    IndexPlugin {
                        id: id(&p.id),
                        publisher: p.id.split('.').next().unwrap().to_owned(),
                        name: "Test plugin".into(),
                        summary: "A test plugin".into(),
                        categories: Some(vec!["developer-tools".into()]),
                        latest: Latest {
                            stable,
                            prerelease: None,
                        },
                        deprecated: p.deprecated.then(|| Deprecated {
                            at: now,
                            reason: "Use acme.other instead".into(),
                            replacement: Some(id("acme.other")),
                        }),
                        detail: FileRef {
                            path: format!("plugins/{}/{digest}.json", p.id),
                            sha256: digest,
                            length: detail.len() as u64,
                        },
                    }
                })
                .collect(),
            revocations: revocations.to_vec(),
        }
    }

    /// A full publish (`publish.yml`): details, `index/<seq+1>.json`, timestamp.
    pub fn publish(
        &mut self,
        now: DateTime<Utc>,
        plugins: &[PluginSpec],
        revocations: &[Revocation],
    ) -> u64 {
        let seq = self.seq + 1;
        let index = self.index_doc(seq, now, plugins, revocations);
        self.publish_index(&index, plugins, now, &self.index_key);
        self.seq = seq;
        self.sign_timestamp(now);
        seq
    }

    /// Writes an index signed by `signer`, and the details it references.
    pub fn publish_index(
        &self,
        index: &Index,
        plugins: &[PluginSpec],
        now: DateTime<Utc>,
        signer: &ThrowawayKey,
    ) {
        for p in plugins {
            let detail = self.detail_bytes(p, now);
            let path = format!("plugins/{}/{}.json", p.id, sha256(&detail));
            self.record(&path, &detail);
            self.site.put(&path, detail);
        }
        let bytes = serde_json::to_vec_pretty(index).unwrap();
        let comment = index_comment(index.seq, index.keys_version);
        let path = format!("index/{}.json", index.seq);
        self.record(&path, &bytes);
        self.site
            .put(&format!("{path}.minisig"), signer.sign(&bytes, &comment));
        self.site.put(&path, bytes);
    }

    /// `timestamp.yml`: re-sign the current seq.
    pub fn sign_timestamp(&self, now: DateTime<Utc>) {
        self.sign_timestamp_with(now, self.seq, self.keys_version.get(), &self.timestamp_key);
    }

    pub fn timestamp_bytes(&self, now: DateTime<Utc>, seq: u64, keys_version: u64) -> Vec<u8> {
        let path = format!("index/{seq}.json");
        let index = self.site.get(&path);
        let timestamp = Timestamp {
            kind: TimestampType::Value,
            spec_version: 1,
            seq,
            keys_version,
            signed_at: now,
            expires: now + self.timestamp_validity,
            index: FileRef {
                path,
                sha256: sha256(&index),
                length: index.len() as u64,
            },
        };
        serde_json::to_vec_pretty(&timestamp).unwrap()
    }

    pub fn sign_timestamp_with(
        &self,
        now: DateTime<Utc>,
        seq: u64,
        keys_version: u64,
        signer: &ThrowawayKey,
    ) {
        let bytes = self.timestamp_bytes(now, seq, keys_version);
        let comment = timestamp_comment(seq, keys_version);
        self.record("timestamp.json", &bytes);
        self.site
            .put("timestamp.json.minisig", signer.sign(&bytes, &comment));
        self.site.put("timestamp.json", bytes);
    }
}

// --------------------------------------------------------------- clients

pub type Client = RegistryClient<MemoryTrustStore>;

pub fn client(registry: &Registry) -> Client {
    RegistryClient::open(
        registry.embedded_roots(),
        MemoryTrustStore::default(),
        Policy::default(),
    )
    .unwrap()
}

/// The same client after a restart: state read back from its store.
pub fn restart(registry: &Registry, client: &Client) -> Client {
    RegistryClient::open(
        registry.embedded_roots(),
        client.store().clone(),
        *client.policy(),
    )
    .unwrap()
}

pub fn file_client(registry: &Registry, dir: &std::path::Path) -> RegistryClient<FileTrustStore> {
    RegistryClient::open(
        registry.embedded_roots(),
        FileTrustStore::new(dir),
        Policy::default(),
    )
    .unwrap()
}

// ---------------------------------------------------------------- schemas

struct ManifestSchema;

impl jsonschema::Retrieve for ManifestSchema {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let manifest: Value = serde_json::from_str(deckpilot_plugin_manifest::SCHEMA_JSON)?;
        if uri.as_str() == manifest["$id"].as_str().unwrap() {
            Ok(manifest)
        } else {
            Err(format!("unexpected schema reference {uri}").into())
        }
    }
}

pub fn validator(schema: &str) -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(schema).unwrap();
    jsonschema::options()
        .should_validate_formats(true)
        .with_retriever(ManifestSchema)
        .build(&schema)
        .expect("the bundled schema compiles")
}

pub fn schema_errors(validator: &jsonschema::Validator, bytes: &[u8]) -> Vec<String> {
    let instance: Value = serde_json::from_slice(bytes).unwrap();
    validator
        .iter_errors(&instance)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect()
}
