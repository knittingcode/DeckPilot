//! What the regression tests of the first-party plugins that replaced the
//! removed built-in modules share (`plugins_github.rs`, `plugins_gitlab.rs`,
//! `plugins_jira.rs`, and `plugins_cutover_servers.rs`): the plugin built
//! to wasm and staged as an installed application ships it, a real agent
//! process with the real sandboxed host, canned answers for the plugin's
//! HTTP, the expected outcome of each scenario, and the check of a
//! plugin's action definitions against the frozen contracts of the
//! built-ins (`crates/deckpilot-modules/legacy`). The plugins were proven
//! to answer as the built-ins did before those were removed
//! (`docs/marketplace/module-migration.md` §11, D2); the expectations are
//! those answers.
//!
//! Not part of `support/mod.rs`: only those tests include it (`#[path]`),
//! since it needs the agent's `plugins` feature.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use deckpilot_core::ActionType;
use deckpilot_core::alias::{self, ActionAlias, ParamRule};
use deckpilot_ipc::plugins::{PluginActionInfo, PluginActionOutcome, PluginSettings};
use deckpilot_ipc::{
    AgentClient, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_storage::DatabaseKey;
use serde_json::{Value, json};

use crate::support::{self, TempDir};

/// The `Date` on every answer (a plugin's only clock).
pub const DATE: &str = "Mon, 16 Nov 2026 10:00:00 GMT";
const STARTUP: Duration = Duration::from_secs(30);

/// A first-party plugin under test.
#[derive(Debug, Clone, Copy)]
pub struct Plugin {
    /// `deckpilot.gitlab`.
    pub id: &'static str,
    /// Its directory under `plugins/first-party`.
    pub dir: &'static str,
    /// Its crate (`deckpilot-gitlab`).
    pub package: &'static str,
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
/// taken from `DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR`),
/// once per plugin and test binary.
pub fn artifacts(plugin: &Plugin) -> &'static Artifacts {
    static ARTIFACTS: Mutex<BTreeMap<&'static str, &'static Artifacts>> =
        Mutex::new(BTreeMap::new());
    let mut built = ARTIFACTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    built.entry(plugin.package).or_insert_with(|| {
        let wasm = format!("{}.wasm", plugin.package.replace('-', "_"));
        let wasm_dir = std::env::var_os("DECKPILOT_E2E_WASM_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.join(&wasm).is_file())
            .unwrap_or_else(|| {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    plugin.package,
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
            module: wasm_dir.join(wasm),
        }))
    })
}

// ----------------------------------------------------------- the answers

/// One canned answer.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

pub fn json_answer(status: u16, body: &Value) -> Answer {
    Answer {
        status,
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("date".into(), DATE.into()),
        ],
        body: body.to_string(),
    }
}

/// A page that is not JSON (a captive portal).
pub fn html_answer() -> Answer {
    Answer {
        status: 200,
        headers: vec![
            ("content-type".into(), "text/html".into()),
            ("date".into(), DATE.into()),
        ],
        body: "<html>a captive portal</html>".into(),
    }
}

impl Answer {
    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn find(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A path segment, percent-decoded.
pub fn decode(text: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", text.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

/// The query of `target` (`/path?query`) as pairs, decoded.
pub fn query(target: &str) -> BTreeMap<String, String> {
    target
        .split_once('?')
        .map(|(_, q)| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default()
}

/// The canned answers' data: the answer to `GET <base><target>`, where
/// `base` is the API's address as the asking side knows it (for links to
/// further pages) and `target` the path below it, with the query.
pub type Answers = fn(base: &str, target: &str) -> Answer;

/// The `rel="next"` target of an answer's `link` header.
pub fn next_link(answer: &Answer) -> Option<String> {
    let link = answer.find("link")?;
    link.split(',').find_map(|part| {
        let (target, rel) = part.split_once(';')?;
        rel.contains("rel=\"next\"").then(|| {
            target
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_owned()
        })
    })
}

/// The canned routes of the plugin's HTTP for `answers` served at `base`:
/// every URL of `urls`, and every page their `next` links lead to; then
/// `failures` (`(url, transport code)`). Longest first: the canned
/// transport takes the first route whose URL starts the request's.
pub fn routes(
    base: &str,
    answers: Answers,
    urls: &[String],
    failures: &[(String, &str)],
) -> Vec<Value> {
    let mut all = Vec::new();
    for first in urls {
        let mut next = Some(first.clone());
        let mut pages = 0;
        while let Some(url) = next.take() {
            pages += 1;
            let target = url
                .strip_prefix(base)
                .unwrap_or_else(|| panic!("{url} is not under {base}"))
                .to_owned();
            all.push(url);
            if pages < 12 {
                next = next_link(&answers(base, &target));
            }
        }
    }
    all.sort_by_key(|u| std::cmp::Reverse(u.len()));
    all.dedup();
    let mut routes: Vec<Value> = failures
        .iter()
        .map(|(url, code)| json!({"url": url, "status": 0, "body": "", "fail": code}))
        .collect();
    routes.extend(all.iter().map(|url| {
        let a = answers(base, url.strip_prefix(base).unwrap());
        json!({"url": url, "status": a.status, "headers": a.headers, "body": a.body})
    }));
    routes
}

// ------------------------------------------------------------- outcomes

/// What a side answered, as the engine sees an action's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub ok: bool,
    pub message: String,
    pub data: BTreeMap<String, String>,
    pub code: Option<String>,
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn from_plugin(outcome: PluginActionOutcome) -> Outcome {
    match outcome {
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
    }
}

/// What a scenario is expected to end in.
#[derive(Debug, Clone, Copy)]
pub enum Expect {
    /// Succeeds with this message and (among others) these outputs.
    Ok(&'static str, &'static [(&'static str, &'static str)]),
    /// Fails with this code, with a message that explains it.
    Failed(&'static str),
}

impl Expect {
    pub fn ok(self) -> bool {
        matches!(self, Self::Ok(..))
    }

    pub fn code(self) -> Option<&'static str> {
        match self {
            Self::Ok(..) => None,
            Self::Failed(code) => Some(code),
        }
    }
}

/// Checks what the plugin answered for the scenario `name` against
/// `expect`, where every `(from, to)` of `vars` replaces `from` in the
/// expected message and outputs (`{web}`: the server's web address).
/// Adds what does not match to `problems`.
pub fn check(
    name: &str,
    outcome: &Outcome,
    expect: Expect,
    vars: &[(&str, &str)],
    problems: &mut Vec<String>,
) {
    let fill = |text: &str| {
        vars.iter()
            .fold(text.to_owned(), |text, (from, to)| text.replace(from, to))
    };
    // The table of the report: what the plugin answered.
    eprintln!(
        "{name}: {}",
        if outcome.ok {
            format!("ok, {} outputs", outcome.data.len())
        } else {
            format!("failed, {}", outcome.code.as_deref().unwrap_or("?"))
        }
    );
    let mut wrong = Vec::new();
    match expect {
        Expect::Ok(message, outputs) => {
            if !outcome.ok {
                wrong.push("not ok".to_owned());
            }
            if outcome.message != fill(message) {
                wrong.push(format!("the message is not {:?}", fill(message)));
            }
            for (key, value) in outputs {
                let value = fill(value);
                if outcome.data.get(*key) != Some(&value) {
                    wrong.push(format!("{key} is not {value:?}"));
                }
            }
        }
        Expect::Failed(code) => {
            if outcome.ok || outcome.code.as_deref() != Some(code) {
                wrong.push(format!("not failed with {code}"));
            }
            if outcome.message.trim().is_empty() {
                wrong.push("no message".to_owned());
            }
        }
    }
    if !wrong.is_empty() {
        problems.push(format!(
            "{name}: {}
  plugin: {outcome:?}",
            wrong.join("; ")
        ));
    }
}

/// The requests of `sent` (`url<TAB>credential` lines) that none of the
/// canned `routes` answers: what the plugin asked for beyond what its
/// scenarios are known to ask for.
pub fn unrouted(sent: &[String], routes: &[Value]) -> Vec<String> {
    sent.iter()
        .filter(|line| {
            let url = line.split('\t').next().unwrap_or_default();
            !routes
                .iter()
                .any(|r| r["url"].as_str().is_some_and(|r| url.starts_with(r)))
        })
        .cloned()
        .collect()
}

// ------------------------------------------------------------ contracts

/// The action types of a removed built-in as 0.9.0 declared them: the
/// `actionTypes` of its frozen contract (`crates/deckpilot-modules/legacy`,
/// written from the crate before it was removed).
pub fn frozen_action_types(contract: &str) -> Vec<ActionType> {
    let mut frozen: Value = serde_json::from_str(contract).expect("a frozen contract parses");
    serde_json::from_value(frozen["actionTypes"].take()).expect("its action types parse")
}

/// The alias the cut-over adds for `<module>.<action>`
/// (`crates/deckpilot-modules/tests/contracts.rs`).
pub fn alias(module: &'static str, plugin: &'static str, action: &str) -> ActionAlias {
    let legacy: &'static str = Box::leak(format!("{module}.{action}").into_boxed_str());
    let target: &'static str = Box::leak(format!("{plugin}.{action}").into_boxed_str());
    ActionAlias {
        legacy,
        target,
        module,
        plugin,
        since: "0.6.0",
        params: if action == "open" {
            &[]
        } else {
            &[ParamRule::DropIfDefault("credential")]
        },
    }
}

/// §5.3's rules for the plugin's action definitions (as the agent's engine
/// registers them) against the frozen built-in's, the check the registry
/// makes before it routes a legacy type to the plugin; and every action of
/// the built-in has its plugin action.
pub fn contract_problems(
    module: &'static str,
    plugin: &'static str,
    contract: &str,
    actions: &[PluginActionInfo],
) -> Vec<String> {
    let builtins = frozen_action_types(contract);
    let mut problems = Vec::new();
    for action in actions {
        let Some(definition) = action.definition.as_ref() else {
            problems.push(format!("{} does not convert", action.action));
            continue;
        };
        let alias = alias(module, plugin, &action.action);
        let Some(builtin) = builtins.iter().find(|t| t.id == alias.legacy_id()) else {
            problems.push(format!("{} is not a built-in action", alias.legacy));
            continue;
        };
        for problem in alias::incompatibilities(&alias, builtin, definition) {
            problems.push(format!("{}: {problem}", alias.legacy));
        }
    }
    for builtin in &builtins {
        let action = builtin
            .id
            .as_str()
            .strip_prefix(&format!("{module}."))
            .unwrap_or_default();
        if !actions.iter().any(|a| a.action == action) {
            problems.push(format!("{} has no plugin action", builtin.id.as_str()));
        }
    }
    problems
}

// ---------------------------------------------------------------- pace

/// The agent lets a plugin send 60 requests per minute (a bucket of 60,
/// refilled one per second). A comparison sends more than that, so it
/// waits where a plugin on a key would simply be throttled.
pub struct Pace {
    started: Instant,
}

impl Pace {
    /// Starts counting: the plugin has sent nothing yet.
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }

    /// Waits until the plugin may send `needed` more requests.
    pub async fn room_for(&self, env: &Env, needed: usize) {
        loop {
            let allowed = 58.0 + self.started.elapsed().as_secs_f64();
            #[allow(clippy::cast_precision_loss)] // a few hundred requests
            let wanted = (env.sent().len() + needed) as f64;
            if wanted <= allowed {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

// ---------------------------------------------------------------- agent

pub struct Env {
    pub dir: TempDir,
    pub paths: AgentPaths,
    key: DatabaseKey,
    plugin: Plugin,
}

impl Env {
    /// A data directory, the plugin staged as bundled (class `default`:
    /// only the migration engine installs a `migration`-class plugin) and
    /// the canned `routes` of its HTTP.
    pub fn new(plugin: Plugin, routes: &[Value]) -> Self {
        let dir = TempDir::new(&format!("plugins-{}", plugin.dir));
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let env = Self {
            dir,
            paths,
            key,
            plugin,
        };
        env.stage();
        std::fs::write(env.http(), serde_json::to_vec_pretty(routes).unwrap()).unwrap();
        env
    }

    fn bundled(&self) -> PathBuf {
        self.dir.path().join("resources/bundled")
    }

    fn pins_file(&self) -> PathBuf {
        self.dir.path().join("compiled-pins.json")
    }

    fn http(&self) -> PathBuf {
        self.dir.path().join("http.json")
    }

    fn opened_file(&self) -> PathBuf {
        self.dir.path().join("opened.txt")
    }

    /// The pages the plugin opened, in order.
    pub fn opened(&self) -> Vec<String> {
        std::fs::read_to_string(self.opened_file())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// What the plugin sent: `url<TAB>credential header` per request.
    pub fn sent(&self) -> Vec<String> {
        std::fs::read_to_string(format!("{}.log", self.http().display()))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// `bundled/<id>/` as `plugins/first-party/build.sh` stages it, and
    /// the pins the agent "was compiled with".
    fn stage(&self) {
        let source = repo().join("plugins/first-party").join(self.plugin.dir);
        let dir = self.bundled().join(self.plugin.id);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for file in ["manifest.json", "LICENSE", "assets/icon-128.png"] {
            std::fs::copy(source.join(file), dir.join(file)).unwrap();
        }
        std::fs::copy(&artifacts(&self.plugin).module, dir.join("plugin.wasm")).unwrap();
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let version = semver::Version::parse(manifest["version"].as_str().unwrap()).unwrap();
        let pins = Pins::compute(
            &self.bundled(),
            &[(
                PluginId::parse(self.plugin.id).unwrap(),
                version,
                BundleClass::Default,
            )],
        )
        .unwrap();
        std::fs::write(self.bundled().join("pins.json"), pins.to_json_pretty()).unwrap();
        std::fs::write(self.pins_file(), pins.to_json_pretty()).unwrap();
    }

    pub async fn start(&self) -> Agent {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.path().join("agent.log"))
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
            .args(["--headless", "--no-usb", "--log-level", "debug"])
            .arg("--enable-plugins")
            .arg("--plugin-host")
            .arg(&artifacts(&self.plugin).host)
            .arg("--bundled-plugins")
            .arg(self.bundled())
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_BUNDLED_PINS", self.pins_file())
            .env("DECKPILOT_TEST_PLUGIN_HTTP", self.http())
            .env("DECKPILOT_TEST_PLUGIN_OPEN_URL", self.opened_file())
            .env_remove("DECKPILOT_ENABLE_PLUGINS")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_URL")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_ROOTS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("the agent starts");
        let agent = Agent {
            child,
            paths: self.paths.clone(),
            log: self.dir.path().join("agent.log"),
            plugin: self.plugin,
        };
        agent
            .request(Request::Unlock {
                key: KeyMaterial::new(self.key.to_hex()),
            })
            .await
            .unwrap();
        agent
    }
}

pub struct Agent {
    child: Child,
    paths: AgentPaths,
    log: PathBuf,
    plugin: Plugin,
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

    pub async fn request(&self, request: Request) -> Result<Response, ClientError> {
        let what = request.name();
        tokio::time::timeout(Duration::from_secs(120), async {
            self.client().await.request(request).await
        })
        .await
        .unwrap_or_else(|_| panic!("{what} took too long"))
    }

    /// Runs one of the plugin's actions as the configuration app does.
    pub async fn invoke_raw(&self, action: &str, params: &Value) -> PluginActionOutcome {
        match self
            .request(Request::InvokePluginAction {
                plugin: self.plugin.id.into(),
                action: action.into(),
                params: serde_json::from_value(params.clone()).unwrap(),
            })
            .await
            .unwrap_or_else(|e| panic!("{action}: {e}\n{}", self.log()))
        {
            Response::PluginActionResult { outcome } => outcome,
            other => panic!("unexpected {other:?}"),
        }
    }

    pub async fn invoke(&self, action: &str, params: &Value) -> Outcome {
        from_plugin(self.invoke_raw(action, params).await)
    }

    pub async fn ok(&self, request: Request) {
        let what = request.name();
        if let Err(e) = self.request(request).await {
            panic!("{what}: {e}\n{}", self.log());
        }
    }

    /// Saves the plugin's settings and returns them as the app sees them.
    pub async fn set_settings(&self, values: BTreeMap<String, Value>) -> PluginSettings {
        match self
            .request(Request::SetPluginSettings {
                plugin: self.plugin.id.into(),
                values,
            })
            .await
            .unwrap_or_else(|e| panic!("setPluginSettings: {e}\n{}", self.log()))
        {
            Response::PluginSettings { settings } => *settings,
            other => panic!("unexpected {other:?}"),
        }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}
