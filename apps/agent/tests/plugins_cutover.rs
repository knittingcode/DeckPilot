//! The GitHub cut-over (`docs/marketplace/module-migration.md` §9, P9) end
//! to end, through the real agent, the real sandboxed plugin host, real
//! storage and IPC, each test in its own data directory (feature
//! `plugins`):
//!
//! * an alpha.3-style `deckpilot.db`: the built-in `github` module saved
//!   and enabled with GitHub's API URL, its token stored as the credential
//!   `github.token`, a profile with an MK.2 key bound to
//!   `github.pull-requests` and a workflow node running `github.checks`;
//! * the shipped alias block (`ACTION_ALIASES`) and migration plan, and
//!   `plugins/first-party/github` built to wasm and bundled as the
//!   release bundles it (class `migration`, pinned with
//!   `DECKPILOT_TEST_BUNDLED_PINS`);
//! * GitHub never answers: the plugin's requests get canned answers
//!   (`DECKPILOT_TEST_PLUGIN_HTTP`, which logs the credential header the
//!   agent injected), and the built-in's go to a stand-in GitHub on
//!   127.0.0.1 that records them (`DECKPILOT_TEST_BUILTIN_GITHUB_API`,
//!   feature `test-hooks`: the module's stored settings, and so the
//!   migration's eligibility, are untouched). Which of the two got a
//!   request says which implementation ran;
//! * the consent, Retry, disable and uninstall come through the app's
//!   requests.
#![cfg(feature = "plugins")]

mod support;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionStatus, ActionTypeId, BindingTarget,
    ControlBinding, CredentialId, DeviceLayout, ExecutionState, ModuleId, ModuleSettings,
    ParameterValue, Profile, Secret, Trigger, WorkflowId,
};
use deckpilot_device::{ControlId, ControlInput, DeviceId};
use deckpilot_ipc::plugins::{
    self as ipc, ConsentDecision, MigrationRoute, ModuleMigrationInfo, PluginOrigin, PluginState,
};
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_store::{MigrationPhase, ModuleMigration, PluginStore, TombstoneDecision};
use deckpilot_storage::{ConfigStore, DatabaseKey};
use serde_json::{Value, json};
use support::TempDir;

const MODULE: &str = "github";
const PLUGIN: &str = "deckpilot.github";
const CREDENTIAL: &str = "github.token";
/// The token the user stored with an alpha.3 app.
const TOKEN: &str = "ghp_cutover-7e3c91-not-a-real-token";
const API: &str = "https://api.github.com";
const DATE: &str = "Mon, 16 Nov 2026 10:00:00 GMT";
const STARTUP: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(60);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime() -> PathBuf {
    repo().join("plugins/runtime")
}

fn source() -> PathBuf {
    repo().join("plugins/first-party/github")
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

struct Artifacts {
    host: PathBuf,
    module: PathBuf,
}

/// The host and the plugin's module, built from `plugins/runtime` (or
/// taken from `DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR`).
fn artifacts() -> &'static Artifacts {
    static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let wasm_dir = std::env::var_os("DECKPILOT_E2E_WASM_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.join("deckpilot_github.wasm").is_file())
            .unwrap_or_else(|| {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    "deckpilot-github",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--profile",
                    "guest",
                ]);
                runtime().join("target/wasm32-unknown-unknown/guest")
            });
        let host = std::env::var_os("DECKPILOT_E2E_PLUGIN_HOST").map_or_else(
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
        );
        Artifacts {
            host,
            module: wasm_dir.join("deckpilot_github.wasm"),
        }
    })
}

// ------------------------------------------------------- GitHub's answers

/// GitHub's answer to `GET <path>`, for both implementations: open pull
/// requests of `octo/site` and the checks of its `main`.
fn answer(path: &str) -> (u16, Value) {
    match path {
        "/search/issues" => (
            200,
            json!({
                "total_count": 2,
                "incomplete_results": false,
                "items": [
                    {
                        "number": 12, "title": "Fix the build", "draft": false,
                        "repository_url": format!("{API}/repos/octo/site"),
                        "user": {"login": "mona"},
                        "html_url": "https://github.com/octo/site/pull/12"
                    },
                    {
                        "number": 11, "title": "Docs", "draft": true,
                        "repository_url": format!("{API}/repos/octo/site"),
                        "user": {"login": "hubot"},
                        "html_url": "https://github.com/octo/site/pull/11"
                    }
                ]
            }),
        ),
        "/repos/octo/site/commits/main/check-runs" => (
            200,
            json!({
                "total_count": 2,
                "check_runs": [
                    {"name": "build", "status": "completed", "conclusion": "success"},
                    {"name": "lint", "status": "completed", "conclusion": "failure"}
                ]
            }),
        ),
        "/repos/octo/site/commits/main/status" => (
            200,
            json!({"sha": "0123abc", "state": "success", "statuses": [
                {"context": "ci/legacy", "state": "success"}
            ]}),
        ),
        _ => (404, json!({"message": "Not Found"})),
    }
}

/// The plugin's canned answers (the longest matching prefix first).
fn plugin_routes() -> Value {
    let route = |prefix: &str, path: &str| {
        let (status, body) = answer(path);
        json!({
            "url": format!("{API}{prefix}"),
            "status": status,
            "headers": [["content-type", "application/json"], ["date", DATE]],
            "body": body.to_string(),
        })
    };
    json!([
        route(
            "/repos/octo/site/commits/main/check-runs",
            "/repos/octo/site/commits/main/check-runs"
        ),
        route(
            "/repos/octo/site/commits/main/status",
            "/repos/octo/site/commits/main/status"
        ),
        route("/search/issues?", "/search/issues"),
    ])
}

/// The stand-in GitHub for the built-in, on 127.0.0.1: [`answer`], and
/// every request it got with its `authorization` header.
struct StandIn {
    port: u16,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StandIn {
    fn start() -> Self {
        let listener = support::http::bind_local();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let (stop, seen) = (stop.clone(), seen.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            serve(stream, &seen);
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(2)),
                    }
                }
            })
        };
        Self {
            port,
            stop,
            seen,
            thread: Some(thread),
        }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(stream: std::net::TcpStream, seen: &Mutex<Vec<String>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let target = line.split(' ').nth(1).unwrap_or("/").to_owned();
    let mut authorization = String::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("authorization")
        {
            value.trim().clone_into(&mut authorization);
        }
    }
    let path = target.split('?').next().unwrap_or("/").to_owned();
    seen.lock()
        .unwrap()
        .push(format!("{path}\t{authorization}"));
    let (status, body) = answer(&path);
    let body = body.to_string();
    let mut stream = reader.into_inner();
    let _ = write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nDate: {DATE}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

// ---------------------------------------------------------- environment

/// How one agent start is set up.
#[derive(Default, Clone, Copy)]
struct Start {
    /// Plugins off (`--disable-plugins`): as an older build, or a build
    /// without the `plugins` feature.
    plugins_off: bool,
}

/// What an alpha.3 app saved, read raw from `deckpilot.db`: the
/// `modules` row, the `credentials` rows and the profile documents.
#[derive(Debug, PartialEq, Eq)]
struct Legacy {
    module: (i64, String),
    credentials: Vec<(String, String, String, i64)>,
    profiles: Vec<(String, String, i64, String, i64)>,
}

struct Env {
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
    standin: StandIn,
    pull_requests: ActionInstanceId,
    checks_workflow: WorkflowId,
}

impl Env {
    /// An alpha.3 configuration with `api_url` as the module's API URL.
    fn new(name: &str, api_url: &str) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let mut env = Self {
            dir,
            paths,
            key,
            standin: StandIn::start(),
            pull_requests: ActionInstanceId::new(),
            checks_workflow: WorkflowId::new(),
        };
        env.write_alpha3(api_url);
        env.stage();
        std::fs::write(
            env.http_file(),
            serde_json::to_vec_pretty(&plugin_routes()).unwrap(),
        )
        .unwrap();
        env
    }

    /// The built-in module saved and enabled, its token stored, and a
    /// profile with a key bound to `github.pull-requests` and a workflow
    /// node running `github.checks`.
    fn write_alpha3(&mut self, api_url: &str) {
        let store = support::open_store(&self.paths, &self.key);
        store
            .set_credential(
                &CredentialId::new(CREDENTIAL).unwrap(),
                "GitHub token",
                Some(&Secret::new(TOKEN)),
            )
            .unwrap();
        store
            .set_module_state(
                &ModuleId::new(MODULE).unwrap(),
                &ModuleSettings {
                    enabled: true,
                    config: ActionParameters::new()
                        .with("apiUrl", ParameterValue::Text(api_url.into()))
                        .with("credential", ParameterValue::Text(CREDENTIAL.into()))
                        .with("timeoutSeconds", ParameterValue::Integer(20)),
                },
            )
            .unwrap();
        let mut profile = Profile::new("Work");
        let instance = |id, name: &str, action: &str, params: ActionParameters| ActionInstance {
            id,
            name: name.into(),
            action_type: ActionTypeId::new(action).unwrap(),
            parameters: params,
            description: String::new(),
            icon: None,
            label: None,
        };
        profile.actions.push(instance(
            self.pull_requests,
            "Open pull requests",
            "github.pull-requests",
            ActionParameters::new()
                .with("repository", ParameterValue::Text("octo/site".into()))
                .with("credential", ParameterValue::Text(String::new())),
        ));
        let checks = ActionInstanceId::new();
        profile.actions.push(instance(
            checks,
            "Checks of main",
            "github.checks",
            ActionParameters::new()
                .with("repository", ParameterValue::Text("octo/site".into()))
                .with("ref", ParameterValue::Text("main".into()))
                .with("credential", ParameterValue::Text(CREDENTIAL.into())),
        ));
        let mut layout = DeviceLayout::new("Main", deckpilot_device::models::stream_deck_mk2().id);
        layout.pages[0].bindings = vec![ControlBinding::new(
            ControlId::Key { index: 0 },
            Trigger::Press,
            BindingTarget::action(self.pull_requests),
        )];
        profile.layouts.push(layout);
        let workflow = support::graph_workflow(
            "Checks",
            support::graph(
                &json!([
                    {"id": "start", "type": "trigger"},
                    {"id": "checks", "type": "action", "action": checks},
                ]),
                &json!([{"from": "start", "to": "checks"}]),
            ),
        );
        self.checks_workflow = workflow.id;
        profile.workflows.push(workflow);
        store.save_profile(&profile).unwrap();
        store.set_active_profile(Some(profile.id)).unwrap();
    }

    fn bundled(&self) -> PathBuf {
        self.dir.path().join("resources/bundled")
    }

    fn pins_file(&self) -> PathBuf {
        self.dir.path().join("compiled-pins.json")
    }

    fn http_file(&self) -> PathBuf {
        self.dir.path().join("github-api.json")
    }

    /// What the plugin sent: one line per request, with the credential
    /// header the agent injected.
    fn plugin_log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("github-api.json.log")).unwrap_or_default()
    }

    /// `bundled/deckpilot.github/` as `plugins/first-party/build.sh` stages
    /// it for a release (class `migration`, `bundled.json`), and the pins
    /// the agent "was compiled with".
    fn stage(&self) {
        let dir = self.bundled().join(PLUGIN);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for file in ["manifest.json", "LICENSE", "assets/icon-128.png"] {
            std::fs::copy(source().join(file), dir.join(file)).unwrap();
        }
        std::fs::copy(&artifacts().module, dir.join("plugin.wasm")).unwrap();
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let version = semver::Version::parse(manifest["version"].as_str().unwrap()).unwrap();
        let released: Value = serde_json::from_slice(
            &std::fs::read(repo().join("plugins/first-party/bundled.json")).unwrap(),
        )
        .unwrap();
        assert!(
            released["plugins"]
                .as_array()
                .unwrap()
                .contains(&json!({"path": "github", "class": "migration"})),
            "the release bundles deckpilot.github as a migration-class plugin: {released}"
        );
        let pins = Pins::compute(
            &self.bundled(),
            &[(
                PluginId::parse(PLUGIN).unwrap(),
                version,
                BundleClass::Migration,
            )],
        )
        .unwrap();
        std::fs::write(self.bundled().join("pins.json"), pins.to_json_pretty()).unwrap();
        std::fs::write(self.pins_file(), pins.to_json_pretty()).unwrap();
    }

    async fn start(&self, start: Start) -> Agent {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.path().join("agent.log"))
            .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"));
        command
            .args(["--headless", "--no-usb", "--log-level", "debug"])
            .args(["--mock-devices", "mk2"])
            .arg("--plugin-host")
            .arg(&artifacts().host)
            .arg("--bundled-plugins")
            .arg(self.bundled())
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_BUNDLED_PINS", self.pins_file())
            .env("DECKPILOT_TEST_PLUGIN_HTTP", self.http_file())
            .env(
                "DECKPILOT_TEST_PLUGIN_OPEN_URL",
                self.dir.path().join("opened.txt"),
            )
            .env("DECKPILOT_TEST_BUILTIN_GITHUB_API", self.standin.base())
            .env_remove("DECKPILOT_TEST_LEGACY_FIXTURE")
            .env_remove("DECKPILOT_TEST_MIGRATION_DECISIONS")
            .env_remove("DECKPILOT_TEST_MIGRATION_EXIT")
            .env_remove("DECKPILOT_TEST_PLUGIN_CRASH")
            .env_remove("DECKPILOT_ENABLE_PLUGINS")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_URL")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_ROOTS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        // Plugins are on without a switch in a build with the feature.
        if start.plugins_off {
            command.arg("--disable-plugins");
        }
        let agent = Agent {
            child: command.spawn().expect("the agent starts"),
            paths: self.paths.clone(),
            log: self.dir.path().join("agent.log"),
            answers: Mutex::new(Vec::new()),
        };
        agent
            .request(Request::Unlock {
                key: KeyMaterial::new(self.key.to_hex()),
            })
            .await
            .unwrap();
        agent
    }

    fn plugin_store(&self) -> PluginStore {
        PluginStore::open(
            &deckpilot_plugin_store::Layout::new(&self.paths.data_dir).database(),
            &deckpilot_agent::plugins::key::plugin_store_key(&self.key),
            Duration::from_secs(5),
        )
        .unwrap()
    }

    fn migration(&self) -> Option<ModuleMigration> {
        self.plugin_store().module_migration(MODULE).unwrap()
    }

    /// What an older app reads for GitHub, byte for byte.
    fn legacy(&self) -> Legacy {
        let store = support::open_store(&self.paths, &self.key);
        let db = store.connection_for_tests();
        let module = db
            .query_row(
                "SELECT enabled, config FROM modules WHERE id = 'github'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let mut credentials = db
            .prepare("SELECT id, description, secret, updated_at FROM credentials ORDER BY id")
            .unwrap();
        let credentials = credentials
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut profiles = db
            .prepare(
                "SELECT id, name, document_version, document, updated_at FROM profiles \
                 ORDER BY id",
            )
            .unwrap();
        let profiles = profiles
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        Legacy {
            module,
            credentials,
            profiles,
        }
    }

    /// Which implementation answered the requests made since `before`:
    /// the stand-in's and the plugin's new requests.
    fn since(&self, before: &Traffic) -> Traffic {
        Traffic {
            builtin: self.standin.seen()[before.builtin.len()..].to_vec(),
            plugin: self.plugin_log().lines().collect::<Vec<_>>()[before.plugin.len()..]
                .iter()
                .map(|l| (*l).to_owned())
                .collect(),
        }
    }

    fn traffic(&self) -> Traffic {
        Traffic {
            builtin: self.standin.seen(),
            plugin: self.plugin_log().lines().map(str::to_owned).collect(),
        }
    }

    /// Presses the key until `side` serves it (the routing reaches the
    /// registry a moment after the migration step is recorded); returns
    /// the requests it made. The other side must never see one.
    async fn press_until(&self, agent: &Agent, device: &DeviceId, side: Side) -> Traffic {
        let deadline = Instant::now() + SETTLE;
        loop {
            let before = self.traffic();
            let (name, status) = agent.press(device).await;
            let new = self.since(&before);
            assert_eq!(name, "Open pull requests");
            match side {
                Side::Builtin if !new.builtin.is_empty() => {
                    assert_eq!(status, ActionStatus::Succeeded, "{new:?}\n{}", agent.log());
                    assert_eq!(new.plugin, Vec::<String>::new());
                    return new;
                }
                Side::Plugin if !new.plugin.is_empty() => {
                    assert_eq!(status, ActionStatus::Succeeded, "{new:?}\n{}", agent.log());
                    assert_eq!(new.builtin, Vec::<String>::new());
                    return new;
                }
                _ => {}
            }
            assert!(
                Instant::now() < deadline,
                "{side:?} never served the key: {status:?}, {new:?}\n{}",
                agent.log()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Runs the workflow; its node's requests.
    async fn run_checks(&self, agent: &Agent) -> Traffic {
        let before = self.traffic();
        let record = agent.run_workflow(self.checks_workflow).await;
        assert_eq!(
            record.summary.state,
            ExecutionState::Succeeded,
            "{record:#?}\n{}",
            agent.log()
        );
        self.since(&before)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Builtin,
    Plugin,
}

/// Requests each implementation made: `<path>\t<authorization>` for the
/// built-in, `<url>\tauthorization: …` for the plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Traffic {
    builtin: Vec<String>,
    plugin: Vec<String>,
}

impl Traffic {
    fn assert_builtin(&self, path: &str) {
        assert_eq!(self.plugin, Vec::<String>::new(), "{self:?}");
        assert!(!self.builtin.is_empty(), "{self:?}");
        for line in &self.builtin {
            assert!(line.starts_with(path), "{line}");
            assert!(line.ends_with(&format!("\tBearer {TOKEN}")), "{line}");
        }
    }

    fn assert_plugin(&self, url: &str) {
        assert_eq!(self.builtin, Vec::<String>::new(), "{self:?}");
        assert!(!self.plugin.is_empty(), "{self:?}");
        for line in &self.plugin {
            assert!(line.starts_with(&format!("{API}{url}")), "{line}");
            assert!(
                line.ends_with(&format!("\tauthorization: Bearer {TOKEN}")),
                "{line}"
            );
        }
    }
}

struct Agent {
    child: Child,
    paths: AgentPaths,
    log: PathBuf,
    /// Every answer the agent gave, to check that none holds the token.
    answers: Mutex<Vec<String>>,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let answers = self.answers.lock().unwrap();
        if !std::thread::panicking() {
            for answer in answers.iter() {
                assert!(!answer.contains(TOKEN), "the token in an answer: {answer}");
            }
            assert!(!self.log().contains(TOKEN), "the token in the agent's log");
        }
    }
}

impl Agent {
    async fn client(&self) -> AgentClient {
        let deadline = Instant::now() + STARTUP;
        loop {
            match AgentClient::connect(&self.paths, ClientKind::Other).await {
                Ok(client) => return client,
                Err(e) => {
                    assert!(
                        Instant::now() < deadline,
                        "the agent did not start: {e}\n{}",
                        self.log()
                    );
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    async fn request(&self, request: Request) -> Result<Response, ClientError> {
        let what = request.name();
        let answer = tokio::time::timeout(Duration::from_secs(120), async {
            self.client().await.request(request).await
        })
        .await
        .unwrap_or_else(|_| panic!("{what} took too long"));
        self.answers.lock().unwrap().push(format!("{answer:?}"));
        answer
    }

    async fn ok(&self, request: Request) -> Response {
        let what = request.name();
        self.request(request)
            .await
            .unwrap_or_else(|e| panic!("{what}: {e}\n{}", self.log()))
    }

    /// The simulated MK.2, once it is connected.
    async fn device(&self) -> DeviceId {
        let deadline = Instant::now() + STARTUP;
        loop {
            let Response::Status { status } = self.ok(Request::GetStatus).await else {
                panic!("status")
            };
            if let Some(device) = status.devices.iter().find(|d| {
                d.source == "mock" && d.state == deckpilot_device::ConnectionState::Connected
            }) {
                return device.id.clone();
            }
            assert!(Instant::now() < deadline, "the MK.2 did not connect");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Presses and releases key 0; the name and status of the action it
    /// ran, from `actionCompleted`.
    async fn press(&self, device: &DeviceId) -> (String, ActionStatus) {
        let mut events = self.client().await;
        events.subscribe().await.unwrap();
        for input in [ControlInput::Press, ControlInput::Release] {
            self.ok(Request::SimulateInput {
                device: device.clone(),
                control: ControlId::Key { index: 0 },
                input,
            })
            .await;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let event = tokio::time::timeout_at(deadline, events.next_event())
                .await
                .expect("an action completed in time")
                .unwrap();
            self.answers.lock().unwrap().push(format!("{event:?}"));
            if let AgentEvent::ActionCompleted { name, status, .. } = event {
                return (name, status);
            }
        }
    }

    async fn run_workflow(&self, workflow: WorkflowId) -> deckpilot_core::ExecutionRecord {
        let Response::ExecutionStarted { execution } =
            self.ok(Request::StartWorkflow { workflow }).await
        else {
            panic!("startWorkflow")
        };
        let deadline = Instant::now() + SETTLE;
        loop {
            let Response::Execution { execution: record } = self
                .ok(Request::GetExecution {
                    execution: execution.id,
                })
                .await
            else {
                panic!("getExecution")
            };
            if record.summary.state != ExecutionState::Running {
                return *record;
            }
            assert!(Instant::now() < deadline, "still running: {record:#?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Runs the key's action until it is unavailable; why.
    async fn wait_unavailable(&self, action: ActionInstanceId) -> String {
        let deadline = Instant::now() + SETTLE;
        loop {
            let why = match self.request(Request::RunAction { action }).await {
                Err(ClientError::Agent(error)) => Some(error.message),
                Ok(Response::ActionResult { result }) if result.status == ActionStatus::Failed => {
                    Some(result.message)
                }
                Ok(_) | Err(_) => None,
            };
            if let Some(why) = why {
                return why;
            }
            assert!(Instant::now() < deadline, "still available\n{}", self.log());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn plugin_state(&self) -> Option<PluginState> {
        match self.ok(Request::ListPlugins).await {
            Response::Plugins { plugins } => plugins
                .into_iter()
                .find(|p| p.id == PLUGIN)
                .map(|p| p.state),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn plugin_origin(&self) -> Option<PluginOrigin> {
        match self.ok(Request::ListPlugins).await {
            Response::Plugins { plugins } => plugins
                .into_iter()
                .find(|p| p.id == PLUGIN)
                .map(|p| p.origin),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// GitHub's migration, as the app lists it.
    async fn listed(&self) -> ModuleMigrationInfo {
        match self.ok(Request::ListModuleMigrations).await {
            Response::ModuleMigrations { migrations } => migrations
                .into_iter()
                .find(|m| m.module == MODULE)
                .unwrap_or_else(|| panic!("no migration of {MODULE}\n{}", self.log())),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn wait_listed(&self, phase: ipc::MigrationPhase) -> ModuleMigrationInfo {
        let deadline = Instant::now() + SETTLE;
        loop {
            let info = self.listed().await;
            if info.phase == phase {
                return info;
            }
            assert!(
                Instant::now() < deadline,
                "not {phase:?}: {info:?}\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn decide(&self, approve: &[&str], decline: &[&str]) -> ModuleMigrationInfo {
        match self
            .ok(Request::DecideModuleMigration {
                module: MODULE.into(),
                approve: approve.iter().map(|k| (*k).to_owned()).collect(),
                decline: decline.iter().map(|k| (*k).to_owned()).collect(),
            })
            .await
        {
            Response::ModuleMigration { migration } => *migration,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn retry(&self) -> ModuleMigrationInfo {
        match self
            .ok(Request::RetryModuleMigration {
                module: MODULE.into(),
            })
            .await
        {
            Response::ModuleMigration { migration } => *migration,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn stop(self) {
        let _ = self.request(Request::Shutdown).await;
        let deadline = Instant::now() + STARTUP;
        let mut agent = self;
        while agent.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the agent did not stop");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

const SEARCH: &str = "/search/issues";
const CHECKS: &str = "/repos/octo/site/commits/main/";

// ---------------------------------------------------------------- tests

/// An alpha.3 GitHub user upgrades to a build with plugins: the bundled
/// plugin is seeded and pinned, the settings copied, and the token waits
/// for the user's consent while the built-in keeps serving. Approved
/// through IPC, the key and the workflow node run in the plugin with the
/// imported token. `deckpilot.db` is byte-identical, so a build without
/// plugins still runs the built-in. The user disabling the plugin makes
/// the legacy actions unavailable, with no fallback; uninstalling it and
/// upgrading again does not bring it back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one upgrade, step by step
async fn an_alpha3_github_user_moves_to_the_plugin_after_consent() {
    let env = Env::new("plugins-cutover-alpha3", API);
    let before = env.legacy();
    let id = PluginId::parse(PLUGIN).unwrap();

    // 1. The upgrade: seeded from the bundle, settings copied, waiting.
    let agent = env.start(Start::default()).await;
    let waiting = agent
        .wait_listed(ipc::MigrationPhase::AwaitingConsent)
        .await;
    eprintln!("step 1: {:?}, route {:?}", waiting.phase, waiting.route);
    assert_eq!(waiting.plugin, PLUGIN);
    assert_eq!(waiting.module_name, "GitHub");
    assert_eq!(waiting.route, MigrationRoute::Builtin);
    let legacy: Vec<&str> = waiting.aliases.iter().map(|a| a.legacy.as_str()).collect();
    assert_eq!(
        legacy,
        [
            "github.current-user",
            "github.repository",
            "github.pull-requests",
            "github.review-requests",
            "github.checks",
            "github.open",
        ]
    );
    // The token, and the `apiUrl` slot (a GitHub Enterprise Server the
    // user may enter later; it allows no server now), as for GitLab.com.
    let [slot, item] = waiting.consent.as_slice() else {
        panic!("the token and the slot wait: {waiting:?}");
    };
    assert_eq!(slot.key, "network:user-origin:apiUrl");
    assert_eq!(slot.decision, ConsentDecision::Pending);
    assert_eq!(slot.origin, None, "no server is asked for");
    assert_eq!(item.key, "credentials:token");
    assert_eq!(item.decision, ConsentDecision::Pending);
    assert_eq!(item.permission.tier, ipc::PermissionTier::High);
    assert_eq!(
        item.import,
        Some(ipc::CredentialImport {
            setting: "credential".into(),
            credential: CREDENTIAL.into(),
            slot: "token".into(),
            hosts: vec!["api.github.com".into()],
            user_origin: Some("apiUrl".into()),
        })
    );
    assert_eq!(agent.plugin_origin().await, Some(PluginOrigin::Bundled));
    assert_eq!(agent.plugin_state().await, Some(PluginState::Disabled));
    let store = env.plugin_store();
    let installed = store.plugin(&id).unwrap().expect("seeded");
    eprintln!(
        "step 1: installed {:?} {} ({:?})",
        installed.source, installed.versions[0].version, installed.state
    );
    assert_eq!(
        installed.source,
        deckpilot_plugin_store::InstallSource::Bundled
    );
    let grants: Vec<(String, String)> = store
        .grants(&id)
        .unwrap()
        .into_iter()
        .map(|g| (g.key.to_string(), g.decided_by))
        .collect();
    assert_eq!(
        grants,
        [
            ("network:api.github.com".into(), "migration:github".into()),
            ("openUrl:github.com".into(), "migration:github".into()),
            (
                "openUrl:user-origin:apiUrl".into(),
                "migration:github".into()
            ),
        ],
        "derived: the built-in's origin and web host, and opening pages of a server the \
         user allows (which opens nothing before one is allowed); never the token"
    );
    assert_eq!(store.credentials(&id).unwrap(), [], "nothing imported yet");
    let settings = store.settings(&id).unwrap();
    eprintln!("step 1: copied settings {settings:?}");
    assert_eq!(settings["timeoutSeconds"], json!(20));
    drop(store);

    // Meanwhile the built-in serves the key and the node, with the token
    // of `github.token`.
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin)
        .await
        .assert_builtin(SEARCH);
    env.run_checks(&agent).await.assert_builtin(CHECKS);
    eprintln!("step 1: the key and the node ran the built-in");

    // 2. The user approves in the app's consent dialog.
    let done = agent
        .decide(&["credentials:token", "network:user-origin:apiUrl"], &[])
        .await;
    eprintln!("step 2: {:?}, route {:?}", done.phase, done.route);
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    assert_eq!(done.route, MigrationRoute::Plugin);
    assert_eq!(agent.plugin_state().await, Some(PluginState::Enabled));
    assert_eq!(
        env.plugin_store().credentials(&id).unwrap().len(),
        1,
        "the token, imported into the slot"
    );

    // The same key and node now run the plugin, with the imported token.
    env.press_until(&agent, &device, Side::Plugin)
        .await
        .assert_plugin(SEARCH);
    env.run_checks(&agent).await.assert_plugin(CHECKS);
    eprintln!("step 2: the key and the node ran the plugin");
    agent.stop().await;

    // 3. Nothing an older app reads changed, byte for byte.
    assert_eq!(env.legacy(), before);
    eprintln!("step 3: deckpilot.db unchanged");

    // 4. An older build (plugins off) on the same data: the built-in.
    let agent = env.start(Start { plugins_off: true }).await;
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin)
        .await
        .assert_builtin(SEARCH);
    env.run_checks(&agent).await.assert_builtin(CHECKS);
    agent.stop().await;
    eprintln!("step 4: plugins off, the built-in ran");

    // 5. Upgrading again: still the plugin, nothing redone.
    let agent = env.start(Start::default()).await;
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Plugin)
        .await
        .assert_plugin(SEARCH);
    assert_eq!(env.migration().unwrap().phase, MigrationPhase::CutOver);

    // 6. The user disables the plugin: unavailable, no fallback.
    agent
        .ok(Request::DisablePlugin {
            plugin: PLUGIN.into(),
        })
        .await;
    let why = agent.wait_unavailable(env.pull_requests).await;
    eprintln!("step 6: disabled: {why}");
    assert!(why.contains("disabled"), "{why}");
    let info = agent.listed().await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    let before_press = env.traffic();
    let (_, status) = agent.press(&device).await;
    assert_eq!(status, ActionStatus::Failed);
    // The workflow does not start: its node names the disabled plugin.
    let refused = agent
        .request(Request::StartWorkflow {
            workflow: env.checks_workflow,
        })
        .await
        .unwrap_err();
    eprintln!("step 6: the workflow: {refused}");
    assert!(
        refused
            .to_string()
            .contains("`deckpilot.github`) is disabled"),
        "{refused}"
    );
    assert_eq!(
        env.since(&before_press),
        Traffic::default(),
        "neither implementation ran"
    );

    // 7. The user uninstalls it; an upgrade does not bring it back.
    agent
        .ok(Request::UninstallPlugin {
            plugin: PLUGIN.into(),
            keep: deckpilot_ipc::plugins::PluginRetention::default(),
        })
        .await;
    agent.wait_unavailable(env.pull_requests).await;
    agent.stop().await;
    env.stage();
    for _ in 0..2 {
        let agent = env.start(Start::default()).await;
        let why = agent.wait_unavailable(env.pull_requests).await;
        assert_eq!(agent.plugin_state().await, None, "{}", agent.log());
        let info = agent.listed().await;
        eprintln!(
            "step 7: after uninstall and upgrade: {:?}, route {:?}: {why}",
            info.phase, info.route
        );
        assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
        let quiet = env.traffic();
        let (_, status) = agent.press(&agent.device().await).await;
        assert_eq!(status, ActionStatus::Failed);
        assert_eq!(env.since(&quiet), Traffic::default());
        agent.stop().await;
    }
    assert_eq!(
        env.plugin_store().tombstone(&id).unwrap().unwrap().decision,
        TombstoneDecision::Uninstalled
    );

    // Nothing of the user's was deleted or rewritten.
    assert_eq!(env.legacy(), before);
}

/// A GitHub Enterprise user is asked for the token **and the server**
/// (the `apiUrl` slot), never derived; the built-in serves until both are
/// allowed, and declining the server keeps it, across restarts. The full
/// upgrade to the plugin at that server is
/// `plugins_cutover_servers.rs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_github_enterprise_user_is_asked_for_the_server() {
    const GHE: &str = "https://ghe.example.com/api/v3";
    let env = Env::new("plugins-cutover-ghe", GHE);
    let id = PluginId::parse(PLUGIN).unwrap();
    let before = env.legacy();
    let agent = env.start(Start::default()).await;
    let info = agent
        .wait_listed(ipc::MigrationPhase::AwaitingConsent)
        .await;
    eprintln!("GHE: {:?}: {:?}", info.phase, info.consent);
    let pending: Vec<&str> = info
        .consent
        .iter()
        .filter(|i| i.decision == ConsentDecision::Pending)
        .map(|i| i.key.as_str())
        .collect();
    for key in [
        "credentials:token",
        "network:user-origin:apiUrl",
        "network:api.github.com",
        "openUrl:github.com",
    ] {
        assert!(pending.contains(&key), "{key} is not asked: {pending:?}");
    }
    let server = info
        .consent
        .iter()
        .find(|i| i.key == "network:user-origin:apiUrl")
        .and_then(|i| i.origin.as_ref())
        .unwrap();
    assert_eq!(server.origin, GHE);
    assert!(!server.private_network);
    assert_eq!(info.route, MigrationRoute::Builtin);
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    assert_eq!(
        env.plugin_store().user_origins(&id).unwrap().len(),
        0,
        "no server allowed"
    );
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin)
        .await
        .assert_builtin(SEARCH);

    // Everything but the server: declined, the built-in keeps serving.
    let others: Vec<&str> = pending
        .iter()
        .copied()
        .filter(|k| *k != "network:user-origin:apiUrl")
        .collect();
    let declined = agent.decide(&others, &["network:user-origin:apiUrl"]).await;
    eprintln!("GHE declined: {:?}: {:?}", declined.phase, declined.reason);
    assert_eq!(declined.phase, ipc::MigrationPhase::NotEligible);
    assert!(declined.can_retry);
    assert_eq!(declined.route, MigrationRoute::Builtin);
    assert_eq!(
        env.plugin_store().user_origins(&id).unwrap().len(),
        0,
        "no server allowed"
    );
    env.press_until(&agent, &device, Side::Builtin)
        .await
        .assert_builtin(SEARCH);
    env.run_checks(&agent).await.assert_builtin(CHECKS);
    agent.stop().await;
    // Again at the next start.
    let agent = env.start(Start::default()).await;
    assert_eq!(agent.listed().await.phase, ipc::MigrationPhase::NotEligible);
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin)
        .await
        .assert_builtin(SEARCH);
    agent.stop().await;
    assert_eq!(env.legacy(), before);
    assert_eq!(env.plugin_log(), "", "the plugin never ran");
}

/// The user declines the token: the built-in keeps serving, across
/// restarts, without asking again, until Retry asks again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declining_keeps_the_builtin_until_retry() {
    let env = Env::new("plugins-cutover-decline", API);
    let id = PluginId::parse(PLUGIN).unwrap();
    let agent = env.start(Start::default()).await;
    agent
        .wait_listed(ipc::MigrationPhase::AwaitingConsent)
        .await;
    let declined = agent
        .decide(&["network:user-origin:apiUrl"], &["credentials:token"])
        .await;
    eprintln!("declined: {:?}: {:?}", declined.phase, declined.reason);
    assert_eq!(declined.phase, ipc::MigrationPhase::NotEligible);
    assert!(declined.can_retry);
    assert_eq!(declined.route, MigrationRoute::Builtin);
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin)
        .await
        .assert_builtin(SEARCH);
    agent.stop().await;

    let agent = env.start(Start::default()).await;
    let info = agent.listed().await;
    assert_eq!(info.phase, ipc::MigrationPhase::NotEligible, "{info:?}");
    let token = |info: &ModuleMigrationInfo| {
        info.consent
            .iter()
            .find(|i| i.key == "credentials:token")
            .map(|i| i.decision)
    };
    assert_eq!(token(&info), Some(ConsentDecision::Declined), "{info:?}");
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin)
        .await
        .assert_builtin(SEARCH);
    env.run_checks(&agent).await.assert_builtin(CHECKS);

    let again = agent.retry().await;
    eprintln!("retry: {:?}", again.phase);
    assert_eq!(again.phase, ipc::MigrationPhase::AwaitingConsent);
    assert_eq!(token(&again), Some(ConsentDecision::Pending), "{again:?}");
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    let pending: Vec<&str> = again
        .consent
        .iter()
        .filter(|i| i.decision == ConsentDecision::Pending)
        .map(|i| i.key.as_str())
        .collect();
    let done = agent.decide(&pending, &[]).await;
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    env.press_until(&agent, &device, Side::Plugin)
        .await
        .assert_plugin(SEARCH);
    agent.stop().await;
}
