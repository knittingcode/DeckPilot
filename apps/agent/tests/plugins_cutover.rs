//! The GitHub cut-over (`docs/marketplace/module-migration.md` §9, P9, and
//! §11 D1: the built-in `github` module is removed from the app) end to
//! end, through the real agent, the real sandboxed plugin host, real
//! storage and IPC, each test in its own data directory (feature
//! `plugins`). What it proves: a 0.9.x configuration migrates to the
//! plugin **without the built-in**, and nothing runs anywhere until the
//! plugin can serve it.
//!
//! * a 0.9.x (alpha.3-style) `deckpilot.db`: the `github` module row saved
//!   and enabled with GitHub's API URL, its token stored as the credential
//!   `github.token`, a profile with an MK.2 key bound to
//!   `github.pull-requests` and a workflow node running `github.checks`
//!   (legacy ids). The agent has no `github` built-in: the migration reads
//!   that row through the frozen descriptors;
//! * the shipped alias block (`ACTION_ALIASES`) and migration plan, and
//!   `plugins/first-party/github` built to wasm and bundled as the
//!   release bundles it (class `migration`, pinned with
//!   `DECKPILOT_TEST_BUNDLED_PINS`);
//! * GitHub never answers: the plugin's requests get canned answers
//!   (`DECKPILOT_TEST_PLUGIN_HTTP`, which logs every request with the
//!   credential header the agent injected), so the log is everything sent;
//! * until the plugin serves (consent pending or declined, plugins off,
//!   plugin disabled or uninstalled) the legacy key and node are
//!   unavailable with a clear reason, the agent keeps answering, and
//!   nothing is sent; once consented the same stored key and node run in
//!   the plugin with the imported token;
//! * the consent, Retry, disable and uninstall come through the app's
//!   requests; `deckpilot.db` is never rewritten and restarts redo nothing.
#![cfg(feature = "plugins")]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
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

/// GitHub's answer to `GET <path>`, for the plugin: open pull
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
    pull_requests: ActionInstanceId,
    checks_workflow: WorkflowId,
}

impl Env {
    /// A 0.9.x configuration with `api_url` as the module's API URL.
    fn new(name: &str, api_url: &str) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let mut env = Self {
            dir,
            paths,
            key,
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

    /// The (now removed) built-in module saved and enabled as 0.9.x saved
    /// it, its token stored, and a
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

    /// Everything sent so far: one line per request of the plugin (there
    /// is no built-in left to send anything).
    fn sent(&self) -> Vec<String> {
        self.plugin_log().lines().map(str::to_owned).collect()
    }

    /// What was sent after the first `before` requests.
    fn since(&self, before: usize) -> Sent {
        Sent(self.sent()[before..].to_vec())
    }

    /// Presses the key until the plugin serves it (the routing reaches the
    /// registry a moment after the migration step is recorded); returns
    /// the requests it made.
    async fn press_until_served(&self, agent: &Agent, device: &DeviceId) -> Sent {
        let deadline = Instant::now() + SETTLE;
        loop {
            let before = self.sent().len();
            let (name, status) = agent.press(device).await;
            let new = self.since(before);
            assert_eq!(name, "Open pull requests");
            if !new.0.is_empty() {
                assert_eq!(status, ActionStatus::Succeeded, "{new:?}\n{}", agent.log());
                return new;
            }
            assert!(
                Instant::now() < deadline,
                "the plugin never served the key: {status:?}\n{}",
                agent.log()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Runs the workflow; its node's requests.
    async fn run_checks(&self, agent: &Agent) -> Sent {
        let before = self.sent().len();
        let record = agent.run_workflow(self.checks_workflow).await;
        assert_eq!(
            record.summary.state,
            ExecutionState::Succeeded,
            "{record:#?}\n{}",
            agent.log()
        );
        self.since(before)
    }

    /// The legacy key and the workflow node are unavailable, with a reason
    /// that contains `why`: running the action fails with it, the key
    /// press fails, the workflow does not start (its node names it), the
    /// agent keeps answering, and nothing is sent anywhere. The reason.
    async fn assert_unavailable(&self, agent: &Agent, device: &DeviceId, why: &str) -> String {
        let reason = agent.wait_unavailable(self.pull_requests, why).await;
        let before = self.sent().len();
        let (name, status) = agent.press(device).await;
        assert_eq!(name, "Open pull requests");
        assert_eq!(status, ActionStatus::Failed);
        let refused = agent
            .request(Request::StartWorkflow {
                workflow: self.checks_workflow,
            })
            .await
            .expect_err("the workflow does not start")
            .to_string();
        assert!(refused.contains(why), "{refused}");
        let Response::Status { .. } = agent.ok(Request::GetStatus).await else {
            panic!("the agent answers")
        };
        assert_eq!(self.since(before), Sent::default(), "nothing was sent");
        reason
    }
}

/// Requests the plugin made: `<url>\tauthorization: …`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Sent(Vec<String>);

impl Sent {
    /// Requests of the plugin, to `url` below GitHub's API, each with the
    /// token the user stored with 0.9.x.
    fn assert_plugin(&self, url: &str) {
        assert!(!self.0.is_empty(), "{self:?}");
        for line in &self.0 {
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

    /// Runs the key's action until it is unavailable for a reason that
    /// contains `why` (the routing settles a moment after a step); the
    /// reason. It never runs meanwhile: nothing serves it.
    async fn wait_unavailable(&self, action: ActionInstanceId, why: &str) -> String {
        let deadline = Instant::now() + SETTLE;
        loop {
            let reason = match self.request(Request::RunAction { action }).await {
                Err(ClientError::Agent(error)) => Some(error.message),
                Ok(Response::ActionResult { result }) if result.status == ActionStatus::Failed => {
                    Some(result.message)
                }
                Ok(_) | Err(_) => None,
            };
            if let Some(reason) = &reason
                && reason.contains(why)
            {
                return reason.clone();
            }
            assert!(
                Instant::now() < deadline,
                "not unavailable because of {why:?}: {reason:?}\n{}",
                self.log()
            );
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

/// Why the legacy types are unavailable while the migration waits.
const WAITS: &str = "`deckpilot.github`) waits for your consent";

/// A 0.9.x GitHub user upgrades to a build without the built-in: the
/// bundled plugin is seeded and pinned, the settings copied, and the token
/// waits for the user's consent; meanwhile the stored key and node are
/// unavailable (nothing sent). Approved through IPC, the same key and
/// workflow node run in the plugin with the imported token.
/// `deckpilot.db` is byte-identical; a build with plugins off has nothing
/// to run them with (unavailable, saying where they come from now). The
/// user disabling the plugin makes them unavailable again; uninstalling
/// it and upgrading again does not bring it back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one upgrade, step by step
async fn a_0_9_github_user_moves_to_the_plugin_after_consent() {
    let env = Env::new("plugins-cutover-alpha3", API);
    let before = env.legacy();
    let id = PluginId::parse(PLUGIN).unwrap();

    // 1. The upgrade: seeded from the bundle, settings copied, waiting.
    let agent = env.start(Start::default()).await;
    let waiting = agent
        .wait_listed(ipc::MigrationPhase::AwaitingConsent)
        .await;
    eprintln!(
        "step 1: {:?}, route {:?}: {:?}",
        waiting.phase, waiting.route, waiting.route_reason
    );
    assert_eq!(waiting.plugin, PLUGIN);
    assert_eq!(waiting.module_name, "GitHub");
    // No built-in to fall back to: unavailable until the user decides.
    assert_eq!(waiting.route, MigrationRoute::PluginWithdrawn);
    assert!(
        waiting
            .route_reason
            .as_deref()
            .is_some_and(|r| r.contains("waits for your consent")),
        "{waiting:?}"
    );
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

    // Meanwhile the stored key and node are unavailable, saying why.
    let device = agent.device().await;
    let why = env.assert_unavailable(&agent, &device, WAITS).await;
    eprintln!("step 1: the key and the node are unavailable: {why}");
    assert_eq!(env.sent(), Vec::<String>::new(), "nothing was sent");

    // 2. The user approves in the app's consent dialog.
    let done = agent
        .decide(&["credentials:token", "network:user-origin:apiUrl"], &[])
        .await;
    eprintln!("step 2: {:?}, route {:?}", done.phase, done.route);
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    assert_eq!(done.route, MigrationRoute::Plugin);
    assert_eq!(done.route_reason, None);
    assert_eq!(agent.plugin_state().await, Some(PluginState::Enabled));
    assert_eq!(
        env.plugin_store().credentials(&id).unwrap().len(),
        1,
        "the token, imported into the slot"
    );

    // The same stored key and node now run the plugin, with the imported
    // token.
    env.press_until_served(&agent, &device)
        .await
        .assert_plugin(SEARCH);
    env.run_checks(&agent).await.assert_plugin(CHECKS);
    eprintln!("step 2: the key and the node ran the plugin");
    agent.stop().await;

    // 3. Nothing an older app reads changed, byte for byte.
    assert_eq!(env.legacy(), before);
    eprintln!("step 3: deckpilot.db unchanged");

    // 4. Plugins off on the same data: nothing can run them, and the
    // reason says where they come from now.
    let agent = env.start(Start { plugins_off: true }).await;
    let device = agent.device().await;
    let why = env
        .assert_unavailable(&agent, &device, "now comes from")
        .await;
    eprintln!("step 4: plugins off: {why}");
    assert!(
        why.contains("`github.pull-requests`") && why.contains(PLUGIN),
        "{why}"
    );
    agent.stop().await;

    // 5. Upgrading again: still the plugin, nothing redone.
    let agent = env.start(Start::default()).await;
    let device = agent.device().await;
    env.press_until_served(&agent, &device)
        .await
        .assert_plugin(SEARCH);
    assert_eq!(env.migration().unwrap().phase, MigrationPhase::CutOver);
    let info = agent.listed().await;
    assert_eq!(info.consent, [], "nothing asked again: {info:?}");

    // 6. The user disables the plugin: unavailable, nothing to fall back to.
    agent
        .ok(Request::DisablePlugin {
            plugin: PLUGIN.into(),
        })
        .await;
    let why = env
        .assert_unavailable(&agent, &device, "`deckpilot.github`) is disabled")
        .await;
    eprintln!("step 6: disabled: {why}");
    let info = agent.listed().await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");

    // 6b. Enabled again: the plugin serves again.
    agent
        .ok(Request::EnablePlugin {
            plugin: PLUGIN.into(),
            consent: Vec::new(),
        })
        .await;
    env.press_until_served(&agent, &device)
        .await
        .assert_plugin(SEARCH);
    assert_eq!(agent.listed().await.route, MigrationRoute::Plugin);

    // 7. The user uninstalls it; an upgrade does not bring it back.
    agent
        .ok(Request::UninstallPlugin {
            plugin: PLUGIN.into(),
            keep: deckpilot_ipc::plugins::PluginRetention::default(),
        })
        .await;
    agent.wait_unavailable(env.pull_requests, PLUGIN).await;
    agent.stop().await;
    env.stage();
    for _ in 0..2 {
        let agent = env.start(Start::default()).await;
        let device = agent.device().await;
        let why = env.assert_unavailable(&agent, &device, PLUGIN).await;
        assert_eq!(agent.plugin_state().await, None, "{}", agent.log());
        let info = agent.listed().await;
        eprintln!(
            "step 7: after uninstall and upgrade: {:?}, route {:?} ({:?}): {why}",
            info.phase, info.route, info.route_reason
        );
        assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
        assert!(
            info.route_reason
                .as_deref()
                .is_some_and(|r| r.contains("uninstalled")),
            "{info:?}"
        );
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
/// (the `apiUrl` slot), never derived; the stored key and node are
/// unavailable until both are allowed, and declining the server keeps
/// them unavailable (saying so), across restarts, with nothing sent. The
/// full upgrade to the plugin at that server is
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
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn);
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    assert_eq!(
        env.plugin_store().user_origins(&id).unwrap().len(),
        0,
        "no server allowed"
    );
    let device = agent.device().await;
    env.assert_unavailable(&agent, &device, WAITS).await;

    // Everything but the server: declined, still unavailable.
    let others: Vec<&str> = pending
        .iter()
        .copied()
        .filter(|k| *k != "network:user-origin:apiUrl")
        .collect();
    let declined = agent.decide(&others, &["network:user-origin:apiUrl"]).await;
    eprintln!(
        "GHE declined: {:?}: {:?}; route {:?}: {:?}",
        declined.phase, declined.reason, declined.route, declined.route_reason
    );
    assert_eq!(declined.phase, ipc::MigrationPhase::NotEligible);
    assert!(declined.can_retry);
    assert!(
        declined
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("its keys are unavailable until you ask again")),
        "{declined:?}"
    );
    assert_eq!(declined.route, MigrationRoute::PluginWithdrawn);
    assert!(
        declined
            .route_reason
            .as_deref()
            .is_some_and(|r| r.contains("you declined what deckpilot.github needs")),
        "{declined:?}"
    );
    assert_eq!(
        env.plugin_store().user_origins(&id).unwrap().len(),
        0,
        "no server allowed"
    );
    env.assert_unavailable(&agent, &device, PLUGIN).await;
    agent.stop().await;
    // Again at the next start.
    let agent = env.start(Start::default()).await;
    let info = agent.listed().await;
    assert_eq!(info.phase, ipc::MigrationPhase::NotEligible);
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn);
    let device = agent.device().await;
    env.assert_unavailable(&agent, &device, PLUGIN).await;
    agent.stop().await;
    assert_eq!(env.legacy(), before);
    assert_eq!(env.plugin_log(), "", "nothing was ever sent");
}

/// The user declines the token: the stored key and node stay
/// unavailable (saying the user declined), across restarts, without
/// asking again, until Retry asks again; approved, the plugin serves.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declining_keeps_the_legacy_types_unavailable_until_retry() {
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
    assert_eq!(declined.route, MigrationRoute::PluginWithdrawn);
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    let device = agent.device().await;
    env.assert_unavailable(&agent, &device, PLUGIN).await;
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
    env.assert_unavailable(&agent, &device, PLUGIN).await;
    assert_eq!(env.sent(), Vec::<String>::new(), "nothing was sent");

    let again = agent.retry().await;
    eprintln!("retry: {:?}", again.phase);
    assert_eq!(again.phase, ipc::MigrationPhase::AwaitingConsent);
    assert_eq!(token(&again), Some(ConsentDecision::Pending), "{again:?}");
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    env.assert_unavailable(&agent, &device, WAITS).await;
    let pending: Vec<&str> = again
        .consent
        .iter()
        .filter(|i| i.decision == ConsentDecision::Pending)
        .map(|i| i.key.as_str())
        .collect();
    let done = agent.decide(&pending, &[]).await;
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    env.press_until_served(&agent, &device)
        .await
        .assert_plugin(SEARCH);
    env.run_checks(&agent).await.assert_plugin(CHECKS);
    agent.stop().await;
}
