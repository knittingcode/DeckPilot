//! The plugin marketplace end to end, through the configuration app: the
//! fixture (an isolated data directory, a signed registry served over
//! HTTPS, canned HTTP for the sample plugin) and the scenario, written
//! once against [`Driver`].
//!
//! Two drivers run it:
//!
//! * `tests/marketplace_gui.rs` drives the built app through `WebDriver`
//!   (`tauri-driver`; Linux and Windows): the real webview, the real Tauri
//!   IPC, the agent the app starts itself.
//! * `src/marketplace_e2e.rs` calls the app's commands through Tauri's
//!   mock runtime (every platform, the only one on macOS, which has no
//!   `WebDriver` for `WKWebView`): the same commands, arguments and agent,
//!   without the webview.
//!
//! Every step reports how it was done ([`Via`]): through the UI, through
//! the app's IPC (`invoke` of the command the frontend would call),
//! through the agent's IPC (the simulated Stream Deck has no UI), or as a
//! fixture (data written from outside, e.g. plugin storage, which the
//! sample plugin never writes itself).
//!
//! Nothing here touches the user's configuration or credential store: the
//! data directory is a fresh temporary one (`DECKPILOT_DATA_DIR`), the
//! configuration is protected by a password (no keychain item), the
//! registry is signed with throwaway keys and served with a throwaway CA
//! only the agent under test trusts, and the plugin's HTTP requests are
//! answered from canned responses (`DECKPILOT_TEST_PLUGIN_HTTP`).
//!
//! The artifacts come from the environment (see `docs/marketplace/e2e.md`):
//! `DECKPILOT_E2E_AGENT_BIN` (an agent built with `plugins,test-hooks`),
//! `DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR` (the sample's
//! `hello_weather.wasm`).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstance, ActionStatus, ControlBinding, DeviceLayout, ExecutionRecord, ExecutionState,
    Profile, ProfileId, Trigger, Workflow,
};
use deckpilot_device::{ControlId, ControlInput, DeviceId};
use deckpilot_ipc::{AgentClient, AgentEvent, AgentPaths, ClientKind, Request, Response};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_registry::publish::{Io, run as registry_cli};
use deckpilot_plugin_store::{PluginData, PluginStore, StorageQuota, StoreKey};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, DatabaseKey, Encryption, SqliteStore};
use serde_json::{Value, json};

#[path = "../../../../agent/tests/support/https.rs"]
mod https;

pub const HELLO: &str = "acme.hello-weather";
pub const NAME: &str = "Hello Weather";
pub const NETWORK: &str = "network:api.open-meteo.com";
pub const CREDENTIAL: &str = "credentials:token";
pub const NOTIFICATIONS: &str = "notifications";
pub const GREET: &str = "greet";
pub const TEMPERATURE: &str = "current-temperature";
pub const GREET_TYPE: &str = "acme.hello-weather.greet";
pub const TEMPERATURE_TYPE: &str = "acme.hello-weather.current-temperature";
/// The configuration's password. Only for these tests.
pub const PASSWORD: &str = "marketplace e2e password";
/// The secret stored for the plugin's credential slot. It must never come
/// back from the agent.
pub const SECRET: &str = "e2e-3c1f-weather-token";
/// The canned forecast for Berlin.
pub const BERLIN: &str = "https://api.open-meteo.com/v1/forecast?latitude=52.52";
/// What the sample plugin keeps in its storage (written by the fixture).
pub const STORED_KEY: &str = "lastCity";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn required_path(name: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(name)
            .unwrap_or_else(|| panic!("{name} is not set (see docs/marketplace/e2e.md)")),
    );
    assert!(path.exists(), "{name}: {} does not exist", path.display());
    std::path::absolute(&path).unwrap()
}

/// Where screenshots, logs and the step report go: `DECKPILOT_E2E_ARTIFACTS`
/// or the fixture's directory.
pub fn artifacts_dir(fallback: &Path) -> PathBuf {
    let dir = std::env::var_os("DECKPILOT_E2E_ARTIFACTS")
        .map_or_else(|| fallback.join("artifacts"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ------------------------------------------------------------------ report

/// How a step was carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// Clicks and typing in the app's windows.
    Ui,
    /// The app's command, invoked as the frontend invokes it.
    AppIpc,
    /// A request straight to the agent (the simulated Stream Deck).
    AgentIpc,
    /// Data written or read from outside, by the test.
    Fixture,
}

impl Via {
    fn label(self) -> &'static str {
        match self {
            Self::Ui => "UI",
            Self::AppIpc => "app IPC",
            Self::AgentIpc => "agent IPC",
            Self::Fixture => "fixture",
        }
    }
}

/// The steps as they were done, written to `<artifacts>/<driver>-steps.txt`
/// after each one (a failure leaves the steps before it).
#[derive(Debug)]
pub struct Report {
    path: PathBuf,
    lines: Vec<String>,
}

impl Report {
    pub fn new(artifacts: &Path, driver: &str) -> Self {
        let path = artifacts.join(format!("{driver}-steps.txt"));
        let _ = std::fs::remove_file(&path);
        Self {
            path,
            lines: Vec::new(),
        }
    }

    pub fn step(&mut self, number: u32, title: &str, via: &[Via], note: &str) {
        let mut how: Vec<&str> = via.iter().map(|v| v.label()).collect();
        how.dedup();
        let line = format!(
            "{number:>2}. {title}: {}{}",
            how.join(" + "),
            if note.is_empty() {
                String::new()
            } else {
                format!(" ({note})")
            }
        );
        eprintln!("[marketplace e2e] {line}");
        self.lines.push(line);
        let _ = std::fs::write(&self.path, self.lines.join("\n") + "\n");
    }
}

// ----------------------------------------------------------------- registry

fn registry_run(dir: &Path, args: &[&str]) -> String {
    let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    if args.first().is_none_or(|c| *c != "keys" && *c != "package") {
        all.push("--dir".into());
        all.push(dir.display().to_string());
    }
    let env = |_: &str| None;
    let mut input: &[u8] = b"";
    let mut out = Vec::new();
    let mut io = Io {
        env: &env,
        stdin: &mut input,
        out: &mut out,
    };
    registry_cli(&all, &mut io).unwrap_or_else(|e| panic!("deckpilot-registry {args:?}: {e}"));
    String::from_utf8(out).unwrap()
}

/// The sample's manifest as `version`, with a setting and a credential
/// slot added: the sample declares neither, and the scenario configures
/// both. 1.1.0 also asks for notifications (an escalation).
pub fn manifest(version: &str) -> Value {
    let mut manifest: Value = serde_json::from_slice(
        &std::fs::read(repo().join("plugins/examples/hello-weather/manifest.json")).unwrap(),
    )
    .unwrap();
    manifest["version"] = json!(version);
    let permissions = manifest["permissions"].as_array_mut().unwrap();
    permissions.push(json!({
        "kind": "credentials", "refs": ["token"],
        "justification": "Signs in to the weather service."
    }));
    if version != "1.0.0" {
        permissions.push(json!({
            "kind": "notifications",
            "justification": "Tells you when the temperature changes a lot."
        }));
    }
    manifest["contributes"]["credentials"] = json!([{
        "key": "token", "label": "API token", "hosts": ["api.open-meteo.com"],
        "schemes": ["bearer"]
    }]);
    manifest["contributes"]["settings"] = json!({"fields": [{
        "key": "defaultCity", "label": "Default city", "required": false,
        "kind": {"type": "choice", "options": ["berlin", "london", "new-york", "tokyo"]},
        "default": {"type": "choice", "value": "berlin"}
    }]});
    manifest
}

/// A local registry signed with throwaway keys by the publishing tool
/// (`deckpilot-registry`, called as a library with the CLI's arguments).
#[derive(Debug)]
pub struct Registry {
    /// `site/v1/`, `releases/`: what the HTTPS server serves.
    pub work: PathBuf,
    /// Secret keys, plugin sources, packages and the trusted roots.
    pub keys: PathBuf,
    wasm: Vec<u8>,
}

impl Registry {
    fn key(&self, name: &str) -> String {
        self.keys.join(name).display().to_string()
    }

    fn run(&self, args: &[&str]) -> String {
        registry_run(&self.work, args)
    }

    fn create(root: &Path, wasm: Vec<u8>) -> Self {
        let registry = Self {
            work: root.join("registry"),
            keys: root.join("registry-keys"),
            wasm,
        };
        std::fs::create_dir_all(&registry.work).unwrap();
        std::fs::create_dir_all(&registry.keys).unwrap();
        for role in ["root", "index", "timestamp"] {
            registry.run(&[
                "keys",
                "generate",
                "--role",
                role,
                "--out",
                &registry.key(&format!("{role}.key")),
                "--no-password",
            ]);
        }
        registry.run(&[
            "keyset",
            "init",
            "--root",
            &registry.key("root.key.pub"),
            "--index",
            &registry.key("index.key.pub"),
            "--timestamp",
            &registry.key("timestamp.key.pub"),
        ]);
        registry.run(&["keyset", "sign", "--key", &registry.key("root.key")]);
        registry.run(&[
            "publisher",
            "--namespace",
            "acme",
            "--github-id",
            "1001",
            "--login",
            "acme",
            "--verified",
        ]);
        registry.publish("1.0.0");
        std::fs::copy(registry.work.join("roots.json"), registry.roots()).unwrap();
        registry
    }

    /// Adds `version` of the sample and signs a new index and timestamp.
    pub fn publish(&self, version: &str) {
        let manifest = manifest(version);
        let src = self.keys.join("src").join(version);
        std::fs::create_dir_all(src.join("assets")).unwrap();
        std::fs::write(
            src.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let sample = repo().join("plugins/examples/hello-weather");
        std::fs::copy(sample.join("LICENSE"), src.join("LICENSE")).unwrap();
        std::fs::copy(
            sample.join("assets/icon-128.png"),
            src.join("assets/icon-128.png"),
        )
        .unwrap();
        let wasm = src.join("module.wasm");
        std::fs::write(&wasm, &self.wasm).unwrap();
        let package = self.keys.join(format!("hello-{version}.dpplugin"));
        let packaged = self.run(&[
            "package",
            "--plugin-dir",
            &src.display().to_string(),
            "--module",
            &wasm.display().to_string(),
            "--out",
            &package.display().to_string(),
        ]);
        assert!(packaged.contains("packaged"), "{packaged}");
        let detail = self.keys.join("plugin.json");
        std::fs::write(
            &detail,
            serde_json::to_vec(&json!({
                "id": HELLO, "name": NAME, "summary": "Sample plugin",
                "description": "Greets someone and shows the temperature of a city.",
                "license": "Apache-2.0", "homepage": "https://acme.example/plugins",
                "source_repository": "acme/plugins", "categories": ["other"]
            }))
            .unwrap(),
        )
        .unwrap();
        self.run(&[
            "add-version",
            "--package",
            &package.display().to_string(),
            "--plugin-detail",
            &detail.display().to_string(),
            "--source-repository",
            "acme/plugins",
            "--tag",
            &format!("v{version}"),
            "--commit",
            &"1".repeat(40),
            "--builder",
            "1.0.0",
        ]);
        self.run(&["build-index", "--source-commit", &"a".repeat(40)]);
        self.run(&["sign-index", "--key", &self.key("index.key")]);
        self.run(&["timestamp", "--key", &self.key("timestamp.key")]);
        let verified = self.run(&["verify-site", "--require-packages"]);
        assert!(verified.contains("site accepted"), "{verified}");
    }

    /// The root keys the agent trusts, kept apart from the registry.
    pub fn roots(&self) -> PathBuf {
        self.keys.join("trusted-roots.json")
    }
}

// ------------------------------------------------------------------ fixture

/// A temporary directory, removed when dropped unless
/// `DECKPILOT_E2E_KEEP` is set.
#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "deckpilot-{name}-{}",
            deckpilot_core::ProfileId::new()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(std::path::absolute(path).unwrap())
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if std::env::var_os("DECKPILOT_E2E_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Everything one run of the scenario needs.
pub struct Fixture {
    pub root: TempDir,
    pub paths: AgentPaths,
    /// The configuration database's key (from the password).
    pub key: DatabaseKey,
    pub registry: Registry,
    server: https::HttpsServer,
    pub ca: PathBuf,
    pub http: PathBuf,
    pub agent_bin: PathBuf,
    pub plugin_host: PathBuf,
    /// The profile the scenario edits (active, with an MK.2 layout).
    pub profile: ProfileId,
    /// A profile nothing touches: it must come out unchanged.
    pub baseline: ProfileId,
    pub artifacts: PathBuf,
    runtime: tokio::runtime::Runtime,
    /// Agents this test started itself (the command-layer driver).
    agents: Vec<Child>,
    spawned: u32,
}

impl std::fmt::Debug for Fixture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fixture")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Canned answers for the only host the sample talks to.
fn canned_http(dir: &Path) -> PathBuf {
    let path = dir.join("http.json");
    let forecast = json!({
        "current": {"temperature_2m": 21.5, "time": "2026-11-14T12:00"},
        "current_units": {"temperature_2m": "°C"}
    });
    std::fs::write(
        &path,
        serde_json::to_vec(&json!([{
            "url": BERLIN,
            "status": 200,
            "headers": [["content-type", "application/json"]],
            "body": forecast.to_string()
        }]))
        .unwrap(),
    )
    .unwrap();
    path
}

/// The plugin store's key: HKDF-SHA256 of the configuration key, as the
/// agent derives it (`apps/agent/src/plugins/key.rs`; that module is only
/// part of agents built with plugins). A wrong derivation cannot pass: the
/// store would not open.
fn plugin_store_key(config: &DatabaseKey) -> StoreKey {
    let hkdf =
        hkdf::Hkdf::<sha2::Sha256>::new(Some(b"deckpilot.plugin-store.v1"), config.as_bytes());
    let mut okm = [0u8; 32];
    hkdf.expand(b"deckpilot plugins.db sqlcipher key", &mut okm)
        .unwrap();
    StoreKey::from_bytes(okm)
}

impl Fixture {
    pub fn new(name: &str) -> Self {
        let agent_bin = required_path("DECKPILOT_E2E_AGENT_BIN");
        let plugin_host = required_path("DECKPILOT_E2E_PLUGIN_HOST");
        let wasm =
            std::fs::read(required_path("DECKPILOT_E2E_WASM_DIR").join("hello_weather.wasm"))
                .expect("hello_weather.wasm");
        let root = TempDir::new(name);
        let artifacts = artifacts_dir(root.path());
        let registry = Registry::create(root.path(), wasm);
        let server = https::HttpsServer::start(&registry.work);
        let ca = registry.keys.join("ca.pem");
        server.write_ca(&ca);
        let http = canned_http(root.path());

        // A password-protected configuration: no credential store involved.
        let paths = AgentPaths::in_dir(root.path().join("data"));
        paths.create_dirs().unwrap();
        let key = Vault::new(&paths.database)
            .with_test_cost()
            .initialize(&Password::new(PASSWORD))
            .unwrap();
        let store = SqliteStore::open(&paths.database, &Encryption::Key(key.clone())).unwrap();
        let mut baseline = Profile::new("Baseline");
        baseline.description = "Must come out of the marketplace scenario unchanged".into();
        let mut wait = Workflow::new("Baseline wait");
        wait.graph = Some(
            serde_json::from_value(json!({"version": 1,
                "nodes": [{"id": "start", "type": "trigger"},
                          {"id": "wait", "type": "delay", "ms": 10}],
                "edges": [{"from": "start", "to": "wait"}]}))
            .unwrap(),
        );
        baseline.workflows.push(wait);
        store.save_profile(&baseline).unwrap();
        let mut profile = Profile::new("Marketplace");
        let layout = DeviceLayout::new("Main", deckpilot_device::models::stream_deck_mk2().id);
        let layout_id = layout.id;
        profile.layouts.push(layout);
        profile.activate_layout(layout_id).unwrap();
        store.save_profile(&profile).unwrap();
        store.set_active_profile(Some(profile.id)).unwrap();
        drop(store);

        Self {
            root,
            paths,
            key,
            registry,
            server,
            ca,
            http,
            agent_bin,
            plugin_host,
            profile: profile.id,
            baseline: baseline.id,
            artifacts,
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap(),
            agents: Vec::new(),
            spawned: 0,
        }
    }

    /// The environment the agent needs. The app passes its own on to the
    /// agent it starts.
    pub fn agent_env(&self) -> Vec<(&'static str, OsString)> {
        vec![
            ("DECKPILOT_DATA_DIR", self.paths.data_dir.clone().into()),
            ("DECKPILOT_ENABLE_PLUGINS", "1".into()),
            ("DECKPILOT_PLUGIN_HOST", self.plugin_host.clone().into()),
            (
                "DECKPILOT_PLUGIN_REGISTRY_URL",
                self.server.url("site/").into(),
            ),
            (
                "DECKPILOT_PLUGIN_REGISTRY_RELEASES_URL",
                self.server.url("releases/").into(),
            ),
            (
                "DECKPILOT_PLUGIN_REGISTRY_ROOTS",
                self.registry.roots().into(),
            ),
            ("DECKPILOT_TEST_REGISTRY_CA", self.ca.clone().into()),
            ("DECKPILOT_TEST_PLUGIN_HTTP", self.http.clone().into()),
        ]
    }

    /// What the agent runs with: no tray, no USB, a simulated MK.2.
    pub const AGENT_ARGS: &'static str = "--headless --no-usb --mock-devices mk2 --log-level debug";

    /// Starts the agent as the app would, with [`Self::AGENT_ARGS`] (the
    /// command-layer driver: the mock runtime has no environment of its
    /// own to pass on).
    pub fn spawn_agent(&mut self) {
        self.spawned += 1;
        let log = std::fs::File::create(
            self.artifacts
                .join(format!("agent-stderr-{}.log", self.spawned)),
        )
        .unwrap();
        let mut command = Command::new(&self.agent_bin);
        command
            .args(Self::AGENT_ARGS.split_whitespace())
            .envs(self.agent_env())
            .env_remove("DECKPILOT_TEST_AGENT_ARGS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        self.agents.push(command.spawn().expect("the agent starts"));
        self.wait_for_agent(true);
    }

    pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }

    /// One request straight to the agent.
    pub fn agent(&self, request: Request) -> Result<Response, deckpilot_ipc::ClientError> {
        let what = request.name();
        self.block_on(async {
            tokio::time::timeout(Duration::from_secs(60), async {
                AgentClient::connect(&self.paths, ClientKind::Other)
                    .await?
                    .request(request)
                    .await
            })
            .await
            .unwrap_or_else(|_| panic!("{what} took too long"))
        })
    }

    pub fn agent_running(&self) -> bool {
        self.block_on(AgentClient::connect(&self.paths, ClientKind::Other))
            .is_ok()
    }

    /// Waits until the agent accepts connections (or is gone).
    pub fn wait_for_agent(&self, running: bool) {
        wait_until(Duration::from_secs(60), "the agent state", || {
            (self.agent_running() == running).then_some(())
        });
    }

    /// Unlocks the running agent with the configuration's key.
    pub fn unlock_agent(&self) {
        self.agent(Request::Unlock {
            key: deckpilot_ipc::KeyMaterial::new(self.key.to_hex()),
        })
        .expect("the agent unlocks");
    }

    /// Records that the user confirmed app `version` (the setting the
    /// "after updates" policy checks), as an earlier start would have.
    pub fn confirm_app_version(&self, version: &str) {
        let store = SqliteStore::open(&self.paths.database, &Encryption::Key(self.key.clone()))
            .expect("the configuration opens with its key");
        store
            .set_setting("security.confirmedVersion", version)
            .unwrap();
    }

    /// Whether the agent has the configuration open.
    pub fn agent_unlocked(&self) -> bool {
        matches!(
            self.agent(Request::GetStatus),
            Ok(Response::Status { status })
                if status.security == deckpilot_ipc::SecurityState::Unlocked
        )
    }

    /// The simulated MK.2, once connected.
    pub fn mock_device(&self) -> DeviceId {
        wait_until(Duration::from_secs(30), "the simulated MK.2", || {
            let Ok(Response::Status { status }) = self.agent(Request::GetStatus) else {
                return None;
            };
            status
                .devices
                .iter()
                .find(|d| {
                    d.source == "mock" && d.state == deckpilot_device::ConnectionState::Connected
                })
                .map(|d| d.id.clone())
        })
    }

    /// Presses and releases key `index` of the simulated MK.2; the name and
    /// status of the action it ran (`actionCompleted`).
    pub fn press(&self, index: u16) -> (String, ActionStatus) {
        let device = self.mock_device();
        self.block_on(async {
            let mut events = AgentClient::connect(&self.paths, ClientKind::Other)
                .await
                .unwrap();
            events.subscribe().await.unwrap();
            for input in [ControlInput::Press, ControlInput::Release] {
                AgentClient::connect(&self.paths, ClientKind::Other)
                    .await
                    .unwrap()
                    .request(Request::SimulateInput {
                        device: device.clone(),
                        control: ControlId::Key { index },
                        input,
                    })
                    .await
                    .unwrap();
            }
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            loop {
                let event = tokio::time::timeout_at(deadline, events.next_event())
                    .await
                    .expect("an action completed in time")
                    .unwrap();
                if let AgentEvent::ActionCompleted { name, status, .. } = event {
                    return (name, status);
                }
            }
        })
    }

    /// Stops the agent (best effort) and waits until it is gone.
    pub fn stop_agent(&mut self) {
        let _ = self.agent(Request::Shutdown);
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.agent_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        for mut child in self.agents.drain(..) {
            let _ = child.wait();
        }
    }

    /// Every URL the canned HTTP answered, in order.
    pub fn fetched(&self) -> Vec<String> {
        let mut log = self.http.as_os_str().to_owned();
        log.push(".log");
        std::fs::read_to_string(PathBuf::from(log))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Every path the registry server was asked for.
    pub fn registry_log(&self) -> Vec<String> {
        self.server.log()
    }

    fn plugin_db(&self) -> PathBuf {
        deckpilot_plugin_store::Layout::new(&self.paths.data_dir).database()
    }

    /// `plugins.db`, opened from outside (the agent allows readers).
    pub fn plugin_store(&self) -> PluginStore {
        PluginStore::open(
            &self.plugin_db(),
            &plugin_store_key(&self.key),
            Duration::from_secs(10),
        )
        .unwrap()
    }

    /// Writes a value into the plugin's storage, as the plugin would with
    /// `storage.set` (the sample never does).
    pub fn seed_storage(&self) {
        let mut data = PluginData::open(
            &self.plugin_db(),
            &plugin_store_key(&self.key),
            Duration::from_secs(10),
        )
        .unwrap();
        data.storage_set(
            &hello(),
            STORED_KEY,
            &json!("berlin"),
            StorageQuota {
                bytes: 256 * 1024,
                keys: 1000,
            },
        )
        .unwrap();
    }

    /// The plugin's storage keys, credential slots with a secret and saved
    /// settings, read from `plugins.db`.
    pub fn plugin_data(&self) -> (Vec<String>, Vec<String>, BTreeMap<String, Value>) {
        let store = self.plugin_store();
        let id = hello();
        (
            store.storage_keys(&id).unwrap(),
            store
                .credentials(&id)
                .unwrap()
                .into_iter()
                .map(|c| c.key)
                .collect(),
            store.settings(&id).unwrap(),
        )
    }

    /// A profile as stored, read from the SQLCipher configuration database
    /// with its key.
    pub fn stored_profile(&self, id: ProfileId) -> Value {
        let store = SqliteStore::open(&self.paths.database, &Encryption::Key(self.key.clone()))
            .expect("the configuration opens with its key");
        let profile = store.load_profile(id).unwrap().expect("the profile exists");
        serde_json::to_value(profile).unwrap()
    }

    /// The configuration database is encrypted: another key, or none,
    /// does not open it.
    pub fn assert_encrypted(&self) {
        let wrong = DatabaseKey::from_bytes([7; 32]);
        assert!(
            SqliteStore::open(&self.paths.database, &Encryption::Key(wrong)).is_err(),
            "the configuration opened with a wrong key"
        );
        let bytes = std::fs::read(&self.paths.database).unwrap();
        assert!(
            !bytes.starts_with(b"SQLite format 3"),
            "the configuration database is plain SQLite"
        );
        for name in ["Baseline", "Marketplace", SECRET] {
            assert!(
                !bytes.windows(name.len()).any(|w| w == name.as_bytes()),
                "`{name}` is readable in the configuration file"
            );
        }
    }

    /// Copies the agent's logs to the artifacts.
    pub fn collect_logs(&self) {
        if let Ok(entries) = std::fs::read_dir(&self.paths.log_dir) {
            for entry in entries.flatten() {
                let _ = std::fs::copy(entry.path(), self.artifacts.join(entry.file_name()));
            }
        }
        let _ = std::fs::write(
            self.artifacts.join("canned-http.log"),
            self.fetched().join("\n"),
        );
        let _ = std::fs::write(
            self.artifacts.join("registry-server.log"),
            self.registry_log().join("\n"),
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.collect_logs();
        // Never leave an agent behind: the app starts it detached.
        self.stop_agent();
    }
}

pub fn hello() -> PluginId {
    PluginId::parse(HELLO).unwrap()
}

/// Polls `check` until it returns `Some`, for at most `timeout`.
pub fn wait_until<T>(timeout: Duration, what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

// ------------------------------------------------------------------- driver

/// One way of using the app. The defaults use the app's commands, as the
/// frontend invokes them; the GUI driver does what it can through the UI.
pub trait Driver {
    /// The name in reports and artifacts.
    fn name(&self) -> &'static str;

    /// Invokes an app command with the frontend's argument names.
    fn invoke(&mut self, command: &str, args: Value) -> Result<Value, Value>;

    /// Starts the app (and with it the agent) and unlocks the
    /// configuration.
    fn start(&mut self, fx: &mut Fixture) -> Vec<Via>;

    /// Stops the agent, closes the app, starts both again and unlocks.
    fn restart(&mut self, fx: &mut Fixture) -> Vec<Via>;

    /// Saves a screenshot (GUI only).
    fn screenshot(&mut self, _name: &str) {}

    fn ok(&mut self, command: &str, args: Value) -> Value {
        self.invoke(command, args)
            .unwrap_or_else(|error| panic!("{command} failed: {error}"))
    }

    fn open_marketplace(&mut self) -> Vec<Via> {
        let runtime = self.ok("get_plugin_runtime", json!({}));
        assert_eq!(runtime["available"], true, "{runtime}");
        vec![Via::AppIpc]
    }

    fn discover(&mut self) -> Vec<Via> {
        vec![Via::AppIpc]
    }

    fn inspect(&mut self) -> Vec<Via> {
        vec![Via::AppIpc]
    }

    fn install(&mut self, consent: &[&str]) -> Vec<Via> {
        self.ok(
            "install_plugin",
            json!({"source": {"type": "registry", "plugin": HELLO, "version": "1.0.0"},
                   "consent": consent}),
        );
        vec![Via::AppIpc]
    }

    fn enable(&mut self, consent: &[&str]) -> Vec<Via> {
        self.ok(
            "enable_plugin",
            json!({"plugin": HELLO, "consent": consent}),
        );
        vec![Via::AppIpc]
    }

    /// Sets the setting `defaultCity` to `tokyo` and stores [`SECRET`] for
    /// the credential slot `token` (Configure).
    fn configure(&mut self) -> Vec<Via> {
        self.ok(
            "set_plugin_settings",
            json!({"plugin": HELLO, "values": {"defaultCity": "tokyo"}}),
        );
        let slots = self.ok(
            "set_plugin_credential",
            json!({"plugin": HELLO, "key": "token", "secret": SECRET}),
        );
        assert!(!slots.to_string().contains(SECRET), "the secret came back");
        vec![Via::AppIpc]
    }

    /// Runs one of the plugin's actions from the marketplace; the text of
    /// the result.
    fn run_plugin_action(&mut self, action: &str, params: Value) -> (Vec<Via>, String) {
        let outcome = self.ok(
            "invoke_plugin_action",
            json!({"plugin": HELLO, "action": action, "params": params}),
        );
        (vec![Via::AppIpc], outcome.to_string())
    }

    fn update(&mut self, consent: &[&str]) -> Vec<Via> {
        let staged = self.ok(
            "update_plugin",
            json!({"plugin": HELLO, "version": null, "consent": []}),
        );
        assert_eq!(staged["outcome"]["type"], "awaitingConsent", "{staged}");
        self.ok(
            "approve_plugin_update",
            json!({"plugin": HELLO, "consent": consent}),
        );
        vec![Via::AppIpc]
    }

    fn rollback(&mut self) -> Vec<Via> {
        self.ok("rollback_plugin", json!({"plugin": HELLO}));
        vec![Via::AppIpc]
    }

    fn revoke(&mut self, permission: &str) -> Vec<Via> {
        self.ok(
            "revoke_plugin_permission",
            json!({"plugin": HELLO, "permission": permission}),
        );
        vec![Via::AppIpc]
    }

    /// Checks in the UI that the action is unavailable; `None` without UI.
    fn denied_in_ui(&mut self) -> Option<Vec<Via>> {
        None
    }

    fn disable(&mut self) -> Vec<Via> {
        self.ok("disable_plugin", json!({"plugin": HELLO}));
        vec![Via::AppIpc]
    }

    fn uninstall(&mut self) -> Vec<Via> {
        self.ok("uninstall_plugin", json!({"plugin": HELLO}));
        vec![Via::AppIpc]
    }
}

fn details(d: &mut dyn Driver) -> Value {
    d.ok("get_plugin", json!({"plugin": HELLO}))
}

fn permission(details: &Value, key: &str) -> String {
    details["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == key)
        .map_or_else(
            || "absent".to_owned(),
            |p| p["status"].as_str().unwrap().to_owned(),
        )
}

fn profile_of(d: &mut dyn Driver, id: ProfileId) -> Value {
    // Through `Profile`, so that numbers compare alike whether they came
    // through the webview's JSON (`0`) or straight from Rust (`0.0`).
    let profile: Profile =
        serde_json::from_value(d.ok("get_profile", json!({"profileId": id}))["profile"].clone())
            .unwrap();
    serde_json::to_value(profile).unwrap()
}

fn settings_of(d: &mut dyn Driver) -> Value {
    d.ok("get_plugin_settings", json!({"plugin": HELLO}))
}

fn credential_set(d: &mut dyn Driver) -> bool {
    let slots = d.ok("list_plugin_credentials", json!({"plugin": HELLO}));
    assert!(!slots.to_string().contains(SECRET), "a secret came back");
    slots
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["key"] == "token" && s["set"] == true)
}

/// Runs an action instance through the agent's engine (`run_action`).
fn run_action(
    d: &mut dyn Driver,
    profile: ProfileId,
    action: &Value,
) -> deckpilot_core::ActionResult {
    serde_json::from_value(d.ok(
        "run_action",
        json!({"profileId": profile, "actionId": action["id"]}),
    ))
    .unwrap()
}

/// The configured data the plugin keeps through a restart, an update and
/// a rollback: settings, the credential, storage and the grants.
fn assert_plugin_data_kept(d: &mut dyn Driver, fx: &Fixture, version: &str) {
    let plugin = details(d);
    assert_eq!(plugin["summary"]["version"], version, "{plugin}");
    assert_eq!(plugin["summary"]["state"], "enabled", "{plugin}");
    assert_eq!(permission(&plugin, NETWORK), "granted");
    assert_eq!(permission(&plugin, CREDENTIAL), "granted");
    let settings = settings_of(d);
    assert_eq!(settings["values"]["defaultCity"], "tokyo", "{settings}");
    assert!(credential_set(d), "the credential is gone");
    let (storage, credentials, saved) = fx.plugin_data();
    assert_eq!(storage, [STORED_KEY]);
    assert_eq!(credentials, ["token"]);
    assert_eq!(saved.get("defaultCity"), Some(&json!("tokyo")));
}

fn assert_execution(d: &mut dyn Driver, execution: &Value) -> ExecutionRecord {
    let id = execution["id"].clone();
    wait_until(Duration::from_secs(60), "the workflow run", || {
        let record: ExecutionRecord =
            serde_json::from_value(d.ok("get_execution", json!({"executionId": id}))).unwrap();
        (record.summary.state != ExecutionState::Running).then_some(record)
    })
}

/// The whole scenario, steps 1 to 22.
#[allow(clippy::too_many_lines)] // one path, step by step
pub fn run(d: &mut dyn Driver, fx: &mut Fixture) {
    let mut report = Report::new(&fx.artifacts, d.name());
    let baseline_before = fx.stored_profile(fx.baseline);

    // 1. The app starts the agent on an isolated, password-protected data
    // directory and is unlocked.
    let via = d.start(fx);
    assert!(fx.agent_unlocked(), "the agent is not unlocked");
    let runtime = d.ok("get_plugin_runtime", json!({}));
    assert_eq!(runtime["available"], true, "{runtime}");
    assert_eq!(runtime["executionSupported"], true, "{runtime}");
    report.step(
        1,
        "Start the app (and the agent) with an isolated data dir",
        &via,
        "",
    );
    d.screenshot("01-started");

    // 2. Settings › Extensions › Plugins.
    let via = d.open_marketplace();
    report.step(2, "Open the Marketplace", &via, "");

    // 3. Discover: the registry over HTTPS lists the sample.
    let mut via = d.discover();
    let available = d.ok("list_available_plugins", json!({"refresh": false}));
    via.push(Via::AppIpc);
    assert_eq!(available["registry"]["freshness"], "fresh", "{available}");
    assert!(
        available["registry"]["source"]
            .as_str()
            .unwrap()
            .starts_with("https://127.0.0.1:"),
        "{available}"
    );
    let entry = available["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == HELLO)
        .cloned()
        .unwrap_or_else(|| panic!("{HELLO} is not listed: {available}"));
    assert_eq!(entry["latest"], "1.0.0");
    assert_eq!(entry["publisherInfo"]["verified"], true, "{entry}");
    assert!(
        fx.registry_log()
            .iter()
            .any(|p| p.contains("timestamp.json")),
        "{:?}",
        fx.registry_log()
    );
    report.step(
        3,
        "Discover Hello Weather",
        &via,
        "registry served over local HTTPS",
    );
    d.screenshot("03-discover");

    // 4. Details: description, permissions, versions.
    let via = d.inspect();
    let version = &entry["versions"][0];
    assert_eq!(version["version"], "1.0.0");
    assert!(
        version["incompatible"].as_array().is_none_or(Vec::is_empty),
        "{version}"
    );
    let declared = version["permissions"].to_string();
    assert!(declared.contains("api.open-meteo.com"), "{declared}");
    report.step(4, "Inspect its details", &via, "");
    d.screenshot("04-details");

    // 5. Install 1.0.0 with consent to its permissions: installed,
    // disabled, nothing fetched yet.
    let via = d.install(&[NETWORK, CREDENTIAL]);
    let plugin = details(d);
    assert_eq!(plugin["summary"]["version"], "1.0.0", "{plugin}");
    assert_eq!(plugin["summary"]["state"], "disabled", "{plugin}");
    assert!(fx.registry_log().iter().any(|p| p.ends_with(".dpplugin")));
    assert!(
        fx.fetched().is_empty(),
        "the plugin ran before it was enabled"
    );
    report.step(5, "Install it", &via, "");
    d.screenshot("05-installed");

    // 6. Enable it, granting every permission it declares.
    let via = d.enable(&[NETWORK, CREDENTIAL]);
    let plugin = details(d);
    assert_eq!(plugin["summary"]["state"], "enabled", "{plugin}");
    assert_eq!(permission(&plugin, NETWORK), "granted");
    assert_eq!(permission(&plugin, CREDENTIAL), "granted");
    report.step(6, "Grant permissions (enable)", &via, "");
    d.screenshot("06-enabled");

    // 7. Configure: a setting, the credential's secret; storage the
    // plugin would write itself.
    let mut via = d.configure();
    let settings = settings_of(d);
    assert_eq!(settings["effective"]["defaultCity"], "tokyo", "{settings}");
    assert!(credential_set(d), "the credential is not set");
    // The agent validates against the manifest.
    let refused = d
        .invoke(
            "set_plugin_settings",
            json!({"plugin": HELLO, "values": {"defaultCity": "paris"}}),
        )
        .expect_err("an invalid setting was accepted");
    assert_eq!(refused["reason"], "invalidData", "{refused}");
    via.push(Via::AppIpc);
    fx.seed_storage();
    via.push(Via::Fixture);
    report.step(
        7,
        "Configure",
        &via,
        "an invalid value is refused (app IPC); storage written into plugins.db because the \
         sample never writes storage",
    );

    // 8. Actions of the plugin in the profile, bound to keys of the
    // simulated MK.2.
    let greet = d.ok(
        "create_action",
        json!({"profileId": fx.profile, "action": {
            "name": "Greet Ada", "actionType": GREET_TYPE,
            "parameters": {"name": {"type": "text", "value": "Ada"}}}}),
    );
    let weather = d.ok(
        "create_action",
        json!({"profileId": fx.profile, "action": {
            "name": "Berlin weather", "actionType": TEMPERATURE_TYPE,
            "parameters": {"city": {"type": "choice", "value": "berlin"}}}}),
    );
    let profile = profile_of(d, fx.profile);
    let mut layout: DeviceLayout = serde_json::from_value(profile["layouts"][0].clone()).unwrap();
    let ids: Vec<ActionInstance> = [&greet, &weather]
        .iter()
        .map(|a| serde_json::from_value((*a).clone()).unwrap())
        .collect();
    layout.pages[0].bindings = ids
        .iter()
        .zip(0u16..)
        .map(|(action, index)| {
            ControlBinding::new(
                ControlId::Key { index },
                Trigger::Press,
                deckpilot_core::BindingTarget::action(action.id),
            )
        })
        .collect();
    // The save returns once the agent has read it, so the keys work now.
    let saved = d.ok(
        "save_layout",
        json!({"profileId": fx.profile, "layout": layout}),
    );
    assert_eq!(saved["agentReload"]["state"], "applied", "{saved}");
    report.step(
        8,
        "Assign the action to a key",
        &[Via::AppIpc],
        "create_action + save_layout: dragging onto a key in the Layout Designer is not \
         driven (pointer drag and drop through WebDriver is not reliable)",
    );

    // 9. Run it: from the marketplace, and from a key of the simulated
    // MK.2 (the engine, in the sandboxed host).
    let (mut via, text) = d.run_plugin_action(GREET, json!({"name": "Ada"}));
    assert!(text.contains("Hello, Ada."), "{text}");
    let (more, text) = d.run_plugin_action(TEMPERATURE, json!({"city": "berlin"}));
    via.extend(more);
    assert!(text.contains("21.5"), "{text}");
    assert_eq!(
        fx.press(0),
        ("Greet Ada".to_owned(), ActionStatus::Succeeded)
    );
    assert_eq!(
        fx.press(1),
        ("Berlin weather".to_owned(), ActionStatus::Succeeded)
    );
    via.push(Via::AgentIpc);
    report.step(
        9,
        "Execute the action",
        &via,
        "key presses on the agent's simulated MK.2 (SimulateInput)",
    );
    d.screenshot("09-ran");

    // 10. A workflow node running the plugin action.
    let mut flow = Workflow::new("Weather flow");
    flow.graph = Some(
        serde_json::from_value(json!({"version": 1,
            "nodes": [{"id": "start", "type": "trigger"},
                      {"id": "temp", "type": "action", "action": weather["id"]}],
            "edges": [{"from": "start", "to": "temp"}]}))
        .unwrap(),
    );
    // As the layout: the workflow can be started right after the save.
    let flow = d.ok(
        "save_workflow",
        json!({"profileId": fx.profile, "workflow": flow}),
    );
    assert_eq!(flow["agentReload"]["state"], "applied", "{flow}");
    let started = d.ok(
        "start_workflow",
        json!({"profileId": fx.profile, "workflowId": flow["id"]}),
    );
    let record = assert_execution(d, &started);
    assert_eq!(
        record.summary.state,
        ExecutionState::Succeeded,
        "{record:#?}"
    );
    let node = record
        .nodes
        .iter()
        .find(|n| n.node.as_str() == "temp")
        .expect("the plugin node ran");
    assert_eq!(node.output["temperature"], "21.5", "{node:#?}");
    report.step(
        10,
        "Execute it in a workflow",
        &[Via::AppIpc],
        "save_workflow + start_workflow",
    );

    // 11. What went out, and how the plugin is doing.
    let requests = fx.fetched();
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert!(
        requests.iter().all(|r| r.starts_with(BERLIN)),
        "{requests:?}"
    );
    assert!(
        requests.iter().all(|r| !r.contains(SECRET)),
        "the secret went to a request that did not ask for it"
    );
    let plugin = details(d);
    assert_eq!(
        plugin["summary"]["health"]["crashes"].as_u64().unwrap_or(0),
        0,
        "{plugin}"
    );
    let result = run_action(d, fx.profile, &greet);
    assert_eq!(result.status, ActionStatus::Succeeded, "{result:?}");
    assert_eq!(result.message, "Hello, Ada.");
    report.step(
        11,
        "Verify the results",
        &[Via::AppIpc, Via::Fixture],
        "canned HTTP log",
    );
    let profile_before = profile_of(d, fx.profile);
    let stored_before = fx.stored_profile(fx.profile);

    // 12. Restart the agent and the app.
    let via = d.restart(fx);
    report.step(12, "Restart the agent and the app", &via, "");
    d.screenshot("12-restarted");

    // 13. Everything is still there and works.
    assert_plugin_data_kept(d, fx, "1.0.0");
    assert_eq!(profile_of(d, fx.profile), profile_before);
    assert_eq!(
        fx.press(0),
        ("Greet Ada".to_owned(), ActionStatus::Succeeded)
    );
    report.step(
        13,
        "Verify persistence",
        &[Via::AppIpc, Via::AgentIpc, Via::Fixture],
        "",
    );

    // 14. 1.1.0 asks for notifications too: staged until consented.
    fx.registry.publish("1.1.0");
    let via = d.update(&[NOTIFICATIONS]);
    let plugin = details(d);
    assert_eq!(plugin["summary"]["version"], "1.1.0", "{plugin}");
    assert_eq!(plugin["summary"]["previousVersion"], "1.0.0", "{plugin}");
    assert_eq!(permission(&plugin, NOTIFICATIONS), "granted");
    report.step(14, "Update to 1.1.0 (adds notifications)", &via, "");
    d.screenshot("14-updated");

    // 15. Its data came along.
    assert_plugin_data_kept(d, fx, "1.1.0");
    let result = run_action(d, fx.profile, &greet);
    assert_eq!(result.status, ActionStatus::Succeeded, "{result:?}");
    report.step(
        15,
        "Verify state migration",
        &[Via::AppIpc, Via::Fixture],
        "",
    );

    // 16. Back to 1.0.0, data kept.
    let via = d.rollback();
    assert_plugin_data_kept(d, fx, "1.0.0");
    report.step(16, "Roll back", &via, "");
    d.screenshot("16-rolled-back");

    // 17. Revoke the network permission.
    let via = d.revoke(NETWORK);
    let plugin = details(d);
    assert_eq!(permission(&plugin, NETWORK), "revoked", "{plugin}");
    assert_eq!(plugin["summary"]["state"], "enabled", "{plugin}");
    report.step(17, "Revoke a permission", &via, "");

    // 18. Nothing that needs it runs, and nothing goes out.
    let mut via = d.denied_in_ui().unwrap_or_default();
    let before = fx.fetched().len();
    for action in plugin["summary"]["actions"].as_array().unwrap() {
        assert_eq!(action["available"], false, "{action}");
        let why = action["unavailableReason"].as_str().unwrap_or_default();
        assert!(
            why.contains(NETWORK) && why.contains("not granted"),
            "{why}"
        );
    }
    let refused = run_action(d, fx.profile, &weather);
    assert_eq!(refused.status, ActionStatus::Failed, "{refused:?}");
    assert!(refused.message.contains("not granted"), "{refused:?}");
    let invoked = d.invoke(
        "invoke_plugin_action",
        json!({"plugin": HELLO, "action": TEMPERATURE, "params": {"city": "berlin"}}),
    );
    assert!(
        invoked.as_ref().is_err() || invoked.as_ref().is_ok_and(|o| o["status"] != "ok"),
        "{invoked:?}"
    );
    via.push(Via::AppIpc);
    assert_eq!(
        fx.press(1),
        ("Berlin weather".to_owned(), ActionStatus::Failed)
    );
    via.push(Via::AgentIpc);
    assert_eq!(fx.fetched().len(), before, "a denied request went out");
    report.step(18, "Verify the denied operation", &via, "");
    d.screenshot("18-denied");

    // 19. Disable.
    let via = d.disable();
    let plugin = details(d);
    assert_eq!(plugin["summary"]["state"], "disabled", "{plugin}");
    let refused = run_action(d, fx.profile, &greet);
    assert_eq!(refused.status, ActionStatus::Failed, "{refused:?}");
    assert!(refused.message.contains("disabled"), "{refused:?}");
    report.step(19, "Disable", &via, "");

    // 20. Uninstall (the default: nothing kept).
    let via = d.uninstall();
    let installed = d.ok("list_plugins", json!({}));
    assert!(
        installed
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["id"] != HELLO),
        "{installed}"
    );
    report.step(20, "Uninstall", &via, "");
    d.screenshot("20-uninstalled");

    // 21. Storage, credentials and settings are gone; plugin_known keeps
    // its name for the instances that refer to it.
    let (storage, credentials, saved) = fx.plugin_data();
    assert!(storage.is_empty(), "{storage:?}");
    assert!(credentials.is_empty(), "{credentials:?}");
    assert!(saved.is_empty(), "{saved:?}");
    let known = d.ok("list_known_plugins", json!({}));
    let entry = known
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["id"] == HELLO)
        .unwrap_or_else(|| panic!("plugin_known lost it: {known}"));
    assert_eq!(entry["installed"], false, "{entry}");
    assert_eq!(entry["name"], NAME, "{entry}");
    let refused = run_action(d, fx.profile, &weather);
    assert!(
        refused
            .message
            .contains("Hello Weather (acme.hello-weather): the plugin is not installed"),
        "{refused:?}"
    );
    report.step(
        21,
        "Verify cleanup",
        &[Via::Fixture, Via::AppIpc],
        "plugins.db read with the derived key",
    );

    // 22. The profiles, workflows and the encrypted configuration.
    assert_eq!(profile_of(d, fx.profile), profile_before);
    assert_eq!(fx.stored_profile(fx.profile), stored_before);
    assert_eq!(fx.stored_profile(fx.baseline), baseline_before);
    assert_eq!(
        profile_of(d, fx.baseline),
        fx.stored_profile(fx.baseline),
        "the app and the database disagree"
    );
    fx.assert_encrypted();
    let mut log = String::new();
    for line in fx.fetched() {
        let _ = writeln!(log, "{line}");
    }
    assert!(!log.contains(SECRET));
    report.step(
        22,
        "Profiles, workflows and the encrypted config intact",
        &[Via::AppIpc, Via::Fixture],
        "SQLCipher database opened with its key",
    );
}
