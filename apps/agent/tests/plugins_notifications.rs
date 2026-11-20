//! `deckpilot.notifications`, the first `default`-class bundled plugin
//! (`docs/marketplace/module-migration.md` §4, §5.8, P10), end to end
//! through the real agent, the real sandboxed plugin host, real storage and
//! IPC, each test in its own data directory (feature `plugins`):
//!
//! * the plugin (`plugins/first-party/notifications`) is built to wasm with
//!   the `guest` profile and bundled as `plugins/first-party/build.sh`
//!   stages it for a release (class `default`, pinned with
//!   `DECKPILOT_TEST_BUNDLED_PINS`);
//! * native notifications go to a file (`DECKPILOT_TEST_NOTIFICATIONS`,
//!   feature `test-hooks`), never to the desktop; a `.deny` file next to it
//!   makes them fail as on a desktop where they are turned off. The status
//!   bar is an app connected as `desktop` and subscribed;
//! * no registry is configured: everything is offline.
//!
//! The tests:
//!
//! * [`the_plugin_answers_as_the_builtin_does`]: every action of the plugin
//!   through the agent and the host against the built-in (in this process,
//!   with a notifier that decides as the agent's does) in the same
//!   conditions: the same status, message, outputs, error code and native
//!   notification. The differences are listed in [`DIFFERENT_REASONS`] and
//!   in `plugins/first-party/notifications/README.md`;
//! * [`a_fresh_install_gets_the_plugin_on_first_launch`]: seeded, enabled
//!   with its one low-tier permission only, cut over, its actions and the
//!   legacy ids running in the plugin at the first start;
//! * [`removing_or_disabling_the_plugin_sticks_across_restarts_and_upgrades`][]:
//!   tombstones;
//! * [`an_alpha3_user_keeps_notifications_through_the_alias`][]: settings
//!   copied, keys and workflow nodes on the plugin, and an older build on
//!   the same data still on the built-in.
#![cfg(feature = "plugins")]

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use deckpilot_core::notify::{Delivery, Notification, Notifier, Severity};
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionResult, ActionStatus, ActionTypeId,
    ExecutionContext, ExecutionRecord, ExecutionState, MachineConfig, ModuleId, ModuleRegistry,
    ModuleSettings, Os, ParameterValue, Profile, WorkflowId,
};
use deckpilot_ipc::plugins::{
    MigrationPhase, MigrationRoute, ModuleMigrationInfo, PluginActionOutcome, PluginDetails,
    PluginOrigin, PluginState, PluginSummary, reason,
};
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_modules::notifications;
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_store::{GrantStatus, PluginStore, TombstoneDecision};
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore, DatabaseKey};
use serde_json::{Value, json};
use support::TempDir;

const ID: &str = "deckpilot.notifications";
const MODULE: &str = "notifications";
/// The plugin's name, which the agent puts before every title.
const PREFIX: &str = "Notifications: ";
const STARTUP: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(60);
/// What `notify` lets a plugin show in a minute.
const NOTIFY_RATE: usize = 5;

/// Scenarios whose `reason` (and so message) differs, and why. Everything
/// else is the same.
const DIFFERENT_REASONS: &[(&str, &str, &str)] = &[
    (
        "show, below the least severity",
        "less important than the plugin's setting",
        "the built-in names its module's setting, the agent the plugin's",
    ),
    (
        "show, both off",
        "notifications are turned off in Settings › Plugins › Notifications",
        "the setting is the plugin's, in Settings › Plugins, not Settings › Notifications",
    ),
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime() -> PathBuf {
    repo().join("plugins/runtime")
}

fn source() -> PathBuf {
    repo().join("plugins/first-party/notifications")
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
            .filter(|dir| dir.join("deckpilot_notifications.wasm").is_file())
            .unwrap_or_else(|| {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    "deckpilot-notifications",
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
            module: wasm_dir.join("deckpilot_notifications.wasm"),
        }
    })
}

// ---------------------------------------------------------- environment

/// One data directory and the application bundle the agent starts from.
struct Env {
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
}

impl Env {
    /// A data directory with an empty encrypted database, as the app
    /// creates it on a fresh install, and the bundle of this build.
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let env = Self { dir, paths, key };
        env.stage(None);
        env
    }

    /// A data directory as 0.6.0 left it: the database and vault of its
    /// validation seed (`crates/deckpilot-storage/tests/fixtures/v0.6.0`),
    /// no `plugins/`, and the bundle of this build.
    fn from_v0_6_0(name: &str) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let fixture = repo().join("crates/deckpilot-storage/tests/fixtures/v0.6.0");
        std::fs::copy(fixture.join("deckpilot.db"), &paths.database).unwrap();
        std::fs::copy(
            fixture.join("deckpilot.vault.json"),
            paths.data_dir.join("deckpilot.vault.json"),
        )
        .unwrap();
        let key = Vault::new(&paths.database)
            .unlock(&Password::new("deckpilot-dev"))
            .unwrap();
        let env = Self { dir, paths, key };
        env.stage(None);
        env
    }

    fn bundled(&self) -> PathBuf {
        self.dir.path().join("resources/bundled")
    }

    fn pins_file(&self) -> PathBuf {
        self.dir.path().join("compiled-pins.json")
    }

    fn notifications_file(&self) -> PathBuf {
        self.dir.path().join("notifications.jsonl")
    }

    fn deny_file(&self) -> PathBuf {
        self.dir.path().join("notifications.jsonl.deny")
    }

    /// Native notifications fail from now on (`on`), or work again.
    fn deny(&self, on: bool) {
        if on {
            std::fs::write(self.deny_file(), b"").unwrap();
        } else {
            let _ = std::fs::remove_file(self.deny_file());
        }
    }

    /// The native notifications shown so far, in order.
    fn shown(&self) -> Vec<Notification> {
        std::fs::read_to_string(self.notifications_file())
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// `bundled/deckpilot.notifications/` as `build.sh` stages it for a
    /// release (class `default`, `bundled.json`), and the pins the agent
    /// "was compiled with". `version`: an app update that bundles a newer
    /// version (only its manifest says so; it is never loaded here).
    fn stage(&self, version: Option<&str>) {
        let dir = self.bundled().join(ID);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for file in ["LICENSE", "assets/icon-128.png"] {
            std::fs::copy(source().join(file), dir.join(file)).unwrap();
        }
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(source().join("manifest.json")).unwrap())
                .unwrap();
        if let Some(version) = version {
            manifest["version"] = json!(version);
        }
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::copy(&artifacts().module, dir.join("plugin.wasm")).unwrap();
        let version = semver::Version::parse(manifest["version"].as_str().unwrap()).unwrap();
        let released: Value = serde_json::from_slice(
            &std::fs::read(repo().join("plugins/first-party/bundled.json")).unwrap(),
        )
        .unwrap();
        assert!(
            released["plugins"]
                .as_array()
                .unwrap()
                .contains(&json!({"path": "notifications", "class": "default"})),
            "the release bundles deckpilot.notifications as a default-class plugin: {released}"
        );
        let pins = Pins::compute(
            &self.bundled(),
            &[(PluginId::parse(ID).unwrap(), version, BundleClass::Default)],
        )
        .unwrap();
        std::fs::write(self.bundled().join("pins.json"), pins.to_json_pretty()).unwrap();
        std::fs::write(self.pins_file(), pins.to_json_pretty()).unwrap();
    }

    /// Starts the agent and unlocks it; `plugins` off is an older build
    /// (or one without the feature) on the same data.
    async fn start(&self, plugins: bool) -> Agent {
        let agent = self.spawn(plugins);
        agent
            .request(Request::Unlock {
                key: KeyMaterial::new(self.key.to_hex()),
            })
            .await
            .unwrap();
        agent
    }

    /// Starts the agent, locked. With `plugins` it gets no plugin switch,
    /// as the configuration app and the login item start it; without,
    /// `--disable-plugins`.
    fn spawn(&self, plugins: bool) -> Agent {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.path().join("agent.log"))
            .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"));
        command
            .args(["--headless", "--no-usb", "--log-level", "debug"])
            .arg("--plugin-host")
            .arg(&artifacts().host)
            .arg("--bundled-plugins")
            .arg(self.bundled())
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_BUNDLED_PINS", self.pins_file())
            .env("DECKPILOT_TEST_NOTIFICATIONS", self.notifications_file())
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
        if !plugins {
            command.arg("--disable-plugins");
        }
        Agent {
            child: command.spawn().expect("the agent starts"),
            paths: self.paths.clone(),
            log: self.dir.path().join("agent.log"),
            app: Mutex::new(None),
        }
    }

    fn plugin_store(&self) -> PluginStore {
        PluginStore::open(
            &deckpilot_plugin_store::Layout::new(&self.paths.data_dir).database(),
            &deckpilot_agent::plugins::key::plugin_store_key(&self.key),
            Duration::from_secs(5),
        )
        .unwrap()
    }

    fn store(&self) -> deckpilot_storage::SqliteStore {
        support::open_store(&self.paths, &self.key)
    }

    /// The `modules` row and the profile documents, raw: what an older app
    /// reads.
    fn legacy(&self) -> Legacy {
        let store = self.store();
        let db = store.connection_for_tests();
        let module = db
            .query_row(
                "SELECT enabled, config FROM modules WHERE id = 'notifications'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let mut profiles = db
            .prepare("SELECT id, document, updated_at FROM profiles ORDER BY id")
            .unwrap();
        let profiles = profiles
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        (module, profiles)
    }
}

/// The `modules` row (enabled, config) and the profile documents (id,
/// document, updated at).
type Legacy = (Option<(i64, String)>, Vec<(String, String, i64)>);

struct Agent {
    child: Child,
    paths: AgentPaths,
    log: PathBuf,
    /// The configuration app, open (connected as `desktop`, subscribed).
    app: Mutex<Option<AgentClient>>,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What an action answered, as the engine sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Outcome {
    ok: bool,
    message: String,
    data: BTreeMap<String, String>,
    code: Option<String>,
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
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
        tokio::time::timeout(Duration::from_secs(120), async {
            self.client().await.request(request).await
        })
        .await
        .unwrap_or_else(|_| panic!("{what} took too long"))
    }

    async fn ok(&self, request: Request) -> Response {
        let what = request.name();
        self.request(request)
            .await
            .unwrap_or_else(|e| panic!("{what}: {e}\n{}", self.log()))
    }

    /// The configuration app opens: its status bar shows notifications.
    async fn open_app(&self) {
        let mut app = AgentClient::connect(&self.paths, ClientKind::Desktop)
            .await
            .unwrap();
        app.subscribe().await.unwrap();
        *self.app.lock().unwrap() = Some(app);
    }

    /// The next notification the open app's status bar got.
    async fn status_bar(&self) -> Notification {
        let mut app = self.app.lock().unwrap().take().expect("the app is open");
        let deadline = tokio::time::Instant::now() + SETTLE;
        let shown = loop {
            let event = tokio::time::timeout_at(deadline, app.next_event())
                .await
                .expect("a notification in time")
                .unwrap();
            if let AgentEvent::Notification { notification } = event {
                break notification;
            }
        };
        *self.app.lock().unwrap() = Some(app);
        shown
    }

    async fn invoke(&self, action: &str, params: &Value) -> Outcome {
        match self
            .ok(Request::InvokePluginAction {
                plugin: ID.into(),
                action: action.into(),
                params: serde_json::from_value(params.clone()).unwrap(),
            })
            .await
        {
            Response::PluginActionResult { outcome } => match outcome {
                PluginActionOutcome::Ok { message, data } => Outcome {
                    ok: true,
                    message: message.unwrap_or_default(),
                    data: data.iter().map(|(k, v)| (k.clone(), text(v))).collect(),
                    code: None,
                },
                PluginActionOutcome::Failed { code, message, .. } => Outcome {
                    ok: false,
                    message: message.unwrap_or_default(),
                    data: BTreeMap::new(),
                    code: Some(code),
                },
                PluginActionOutcome::Skipped { message } => panic!("skipped: {message:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn set_settings(&self, values: &Value) {
        self.ok(Request::SetPluginSettings {
            plugin: ID.into(),
            values: serde_json::from_value(values.clone()).unwrap(),
        })
        .await;
    }

    async fn run_action(&self, action: ActionInstanceId) -> ActionResult {
        match self.ok(Request::RunAction { action }).await {
            Response::ActionResult { result } => result,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn run_workflow(&self, workflow: WorkflowId) -> ExecutionRecord {
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

    /// Runs `action` until it is unavailable; why.
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

    async fn plugin(&self) -> Option<PluginSummary> {
        match self.ok(Request::ListPlugins).await {
            Response::Plugins { plugins } => plugins.into_iter().find(|p| p.id == ID),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn details(&self) -> PluginDetails {
        match self.ok(Request::GetPlugin { plugin: ID.into() }).await {
            Response::Plugin { plugin } => *plugin,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn migration(&self) -> Option<ModuleMigrationInfo> {
        match self.ok(Request::ListModuleMigrations).await {
            Response::ModuleMigrations { migrations } => {
                migrations.into_iter().find(|m| m.module == MODULE)
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The notifications migration once it is in `phase` with `route`.
    async fn wait_migration(
        &self,
        phase: MigrationPhase,
        route: MigrationRoute,
    ) -> ModuleMigrationInfo {
        let deadline = Instant::now() + SETTLE;
        loop {
            let info = self.migration().await;
            if let Some(info) = info
                .as_ref()
                .filter(|i| i.phase == phase && i.route == route)
            {
                return info.clone();
            }
            assert!(
                Instant::now() < deadline,
                "not {phase:?} {route:?}: {info:?}\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Runs `action` until the plugin serves it (the routing reaches the
    /// registry a moment after the cut-over is recorded): the native
    /// notification is titled with the plugin's name.
    async fn run_on_plugin(&self, env: &Env, action: ActionInstanceId) -> ActionResult {
        let deadline = Instant::now() + SETTLE;
        loop {
            let before = env.shown().len();
            let result = self.run_action(action).await;
            let shown = env.shown();
            if shown.len() > before && shown[before].title.starts_with(PREFIX) {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "not on the plugin: {result:?}\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn stop(self) {
        let _ = self.request(Request::Shutdown).await;
        let deadline = Instant::now() + STARTUP;
        let mut agent = self;
        agent.app.lock().unwrap().take();
        while agent.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the agent did not stop");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

// ------------------------------------------------------- the built-in

/// The agent's notifier as the built-in sees it (`AgentNotifier`): the
/// status bar while the app is open, the native notifier (here a record,
/// or a refusal as on a desktop where notifications are off).
#[derive(Debug)]
struct AppNotifier {
    app_open: bool,
    native_ok: bool,
    shown: Mutex<Vec<Notification>>,
}

impl Notifier for AppNotifier {
    fn notify(&self, notification: &Notification) -> Result<Delivery, String> {
        let status_bar = notification.status_bar && self.app_open;
        let (desktop, problem) = match (notification.desktop, self.native_ok) {
            (false, _) => (false, None),
            (true, true) => {
                self.shown.lock().unwrap().push(notification.clone());
                (true, None)
            }
            (true, false) => (
                false,
                Some("notifications are turned off for DeckPilot".to_owned()),
            ),
        };
        if status_bar || desktop {
            Ok(Delivery {
                desktop,
                status_bar,
            })
        } else {
            Err(problem.unwrap_or_else(|| {
                "the app is not open and desktop notifications are off".to_owned()
            }))
        }
    }
}

/// The plugin's settings as the built-in module's configuration.
fn module_config(settings: &Value) -> ActionParameters {
    let mut config = ActionParameters::new();
    for (key, value) in settings.as_object().unwrap() {
        let value = match value {
            Value::Bool(b) => ParameterValue::Boolean(*b),
            Value::String(s) => ParameterValue::Choice(s.clone()),
            other => panic!("{other:?}"),
        };
        config = config.with(key.clone(), value);
    }
    config
}

fn parameters(params: &Value) -> ActionParameters {
    let mut out = ActionParameters::new();
    for (key, value) in params.as_object().unwrap() {
        let value = match value {
            Value::String(s) if key == "severity" => ParameterValue::Choice(s.clone()),
            Value::String(s) => ParameterValue::Text(s.clone()),
            Value::Bool(b) => ParameterValue::Boolean(*b),
            other => panic!("{other:?}"),
        };
        out = out.with(key.clone(), value);
    }
    out
}

/// One action of the built-in, with its settings, in the conditions of a
/// scenario; and the native notifications it showed.
fn run_builtin(scenario: &Scenario, settings: &Value) -> (Outcome, Vec<Notification>) {
    let mut modules = ModuleRegistry::new(Os::current());
    modules
        .register(Box::new(notifications::NotificationsModule::new()))
        .unwrap();
    modules
        .set_config(&notifications::module_id(), &module_config(settings))
        .unwrap();
    let registry = modules.action_registry();
    let id = ActionTypeId::new(format!("{MODULE}.{}", scenario.action)).unwrap();
    let normalized = registry
        .normalize(&id, &parameters(&scenario.params))
        .unwrap();
    let profile = Profile::new("Parity");
    let machine = MachineConfig::new(Os::current());
    let notifier = AppNotifier {
        app_open: scenario.app_open,
        native_ok: !scenario.denied,
        shown: Mutex::new(Vec::new()),
    };
    let context = ExecutionContext::new(&profile, None, &machine).with_notifier(&notifier);
    let result = registry.dispatch(&id, &normalized, &context);
    let ok = result.status == ActionStatus::Succeeded;
    let outcome = Outcome {
        ok,
        message: result.message,
        data: if ok { result.data } else { BTreeMap::new() },
        code: result.code,
    };
    let shown = notifier.shown.lock().unwrap().clone();
    (outcome, shown)
}

// ------------------------------------------------------------ scenarios

struct Scenario {
    name: &'static str,
    action: &'static str,
    params: Value,
    /// The configuration app is open (its status bar shows them).
    app_open: bool,
    /// Native notifications are turned off on this desktop.
    denied: bool,
    /// The settings (plugin) and configuration (built-in), on top of the
    /// defaults.
    settings: Value,
}

fn scenario(name: &'static str, action: &'static str, params: Value) -> Scenario {
    Scenario {
        name,
        action,
        params,
        app_open: false,
        denied: false,
        settings: json!({}),
    }
}

impl Scenario {
    fn app_open(mut self) -> Self {
        self.app_open = true;
        self
    }

    fn denied(mut self) -> Self {
        self.denied = true;
        self
    }

    fn settings(mut self, settings: Value) -> Self {
        self.settings = settings;
        self
    }
}

const DEFAULTS: fn() -> Value =
    || json!({"desktop": true, "statusBar": true, "minimumSeverity": "info"});

fn with_defaults(settings: &Value) -> Value {
    let mut all = DEFAULTS();
    for (key, value) in settings.as_object().unwrap() {
        all[key] = value.clone();
    }
    all
}

/// The scenarios, in batches of at most [`NOTIFY_RATE`]: each batch runs in
/// a fresh agent, so the plugin's rate never refuses one of them.
fn batches() -> Vec<Vec<Scenario>> {
    // As long as the built-in takes: 100 and 500 characters.
    let long_title: String = "Release ".repeat(13).chars().take(100).collect();
    let long_body: String = "All checks passed on main. "
        .repeat(19)
        .chars()
        .take(500)
        .collect();
    vec![
        vec![
            scenario(
                "show, on the desktop",
                "show",
                json!({"title": "Deploy done", "body": "main is green", "severity": "success"}),
            ),
            scenario(
                "show, markup and control characters",
                "show",
                json!({"title": "Build\u{7} <b>failed</b>",
                       "body": "line one\nline\ttwo\u{202e}", "severity": "error"}),
            ),
            scenario(
                "show, long text",
                "show",
                json!({"title": long_title, "body": long_body, "severity": "warning"}),
            ),
            scenario(
                "summary outside a workflow",
                "workflow-summary",
                json!({"details": "Branch: main"}),
            ),
            scenario(
                "summary only on failure, outside a workflow",
                "workflow-summary",
                json!({"title": "Nightly", "onlyOnFailure": true}),
            ),
        ],
        vec![
            scenario("show, nowhere", "show", json!({"title": "Hello"})).denied(),
            scenario(
                "show, nowhere, critical",
                "show",
                json!({"title": "Hello", "critical": true}),
            )
            .denied(),
            scenario(
                "summary, nowhere, critical",
                "workflow-summary",
                json!({"critical": true}),
            )
            .denied(),
            scenario(
                "show, desktop off and the app closed",
                "show",
                json!({"title": "Hello"}),
            )
            .settings(json!({"desktop": false})),
        ],
        vec![
            scenario(
                "show, the app open",
                "show",
                json!({"title": "Tests passed", "severity": "success"}),
            )
            .app_open(),
            scenario(
                "show, status bar only",
                "show",
                json!({"title": "Tests passed"}),
            )
            .app_open()
            .settings(json!({"desktop": false})),
            scenario(
                "show, below the least severity",
                "show",
                json!({"title": "FYI"}),
            )
            .app_open()
            .settings(json!({"minimumSeverity": "warning"})),
            scenario(
                "show, at the least severity",
                "show",
                json!({"title": "Broken", "severity": "error"}),
            )
            .app_open()
            .settings(json!({"minimumSeverity": "warning"})),
            scenario("show, both off", "show", json!({"title": "Hello"}))
                .app_open()
                .settings(json!({"desktop": false, "statusBar": false})),
        ],
    ]
}

/// The native notification the plugin showed, as the built-in's would be:
/// without the plugin's name before the title.
fn unprefixed(mut shown: Notification) -> Notification {
    shown.title = shown
        .title
        .strip_prefix(PREFIX)
        .unwrap_or_else(|| panic!("the plugin's name is missing: {}", shown.title))
        .to_owned();
    shown
}

/// Every action of the plugin, in every condition, through the real agent
/// and host, answers as the built-in does: the same status, message,
/// outputs and error code, and the same native notification but for the
/// plugin's name before the title. The two reasons of
/// [`DIFFERENT_REASONS`] name the plugin's setting. Long text is cut
/// further (what `notify` takes); the node's message and outputs are the
/// built-in's. Past [`NOTIFY_RATE`] notifications a minute the plugin's
/// are dropped, which the built-in has no limit for.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // the steps of one comparison, in order
async fn the_plugin_answers_as_the_builtin_does() {
    let env = Env::new("plugins-notifications-parity");
    let mut problems = Vec::new();
    let mut compared = 0;
    for batch in batches() {
        assert!(batch.len() <= NOTIFY_RATE);
        let agent = env.start(true).await;
        agent
            .wait_migration(MigrationPhase::CutOver, MigrationRoute::Plugin)
            .await;
        for scenario in &batch {
            let settings = with_defaults(&scenario.settings);
            agent.set_settings(&settings).await;
            env.deny(scenario.denied);
            if scenario.app_open && agent.app.lock().unwrap().is_none() {
                agent.open_app().await;
            }
            let before = env.shown().len();
            let plugin = agent.invoke(scenario.action, &scenario.params).await;
            let shown: Vec<Notification> = env.shown()[before..]
                .iter()
                .cloned()
                .map(unprefixed)
                .collect();
            let (builtin, builtin_shown) = run_builtin(scenario, &settings);
            eprintln!(
                "{}: {}",
                scenario.name,
                if plugin.ok {
                    format!("ok, shown {}", plugin.data["shown"])
                } else {
                    format!("failed, {}", plugin.code.as_deref().unwrap_or("?"))
                }
            );
            compared += 1;
            let mut expected = builtin.clone();
            if let Some((_, reason, _)) =
                DIFFERENT_REASONS.iter().find(|(n, ..)| *n == scenario.name)
            {
                assert_eq!(
                    plugin.data.get("reason").map(String::as_str),
                    Some(*reason),
                    "{}",
                    scenario.name
                );
                let theirs = builtin.data["reason"].clone();
                expected.message = expected.message.replace(&theirs, reason);
                expected.data.insert("reason".into(), (*reason).to_owned());
            }
            if expected != plugin {
                problems.push(format!(
                    "{}:\n  built-in: {builtin:?}\n  plugin:   {plugin:?}",
                    scenario.name
                ));
            }
            if scenario.name == "show, long text" {
                // `notify` takes 64 and 300 characters: the plugin cuts
                // them as the core does, with an ellipsis.
                let [shown] = shown.as_slice() else {
                    panic!("{shown:?}")
                };
                let [builtin] = builtin_shown.as_slice() else {
                    panic!("{builtin_shown:?}")
                };
                assert_eq!(shown.title.chars().count(), 64, "{}", shown.title);
                assert_eq!(shown.body.chars().count(), 300);
                assert!(builtin.title.starts_with(shown.title.trim_end_matches('…')));
                assert!(builtin.body.starts_with(shown.body.trim_end_matches('…')));
                assert_eq!(shown.severity, builtin.severity);
            } else if shown != builtin_shown {
                problems.push(format!(
                    "{} (native):\n  built-in: {builtin_shown:?}\n  plugin:   {shown:?}",
                    scenario.name
                ));
            }
            if plugin.data.get("statusBar").map(String::as_str) == Some("true") {
                let status = unprefixed(agent.status_bar().await);
                assert_eq!(
                    status.title,
                    notifications_title(&scenario.params),
                    "{}",
                    scenario.name
                );
            }
        }
        if batch.len() == NOTIFY_RATE {
            // One more in the same minute: the plugin's rate drops it, and
            // the node goes on (not critical). The built-in has no limit.
            env.deny(false);
            agent.set_settings(&DEFAULTS()).await;
            let dropped = agent
                .invoke("show", &json!({"title": "One too many"}))
                .await;
            assert!(dropped.ok, "{dropped:?}");
            assert_eq!(dropped.data["shown"], "false");
            assert!(
                dropped.data["reason"].contains("more than 5 notifications a minute"),
                "{dropped:?}"
            );
        }
        agent.stop().await;
    }
    assert_eq!(compared, batches().iter().map(Vec::len).sum::<usize>());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The title a `show` scenario's notification has.
fn notifications_title(params: &Value) -> String {
    params["title"].as_str().unwrap_or("Workflow").to_owned()
}

// ------------------------------------------------- first launch, fresh

fn instance(name: &str, action_type: &str, params: &Value) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: name.into(),
        action_type: ActionTypeId::new(action_type).unwrap(),
        parameters: parameters(params),
        description: String::new(),
        icon: None,
        label: None,
    }
}

/// A fresh install, offline (no registry): at the first start the agent
/// seeds `deckpilot.notifications` from the application bundle, enabled,
/// with its one permission (`notifications`, low tier) granted by the
/// bundle and nothing else, and cuts the module over (nothing to migrate,
/// so no settings, no consent). Its actions run at once, and so do the
/// legacy ids, through the alias.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one first launch, step by step
async fn a_fresh_install_gets_the_plugin_on_first_launch() {
    let env = Env::new("plugins-notifications-fresh");
    assert_eq!(env.legacy(), (None, Vec::new()), "a fresh database");
    let agent = env.start(true).await;

    // Seeded, enabled, bundled.
    let summary = agent
        .plugin()
        .await
        .unwrap_or_else(|| panic!("not seeded\n{}", agent.log()));
    assert_eq!(summary.state, PluginState::Enabled, "{}", agent.log());
    assert_eq!(summary.origin, PluginOrigin::Bundled);
    assert_eq!(summary.version, "0.1.0");
    assert_eq!(
        summary
            .actions
            .iter()
            .map(|a| (a.action.as_str(), a.available))
            .collect::<Vec<_>>(),
        [("show", true), ("workflow-summary", true)]
    );
    // Its only permission, low tier, granted by the bundle; nothing else.
    let details = agent.details().await;
    assert_eq!(
        details
            .permissions
            .iter()
            .map(|p| (
                p.key.as_str(),
                format!("{:?}", p.tier),
                format!("{:?}", p.status)
            ))
            .collect::<Vec<_>>(),
        [("notifications", "Low".to_owned(), "Granted".to_owned())]
    );
    let grants = env
        .plugin_store()
        .grants(&PluginId::parse(ID).unwrap())
        .unwrap();
    assert_eq!(grants.len(), 1, "{grants:?}");
    assert_eq!(grants[0].status, GrantStatus::Granted);
    assert!(
        grants[0].decided_by.starts_with("bundled:"),
        "{}",
        grants[0].decided_by
    );

    // Cut over at once: nothing to migrate.
    let migration = agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::Plugin)
        .await;
    assert_eq!(migration.plugin, ID);
    assert_eq!(migration.consent.len(), 0, "{migration:?}");
    assert_eq!(
        migration
            .aliases
            .iter()
            .map(|a| (a.legacy.as_str(), a.target.as_str()))
            .collect::<Vec<_>>(),
        [
            ("notifications.show", "deckpilot.notifications.show"),
            (
                "notifications.workflow-summary",
                "deckpilot.notifications.workflow-summary"
            ),
        ]
    );
    // The defaults, nothing copied (no `modules` row).
    let settings = env
        .plugin_store()
        .settings(&PluginId::parse(ID).unwrap())
        .unwrap();
    assert!(settings.is_empty(), "{settings:?}");

    // Its actions run: on the desktop (the app is closed).
    let shown = agent
        .invoke(
            "show",
            &json!({"title": "Welcome", "body": "DeckPilot is ready", "severity": "success"}),
        )
        .await;
    assert!(shown.ok, "{shown:?}\n{}", agent.log());
    assert_eq!(
        (
            shown.data["shown"].as_str(),
            shown.data["desktop"].as_str(),
            shown.data["statusBar"].as_str()
        ),
        ("true", "true", "false")
    );
    let native = env.shown();
    assert_eq!(native.len(), 1);
    assert_eq!(native[0].title, "Notifications: Welcome");
    assert_eq!(native[0].body, "DeckPilot is ready");
    assert_eq!(native[0].severity, Severity::Success);

    // The first profile the user makes: a canonical instance and a legacy
    // one (an older app, or an import), on a key and in a workflow.
    let mut profile = Profile::new("Work");
    let canonical = instance(
        "Announce",
        "deckpilot.notifications.show",
        &json!({"title": "Canonical"}),
    );
    let legacy = instance(
        "Legacy announce",
        "notifications.show",
        &json!({"title": "Legacy"}),
    );
    let summary_node = instance(
        "Summary",
        "notifications.workflow-summary",
        &json!({"details": "First run"}),
    );
    let (canonical_id, legacy_id, summary_id) = (canonical.id, legacy.id, summary_node.id);
    profile.actions.extend([canonical, legacy, summary_node]);
    let workflow = support::graph_workflow(
        "Start work",
        support::graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "announce", "type": "action", "action": canonical_id},
                {"id": "summary", "type": "action", "action": summary_id},
            ]),
            &json!([{"from": "start", "to": "announce"}, {"from": "announce", "to": "summary"}]),
        ),
    );
    let workflow_id = workflow.id;
    profile.workflows.push(workflow);
    {
        let store = env.store();
        store.save_profile(&profile).unwrap();
        store.set_active_profile(Some(profile.id)).unwrap();
    }
    agent.ok(Request::ReloadConfiguration).await;
    let ran = agent.run_on_plugin(&env, canonical_id).await;
    assert_eq!(ran.status, ActionStatus::Succeeded, "{ran:?}");
    assert_eq!(ran.message, "showed “Canonical”");
    let ran = agent.run_on_plugin(&env, legacy_id).await;
    assert_eq!(ran.status, ActionStatus::Succeeded, "{ran:?}");
    assert_eq!(env.shown().last().unwrap().title, "Notifications: Legacy");

    // A workflow: the summary of its own run, from `run.context`.
    let record = agent.run_workflow(workflow_id).await;
    assert_eq!(
        record.summary.state,
        ExecutionState::Succeeded,
        "{record:#?}"
    );
    let node = record
        .nodes
        .iter()
        .find(|n| n.node.as_str() == "summary")
        .unwrap();
    assert_eq!(node.output["shown"], "true", "{node:?}");
    assert!(
        node.output["summary"].starts_with("1 succeeded · "),
        "{node:?}"
    );
    let last = env.shown().last().unwrap().clone();
    assert_eq!(last.title, "Notifications: Start work");
    assert!(last.body.ends_with("\nFirst run"), "{}", last.body);
    assert_eq!(last.severity, Severity::Success);

    // Nothing was written to deckpilot.db but the profile the user saved.
    let (module, profiles) = env.legacy();
    assert_eq!(module, None);
    assert_eq!(profiles.len(), 1);
    agent.stop().await;
}

// ---------------------------------------------------------- tombstones

/// The user's choices stick (§4.2): disabling the seeded plugin writes a
/// tombstone, and neither a restart nor an app update that bundles a newer
/// version re-enables it; the legacy actions are unavailable, with no
/// fallback to the built-in. Enabling it again is the user's to do, and
/// clears the tombstone. Uninstalling it is final in the same way: it is
/// never seeded again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one sequence of the user's choices
async fn removing_or_disabling_the_plugin_sticks_across_restarts_and_upgrades() {
    let env = Env::new("plugins-notifications-tombstones");
    let mut profile = Profile::new("Work");
    let legacy = instance(
        "Legacy announce",
        "notifications.show",
        &json!({"title": "Legacy"}),
    );
    let legacy_id = legacy.id;
    profile.actions.push(legacy);
    {
        let store = env.store();
        store.save_profile(&profile).unwrap();
        store.set_active_profile(Some(profile.id)).unwrap();
    }
    let plugin_id = PluginId::parse(ID).unwrap();

    // 1. Seeded and cut over. The user disables it and enables it again:
    //    the tombstone comes and goes with the user's choice.
    let agent = env.start(true).await;
    agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::Plugin)
        .await;
    agent.run_on_plugin(&env, legacy_id).await;
    agent.ok(Request::DisablePlugin { plugin: ID.into() }).await;
    assert_eq!(
        env.plugin_store()
            .tombstone(&plugin_id)
            .unwrap()
            .map(|t| t.decision),
        Some(TombstoneDecision::Disabled)
    );
    agent
        .ok(Request::EnablePlugin {
            plugin: ID.into(),
            consent: vec!["notifications".into()],
        })
        .await;
    assert_eq!(env.plugin_store().tombstone(&plugin_id).unwrap(), None);
    agent.run_on_plugin(&env, legacy_id).await;

    // 2. Disabled for good: withdrawn at once, and after a restart.
    agent.ok(Request::DisablePlugin { plugin: ID.into() }).await;
    let why = agent.wait_unavailable(legacy_id).await;
    assert!(
        why.contains("`deckpilot.notifications`) is disabled"),
        "{why}"
    );
    agent.stop().await;
    let agent = env.start(true).await;
    assert_eq!(
        agent.plugin().await.map(|p| p.state),
        Some(PluginState::Disabled)
    );
    let migration = agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::PluginWithdrawn)
        .await;
    assert!(
        migration
            .route_reason
            .as_deref()
            .is_some_and(|r| r.contains("disabled by you")),
        "{migration:?}"
    );
    let shown = env.shown().len();
    let why = agent.wait_unavailable(legacy_id).await;
    assert!(
        why.contains("`deckpilot.notifications`) is disabled"),
        "{why}"
    );
    assert_eq!(env.shown().len(), shown, "no fallback to the built-in");
    agent.stop().await;

    // 3. An app update bundles 0.1.1: still disabled, still 0.1.0.
    env.stage(Some("0.1.1"));
    let agent = env.start(true).await;
    let summary = agent.plugin().await.unwrap();
    assert_eq!(
        (summary.state, summary.version.as_str()),
        (PluginState::Disabled, "0.1.0")
    );
    assert!(
        agent.log().contains("disabled by the user"),
        "seeding says why it skipped"
    );
    agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::PluginWithdrawn)
        .await;

    // 4. Uninstalled: never seeded again, by a restart or an update.
    agent
        .ok(Request::UninstallPlugin {
            plugin: ID.into(),
            keep: deckpilot_ipc::plugins::PluginRetention::default(),
        })
        .await;
    assert_eq!(
        env.plugin_store()
            .tombstone(&plugin_id)
            .unwrap()
            .map(|t| t.decision),
        Some(TombstoneDecision::Uninstalled)
    );
    agent.stop().await;
    for _ in 0..2 {
        let agent = env.start(true).await;
        assert!(agent.plugin().await.is_none(), "{}", agent.log());
        assert_eq!(
            env.plugin_store()
                .tombstone(&plugin_id)
                .unwrap()
                .map(|t| t.decision),
            Some(TombstoneDecision::Uninstalled)
        );
        let why = agent.wait_unavailable(legacy_id).await;
        assert!(why.contains("deckpilot.notifications"), "{why}");
        agent.stop().await;
    }
    assert_eq!(env.shown().len(), shown, "nothing was shown since");
}

/// The user's Restore (`restoreBundledPlugin`, protocol 15) undoes their
/// uninstall: the tombstone goes, the pinned copy comes back from the
/// bundle with its kept settings, the legacy actions run on the plugin
/// again, and a restart keeps it. Its broken copy is reinstalled the same
/// way; a healthy one and an id DeckPilot does not bundle are refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one sequence of the user's choices
async fn a_removed_or_broken_plugin_is_restored_from_the_bundle() {
    let env = Env::new("plugins-notifications-restore");
    let mut profile = Profile::new("Work");
    let legacy = instance(
        "Legacy announce",
        "notifications.show",
        &json!({"title": "Legacy"}),
    );
    let legacy_id = legacy.id;
    profile.actions.push(legacy);
    {
        let store = env.store();
        store.save_profile(&profile).unwrap();
        store.set_active_profile(Some(profile.id)).unwrap();
    }
    let plugin_id = PluginId::parse(ID).unwrap();
    let agent = env.start(true).await;
    agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::Plugin)
        .await;
    agent.set_settings(&json!({"statusBar": false})).await;
    agent
        .ok(Request::UninstallPlugin {
            plugin: ID.into(),
            keep: deckpilot_ipc::plugins::PluginRetention {
                settings: true,
                credentials: false,
            },
        })
        .await;
    agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::PluginWithdrawn)
        .await;
    let Response::KnownPlugins { plugins } = agent.ok(Request::ListKnownPlugins).await else {
        panic!("known plugins")
    };
    let known = plugins.iter().find(|k| k.id == ID).unwrap();
    assert!(!known.installed && known.restorable, "{known:?}");
    let Response::PluginRuntime { status } = agent.ok(Request::GetPluginRuntime).await else {
        panic!("runtime")
    };
    assert!(status.has_feature(deckpilot_ipc::plugins::feature::RESTORE_BUNDLED));

    // Restore, watched by an app subscribed to plugin events.
    let mut app = AgentClient::connect(&env.paths, ClientKind::Desktop)
        .await
        .unwrap();
    app.subscribe_plugin_events().await.unwrap();
    let restored = agent
        .ok(Request::RestoreBundledPlugin { plugin: ID.into() })
        .await;
    let Response::Plugin { plugin } = restored else {
        panic!("{restored:?}")
    };
    assert_eq!(plugin.summary.state, PluginState::Enabled);
    assert_eq!(plugin.summary.origin, PluginOrigin::Bundled);
    assert_eq!(plugin.summary.version, "0.1.0");
    assert_eq!(env.plugin_store().tombstone(&plugin_id).unwrap(), None);
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + SETTLE;
    while !seen.iter().any(|e| {
        matches!(e, deckpilot_ipc::plugins::PluginEvent::Changed { plugin, state }
            if plugin == ID && *state == Some(PluginState::Enabled))
    }) {
        let event = tokio::time::timeout_at(deadline, app.next_plugin_event())
            .await
            .expect("plugin events in time")
            .unwrap();
        seen.push(event);
    }
    assert!(
        seen.iter().any(|e| matches!(e,
            deckpilot_ipc::plugins::PluginEvent::Progress { plugin, stage, .. }
                if plugin == ID && *stage == deckpilot_ipc::plugins::PluginStage::Finished)),
        "{seen:?}"
    );
    drop(app);
    let Response::PluginSettings { settings } = agent
        .ok(Request::GetPluginSettings { plugin: ID.into() })
        .await
    else {
        panic!("settings")
    };
    assert_eq!(settings.values["statusBar"], json!(false), "kept settings");
    agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::Plugin)
        .await;
    agent.run_on_plugin(&env, legacy_id).await;
    let greeting = agent.invoke("show", &json!({"title": "Back again"})).await;
    assert!(greeting.ok, "{greeting:?}");

    // Installed and healthy: nothing to restore.
    let refused = agent
        .request(Request::RestoreBundledPlugin { plugin: ID.into() })
        .await;
    assert!(
        matches!(&refused, Err(ClientError::Agent(e))
            if e.reason.as_deref() == Some(reason::ALREADY_INSTALLED)),
        "{refused:?}"
    );
    let refused = agent
        .request(Request::RestoreBundledPlugin {
            plugin: "acme.tool".into(),
        })
        .await;
    assert!(
        matches!(&refused, Err(ClientError::Agent(e))
            if e.reason.as_deref() == Some(reason::NOT_BUNDLED)),
        "{refused:?}"
    );
    agent.stop().await;

    // A restart finds the pinned copy: still enabled, still on the plugin.
    let agent = env.start(true).await;
    assert_eq!(
        agent.plugin().await.map(|p| p.state),
        Some(PluginState::Enabled)
    );
    agent.run_on_plugin(&env, legacy_id).await;
    agent.stop().await;

    // Its installed copy changes while the agent is stopped: broken at the
    // next start, then reinstalled from the bundle.
    let wasm = env
        .paths
        .data_dir
        .join("plugins")
        .join(ID)
        .join("0.1.0")
        .join("plugin.wasm");
    let mut permissions = std::fs::metadata(&wasm).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&wasm, permissions).unwrap();
    let mut bytes = std::fs::read(&wasm).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&wasm, bytes).unwrap();
    let agent = env.start(true).await;
    assert_eq!(
        agent.plugin().await.map(|p| p.state),
        Some(PluginState::Broken),
        "{}",
        agent.log()
    );
    let Response::Plugin { plugin } = agent
        .ok(Request::RestoreBundledPlugin { plugin: ID.into() })
        .await
    else {
        panic!("reinstalled")
    };
    assert_eq!(plugin.summary.state, PluginState::Enabled);
    assert!(
        plugin
            .permissions
            .iter()
            .all(|p| p.status == deckpilot_ipc::plugins::PermissionStatus::Granted)
    );
    assert_eq!(env.plugin_store().tombstone(&plugin_id).unwrap(), None);
    agent.run_on_plugin(&env, legacy_id).await;
    agent.stop().await;
}

// ------------------------------------------------------- alpha.3 upgrade

/// The data an alpha.3 app saved: the module's settings (desktop off, from
/// warning on), a key's action and a workflow node with the legacy ids.
struct Alpha3 {
    deploy: ActionInstanceId,
    fyi: ActionInstanceId,
    workflow: WorkflowId,
}

fn write_alpha3(env: &Env) -> Alpha3 {
    let store = env.store();
    store
        .set_module_state(
            &ModuleId::new(MODULE).unwrap(),
            &ModuleSettings {
                enabled: true,
                config: ActionParameters::new()
                    .with(notifications::DESKTOP, ParameterValue::Boolean(false))
                    .with(notifications::STATUS_BAR, ParameterValue::Boolean(true))
                    .with(
                        notifications::MINIMUM_SEVERITY,
                        ParameterValue::Choice("warning".into()),
                    ),
            },
        )
        .unwrap();
    let mut profile = Profile::new("Work");
    let deploy = instance(
        "Deploy failed",
        "notifications.show",
        &json!({"title": "Deploy failed", "body": "main is red", "severity": "error"}),
    );
    let fyi = instance("FYI", "notifications.show", &json!({"title": "FYI"}));
    let summary = instance(
        "Summary",
        "notifications.workflow-summary",
        &json!({"title": "Nightly", "details": "Branch: main"}),
    );
    let warn = instance(
        "Careful",
        "notifications.show",
        &json!({"title": "Careful", "severity": "warning"}),
    );
    let ids = Alpha3 {
        deploy: deploy.id,
        fyi: fyi.id,
        workflow: WorkflowId::new(),
    };
    let workflow = support::graph_workflow(
        "Nightly",
        support::graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "careful", "type": "action", "action": warn.id},
                {"id": "summary", "type": "action", "action": summary.id},
            ]),
            &json!([{"from": "start", "to": "careful"}, {"from": "careful", "to": "summary"}]),
        ),
    );
    let mut workflow = workflow;
    workflow.id = ids.workflow;
    profile.actions.extend([deploy, fyi, summary, warn]);
    profile.workflows.push(workflow);
    store.save_profile(&profile).unwrap();
    store.set_active_profile(Some(profile.id)).unwrap();
    ids
}

/// What a node or an action answered, without the run's duration.
fn timeless(text: &str) -> String {
    match text.rsplit_once(" · ") {
        Some((counts, _)) => format!("{counts} · <duration>"),
        None => text.to_owned(),
    }
}

fn node_outputs(record: &ExecutionRecord) -> BTreeMap<String, BTreeMap<String, String>> {
    record
        .nodes
        .iter()
        .map(|n| {
            (
                n.node.as_str().to_owned(),
                n.output
                    .iter()
                    .map(|(k, v)| (k.clone(), timeless(v)))
                    .collect(),
            )
        })
        .collect()
}

/// An alpha.3 user upgrades to a build with plugins: the bundled plugin is
/// seeded, the module's settings copied, and the module cut over with no
/// consent (one low-tier permission). The key's legacy action and the
/// workflow nodes run in the plugin, honouring the copied settings
/// (desktop off, from warning on). `deckpilot.db` is unchanged, so an
/// older build (plugins off) on the same data runs the built-in, with the
/// same results but for the plugin's name and reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one upgrade, step by step
async fn an_alpha3_user_keeps_notifications_through_the_alias() {
    let env = Env::new("plugins-notifications-alpha3");
    let alpha3 = write_alpha3(&env);
    let before = env.legacy();

    // The older build first: the built-in, on the module's settings.
    let older = env.start(false).await;
    older.open_app().await;
    let builtin_deploy = older.run_action(alpha3.deploy).await;
    let status = older.status_bar().await;
    assert_eq!(status.title, "Deploy failed", "the built-in has no prefix");
    let builtin_fyi = older.run_action(alpha3.fyi).await;
    let builtin_run = older.run_workflow(alpha3.workflow).await;
    assert!(env.shown().is_empty(), "desktop notifications are off");
    older.stop().await;
    let plugins_db = deckpilot_plugin_store::Layout::new(&env.paths.data_dir).database();
    assert!(
        !plugins_db.exists() || env.plugin_store().module_migrations().unwrap().is_empty(),
        "no plugins, no migration"
    );

    // The upgrade.
    let agent = env.start(true).await;
    let migration = agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::Plugin)
        .await;
    assert_eq!(migration.consent.len(), 0, "{migration:?}");
    let settings = env
        .plugin_store()
        .settings(&PluginId::parse(ID).unwrap())
        .unwrap();
    assert_eq!(
        serde_json::to_value(&settings).unwrap(),
        json!({"desktop": false, "statusBar": true, "minimumSeverity": "warning"}),
        "the module's settings, copied"
    );
    assert_eq!(
        agent.plugin().await.map(|p| p.state),
        Some(PluginState::Enabled)
    );
    agent.open_app().await;
    // The key's action on the plugin: the status bar only (desktop off).
    let deadline = Instant::now() + SETTLE;
    let (deploy, status) = loop {
        let result = agent.run_action(alpha3.deploy).await;
        let status = agent.status_bar().await;
        if status.title.starts_with(PREFIX) {
            break (result, status);
        }
        assert!(
            Instant::now() < deadline,
            "not on the plugin\n{}",
            agent.log()
        );
    };
    assert_eq!(status.title, "Notifications: Deploy failed");
    assert_eq!(status.body, "main is red");
    assert_eq!(status.severity, Severity::Error);
    assert_eq!(
        (deploy.status, &deploy.message, &deploy.data),
        (
            builtin_deploy.status,
            &builtin_deploy.message,
            &builtin_deploy.data
        )
    );
    // Below the copied least severity: not shown, the node goes on.
    let fyi = agent.run_action(alpha3.fyi).await;
    assert_eq!(fyi.status, ActionStatus::Succeeded);
    assert_eq!(fyi.data["shown"], "false");
    assert_eq!(
        fyi.data["reason"], "less important than the plugin's setting",
        "{fyi:?}"
    );
    assert_eq!(
        builtin_fyi.data["reason"],
        "less important than the module's setting"
    );
    // The workflow's nodes: the same outputs but for the run's duration
    // and that reason (the summary of a clean run is a success, below
    // warning).
    let run = agent.run_workflow(alpha3.workflow).await;
    assert_eq!(run.summary.state, ExecutionState::Succeeded, "{run:#?}");
    let mut expected = node_outputs(&builtin_run);
    let plugin_nodes = node_outputs(&run);
    if let Some(summary) = expected.get_mut("summary") {
        summary.insert(
            "reason".into(),
            "less important than the plugin's setting".into(),
        );
    }
    assert_eq!(plugin_nodes, expected);
    assert_eq!(plugin_nodes["careful"]["statusBar"], "true");
    assert_eq!(
        plugin_nodes["summary"]["summary"],
        "1 succeeded · <duration>"
    );
    assert!(
        env.shown().is_empty(),
        "desktop notifications are still off"
    );
    agent.stop().await;

    // Nothing an older app reads changed, byte for byte.
    assert_eq!(env.legacy(), before);

    // An older build on the same data again: the built-in.
    let older = env.start(false).await;
    older.open_app().await;
    let again = older.run_action(alpha3.deploy).await;
    assert_eq!(older.status_bar().await.title, "Deploy failed");
    assert_eq!(again.data, builtin_deploy.data);
    older.stop().await;

    // And the plugin build once more: still cut over, nothing redone.
    let agent = env.start(true).await;
    let again = agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::Plugin)
        .await;
    assert_eq!(again.attempts, 0);
    agent.stop().await;
    assert_eq!(env.legacy(), before);
}

// ------------------------------------------------------- 0.6.0 upgrade

/// `deckpilot.db` (and its write-ahead log, when there is one), byte for
/// byte.
fn database_files(paths: &AgentPaths) -> BTreeMap<String, Vec<u8>> {
    let name = paths.database.file_name().unwrap().to_string_lossy();
    ["", "-wal"]
        .into_iter()
        .filter_map(|suffix| {
            let file = paths.data_dir.join(format!("{name}{suffix}"));
            std::fs::read(&file)
                .ok()
                .map(|bytes| (format!("{name}{suffix}"), bytes))
        })
        .collect()
}

/// A 0.6.0 user (an encrypted `deckpilot.db`, no `plugins.db`) updates to
/// a build with the `plugins` feature, started as the configuration app
/// and the login item start it: with no plugin switch at all. Locked, the
/// agent says plugins are on and waits for the key, creating nothing;
/// unlocked, it creates `plugins.db`, seeds the bundled default plugin and
/// cuts its module over. `deckpilot.db` is not written, byte for byte, so
/// 0.6.0 on the same data still works; `--disable-plugins` turns plugins
/// off again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_0_6_0_data_dir_starts_with_plugins_on_without_a_switch() {
    let env = Env::from_v0_6_0("plugins-notifications-v060");
    let plugins_dir = env.paths.data_dir.join("plugins");

    // 0.6.0 itself (here: plugins off) brings the database to this
    // build's schema, as the 0.6.0 release did; plugins create nothing.
    let older = env.start(false).await;
    let Response::PluginRuntime { status } = older.ok(Request::GetPluginRuntime).await else {
        panic!("status")
    };
    assert!(
        status.built && !status.enabled && !status.available,
        "{status:?}"
    );
    older.stop().await;
    assert!(!plugins_dir.exists(), "plugins off creates nothing");
    let before = database_files(&env.paths);
    assert!(before.contains_key("deckpilot.db"), "{:?}", before.keys());

    // The update, locked: plugins are on, waiting for the key.
    let agent = env.spawn(true);
    let Response::PluginRuntime { status } = agent.ok(Request::GetPluginRuntime).await else {
        panic!("status")
    };
    assert!(status.built && status.enabled, "{status:?}");
    assert!(!status.available, "locked: {status:?}");
    let locked = agent.request(Request::ListPlugins).await;
    assert!(
        matches!(&locked, Err(ClientError::Agent(e)) if e.reason.as_deref() == Some(reason::LOCKED)),
        "{locked:?}"
    );
    assert!(
        !plugins_dir.join("plugins.db").exists(),
        "nothing is created without the key"
    );

    // Unlocked: plugins.db, the bundled default plugin, the cut-over.
    agent
        .request(Request::Unlock {
            key: KeyMaterial::new(env.key.to_hex()),
        })
        .await
        .unwrap();
    let deadline = Instant::now() + SETTLE;
    let status = loop {
        let Response::PluginRuntime { status } = agent.ok(Request::GetPluginRuntime).await else {
            panic!("status")
        };
        if status.available || Instant::now() >= deadline {
            break status;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(
        status.available && status.enabled,
        "{status:?}\n{}",
        agent.log()
    );
    let migration = agent
        .wait_migration(MigrationPhase::CutOver, MigrationRoute::Plugin)
        .await;
    assert_eq!(migration.plugin, ID);
    let summary = agent
        .plugin()
        .await
        .unwrap_or_else(|| panic!("not seeded\n{}", agent.log()));
    assert_eq!(summary.state, PluginState::Enabled);
    assert_eq!(summary.origin, PluginOrigin::Bundled);
    agent.stop().await;
    assert!(plugins_dir.join("plugins.db").is_file());
    // Recorded in plugins.db (with the other modules that can migrate,
    // whose plugins this bundle lacks).
    let recorded = env.plugin_store().module_migrations().unwrap();
    assert!(
        recorded
            .iter()
            .any(|m| m.module_id == MODULE
                && m.phase == deckpilot_plugin_store::MigrationPhase::CutOver),
        "{recorded:?}"
    );

    // deckpilot.db was not written.
    assert_eq!(database_files(&env.paths), before, "deckpilot.db changed");

    // Turned off explicitly on the same data: nothing answers, nothing
    // changes.
    let off = env.start(false).await;
    let Response::PluginRuntime { status } = off.ok(Request::GetPluginRuntime).await else {
        panic!("status")
    };
    assert!(status.built && !status.enabled && !status.available);
    let refused = off.request(Request::ListPlugins).await;
    assert!(
        matches!(&refused, Err(ClientError::Agent(e))
            if e.reason.as_deref() == Some(reason::PLUGINS_UNAVAILABLE)),
        "{refused:?}"
    );
    off.stop().await;
    assert_eq!(database_files(&env.paths), before, "deckpilot.db changed");
}
