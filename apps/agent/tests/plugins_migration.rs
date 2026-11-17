//! The module migration engine end to end, through the real agent, the
//! real sandboxed plugin host and real storage, each test in its own data
//! directory (feature `plugins`; `docs/marketplace/module-migration.md`
//! §5.5–§5.9):
//!
//! * the agent's fixture legacy module `legacy-fixture` and its alias
//!   (feature `test-hooks`, with `DECKPILOT_TEST_LEGACY_FIXTURE=1`; the
//!   shipped alias table stays empty) and the fixture plugin
//!   `plugins/first-party/test-fixtures/legacy-fixture`, built to wasm and
//!   staged as a `migration`-class bundled plugin with its pins
//!   (`DECKPILOT_TEST_BUNDLED_PINS`);
//! * an alpha.3-style `deckpilot.db`: the module's saved settings, its
//!   stored credential, a profile with an instance of the legacy type;
//! * the fixture API answers from canned responses
//!   (`DECKPILOT_TEST_PLUGIN_HTTP`), which also log the token the plugin
//!   sent;
//! * the user's consent and Retry come through the app's requests
//!   (`listModuleMigrations`, `decideModuleMigration`,
//!   `retryModuleMigration`) and their events, or, to decide before a
//!   start, from `DECKPILOT_TEST_MIGRATION_DECISIONS`;
//!   `DECKPILOT_TEST_MIGRATION_EXIT` ends the agent after a step, as a
//!   power cut would.
#![cfg(feature = "plugins")]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionResult, ActionStatus, ActionTypeId,
    CredentialId, ModuleId, ModuleSettings, ParameterValue, Profile, Secret,
};
use deckpilot_ipc::plugins::{
    self as ipc, ConsentDecision, MigrationRoute, ModuleMigrationInfo, PluginEvent, PluginOrigin,
    PluginState,
};
use deckpilot_ipc::{
    AgentClient, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_store::{MigrationPhase, ModuleMigration, PluginStore, TombstoneDecision};
use deckpilot_storage::{ConfigStore, DatabaseKey};
use serde_json::{Value, json};
use support::TempDir;

const MODULE: &str = "legacy-fixture";
const PLUGIN: &str = "deckpilot.legacy-fixture";
const ACTION: &str = "legacy-fixture.whoami";
const CREDENTIAL: &str = "fixture.token";
const TOKEN: &str = "s3cret-fixture-token";
const STARTUP: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(60);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime() -> PathBuf {
    repo().join("plugins/runtime")
}

fn fixture() -> PathBuf {
    repo().join("plugins/first-party/test-fixtures/legacy-fixture")
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

/// The host and the fixture's module, built from `plugins/runtime` (or
/// taken from `DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR`).
fn artifacts() -> &'static Artifacts {
    static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let wasm_dir = std::env::var_os("DECKPILOT_E2E_WASM_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.join("deckpilot_legacy_fixture.wasm").is_file())
            .unwrap_or_else(|| {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    "deckpilot-legacy-fixture",
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
            module: wasm_dir.join("deckpilot_legacy_fixture.wasm"),
        }
    })
}

/// How one agent start is set up.
#[derive(Default, Clone)]
struct Start {
    /// `DECKPILOT_TEST_MIGRATION_DECISIONS`.
    decisions: &'static str,
    /// Plugins off (`--disable-plugins`): as an older build.
    plugins_off: bool,
    /// No pins and no bundle: a build without the plugin.
    unbundled: bool,
    /// Extra environment.
    env: Vec<(&'static str, String)>,
}

impl Start {
    fn consent() -> Self {
        Self {
            decisions: "consent:legacy-fixture",
            ..Self::default()
        }
    }
}

/// A data directory with an alpha.3-style configuration, the application
/// resources with the fixture plugin bundled, the pins of exactly those
/// files, and the canned fixture API.
struct Env {
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
    action: ActionInstanceId,
}

impl Env {
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let action = ActionInstanceId::new();
        let env = Self {
            dir,
            paths,
            key,
            action,
        };
        env.write_alpha3();
        env.stage("0.1.0", |_| {});
        std::fs::write(
            env.http_file(),
            json!([{
                "url": "https://api.example.com/whoami",
                "status": 200,
                "body": "{\"login\":\"octo\"}"
            }])
            .to_string(),
        )
        .unwrap();
        env
    }

    /// What an older app left in `deckpilot.db`: the fixture module saved
    /// (enabled, a greeting), its token stored, a profile using it.
    fn write_alpha3(&self) {
        let store = support::open_store(&self.paths, &self.key);
        store
            .set_credential(
                &CredentialId::new(CREDENTIAL).unwrap(),
                "Fixture token",
                Some(&Secret::new(TOKEN)),
            )
            .unwrap();
        let mut profile = Profile::new("Work");
        profile.actions.push(ActionInstance {
            id: self.action,
            name: "Who am I".into(),
            action_type: ActionTypeId::new(ACTION).unwrap(),
            parameters: ActionParameters::new()
                .with("credential", ParameterValue::Text(String::new())),
            description: String::new(),
            icon: None,
            label: None,
        });
        store.save_profile(&profile).unwrap();
        store.set_active_profile(Some(profile.id)).unwrap();
        drop(store);
        self.save_greeting("Hi");
    }

    /// The `modules` row, as an older app writes it.
    fn save_greeting(&self, greeting: &str) {
        let store = support::open_store(&self.paths, &self.key);
        store
            .set_module_state(
                &ModuleId::new(MODULE).unwrap(),
                &ModuleSettings {
                    enabled: true,
                    config: ActionParameters::new()
                        .with("greeting", ParameterValue::Text(greeting.into()))
                        .with("credential", ParameterValue::Text(CREDENTIAL.into())),
                },
            )
            .unwrap();
    }

    fn bundled(&self) -> PathBuf {
        self.dir.path().join("resources/bundled")
    }

    fn pins_file(&self) -> PathBuf {
        self.dir.path().join("compiled-pins.json")
    }

    fn empty_pins_file(&self) -> PathBuf {
        let path = self.dir.path().join("no-pins.json");
        std::fs::write(&path, Pins::empty().to_json_pretty()).unwrap();
        path
    }

    fn http_file(&self) -> PathBuf {
        self.dir.path().join("fixture-api.json")
    }

    /// What reached the fixture API: one line per request, with the
    /// credential header the agent injected.
    fn http_log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("fixture-api.json.log")).unwrap_or_default()
    }

    /// `bundled/deckpilot.legacy-fixture/` at `version` (its manifest
    /// changed by `edit`), and the pins the agent "was compiled with".
    fn stage(&self, version: &str, edit: impl FnOnce(&mut Value)) {
        let dir = self.bundled().join(PLUGIN);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for file in ["LICENSE", "assets/icon-128.png"] {
            std::fs::copy(fixture().join(file), dir.join(file)).unwrap();
        }
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(fixture().join("manifest.json")).unwrap())
                .unwrap();
        manifest["version"] = version.into();
        edit(&mut manifest);
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::copy(&artifacts().module, dir.join("plugin.wasm")).unwrap();
        let pins = Pins::compute(
            &self.bundled(),
            &[(
                PluginId::parse(PLUGIN).unwrap(),
                semver::Version::parse(version).unwrap(),
                BundleClass::Migration,
            )],
        )
        .unwrap();
        std::fs::write(self.bundled().join("pins.json"), pins.to_json_pretty()).unwrap();
        std::fs::write(self.pins_file(), pins.to_json_pretty()).unwrap();
    }

    fn command(&self, start: &Start) -> Command {
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
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_LEGACY_FIXTURE", "1")
            .env("DECKPILOT_TEST_PLUGIN_HTTP", self.http_file())
            .env("DECKPILOT_TEST_MIGRATION_DECISIONS", start.decisions)
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
        if start.unbundled {
            command.env("DECKPILOT_TEST_BUNDLED_PINS", self.empty_pins_file());
        } else {
            command
                .arg("--bundled-plugins")
                .arg(self.bundled())
                .env("DECKPILOT_TEST_BUNDLED_PINS", self.pins_file());
        }
        for (name, value) in &start.env {
            command.env(name, value);
        }
        command
    }

    async fn start(&self, start: &Start) -> Agent {
        let child = self.command(start).spawn().expect("the agent starts");
        let agent = Agent {
            child,
            paths: self.paths.clone(),
            log: self.dir.path().join("agent.log"),
        };
        agent
            .request(Request::Unlock {
                key: KeyMaterial::new(self.key.to_hex()),
            })
            .await
            .unwrap();
        agent
    }

    /// Starts an agent that ends itself (a test crash point) and waits
    /// for that.
    async fn start_until_exit(&self, start: &Start) {
        let mut agent = Agent {
            child: self.command(start).spawn().expect("the agent starts"),
            paths: self.paths.clone(),
            log: self.dir.path().join("agent.log"),
        };
        // The unlock may or may not be answered before the agent ends.
        let _ = tokio::time::timeout(
            STARTUP,
            agent.try_request(Request::Unlock {
                key: KeyMaterial::new(self.key.to_hex()),
            }),
        )
        .await;
        let deadline = Instant::now() + SETTLE;
        loop {
            if let Some(status) = agent.child.try_wait().unwrap() {
                let log = agent.log();
                let tail: Vec<&str> = log.lines().rev().take(40).collect();
                // 86, or a signal when another thread was still tearing
                // down while the process exited (as after a power cut, the
                // next start must cope either way).
                assert!(
                    status.code().is_none_or(|code| code == 86),
                    "{status:?}\n{}",
                    tail.join("\n")
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the agent did not end\n{}",
                agent.log()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The plugin store, opened from outside with the derived key.
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

    /// Waits until the module's migration is in `phase`.
    async fn wait_phase(&self, agent: &Agent, phase: MigrationPhase) -> ModuleMigration {
        let deadline = Instant::now() + SETTLE;
        loop {
            if let Some(migration) = self.migration()
                && migration.phase == phase
            {
                return migration;
            }
            assert!(
                Instant::now() < deadline,
                "not {phase}: {:?}\n{}",
                self.migration(),
                agent.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Everything an older app reads from `deckpilot.db` for the module.
    fn legacy_view(&self) -> (Value, Value, String, String) {
        let store = support::open_store(&self.paths, &self.key);
        let modules = store.module_states().unwrap();
        let row = serde_json::to_value(&modules[&ModuleId::new(MODULE).unwrap()]).unwrap();
        let credentials = serde_json::to_value(store.credentials().unwrap()).unwrap();
        let secret = deckpilot_core::CredentialSource::secret(
            &store.load_credentials().unwrap(),
            &CredentialId::new(CREDENTIAL).unwrap(),
        )
        .unwrap()
        .expose()
        .to_owned();
        let profiles = store.list_profiles().unwrap();
        let profile = store.load_profile(profiles[0].id).unwrap().unwrap();
        (
            row,
            credentials,
            secret,
            profile.actions[0].action_type.to_string(),
        )
    }

    /// Migrated through the user's consent; returns the running agent.
    async fn migrated(&self) -> Agent {
        let agent = self.start(&Start::consent()).await;
        self.wait_phase(&agent, MigrationPhase::CutOver).await;
        agent.wait_source(self.action, "plugin").await;
        agent
    }
}

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
    async fn client(&self) -> Option<AgentClient> {
        let deadline = Instant::now() + STARTUP;
        loop {
            match AgentClient::connect(&self.paths, ClientKind::Other).await {
                Ok(client) => return Some(client),
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(_) => return None,
            }
        }
    }

    async fn try_request(&self, request: Request) -> Option<Result<Response, ClientError>> {
        Some(self.client().await?.request(request).await)
    }

    async fn request(&self, request: Request) -> Result<Response, ClientError> {
        let what = request.name();
        tokio::time::timeout(Duration::from_secs(120), self.try_request(request))
            .await
            .unwrap_or_else(|_| panic!("{what} took too long"))
            .unwrap_or_else(|| panic!("the agent did not start\n{}", self.log()))
    }

    async fn run(&self, action: ActionInstanceId) -> Result<ActionResult, ClientError> {
        match self.request(Request::RunAction { action }).await? {
            Response::ActionResult { result } => Ok(result),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Runs `action` until the implementation `source` answers (the
    /// routing reaches the registry a moment after the step is recorded).
    async fn wait_source(&self, action: ActionInstanceId, source: &str) -> ActionResult {
        let deadline = Instant::now() + SETTLE;
        loop {
            let result = self.run(action).await;
            if let Ok(result) = &result
                && result.data.get("source").map(String::as_str) == Some(source)
            {
                return result.clone();
            }
            assert!(
                Instant::now() < deadline,
                "{source} never answered: {result:?}\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Runs `action` until it is unavailable; returns why.
    async fn wait_unavailable(&self, action: ActionInstanceId) -> String {
        let deadline = Instant::now() + SETTLE;
        loop {
            let why = match self.run(action).await {
                Err(ClientError::Agent(error)) => Some(error.message),
                Ok(result) if result.status == ActionStatus::Failed => Some(result.message),
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
        match self.request(Request::ListPlugins).await {
            Ok(Response::Plugins { plugins }) => plugins
                .into_iter()
                .find(|p| p.id == PLUGIN)
                .map(|p| p.state),
            Ok(other) => panic!("unexpected {other:?}"),
            // Plugins off.
            Err(_) => None,
        }
    }

    async fn plugin_origin(&self) -> Option<PluginOrigin> {
        match self.request(Request::ListPlugins).await {
            Ok(Response::Plugins { plugins }) => plugins
                .into_iter()
                .find(|p| p.id == PLUGIN)
                .map(|p| p.origin),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The fixture module's migration, as the app lists it.
    async fn listed(&self) -> ModuleMigrationInfo {
        match self.request(Request::ListModuleMigrations).await {
            Ok(Response::ModuleMigrations { migrations }) => {
                let json = serde_json::to_string(&migrations).unwrap();
                assert!(!json.contains(TOKEN), "a secret in {json}");
                migrations
                    .into_iter()
                    .find(|m| m.module == MODULE)
                    .unwrap_or_else(|| panic!("no migration of {MODULE} in {json}"))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Waits until the listed migration is in `phase`.
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

    /// `decideModuleMigration`; the answer, never with a secret.
    async fn decide(
        &self,
        approve: &[&str],
        decline: &[&str],
    ) -> Result<ModuleMigrationInfo, ClientError> {
        let answer = self
            .request(Request::DecideModuleMigration {
                module: MODULE.into(),
                approve: approve.iter().map(|k| (*k).to_owned()).collect(),
                decline: decline.iter().map(|k| (*k).to_owned()).collect(),
            })
            .await?;
        match answer {
            Response::ModuleMigration { migration } => {
                assert!(!serde_json::to_string(&migration).unwrap().contains(TOKEN));
                Ok(*migration)
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// `retryModuleMigration`.
    async fn retry(&self) -> Result<ModuleMigrationInfo, ClientError> {
        match self
            .request(Request::RetryModuleMigration {
                module: MODULE.into(),
            })
            .await?
        {
            Response::ModuleMigration { migration } => Ok(*migration),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// A connection subscribed to plugin events, with or without
    /// migration events.
    async fn events(&self, migrations: bool) -> AgentClient {
        let mut client = self.client().await.expect("the agent answers");
        client
            .subscribe_plugin_events_with(migrations)
            .await
            .unwrap();
        client
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

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

/// Waits for the fixture module's migration event with `phase`.
async fn wait_event(client: &mut AgentClient, phase: ipc::MigrationPhase) {
    let deadline = Instant::now() + SETTLE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = tokio::time::timeout(left, client.next_plugin_event())
            .await
            .unwrap_or_else(|_| panic!("no migration event with {phase:?}"))
            .unwrap();
        if let PluginEvent::MigrationChanged {
            module,
            plugin,
            phase: now,
        } = &event
            && module == MODULE
            && *now == phase
        {
            assert_eq!(plugin, PLUGIN);
            return;
        }
    }
}

/// The plugin events a connection receives within `window`.
async fn events_within(client: &mut AgentClient, window: Duration) -> Vec<PluginEvent> {
    let deadline = Instant::now() + window;
    let mut events = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout(
        deadline.saturating_duration_since(Instant::now()),
        client.next_plugin_event(),
    )
    .await
    {
        events.push(event);
    }
    events
}

fn reason_of(error: &ClientError) -> Option<&str> {
    match error {
        ClientError::Agent(error) => error.reason.as_deref(),
        _ => None,
    }
}

// ---------------------------------------------------------------- tests

/// The app's consent dialog through the real requests: what waits is
/// listed by name (the credential's id, never its secret), the plugin
/// shows as bundled, approving the item imports the token and cuts over,
/// and the change arrives as an event only on connections that asked for
/// migration events.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_app_lists_and_approves_the_consent() {
    let env = Env::new("plugins-migration-ipc-approve");
    let agent = env.start(&Start::default()).await;
    let waiting = agent
        .wait_listed(ipc::MigrationPhase::AwaitingConsent)
        .await;
    assert_eq!(waiting.module_name, "Legacy Fixture");
    assert_eq!(waiting.plugin, PLUGIN);
    assert_eq!(waiting.plugin_name.as_deref(), Some("Legacy Fixture"));
    assert_eq!(waiting.route, MigrationRoute::Builtin);
    assert_eq!(waiting.max_attempts, 3);
    assert!(!waiting.can_retry);
    assert_eq!(
        waiting.aliases,
        [ipc::ModuleActionAlias {
            legacy: ACTION.into(),
            target: "deckpilot.legacy-fixture.whoami".into(),
        }]
    );
    let [item] = waiting.consent.as_slice() else {
        panic!("one item waits: {waiting:?}");
    };
    assert_eq!(item.key, "credentials:token");
    assert_eq!(item.decision, ConsentDecision::Pending);
    assert_eq!(item.permission.tier, ipc::PermissionTier::High);
    assert_eq!(
        item.import,
        Some(ipc::CredentialImport {
            setting: "credential".into(),
            credential: CREDENTIAL.into(),
            slot: "token".into(),
            hosts: vec!["api.example.com".into()],
            user_origin: None,
        })
    );
    assert!(item.why.contains(CREDENTIAL), "{}", item.why);
    assert_eq!(agent.plugin_origin().await, Some(PluginOrigin::Bundled));

    let mut with = agent.events(true).await;
    let mut without = agent.events(false).await;

    // Only what waits can be decided, and Retry is for failed ones.
    let refused = agent
        .decide(&["network:api.example.com"], &[])
        .await
        .unwrap_err();
    assert_eq!(
        reason_of(&refused),
        Some("unknownPermission"),
        "{refused:?}"
    );
    let refused = agent.retry().await.unwrap_err();
    assert_eq!(reason_of(&refused), Some("invalidState"), "{refused:?}");
    assert_eq!(
        env.migration().unwrap().phase,
        MigrationPhase::AwaitingConsent
    );
    assert_eq!(
        env.plugin_store()
            .credentials(&PluginId::parse(PLUGIN).unwrap())
            .unwrap(),
        []
    );

    let done = agent.decide(&["credentials:token"], &[]).await.unwrap();
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    assert_eq!(done.route, MigrationRoute::Plugin);
    assert_eq!(done.consent, []);
    wait_event(&mut with, ipc::MigrationPhase::CutOver).await;
    let plain = events_within(&mut without, Duration::from_secs(2)).await;
    assert!(
        plain
            .iter()
            .any(|e| matches!(e, PluginEvent::Changed { plugin, .. } if plugin == PLUGIN)),
        "{plain:?}"
    );
    assert!(
        !plain
            .iter()
            .any(|e| matches!(e, PluginEvent::MigrationChanged { .. })),
        "{plain:?}"
    );
    let result = agent.wait_source(env.action, "plugin").await;
    assert_eq!(result.message, "Hi, octo");
    assert!(
        env.http_log()
            .contains(&format!("authorization: Bearer {TOKEN}")),
        "{}",
        env.http_log()
    );
    assert!(!agent.log().contains(TOKEN), "the token in the agent's log");
    agent.stop().await;
}

/// The user declines the import: nothing is granted or copied, the
/// built-in keeps serving across restarts without asking again, and Retry
/// asks again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_declined_consent_keeps_the_builtin_until_retry() {
    let env = Env::new("plugins-migration-ipc-decline");
    let id = PluginId::parse(PLUGIN).unwrap();
    let agent = env.start(&Start::default()).await;
    agent
        .wait_listed(ipc::MigrationPhase::AwaitingConsent)
        .await;
    let mut events = agent.events(true).await;
    let declined = agent.decide(&[], &["credentials:token"]).await.unwrap();
    assert_eq!(
        declined.phase,
        ipc::MigrationPhase::NotEligible,
        "{declined:?}"
    );
    assert!(declined.can_retry);
    assert!(
        declined
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("you declined credentials:token")),
        "{declined:?}"
    );
    assert_eq!(declined.consent.len(), 1);
    assert_eq!(declined.consent[0].decision, ConsentDecision::Declined);
    wait_event(&mut events, ipc::MigrationPhase::NotEligible).await;
    let result = agent.wait_source(env.action, "builtin").await;
    assert_eq!(result.data["who"], "token-holder");
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    assert_eq!(agent.plugin_state().await, Some(PluginState::Disabled));
    agent.stop().await;

    // A restart does not ask again.
    let agent = env.start(&Start::default()).await;
    agent.wait_source(env.action, "builtin").await;
    let info = agent.listed().await;
    assert_eq!(info.phase, ipc::MigrationPhase::NotEligible, "{info:?}");
    assert_eq!(info.consent[0].decision, ConsentDecision::Declined);

    // Retry asks again; nothing was granted meanwhile.
    let again = agent.retry().await.unwrap();
    assert_eq!(
        again.phase,
        ipc::MigrationPhase::AwaitingConsent,
        "{again:?}"
    );
    assert_eq!(again.consent[0].decision, ConsentDecision::Pending);
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    let done = agent.decide(&["credentials:token"], &[]).await.unwrap();
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    let result = agent.wait_source(env.action, "plugin").await;
    assert_eq!(result.message, "Hi, octo");
    agent.stop().await;
}

/// An alpha.3 configuration migrates: settings copied, grants derived per
/// policy, the credential imported only after consent, and the legacy
/// action id runs in the plugin. `deckpilot.db` is unchanged, so an older
/// app and a build without plugins keep the built-in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_alpha3_configuration_migrates_after_consent() {
    let env = Env::new("plugins-migration-alpha3");
    let before = env.legacy_view();

    // First start: installed and prepared, waiting for the user.
    let agent = env.start(&Start::default()).await;
    let migration = env
        .wait_phase(&agent, MigrationPhase::AwaitingConsent)
        .await;
    assert!(
        migration.reason.as_deref().unwrap().contains(CREDENTIAL),
        "{migration:?}"
    );
    assert_eq!(agent.plugin_state().await, Some(PluginState::Disabled));
    let store = env.plugin_store();
    let id = PluginId::parse(PLUGIN).unwrap();
    let granted: Vec<(String, String)> = store
        .grants(&id)
        .unwrap()
        .into_iter()
        .map(|g| (g.key.to_string(), g.decided_by))
        .collect();
    assert_eq!(
        granted,
        [
            (
                "network:api.example.com".into(),
                "migration:legacy-fixture".into()
            ),
            ("notifications".into(), "migration:legacy-fixture".into()),
        ]
    );
    assert_eq!(store.credentials(&id).unwrap(), [], "nothing imported yet");
    assert_eq!(store.settings(&id).unwrap()["greeting"], json!("Hi"));
    // The built-in serves meanwhile.
    let result = agent.wait_source(env.action, "builtin").await;
    assert_eq!(result.data["who"], "token-holder");
    assert_eq!(env.http_log(), "", "the plugin never ran");
    agent.stop().await;

    // The user consents: the token is imported, the legacy id runs in the
    // plugin with the copied greeting and the imported token.
    let agent = env.migrated().await;
    let result = agent.wait_source(env.action, "plugin").await;
    assert_eq!(result.message, "Hi, octo");
    assert_eq!(result.data["who"], "octo");
    let log = env.http_log();
    assert!(
        log.contains(&format!(
            "https://api.example.com/whoami\tauthorization: Bearer {TOKEN}"
        )),
        "{log}"
    );
    assert_eq!(agent.plugin_state().await, Some(PluginState::Enabled));
    agent.stop().await;

    // An older app sees exactly what it wrote: the row, the credential and
    // the built-in id in the profile.
    assert_eq!(env.legacy_view(), before);
    assert_eq!(before.3, ACTION);

    // A downgrade: a build without plugins, and one without the plugin,
    // run the built-in.
    let agent = env
        .start(&Start {
            plugins_off: true,
            ..Start::default()
        })
        .await;
    agent.wait_source(env.action, "builtin").await;
    agent.stop().await;
    let agent = env
        .start(&Start {
            unbundled: true,
            ..Start::default()
        })
        .await;
    agent.wait_source(env.action, "builtin").await;
    agent.stop().await;
    // And upgrading again routes to the plugin, with nothing to redo.
    let agent = env.start(&Start::default()).await;
    agent.wait_source(env.action, "plugin").await;
    assert_eq!(env.migration().unwrap().phase, MigrationPhase::CutOver);
    agent.stop().await;
    assert_eq!(env.legacy_view(), before);
}

/// The agent ends after each step (and inside the install): the next
/// start resumes from the journal and finishes once, idempotently.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restarts_mid_migration_resume() {
    let crashes = [
        ("DECKPILOT_TEST_MIGRATION_EXIT", "pending"),
        ("DECKPILOT_TEST_MIGRATION_EXIT", "installed"),
        ("DECKPILOT_TEST_MIGRATION_EXIT", "settings_copied"),
        ("DECKPILOT_TEST_PLUGIN_CRASH", "install:files_unpacked"),
    ];
    for (name, value) in crashes {
        let env = Env::new("plugins-migration-resume");
        env.start_until_exit(&Start {
            env: vec![(name, value.to_owned())],
            ..Start::consent()
        })
        .await;
        let agent = env.migrated().await;
        let result = agent.wait_source(env.action, "plugin").await;
        assert_eq!(result.message, "Hi, octo", "{value}");
        let store = env.plugin_store();
        let id = PluginId::parse(PLUGIN).unwrap();
        let record = store.plugin(&id).unwrap().unwrap();
        assert_eq!(record.versions.len(), 1, "{value}: {record:?}");
        assert_eq!(store.journal().unwrap(), [], "{value}");
        assert_eq!(store.credentials(&id).unwrap().len(), 1, "{value}");
        assert_eq!(env.migration().unwrap().attempts, 0, "{value}");
        agent.stop().await;
    }
}

/// A plugin that cannot replace the built-in (its action times out sooner)
/// fails its health check: the built-in serves, the failure is journaled,
/// retried at the next starts up to three times, then only on Retry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_health_check_falls_back_and_retries_are_limited() {
    let env = Env::new("plugins-migration-health");
    env.stage("0.1.0", |manifest| {
        manifest["contributes"]["actions"][0]["node"]["timeoutMs"] = json!(1000);
    });
    let agent = env.start(&Start::consent()).await;
    let migration = env.wait_phase(&agent, MigrationPhase::Failed).await;
    assert!(
        migration
            .reason
            .as_deref()
            .unwrap()
            .starts_with("health check:"),
        "{migration:?}"
    );
    assert_eq!(migration.attempts, 1);
    agent.wait_source(env.action, "builtin").await;
    agent.stop().await;

    for attempts in 2..=3 {
        let agent = env.start(&Start::default()).await;
        let deadline = Instant::now() + SETTLE;
        while env.migration().unwrap().attempts < attempts {
            assert!(Instant::now() < deadline, "{}", agent.log());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        agent.wait_source(env.action, "builtin").await;
        agent.stop().await;
    }

    // Fixed in an app update: still no automatic attempt…
    env.stage("0.1.1", |_| {});
    let exhausted = env.migration().unwrap();
    let agent = env.start(&Start::default()).await;
    agent.wait_source(env.action, "builtin").await;
    agent.stop().await;
    assert_eq!(env.migration().unwrap(), exhausted);

    // …until the user presses Retry in the app.
    let agent = env.start(&Start::default()).await;
    agent.wait_source(env.action, "builtin").await;
    let failed = agent.listed().await;
    assert_eq!(failed.phase, ipc::MigrationPhase::Failed, "{failed:?}");
    assert_eq!((failed.attempts, failed.max_attempts), (3, 3));
    assert!(failed.can_retry);
    assert!(
        failed
            .reason
            .as_deref()
            .is_some_and(|r| r.starts_with("health check:")),
        "{failed:?}"
    );
    let mut events = agent.events(true).await;
    // The consent was given before; Retry runs every step again.
    let retried = agent.retry().await.unwrap();
    assert_eq!(retried.phase, ipc::MigrationPhase::CutOver, "{retried:?}");
    assert_eq!(retried.attempts, 0);
    wait_event(&mut events, ipc::MigrationPhase::CutOver).await;
    let result = agent.wait_source(env.action, "plugin").await;
    assert_eq!(result.message, "Hi, octo");
    assert_eq!(env.migration().unwrap().attempts, 0);
    agent.stop().await;
}

/// An older app changed the built-in's settings after the migration: the
/// plugin's settings are copied again while the user left them alone, and
/// kept, with a note, once the user changed them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drift_is_reconciled() {
    let env = Env::new("plugins-migration-drift");
    env.migrated().await.stop().await;

    env.save_greeting("Hey");
    let agent = env.start(&Start::default()).await;
    let deadline = Instant::now() + SETTLE;
    loop {
        let result = agent.wait_source(env.action, "plugin").await;
        if result.message == "Hey, octo" {
            break;
        }
        assert!(Instant::now() < deadline, "{result:?}\n{}", agent.log());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        env.migration()
            .unwrap()
            .reason
            .unwrap()
            .contains("copied again"),
        "{:?}",
        env.migration()
    );
    // The user sets the plugin's greeting.
    agent
        .request(Request::SetPluginSettings {
            plugin: PLUGIN.into(),
            values: [("greeting".to_owned(), json!("Yo"))].into(),
        })
        .await
        .unwrap();
    agent.stop().await;

    env.save_greeting("Howdy");
    let agent = env.start(&Start::default()).await;
    let deadline = Instant::now() + SETTLE;
    while !env
        .migration()
        .unwrap()
        .reason
        .is_some_and(|r| r.contains("were kept"))
    {
        assert!(
            Instant::now() < deadline,
            "{:?}\n{}",
            env.migration(),
            agent.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let result = agent.wait_source(env.action, "plugin").await;
    assert_eq!(result.message, "Yo, octo");
    agent.stop().await;
}

/// The user disables, then uninstalls the plugin: the legacy actions are
/// unavailable, never silently the built-in's, across restarts and an app
/// update with a newer bundled copy, which is not installed again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_users_disable_and_uninstall_are_respected() {
    let env = Env::new("plugins-migration-withdrawn");
    let agent = env.migrated().await;
    agent
        .request(Request::DisablePlugin {
            plugin: PLUGIN.into(),
        })
        .await
        .unwrap();
    let why = agent.wait_unavailable(env.action).await;
    assert!(why.contains("disabled"), "{why}");
    // The app shows why the legacy actions are unavailable.
    let info = agent.listed().await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    assert_eq!(
        info.route_reason.as_deref(),
        Some("Legacy Fixture (deckpilot.legacy-fixture) was disabled by you")
    );
    assert!(!info.can_retry);
    agent.stop().await;

    let agent = env.start(&Start::default()).await;
    // The agent routed it before the first request is answered? Not
    // necessarily: wait for the routing, then check it never ran.
    let why = agent.wait_unavailable(env.action).await;
    assert!(why.contains("disabled"), "{why}");
    assert_eq!(agent.plugin_state().await, Some(PluginState::Disabled));
    agent
        .request(Request::UninstallPlugin {
            plugin: PLUGIN.into(),
            keep: deckpilot_ipc::plugins::PluginRetention::default(),
        })
        .await
        .unwrap();
    agent.wait_unavailable(env.action).await;
    agent.stop().await;

    // An app update bundles a newer copy.
    env.stage("0.2.0", |_| {});
    for _ in 0..2 {
        let agent = env.start(&Start::consent()).await;
        agent.wait_unavailable(env.action).await;
        assert_eq!(agent.plugin_state().await, None, "{}", agent.log());
        let result = agent.run(env.action).await;
        assert!(
            !matches!(&result, Ok(r) if r.data.contains_key("source")),
            "{result:?}"
        );
        agent.stop().await;
    }
    let store = env.plugin_store();
    let id = PluginId::parse(PLUGIN).unwrap();
    assert_eq!(
        store.tombstone(&id).unwrap().unwrap().decision,
        TombstoneDecision::Uninstalled
    );
    assert_eq!(env.migration().unwrap().phase, MigrationPhase::CutOver);
    // Nothing of the user's was deleted from deckpilot.db.
    assert_eq!(env.legacy_view().2, TOKEN);
}
