//! What the parity tests of the first-party plugins for a server the user
//! enters share (`plugins_gitlab.rs`, `plugins_jira.rs`): the plugin built
//! to wasm and staged as an installed application ships it, a real agent
//! process with the real sandboxed host, canned answers for the plugin's
//! HTTP, a stand-in server on 127.0.0.1 for the built-in module, and the
//! comparison of what each side answered.
//!
//! Not part of `support/mod.rs`: only those tests include it (`#[path]`),
//! since it needs the agent's `plugins` feature.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionParameters, ActionStatus, ActionTypeId, CredentialId, ExecutionContext, MachineConfig,
    MemoryCredentials, ModuleRegistry, Os, ParameterValue, Profile, Secret,
};
use deckpilot_ipc::plugins::{PluginActionOutcome, PluginSettings};
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
/// taken from `DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR`).
/// One plugin per test binary.
pub fn artifacts(plugin: &Plugin) -> &'static Artifacts {
    static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
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
        Artifacts {
            host,
            module: wasm_dir.join(wasm),
        }
    })
}

// ----------------------------------------------------------- the answers

/// One answer of a stand-in server.
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

/// The stand-in server's data: the answer to `GET <base><target>`, where
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

impl Outcome {
    /// With `from` replaced by `to` in the message and every output: the
    /// built-in's own address (127.0.0.1) as the plugin's server.
    #[must_use]
    pub fn replacing(mut self, from: &str, to: &str) -> Self {
        self.message = self.message.replace(from, to);
        for value in self.data.values_mut() {
            *value = value.replace(from, to);
        }
        self
    }
}

/// Compares what both sides answered. A failure listed in `different`
/// (by scenario name, with the reason) may have another message; its code
/// is the same.
pub fn compare(
    name: &str,
    builtin: &Outcome,
    plugin: &Outcome,
    different: &[(&str, &str)],
    problems: &mut Vec<String>,
) {
    let mut expected = builtin.clone();
    if !builtin.ok && different.iter().any(|(n, _)| *n == name) {
        expected.message.clone_from(&plugin.message);
    }
    // The table of the report: what each side answered.
    eprintln!(
        "{name}: {}",
        if plugin.ok {
            format!("ok, {} outputs", plugin.data.len())
        } else {
            format!("failed, {}", plugin.code.as_deref().unwrap_or("?"))
        }
    );
    if expected != *plugin {
        problems.push(format!(
            "{name}:\n  built-in: {builtin:?}\n  plugin:   {plugin:?}"
        ));
    }
}

// ------------------------------------------------------------- built-in

/// A stand-in server on 127.0.0.1 for the built-in module: `answers`
/// below `prefix` (the API's path), with links on its own address.
pub struct Server {
    port: u16,
    prefix: String,
    stop: Arc<AtomicBool>,
    /// `target<TAB>credential header value` of every request.
    pub seen: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// `credential_header`: the request header that carries the secret
    /// (`authorization`, `private-token`), recorded with each request.
    pub fn start(prefix: &str, answers: Answers, credential_header: &'static str) -> Self {
        let listener = support::http::bind_local();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let (stop, seen, prefix) = (stop.clone(), seen.clone(), prefix.to_owned());
            std::thread::spawn(move || {
                let base = format!("http://127.0.0.1:{port}{prefix}");
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            serve(stream, &base, &prefix, answers, credential_header, &seen);
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(2)),
                    }
                }
            })
        };
        Self {
            port,
            prefix: prefix.to_owned(),
            stop,
            seen,
            thread: Some(thread),
        }
    }

    /// `http://127.0.0.1:<port>`.
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The API's address: the origin and the prefix.
    pub fn base(&self) -> String {
        format!("{}{}", self.origin(), self.prefix)
    }

    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    stream: std::net::TcpStream,
    base: &str,
    prefix: &str,
    answers: Answers,
    credential_header: &str,
    seen: &Mutex<Vec<String>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let full = line.split(' ').nth(1).unwrap_or("/").to_owned();
    let mut credential = String::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case(credential_header)
        {
            value.trim().clone_into(&mut credential);
        }
    }
    let answer = if let Some(target) = full.strip_prefix(prefix) {
        seen.lock().unwrap().push(format!("{target}\t{credential}"));
        answers(base, target)
    } else {
        // Outside the API: nothing a module should ask for.
        seen.lock()
            .unwrap()
            .push(format!("OUTSIDE {full}\t{credential}"));
        json_answer(404, &json!({"message": "outside the API"}))
    };
    let mut stream = reader.into_inner();
    let mut head = format!("HTTP/1.1 {} X\r\n", answer.status);
    for (name, value) in &answer.headers {
        let _ = std::fmt::Write::write_fmt(&mut head, format_args!("{name}: {value}\r\n"));
    }
    let _ = std::fmt::Write::write_fmt(
        &mut head,
        format_args!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            answer.body.len()
        ),
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(answer.body.as_bytes());
    let _ = stream.flush();
}

/// Action parameters from JSON: `choices` are the keys of choice
/// parameters, `booleans` come from JSON booleans.
pub fn parameters(params: &Value, choices: &[&str]) -> ActionParameters {
    let mut out = ActionParameters::new();
    for (key, value) in params.as_object().unwrap() {
        let value = match value {
            Value::String(s) if choices.contains(&key.as_str()) => {
                ParameterValue::Choice(s.clone())
            }
            Value::String(s) => ParameterValue::Text(s.clone()),
            Value::Number(n) => ParameterValue::Integer(n.as_i64().unwrap()),
            Value::Bool(b) => ParameterValue::Boolean(*b),
            other => panic!("{other:?}"),
        };
        out = out.with(key.clone(), value);
    }
    out
}

/// Runs a built-in action as the agent does (normalized, through the
/// registry), with `secret` stored as the credential `credential`.
pub fn run_builtin(
    modules: &ModuleRegistry,
    action_type: &str,
    params: &ActionParameters,
    credential: &str,
    secret: Option<&str>,
) -> Outcome {
    let registry = modules.action_registry();
    let id = ActionTypeId::new(action_type).unwrap();
    let normalized = registry.normalize(&id, params).unwrap();
    let profile = Profile::new("Parity");
    let machine = MachineConfig::new(Os::current());
    let mut credentials = MemoryCredentials::new();
    if let Some(secret) = secret {
        credentials = credentials.with(CredentialId::new(credential).unwrap(), Secret::new(secret));
    }
    let context = ExecutionContext::new(&profile, None, &machine).with_credentials(&credentials);
    let result = registry.dispatch(&id, &normalized, &context);
    Outcome {
        ok: result.status == ActionStatus::Succeeded,
        message: result.message,
        // A failure's data (`retryAfterSeconds` of `rate-limited`) is not
        // an output; plugin failures carry none.
        data: if result.status == ActionStatus::Succeeded {
            result.data
        } else {
            BTreeMap::new()
        },
        code: result.code,
    }
}

/// The built-in blocks on its own HTTP runtime: off this test's.
pub fn blocking<T>(run: impl FnOnce() -> T) -> T {
    tokio::task::block_in_place(run)
}

/// The paths of requests (`url-or-target<TAB>…` lines), without `base`
/// and the query, repeats in a row counted once: the same requests but
/// for the page size.
pub fn paths(lines: &[String], base: &str) -> Vec<String> {
    let mut paths: Vec<String> = lines
        .iter()
        .map(|l| {
            let target = l.split('\t').next().unwrap();
            let target = target.strip_prefix(base).unwrap_or(target);
            target.split('?').next().unwrap().to_owned()
        })
        .collect();
    paths.dedup();
    paths
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
