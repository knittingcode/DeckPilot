//! The core with every optional plugin absent or off (WP-T1, the
//! acceptance gate of `docs/marketplace/module-migration.md` §11 D1, D2):
//! the built-ins `github`, `gitlab`, `jira`, `git`, `docker`, `workspace`
//! and `projects` are gone, and the core must stand on its own. Through
//! the real agent, the real sandboxed host and every first-party plugin of
//! `plugins/first-party` built to wasm and bundled as the release bundles
//! them (staged as `default`-class so that a fresh install seeds them),
//! with a simulated Stream Deck MK.2, three ways:
//!
//! * `--disable-plugins`: no plugin runtime at all;
//! * every first-party plugin installed and disabled by the user;
//! * every first-party plugin uninstalled by the user (tombstoned).
//!
//! Each time:
//!
//! * the agent starts and the device renders every key of its home page;
//! * a profile seeded with **every** legacy action id of the seven moved
//!   modules (`ACTION_ALIASES`) and the development seed's "Start Work"
//!   and "End Work" graphs (`tools/dev/src/seed/examples.rs`, included as
//!   is) loads;
//! * every legacy action resolves unavailable with a reason, never as an
//!   unknown type and never with a panic; pressing their keys leaves the
//!   agent answering;
//! * a workflow bound to a key still runs (a core one succeeds; the seed's
//!   graph, whose nodes are the plugins', is refused with the reason);
//! * the stored layouts, actions and workflows are byte for byte the same
//!   after the app's save round-trip;
//! * `getActiveProject` answers the clear "the Projects plugin is not
//!   active" error;
//! * turning the plugins back on (a start without the switch, enabling,
//!   restoring; and the user's consent to the migrations the legacy ids
//!   start) brings every legacy action back to its plugin, with nothing in
//!   the configuration changed.
//!
//! Nothing leaves the computer: the plugins' HTTP gets no canned answer
//! (`DECKPILOT_TEST_PLUGIN_HTTP`), pages and programs are recorded and
//! never opened (`DECKPILOT_TEST_PLUGIN_OPEN_URL`,
//! `DECKPILOT_TEST_PLUGIN_LAUNCH`), and `PATH` is an empty folder, so no
//! `git` or `docker` can run.
#![cfg(all(feature = "plugins", feature = "test-hooks"))]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use deckpilot_core::alias::ACTION_ALIASES;
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionTypeId, BindingTarget, DeviceLayout,
    ExecutionState, LayoutStatus, ParameterValue, Profile, Project, ProjectCatalog, ProjectKey,
    ResourceKey, WorkflowId,
};
use deckpilot_device::{ControlId, ControlInput, DeviceId, models};
use deckpilot_ipc::plugins::PluginState;
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, AgentStatus, ClientError, ClientKind, KeyMaterial,
    Request, Response,
};
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_storage::{ConfigStore, DatabaseKey};
use serde_json::Value;
use support::TempDir;

const STARTUP: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(90);

/// The modules whose built-ins were removed.
const REMOVED: [&str; 7] = [
    "github",
    "gitlab",
    "jira",
    "git",
    "docker",
    "workspace",
    "projects",
];

/// Every first-party plugin: its directory (and id `deckpilot.<dir>`) and
/// its crate.
const FIRST_PARTY: [(&str, &str); 9] = [
    ("docker", "deckpilot-docker"),
    ("git", "deckpilot-git"),
    ("github", "deckpilot-github"),
    ("gitlab", "deckpilot-gitlab"),
    ("jira", "deckpilot-jira"),
    ("notifications", "deckpilot-notifications"),
    ("projects", "deckpilot-projects"),
    ("workflows", "deckpilot-workflows"),
    ("workspace", "deckpilot-workspace"),
];

fn plugin_id(dir: &str) -> String {
    format!("deckpilot.{dir}")
}

// ------------------------------------------------- the seed's examples

/// The seed's builder interface, for `examples.rs`: a legacy id is stored
/// as given (as 0.9.x stored it), never checked against a built-in.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Builder<'a> {
    registry: &'a deckpilot_core::ActionRegistry,
    profile: Profile,
}

impl Builder<'_> {
    fn action(
        &mut self,
        name: &str,
        action_type: ActionTypeId,
        parameters: &ActionParameters,
        icon: &str,
        label: Option<&str>,
    ) -> Result<ActionInstanceId> {
        let mut instance = if deckpilot_core::alias::alias_of(&action_type).is_some() {
            legacy(name, action_type, parameters.clone())
        } else {
            self.registry
                .create_instance(name, action_type, parameters)?
        };
        instance.icon = Some(icon.to_owned());
        instance.label = label.map(str::to_owned);
        let id = instance.id;
        self.profile.actions.push(instance);
        Ok(id)
    }
}

/// The seed's legacy ids (`tools/dev/src/seed.rs` `actions`).
mod actions {
    pub const CURRENT: &str = "projects.current";

    pub fn id(value: &str) -> deckpilot_core::ActionTypeId {
        deckpilot_core::ActionTypeId::new(value).unwrap()
    }
}

#[path = "../../../tools/dev/src/seed/examples.rs"]
mod examples;

fn legacy(name: &str, action_type: ActionTypeId, parameters: ActionParameters) -> ActionInstance {
    ActionInstance {
        id: ActionInstanceId::new(),
        name: name.to_owned(),
        action_type,
        parameters,
        description: String::new(),
        icon: None,
        label: None,
    }
}

/// The profile: two projects, a core key, a core workflow, one action of
/// every legacy id of the removed modules and the seed's two graphs, on a
/// generated MK.2 layout whose home page starts with the core key and the
/// core workflow.
struct Seeded {
    profile: Profile,
    core_action: ActionInstanceId,
    core_workflow: WorkflowId,
    start_work: WorkflowId,
    legacy: Vec<(ActionInstanceId, String)>,
}

fn seeded() -> Seeded {
    let registry = deckpilot_modules::builtin(deckpilot_core::Os::current()).action_registry();
    let mut b = Builder {
        registry: &registry,
        profile: Profile::new("Work"),
    };
    for key in ["SHOP", "CMR"] {
        let mut project = Project::new(ProjectKey::new(key).unwrap(), key);
        project.resources.insert(
            "repository".into(),
            ResourceKey::new(format!("{}.repository", key.to_lowercase())).unwrap(),
        );
        b.profile.add_project(project).unwrap();
    }
    b.profile.default_project = Some(b.profile.projects[0].id);
    let core_action = b
        .action(
            "Core key",
            deckpilot_modules::general::log_id(),
            &ActionParameters::new().with("message", ParameterValue::Text("core".into())),
            "bell",
            None,
        )
        .unwrap();
    let mut legacy_ids = Vec::new();
    for alias in ACTION_ALIASES
        .iter()
        .filter(|a| REMOVED.contains(&a.module))
    {
        let instance = legacy(
            alias.legacy,
            ActionTypeId::new(alias.legacy).unwrap(),
            ActionParameters::new(),
        );
        legacy_ids.push((instance.id, alias.legacy.to_owned()));
        b.profile.actions.push(instance);
    }
    let examples = examples::actions(&mut b).unwrap();
    let (start, end) = (
        examples::start_work(&examples),
        examples::end_work(&examples),
    );
    let start_work = start.id;
    let core = support::graph_workflow(
        "Core run",
        support::graph(
            &serde_json::json!([
                {"id": "start", "type": "trigger"},
                {"id": "log", "type": "action", "action": core_action},
            ]),
            &serde_json::json!([{"from": "start", "to": "log"}]),
        ),
    );
    let core_workflow = core.id;
    b.profile.workflows.extend([core, start, end]);
    let mut profile = b.profile;
    let mut layout = DeviceLayout::generate("Main", &models::stream_deck_mk2(), &profile);
    // Key 1 of the home page runs the core workflow (swapped with the key
    // the generator gave it on a later page).
    let workflow = BindingTarget::Workflow {
        workflow: core_workflow,
    };
    let home = layout.home_page;
    let key_one = layout
        .pages
        .iter()
        .find(|p| p.id == home)
        .unwrap()
        .bindings
        .iter()
        .find(|b| b.control == ControlId::Key { index: 1 })
        .unwrap()
        .target
        .clone();
    for page in &mut layout.pages {
        for binding in &mut page.bindings {
            if page.id == home && binding.control == (ControlId::Key { index: 1 }) {
                binding.target = workflow.clone();
            } else if binding.target == workflow {
                binding.target = key_one.clone();
            }
        }
    }
    profile.layouts.push(layout);
    let catalog = deckpilot_device::ModelCatalog::builtin();
    let issues = profile.validate(&catalog);
    assert!(issues.is_empty(), "{issues:?}");
    Seeded {
        profile,
        core_action,
        core_workflow,
        start_work,
        legacy: legacy_ids,
    }
}

// ------------------------------------------------------------ artifacts

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Artifacts {
    host: PathBuf,
    wasm: PathBuf,
}

fn cargo_in_runtime(args: &[&str]) {
    let status = Command::new(option_env!("CARGO").unwrap_or("cargo"))
        .current_dir(repo().join("plugins/runtime"))
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

/// The host and every first-party plugin's module, built once from
/// `plugins/runtime` (or taken from `DECKPILOT_E2E_PLUGIN_HOST` and
/// `DECKPILOT_E2E_WASM_DIR`).
fn artifacts() -> &'static Artifacts {
    static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let file = |krate: &str| format!("{}.wasm", krate.replace('-', "_"));
        let wasm = std::env::var_os("DECKPILOT_E2E_WASM_DIR")
            .map(PathBuf::from)
            .filter(|dir| FIRST_PARTY.iter().all(|(_, k)| dir.join(file(k)).is_file()))
            .unwrap_or_else(|| {
                let mut args = vec!["build", "--locked"];
                for (_, krate) in &FIRST_PARTY {
                    args.extend(["-p", krate]);
                }
                args.extend(["--target", "wasm32-unknown-unknown", "--profile", "guest"]);
                cargo_in_runtime(&args);
                repo().join("plugins/runtime/target/wasm32-unknown-unknown/guest")
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
                repo().join(format!(
                    "plugins/runtime/target/debug/deckpilot-plugin-host{}",
                    std::env::consts::EXE_SUFFIX
                ))
            },
            PathBuf::from,
        );
        Artifacts { host, wasm }
    })
}

// ---------------------------------------------------------- environment

struct Env {
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
    seeded: Seeded,
}

impl Env {
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let seeded = seeded();
        let store = support::open_store(&paths, &key);
        store.save_profile(&seeded.profile).unwrap();
        store.set_active_profile(Some(seeded.profile.id)).unwrap();
        store
            .set_active_project(seeded.profile.id, Some(seeded.profile.projects[0].id))
            .unwrap();
        drop(store);
        let env = Self {
            dir,
            paths,
            key,
            seeded,
        };
        env.stage();
        std::fs::create_dir_all(env.empty_path()).unwrap();
        std::fs::write(env.dir.path().join("http.json"), "[]").unwrap();
        env
    }

    fn bundled(&self) -> PathBuf {
        self.dir.path().join("resources/bundled")
    }

    fn pins_file(&self) -> PathBuf {
        self.dir.path().join("compiled-pins.json")
    }

    fn empty_path(&self) -> PathBuf {
        self.dir.path().join("no-programs")
    }

    /// Every first-party plugin as `build.sh` stages it, pinned with class
    /// `default` (seeded on a fresh install, as the bundled migration-class
    /// ones are once a migration needs them).
    fn stage(&self) {
        let mut pinned = Vec::new();
        for (dir_name, krate) in FIRST_PARTY {
            let source = repo().join("plugins/first-party").join(dir_name);
            let id = plugin_id(dir_name);
            let dir = self.bundled().join(&id);
            std::fs::create_dir_all(dir.join("assets")).unwrap();
            std::fs::create_dir_all(dir.join("locales")).unwrap();
            let manifest: Value =
                serde_json::from_slice(&std::fs::read(source.join("manifest.json")).unwrap())
                    .unwrap();
            let mut files = vec!["manifest.json".to_owned(), "LICENSE".to_owned()];
            files.extend(
                manifest["icons"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(_, v)| v.as_str().unwrap().to_owned()),
            );
            files.extend(
                manifest["locales"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|tag| format!("locales/{}.json", tag.as_str().unwrap())),
            );
            for file in files {
                std::fs::copy(source.join(&file), dir.join(&file)).unwrap();
            }
            std::fs::copy(
                artifacts()
                    .wasm
                    .join(format!("{}.wasm", krate.replace('-', "_"))),
                dir.join("plugin.wasm"),
            )
            .unwrap();
            let version = semver::Version::parse(manifest["version"].as_str().unwrap()).unwrap();
            pinned.push((PluginId::parse(&id).unwrap(), version, BundleClass::Default));
        }
        let pins = Pins::compute(&self.bundled(), &pinned).unwrap();
        std::fs::write(self.bundled().join("pins.json"), pins.to_json_pretty()).unwrap();
        std::fs::write(self.pins_file(), pins.to_json_pretty()).unwrap();
    }

    /// Starts the agent with a simulated MK.2 and unlocks it.
    async fn start(&self, plugins: bool) -> Agent {
        let log_path = self.dir.path().join("agent.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"));
        command
            .args([
                "--headless",
                "--no-usb",
                "--mock-devices",
                "mk2",
                "--log-level",
                "debug",
            ])
            .arg("--plugin-host")
            .arg(&artifacts().host)
            .arg("--bundled-plugins")
            .arg(self.bundled());
        if !plugins {
            command.arg("--disable-plugins");
        }
        command
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_BUNDLED_PINS", self.pins_file())
            .env(
                "DECKPILOT_TEST_PLUGIN_LAUNCH",
                self.dir.path().join("launched.jsonl"),
            )
            .env(
                "DECKPILOT_TEST_PLUGIN_OPEN_URL",
                self.dir.path().join("opened.txt"),
            )
            .env(
                "DECKPILOT_TEST_PLUGIN_HTTP",
                self.dir.path().join("http.json"),
            )
            .env("PATH", self.empty_path())
            .env_remove("DECKPILOT_TEST_AGENT_ARGS")
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
        let agent = Agent {
            child: command.spawn().expect("the agent starts"),
            paths: self.paths.clone(),
            log: log_path,
        };
        agent
            .ok(Request::Unlock {
                key: KeyMaterial::new(self.key.to_hex()),
            })
            .await;
        agent
    }

    /// What the store holds of the profile, serialized: layouts, actions
    /// and workflows.
    fn stored(&self) -> (String, String, String) {
        let store = support::open_store(&self.paths, &self.key);
        let profile = store.load_profile(self.seeded.profile.id).unwrap().unwrap();
        (
            serde_json::to_string(&profile.layouts).unwrap(),
            serde_json::to_string(&profile.actions).unwrap(),
            serde_json::to_string(&profile.workflows).unwrap(),
        )
    }

    fn seeded_documents(&self) -> (String, String, String) {
        let profile = &self.seeded.profile;
        (
            serde_json::to_string(&profile.layouts).unwrap(),
            serde_json::to_string(&profile.actions).unwrap(),
            serde_json::to_string(&profile.workflows).unwrap(),
        )
    }
}

// ---------------------------------------------------------------- agent

struct Agent {
    child: Child,
    paths: AgentPaths,
    log: PathBuf,
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

    async fn request(&self, request: Request) -> std::result::Result<Response, ClientError> {
        let what = request.name();
        tokio::time::timeout(Duration::from_secs(120), async {
            self.client().await.request(request).await
        })
        .await
        .unwrap_or_else(|_| panic!("{what} took too long\n{}", self.log()))
    }

    async fn ok(&self, request: Request) -> Response {
        let what = request.name();
        self.request(request)
            .await
            .unwrap_or_else(|e| panic!("{what}: {e}\n{}", self.log()))
    }

    async fn status(&self) -> AgentStatus {
        match self.ok(Request::GetStatus).await {
            Response::Status { status } => *status,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn wait_status(&self, what: &str, done: impl Fn(&AgentStatus) -> bool) -> AgentStatus {
        let deadline = Instant::now() + SETTLE;
        loop {
            let status = self.status().await;
            if done(&status) {
                return status;
            }
            assert!(Instant::now() < deadline, "{what}\n{}", self.log());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The first-party plugins installed now, with their state.
    async fn first_party(&self) -> Vec<(String, PluginState)> {
        match self.ok(Request::ListPlugins).await {
            Response::Plugins { plugins } => plugins
                .into_iter()
                .filter(|p| FIRST_PARTY.iter().any(|(dir, _)| plugin_id(dir) == p.id))
                .map(|p| (p.id, p.state))
                .collect(),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn wait_plugins(&self, what: &str, done: impl Fn(&[(String, PluginState)]) -> bool) {
        let deadline = Instant::now() + SETTLE;
        loop {
            let plugins = self.first_party().await;
            if done(&plugins) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: {plugins:?}\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Enables every first-party plugin with the user's consent to every
    /// permission it declares (the optional ones included).
    async fn enable_all(&self) {
        for (dir, _) in FIRST_PARTY {
            let id = plugin_id(dir);
            let Response::Plugin { plugin } =
                self.ok(Request::GetPlugin { plugin: id.clone() }).await
            else {
                panic!("getPlugin {id}");
            };
            let consent: Vec<String> = plugin.permissions.iter().map(|p| p.key.clone()).collect();
            if plugin.summary.state == PluginState::Enabled {
                // Enabled by its migration: the optional permissions too
                // (`tools:docker:modify` is never derived).
                self.ok(Request::GrantPluginPermissions {
                    plugin: id.clone(),
                    permissions: consent,
                })
                .await;
                continue;
            }
            self.ok(Request::EnablePlugin {
                plugin: id.clone(),
                consent,
            })
            .await;
        }
        self.wait_plugins("every first-party plugin enabled", |plugins| {
            plugins.len() == FIRST_PARTY.len()
                && plugins.iter().all(|(_, s)| *s == PluginState::Enabled)
        })
        .await;
    }

    /// Approves everything the module migrations wait for (the profile
    /// uses the legacy ids, so the migration engine asks for the consent
    /// the plugins' high tiers need), as the user does in the migration
    /// dialog, until none waits.
    async fn approve_migrations(&self) {
        use deckpilot_ipc::plugins::MigrationPhase;
        let deadline = Instant::now() + SETTLE;
        loop {
            let Response::ModuleMigrations { migrations } =
                self.ok(Request::ListModuleMigrations).await
            else {
                panic!("listModuleMigrations");
            };
            let waiting: Vec<_> = migrations
                .into_iter()
                .filter(|m| m.phase == MigrationPhase::AwaitingConsent)
                .collect();
            if waiting.is_empty() {
                return;
            }
            for migration in waiting {
                self.ok(Request::DecideModuleMigration {
                    module: migration.module,
                    approve: migration.consent.into_iter().map(|c| c.key).collect(),
                    decline: Vec::new(),
                })
                .await;
            }
            assert!(
                Instant::now() < deadline,
                "migrations still wait\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// What running the stored action `action` answered: the result's
    /// message, or the refusal's.
    async fn run(&self, action: ActionInstanceId) -> String {
        match self.request(Request::RunAction { action }).await {
            Ok(Response::ActionResult { result }) => result.message,
            Ok(other) => panic!("unexpected {other:?}"),
            Err(ClientError::Agent(error)) => error.message,
            Err(other) => panic!("runAction: {other}\n{}", self.log()),
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

fn unavailable(message: &str) -> bool {
    message.contains("is unavailable")
}

/// The next finished run of `workflow` on `events`.
async fn finished(events: &mut AgentClient, workflow: WorkflowId) -> ExecutionState {
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let event = tokio::time::timeout_at(deadline, events.next_event())
            .await
            .expect("the run finishes in time")
            .unwrap();
        if let AgentEvent::ExecutionFinished { execution } = event
            && execution.workflow == workflow
        {
            return execution.state;
        }
    }
}

// ---------------------------------------------------------------- checks

/// Everything the core must do with no optional plugin serving.
#[allow(clippy::too_many_lines)] // One pass over every promise of WP-T1.
async fn the_core_stands_alone(env: &Env, agent: &Agent) {
    // The device renders every key of its home page.
    let status = agent
        .wait_status("the simulated MK.2 shows the layout", |s| {
            s.devices
                .first()
                .is_some_and(|d| matches!(d.layout, LayoutStatus::Compatible { .. }))
        })
        .await;
    let device: DeviceId = status.devices[0].id.clone();
    assert_eq!(status.devices[0].page.as_deref(), Some("Home"));
    let deadline = Instant::now() + SETTLE;
    loop {
        let Response::MockDevice { inspection } = agent
            .ok(Request::InspectMockDevice {
                device: device.clone(),
            })
            .await
        else {
            panic!("inspectMockDevice");
        };
        let drawn = (0..15)
            .filter(|index| {
                inspection
                    .frames
                    .iter()
                    .any(|f| f.control == ControlId::Key { index: *index } && f.hash.is_some())
            })
            .count();
        if drawn == 15 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "only {drawn} of 15 keys drawn\n{}",
            agent.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Every legacy action: unavailable with a reason that names it or its
    // plugin, never unknown.
    for (action, legacy) in &env.seeded.legacy {
        let message = agent.run(*action).await;
        assert!(unavailable(&message), "{legacy}: {message}");
        let module = legacy.split('.').next().unwrap();
        assert!(
            message.contains(&format!("deckpilot.{module}")) || message.contains(legacy.as_str()),
            "{legacy}: {message}"
        );
        assert!(!message.contains("unknown"), "{legacy}: {message}");
    }
    // The core key still runs.
    assert_eq!(agent.run(env.seeded.core_action).await, "core");

    // Keys bound to them leave the agent answering; the workflow key runs
    // its workflow.
    let mut events = agent.client().await;
    events.subscribe().await.unwrap();
    // Not key 14: it opens the next page.
    for index in 2..14 {
        for input in [ControlInput::Press, ControlInput::Release] {
            agent
                .ok(Request::SimulateInput {
                    device: device.clone(),
                    control: ControlId::Key { index },
                    input,
                })
                .await;
        }
    }
    for input in [ControlInput::Press, ControlInput::Release] {
        agent
            .ok(Request::SimulateInput {
                device: device.clone(),
                control: ControlId::Key { index: 1 },
                input,
            })
            .await;
    }
    assert_eq!(
        finished(&mut events, env.seeded.core_workflow).await,
        ExecutionState::Succeeded
    );
    // The seed's graph, whose nodes are the plugins', is refused with the
    // reason before it starts; nothing else happens.
    match agent
        .request(Request::StartWorkflow {
            workflow: env.seeded.start_work,
        })
        .await
    {
        Err(ClientError::Agent(error)) => assert!(
            error.message.contains("cannot run") && error.message.contains("deckpilot."),
            "{}",
            error.message
        ),
        other => panic!("startWorkflow: {other:?}"),
    }
    assert!(!agent.log().contains("panicked"), "{}", agent.log());
    agent.status().await;

    // No project context provider: a clear refusal; nothing is lost.
    match agent.request(Request::GetActiveProject).await {
        Err(ClientError::Agent(error)) => assert!(
            error.message.contains("the Projects plugin is not active"),
            "{}",
            error.message
        ),
        other => panic!("getActiveProject: {other:?}"),
    }
    assert!(agent.status().await.active_project.is_none());

    // The app's save round-trip changes no byte of the documents.
    assert_eq!(env.stored(), env.seeded_documents());
    {
        let store = support::open_store(&env.paths, &env.key);
        let profile = store.load_profile(env.seeded.profile.id).unwrap().unwrap();
        store.save_profile(&profile).unwrap();
        assert_eq!(
            store.active_project(profile.id).unwrap(),
            Some(env.seeded.profile.projects[0].id)
        );
    }
    agent.ok(Request::ReloadConfiguration).await;
    assert_eq!(env.stored(), env.seeded_documents());
}

/// With the plugins back, every legacy action reaches its plugin again,
/// the project context is back, and nothing in the configuration changed.
async fn every_key_recovers(env: &Env, agent: &Agent) {
    // The migrations first: approving them enables their plugins.
    agent.approve_migrations().await;
    agent.enable_all().await;
    let deadline = Instant::now() + SETTLE;
    for (action, legacy) in &env.seeded.legacy {
        loop {
            let message = agent.run(*action).await;
            if !unavailable(&message) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{legacy} did not recover: {message}\n{}",
                agent.log()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    assert!(matches!(
        agent.ok(Request::GetActiveProject).await,
        Response::ActiveProject { .. }
    ));
    let (layouts, actions, workflows) = env.stored();
    let (seeded_layouts, seeded_actions, seeded_workflows) = env.seeded_documents();
    assert_eq!(layouts, seeded_layouts);
    assert_eq!(actions, seeded_actions);
    assert_eq!(workflows, seeded_workflows);
}

// ------------------------------------------------------------------ tests

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_core_runs_with_plugins_switched_off() {
    let env = Env::new("deckpilot-core-alone-switch");
    let agent = env.start(false).await;
    assert!(matches!(
        agent.request(Request::ListPlugins).await,
        Err(ClientError::Agent(_))
    ));
    the_core_stands_alone(&env, &agent).await;
    agent.stop().await;

    // Started again with plugins: seeded, enabled, every key back.
    let agent = env.start(true).await;
    agent
        .wait_plugins("every first-party plugin seeded", |plugins| {
            plugins.len() == FIRST_PARTY.len()
        })
        .await;
    every_key_recovers(&env, &agent).await;
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_core_runs_with_every_first_party_plugin_disabled() {
    let env = Env::new("deckpilot-core-alone-disabled");
    let agent = env.start(true).await;
    agent
        .wait_plugins("every first-party plugin seeded", |plugins| {
            plugins.len() == FIRST_PARTY.len()
        })
        .await;
    for (id, state) in agent.first_party().await {
        if state == PluginState::Enabled {
            agent.ok(Request::DisablePlugin { plugin: id }).await;
        }
    }
    agent
        .wait_plugins("every first-party plugin disabled", |plugins| {
            plugins.iter().all(|(_, s)| *s == PluginState::Disabled)
        })
        .await;
    the_core_stands_alone(&env, &agent).await;
    every_key_recovers(&env, &agent).await;
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_core_runs_with_every_first_party_plugin_uninstalled() {
    let env = Env::new("deckpilot-core-alone-uninstalled");
    let agent = env.start(true).await;
    agent
        .wait_plugins("every first-party plugin seeded", |plugins| {
            plugins.len() == FIRST_PARTY.len()
        })
        .await;
    for (dir, _) in FIRST_PARTY {
        agent
            .ok(Request::UninstallPlugin {
                plugin: plugin_id(dir),
                keep: deckpilot_ipc::plugins::PluginRetention::default(),
            })
            .await;
    }
    agent
        .wait_plugins("every first-party plugin uninstalled", <[_]>::is_empty)
        .await;
    agent.stop().await;
    // Tombstoned: a restart seeds none of them again.
    let agent = env.start(true).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(agent.first_party().await, Vec::new());
    the_core_stands_alone(&env, &agent).await;

    for (dir, _) in FIRST_PARTY {
        agent
            .ok(Request::RestoreBundledPlugin {
                plugin: plugin_id(dir),
            })
            .await;
    }
    agent
        .wait_plugins("every first-party plugin restored", |plugins| {
            plugins.len() == FIRST_PARTY.len()
        })
        .await;
    every_key_recovers(&env, &agent).await;
    agent.stop().await;
}
