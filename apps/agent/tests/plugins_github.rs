//! `deckpilot.github` against the built-in `github` module it replaces
//! (`docs/marketplace/module-migration.md` §6, P8), on the same GitHub
//! answers (feature `plugins`):
//!
//! * the plugin (`plugins/first-party/github`) is built to wasm with the
//!   `guest` profile and bundled as an installed application ships it;
//!   the real agent seeds it, the user consents to its permissions and
//!   stores its token, and every action runs in the real sandboxed host,
//!   invoked over IPC like the configuration app does;
//! * its HTTP goes to canned answers (`DECKPILOT_TEST_PLUGIN_HTTP`, feature
//!   `test-hooks`) and its opened pages to a file
//!   (`DECKPILOT_TEST_PLUGIN_OPEN_URL`): never the real GitHub, never a
//!   browser;
//! * the built-in runs in this process against a stand-in GitHub on
//!   127.0.0.1 (its API URL, allowed on a private host) that serves the
//!   same answers from the same function ([`answer`]), with a recording
//!   launcher;
//! * every action's result is compared: the same status, message, outputs
//!   and error code. Where the two cannot agree the difference is listed
//!   in [`DIFFERENT_MESSAGES`] with its reason (and in
//!   `plugins/first-party/github/README.md`);
//! * network failures (a name that does not resolve, a certificate that
//!   is not trusted) report the built-in's transport code on both sides:
//!   the plugin reads the agent's network kind (`dp_features`);
//! * a GitHub Enterprise Server the user enters in `apiUrl` (the plugin's
//!   user-origin slot) is reached only once the user allowed exactly that
//!   server (a private one only with its own consent), and only until the
//!   consent is revoked; api.github.com needs none of its own.
//!
//! The plugin is pinned here as a `default`-class plugin and invoked by its
//! own action types: `migration`-class plugins are installed only by the
//! migration engine, and the legacy `github.*` types reach the plugin only
//! after a computer's migration is cut over. That path, from an alpha.3
//! configuration through the consent, is `plugins_cutover.rs`.
#![cfg(feature = "plugins")]

#[path = "support/https.rs"]
mod https;
mod support;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use deckpilot_core::alias::{self, ActionAlias, ParamRule};
use deckpilot_core::launch::RecordingLauncher;
use deckpilot_core::{
    ActionParameters, ActionStatus, ActionTypeId, CredentialId, ExecutionContext, MachineConfig,
    MemoryCredentials, ModuleRegistry, Os, ParameterValue, Profile, Secret,
};
use deckpilot_ipc::plugins::{
    PluginActionOutcome, PluginSecret, PluginSettings, PluginState, UserOriginStatus,
};
use deckpilot_ipc::{
    AgentClient, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_modules::github;
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_storage::DatabaseKey;
use serde_json::{Value, json};
use support::TempDir;

const ID: &str = "deckpilot.github";
const GRANTS: &[&str] = &[
    "network:api.github.com",
    "network:user-origin:apiUrl",
    "credentials:token",
    "openUrl:github.com",
    "openUrl:user-origin:apiUrl",
];
/// The token the user stores, for the plugin and for the built-in.
const TOKEN: &str = "ghp_parity-4d1f7a-not-a-real-token";
const API: &str = "https://api.github.com";
/// A GitHub Enterprise Server the user enters, as the agent normalizes it.
const GHE_API: &str = "https://ghe.example.com/api/v3";
/// GitHub's `Date` on every answer (the plugin's only clock).
const DATE: &str = "Mon, 16 Nov 2026 10:00:00 GMT";
const STARTUP: Duration = Duration::from_secs(30);
/// What the plugin answers may use of the IPC message limit.
const ANSWER_BUDGET: usize = deckpilot_ipc::framing::MAX_MESSAGE_BYTES / 2;

/// Failures whose message differs, by scenario, and why. The code is the
/// same.
const DIFFERENT_MESSAGES: &[(&str, &str)] = &[
    (
        "repository octo/revoked",
        "the built-in names the credential id (`github.token`); the plugin has a slot, not an id",
    ),
    (
        "current-user without a token",
        "the built-in names the credential id and Settings › HTTP › Credentials; the plugin \
         names its slot in Settings › Plugins › GitHub",
    ),
];

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

/// One answer of the stand-in GitHub.
#[derive(Debug, Clone)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

fn json_answer(status: u16, body: &Value) -> Answer {
    Answer {
        status,
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("date".into(), DATE.into()),
        ],
        body: body.to_string(),
    }
}

impl Answer {
    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

fn decode(text: &str) -> String {
    url::form_urlencoded::parse(format!("x={text}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

/// The query of `target` (`/path?query`) as pairs, decoded.
fn query(target: &str) -> BTreeMap<String, String> {
    target
        .split_once('?')
        .map(|(_, q)| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default()
}

/// One page of `items`, GitHub style: `per_page` and `page` from the
/// query, a `link` to the next page (on `base`) while there is one.
fn page(base: &str, target: &str, key: &str, items: &[Value], extra: Value) -> Answer {
    let query = query(target);
    let per_page: usize = query.get("per_page").map_or(30, |v| v.parse().unwrap());
    let page: usize = query.get("page").map_or(1, |v| v.parse().unwrap());
    let start = ((page - 1) * per_page).min(items.len());
    let end = (page * per_page).min(items.len());
    let mut body = extra;
    body[key] = Value::Array(items[start..end].to_vec());
    let mut answer = json_answer(200, &body);
    if end < items.len() {
        let path = target.split('?').next().unwrap();
        let mut next = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in query.iter().filter(|(k, _)| *k != "page") {
            next.append_pair(k, v);
        }
        next.append_pair("page", &(page + 1).to_string());
        let link = format!(
            "<{base}{path}?{}>; rel=\"next\", <{base}{path}?page=99>; rel=\"last\"",
            next.finish()
        );
        answer = answer.header("link", &link);
    }
    answer
}

/// Open pull requests found by a search `q`.
fn search_items(q: &str) -> Vec<Value> {
    let site = q.contains("repo:octo/site");
    let count = match (
        site,
        q.contains("author:@me"),
        q.contains("review-requested:@me"),
    ) {
        (true, false, false) => 73,
        (true, false, true) => 2,
        (false, true, false) => 3,
        (false, false, true) => 5,
        _ => 0,
    };
    (1..=count)
        .map(|i| {
            let repo = if site || i % 2 == 1 {
                "octo/site"
            } else {
                "octo/api"
            };
            let number = 1000 - i;
            let title = if i == 1 {
                "Fix\tthe\u{202e} build\nnow".to_owned()
            } else {
                format!("Change number {i}")
            };
            json!({
                "number": number,
                "title": title,
                "draft": i % 5 == 0,
                "repository_url": format!("{API}/repos/{repo}"),
                "user": {"login": format!("dev{}", i % 3)},
                "html_url": format!("https://github.com/{repo}/pull/{number}")
            })
        })
        .collect()
}

fn check_runs(reference: &str) -> Vec<Value> {
    let count = match reference {
        "main" => 7,
        "sha9" => 130,
        "huge" => 620,
        _ => 0,
    };
    (0..count)
        .map(|i| {
            let (status, conclusion) = match i % 7 {
                0 => ("in_progress", Value::Null),
                1 => ("completed", json!("failure")),
                2 => ("completed", json!("cancelled")),
                3 => ("completed", json!("neutral")),
                4 => ("completed", json!("skipped")),
                _ => ("completed", json!("success")),
            };
            json!({"name": format!("check-{i}"), "status": status, "conclusion": conclusion})
        })
        .collect()
}

fn statuses(reference: &str) -> (Vec<Value>, &'static str) {
    match reference {
        "main" => (
            vec![
                json!({"context": "ci/a", "state": "success"}),
                json!({"context": "ci/b", "state": "pending"}),
                json!({"context": "ci/c\u{7}", "state": "error"}),
            ],
            "0123abc",
        ),
        "sha9" => (
            vec![json!({"context": "ci/legacy", "state": "failure"})],
            "sha9",
        ),
        _ => (Vec::new(), "f00d"),
    }
}

/// The stand-in GitHub: the answer to `GET <base><target>`, where
/// `target` is the path and query. The plugin's answers (on
/// `https://api.github.com`) and the built-in's (on 127.0.0.1) come from
/// here, so both see the same data.
fn answer(base: &str, target: &str) -> Answer {
    let path = target.split('?').next().unwrap();
    let not_found = || json_answer(404, &json!({"message": "Not Found"}));
    match path {
        "/user" => json_answer(
            200,
            &json!({"login": "octocat", "name": "Mona\nOcto", "html_url": "https://github.com/octocat"}),
        )
        .header("x-ratelimit-remaining", "4321"),
        "/repos/octo/site" => json_answer(
            200,
            &json!({
                "full_name": "octo/site", "default_branch": "main", "private": true,
                "archived": false, "open_issues_count": 12, "stargazers_count": 345,
                "forks_count": 6, "html_url": "https://github.com/octo/site",
                "description": "The\u{202e} web site\nof octo", "pushed_at": "2026-11-15T08:00:00Z"
            }),
        ),
        "/repos/octo/bare" => json_answer(
            200,
            &json!({
                "full_name": "octo/bare", "default_branch": "trunk", "private": false,
                "archived": true, "open_issues_count": 0, "description": null, "pushed_at": null
            }),
        ),
        "/repos/octo/forbidden" => json_answer(
            403,
            &json!({"message": "Resource protected by organization SAML enforcement"}),
        ),
        "/repos/octo/broken" => json_answer(502, &json!({"message": "Server Error"})),
        "/repos/octo/limited" => json_answer(403, &json!({"message": "API rate limit exceeded"}))
            .header("x-ratelimit-remaining", "0")
            .header("x-ratelimit-reset", "1000000000"),
        "/repos/octo/secondary" => json_answer(429, &json!({})).header("retry-after", "30"),
        "/repos/octo/revoked" => json_answer(401, &json!({"message": "Bad credentials"})),
        "/repos/octo/html" => Answer {
            status: 200,
            headers: vec![
                ("content-type".into(), "text/html".into()),
                ("date".into(), DATE.into()),
            ],
            body: "<html>a captive portal</html>".into(),
        },
        "/repos/octo/shape" => json_answer(200, &json!({"name": "shape"})),
        "/repos/octo/site/pulls/9" => json_answer(
            200,
            &json!({"head": {"sha": "sha9"}, "html_url": "https://github.com/octo/site/pull/9"}),
        ),
        "/search/issues" => {
            let items = search_items(&query(target)["q"]);
            page(
                base,
                target,
                "items",
                &items,
                json!({"total_count": items.len(), "incomplete_results": false}),
            )
        }
        _ => {
            let Some(rest) = path.strip_prefix("/repos/octo/site/commits/") else {
                return not_found();
            };
            match rest.split_once('/') {
                Some((reference, "check-runs")) => {
                    let runs = check_runs(&decode(reference));
                    page(
                        base,
                        target,
                        "check_runs",
                        &runs,
                        json!({"total_count": runs.len()}),
                    )
                }
                Some((reference, "status")) => {
                    let (list, sha) = statuses(&decode(reference));
                    page(
                        base,
                        target,
                        "statuses",
                        &list,
                        json!({"sha": sha, "state": "pending"}),
                    )
                }
                _ => not_found(),
            }
        }
    }
}

// ------------------------------------------------------------ scenarios

/// An action, its parameters, and a name for messages.
struct Scenario {
    name: &'static str,
    action: &'static str,
    params: Value,
}

fn scenario(name: &'static str, action: &'static str, params: Value) -> Scenario {
    Scenario {
        name,
        action,
        params,
    }
}

/// Every read action, on success and on each failure GitHub can answer.
fn readings() -> Vec<Scenario> {
    let repository = |name, repo: &str| scenario(name, "repository", json!({"repository": repo}));
    vec![
        scenario("current-user", "current-user", json!({})),
        repository("repository octo/site", "octo/site"),
        repository("repository octo/bare (fallbacks)", "octo/bare.git"),
        repository("repository not a repository", "not a repo"),
        repository("repository octo/missing", "octo/missing"),
        repository("repository octo/forbidden", "octo/forbidden"),
        repository("repository octo/broken", "octo/broken"),
        repository("repository octo/limited", "octo/limited"),
        repository("repository octo/secondary", "octo/secondary"),
        repository("repository octo/revoked", "octo/revoked"),
        repository("repository octo/html", "octo/html"),
        repository("repository octo/shape", "octo/shape"),
        scenario(
            "pull-requests of a repository",
            "pull-requests",
            json!({"repository": "octo/site"}),
        ),
        scenario(
            "pull-requests, 70 listed",
            "pull-requests",
            json!({"repository": "octo/site", "limit": 70}),
        ),
        scenario(
            "pull-requests, 100 listed",
            "pull-requests",
            json!({"repository": "octo/site", "limit": 100}),
        ),
        scenario(
            "pull-requests, mine",
            "pull-requests",
            json!({"filter": "author-me"}),
        ),
        scenario(
            "pull-requests, review requested in a repository",
            "pull-requests",
            json!({"repository": "octo/site", "filter": "review-requested", "limit": 1}),
        ),
        scenario(
            "pull-requests, all without a repository",
            "pull-requests",
            json!({"filter": "all"}),
        ),
        scenario("review-requests", "review-requests", json!({})),
        scenario(
            "review-requests in a repository",
            "review-requests",
            json!({"repository": "octo/site", "limit": 1}),
        ),
        scenario(
            "checks of a branch",
            "checks",
            json!({"repository": "octo/site", "ref": "main"}),
        ),
        scenario(
            "checks of a pull request (paged)",
            "checks",
            json!({"repository": "octo/site", "pullRequest": 9, "ref": "ignored"}),
        ),
        scenario(
            "checks beyond 500 (truncated)",
            "checks",
            json!({"repository": "octo/site", "ref": "huge"}),
        ),
        scenario(
            "checks of a missing pull request",
            "checks",
            json!({"repository": "octo/site", "pullRequest": 404}),
        ),
        scenario(
            "checks of an invalid ref",
            "checks",
            json!({"repository": "octo/site", "ref": "a..b"}),
        ),
        scenario(
            "checks without a ref",
            "checks",
            json!({"repository": "octo/site"}),
        ),
    ]
}

/// What both sides answered, as the engine sees an action's result.
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

fn from_plugin(outcome: PluginActionOutcome) -> Outcome {
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

// ------------------------------------------------------------- built-in

/// The stand-in GitHub on 127.0.0.1 for the built-in module: [`answer`]
/// with links on its own address.
struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = support::http::bind_local();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let (stop, seen) = (stop.clone(), seen.clone());
            std::thread::spawn(move || {
                let base = format!("http://127.0.0.1:{port}");
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            serve(stream, &base, &seen);
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
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(stream: std::net::TcpStream, base: &str, seen: &Mutex<Vec<String>>) {
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
    seen.lock()
        .unwrap()
        .push(format!("{target}\t{authorization}"));
    let answer = answer(base, &target);
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

/// The built-in module, configured with `config` and the launcher, as the
/// agent registers it.
fn builtin(config: &ActionParameters, launcher: &Arc<RecordingLauncher>) -> ModuleRegistry {
    let mut modules = ModuleRegistry::new(Os::current());
    modules
        .register(Box::new(github::GithubModule::with_launcher(
            launcher.clone(),
        )))
        .unwrap();
    modules.set_config(&github::module_id(), config).unwrap();
    modules
}

fn parameters(params: &Value) -> ActionParameters {
    let mut out = ActionParameters::new();
    for (key, value) in params.as_object().unwrap() {
        let value = match value {
            Value::String(s) if key == "filter" => ParameterValue::Choice(s.clone()),
            Value::String(s) => ParameterValue::Text(s.clone()),
            Value::Number(n) => ParameterValue::Integer(n.as_i64().unwrap()),
            other => panic!("{other:?}"),
        };
        out = out.with(key.clone(), value);
    }
    out
}

/// Runs a built-in action as the agent does (normalized, through the
/// registry), with `token` as the stored `github.token`.
fn run_builtin(
    modules: &ModuleRegistry,
    action: &str,
    params: &Value,
    token: Option<&str>,
) -> Outcome {
    let registry = modules.action_registry();
    let id = ActionTypeId::new(format!("github.{action}")).unwrap();
    let normalized = registry.normalize(&id, &parameters(params)).unwrap();
    let profile = Profile::new("Parity");
    let machine = MachineConfig::new(Os::current());
    let mut credentials = MemoryCredentials::new();
    if let Some(token) = token {
        credentials = credentials.with(
            CredentialId::new("github.token").unwrap(),
            Secret::new(token),
        );
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
fn blocking<T>(run: impl FnOnce() -> T) -> T {
    tokio::task::block_in_place(run)
}

/// The built-in's web address for its API on 127.0.0.1 is that address;
/// GitHub's is github.com.
fn on_github(mut outcome: Outcome, server: &Server) -> Outcome {
    let local = format!("{}/", server.base());
    let fix = |s: &str| s.replace(&local, "https://github.com/");
    outcome.message = fix(&outcome.message);
    for value in outcome.data.values_mut() {
        *value = fix(value);
    }
    outcome
}

// ---------------------------------------------------------------- agent

/// The plugin's canned answers: [`answer`] for every URL the plugin asks
/// for in [`readings`], found by following the same requests and links.
fn plugin_routes() -> Value {
    let mut urls: Vec<String> = [
        "/user",
        "/repos/octo/site",
        "/repos/octo/bare",
        "/repos/octo/missing",
        "/repos/octo/forbidden",
        "/repos/octo/broken",
        "/repos/octo/limited",
        "/repos/octo/secondary",
        "/repos/octo/revoked",
        "/repos/octo/html",
        "/repos/octo/shape",
        "/repos/octo/site/pulls/9",
        "/repos/octo/site/pulls/404",
    ]
    .iter()
    .map(|p| format!("{API}{p}"))
    .collect();
    let search = |q: &str, per_page: usize| {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("q", q),
                ("sort", "updated"),
                ("order", "desc"),
                ("per_page", &per_page.to_string()),
            ])
            .finish();
        format!("{API}/search/issues?{query}")
    };
    let mut lists = vec![
        search("is:pr is:open repo:octo/site", 20),
        search("is:pr is:open repo:octo/site", 50),
        search("is:pr is:open archived:false author:@me", 20),
        search("is:pr is:open repo:octo/site review-requested:@me", 1),
        search("is:pr is:open archived:false review-requested:@me", 20),
    ];
    for reference in ["main", "sha9", "huge"] {
        let commits = format!("{API}/repos/octo/site/commits/{reference}");
        lists.push(format!("{commits}/check-runs?per_page=50&filter=latest"));
        lists.push(format!("{commits}/status?per_page=50"));
    }
    for first in lists {
        let mut next = Some(first);
        let mut pages = 0;
        while let Some(url) = next.take() {
            pages += 1;
            let target = url.strip_prefix(API).unwrap().to_owned();
            urls.push(url);
            if pages < 12
                && let Some((_, link)) = answer(API, &target)
                    .headers
                    .iter()
                    .find(|(n, _)| n == "link")
            {
                next = Some(
                    link.split(">; rel=\"next\"")
                        .next()
                        .unwrap()
                        .trim_start_matches('<')
                        .to_owned(),
                );
            }
        }
    }
    // Longest first: the canned transport takes the first route whose URL
    // starts the request's.
    urls.sort_by_key(|u| std::cmp::Reverse(u.len()));
    urls.dedup();
    let mut routes: Vec<Value> = NETWORK_FAILURES
        .iter()
        .map(|(repository, code)| {
            json!({"url": format!("{API}/repos/{repository}"), "status": 0, "body": "", "fail": code})
        })
        .collect();
    routes.extend(urls.iter().map(|url| {
        let a = answer(API, url.strip_prefix(API).unwrap());
        json!({"url": url, "status": a.status, "headers": a.headers, "body": a.body})
    }));
    // The GitHub Enterprise Server's: the signed-in user only.
    let a = answer(GHE_API, "/user");
    routes.push(json!({"url": format!("{GHE_API}/user"), "status": a.status,
                       "headers": a.headers, "body": a.body}));
    Value::Array(routes)
}

/// Repositories whose request gets no answer, and the transport's code:
/// canned for the plugin, real for the built-in (a name under `.invalid`,
/// which never resolves; a local HTTPS server whose throwaway CA the
/// built-in does not trust).
const NETWORK_FAILURES: &[(&str, &str)] = &[
    ("octo/unresolved", "dns-failed"),
    ("octo/untrusted", "tls-failed"),
];

struct Env {
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
}

impl Env {
    fn new() -> Self {
        let dir = TempDir::new("plugins-github");
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let env = Self { dir, paths, key };
        env.stage();
        std::fs::write(
            env.http(),
            serde_json::to_vec_pretty(&plugin_routes()).unwrap(),
        )
        .unwrap();
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

    fn opened(&self) -> PathBuf {
        self.dir.path().join("opened.txt")
    }

    /// `bundled/deckpilot.github/` as `plugins/first-party/build.sh`
    /// stages it, and the pins the agent "was compiled with".
    fn stage(&self) {
        let dir = self.bundled().join(ID);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for file in ["manifest.json", "LICENSE", "assets/icon-128.png"] {
            std::fs::copy(source().join(file), dir.join(file)).unwrap();
        }
        std::fs::copy(&artifacts().module, dir.join("plugin.wasm")).unwrap();
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let version = semver::Version::parse(manifest["version"].as_str().unwrap()).unwrap();
        // `default`, not the release's `migration`: only the migration
        // engine installs a migration-class plugin.
        let pins = Pins::compute(
            &self.bundled(),
            &[(PluginId::parse(ID).unwrap(), version, BundleClass::Default)],
        )
        .unwrap();
        std::fs::write(self.bundled().join("pins.json"), pins.to_json_pretty()).unwrap();
        std::fs::write(self.pins_file(), pins.to_json_pretty()).unwrap();
    }

    async fn start(&self) -> Agent {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.path().join("agent.log"))
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
            .args(["--headless", "--no-usb", "--log-level", "debug"])
            .arg("--enable-plugins")
            .arg("--plugin-host")
            .arg(&artifacts().host)
            .arg("--bundled-plugins")
            .arg(self.bundled())
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_BUNDLED_PINS", self.pins_file())
            .env("DECKPILOT_TEST_PLUGIN_HTTP", self.http())
            .env("DECKPILOT_TEST_PLUGIN_OPEN_URL", self.opened())
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
                    assert!(Instant::now() < deadline, "the agent did not start: {e}");
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

    async fn invoke(&self, action: &str, params: &Value) -> Outcome {
        match self
            .request(Request::InvokePluginAction {
                plugin: ID.into(),
                action: action.into(),
                params: serde_json::from_value(params.clone()).unwrap(),
            })
            .await
            .unwrap_or_else(|e| panic!("{action}: {e}\n{}", self.log()))
        {
            Response::PluginActionResult { outcome } => from_plugin(outcome),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn ok(&self, request: Request) {
        let what = request.name();
        if let Err(e) = self.request(request).await {
            panic!("{what}: {e}\n{}", self.log());
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

/// The aliases the cut-over adds (`crates/deckpilot-modules/tests/contracts.rs`).
fn github_alias(action: &str) -> ActionAlias {
    let legacy: &'static str = Box::leak(format!("github.{action}").into_boxed_str());
    let target: &'static str = Box::leak(format!("{ID}.{action}").into_boxed_str());
    ActionAlias {
        legacy,
        target,
        module: "github",
        plugin: ID,
        since: "0.6.0",
        params: if action == "open" {
            &[]
        } else {
            &[ParamRule::DropIfDefault("credential")]
        },
    }
}

fn compare(name: &str, builtin: &Outcome, plugin: &Outcome, problems: &mut Vec<String>) {
    let mut expected = builtin.clone();
    if !builtin.ok && DIFFERENT_MESSAGES.iter().any(|(n, _)| *n == name) {
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

// ---------------------------------------------------------------- test

/// Every action of `deckpilot.github` against the built-in, on the same
/// answers: identical results, the documented differences aside.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // the steps of one comparison, in order
async fn the_plugin_answers_as_the_builtin_does() {
    let env = Env::new();
    let agent = env.start().await;
    let server = Server::start();
    let launcher = Arc::new(RecordingLauncher::new());
    let reads = builtin(
        &ActionParameters::new()
            .with(github::API_URL, ParameterValue::Text(server.base()))
            .with(github::ALLOW_PRIVATE, ParameterValue::Boolean(true)),
        &launcher,
    );
    let opens = builtin(&ActionParameters::new(), &launcher);

    // Seeded, waiting for the user's consent to its permissions.
    let summary = match agent.request(Request::ListPlugins).await.unwrap() {
        Response::Plugins { plugins } => plugins
            .into_iter()
            .find(|p| p.id == ID)
            .unwrap_or_else(|| panic!("not seeded\n{}", agent.log())),
        other => panic!("unexpected {other:?}"),
    };
    assert_ne!(summary.state, PluginState::Enabled, "{}", agent.log());

    // The definitions as the agent's engine registers them: §5.3's rules
    // hold against the built-in's, the check the registry makes before it
    // routes a legacy type to the plugin.
    let registry = opens.action_registry();
    let mut problems = Vec::new();
    for action in &summary.actions {
        let definition = action.definition.as_ref().expect("every action converts");
        let alias = github_alias(&action.action);
        let builtin = registry
            .definition(&alias.legacy_id())
            .unwrap_or_else(|| panic!("{} is not a built-in action", alias.legacy));
        for problem in alias::incompatibilities(&alias, builtin, definition) {
            problems.push(format!("{}: {problem}", alias.legacy));
        }
    }
    assert_eq!(summary.actions.len(), 6);
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    agent
        .ok(Request::EnablePlugin {
            plugin: ID.into(),
            consent: GRANTS.iter().map(|g| (*g).to_owned()).collect(),
        })
        .await;
    match agent
        .request(Request::GetPlugin { plugin: ID.into() })
        .await
        .unwrap()
    {
        response @ Response::Plugin { .. } => {
            let size = serde_json::to_vec(&response).unwrap().len();
            assert!(size < ANSWER_BUDGET, "getPlugin answers {size} bytes");
        }
        other => panic!("unexpected {other:?}"),
    }

    // Without a token neither sends anything.
    let name = "current-user without a token";
    let plugin = agent.invoke("current-user", &json!({})).await;
    let builtin = blocking(|| run_builtin(&reads, "current-user", &json!({}), None));
    assert_eq!(plugin.code.as_deref(), Some("credential-missing"));
    compare(name, &builtin, &plugin, &mut problems);

    agent
        .ok(Request::SetPluginCredential {
            plugin: ID.into(),
            key: "token".into(),
            secret: PluginSecret::new(TOKEN),
        })
        .await;

    let ghe_opened = github_enterprise(&env, &agent).await;

    // Every read, on both.
    let mut compared = Vec::new();
    for s in readings() {
        let plugin = agent.invoke(s.action, &s.params).await;
        let builtin = on_github(
            blocking(|| run_builtin(&reads, s.action, &s.params, Some(TOKEN))),
            &server,
        );
        compare(s.name, &builtin, &plugin, &mut problems);
        compared.push((s.name, builtin.ok, builtin.code));
    }

    // Open: both open exactly the same page and show it without query.
    // (One page per 2 s for a plugin.)
    let pages = [
        (
            "open a pull request",
            json!({"repository": "octo/site", "pullRequest": 12}),
        ),
        (
            "open a page",
            json!({"url": " https://GitHub.com/octo/site/pull/12/files?diff=split#top "}),
        ),
        (
            "open another host",
            json!({"url": "https://gist.github.com/octo/1"}),
        ),
        ("open nothing", json!({})),
    ];
    for (i, (name, params)) in pages.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(2_100)).await;
        }
        let plugin = agent.invoke("open", params).await;
        let builtin = blocking(|| run_builtin(&opens, "open", params, None));
        compare(name, &builtin, &plugin, &mut problems);
        compared.push((name, builtin.ok, builtin.code));
    }
    let started: Vec<String> = launcher
        .plans()
        .iter()
        .map(|p| p.args.last().unwrap().clone())
        .collect();
    let opened: Vec<String> = std::fs::read_to_string(env.opened())
        .unwrap_or_default()
        .lines()
        .skip(ghe_opened)
        .map(str::to_owned)
        .collect();
    assert_eq!(opened, started);
    assert_eq!(
        opened,
        [
            "https://github.com/octo/site/pull/12",
            "https://github.com/octo/site/pull/12/files?diff=split#top"
        ]
    );

    assert!(
        problems.is_empty(),
        "{}\n{}",
        problems.join("\n"),
        agent.log()
    );
    // The scenarios cover success and every failure code of the module.
    let codes: std::collections::BTreeSet<_> =
        compared.iter().filter_map(|(_, _, c)| c.clone()).collect();
    for code in [
        "invalid-repository",
        "invalid-ref",
        "unauthorized",
        "forbidden",
        "not-found",
        "rate-limited",
        "github-error",
        "invalid-response",
        "url-not-allowed",
        "invalid-parameters",
    ] {
        assert!(codes.contains(code), "no scenario ends in {code}");
    }

    // What went out: only GET to api.github.com, each with the token as a
    // bearer token, and the same requests as the built-in's but for the
    // page size (50 rather than up to 100).
    let log = std::fs::read_to_string(format!("{}.log", env.http().display())).unwrap();
    // The GitHub Enterprise Server's requests are checked by
    // `github_enterprise`.
    let sent: Vec<&str> = log
        .lines()
        .filter(|l| !l.starts_with(&format!("{GHE_API}/")))
        .collect();
    assert_ne!(sent.len(), 0);
    for line in &sent {
        let (url, auth) = line.split_once('\t').unwrap_or((line, ""));
        assert!(url.starts_with("https://api.github.com/"), "{line}");
        assert_eq!(auth, format!("authorization: Bearer {TOKEN}"), "{url}");
    }
    let seen = server.seen.lock().unwrap().clone();
    for line in &seen {
        assert!(line.ends_with(&format!("\tBearer {TOKEN}")), "{line}");
    }
    let paths = |lines: &[String]| {
        let mut paths: Vec<String> = lines
            .iter()
            .map(|l| {
                let target = l.split('\t').next().unwrap();
                let target = target.strip_prefix(API).unwrap_or(target);
                target.split('?').next().unwrap().to_owned()
            })
            .collect();
        paths.dedup();
        paths
    };
    let plugin_paths = paths(&sent.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    assert_eq!(plugin_paths, paths(&seen));

    network_failures(&env, &agent).await;
}

fn sent(env: &Env) -> Vec<String> {
    std::fs::read_to_string(format!("{}.log", env.http().display()))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn opened(env: &Env) -> Vec<String> {
    std::fs::read_to_string(env.opened())
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

async fn settings(agent: &Agent, api_url: Value) -> PluginSettings {
    match agent
        .request(Request::SetPluginSettings {
            plugin: ID.into(),
            values: BTreeMap::from([("apiUrl".to_owned(), api_url)]),
        })
        .await
        .unwrap_or_else(|e| panic!("setPluginSettings: {e}\n{}", agent.log()))
    {
        Response::PluginSettings { settings } => *settings,
        other => panic!("unexpected {other:?}"),
    }
}

/// GitHub Enterprise Server in `apiUrl`, the plugin's user-origin slot:
/// nothing is sent there and nothing opened before the user allowed
/// exactly that server, a private one needs its own consent, and a
/// revoked consent refuses the very next call. Back on api.github.com,
/// no consent is needed. Returns how many pages it opened.
async fn github_enterprise(env: &Env, agent: &Agent) -> usize {
    let entered = settings(agent, json!("https://GHE.example.com:443/api/v3/")).await;
    let origin = &entered.origins[0];
    assert_eq!(origin.setting, "apiUrl");
    assert_eq!(origin.origin.as_deref(), Some(GHE_API), "{origin:?}");
    assert_eq!(origin.status, UserOriginStatus::NeedsConsent, "{origin:?}");
    assert_eq!(origin.credentials, ["token"]);
    assert!(origin.opens_pages && !origin.private_host, "{origin:?}");
    assert!(origin.private_network_allowed, "{origin:?}");

    // Entered, not allowed: no request, no token, no page.
    let before = sent(env);
    let refused = agent.invoke("current-user", &json!({})).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("not-permitted"),
        "{refused:?}"
    );
    let refused = agent
        .invoke("open", &json!({"repository": "octo/site"}))
        .await;
    assert_eq!(
        refused.code.as_deref(),
        Some("not-permitted"),
        "{refused:?}"
    );
    assert_eq!(sent(env), before, "a request before the consent");
    assert_eq!(
        opened(env),
        Vec::<String>::new(),
        "a page before the consent"
    );

    // The user allows exactly that server.
    agent
        .ok(Request::GrantPluginUserOrigin {
            plugin: ID.into(),
            setting: "apiUrl".into(),
            origin: GHE_API.into(),
            private_network: false,
        })
        .await;
    let user = agent.invoke("current-user", &json!({})).await;
    assert!(user.ok, "{user:?}\n{}", agent.log());
    assert_eq!(user.data["login"], "octocat");
    let new = sent(env)[before.len()..].to_vec();
    assert_eq!(
        new,
        [format!("{GHE_API}/user\tauthorization: Bearer {TOKEN}")],
        "only the allowed server, with the token"
    );
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    let page = agent
        .invoke(
            "open",
            &json!({"repository": "octo/site", "pullRequest": 12}),
        )
        .await;
    assert!(page.ok, "{page:?}");
    assert_eq!(opened(env), ["https://ghe.example.com/octo/site/pull/12"]);
    // github.com is not that server's web host.
    let other = agent
        .invoke("open", &json!({"url": "https://github.com/octo/site"}))
        .await;
    assert_eq!(other.code.as_deref(), Some("url-not-allowed"), "{other:?}");

    // Revoked: refused on the very next call.
    agent
        .ok(Request::RevokePluginUserOrigin {
            plugin: ID.into(),
            setting: "apiUrl".into(),
        })
        .await;
    let before = sent(env);
    let refused = agent.invoke("current-user", &json!({})).await;
    assert_eq!(
        refused.code.as_deref(),
        Some("not-permitted"),
        "{refused:?}"
    );
    assert_eq!(sent(env), before, "a request after the consent was revoked");

    // A server on a private network: allowing it needs the separate
    // private-network consent.
    let private = settings(agent, json!("https://10.0.0.5/api/v3")).await;
    assert!(private.origins[0].private_host, "{:?}", private.origins);
    let refused = agent
        .request(Request::GrantPluginUserOrigin {
            plugin: ID.into(),
            setting: "apiUrl".into(),
            origin: "https://10.0.0.5/api/v3".into(),
            private_network: false,
        })
        .await;
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.to_string().contains("private network")),
        "{refused:?}"
    );

    // Back to api.github.com: no consent of its own.
    let public = settings(agent, Value::Null).await;
    assert_eq!(public.effective["apiUrl"], json!(API));
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    1
}

/// No answer at all: a name that does not resolve and a certificate that
/// is not trusted end in the built-in's transport codes on both sides,
/// with the transport's retryability for the plugin (`dns-failed`
/// retryable, `tls-failed` not). The messages are each side's
/// explanation of the same failure; neither names the token.
async fn network_failures(env: &Env, agent: &Agent) {
    let launcher = Arc::new(RecordingLauncher::new());
    let root = env.dir.path().join("https-root");
    std::fs::create_dir_all(&root).unwrap();
    let untrusted = https::HttpsServer::start(&root);
    let unresolved = builtin(
        &ActionParameters::new().with(
            github::API_URL,
            ParameterValue::Text("https://api.github.invalid".into()),
        ),
        &launcher,
    );
    let untrusted_api = builtin(
        &ActionParameters::new()
            .with(
                github::API_URL,
                ParameterValue::Text(format!("https://127.0.0.1:{}", untrusted.port)),
            )
            .with(github::ALLOW_PRIVATE, ParameterValue::Boolean(true)),
        &launcher,
    );
    for (repository, code) in NETWORK_FAILURES {
        let params = json!({"repository": repository});
        let modules = if *code == "dns-failed" {
            &unresolved
        } else {
            &untrusted_api
        };
        let builtin = blocking(|| run_builtin(modules, "repository", &params, Some(TOKEN)));
        assert_eq!(
            builtin.code.as_deref(),
            Some(*code),
            "the built-in: {builtin:?}"
        );
        let outcome = match agent
            .request(Request::InvokePluginAction {
                plugin: ID.into(),
                action: "repository".into(),
                params: serde_json::from_value(params.clone()).unwrap(),
            })
            .await
            .unwrap()
        {
            Response::PluginActionResult { outcome } => outcome,
            other => panic!("unexpected {other:?}"),
        };
        eprintln!("network {repository}: {outcome:?}");
        let PluginActionOutcome::Failed {
            code: plugin_code,
            message,
            retryable,
        } = &outcome
        else {
            panic!("{repository}: {outcome:?}\n{}", agent.log());
        };
        assert_eq!(plugin_code, code, "{outcome:?}");
        assert_eq!(*retryable, *code == "dns-failed", "{outcome:?}");
        for text in [message.as_deref().unwrap_or_default(), &builtin.message] {
            assert!(!text.contains(TOKEN), "{text}");
            assert_ne!(text, "");
        }
    }
}
