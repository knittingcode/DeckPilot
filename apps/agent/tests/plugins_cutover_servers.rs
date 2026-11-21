//! The GitLab and Jira cut-overs (`docs/marketplace/module-migration.md`
//! §9, P11), and the GitHub cut-over for GitHub Enterprise Server, end to
//! end, now that the built-in `gitlab`, `jira` and `github` modules are
//! removed from the app (§11 D1), through the real agent, the real
//! sandboxed plugin host, real storage and IPC, each test in its own data
//! directory (feature `plugins`). What it proves: a 0.9.x configuration of
//! a server the user entered migrates to the plugin **without the
//! built-in**, and nothing runs anywhere until the plugin can serve it.
//!
//! * a 0.9.x (alpha.3-style) `deckpilot.db`: the module row saved and
//!   enabled with **the server the user entered** (a self-managed GitLab
//!   under a path, a Jira Cloud site, a GitHub Enterprise Server), its
//!   credential stored, a profile with an MK.2 key bound to a legacy
//!   action and a workflow node running another (legacy ids). The agent
//!   has no built-in: the migration reads that row through the frozen
//!   descriptors;
//! * the shipped alias blocks (`ACTION_ALIASES`) and migration plans, and
//!   `plugins/first-party/{gitlab,jira,github}` built to wasm and bundled
//!   as the release bundles them (class `migration`, pinned with
//!   `DECKPILOT_TEST_BUNDLED_PINS`);
//! * no server ever answers: the plugin's requests get canned answers
//!   (`DECKPILOT_TEST_PLUGIN_HTTP`, which logs every request with the
//!   credential header the agent injected), so the log is everything sent;
//! * until the plugin serves (consent to the credential, **to the server**
//!   or, separately, to a private network pending or declined; a server no
//!   plugin can reach; plugins off; the plugin disabled or uninstalled)
//!   the legacy key and node are unavailable with a clear reason, the
//!   agent keeps answering, and nothing is sent; once allowed, the same
//!   stored key and node run in the plugin at exactly the user's server;
//! * the consents, Retry, disable and uninstall come through the app's
//!   requests; `deckpilot.db` is never rewritten and restarts redo nothing.
#![cfg(feature = "plugins")]

#[path = "support/parity.rs"]
mod parity;
mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
use parity::{Answer, json_answer};
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
    /// The module's setting with the server.
    setting: &'static str,
    /// The rest of what the 0.9.x user saved.
    settings: fn(&Module) -> ActionParameters,
    /// On the key, and in the workflow's node.
    key: Use,
    node: Use,
    /// The server's answers, canned for the plugin.
    answers: parity::Answers,
    /// What the plugin's log shows after the URL: the credential header.
    logged: fn(&Module) -> String,
    /// The plugin's declared API host and web host, if it has them.
    hosts: Option<(&'static str, &'static str)>,
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
    logged: |m| format!("\tPRIVATE-TOKEN: {}", m.secret),
    hosts: Some(("gitlab.com", "gitlab.com")),
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
    logged: |m| format!("\tauthorization: Basic {}", m.secret),
    hosts: None,
};

/// A Jira Cloud site.
const OWN_JIRA: &str = "https://team.atlassian.net";

// ------------------------------------------------- GitHub Enterprise Server

fn github_answers(base: &str, target: &str) -> Answer {
    let path = target.split('?').next().unwrap();
    let web = base.strip_suffix("/api/v3").unwrap_or(base);
    match path {
        "/search/issues" => json_answer(
            200,
            &json!({"total_count": 2, "incomplete_results": false, "items": [
                {"number": 12, "title": "Fix the build", "draft": false,
                 "repository_url": format!("{base}/repos/octo/site"),
                 "user": {"login": "mona"}, "html_url": format!("{web}/octo/site/pull/12")},
                {"number": 11, "title": "Docs", "draft": true,
                 "repository_url": format!("{base}/repos/octo/site"),
                 "user": {"login": "hubot"}, "html_url": format!("{web}/octo/site/pull/11")}
            ]}),
        ),
        "/repos/octo/site/commits/main/check-runs" => json_answer(
            200,
            &json!({"total_count": 2, "check_runs": [
                {"name": "build", "status": "completed", "conclusion": "success"},
                {"name": "lint", "status": "completed", "conclusion": "failure"}
            ]}),
        ),
        "/repos/octo/site/commits/main/status" => json_answer(
            200,
            &json!({"sha": "0123abc", "state": "success", "statuses": [
                {"context": "ci/legacy", "state": "success"}
            ]}),
        ),
        _ => json_answer(404, &json!({"message": "Not Found"})),
    }
}

const GITHUB: Module = Module {
    id: "github",
    name: "GitHub",
    plugin: parity::Plugin {
        id: "deckpilot.github",
        dir: "github",
        package: "deckpilot-github",
    },
    credential: "github.token",
    secret: "ghp_cutover-ghe-5c2a-not-a-real-token",
    slot: "token",
    setting: "apiUrl",
    settings: |m| {
        ActionParameters::new()
            .with("credential", text(m.credential))
            .with("timeoutSeconds", ParameterValue::Integer(30))
    },
    key: Use {
        name: "Open pull requests",
        action: "github.pull-requests",
        parameters: |_| {
            ActionParameters::new()
                .with("repository", text("octo/site"))
                .with("credential", text(""))
        },
        asks: "/search/issues",
    },
    node: Use {
        name: "Checks of main",
        action: "github.checks",
        parameters: |m| {
            ActionParameters::new()
                .with("repository", text("octo/site"))
                .with("ref", text("main"))
                .with("credential", text(m.credential))
        },
        asks: "/repos/octo/site/commits/main/check-runs",
    },
    answers: github_answers,
    logged: |m| format!("\tauthorization: Bearer {}", m.secret),
    hosts: Some(("api.github.com", "github.com")),
};

/// A GitHub Enterprise Server: its API under `/api/v3`.
const OWN_GITHUB: &str = "https://ghe.example.com/api/v3";

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
    on_key: ActionInstanceId,
    workflow: WorkflowId,
}

impl Env {
    /// A 0.9.x configuration of `module`: `saved` as its server setting
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
        if self.module.id == "github" {
            targets.push("/repos/octo/site/commits/main/status".into());
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

    /// The (now removed) built-in module saved and enabled as 0.9.x saved
    /// it, its credential stored, and a profile with a key bound to one legacy action and a workflow
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
            .env_remove("DECKPILOT_TEST_BUILTIN_GITHUB_API")
            .env_remove("DECKPILOT_TEST_BUILTIN_GITLAB_API")
            .env_remove("DECKPILOT_TEST_BUILTIN_JIRA_SITE")
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

    /// What was sent after the first `before` requests (there is no
    /// built-in left to send anything).
    fn since(&self, before: usize) -> Vec<String> {
        self.plugin_log()[before..].to_vec()
    }

    /// Presses the key until the plugin serves it (the routing reaches the
    /// registry a moment after the migration step is recorded); checks
    /// the requests it made.
    async fn press_until_served(&self, agent: &Agent, device: &DeviceId) {
        let deadline = Instant::now() + SETTLE;
        loop {
            let before = self.plugin_log().len();
            let (name, status) = agent.press(device).await;
            let new = self.since(before);
            assert_eq!(name, self.module.key.name);
            if !new.is_empty() {
                assert_eq!(status, ActionStatus::Succeeded, "{new:?}\n{}", agent.log());
                self.assert_plugin(&new, self.module.key.asks);
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the plugin never served the key: {status:?}\n{}",
                agent.log()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Runs the workflow; its node ran in the plugin.
    async fn run_node(&self, agent: &Agent) {
        let before = self.plugin_log().len();
        let record = agent.run_workflow(self.workflow).await;
        assert_eq!(
            record.summary.state,
            ExecutionState::Succeeded,
            "{record:#?}\n{}",
            agent.log()
        );
        self.assert_plugin(&self.since(before), self.module.node.asks);
    }

    /// The plugin's requests: for `asks` first, each to its server and
    /// nowhere else, with the credential the user stored with 0.9.x.
    fn assert_plugin(&self, sent: &[String], asks: &str) {
        let m = self.module;
        assert!(
            sent.first()
                .is_some_and(|first| first.starts_with(&format!("{}{asks}", self.server))),
            "{sent:?}"
        );
        for line in sent {
            assert!(line.starts_with(&format!("{}/", self.server)), "{line}");
            assert!(line.ends_with(&(m.logged)(m)), "{line}");
        }
    }

    /// The legacy key and the workflow node are unavailable, with a reason
    /// that contains `why`: running the action fails with it, the key
    /// press fails, the workflow does not start (its node names it), the
    /// agent keeps answering, and nothing is sent anywhere. The reason.
    async fn assert_unavailable(&self, agent: &Agent, device: &DeviceId, why: &str) -> String {
        let reason = agent.wait_unavailable(self.on_key, why).await;
        let before = self.plugin_log().len();
        let (name, status) = agent.press(device).await;
        assert_eq!(name, self.module.key.name);
        assert_eq!(status, ActionStatus::Failed);
        let refused = agent
            .request(Request::StartWorkflow {
                workflow: self.workflow,
            })
            .await
            .expect_err("the workflow does not start")
            .to_string();
        assert!(refused.contains(why), "{refused}");
        let Response::Status { .. } = agent.ok(Request::GetStatus).await else {
            panic!("the agent answers")
        };
        assert_eq!(self.since(before), Vec::<String>::new(), "nothing was sent");
        reason
    }
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

    /// Runs the key's action until it is unavailable for a reason that
    /// contains `why` (the routing settles a moment after a step); the
    /// reason.
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

/// Why the legacy types are unavailable while `module`'s migration waits.
fn waits(module: &Module) -> String {
    format!("`{}`) waits for your consent", module.plugin.id)
}

// ---------------------------------------------------------------- tests

/// A 0.9.x user whose module talks to a server they entered upgrades to a
/// build without the built-in: the bundled plugin is installed and
/// pinned, the settings and the server carried, and the credential **and
/// the server** wait for the user's consent; meanwhile the stored key and
/// node are unavailable (nothing sent). Neither consent allows the other.
/// Approved through IPC, the same key and workflow node run in the
/// plugin, at exactly that server, with the imported credential.
/// `deckpilot.db` is byte-identical; with plugins off nothing can run
/// them (unavailable, saying where they come from now). The user
/// disabling the plugin makes them unavailable again; uninstalling it and
/// upgrading again does not bring it back.
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
        "{} step 1: {:?}, route {:?}: {:?}",
        module.id, asked.phase, asked.route, asked.route_reason
    );
    assert_eq!(asked.plugin, module.plugin.id);
    assert_eq!(asked.module_name, module.name);
    // No built-in to fall back to: unavailable until the user decides.
    assert_eq!(asked.route, MigrationRoute::PluginWithdrawn);
    assert!(
        asked
            .route_reason
            .as_deref()
            .is_some_and(|r| r.contains("waits for your consent")),
        "{asked:?}"
    );
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
    if let Some((api, web)) = module.hosts {
        expected.push(format!("network:{api}"));
        expected.push(format!("openUrl:{web}"));
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

    // Meanwhile the stored key and node are unavailable, saying why.
    let device = agent.device().await;
    let why = env
        .assert_unavailable(&agent, &device, &waits(module))
        .await;
    eprintln!("{} step 1: unavailable: {why}", module.id);

    // 2. Everything but the server: the credential's consent does not
    // allow the server. Still unavailable, nothing sent.
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
    assert_eq!(partly.route, MigrationRoute::PluginWithdrawn);
    assert_eq!(env.allowed_servers(), []);
    assert_eq!(env.plugin_store().credentials(&id).unwrap(), []);
    env.assert_unavailable(&agent, &device, &waits(module))
        .await;
    assert_eq!(env.plugin_log(), Vec::<String>::new(), "nothing was sent");

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
    // The same stored key and node now run the plugin, at the user's
    // server.
    env.press_until_served(&agent, &device).await;
    env.run_node(&agent).await;
    agent.stop().await;

    // 4. Nothing an older app reads changed, byte for byte.
    assert_eq!(env.legacy(), before);

    // 5. Plugins off on the same data: nothing can run them, and the
    // reason says where they come from now.
    let agent = env.start(Start { plugins_off: true }).await;
    let device = agent.device().await;
    let why = env
        .assert_unavailable(&agent, &device, "now comes from")
        .await;
    eprintln!("{} step 5: plugins off: {why}", module.id);
    assert!(
        why.contains(&format!("`{}`", module.key.action)) && why.contains(module.plugin.id),
        "{why}"
    );
    agent.stop().await;

    // 6. Upgrading again: still the plugin, nothing asked again.
    let agent = env.start(Start::default()).await;
    let device = agent.device().await;
    env.press_until_served(&agent, &device).await;
    let info = agent.listed().await;
    assert_eq!(info.phase, ipc::MigrationPhase::CutOver, "{info:?}");
    assert_eq!(info.consent, []);

    // 7. The user disables the plugin: unavailable, nothing to fall back
    // to; enabled again, it serves again.
    agent
        .ok(Request::DisablePlugin {
            plugin: module.plugin.id.into(),
        })
        .await;
    let why = env
        .assert_unavailable(
            &agent,
            &device,
            &format!("`{}`) is disabled", module.plugin.id),
        )
        .await;
    eprintln!("{} step 7: disabled: {why}", module.id);
    let info = agent.listed().await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    agent
        .ok(Request::EnablePlugin {
            plugin: module.plugin.id.into(),
            consent: Vec::new(),
        })
        .await;
    env.press_until_served(&agent, &device).await;
    assert_eq!(agent.listed().await.route, MigrationRoute::Plugin);

    // 8. The user uninstalls it; an upgrade does not bring it back.
    agent
        .ok(Request::UninstallPlugin {
            plugin: module.plugin.id.into(),
            keep: deckpilot_ipc::plugins::PluginRetention::default(),
        })
        .await;
    agent.wait_unavailable(env.on_key, module.plugin.id).await;
    agent.stop().await;
    env.stage();
    let agent = env.start(Start::default()).await;
    let device = agent.device().await;
    env.assert_unavailable(&agent, &device, module.plugin.id)
        .await;
    assert_eq!(agent.plugin_state().await, None, "{}", agent.log());
    let info = agent.listed().await;
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    assert!(
        info.route_reason
            .as_deref()
            .is_some_and(|r| r.contains("uninstalled")),
        "{info:?}"
    );
    agent.stop().await;
    assert_eq!(
        env.plugin_store().tombstone(&id).unwrap().unwrap().decision,
        TombstoneDecision::Uninstalled
    );

    // Nothing of the user's was deleted or rewritten.
    assert_eq!(env.legacy(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_0_9_user_of_a_self_managed_gitlab_moves_to_the_plugin_after_consent() {
    upgrade(&GITLAB, OWN_GITLAB).await;
}

/// GitHub Enterprise Server (the `apiUrl` slot of `deckpilot.github`):
/// the same as a self-managed GitLab; api.github.com and github.com, which
/// the built-in did not talk to here, are asked for too, never derived.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_0_9_github_enterprise_user_moves_to_the_plugin_after_consent() {
    upgrade(&GITHUB, OWN_GITHUB).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_0_9_jira_user_moves_to_the_plugin_after_consent() {
    upgrade(&JIRA, OWN_JIRA).await;
}

/// A GitLab.com user: the declared host and its pages are derived (the
/// built-in used them), no server is asked about, and the token and the
/// origin slot (high tier: it may be on a private network) wait, the
/// legacy types unavailable meanwhile.
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
    env.assert_unavailable(&agent, &device, &waits(&GITLAB))
        .await;
    let done = agent
        .decide(&["credentials:token", "network:user-origin:apiUrl"], &[])
        .await;
    assert_eq!(done.phase, ipc::MigrationPhase::CutOver, "{done:?}");
    assert_eq!(env.allowed_servers(), [], "GitLab.com needs no consent");
    env.press_until_served(&agent, &device).await;
    env.run_node(&agent).await;
    agent.stop().await;
}

/// A GitLab on a private network: reaching it is an item of its own.
/// Declined, the legacy types stay unavailable (saying the user
/// declined), across restarts, until Retry; allowed together with the
/// server, the plugin serves, and the consent records the private network
/// for exactly that server.
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
    let device = agent.device().await;
    env.assert_unavailable(&agent, &device, &waits(&GITLAB))
        .await;

    // Everything allowed but the private network: declined.
    let others: Vec<&str> = all
        .iter()
        .map(String::as_str)
        .filter(|k| *k != private)
        .collect();
    let declined = agent.decide(&others, &[private]).await;
    eprintln!(
        "declined: {:?}: {:?}; route {:?}: {:?}",
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
            .is_some_and(|r| r.contains("you declined what deckpilot.gitlab needs")),
        "{declined:?}"
    );
    assert_eq!(env.allowed_servers(), []);
    env.assert_unavailable(&agent, &device, GITLAB.plugin.id)
        .await;
    agent.stop().await;

    // Still declined at the next start; nothing asked, nothing sent.
    let agent = env.start(Start::default()).await;
    let info = agent.listed().await;
    assert_eq!(info.phase, ipc::MigrationPhase::NotEligible, "{info:?}");
    assert_eq!(info.route, MigrationRoute::PluginWithdrawn, "{info:?}");
    let device = agent.device().await;
    env.assert_unavailable(&agent, &device, GITLAB.plugin.id)
        .await;
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
    env.press_until_served(&agent, &device).await;
    agent.stop().await;
    assert_eq!(env.legacy(), before);
}

/// What a plugin can never reach (plain `http`): nothing installed,
/// nothing asked, and with no built-in left the legacy types are
/// unavailable, saying where they come from now and that the plugin is
/// not installed; nothing is sent. And
/// a computer that does not use the module (a fresh install) gets no
/// migration-class plugin at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unreachable_servers_and_unused_modules_install_nothing() {
    for (module, saved) in [
        (&GITLAB, "http://gitlab.example.com/api/v4"),
        (&JIRA, "http://jira.example.com"),
        (&GITHUB, "http://ghe.example.com/api/v3"),
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
        eprintln!(
            "{}: {:?}; route {:?}: {:?}",
            module.id, info.reason, info.route, info.route_reason
        );
        let reason = info.reason.as_deref().unwrap();
        assert!(
            reason.contains("is not a server a plugin can reach"),
            "{info:?}"
        );
        assert!(!reason.contains("example.com"), "{reason}");
        // Routed to the plugin: there is nothing else to route to.
        assert_eq!(info.route, MigrationRoute::Plugin);
        assert_eq!(info.consent, []);
        assert_eq!(agent.plugin_state().await, None, "never installed");
        let device = agent.device().await;
        let why = env
            .assert_unavailable(&agent, &device, "install it from the Marketplace")
            .await;
        eprintln!("{}: unavailable: {why}", module.id);
        assert!(
            why.contains(&format!(
                "`{}` now comes from `{}`",
                module.key.action, module.plugin.id
            )),
            "{why}"
        );
        agent.stop().await;
        assert_eq!(env.legacy(), before);
        assert_eq!(env.plugin_log(), Vec::<String>::new(), "the plugin ran");
    }
    // A fresh data directory: the module is unused, so its
    // migration-class plugin is not installed.
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
        eprintln!(
            "{} fresh: {:?}; route {:?}: {:?}; aliases {:?}",
            module.id, info.reason, info.route, info.route_reason, info.aliases
        );
        assert_eq!(
            info.reason.as_deref(),
            Some("the built-in module is not used on this computer"),
            "{info:?}"
        );
        assert_eq!(info.route, MigrationRoute::Plugin);
        assert_eq!(agent.plugin_state().await, None, "never installed");
        assert!(
            info.aliases.iter().any(|a| a.legacy == module.key.action),
            "{info:?}"
        );
        agent.stop().await;
        assert_eq!(env.legacy().module, None);
    }
}
