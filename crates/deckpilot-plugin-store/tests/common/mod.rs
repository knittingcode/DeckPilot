//! Fixtures: verified packages built on the fly, an environment with a
//! data directory and a key, verdicts, and comparable snapshots.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_package::{ExpectedPackage, PackageBuilder, VerifiedPackage, verify_package};
use deckpilot_plugin_registry::InstalledVerdict;
use deckpilot_plugin_store::{
    Failpoints, GrantSet, ManagerOptions, NoFailpoints, PluginHooks, PluginManager, StoreKey,
    VerdictSource,
};
use semver::Version;
use serde_json::{Value, json};

pub const ID: &str = "acme.github-prs";
const EXAMPLE: &str = include_str!("../fixtures/example-manifest.json");

pub fn id() -> PluginId {
    PluginId::parse(ID).unwrap()
}

/// The fixture's permissions: network (api.github.com, GET), credentials
/// (token), background (120 s), device state.
pub fn base_permissions() -> Vec<Value> {
    let value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["permissions"].as_array().unwrap().clone()
}

/// The base permissions plus notifications (an escalation).
pub fn with_notifications() -> Vec<Value> {
    let mut permissions = base_permissions();
    permissions.push(json!({
        "kind": "notifications",
        "justification": "Tells you when a review is requested."
    }));
    permissions
}

/// The base permissions without background (a narrowing).
pub fn without_background() -> Vec<Value> {
    base_permissions()
        .into_iter()
        .filter(|p| p["kind"] != "background")
        .collect()
}

fn manifest_json(id: &str, version: &str, permissions: &[Value]) -> Vec<u8> {
    let mut value: Value = serde_json::from_str(EXAMPLE).unwrap();
    value["id"] = Value::from(id);
    value["version"] = Value::from(version);
    value["publisher"]["id"] = Value::from(id.split('.').next().unwrap());
    value["permissions"] = Value::Array(permissions.to_vec());
    let mut text = serde_json::to_string_pretty(&value).unwrap();
    text.push('\n');
    text.into_bytes()
}

/// Deterministic incompressible bytes.
fn noise(len: usize, seed: u64) -> Vec<u8> {
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

/// The bytes of a package and what the registry would pin for them.
pub fn built(id: &str, version: &str, permissions: &[Value]) -> (Vec<u8>, ExpectedPackage) {
    let mut builder = PackageBuilder::new();
    builder
        .add("manifest.json", manifest_json(id, version, permissions))
        .unwrap()
        .add(
            "plugin.js",
            format!("export function activate(ctx) {{ ctx.log('{id} {version}'); }}\n"),
        )
        .unwrap()
        .add("LICENSE", b"MIT License\n".to_vec())
        .unwrap()
        .add("README.md", b"# GitHub PR Count\n".to_vec())
        .unwrap()
        .add("assets/icon-128.png", noise(300, 1))
        .unwrap()
        .add("assets/icon-32.png", noise(100, 2))
        .unwrap()
        .add("assets/keys/pr.png", noise(100, 3))
        .unwrap();
    let built = builder.build().unwrap();
    let expected = ExpectedPackage {
        id: built.manifest.id.clone(),
        version: built.manifest.version.clone(),
        package: built.integrity.package.clone(),
        manifest_sha256: built.integrity.manifest_sha256,
        permissions: Some(permissions.to_vec()),
    };
    (built.bytes, expected)
}

pub fn package_for(id: &str, version: &str, permissions: &[Value]) -> VerifiedPackage {
    let (bytes, expected) = built(id, version, permissions);
    verify_package(
        &bytes,
        &expected,
        &deckpilot_plugin_package::Limits::default(),
    )
    .unwrap()
}

pub fn package(version: &str, permissions: &[Value]) -> VerifiedPackage {
    package_for(ID, version, permissions)
}

pub fn grants_of(package: &VerifiedPackage) -> GrantSet {
    GrantSet::from_manifest(package.manifest())
}

pub fn key() -> StoreKey {
    StoreKey::from_bytes([0x5a; 32])
}

/// Counts secret clean-ups; optionally fails the first ones.
#[derive(Debug, Default)]
pub struct SecretHook {
    pub calls: Mutex<Vec<String>>,
    pub fail_first: Mutex<u32>,
}

impl PluginHooks for SecretHook {
    fn delete_secrets(&self, id: &PluginId) -> Result<(), String> {
        let mut fail = self.fail_first.lock().unwrap();
        if *fail > 0 {
            *fail -= 1;
            return Err("keystore unavailable".into());
        }
        self.calls.lock().unwrap().push(id.as_str().to_owned());
        Ok(())
    }
}

/// A data directory and the options to open it with.
#[derive(Debug)]
pub struct Env {
    pub dir: tempfile::TempDir,
    pub hook: Arc<SecretHook>,
}

impl Env {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            hook: Arc::new(SecretHook::default()),
        }
    }

    pub fn data_dir(&self) -> &Path {
        self.dir.path()
    }

    pub fn options(&self, failpoints: Arc<dyn Failpoints>) -> ManagerOptions {
        ManagerOptions {
            failpoints,
            hooks: self.hook.clone(),
            ..ManagerOptions::default()
        }
    }

    pub fn open(&self) -> PluginManager {
        self.open_with(Arc::new(NoFailpoints))
    }

    pub fn open_with(&self, failpoints: Arc<dyn Failpoints>) -> PluginManager {
        PluginManager::open(self.data_dir(), &key(), self.options(failpoints)).unwrap()
    }
}

/// Verdicts by (id, version); everything else runs.
#[derive(Debug, Default)]
pub struct Verdicts(pub HashMap<(String, String), InstalledVerdict>);

impl Verdicts {
    pub fn disable(id: &str, version: &str, reason: &str) -> Self {
        let mut map = HashMap::new();
        map.insert(
            (id.to_owned(), version.to_owned()),
            InstalledVerdict::Disable {
                reason: reason.to_owned(),
                advisory: Some("https://example.com/advisory".to_owned()),
            },
        );
        Self(map)
    }
}

impl VerdictSource for Verdicts {
    fn verdict(&self, id: &PluginId, version: &Version) -> InstalledVerdict {
        self.0
            .get(&(id.as_str().to_owned(), version.to_string()))
            .cloned()
            .unwrap_or(InstalledVerdict::Run)
    }
}

/// Every file and directory under `root`, relative, `/`-separated.
pub fn tree(root: &Path) -> BTreeSet<String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(relative);
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(root, root, &mut out);
    out
}

/// The plugin directory without the database and lock files.
pub fn plugin_files(manager: &PluginManager) -> BTreeSet<String> {
    tree(manager.layout().root())
        .into_iter()
        .filter(|path| !path.starts_with("plugins.db") && path != ".lock")
        .collect()
}

/// Everything observable about the store, without timestamps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub plugins: BTreeMap<String, (String, Vec<(String, String)>)>,
    pub grants: BTreeMap<String, Vec<(String, String, String)>>,
    pub files: BTreeSet<String>,
}

pub fn snapshot(manager: &PluginManager) -> Snapshot {
    let mut plugins = BTreeMap::new();
    let mut grants = BTreeMap::new();
    for plugin in manager.store().plugins().unwrap() {
        let mut versions: Vec<(String, String)> = plugin
            .versions
            .iter()
            .map(|v| (v.version.to_string(), v.role.as_str().to_owned()))
            .collect();
        versions.sort();
        plugins.insert(
            plugin.id.as_str().to_owned(),
            (plugin.state.as_str().to_owned(), versions),
        );
        let rows = manager
            .store()
            .grants(&plugin.id)
            .unwrap()
            .into_iter()
            .map(|g| {
                (
                    g.key.to_string(),
                    g.status.as_str().to_owned(),
                    serde_json::to_string(&g.scope).unwrap(),
                )
            })
            .collect();
        grants.insert(plugin.id.as_str().to_owned(), rows);
    }
    Snapshot {
        plugins,
        grants,
        files: plugin_files(manager),
    }
}

pub fn data_files(root: &Path) -> BTreeSet<PathBuf> {
    tree(root).into_iter().map(PathBuf::from).collect()
}
