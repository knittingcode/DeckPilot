//! The plugin marketplace backend end to end (feature `plugins`):
//!
//! * the real agent executable (`--enable-plugins`), driven over IPC
//!   protocol 10 like the configuration app;
//! * the real `deckpilot-plugin-host` and the sample plugin
//!   (`plugins/examples/hello-weather`), both built from `plugins/runtime`
//!   by this test (or taken from `DECKPILOT_E2E_PLUGIN_HOST` and
//!   `DECKPILOT_E2E_WASM_DIR`);
//! * a local registry signed with throwaway keys by the publishing tool
//!   (`deckpilot-registry` as a library), holding the sample twice (1.0.0,
//!   and 1.1.0 with an extra permission) and a few plugins that must be
//!   refused;
//! * HTTP through canned responses (`DECKPILOT_TEST_PLUGIN_HTTP`, feature
//!   `test-hooks`): only what the broker allowed reaches them, and every
//!   URL is logged, so the test sees exactly what went "out".
//!
//! On Windows the host sandbox is not verified: installing works, enabling
//! and running are refused (`windows_refuses_to_run_plugins`).
#![cfg(feature = "plugins")]
// Most of the flow runs only where plugins may run (macOS, Linux).
#![cfg_attr(windows, allow(dead_code, unused_imports))]

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use base64::Engine as _;
use deckpilot_ipc::plugins::{
    PermissionStatus, PluginActionOutcome, PluginDetails, PluginEvent, PluginInstallSource,
    PluginOperation, PluginStage, PluginState, PluginUpdateOutcome, RegistryFreshness, reason,
};
use deckpilot_ipc::{
    AgentClient, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_registry::publish::{Io, run};
use deckpilot_storage::DatabaseKey;
use serde_json::{Value, json};
use support::TempDir;

const HELLO: &str = "acme.hello-weather";
const CRASHY: &str = "acme.sdk-test";
const FUTURE: &str = "acme.future-weather";
const REVOKED: &str = "acme.revoked-weather";
const NETWORK: &str = "network:api.open-meteo.com";
const STARTUP: Duration = Duration::from_secs(30);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime() -> PathBuf {
    repo().join("plugins/runtime")
}

// ------------------------------------------------------------ artifacts

struct Artifacts {
    /// `None` on Windows, where plugins never run.
    host: Option<PathBuf>,
    hello: Vec<u8>,
    sdk_test: Vec<u8>,
}

fn cargo_in_runtime(args: &[&str]) {
    let status = Command::new(option_env!("CARGO").unwrap_or("cargo"))
        .current_dir(runtime())
        .args(args)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .status()
        .expect("cargo runs");
    assert!(status.success(), "cargo {args:?} in plugins/runtime failed");
}

fn artifacts() -> &'static Artifacts {
    static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let wasm_dir = std::env::var_os("DECKPILOT_E2E_WASM_DIR").map_or_else(
            || {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    "hello-weather",
                    "-p",
                    "deckpilot-sdk-test-guest",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--profile",
                    "guest",
                ]);
                runtime().join("target/wasm32-unknown-unknown/guest")
            },
            PathBuf::from,
        );
        let host = cfg!(unix).then(|| {
            std::env::var_os("DECKPILOT_E2E_PLUGIN_HOST").map_or_else(
                || {
                    cargo_in_runtime(&[
                        "build",
                        "--locked",
                        "-p",
                        "deckpilot-plugin-host",
                        "--bin",
                        "deckpilot-plugin-host",
                    ]);
                    runtime().join(format!(
                        "target/debug/deckpilot-plugin-host{}",
                        std::env::consts::EXE_SUFFIX
                    ))
                },
                PathBuf::from,
            )
        });
        Artifacts {
            host,
            hello: std::fs::read(wasm_dir.join("hello_weather.wasm")).unwrap(),
            sdk_test: std::fs::read(wasm_dir.join("deckpilot_sdk_test_guest.wasm")).unwrap(),
        }
    })
}

// ------------------------------------------------------------- registry

/// A directory: removed when dropped, or (the shared registry, which
/// lives as long as the test binary) left in Cargo's per-target scratch
/// directory.
enum Dir {
    Temp(TempDir),
    Kept(PathBuf),
}

impl Dir {
    fn path(&self) -> &Path {
        match self {
            Self::Temp(dir) => dir.path(),
            Self::Kept(path) => path,
        }
    }

    fn kept(name: &str) -> Self {
        let path =
            Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self::Kept(path)
    }
}

/// A local registry signed with throwaway keys, and the packages in it.
struct Registry {
    /// The working directory: `site/v1/`, `releases/`.
    work: Dir,
    /// Secret keys, plugin sources, packages and the trusted roots.
    keys: Dir,
}

fn publish(dir: &Path, args: &[&str]) -> String {
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
    run(&all, &mut io).unwrap_or_else(|e| panic!("deckpilot-registry {args:?}: {e}"));
    String::from_utf8(out).unwrap()
}

fn sample_manifest() -> Value {
    serde_json::from_slice(
        &std::fs::read(repo().join("plugins/examples/hello-weather/manifest.json")).unwrap(),
    )
    .unwrap()
}

/// The sample's manifest as `id` `version`.
fn hello_manifest(id: &str, version: &str) -> Value {
    let mut manifest = sample_manifest();
    manifest["id"] = json!(id);
    manifest["version"] = json!(version);
    manifest
}

/// 1.1.0: everything of 1.0.0 plus notifications (an escalation).
fn hello_v1_1() -> Value {
    let mut manifest = hello_manifest(HELLO, "1.1.0");
    manifest["permissions"].as_array_mut().unwrap().push(json!({
        "kind": "notifications",
        "justification": "Tells you when the temperature changes a lot."
    }));
    manifest
}

/// The SDK test guest (it can panic on request) as a plugin.
fn crashy_manifest() -> Value {
    let mut manifest = hello_manifest(CRASHY, "1.0.0");
    manifest["name"] = json!("SDK test");
    let actions: Vec<Value> = [
        "scale",
        "inits",
        "arm-shutdown-panic",
        "authed",
        "many-calls",
        "storage",
        "bad-code",
        "panic",
        "spin",
        "slow",
    ]
    .iter()
    .map(|id| {
        json!({
            "id": id, "name": id, "description": "A test action.",
            "category": "Test", "parameters": [],
            "node": {"version": 1, "outputs": [{"key": "inits", "label": "Inits", "kind": "text"}]}
        })
    })
    .collect();
    manifest["contributes"]["actions"] = json!(actions);
    manifest
}

fn write_plugin_dir(dir: &Path, manifest: &Value) {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(manifest).unwrap(),
    )
    .unwrap();
    std::fs::copy(
        repo().join("plugins/examples/hello-weather/LICENSE"),
        dir.join("LICENSE"),
    )
    .unwrap();
    std::fs::copy(
        repo().join("plugins/examples/hello-weather/assets/icon-128.png"),
        dir.join("assets/icon-128.png"),
    )
    .unwrap();
}

impl Registry {
    fn key(&self, name: &str) -> String {
        self.keys.path().join(name).display().to_string()
    }

    fn run(&self, args: &[&str]) -> String {
        publish(self.work.path(), args)
    }

    /// Packages `manifest` with `module` (the `package` command).
    fn package(&self, manifest: &Value, module: &[u8]) -> PathBuf {
        let name = format!(
            "{}-{}",
            manifest["id"].as_str().unwrap(),
            manifest["version"].as_str().unwrap()
        );
        let dir = self.keys.path().join("src").join(&name);
        write_plugin_dir(&dir, manifest);
        let wasm = dir.join("module.wasm");
        std::fs::write(&wasm, module).unwrap();
        let out = self.keys.path().join(format!("{name}.dpplugin"));
        let report = publish(
            self.work.path(),
            &[
                "package",
                "--plugin-dir",
                &dir.display().to_string(),
                "--module",
                &wasm.display().to_string(),
                "--out",
                &out.display().to_string(),
            ],
        );
        assert!(report.contains("packaged"), "{report}");
        out
    }

    fn add(&self, manifest: &Value, module: &[u8]) -> PathBuf {
        let package = self.package(manifest, module);
        let id = manifest["id"].as_str().unwrap();
        let version = manifest["version"].as_str().unwrap();
        let detail = self.keys.path().join(format!("{id}.plugin.json"));
        std::fs::write(
            &detail,
            serde_json::to_vec(&json!({
                "id": id,
                "name": manifest["name"],
                "summary": "A plugin of the end-to-end test",
                "description": manifest["description"],
                "license": "Apache-2.0",
                "homepage": "https://acme.example/plugins",
                "source_repository": "acme/plugins",
                "categories": ["other"]
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
        package
    }

    /// The root keys the agent trusts, kept apart from the registry.
    fn roots(&self) -> PathBuf {
        self.keys.path().join("trusted-roots.json")
    }

    fn create() -> Self {
        let artifacts = artifacts();
        let registry = Self {
            work: Dir::kept("plugins-e2e-registry"),
            keys: Dir::kept("plugins-e2e-registry-keys"),
        };
        for (role, name) in [
            ("root", "root.key"),
            ("index", "index.key"),
            ("timestamp", "timestamp.key"),
        ] {
            registry.run(&[
                "keys",
                "generate",
                "--role",
                role,
                "--out",
                &registry.key(name),
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
        registry.add(&hello_manifest(HELLO, "1.0.0"), &artifacts.hello);
        registry.add(&hello_v1_1(), &artifacts.hello);
        registry.add(&crashy_manifest(), &artifacts.sdk_test);
        let mut future = hello_manifest(FUTURE, "1.0.0");
        future["engines"]["deckpilot"] = json!(">=9.0.0 <10.0.0");
        registry.add(&future, &artifacts.hello);
        registry.add(&hello_manifest(REVOKED, "1.0.0"), &artifacts.hello);
        registry.run(&[
            "revoke",
            "--id",
            REVOKED,
            "--versions",
            "1.0.0",
            "--action",
            "disable",
            "--reason",
            "exfiltrates data",
        ]);
        registry.run(&["build-index", "--source-commit", &"a".repeat(40)]);
        registry.run(&["sign-index", "--key", &registry.key("index.key")]);
        registry.run(&["timestamp", "--key", &registry.key("timestamp.key")]);
        let verified = registry.run(&["verify-site", "--require-packages"]);
        assert!(verified.contains("verified"), "{verified}");
        std::fs::copy(registry.work.path().join("roots.json"), registry.roots()).unwrap();
        registry
    }

    /// The release asset of `id` `version`, as the registry pins it.
    fn release(&self, id: &str, version: &str) -> PathBuf {
        self.work
            .path()
            .join(format!("releases/{id}--v{version}/{id}-{version}.dpplugin"))
    }
}

// ---------------------------------------------------------------- agent

/// Canned HTTP answers for the sample (the only network it uses).
fn canned_http(dir: &Path) -> PathBuf {
    let path = dir.join("http.json");
    let forecast = json!({
        "current": {"temperature_2m": 21.5, "time": "2026-11-14T12:00"},
        "current_units": {"temperature_2m": "°C"}
    });
    std::fs::write(
        &path,
        serde_json::to_vec(&json!([
            {
                "url": "https://api.open-meteo.com/v1/forecast?latitude=52.52",
                "status": 200,
                "headers": [["content-type", "application/json"], ["set-cookie", "secret=1"]],
                "body": forecast.to_string()
            },
            // London redirects to an origin the plugin was never granted.
            {
                "url": "https://api.open-meteo.com/v1/forecast?latitude=51.51",
                "status": 302,
                "headers": [["location", "https://collector.example.net/steal"]]
            },
            {"url": "https://collector.example.net/", "status": 200, "body": "{}"}
        ]))
        .unwrap(),
    )
    .unwrap();
    path
}

fn fetched(http: &Path) -> Vec<String> {
    let mut log = http.as_os_str().to_owned();
    log.push(".log");
    std::fs::read_to_string(PathBuf::from(log))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// One test's data directory with an encrypted configuration database.
struct Env {
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
    http: PathBuf,
}

impl Env {
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        // A row that exists before plugins do.
        let store = support::open_store(&paths, &key);
        let profile = deckpilot_core::Profile::new("Before plugins");
        deckpilot_storage::ConfigStore::save_profile(&store, &profile).unwrap();
        drop(store);
        let http = canned_http(dir.path());
        Self {
            dir,
            paths,
            key,
            http,
        }
    }

    fn plugins_dir(&self) -> PathBuf {
        self.paths.data_dir.join("plugins")
    }

    fn start(&self, registry: &Registry) -> Agent {
        self.start_with(registry, &[], true)
    }

    fn start_with(&self, registry: &Registry, env: &[(&str, &str)], plugins: bool) -> Agent {
        let log = std::fs::File::create(self.dir.path().join(format!(
            "agent-{}.log",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )))
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"));
        command
            .args(["--headless", "--no-usb", "--log-level", "debug"])
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_PLUGIN_HTTP", &self.http)
            .env_remove("DECKPILOT_ENABLE_PLUGINS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        if plugins {
            command
                .arg("--enable-plugins")
                .arg("--plugin-registry")
                .arg(registry.work.path())
                .arg("--plugin-registry-roots")
                .arg(registry.roots());
            if let Some(host) = &artifacts().host {
                command.arg("--plugin-host").arg(host);
            }
        }
        for (name, value) in env {
            command.env(name, value);
        }
        Agent {
            child: command.spawn().expect("the agent starts"),
            paths: self.paths.clone(),
            key: self.key.clone(),
        }
    }

    /// The configuration database, row by row (decrypted), except the
    /// time module schemas were last applied.
    fn config_rows(&self) -> BTreeMap<String, Vec<String>> {
        let connection = rusqlite::Connection::open(&self.paths.database).unwrap();
        connection
            .pragma_update(None, "key", format!("x'{}'", self.key.to_hex().as_str()))
            .unwrap();
        let tables: Vec<String> = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        tables
            .into_iter()
            .map(|table| {
                let mut statement = connection
                    .prepare(&format!("SELECT * FROM \"{table}\""))
                    .unwrap();
                let columns = statement.column_count();
                let mut rows: Vec<String> = statement
                    .query_map([], |row| {
                        Ok((0..columns)
                            .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                            .collect::<Vec<_>>()
                            .join("|"))
                    })
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                if table == "module_schemas" {
                    // `applied_at` is rewritten by every unlock (the agent
                    // applies module schemas each time), plugins or not.
                    for row in &mut rows {
                        if let Some(at) = row.rfind('|') {
                            row.truncate(at);
                        }
                    }
                }
                rows.sort();
                (table, rows)
            })
            .collect()
    }

    /// The plugin store, opened from outside with the derived key (the
    /// agent may be running: the store allows readers).
    fn plugin_store(&self) -> deckpilot_plugin_store::PluginStore {
        deckpilot_plugin_store::PluginStore::open(
            &self.plugins_dir().join("plugins.db"),
            &deckpilot_agent::plugins::key::plugin_store_key(&self.key),
            Duration::from_secs(5),
        )
        .unwrap()
    }
}

struct Agent {
    child: Child,
    paths: AgentPaths,
    key: DatabaseKey,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Agent {
    async fn client(&self) -> AgentClient {
        let deadline = Instant::now() + STARTUP;
        loop {
            match AgentClient::connect(&self.paths, ClientKind::Other).await {
                Ok(client) => return client,
                Err(e) => {
                    assert!(Instant::now() < deadline, "the agent did not start: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    async fn request(&self, request: Request) -> Result<Response, ClientError> {
        let what = request.name();
        tokio::time::timeout(Duration::from_secs(120), async {
            self.client().await.request(request).await
        })
        .await
        .unwrap_or_else(|_| panic!("{what} took too long"))
    }

    async fn unlock(&self) {
        self.request(Request::Unlock {
            key: KeyMaterial::new(self.key.to_hex()),
        })
        .await
        .unwrap();
    }

    async fn started(env: &Env, registry: &Registry) -> Self {
        let agent = env.start(registry);
        agent.unlock().await;
        agent
    }

    /// Asks the agent to stop and waits for its exit.
    async fn stop(mut self) {
        let _ = self.request(Request::Shutdown).await;
        let deadline = Instant::now() + STARTUP;
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the agent did not stop");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn details(&self, plugin: &str) -> PluginDetails {
        match self
            .request(Request::GetPlugin {
                plugin: plugin.into(),
            })
            .await
            .unwrap()
        {
            Response::Plugin { plugin } => *plugin,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn install(&self, plugin: &str, version: Option<&str>) -> Result<Response, ClientError> {
        self.request(Request::InstallPlugin {
            source: PluginInstallSource::Registry {
                plugin: plugin.into(),
                version: version.map(str::to_owned),
            },
            consent: vec![],
        })
        .await
    }

    async fn install_file(&self, path: &Path) -> Result<Response, ClientError> {
        self.request(Request::InstallPlugin {
            source: PluginInstallSource::File {
                path: path.display().to_string(),
            },
            consent: vec![],
        })
        .await
    }

    async fn enable(&self, plugin: &str, consent: &[&str]) -> Result<Response, ClientError> {
        self.request(Request::EnablePlugin {
            plugin: plugin.into(),
            consent: consent.iter().map(|s| (*s).to_owned()).collect(),
        })
        .await
    }

    async fn invoke(
        &self,
        plugin: &str,
        action: &str,
        params: Value,
    ) -> Result<PluginActionOutcome, ClientError> {
        let params = serde_json::from_value(params).unwrap();
        match self
            .request(Request::InvokePluginAction {
                plugin: plugin.into(),
                action: action.into(),
                params,
            })
            .await?
        {
            Response::PluginActionResult { outcome } => Ok(outcome),
            other => panic!("unexpected {other:?}"),
        }
    }
}

/// The plugin events that arrived so far (waits briefly for stragglers).
async fn drain(client: &mut AgentClient) -> Vec<PluginEvent> {
    let mut seen = Vec::new();
    while let Ok(Ok(event)) =
        tokio::time::timeout(Duration::from_millis(300), client.next_plugin_event()).await
    {
        seen.push(event);
    }
    seen
}

fn reason_of(result: &Result<impl std::fmt::Debug, ClientError>) -> &str {
    match result {
        Err(ClientError::Agent(error)) => error.reason.as_deref().unwrap_or("(none)"),
        other => panic!("expected an agent error, got {other:?}"),
    }
}

fn message_of(result: &Result<impl std::fmt::Debug, ClientError>) -> String {
    match result {
        Err(ClientError::Agent(error)) => error.message.clone(),
        other => panic!("expected an agent error, got {other:?}"),
    }
}

fn permission_status(details: &PluginDetails, key: &str) -> Option<PermissionStatus> {
    details
        .permissions
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.status)
}

fn shared_registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::create)
}

// ---------------------------------------------------------------- tests

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one lifecycle, step by step
async fn a_plugin_goes_through_its_whole_lifecycle() {
    let registry = shared_registry();
    let env = Env::new("plugins-e2e-lifecycle");
    let agent = Agent::started(&env, registry).await;
    let mut events = agent.client().await;
    events.subscribe_plugin_events().await.unwrap();
    let config_before = env.config_rows();

    // The runtime is available and the registry verified.
    let Response::PluginRuntime { status } =
        agent.request(Request::GetPluginRuntime).await.unwrap()
    else {
        panic!("runtime status")
    };
    assert!(status.available && status.built && status.enabled && status.execution_supported);

    // Inspect metadata and permissions before installing.
    let Response::AvailablePlugins {
        registry: state,
        plugins,
    } = agent
        .request(Request::ListAvailablePlugins { refresh: true })
        .await
        .unwrap()
    else {
        panic!("available plugins")
    };
    assert_eq!(state.freshness, RegistryFreshness::Fresh);
    let hello = plugins.iter().find(|p| p.id == HELLO).unwrap();
    assert_eq!(hello.name, "Hello Weather");
    assert_eq!(hello.latest.as_deref(), Some("1.1.0"));
    assert_eq!(
        hello
            .versions
            .iter()
            .map(|v| v.version.as_str())
            .collect::<Vec<_>>(),
        ["1.1.0", "1.0.0"]
    );
    let v1 = &hello.versions[1];
    assert_eq!(v1.permissions[0]["hosts"], json!(["api.open-meteo.com"]));
    assert_eq!(hello.versions[0].permissions.len(), 2);
    let future = plugins.iter().find(|p| p.id == FUTURE).unwrap();
    assert_ne!(future.versions[0].incompatible, Vec::<String>::new());
    // A revoked plugin is not offered (or only as withdrawn).
    assert!(
        plugins
            .iter()
            .filter(|p| p.id == REVOKED)
            .all(|p| p.versions.iter().all(|v| v.withdrawn.is_some())),
        "{plugins:?}"
    );

    // Install 1.0.0 (signatures, pins and module verified), without
    // consent: installed, disabled, nothing granted.
    let Response::Plugin { plugin } = agent.install(HELLO, Some("1.0.0")).await.unwrap() else {
        panic!("installed")
    };
    assert_eq!(plugin.summary.state, PluginState::Disabled);
    assert_eq!(plugin.summary.version, "1.0.0");
    assert_eq!(
        permission_status(&plugin, NETWORK),
        Some(PermissionStatus::NotGranted)
    );
    assert_eq!(
        plugin
            .summary
            .actions
            .iter()
            .map(|a| a.action_type.as_str())
            .collect::<Vec<_>>(),
        [
            "acme.hello-weather.greet",
            "acme.hello-weather.current-temperature"
        ]
    );
    assert!(plugin.summary.actions.iter().all(|a| !a.available));
    // Disabled plugins do not run.
    let refused = agent.invoke(HELLO, "greet", json!({"name": "Ada"})).await;
    assert_eq!(reason_of(&refused), reason::NOT_ENABLED);

    // The state is in plugins.db, encrypted under the derived key.
    let database = std::fs::read(env.plugins_dir().join("plugins.db")).unwrap();
    assert!(
        !database.starts_with(b"SQLite format 3"),
        "plugins.db is not encrypted"
    );
    let id = PluginId::parse(HELLO).unwrap();
    let stored = env.plugin_store().plugin(&id).unwrap().unwrap();
    assert_eq!(stored.state, deckpilot_plugin_store::PluginState::Disabled);
    assert!(
        deckpilot_plugin_store::PluginStore::open(
            &env.plugins_dir().join("plugins.db"),
            &deckpilot_plugin_store::StoreKey::from_bytes(*env.key.as_bytes()),
            Duration::from_secs(1),
        )
        .is_err(),
        "the configuration key itself must not open plugins.db"
    );

    // Enabling needs consent for every declared permission.
    let refused = agent.enable(HELLO, &[]).await;
    assert_eq!(reason_of(&refused), reason::CONSENT_REQUIRED);
    let Response::Plugin { plugin } = agent.enable(HELLO, &[NETWORK]).await.unwrap() else {
        panic!("enabled")
    };
    assert_eq!(plugin.summary.state, PluginState::Enabled);
    assert!(plugin.summary.actions.iter().all(|a| a.available));

    // A pure action, then the brokered HTTPS action: structured results.
    let greeting = agent
        .invoke(HELLO, "greet", json!({"name": "Ada", "style": "excited"}))
        .await
        .unwrap();
    let PluginActionOutcome::Ok { message, data } = greeting else {
        panic!("{greeting:?}")
    };
    assert_eq!(message.as_deref(), Some("Hello, Ada!"));
    assert_eq!(data["greeting"], "Hello, Ada!");
    let weather = agent
        .invoke(HELLO, "current-temperature", json!({"city": "berlin"}))
        .await
        .unwrap();
    let PluginActionOutcome::Ok { data, .. } = &weather else {
        panic!("{weather:?}")
    };
    assert_eq!(data["temperature"], "21.5");
    assert_eq!(data["unit"], "°C");
    assert_eq!(data["city"], "Berlin");
    assert_eq!(data["observedAt"], "2026-11-14T12:00");
    assert_eq!(fetched(&env.http).len(), 1);
    // Invalid parameters are refused by the plugin's SDK, not the agent.
    let invalid = agent
        .invoke(HELLO, "current-temperature", json!({"city": "paris"}))
        .await
        .unwrap();
    assert!(
        matches!(&invalid, PluginActionOutcome::Failed { code, .. } if code == "invalid-parameters"),
        "{invalid:?}"
    );

    // A redirect to an origin that was never granted is re-checked by the
    // broker and refused; nothing reaches that origin.
    let redirected = agent
        .invoke(HELLO, "current-temperature", json!({"city": "london"}))
        .await
        .unwrap();
    assert!(
        matches!(&redirected, PluginActionOutcome::Failed { code, .. } if code == "not-permitted"),
        "{redirected:?}"
    );
    assert!(
        !fetched(&env.http)
            .iter()
            .any(|url| url.contains("collector.example.net")),
        "{:?}",
        fetched(&env.http)
    );

    // Revoke: the same action is denied on its next call, before the
    // network layer; the plugin stays enabled.
    let before = fetched(&env.http).len();
    let Response::Plugin { plugin } = agent
        .request(Request::RevokePluginPermission {
            plugin: HELLO.into(),
            permission: NETWORK.into(),
        })
        .await
        .unwrap()
    else {
        panic!("revoked")
    };
    assert_eq!(
        permission_status(&plugin, NETWORK),
        Some(PermissionStatus::Revoked)
    );
    assert_eq!(plugin.summary.state, PluginState::Enabled);
    let denied = agent
        .invoke(HELLO, "current-temperature", json!({"city": "berlin"}))
        .await
        .unwrap();
    let PluginActionOutcome::Failed { code, message, .. } = &denied else {
        panic!("{denied:?}")
    };
    assert_eq!(code, "not-permitted");
    assert!(
        message
            .as_deref()
            .unwrap_or_default()
            .contains("api.open-meteo.com"),
        "{message:?}"
    );
    assert_eq!(
        fetched(&env.http).len(),
        before,
        "a denied request went out"
    );

    // Grant it again: allowed again.
    agent
        .request(Request::GrantPluginPermissions {
            plugin: HELLO.into(),
            permissions: vec![NETWORK.into()],
        })
        .await
        .unwrap();
    let again = agent
        .invoke(HELLO, "current-temperature", json!({"city": "berlin"}))
        .await
        .unwrap();
    assert!(matches!(again, PluginActionOutcome::Ok { .. }), "{again:?}");

    // Disable, restart the agent: the state and grants persisted.
    let Response::Plugin { plugin } = agent
        .request(Request::DisablePlugin {
            plugin: HELLO.into(),
        })
        .await
        .unwrap()
    else {
        panic!("disabled")
    };
    assert_eq!(plugin.summary.state, PluginState::Disabled);
    // The subscribed client saw the install's progress and every change.
    let seen = drain(&mut events).await;
    for expected in [
        PluginStage::Fetching,
        PluginStage::Verifying,
        PluginStage::Installing,
    ] {
        assert!(
            seen.iter().any(|e| matches!(
                e,
                PluginEvent::Progress { plugin, operation: PluginOperation::Install, stage, .. }
                    if plugin == HELLO && *stage == expected
            )),
            "no install {expected:?} event: {seen:?}"
        );
    }
    for state in [PluginState::Disabled, PluginState::Enabled] {
        assert!(
            seen.iter().any(|e| matches!(
                e,
                PluginEvent::Changed { plugin, state: Some(s) } if plugin == HELLO && *s == state
            )),
            "no change to {state:?}: {seen:?}"
        );
    }
    drop(events);
    agent.stop().await;
    let agent = Agent::started(&env, registry).await;
    let plugin = agent.details(HELLO).await;
    assert_eq!(plugin.summary.state, PluginState::Disabled);
    assert_eq!(plugin.summary.version, "1.0.0");
    assert_eq!(
        permission_status(&plugin, NETWORK),
        Some(PermissionStatus::Granted)
    );
    assert!(plugin.audit.iter().any(|a| a.op == "revoke"));

    // Re-enable without new consent (everything is granted) and run.
    agent.enable(HELLO, &[]).await.unwrap();
    let greeting = agent
        .invoke(HELLO, "greet", json!({"name": "Grace"}))
        .await
        .unwrap();
    assert!(
        matches!(greeting, PluginActionOutcome::Ok { .. }),
        "{greeting:?}"
    );

    // Update to 1.1.0: it asks for notifications, so it is staged until
    // consented; the running version is unchanged meanwhile.
    let Response::PluginUpdate { outcome, .. } = agent
        .request(Request::UpdatePlugin {
            plugin: HELLO.into(),
            version: None,
            consent: vec![],
        })
        .await
        .unwrap()
    else {
        panic!("update")
    };
    let PluginUpdateOutcome::AwaitingConsent { version, changes } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(version, "1.1.0");
    assert!(changes.escalation);
    assert_eq!(
        changes
            .added
            .iter()
            .map(|p| p.key.as_str())
            .collect::<Vec<_>>(),
        ["notifications"]
    );
    let plugin = agent.details(HELLO).await;
    assert_eq!(plugin.summary.version, "1.0.0");
    assert_eq!(plugin.summary.staged_version.as_deref(), Some("1.1.0"));
    assert_eq!(plugin.pending_update.as_ref().unwrap().version, "1.1.0");
    let still = agent
        .invoke(HELLO, "greet", json!({"name": "Grace"}))
        .await
        .unwrap();
    assert!(matches!(still, PluginActionOutcome::Ok { .. }));
    // Approving without consent for the escalation is refused.
    let refused = agent
        .request(Request::ApprovePluginUpdate {
            plugin: HELLO.into(),
            consent: vec![],
        })
        .await;
    assert_eq!(reason_of(&refused), reason::CONSENT_REQUIRED);
    assert_eq!(agent.details(HELLO).await.summary.version, "1.0.0");
    let Response::PluginUpdate { outcome, .. } = agent
        .request(Request::ApprovePluginUpdate {
            plugin: HELLO.into(),
            consent: vec!["notifications".into()],
        })
        .await
        .unwrap()
    else {
        panic!("approved")
    };
    assert!(
        matches!(&outcome, PluginUpdateOutcome::Applied { version, .. } if version == "1.1.0"),
        "{outcome:?}"
    );
    let plugin = agent.details(HELLO).await;
    assert_eq!(plugin.summary.version, "1.1.0");
    assert_eq!(plugin.summary.previous_version.as_deref(), Some("1.0.0"));
    assert_eq!(plugin.summary.state, PluginState::Enabled);
    assert_eq!(
        permission_status(&plugin, "notifications"),
        Some(PermissionStatus::Granted)
    );
    let on_new = agent
        .invoke(HELLO, "current-temperature", json!({"city": "berlin"}))
        .await
        .unwrap();
    assert!(
        matches!(on_new, PluginActionOutcome::Ok { .. }),
        "{on_new:?}"
    );

    // Roll back to 1.0.0: the grant only 1.1.0 declared is gone.
    let Response::Plugin { plugin } = agent
        .request(Request::RollbackPlugin {
            plugin: HELLO.into(),
        })
        .await
        .unwrap()
    else {
        panic!("rolled back")
    };
    assert_eq!(plugin.summary.version, "1.0.0");
    assert_eq!(permission_status(&plugin, "notifications"), None);
    let rolled = agent
        .invoke(HELLO, "greet", json!({"name": "Linus"}))
        .await
        .unwrap();
    assert!(
        matches!(rolled, PluginActionOutcome::Ok { .. }),
        "{rolled:?}"
    );

    // Uninstall: no files, no plugin rows.
    let mut events = agent.client().await;
    events.subscribe_plugin_events().await.unwrap();
    assert_eq!(
        agent
            .request(Request::UninstallPlugin {
                plugin: HELLO.into(),
            })
            .await
            .unwrap(),
        Response::Ok
    );
    let Response::Plugins { plugins } = agent.request(Request::ListPlugins).await.unwrap() else {
        panic!("plugins")
    };
    assert!(plugins.is_empty(), "{plugins:?}");
    assert!(!env.plugins_dir().join(HELLO).exists());
    let leftovers: Vec<_> = std::fs::read_dir(env.plugins_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with("plugins.db") && !name.starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    for hidden in [".staging", ".trash"] {
        let dir = env.plugins_dir().join(hidden);
        assert!(
            !dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none(),
            "{hidden} is not empty"
        );
    }
    let store = env.plugin_store();
    assert!(store.plugin(&id).unwrap().is_none());
    assert_eq!(store.grants(&id).unwrap(), []);
    assert_eq!(store.journal().unwrap(), []);
    // The audit keeps what happened (decisions only, never values).
    assert!(
        store
            .audit(&id)
            .unwrap()
            .iter()
            .any(|a| a.op == "uninstall")
    );

    // Events told the app about the uninstall.
    let seen = drain(&mut events).await;
    assert!(
        seen.iter().any(|e| matches!(
            e,
            PluginEvent::Changed { plugin, state: None } if plugin == HELLO
        )),
        "no change event for the uninstall: {seen:?}"
    );
    drop(events);
    agent.stop().await;

    // The configuration database is exactly as it was.
    assert_eq!(env.config_rows(), config_before);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_packages_and_registries_are_refused() {
    let registry = shared_registry();
    let env = Env::new("plugins-e2e-refusals");
    let agent = Agent::started(&env, registry).await;
    let scratch = env.dir.path();

    // A corrupt package: one byte changed. It no longer matches the pins.
    let mut corrupt = std::fs::read(registry.release(HELLO, "1.0.0")).unwrap();
    let middle = corrupt.len() / 2;
    corrupt[middle] ^= 0x40;
    let path = scratch.join("corrupt.dpplugin");
    std::fs::write(&path, &corrupt).unwrap();
    let refused = agent.install_file(&path).await;
    assert_eq!(reason_of(&refused), reason::INVALID_PACKAGE, "{refused:?}");

    // A valid package that is not the one the registry pinned (another
    // LICENSE): refused by the pins.
    let mut manifest = hello_manifest(HELLO, "1.0.0");
    manifest["description"] = json!("Same id and version, other bytes.");
    let other = registry.package(&manifest, &artifacts().hello);
    let refused = agent.install_file(&other).await;
    assert_eq!(reason_of(&refused), reason::INVALID_PACKAGE, "{refused:?}");
    assert!(message_of(&refused).contains("pinned"), "{refused:?}");

    // An invalid manifest (no license).
    let mut broken = hello_manifest(HELLO, "1.0.0");
    broken.as_object_mut().unwrap().remove("license");
    let path = raw_package(
        scratch,
        "invalid-manifest",
        &broken,
        &[("plugin.wasm", artifacts().hello.clone())],
    );
    let refused = agent.install_file(&path).await;
    assert_eq!(reason_of(&refused), reason::INVALID_PACKAGE, "{refused:?}");
    assert!(message_of(&refused).contains("license"), "{refused:?}");

    // A JavaScript package of the #191 contract: refused, naming the
    // contract.
    let mut js = hello_manifest(HELLO, "1.0.0");
    js.as_object_mut().unwrap().remove("runtime");
    let path = raw_package(
        scratch,
        "javascript",
        &js,
        &[("plugin.js", b"export default {};".to_vec())],
    );
    let refused = agent.install_file(&path).await;
    assert_eq!(
        reason_of(&refused),
        reason::UNSUPPORTED_RUNTIME,
        "{refused:?}"
    );
    assert!(message_of(&refused).contains("plugin.wasm"), "{refused:?}");

    // A revoked version and an incompatible one.
    let refused = agent.install(REVOKED, Some("1.0.0")).await;
    assert_eq!(reason_of(&refused), reason::REVOKED, "{refused:?}");
    let refused = agent.install(FUTURE, Some("1.0.0")).await;
    assert_eq!(reason_of(&refused), reason::INCOMPATIBLE, "{refused:?}");
    assert!(message_of(&refused).contains(">=9.0.0"), "{refused:?}");

    // Nothing was installed by any of these.
    let Response::Plugins { plugins } = agent.request(Request::ListPlugins).await.unwrap() else {
        panic!("plugins")
    };
    assert!(plugins.is_empty(), "{plugins:?}");

    // The pinned package file itself installs (a local file is accepted
    // only as the bytes the registry pins).
    let Response::Plugin { plugin } = agent
        .install_file(&registry.release(HELLO, "1.0.0"))
        .await
        .unwrap()
    else {
        panic!("installed")
    };
    assert_eq!(plugin.summary.version, "1.0.0");
    agent.stop().await;
}

/// A package archive written without the builder (which refuses what the
/// reader refuses): `manifest`, LICENSE, the icon and `extra`.
fn raw_package(dir: &Path, name: &str, manifest: &Value, extra: &[(&str, Vec<u8>)]) -> PathBuf {
    use std::io::Write as _;
    let mut files: Vec<(String, Vec<u8>)> = vec![
        (
            "manifest.json".into(),
            serde_json::to_vec(manifest).unwrap(),
        ),
        (
            "LICENSE".into(),
            std::fs::read(repo().join("plugins/examples/hello-weather/LICENSE")).unwrap(),
        ),
        (
            "assets/icon-128.png".into(),
            std::fs::read(repo().join("plugins/examples/hello-weather/assets/icon-128.png"))
                .unwrap(),
        ),
    ];
    files.extend(extra.iter().map(|(p, b)| ((*p).to_owned(), b.clone())));
    files.sort();
    let mut tar = tar::Builder::new(Vec::new());
    for (path, bytes) in &files {
        let mut header = tar::Header::new_ustar();
        header.set_path(path).unwrap();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        tar.append(&header, bytes.as_slice()).unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    let path = dir.join(format!("{name}.dpplugin"));
    std::fs::write(&path, gzip.finish().unwrap()).unwrap();
    path
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_registry_with_an_invalid_signature_is_refused() {
    let registry = shared_registry();
    // A copy of the registry whose timestamp signature was tampered with.
    let tampered = Registry {
        work: Dir::Temp(TempDir::new("plugin-registry-tampered")),
        keys: Dir::Temp(TempDir::new("plugin-registry-tampered-keys")),
    };
    copy_dir(registry.work.path(), tampered.work.path());
    std::fs::copy(registry.roots(), tampered.roots()).unwrap();
    let signature = tampered.work.path().join("site/v1/timestamp.json.minisig");
    // The file is the minisign signature in base64 (the updater's format).
    // Flip one character of the signature line inside it.
    let base64 = base64::engine::general_purpose::STANDARD;
    let text = std::fs::read_to_string(&signature).unwrap();
    let decoded = String::from_utf8(base64.decode(text.trim()).unwrap()).unwrap();
    let mut lines: Vec<String> = decoded.lines().map(str::to_owned).collect();
    let line = &mut lines[1];
    let at = line.len() / 2;
    let flipped = if line.as_bytes()[at] == b'A' {
        "B"
    } else {
        "A"
    };
    line.replace_range(at..=at, flipped);
    std::fs::write(&signature, base64.encode(lines.join("\n") + "\n")).unwrap();

    let env = Env::new("plugins-e2e-signature");
    let agent = Agent::started(&env, &tampered).await;
    let refused = agent
        .request(Request::ListAvailablePlugins { refresh: true })
        .await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    let refused = agent.install(HELLO, Some("1.0.0")).await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    let Response::PluginRuntime { status } =
        agent.request(Request::GetPluginRuntime).await.unwrap()
    else {
        panic!("status")
    };
    assert_eq!(
        status.registry.unwrap().freshness,
        RegistryFreshness::NeverVerified
    );
    assert!(
        status.reason.unwrap_or_default().contains("refused"),
        "the status names the refused registry"
    );
    agent.stop().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_crash_during_an_invocation_is_contained() {
    let registry = shared_registry();
    let env = Env::new("plugins-e2e-crash");
    let agent = Agent::started(&env, registry).await;
    agent.install(CRASHY, None).await.unwrap();
    agent.enable(CRASHY, &[NETWORK]).await.unwrap();
    let first = agent.invoke(CRASHY, "inits", json!({})).await.unwrap();
    assert!(matches!(first, PluginActionOutcome::Ok { .. }), "{first:?}");

    // The guest panics: the host process traps and exits. The agent
    // reports it and keeps running.
    let crashed = agent.invoke(CRASHY, "panic", json!({})).await;
    assert_eq!(reason_of(&crashed), reason::HOST_FAILED, "{crashed:?}");
    assert!(message_of(&crashed).contains("stopped"), "{crashed:?}");
    let Response::Status { status } = agent.request(Request::GetStatus).await.unwrap() else {
        panic!("status")
    };
    assert!(status.pid > 0);

    // After the restart backoff (1 s) a new host serves the next call.
    tokio::time::sleep(Duration::from_millis(1300)).await;
    let again = agent.invoke(CRASHY, "inits", json!({})).await.unwrap();
    assert!(matches!(again, PluginActionOutcome::Ok { .. }), "{again:?}");
    // The plugin is still enabled (one crash is not a crash loop).
    assert_eq!(
        agent.details(CRASHY).await.summary.state,
        PluginState::Enabled
    );
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_install_interrupted_by_an_agent_crash_is_recovered() {
    let registry = shared_registry();
    let env = Env::new("plugins-e2e-recovery");
    let crash_during_install = |point: &str| {
        // The agent exits (as in a power cut) at `point` of the install.
        let agent = env.start_with(registry, &[("DECKPILOT_TEST_PLUGIN_CRASH", point)], true);
        async move {
            agent.unlock().await;
            let interrupted = agent.install(HELLO, Some("1.0.0")).await;
            assert!(
                matches!(interrupted, Err(ClientError::Closed | ClientError::Io(_))),
                "{interrupted:?}"
            );
            drop(agent);
        }
    };
    let staging_is_empty = || {
        let dir = env.plugins_dir().join(".staging");
        !dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none()
    };

    // Crashed after the journal entry, before any file: undone on restart.
    crash_during_install("install:journal_written").await;
    let agent = Agent::started(&env, registry).await;
    let Response::Plugins { plugins } = agent.request(Request::ListPlugins).await.unwrap() else {
        panic!("plugins")
    };
    assert!(plugins.is_empty(), "{plugins:?}");
    assert!(!env.plugins_dir().join(HELLO).exists());
    assert_eq!(env.plugin_store().journal().unwrap(), []);
    assert!(staging_is_empty());
    agent.stop().await;

    // Crashed with the files unpacked but nothing committed: the files
    // match the journal's integrity record, so recovery completes it.
    crash_during_install("install:files_unpacked").await;
    let agent = Agent::started(&env, registry).await;
    let plugin = agent.details(HELLO).await;
    assert_eq!(plugin.summary.state, PluginState::Disabled);
    assert_eq!(plugin.summary.version, "1.0.0");
    assert_eq!(env.plugin_store().journal().unwrap(), []);
    assert!(staging_is_empty());
    // The recovered install is a normal one: it can be removed and
    // installed again (from the pinned package file).
    agent
        .request(Request::UninstallPlugin {
            plugin: HELLO.into(),
        })
        .await
        .unwrap();
    let Response::Plugin { plugin } = agent
        .install_file(&registry.release(HELLO, "1.0.0"))
        .await
        .unwrap()
    else {
        panic!("installed")
    };
    assert_eq!(plugin.summary.state, PluginState::Disabled);
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugins_db_needs_the_key_derived_from_the_configuration_key() {
    let registry = shared_registry();
    let env = Env::new("plugins-e2e-key");
    // Locked (no key yet): plugin requests wait for the unlock.
    let agent = env.start(registry);
    let locked = agent.request(Request::ListPlugins).await;
    assert_eq!(reason_of(&locked), reason::LOCKED, "{locked:?}");
    agent.unlock().await;
    agent.install(HELLO, Some("1.0.0")).await.unwrap();
    agent.stop().await;
    let database = env.plugins_dir().join("plugins.db");
    let before = std::fs::read(&database).unwrap();

    // The configuration app changes the password while the agent is not
    // running: the configuration key changes, plugins.db's does not.
    let vault = deckpilot_storage::vault::Vault::new(&env.paths.database).with_test_cost();
    let new_key = vault
        .use_password(
            &env.key,
            &deckpilot_storage::crypto::Password::new("another password"),
        )
        .unwrap();
    let agent = env.start(registry);
    agent
        .request(Request::Unlock {
            key: KeyMaterial::new(new_key.to_hex()),
        })
        .await
        .unwrap();
    let Response::PluginRuntime { status } =
        agent.request(Request::GetPluginRuntime).await.unwrap()
    else {
        panic!("status")
    };
    assert!(!status.available);
    assert!(
        status
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("plugins.db"),
        "{status:?}"
    );
    let refused = agent.request(Request::ListPlugins).await;
    assert_eq!(
        reason_of(&refused),
        reason::STORE_UNAVAILABLE,
        "{refused:?}"
    );
    agent.stop().await;
    // Nothing was overwritten or reset.
    assert_eq!(std::fs::read(&database).unwrap(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_enable_plugins_nothing_is_available() {
    let registry = shared_registry();
    let env = Env::new("plugins-e2e-off");
    let agent = env.start_with(registry, &[], false);
    agent.unlock().await;
    let Response::PluginRuntime { status } =
        agent.request(Request::GetPluginRuntime).await.unwrap()
    else {
        panic!("status")
    };
    assert!(status.built && !status.enabled && !status.available);
    let refused = agent.install(HELLO, None).await;
    assert_eq!(
        reason_of(&refused),
        reason::PLUGINS_UNAVAILABLE,
        "{refused:?}"
    );
    agent.stop().await;
    assert!(
        !env.plugins_dir().exists(),
        "nothing is created without plugins"
    );
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_refuses_to_run_plugins() {
    let registry = shared_registry();
    let env = Env::new("plugins-e2e-windows");
    let agent = Agent::started(&env, registry).await;
    let Response::PluginRuntime { status } =
        agent.request(Request::GetPluginRuntime).await.unwrap()
    else {
        panic!("status")
    };
    assert!(status.available && !status.execution_supported);
    assert!(status.reason.unwrap_or_default().contains("AppContainer"));
    // Installing and inspecting work; enabling and running do not.
    let Response::Plugin { plugin } = agent.install(HELLO, Some("1.0.0")).await.unwrap() else {
        panic!("installed")
    };
    assert!(plugin.summary.actions.iter().all(|a| !a.available));
    let refused = agent.enable(HELLO, &[NETWORK]).await;
    assert_eq!(
        reason_of(&refused),
        reason::PLATFORM_UNSUPPORTED,
        "{refused:?}"
    );
    let refused = agent.invoke(HELLO, "greet", json!({"name": "Ada"})).await;
    assert_eq!(
        reason_of(&refused),
        reason::PLATFORM_UNSUPPORTED,
        "{refused:?}"
    );
    assert_eq!(
        agent.details(HELLO).await.summary.state,
        PluginState::Disabled
    );
    agent
        .request(Request::UninstallPlugin {
            plugin: HELLO.into(),
        })
        .await
        .unwrap();
    agent.stop().await;
}
