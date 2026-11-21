//! The harness of the plugin API 1.9 first-party plugins' end-to-end tests
//! (`plugins_workspace.rs`, `plugins_projects.rs`, `plugins_workflows.rs`):
//! the plugin built to wasm with the `guest` profile and bundled as
//! `plugins/first-party/build.sh` stages it (its locales included), pinned
//! with `DECKPILOT_TEST_BUNDLED_PINS`; the real agent and the real
//! sandboxed host, each test in its own data directory, with a profile and
//! this computer's resource bindings seeded; every program the agent would
//! start recorded (`DECKPILOT_TEST_PLUGIN_LAUNCH`, feature `test-hooks`)
//! and nothing started; no registry, everything offline.
#![allow(dead_code)] // Each test uses part of it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionResult, ActionTypeId,
    ExecutionRecord, ExecutionState, MachineConfig, ParameterValue, Profile, WorkflowId,
};
use deckpilot_ipc::plugins::{PluginActionOutcome, PluginDetails, PluginState, PluginSummary};
use deckpilot_ipc::{AgentClient, AgentPaths, ClientKind, KeyMaterial, Request, Response};
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_storage::{ConfigStore, DatabaseKey};
use serde_json::{Value, json};

use crate::support::{self, TempDir};

pub const STARTUP: Duration = Duration::from_secs(30);
pub const SETTLE: Duration = Duration::from_secs(60);

/// One first-party plugin of `plugins/first-party`.
#[derive(Debug, Clone, Copy)]
pub struct FirstParty {
    pub id: &'static str,
    /// Its directory under `plugins/first-party` (and in `bundled.json`).
    pub dir: &'static str,
    /// Its crate, built to `<crate with underscores>.wasm`.
    pub krate: &'static str,
    /// The class `bundled.json` gives it.
    pub released: &'static str,
}

impl FirstParty {
    pub fn source(&self) -> PathBuf {
        repo().join("plugins/first-party").join(self.dir)
    }

    pub fn manifest(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.source().join("manifest.json")).unwrap())
            .unwrap()
    }

    /// The release bundles it with the class this test expects.
    pub fn assert_released(&self) {
        let released: Value = serde_json::from_slice(
            &std::fs::read(repo().join("plugins/first-party/bundled.json")).unwrap(),
        )
        .unwrap();
        assert!(
            released["plugins"]
                .as_array()
                .unwrap()
                .contains(&json!({"path": self.dir, "class": self.released})),
            "the release bundles {} as a {}-class plugin: {released}",
            self.id,
            self.released
        );
    }
}

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime() -> PathBuf {
    repo().join("plugins/runtime")
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

pub struct Artifacts {
    pub host: PathBuf,
    pub module: PathBuf,
}

/// The host and the plugin's module, built from `plugins/runtime` (or
/// taken from `DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR`).
pub fn artifacts(plugin: FirstParty) -> &'static Artifacts {
    static ARTIFACTS: Mutex<BTreeMap<&'static str, &'static Artifacts>> =
        Mutex::new(BTreeMap::new());
    let mut built = ARTIFACTS.lock().unwrap_or_else(PoisonError::into_inner);
    built.entry(plugin.krate).or_insert_with(|| {
        let file = format!("{}.wasm", plugin.krate.replace('-', "_"));
        let wasm_dir = std::env::var_os("DECKPILOT_E2E_WASM_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.join(&file).is_file())
            .unwrap_or_else(|| {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    plugin.krate,
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
        Box::leak(Box::new(Artifacts {
            host,
            module: wasm_dir.join(file),
        }))
    })
}

// ---------------------------------------------------------- environment

/// One data directory, the profile and bindings it starts with, and the
/// application bundle the agent starts from.
pub struct Env {
    pub dir: TempDir,
    pub paths: AgentPaths,
    pub key: DatabaseKey,
    pub plugin: FirstParty,
    /// More of the agent's environment (`plugins_git.rs` puts a fake
    /// `git` first on its `PATH`).
    #[allow(clippy::struct_field_names)] // `env.extra_env`, as it reads.
    pub extra_env: Vec<(String, std::ffi::OsString)>,
    /// Bundled next to the plugin, as `default`-class plugins
    /// ([`Env::with_projects`]).
    pub companions: Vec<FirstParty>,
}

/// The Projects plugin: the only project context provider since the
/// built-in `projects` module is gone (WP-C6). Plugins that read the active
/// project's roles (git, docker, workspace) need it enabled.
pub const PROJECTS: FirstParty = FirstParty {
    id: "deckpilot.projects",
    dir: "projects",
    krate: "deckpilot-projects",
    released: "migration",
};

impl Env {
    /// A fresh install with `profile` active (and `project` its active
    /// project) and `machine` this computer's bindings. The plugin is
    /// pinned with class `class` (a `migration`-class plugin is staged as
    /// `default` so that the agent seeds it without a migration).
    pub fn new(
        name: &str,
        plugin: FirstParty,
        class: BundleClass,
        profile: Option<(&Profile, Option<deckpilot_core::ProjectId>)>,
        machine: Option<&MachineConfig>,
    ) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let store = support::open_store(&paths, &key);
        if let Some((profile, project)) = profile {
            store.save_profile(profile).unwrap();
            store.set_active_profile(Some(profile.id)).unwrap();
            store.set_active_project(profile.id, project).unwrap();
        }
        if let Some(machine) = machine {
            store.save_machine_config(machine).unwrap();
        }
        drop(store);
        let env = Self {
            dir,
            paths,
            key,
            plugin,
            extra_env: Vec::new(),
            companions: Vec::new(),
        };
        env.stage(class);
        env
    }

    /// The same with the Projects plugin bundled too (class `default`, so
    /// it is seeded; [`Env::start`] enables it with the user's consent), as
    /// an installed app bundles it: the active project's roles resolve only
    /// with it.
    #[must_use]
    pub fn with_projects(mut self, class: BundleClass) -> Self {
        self.companions.push(PROJECTS);
        self.stage(class);
        self
    }

    pub fn bundled(&self) -> PathBuf {
        self.dir.path().join("resources/bundled")
    }

    fn pins_file(&self) -> PathBuf {
        self.dir.path().join("compiled-pins.json")
    }

    /// Where the agent records the programs it would have started.
    pub fn launch_file(&self) -> PathBuf {
        self.dir.path().join("launched.jsonl")
    }

    /// Where the agent records the pages `url.open` would have opened.
    pub fn opened_file(&self) -> PathBuf {
        self.dir.path().join("opened.txt")
    }

    /// How many seconds the agent's clock runs ahead
    /// (`DECKPILOT_TEST_CLOCK_OFFSET`).
    pub fn clock_offset(&self) -> PathBuf {
        self.dir.path().join("clock-offset.txt")
    }

    /// The programs the agent would have started, in order:
    /// `(program, args)`.
    pub fn launched(&self) -> Vec<(String, Vec<String>)> {
        std::fs::read_to_string(self.launch_file())
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let plan: Value = serde_json::from_str(line).unwrap();
                (
                    plan["program"].as_str().unwrap().to_owned(),
                    plan["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap().to_owned())
                        .collect(),
                )
            })
            .collect()
    }

    /// Nothing reached a browser through `url.open`.
    pub fn assert_no_page_opened(&self) {
        assert_eq!(
            std::fs::read_to_string(self.opened_file()).unwrap_or_default(),
            ""
        );
    }

    /// `bundled/<id>/` as `build.sh` stages it: the manifest, the LICENSE,
    /// the icon, the locales and the module, and the pins the agent "was
    /// compiled with".
    fn stage(&self, class: BundleClass) {
        let mut pinned = Vec::new();
        for (plugin, class) in std::iter::once((self.plugin, class))
            .chain(self.companions.iter().map(|c| (*c, BundleClass::Default)))
        {
            pinned.push(self.stage_one(plugin, class));
        }
        let pins = Pins::compute(&self.bundled(), &pinned).unwrap();
        std::fs::write(self.bundled().join("pins.json"), pins.to_json_pretty()).unwrap();
        std::fs::write(self.pins_file(), pins.to_json_pretty()).unwrap();
    }

    /// `bundled/<id>/` of `plugin`, and its pin.
    fn stage_one(
        &self,
        plugin: FirstParty,
        class: BundleClass,
    ) -> (PluginId, semver::Version, BundleClass) {
        plugin.assert_released();
        let dir = self.bundled().join(plugin.id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::create_dir_all(dir.join("locales")).unwrap();
        let manifest = plugin.manifest();
        let mut files = vec!["manifest.json".to_owned(), "LICENSE".to_owned()];
        files.extend(
            manifest["icons"]
                .as_object()
                .unwrap()
                .values()
                .map(|v| v.as_str().unwrap().to_owned()),
        );
        files.extend(
            manifest["locales"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tag| format!("locales/{}.json", tag.as_str().unwrap())),
        );
        for file in files {
            std::fs::copy(plugin.source().join(&file), dir.join(&file)).unwrap();
        }
        std::fs::copy(&artifacts(plugin).module, dir.join("plugin.wasm")).unwrap();
        let version = semver::Version::parse(manifest["version"].as_str().unwrap()).unwrap();
        (PluginId::parse(plugin.id).unwrap(), version, class)
    }

    /// Starts the agent and unlocks it.
    pub async fn start(&self) -> Agent {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.path().join("agent.log"))
            .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"));
        command
            .args(["--headless", "--no-usb", "--log-level", "debug"])
            .arg("--plugin-host")
            .arg(&artifacts(self.plugin).host)
            .arg("--bundled-plugins")
            .arg(self.bundled())
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_BUNDLED_PINS", self.pins_file())
            .env("DECKPILOT_TEST_PLUGIN_LAUNCH", self.launch_file())
            .env("DECKPILOT_TEST_PLUGIN_OPEN_URL", self.opened_file())
            .env("DECKPILOT_TEST_CLOCK_OFFSET", self.clock_offset())
            .env_remove("DECKPILOT_TEST_LEGACY_FIXTURE")
            .env_remove("DECKPILOT_TEST_MIGRATION_DECISIONS")
            .env_remove("DECKPILOT_TEST_MIGRATION_EXIT")
            .env_remove("DECKPILOT_TEST_PLUGIN_CRASH")
            .env_remove("DECKPILOT_TEST_PLUGIN_HTTP")
            .env_remove("DECKPILOT_ENABLE_PLUGINS")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_URL")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_ROOTS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        for (name, value) in &self.extra_env {
            command.env(name, value);
        }
        let agent = Agent {
            child: command.spawn().expect("the agent starts"),
            paths: self.paths.clone(),
            log: self.dir.path().join("agent.log"),
            id: self.plugin.id,
        };
        agent
            .ok(Request::Unlock {
                key: KeyMaterial::new(self.key.to_hex()),
            })
            .await;
        for companion in &self.companions {
            agent.enable_plugin(companion.id).await;
        }
        agent
    }
}

// ---------------------------------------------------------------- agent

pub struct Agent {
    child: Child,
    paths: AgentPaths,
    log: PathBuf,
    id: &'static str,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What an action answered: status, message, outputs (as texts) and
/// error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub ok: bool,
    pub message: String,
    pub data: BTreeMap<String, String>,
    pub code: Option<String>,
}

impl From<ActionResult> for Outcome {
    fn from(result: ActionResult) -> Self {
        Self {
            ok: result.status == deckpilot_core::ActionStatus::Succeeded,
            message: result.message,
            data: result.data,
            code: result.code,
        }
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

impl Agent {
    pub async fn client(&self) -> AgentClient {
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

    pub async fn request(&self, request: Request) -> Result<Response, deckpilot_ipc::ClientError> {
        let what = request.name();
        tokio::time::timeout(Duration::from_secs(120), async {
            self.client().await.request(request).await
        })
        .await
        .unwrap_or_else(|_| panic!("{what} took too long"))
    }

    pub async fn ok(&self, request: Request) -> Response {
        let what = request.name();
        self.request(request)
            .await
            .unwrap_or_else(|e| panic!("{what}: {e}\n{}", self.log()))
    }

    /// A client subscribed to the agent's events.
    pub async fn events(&self) -> AgentClient {
        let mut client = self.client().await;
        client.subscribe().await.unwrap();
        client
    }

    pub async fn summary(&self) -> Option<PluginSummary> {
        match self.ok(Request::ListPlugins).await {
            Response::Plugins { plugins } => plugins.into_iter().find(|p| p.id == self.id),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The plugin once the agent seeded it.
    pub async fn seeded(&self) -> PluginSummary {
        let deadline = Instant::now() + SETTLE;
        loop {
            if let Some(summary) = self.summary().await {
                return summary;
            }
            assert!(Instant::now() < deadline, "not seeded\n{}", self.log());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn details(&self) -> PluginDetails {
        match self
            .ok(Request::GetPlugin {
                plugin: self.id.into(),
            })
            .await
        {
            Response::Plugin { plugin } => *plugin,
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Enables the plugin with the user's consent to every permission it
    /// declares; the grant keys.
    pub async fn enable(&self) -> Vec<String> {
        let consent: Vec<String> = self
            .details()
            .await
            .permissions
            .iter()
            .map(|p| p.key.clone())
            .collect();
        self.ok(Request::EnablePlugin {
            plugin: self.id.into(),
            consent: consent.clone(),
        })
        .await;
        self.wait_state(PluginState::Enabled).await;
        consent
    }

    /// Enables another bundled plugin (`Env::with_projects`) once it is
    /// seeded, with the user's consent to every permission it declares.
    pub async fn enable_plugin(&self, id: &str) {
        let deadline = Instant::now() + SETTLE;
        let details = loop {
            if let Ok(Response::Plugin { plugin }) =
                self.request(Request::GetPlugin { plugin: id.into() }).await
            {
                break *plugin;
            }
            assert!(Instant::now() < deadline, "{id} not seeded\n{}", self.log());
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        if details.summary.state != PluginState::Enabled {
            self.ok(Request::EnablePlugin {
                plugin: id.into(),
                consent: details.permissions.iter().map(|p| p.key.clone()).collect(),
            })
            .await;
        }
        loop {
            let enabled = matches!(
                self.ok(Request::GetPlugin { plugin: id.into() }).await,
                Response::Plugin { plugin } if plugin.summary.state == PluginState::Enabled
            );
            if enabled {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{id} not enabled\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Enables the plugin with the user's consent to every permission it
    /// needs, none of the optional ones (plugin API 1.9,
    /// `requiresPermissions`); the grant keys.
    pub async fn enable_required(&self) -> Vec<String> {
        let consent: Vec<String> = self
            .details()
            .await
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.key.clone())
            .collect();
        self.ok(Request::EnablePlugin {
            plugin: self.id.into(),
            consent: consent.clone(),
        })
        .await;
        self.wait_state(PluginState::Enabled).await;
        consent
    }

    pub async fn disable(&self) {
        self.ok(Request::DisablePlugin {
            plugin: self.id.into(),
        })
        .await;
        self.wait_state(PluginState::Disabled).await;
    }

    pub async fn wait_state(&self, state: PluginState) -> PluginSummary {
        let deadline = Instant::now() + SETTLE;
        loop {
            let summary = self.summary().await;
            if let Some(summary) = summary.as_ref().filter(|s| s.state == state) {
                return summary.clone();
            }
            assert!(
                Instant::now() < deadline,
                "not {state:?}: {summary:?}\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Runs the plugin's `action` as the app's "Try it" does (a run the
    /// user started).
    pub async fn invoke(&self, action: &str, params: &Value) -> Outcome {
        match self
            .ok(Request::InvokePluginAction {
                plugin: self.id.into(),
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

    /// Runs a stored action instance as the app's `runAction` does (a run
    /// the user started), through the engine: resource parameters, the
    /// project parameter and their resolution included.
    pub async fn run_action(&self, action: ActionInstanceId) -> Outcome {
        match self.ok(Request::RunAction { action }).await {
            Response::ActionResult { result } => result.into(),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The first run of `workflow` (one its trigger started), finished.
    pub async fn first_run(&self, workflow: WorkflowId) -> ExecutionRecord {
        let deadline = Instant::now() + SETTLE;
        loop {
            let Response::Executions { executions } = self
                .ok(Request::ListExecutions {
                    workflow: Some(workflow),
                    limit: None,
                })
                .await
            else {
                panic!("executions")
            };
            if let Some(run) = executions.first() {
                return self.finished(run.id).await;
            }
            assert!(
                Instant::now() < deadline,
                "the trigger did not fire\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn finished(&self, execution: deckpilot_core::ExecutionId) -> ExecutionRecord {
        let deadline = Instant::now() + SETTLE;
        loop {
            let Response::Execution { execution: record } =
                self.ok(Request::GetExecution { execution }).await
            else {
                panic!("execution")
            };
            if record.summary.state != ExecutionState::Running {
                return *record;
            }
            assert!(Instant::now() < deadline, "still running: {record:#?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn start_workflow(&self, workflow: WorkflowId) -> ExecutionRecord {
        let Response::ExecutionStarted { execution } =
            self.ok(Request::StartWorkflow { workflow }).await
        else {
            panic!("startWorkflow")
        };
        self.finished(execution.id).await
    }

    /// What `references` resolve to in a preview of `profile` (its active
    /// project).
    pub async fn preview(
        &self,
        profile: deckpilot_core::ProfileId,
        references: &[&str],
    ) -> Vec<deckpilot_core::variables::ReferenceStatus> {
        let items = references
            .iter()
            .map(|reference| deckpilot_core::variables::PreviewItem {
                reference: (*reference).to_owned(),
                expected: None,
            })
            .collect();
        match self
            .ok(Request::PreviewReferences {
                context: Box::new(deckpilot_core::variables::ResolutionContext::preview(
                    profile,
                )),
                items,
            })
            .await
        {
            Response::ReferencePreviews { previews } => {
                previews.into_iter().map(|p| p.status).collect()
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    pub async fn stop(self) {
        let _ = self.request(Request::Shutdown).await;
        let deadline = Instant::now() + STARTUP;
        let mut agent = self;
        while agent.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the agent did not stop");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

// ------------------------------------------------------------- fixtures

/// An action instance of the plugin's `action` with `parameters`, added
/// to `profile`.
pub fn instance(
    profile: &mut Profile,
    plugin: &str,
    action: &str,
    parameters: &[(&str, ParameterValue)],
) -> ActionInstanceId {
    let mut values = ActionParameters::new();
    for (key, value) in parameters {
        values = values.with(*key, value.clone());
    }
    let instance = ActionInstance {
        id: ActionInstanceId::new(),
        name: format!("{plugin}.{action}"),
        action_type: ActionTypeId::new(format!("{plugin}.{action}")).unwrap(),
        parameters: values,
        description: String::new(),
        icon: None,
        label: None,
    };
    let id = instance.id;
    profile.actions.push(instance);
    id
}

/// A workflow `start → <node>` running `action`, run every minute by its
/// schedule (a run the user did not start).
pub fn scheduled_workflow(
    profile: &mut Profile,
    name: &str,
    action: ActionInstanceId,
) -> WorkflowId {
    let mut workflow = support::graph_workflow(
        name,
        support::graph(
            &json!([
                {"id": "start", "type": "trigger"},
                {"id": "run", "type": "action", "action": action},
            ]),
            &json!([{"from": "start", "to": "run"}]),
        ),
    );
    workflow
        .triggers
        .push(deckpilot_core::WorkflowTrigger::schedule(
            deckpilot_core::Schedule::Interval { minutes: 1 },
        ));
    let id = workflow.id;
    profile.workflows.push(workflow);
    id
}

/// The outputs of node `node` of a finished run.
pub fn node<'a>(record: &'a ExecutionRecord, node: &str) -> &'a deckpilot_core::graph::NodeReport {
    record
        .nodes
        .iter()
        .find(|n| n.node.as_str() == node)
        .unwrap_or_else(|| panic!("node {node} ran: {record:#?}"))
}
