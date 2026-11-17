//! The GitLab and Jira cut-overs (`docs/marketplace/module-migration.md`
//! §9, P11) end to end, through the real agent, the real sandboxed plugin
//! host, real storage and IPC, each test in its own data directory
//! (feature `plugins`):
//!
//! * an alpha.3-style `deckpilot.db`: the built-in module saved and
//!   enabled with **the server the user entered** (a self-managed GitLab
//!   under a path, a Jira Cloud site), its credential stored, a profile
//!   with an MK.2 key bound to a legacy action and a workflow node running
//!   another;
//! * the shipped alias blocks (`ACTION_ALIASES`) and migration plans, and
//!   `plugins/first-party/{gitlab,jira}` built to wasm and bundled as the
//!   release bundles them (class `migration`, pinned with
//!   `DECKPILOT_TEST_BUNDLED_PINS`);
//! * no server ever answers: the plugin's requests get canned answers
//!   (`DECKPILOT_TEST_PLUGIN_HTTP`, which logs the credential header the
//!   agent injected), and the built-in's go to a stand-in on 127.0.0.1
//!   that records them (`DECKPILOT_TEST_BUILTIN_GITLAB_API`,
//!   `DECKPILOT_TEST_BUILTIN_JIRA_SITE`, feature `test-hooks`: the
//!   module's stored settings, and so what the migration carries, are
//!   untouched). Which of the two got a request says which implementation
//!   ran;
//! * the consent to the credential, **to the server** and, separately, to
//!   a private network, Retry, disable and uninstall come through the
//!   app's requests.
#![cfg(feature = "plugins")]

#[path = "support/parity.rs"]
mod parity;
mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use deckpilot_core::{
    ActionInstance, ActionInstanceId, ActionParameters, ActionStatus, ActionTypeId, BindingTarget,
    ControlBinding, CredentialId, DeviceLayout, ExecutionState, ModuleId, ModuleSettings,
    ParameterValue, Profile, Secret, Trigger, WorkflowId,
};
use deckpilot_device::{ControlId, ControlInput, DeviceId};
use deckpilot_ipc::plugins::{
    self as ipc, ConsentDecision, MigrationRoute, ModuleMigrationInfo, PluginState,
};
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_plugin_store::{PluginStore, TombstoneDecision};
use deckpilot_storage::{ConfigStore, DatabaseKey};
use parity::{Answer, Server, json_answer};
use serde_json::{Value, json};
use support::TempDir;

const STARTUP: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(60);

/// A legacy action of the alpha.3 profile: its type, its parameters, and
/// the path (below the server) its first request asks for.
struct Use {
    name: &'static str,
    action: &'static str,
    parameters: fn(&Module) -> ActionParameters,
    asks: &'static str,
}

/// One module that moves to its plugin.
struct Module {
    id: &'static str,
    name: &'static str,
    plugin: parity::Plugin,
    /// The stored credential's id and secret, and the plugin's slot.
    credential: &'static str,
    secret: &'static str,
    slot: &'static str,
    /// The module's setting with the server, and the stand-in's variable.
    setting: &'static str,
    stand_in: &'static str,
    /// The rest of what the alpha.3 user saved.
    settings: fn(&Module) -> ActionParameters,
    /// On the key, and in the workflow's node.
    key: Use,
    node: Use,
    answers: parity::Answers,
    /// The request header with the credential, and its value.
    header: &'static str,
    sent: fn(&Module) -> String,
    /// What the plugin's log shows after the URL.
    logged: fn(&Module) -> String,
    /// The plugin's declared host and web host, if it has one.
    host: Option<&'static str>,
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(value.into())
}

// ----------------------------------------------------------------- GitLab

fn gitlab_answers(base: &str, target: &str) -> Answer {
    let path = target.split('?').next().unwrap();
    let web = base.strip_suffix("/api/v4").unwrap_or(base);
    match path {
        "/projects/group%2Fapp/merge_requests" => json_answer(
            200,
            &json!([
                {"iid": 12, "title": "Fix the build", "draft": false,
                 "references": {"full": "group/app!12"}, "author": {"username": "dev"},
                 "web_url": format!("{web}/group/app/-/merge_requests/12")},
                {"iid": 11, "title": "Docs", "draft": true,
                 "references": {"full": "group/app!11"}, "author": {"username": "doc"},
                 "web_url": format!("{web}/group/app/-/merge_requests/11")}
            ]),
        )
        .header("x-total", "2"),
        "/projects/group%2Fapp/pipelines" => json_answer(
            200,
            &json!([{"id": 77, "status": "success", "sha": "0123abc"}]),
        ),
        "/projects/group%2Fapp/pipelines/77/jobs" => json_answer(
            200,
            &json!([
                {"name": "build", "status": "success", "allow_failure": false},
                {"name": "lint", "status": "failed", "allow_failure": false}
            ]),
        )
        .header("x-total", "2"),
        _ => json_answer(404, &json!({"message": "404 Not Found"})),
    }
}

const GITLAB: Module = Module {
    id: "gitlab",
    name: "GitLab",
    plugin: parity::Plugin {
        id: "deckpilot.gitlab",
        dir: "gitlab",
        package: "deckpilot-gitlab",
    },
    credential: "gitlab.token",
    secret: "glpat-cutover-4b8d1e-not-a-real-token",
    slot: "token",
    setting: "apiUrl",
    stand_in: "DECKPILOT_TEST_BUILTIN_GITLAB_API",
    settings: |m| {
        ActionParameters::new()
            .with("credential", text(m.credential))
            .with("timeoutSeconds", ParameterValue::Integer(30))
    },
    key: Use {
        name: "Open merge requests",
        action: "gitlab.merge-requests",
        parameters: |_| {
            ActionParameters::new()
                .with("project", text("group/app"))
                .with("credential", text(""))
        },
        asks: "/projects/group%2Fapp/merge_requests",
    },
    node: Use {
        name: "Pipeline of main",
        action: "gitlab.pipeline",
        parameters: |m| {
            ActionParameters::new()
                .with("project", text("group/app"))
                .with("ref", text("main"))
                .with("credential", text(m.credential))
        },
        asks: "/projects/group%2Fapp/pipelines",
    },
    answers: gitlab_answers,
    header: "private-token",
    sent: |m| m.secret.to_owned(),
    logged: |m| format!("\tPRIVATE-TOKEN: {}", m.secret),
    host: Some("gitlab.com"),
};

/// A self-managed GitLab under a path.
const OWN_GITLAB: &str = "https://code.example.com/gitlab/api/v4";

// ------------------------------------------------------------------- Jira

fn jira_answers(_site: &str, target: &str) -> Answer {
    let path = target.split('?').next().unwrap();
    match path {
        "/rest/api/3/search/jql" => json_answer(
            200,
            &json!({"isLast": true, "issues": [
                {"key": "WEB-12", "fields": {"summary": "Fix the login page",
                 "status": {"name": "In Progress"}, "assignee": {"displayName": "Dev"}}},
                {"key": "WEB-11", "fields": {"summary": "Docs", "status": {"name": "To Do"},
                 "assignee": null}}
            ]}),
        ),
        "/rest/api/3/issue/WEB-12" => json_answer(
            200,
            &json!({"key": "WEB-12", "fields": {"summary": "Fix the login page",
                    "status": {"name": "In Progress",
                               "statusCategory": {"key": "indeterminate"}}}}),
        ),
        _ => json_answer(404, &json!({"errorMessages": ["Not found"]})),
    }
}

const JIRA: Module = Module {
    id: "jira",
    name: "Jira",
    plugin: parity::Plugin {
        id: "deckpilot.jira",
        dir: "jira",
        package: "deckpilot-jira",
    },
    credential: "jira.token",
    secret: "dev@example.com:ATATT-cutover-9f2a-not-a-real-token",
    slot: "credential",
    setting: "siteUrl",
    stand_in: "DECKPILOT_TEST_BUILTIN_JIRA_SITE",
    settings: |m| {
        ActionParameters::new()
            .with("deployment", ParameterValue::Choice("cloud".into()))
            .with("credential", text(m.credential))
            .with("timeoutSeconds", ParameterValue::Integer(30))
    },
    key: Use {
        name: "My issues",
        action: "jira.issues",
        parameters: |_| ActionParameters::new().with("credential", text("")),
        asks: "/rest/api/3/search/jql",
    },
    node: Use {
        name: "The login issue",
        action: "jira.issue",
        parameters: |m| {
            ActionParameters::new()
                .with("issue", text("WEB-12"))
                .with("credential", text(m.credential))
        },
        asks: "/rest/api/3/issue/WEB-12",
    },
    answers: jira_answers,
    header: "authorization",
    sent: |m| {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(m.secret)
        )
    },
    logged: |m| format!("\tauthorization: Basic {}", m.secret),
    host: None,
};

/// A Jira Cloud site.
const OWN_JIRA: &str = "https://team.atlassian.net";

// ---------------------------------------------------------- environment

/// How one agent start is set up.
#[derive(Default, Clone, Copy)]
struct Start {
    /// Plugins off (`--enable-plugins` not given): an older build, or a
    /// release build today.
    plugins_off: bool,
}

/// What an alpha.3 app saved, read raw from `deckpilot.db`: the
/// `modules` row, the `credentials` rows and the profile documents.
#[derive(Debug, PartialEq, Eq)]
struct Legacy {
    module: Option<(i64, String)>,
    credentials: Vec<(String, String, String, i64)>,
    profiles: Vec<(String, String, i64, String, i64)>,
}

struct Env {
    module: &'static Module,
    /// The server the plugin talks to (the user's, or the default).
    server: String,
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
    stand_in: Server,
    on_key: ActionInstanceId,
    workflow: WorkflowId,
}

/// The path of an `https` server (`/gitlab/api/v4`, or nothing).
fn path_of(server: &str) -> &str {
    server
        .strip_prefix("https://")
        .and_then(|rest| rest.find('/').map(|at| &rest[at..]))
        .unwrap_or("")
}

impl Env {
    /// An alpha.3 configuration of `module`: `saved` as its server setting
    /// (`None`: never saved), `private` as its private-network switch,
    /// and, with `used`, its credential and a profile that uses it. The
    /// plugin talks to `server`.
    fn new(
        name: &str,
        module: &'static Module,
        saved: Option<&str>,
        server: &str,
        private: bool,
        used: bool,
    ) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let mut env = Self {
            module,
            server: server.to_owned(),
            dir,
            paths,
            key,
            stand_in: Server::start(path_of(server), module.answers, module.header),
            on_key: ActionInstanceId::new(),
            workflow: WorkflowId::new(),
        };
        if used {
            env.write_alpha3(saved, private);
        }
        env.stage();
        std::fs::write(
            env.http_file(),
            serde_json::to_vec_pretty(&env.plugin_routes()).unwrap(),
        )
        .unwrap();
        env
    }

    /// The plugin's canned answers at its server: what the key and the
    /// node ask for (the longest matching prefix first).
    fn plugin_routes(&self) -> Vec<Value> {
        let mut targets = vec![
            self.module.key.asks.to_owned(),
            self.module.node.asks.to_owned(),
        ];
        if self.module.id == "gitlab" {
            targets.push("/projects/group%2Fapp/pipelines/77/jobs".into());
        }
        targets.sort_by_key(|t| std::cmp::Reverse(t.len()));
        targets
            .iter()
            .map(|target| {
                let a = (self.module.answers)(&self.server, target);
                json!({"url": format!("{}{target}", self.server), "status": a.status,
                       "headers": a.headers, "body": a.body})
            })
            .collect()
    }

    /// The built-in module saved and enabled, its credential stored, and
    /// a profile with a key bound to one legacy action and a workflow
    /// node running another.
    fn write_alpha3(&mut self, saved: Option<&str>, private: bool) {
        let m = self.module;
        let store = support::open_store(&self.paths, &self.key);
        store
            .set_credential(
                &CredentialId::new(m.credential).unwrap(),
                "The module's credential",
                Some(&Secret::new(m.secret)),
            )
            .unwrap();
        let mut config = (m.settings)(m);
        if let Some(saved) = saved {
            config = config.with(m.setting, text(saved));
        }
        if private {
            config = config.with("allowPrivateHost", ParameterValue::Boolean(true));
        }
        store
            .set_module_state(
                &ModuleId::new(m.id).unwrap(),
                &ModuleSettings {
                    enabled: true,
                    config,
                },
            )
            .unwrap();
        let mut profile = Profile::new("Work");
        let instance = |id, used: &Use| ActionInstance {
            id,
            name: used.name.into(),
            action_type: ActionTypeId::new(used.action).unwrap(),
            parameters: (used.parameters)(m),
            description: String::new(),
            icon: None,
            label: None,
        };
        profile.actions.push(instance(self.on_key, &m.key));
        let in_node = ActionInstanceId::new();
        profile.actions.push(instance(in_node, &m.node));
        let mut layout = DeviceLayout::new("Main", deckpilot_device::models::stream_deck_mk2().id);
        layout.pages[0].bindings = vec![ControlBinding::new(
            ControlId::Key { index: 0 },
            Trigger::Press,
            BindingTarget::action(self.on_key),
        )];
        profile.layouts.push(layout);
        let workflow = support::graph_workflow(
            "Node",
            support::graph(
                &json!([
                    {"id": "start", "type": "trigger"},
                    {"id": "node", "type": "action", "action": in_node},
                ]),
                &json!([{"from": "start", "to": "node"}]),
            ),
        );
        self.workflow = workflow.id;
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
        self.dir.path().join("plugin-http.json")
    }

    /// What the plugin sent: one line per request, with the credential
    /// header the agent injected.
    fn plugin_log(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("plugin-http.json.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// `bundled/<id>/` as `plugins/first-party/build.sh` stages it for a
    /// release (class `migration`, `bundled.json`), and the pins the agent
    /// "was compiled with".
    fn stage(&self) {
        let plugin = &self.module.plugin;
        let source = parity::repo().join("plugins/first-party").join(plugin.dir);
        let dir = self.bundled().join(plugin.id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for file in ["manifest.json", "LICENSE", "assets/icon-128.png"] {
            std::fs::copy(source.join(file), dir.join(file)).unwrap();
        }
        std::fs::copy(&parity::artifacts(plugin).module, dir.join("plugin.wasm")).unwrap();
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let version = semver::Version::parse(manifest["version"].as_str().unwrap()).unwrap();
        let released: Value = serde_json::from_slice(
            &std::fs::read(parity::repo().join("plugins/first-party/bundled.json")).unwrap(),
        )
        .unwrap();
        assert!(
            released["plugins"]
                .as_array()
                .unwrap()
                .contains(&json!({"path": plugin.dir, "class": "migration"})),
            "the release bundles {} as a migration-class plugin: {released}",
            plugin.id
        );
        let pins = Pins::compute(
            &self.bundled(),
            &[(
                PluginId::parse(plugin.id).unwrap(),
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
            .arg(&parity::artifacts(&self.module.plugin).host)
            .arg("--bundled-plugins")
            .arg(self.bundled())
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_BUNDLED_PINS", self.pins_file())
            .env("DECKPILOT_TEST_PLUGIN_HTTP", self.http_file())
            .env(
                "DECKPILOT_TEST_PLUGIN_OPEN_URL",
                self.dir.path().join("opened.txt"),
            )
            .env(self.module.stand_in, self.stand_in.base())
            .env_remove("DECKPILOT_TEST_BUILTIN_GITHUB_API")
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
        if !start.plugins_off {
            command.arg("--enable-plugins");
        }
        let agent = Agent {
            module: self.module,
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

    fn plugin_id(&self) -> PluginId {
        PluginId::parse(self.module.plugin.id).unwrap()
    }

    /// The consents to servers the user entered: the server, whether on a
    /// private network, and who decided.
    fn allowed_servers(&self) -> Vec<(String, bool, String)> {
        self.plugin_store()
            .user_origins(&self.plugin_id())
            .unwrap()
            .into_iter()
            .map(|row| {
                (
                    row.consent.display(),
                    row.consent.private_network,
                    row.decided_by,
                )
            })
            .collect()
    }

    /// What an older app reads for the module, byte for byte.
    fn legacy(&self) -> Legacy {
        let store = support::open_store(&self.paths, &self.key);
        let db = store.connection_for_tests();
        let module = db
            .query_row(
                "SELECT enabled, config FROM modules WHERE id = ?1",
                [self.module.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
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

    fn traffic(&self) -> Traffic {
        Traffic {
            builtin: self.stand_in.seen(),
            plugin: self.plugin_log(),
        }
    }

    /// Which implementation answered the requests made since `before`.
    fn since(&self, before: &Traffic) -> Traffic {
        Traffic {
            builtin: self.stand_in.seen()[before.builtin.len()..].to_vec(),
            plugin: self.plugin_log()[before.plugin.len()..].to_vec(),
        }
    }

    /// Presses the key until `side` serves it (the routing reaches the
    /// registry a moment after the migration step is recorded); checks
    /// the requests it made. The other side must never see one.
    async fn press_until(&self, agent: &Agent, device: &DeviceId, side: Side) {
        let deadline = Instant::now() + SETTLE;
        loop {
            let before = self.traffic();
            let (name, status) = agent.press(device).await;
            let new = self.since(&before);
            assert_eq!(name, self.module.key.name);
            let served = match side {
                Side::Builtin => !new.builtin.is_empty(),
                Side::Plugin => !new.plugin.is_empty(),
            };
            if served {
                assert_eq!(status, ActionStatus::Succeeded, "{new:?}\n{}", agent.log());
                self.assert_side(&new, side, self.module.key.asks);
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{side:?} never served the key: {status:?}, {new:?}\n{}",
                agent.log()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Runs the workflow; its node ran on `side`.
    async fn run_node(&self, agent: &Agent, side: Side) {
        let before = self.traffic();
        let record = agent.run_workflow(self.workflow).await;
        assert_eq!(
            record.summary.state,
            ExecutionState::Succeeded,
            "{record:#?}\n{}",
            agent.log()
        );
        self.assert_side(&self.since(&before), side, self.module.node.asks);
    }

    /// Only `side` got requests: for `asks` first, each with the stored
    /// credential; the plugin's go to its server and nowhere else.
    fn assert_side(&self, traffic: &Traffic, side: Side, asks: &str) {
        let m = self.module;
        match side {
            Side::Builtin => {
                assert_eq!(traffic.plugin, Vec::<String>::new(), "{traffic:?}");
                assert!(traffic.builtin[0].starts_with(asks), "{traffic:?}");
                for line in &traffic.builtin {
                    assert!(!line.starts_with("OUTSIDE"), "{line}");
                    assert!(line.ends_with(&format!("\t{}", (m.sent)(m))), "{line}");
                }
            }
            Side::Plugin => {
                assert_eq!(traffic.builtin, Vec::<String>::new(), "{traffic:?}");
                assert!(
                    traffic.plugin[0].starts_with(&format!("{}{asks}", self.server)),
                    "{traffic:?}"
                );
                for line in &traffic.plugin {
                    assert!(line.starts_with(&format!("{}/", self.server)), "{line}");
                    assert!(line.ends_with(&(m.logged)(m)), "{line}");
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Builtin,
    Plugin,
}

/// Requests each implementation made: `<target>\t<credential>` for the
/// built-in, `<url>\t<credential header>` for the plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Traffic {
    builtin: Vec<String>,
    plugin: Vec<String>,
}

struct Agent {
    module: &'static Module,
    child: Child,
    paths: AgentPaths,
    log: PathBuf,
    /// Every answer the agent gave, to check that none holds the secret.
    answers: Mutex<Vec<String>>,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let answers = self.answers.lock().unwrap();
        if !std::thread::panicking() {
            // The whole secret, and the token of `email:token`.
            let token = self.module.secret.rsplit(':').next().unwrap();
            for answer in answers.iter() {
                assert!(!answer.contains(token), "the secret in an answer: {answer}");
            }
            assert!(!self.log().contains(token), "the secret in the agent's log");
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
                .find(|p| p.id == self.module.plugin.id)
                .map(|p| p.state),
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The module's migration, as the app lists it.
    async fn listed(&self) -> ModuleMigrationInfo {
        match self.ok(Request::ListModuleMigrations).await {
            Response::ModuleMigrations { migrations } => migrations
                .into_iter()
                .find(|m| m.module == self.module.id)
                .unwrap_or_else(|| panic!("no migration of {}\n{}", self.module.id, self.log())),
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
                module: self.module.id.into(),
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
                module: self.module.id.into(),
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

/// The keys of what waits for the user's decision.
fn waiting(info: &ModuleMigrationInfo) -> BTreeSet<String> {
    info.consent
        .iter()
        .filter(|i| i.decision == ConsentDecision::Pending)
        .map(|i| i.key.clone())
        .collect()
}

fn keys(items: &[String]) -> BTreeSet<String> {
    items.iter().cloned().collect()
}

// ---------------------------------------------------------------- tests

/// An alpha.3 user whose module talks to a server they entered upgrades
/// to a build with plugins: the bundled plugin is installed and pinned,
/// the settings and the server carried, and the credential **and the
/// server** wait for the user's consent while the built-in keeps
/// serving. Neither allows the other. Approved through IPC, the key and
/// the workflow node run in the plugin, at exactly that server, with the
/// imported credential. `deckpilot.db` is byte-identical, so a build
/// without plugins still runs the built-in. The user disabling the plugin
/// makes the legacy actions unavailable, with no fallback; uninstalling
/// it and upgrading again does not bring it back.
#[allow(clippy::too_many_lines)] // one upgrade, step by step
async fn upgrade(module: &'static Module, server: &'static str) {
    let env = Env::new(
        &format!("plugins-cutover-{}", module.id),
        module,
        Some(server),
        server,
        false,
        true,
    );
    let before = env.legacy();
    let id = env.plugin_id();
    let credential = format!("credentials:{}", module.slot);
    let slot = format!("network:user-origin:{}", module.setting);
    let pages = format!("openUrl:user-origin:{}", module.setting);

    // 1. The upgrade: installed from the bundle, settings and the server
    // carried, waiting.
    let agent = env.start(Start::default()).await;
    let asked = agent
        .wait_listed(ipc::MigrationPhase::AwaitingConsent)
        .await;
    eprintln!(
        "{} step 1: {:?}, route {:?}",
        module.id, asked.phase, asked.route
    );
    assert_eq!(asked.plugin, module.plugin.id);
    assert_eq!(asked.module_name, module.name);
    assert_eq!(asked.route, MigrationRoute::Builtin);
    assert!(
        asked
            .aliases
            .iter()
            .all(|a| a.target == format!("deckpilot.{}", a.legacy)),
        "{asked:?}"
    );
    // What waits: the credential and the server, always; and the declared
    // host the built-in did not talk to here (medium tier, not derived).
    let mut expected = vec![credential.clone(), slot.clone()];
    if let Some(host) = module.host {
        expected.push(format!("network:{host}"));
        expected.push(format!("openUrl:{host}"));
    }
    assert_eq!(waiting(&asked), keys(&expected), "{asked:?}");
    let item = |key: &str| asked.consent.iter().find(|i| i.key == key).unwrap();
    let import = item(&credential).import.as_ref().unwrap();
    assert_eq!(import.credential, module.credential);
    assert_eq!(import.slot, module.slot);
    assert_eq!(import.user_origin.as_deref(), Some(module.setting));
    let asked_server = item(&slot).origin.as_ref().unwrap();
    assert_eq!(asked_server.origin, server);
    assert_eq!(asked_server.setting, module.setting);
    assert_eq!(asked_server.credentials, [module.slot]);
    assert!(asked_server.opens_pages && !asked_server.private_network);
    assert_eq!(item(&slot).permission.tier, ipc::PermissionTier::High);
    assert_eq!(agent.plugin_state().await, Some(PluginState::Disabled));
    let store = env.plugin_store();
    let grants: Vec<(String, String)> = store
        .grants(&id)
        .unwrap()
        .into_iter()
        .map(|g| (g.key.to_string(), g.decided_by))
        .collect();
    assert_eq!(
        grants,
        [(pages.clone(), format!("migration:{}", module.id))],
        "derived: opening pages of the server, which opens nothing before the server is \
         allowed; never the server, the credential or a host the built-in did not use"
    );
    assert_eq!(store.credentials(&id).unwrap(), [], "nothing imported yet");
    let settings = store.settings(&id).unwrap();
    eprintln!("{} step 1: copied settings {settings:?}", module.id);
    assert_eq!(settings[module.setting], json!(server));
    assert_eq!(settings["timeoutSeconds"], json!(30));
    drop(store);
    assert_eq!(env.allowed_servers(), [], "no server allowed");

    // Meanwhile the built-in serves the key and the node.
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin).await;
    env.run_node(&agent, Side::Builtin).await;

    // 2. Everything but the server: the credential's consent does not
    // allow the server. The built-in keeps serving.
    let others: Vec<&str> = expected
        .iter()
        .filter(|k| **k != slot)
        .map(String::as_str)
        .collect();
    let partly = agent.decide(&others, &[]).await;
    assert_eq!(
        partly.phase,
        ipc::MigrationPhase::AwaitingConsent,
        "{partly:?}"
    );
    assert_eq!(waiting(&partly), keys(std::slice::from_ref(&slot)));
    assert_eq!(env.allowed_servers(), []);
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    env.press_until(&agent, &device, Side::Builtin).await;
    assert_eq!(env.plugin_log(), Vec::<String>::new(), "the plugin sent");

    // 3. The server: allowed, exactly it, by this decision.
    let done = agent.decide(&[&slot], &[]).await;
    eprintln!(
        "{} step 3: {:?}, route {:?}",
        module.id, done.phase, done.route
    );
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    assert_eq!(done.route, MigrationRoute::Plugin);
    assert_eq!(agent.plugin_state().await, Some(PluginState::Enabled));
    assert_eq!(
        env.allowed_servers(),
        [(
            server.to_owned(),
            false,
            format!("user:migration:{}", module.id)
        )]
    );
    assert_eq!(env.plugin_store().credentials(&id).unwrap().len(), 1);
    // The same key and node now run the plugin, at the user's server.
    env.press_until(&agent, &device, Side::Plugin).await;
    env.run_node(&agent, Side::Plugin).await;
    agent.stop().await;

    // 4. Nothing an older app reads changed, byte for byte.
    assert_eq!(env.legacy(), before);

    // 5. An older build (plugins off) on the same data: the built-in.
    let agent = env.start(Start { plugins_off: true }).await;
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin).await;
    env.run_node(&agent, Side::Builtin).await;
    agent.stop().await;

    // 6. Upgrading again: still the plugin, nothing asked again.
    let agent = env.start(Start::default()).await;
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Plugin).await;
    let info = agent.listed().await;
    assert_eq!(info.phase, ipc::MigrationPhase::CutOver, "{info:?}");
    assert_eq!(info.consent, []);

    // 7. The user disables the plugin: unavailable, no fallback.
    agent
        .ok(Request::DisablePlugin {
            plugin: module.plugin.id.into(),
        })
        .await;
    let why = agent.wait_unavailable(env.on_key).await;
    eprintln!("{} step 7: disabled: {why}", module.id);
    assert!(why.contains("disabled"), "{why}");
    let info = agent.listed().await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    let quiet = env.traffic();
    let (_, status) = agent.press(&device).await;
    assert_eq!(status, ActionStatus::Failed);
    let refused = agent
        .request(Request::StartWorkflow {
            workflow: env.workflow,
        })
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains(&format!("`{}`) is disabled", module.plugin.id)),
        "{refused}"
    );
    assert_eq!(
        env.since(&quiet),
        Traffic::default(),
        "neither implementation ran"
    );

    // 8. The user uninstalls it; an upgrade does not bring it back.
    agent
        .ok(Request::UninstallPlugin {
            plugin: module.plugin.id.into(),
            keep: deckpilot_ipc::plugins::PluginRetention::default(),
        })
        .await;
    agent.wait_unavailable(env.on_key).await;
    agent.stop().await;
    env.stage();
    let agent = env.start(Start::default()).await;
    agent.wait_unavailable(env.on_key).await;
    assert_eq!(agent.plugin_state().await, None, "{}", agent.log());
    let info = agent.listed().await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    let quiet = env.traffic();
    let (_, status) = agent.press(&agent.device().await).await;
    assert_eq!(status, ActionStatus::Failed);
    assert_eq!(env.since(&quiet), Traffic::default());
    agent.stop().await;
    assert_eq!(
        env.plugin_store().tombstone(&id).unwrap().unwrap().decision,
        TombstoneDecision::Uninstalled
    );

    // Nothing of the user's was deleted or rewritten.
    assert_eq!(env.legacy(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_alpha3_user_of_a_self_managed_gitlab_moves_to_the_plugin_after_consent() {
    upgrade(&GITLAB, OWN_GITLAB).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_alpha3_jira_user_moves_to_the_plugin_after_consent() {
    upgrade(&JIRA, OWN_JIRA).await;
}

/// A GitLab.com user: the declared host and its pages are derived (the
/// built-in used them), no server is asked about, and the token and the
/// origin slot (high tier: it may be on a private network) wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gitlab_com_user_is_asked_for_the_token_only_once_and_no_server() {
    let api = "https://gitlab.com/api/v4";
    let env = Env::new(
        "plugins-cutover-gitlab-com",
        &GITLAB,
        None,
        api,
        false,
        true,
    );
    let id = env.plugin_id();
    let agent = env.start(Start::default()).await;
    let asked = agent
        .wait_listed(ipc::MigrationPhase::AwaitingConsent)
        .await;
    assert_eq!(
        waiting(&asked),
        keys(&[
            "credentials:token".to_owned(),
            "network:user-origin:apiUrl".to_owned()
        ]),
        "{asked:?}"
    );
    assert!(
        asked.consent.iter().all(|i| i.origin.is_none()),
        "{asked:?}"
    );
    let derived: Vec<String> = env
        .plugin_store()
        .grants(&id)
        .unwrap()
        .into_iter()
        .filter(|g| g.decided_by == "migration:gitlab")
        .map(|g| g.key.to_string())
        .collect();
    assert_eq!(
        derived,
        [
            "network:gitlab.com",
            "openUrl:gitlab.com",
            "openUrl:user-origin:apiUrl"
        ]
    );
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin).await;
    let done = agent
        .decide(&["credentials:token", "network:user-origin:apiUrl"], &[])
        .await;
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    assert_eq!(env.allowed_servers(), [], "GitLab.com needs no consent");
    env.press_until(&agent, &device, Side::Plugin).await;
    env.run_node(&agent, Side::Plugin).await;
    agent.stop().await;
}

/// A GitLab on a private network: reaching it is an item of its own.
/// Declined, the built-in keeps serving, across restarts, until Retry;
/// allowed together with the server, the plugin serves, and the consent
/// records the private network for exactly that server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_private_gitlab_needs_its_own_consent_to_the_private_network() {
    let server = "https://gitlab.internal/api/v4";
    let env = Env::new(
        "plugins-cutover-gitlab-private",
        &GITLAB,
        Some(server),
        server,
        true,
        true,
    );
    let before = env.legacy();
    let slot = "network:user-origin:apiUrl";
    let private = "network:user-origin:apiUrl:private-network";
    let agent = env.start(Start::default()).await;
    let asked = agent
        .wait_listed(ipc::MigrationPhase::AwaitingConsent)
        .await;
    let item = asked.consent.iter().find(|i| i.key == private).unwrap();
    assert_eq!(item.permission.tier, ipc::PermissionTier::High);
    let origin = item.origin.as_ref().unwrap();
    assert!(origin.private_network);
    assert_eq!(origin.origin, server);
    let all: Vec<String> = waiting(&asked).into_iter().collect();
    assert!(all.contains(&slot.to_owned()) && all.contains(&private.to_owned()));

    // Everything allowed but the private network: declined.
    let others: Vec<&str> = all
        .iter()
        .map(String::as_str)
        .filter(|k| *k != private)
        .collect();
    let declined = agent.decide(&others, &[private]).await;
    eprintln!("declined: {:?}: {:?}", declined.phase, declined.reason);
    assert_eq!(declined.phase, ipc::MigrationPhase::NotEligible);
    assert!(declined.can_retry);
    assert_eq!(declined.route, MigrationRoute::Builtin);
    assert_eq!(env.allowed_servers(), []);
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin).await;
    agent.stop().await;

    // Still declined at the next start; nothing asked, nothing sent.
    let agent = env.start(Start::default()).await;
    let info = agent.listed().await;
    assert_eq!(info.phase, ipc::MigrationPhase::NotEligible, "{info:?}");
    let device = agent.device().await;
    env.press_until(&agent, &device, Side::Builtin).await;
    assert_eq!(env.plugin_log(), Vec::<String>::new());

    // Retry asks again; the server and its private network, together.
    let again = agent.retry().await;
    assert_eq!(again.phase, ipc::MigrationPhase::AwaitingConsent);
    let all: Vec<String> = waiting(&again).into_iter().collect();
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    let done = agent.decide(&all, &[]).await;
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    assert_eq!(
        env.allowed_servers(),
        [(server.to_owned(), true, "user:migration:gitlab".to_owned())]
    );
    env.press_until(&agent, &device, Side::Plugin).await;
    agent.stop().await;
    assert_eq!(env.legacy(), before);
}

/// What a plugin can never reach (plain `http`) keeps the built-in:
/// nothing installed, nothing asked. And a computer that does not use
/// the module (a fresh install) gets no migration-class plugin at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreachable_servers_and_unused_modules_stay_on_the_builtin() {
    for (module, saved) in [
        (&GITLAB, "http://gitlab.example.com/api/v4"),
        (&JIRA, "http://jira.example.com"),
    ] {
        let env = Env::new(
            &format!("plugins-cutover-{}-http", module.id),
            module,
            Some(saved),
            "https://unused.example.com",
            false,
            true,
        );
        let before = env.legacy();
        let agent = env.start(Start::default()).await;
        let info = agent.wait_listed(ipc::MigrationPhase::NotEligible).await;
        eprintln!("{}: {:?}", module.id, info.reason);
        let reason = info.reason.as_deref().unwrap();
        assert!(
            reason.contains("is not a server a plugin can reach"),
            "{info:?}"
        );
        assert!(!reason.contains("example.com"), "{reason}");
        assert_eq!(info.route, MigrationRoute::Builtin);
        assert_eq!(info.consent, []);
        assert_eq!(agent.plugin_state().await, None, "never installed");
        let device = agent.device().await;
        env.press_until(&agent, &device, Side::Builtin).await;
        env.run_node(&agent, Side::Builtin).await;
        agent.stop().await;
        assert_eq!(env.legacy(), before);
        assert_eq!(env.plugin_log(), Vec::<String>::new(), "the plugin ran");
    }
    // A fresh data directory: the module is unused, so its
    // migration-class plugin is not installed and its legacy action types
    // stay with the built-in.
    for module in [&GITLAB, &JIRA] {
        let env = Env::new(
            &format!("plugins-cutover-{}-fresh", module.id),
            module,
            None,
            "https://unused.example.com",
            false,
            false,
        );
        let agent = env.start(Start::default()).await;
        let info = agent.wait_listed(ipc::MigrationPhase::NotEligible).await;
        assert_eq!(
            info.reason.as_deref(),
            Some("the built-in module is not used on this computer"),
            "{info:?}"
        );
        assert_eq!(info.route, MigrationRoute::Builtin);
        assert_eq!(agent.plugin_state().await, None, "never installed");
        assert!(
            info.aliases.iter().any(|a| a.legacy == module.key.action),
            "{info:?}"
        );
        agent.stop().await;
        assert_eq!(env.legacy().module, None);
    }
}
