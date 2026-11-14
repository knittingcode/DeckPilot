//! Test fixture: an in-memory static registry site, signed with throwaway
//! keys generated per test, plus plugin package builders.
#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use marketplace_trust_spike::client::{FetchError, Source};
use marketplace_trust_spike::package::{self, PackageFile};
use marketplace_trust_spike::registry::*;
use marketplace_trust_spike::sig;
use marketplace_trust_spike::signer::ThrowawayKey;
use time::{Duration, OffsetDateTime, macros::datetime};

pub const T0: OffsetDateTime = datetime!(2026-11-14 12:00 UTC);

#[derive(Default)]
pub struct Site {
    pub files: RefCell<HashMap<String, Vec<u8>>>,
    pub offline: Cell<bool>,
}

impl Source for Site {
    fn get(&self, path: &str) -> Result<Vec<u8>, FetchError> {
        if self.offline.get() {
            return Err(FetchError::Offline);
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
    pub fn get_bytes(&self, path: &str) -> Vec<u8> {
        self.files.borrow()[path].clone()
    }
    pub fn snapshot(&self) -> HashMap<String, Vec<u8>> {
        self.files.borrow().clone()
    }
    /// A mirror or MITM serving an old, validly signed timestamp.
    pub fn replay_timestamp(&self, snapshot: &HashMap<String, Vec<u8>>) {
        for p in ["timestamp.json", "timestamp.json.minisig"] {
            self.put(p, snapshot[p].clone());
        }
    }
}

pub fn role(keys: &[&ThrowawayKey], threshold: u32) -> Role {
    Role {
        keys: keys.iter().map(|k| k.public().to_owned()).collect(),
        threshold,
    }
}

pub struct PluginVersion {
    pub version: String,
    pub bytes: Vec<u8>,
    pub manifest: Vec<u8>,
    pub publisher_signature: Option<String>,
    pub yanked: Option<String>,
}

pub struct PluginSpec {
    pub id: String,
    pub publisher: String,
    pub versions: Vec<PluginVersion>,
}

pub fn manifest_bytes(id: &str, version: &str) -> Vec<u8> {
    format!(
        "{{\n  \"id\": \"{id}\",\n  \"version\": \"{version}\",\n  \"name\": \"Test plugin\",\n  \"permissions\": []\n}}\n"
    )
    .into_bytes()
}

/// Deterministic pseudo-random (incompressible) bytes.
pub fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 32) as u8
        })
        .collect()
}

pub fn plugin_files(id: &str, version: &str, asset_bytes: usize) -> Vec<PackageFile> {
    let mut files = vec![
        PackageFile::new("manifest.json", manifest_bytes(id, version)),
        PackageFile::new(
            "plugin.js",
            format!("export function activate(ctx) {{ ctx.log('{id} {version}'); }}\n"),
        ),
        PackageFile::new("LICENSE", "MIT License\n"),
        PackageFile::new("README.md", "# Test plugin\n"),
    ];
    if asset_bytes > 0 {
        files.push(PackageFile::new("assets/icon.png", noise(asset_bytes, 42)));
    }
    files
}

pub fn version(id: &str, v: &str) -> PluginVersion {
    let bytes = package::build(&plugin_files(id, v, 0)).unwrap();
    PluginVersion {
        version: v.to_owned(),
        bytes,
        manifest: manifest_bytes(id, v),
        publisher_signature: None,
        yanked: None,
    }
}

pub fn plugin(id: &str, versions: &[&str]) -> PluginSpec {
    PluginSpec {
        id: id.to_owned(),
        publisher: id.split('.').next().unwrap().to_owned(),
        versions: versions.iter().map(|v| version(id, v)).collect(),
    }
}

pub fn release_url(id: &str, v: &str) -> String {
    format!("{DOWNLOAD_PREFIX}{id}--v{v}/{id}-{v}.dpplugin")
}

pub fn revocation(id: &str, versions: Option<&[&str]>, action: RevocationAction) -> Revocation {
    Revocation {
        id: id.to_owned(),
        versions: match versions {
            None => RevokedVersions::All(AllMarker::All),
            Some(v) => RevokedVersions::List(v.iter().map(|s| (*s).to_owned()).collect()),
        },
        action,
        reason: "test revocation".into(),
        advisory: Some("https://example.invalid/advisory".into()),
        at: T0,
    }
}

/// The registry side: keys, documents and the published site.
pub struct Registry {
    pub roots: Vec<ThrowawayKey>,
    pub root_threshold: u32,
    pub index_key: ThrowawayKey,
    pub timestamp_key: ThrowawayKey,
    pub keys_version: Cell<u64>,
    pub seq: u64,
    pub site: Site,
    /// EXTENSION (U-2): publisher keys pinned in the index by namespace.
    pub publisher_keys: HashMap<String, String>,
}

impl Registry {
    /// Two root keys, threshold 1 (U-10 recommendation), and keys/1.json.
    pub fn new() -> Self {
        let r = Self {
            roots: vec![ThrowawayKey::generate(), ThrowawayKey::generate()],
            root_threshold: 1,
            index_key: ThrowawayKey::generate(),
            timestamp_key: ThrowawayKey::generate(),
            keys_version: Cell::new(0),
            seq: 0,
            site: Site::default(),
            publisher_keys: HashMap::new(),
        };
        let root = r.root_role();
        let doc = r.keyset_doc(&root, 0, false);
        r.publish_keyset(&doc, &[&r.roots[0]]);
        r
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
            kind: "deckpilot-registry-keys".into(),
            spec_version: 1,
            version: self.keys_version.get() + 1,
            root: root.clone(),
            index: role(&[&self.index_key], 1),
            timestamp: role(&[&self.timestamp_key], 1),
            seq_floor,
            seq_floor_reset: reset,
        }
    }

    /// Writes keys/<n>.json and its signature array, signed by `signers`.
    pub fn publish_keyset(&self, doc: &Keyset, signers: &[&ThrowawayKey]) {
        let bytes = serde_json::to_vec_pretty(doc).unwrap();
        let comment = sig::keys_comment(doc.version);
        let sigs: Vec<String> = signers.iter().map(|k| k.sign(&bytes, &comment)).collect();
        let path = format!("keys/{}.json", doc.version);
        self.site.put(&path, bytes);
        self.site.put(
            &format!("{path}.minisig"),
            serde_json::to_vec(&sigs).unwrap(),
        );
        self.keys_version.set(doc.version);
    }

    pub fn detail_bytes(&self, spec: &PluginSpec, now: OffsetDateTime) -> Vec<u8> {
        let detail = Detail {
            kind: "deckpilot-registry-plugin".into(),
            spec_version: 1,
            id: spec.id.clone(),
            publisher: spec.publisher.clone(),
            name: "Test plugin".into(),
            description: "A test plugin".into(),
            license: "MIT".into(),
            homepage: None,
            source_repository: Some("someone/test-plugin".into()),
            versions: spec
                .versions
                .iter()
                .map(|v| VersionEntry {
                    version: v.version.clone(),
                    published_at: now,
                    engines: serde_json::json!({"deckpilot": ">=0.7.0", "api": "^1.0.0"}),
                    platforms: None,
                    permissions: vec![],
                    manifest_sha256: sha256_hex(&v.manifest),
                    source: serde_json::json!({
                        "repository": "someone/test-plugin",
                        "tag": format!("v{}", v.version),
                        "commit": "0".repeat(40),
                    }),
                    package: PackageRef {
                        url: release_url(&spec.id, &v.version),
                        sha256: sha256_hex(&v.bytes),
                        size: v.bytes.len() as u64,
                    },
                    builder: "1.0.0".into(),
                    attestation: None,
                    yanked: v.yanked.as_ref().map(|reason| Yanked {
                        at: now,
                        reason: reason.clone(),
                    }),
                    publisher_signature: v.publisher_signature.clone(),
                })
                .collect(),
        };
        serde_json::to_vec_pretty(&detail).unwrap()
    }

    pub fn index_doc(
        &self,
        seq: u64,
        now: OffsetDateTime,
        plugins: &[PluginSpec],
        revocations: &[Revocation],
    ) -> Index {
        let mut namespaces: Vec<String> = plugins.iter().map(|p| p.publisher.clone()).collect();
        namespaces.sort();
        namespaces.dedup();
        Index {
            kind: "deckpilot-registry-index".into(),
            spec_version: 1,
            seq,
            keys_version: self.keys_version.get(),
            generated_at: now,
            expires: now + Duration::days(90),
            source_commit: "a".repeat(40),
            download_prefix: DOWNLOAD_PREFIX.into(),
            min_app_version: None,
            publishers: namespaces
                .iter()
                .enumerate()
                .map(|(i, ns)| Publisher {
                    namespace: ns.clone(),
                    github_id: 1000 + i as u64,
                    login: ns.clone(),
                    display_name: None,
                    verified: true,
                    signing_key: self.publisher_keys.get(ns).cloned(),
                })
                .collect(),
            plugins: plugins
                .iter()
                .map(|p| {
                    let detail = self.detail_bytes(p, now);
                    let hash = sha256_hex(&detail);
                    PluginEntry {
                        id: p.id.clone(),
                        publisher: p.publisher.clone(),
                        name: "Test plugin".into(),
                        summary: "A test plugin".into(),
                        categories: None,
                        latest: Latest {
                            stable: p.versions.last().map(|v| v.version.clone()),
                            prerelease: None,
                        },
                        deprecated: None,
                        detail: FileRef {
                            path: format!("plugins/{}/{hash}.json", p.id),
                            sha256: hash,
                            length: detail.len() as u64,
                        },
                    }
                })
                .collect(),
            revocations: revocations.to_vec(),
        }
    }

    /// Full publish (publish.yml): details, index/<seq+1>.json, timestamp.
    pub fn publish(
        &mut self,
        now: OffsetDateTime,
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

    /// Writes an index (signed by `signer`) and the details it references.
    pub fn publish_index(
        &self,
        index: &Index,
        plugins: &[PluginSpec],
        now: OffsetDateTime,
        signer: &ThrowawayKey,
    ) {
        for p in plugins {
            let detail = self.detail_bytes(p, now);
            self.site.put(
                &format!("plugins/{}/{}.json", p.id, sha256_hex(&detail)),
                detail,
            );
        }
        let bytes = serde_json::to_vec_pretty(index).unwrap();
        let comment = sig::seq_comment("index", index.seq, index.keys_version);
        let path = format!("index/{}.json", index.seq);
        self.site
            .put(&format!("{path}.minisig"), signer.sign(&bytes, &comment));
        self.site.put(&path, bytes);
    }

    /// timestamp.yml: re-sign the current seq with a 7-day expiry.
    pub fn sign_timestamp(&self, now: OffsetDateTime) {
        self.sign_timestamp_with(now, self.seq, self.keys_version.get(), &self.timestamp_key);
    }

    pub fn sign_timestamp_with(
        &self,
        now: OffsetDateTime,
        seq: u64,
        keys_version: u64,
        signer: &ThrowawayKey,
    ) {
        let path = format!("index/{seq}.json");
        let index = self.site.get_bytes(&path);
        let ts = Timestamp {
            kind: "deckpilot-registry-timestamp".into(),
            spec_version: 1,
            seq,
            keys_version,
            signed_at: now,
            expires: now + Duration::days(7),
            index: FileRef {
                path,
                sha256: sha256_hex(&index),
                length: index.len() as u64,
            },
        };
        let bytes = serde_json::to_vec_pretty(&ts).unwrap();
        let comment = sig::seq_comment("timestamp", seq, keys_version);
        self.site
            .put("timestamp.json.minisig", signer.sign(&bytes, &comment));
        self.site.put("timestamp.json", bytes);
    }
}
