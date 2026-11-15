//! The example plugins (`plugins/examples/hello-world` and
//! `plugins/examples/developer-tools`) through the real agent and the real
//! sandboxed host (feature `plugins`):
//!
//! * each example is packaged from its own directory with the publishing
//!   tool's `package` command (twice: the packages must be identical),
//!   published into a local registry signed with throwaway keys, installed,
//!   enabled with exactly its declared permissions and invoked over IPC
//!   protocol 10, like the configuration app does;
//! * HTTP goes to canned responses (`DECKPILOT_TEST_PLUGIN_HTTP`, feature
//!   `test-hooks`), never to the real GitHub API; the canned transport logs
//!   what would have gone on the wire, injected credentials included, so
//!   the test sees exactly what the broker let out;
//! * the plugin answers (`listPlugins`, `getPlugin`) must stay well below
//!   the IPC message limit (`MAX_MESSAGE_BYTES`, 64 KiB).
#![cfg(feature = "plugins")]

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use deckpilot_core::variables::{ReferenceStatus, ValueSource, VariableValue};
use deckpilot_ipc::framing::MAX_MESSAGE_BYTES;
use deckpilot_ipc::plugins::{PermissionStatus, PluginActionOutcome, PluginDetails, PluginState};
use deckpilot_ipc::{
    AgentClient, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response, ServerMessage,
};
use deckpilot_plugin_registry::publish::{Io, run};
use deckpilot_storage::DatabaseKey;
use serde_json::{Value, json};
use support::TempDir;

const HELLO: &str = "acme.hello-world";
const TOOLS: &str = "acme.developer-tools";
const TOOLS_GRANTS: &[&str] = &[
    "network:api.github.com",
    "credentials:githubToken",
    "notifications",
];
/// The token the user stores for Developer Tools.
const TOKEN: &str = "ghp-test-7c41e0d9-not-a-real-token";
const STARTUP: Duration = Duration::from_secs(30);
/// What the plugin answers may use of the IPC message limit: well below
/// it, so that a few more actions or outputs do not break the app.
const ANSWER_BUDGET: usize = MAX_MESSAGE_BYTES / 2;

const SEARCH_URL: &str =
    "https://api.github.com/search/issues?q=repo%3Aacme%2Fapp%20is%3Apr%20is%3Aopen&per_page=1";
const RUNS_URL: &str = "https://api.github.com/repos/acme/app/actions/runs?branch=main&per_page=1";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime() -> PathBuf {
    repo().join("plugins/runtime")
}

fn example(name: &str) -> PathBuf {
    repo().join("plugins/examples").join(name)
}

// ------------------------------------------------------------ artifacts

struct Artifacts {
    host: PathBuf,
    hello: PathBuf,
    tools: PathBuf,
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

/// The host and both modules, built from `plugins/runtime` (or taken from
/// `DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR`).
fn artifacts() -> &'static Artifacts {
    static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let wasm_dir = std::env::var_os("DECKPILOT_E2E_WASM_DIR").map_or_else(
            || {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    "hello-world",
                    "-p",
                    "developer-tools",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--profile",
                    "guest",
                ]);
                runtime().join("target/wasm32-unknown-unknown/guest")
            },
            PathBuf::from,
        );
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
            hello: wasm_dir.join("hello_world.wasm"),
            tools: wasm_dir.join("developer_tools.wasm"),
        }
    })
}

// ------------------------------------------------------------- registry

/// `deckpilot-registry` as a library (no process, no environment).
fn registry_tool(args: &[String]) -> String {
    let env = |_: &str| None;
    let mut input: &[u8] = b"";
    let mut out = Vec::new();
    let mut io = Io {
        env: &env,
        stdin: &mut input,
        out: &mut out,
    };
    run(args, &mut io).unwrap_or_else(|e| panic!("deckpilot-registry {args:?}: {e}"));
    String::from_utf8(out).unwrap()
}

/// A local registry with both examples, signed with throwaway keys.
struct Registry {
    _dir: TempDir,
    work: PathBuf,
    roots: PathBuf,
}

impl Registry {
    #[allow(clippy::too_many_lines)] // the registry tool's steps, in order
    fn create() -> Self {
        let artifacts = artifacts();
        let dir = TempDir::new("plugins-examples-registry");
        let work = dir.path().join("registry");
        let keys = dir.path().join("keys");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&keys).unwrap();
        let key = |name: &str| keys.join(name).display().to_string();
        let in_registry = |args: &[&str]| {
            let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
            all.extend(["--dir".to_owned(), work.display().to_string()]);
            registry_tool(&all)
        };
        for role in ["root", "index", "timestamp"] {
            registry_tool(&[
                "keys".into(),
                "generate".into(),
                "--role".into(),
                role.into(),
                "--out".into(),
                key(&format!("{role}.key")),
                "--no-password".into(),
            ]);
        }
        in_registry(&[
            "keyset",
            "init",
            "--root",
            &key("root.key.pub"),
            "--index",
            &key("index.key.pub"),
            "--timestamp",
            &key("timestamp.key.pub"),
        ]);
        in_registry(&["keyset", "sign", "--key", &key("root.key")]);
        in_registry(&[
            "publisher",
            "--namespace",
            "acme",
            "--github-id",
            "1001",
            "--login",
            "acme",
            "--verified",
        ]);
        for (name, id, module) in [
            ("hello-world", HELLO, &artifacts.hello),
            ("developer-tools", TOOLS, &artifacts.tools),
        ] {
            // The documented invocation, straight from the example's
            // directory, twice: packaging is deterministic.
            let package = |out: &Path| {
                let report = registry_tool(&[
                    "package".into(),
                    "--plugin-dir".into(),
                    example(name).display().to_string(),
                    "--module".into(),
                    module.display().to_string(),
                    "--out".into(),
                    out.display().to_string(),
                ]);
                assert!(report.contains("packaged"), "{report}");
                std::fs::read(out).unwrap()
            };
            let first = keys.join(format!("{name}.dpplugin"));
            let again = package(&keys.join(format!("{name}-again.dpplugin")));
            assert_eq!(
                package(&first),
                again,
                "{name}: packaging is not deterministic"
            );
            let manifest: Value = serde_json::from_slice(
                &std::fs::read(example(name).join("manifest.json")).unwrap(),
            )
            .unwrap();
            let detail = keys.join(format!("{id}.plugin.json"));
            std::fs::write(
                &detail,
                serde_json::to_vec(&json!({
                    "id": id,
                    "name": manifest["name"],
                    "summary": "An example plugin",
                    "description": manifest["description"],
                    "license": "Apache-2.0",
                    "homepage": "https://acme.example/plugins",
                    "source_repository": "acme/plugins",
                    "categories": manifest["categories"]
                }))
                .unwrap(),
            )
            .unwrap();
            in_registry(&[
                "add-version",
                "--package",
                &first.display().to_string(),
                "--plugin-detail",
                &detail.display().to_string(),
                "--source-repository",
                "acme/plugins",
                "--tag",
                "v0.1.0",
                "--commit",
                &"1".repeat(40),
                "--builder",
                "1.0.0",
            ]);
        }
        in_registry(&["build-index", "--source-commit", &"a".repeat(40)]);
        in_registry(&["sign-index", "--key", &key("index.key")]);
        in_registry(&["timestamp", "--key", &key("timestamp.key")]);
        let verified = in_registry(&["verify-site", "--require-packages"]);
        assert!(verified.contains("verified"), "{verified}");
        let roots = keys.join("trusted-roots.json");
        std::fs::copy(work.join("roots.json"), &roots).unwrap();
        Self {
            _dir: dir,
            work,
            roots,
        }
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::create)
}

// ---------------------------------------------------------------- agent

/// Canned GitHub answers: never the real API.
fn canned_http(dir: &Path) -> PathBuf {
    let path = dir.join("http.json");
    let date = ["date", "Sun, 15 Nov 2026 09:00:00 GMT"];
    std::fs::write(
        &path,
        serde_json::to_vec(&json!([
            {
                "url": SEARCH_URL,
                "status": 200,
                "headers": [["content-type", "application/json"], date,
                    ["x-ratelimit-remaining", "59"]],
                "body": json!({"total_count": 4, "incomplete_results": false,
                    "items": [{"number": 7}]}).to_string()
            },
            {
                "url": RUNS_URL,
                "status": 200,
                "headers": [["content-type", "application/json"], date],
                "body": json!({"total_count": 1, "workflow_runs": [{
                    "name": "CI", "status": "completed", "conclusion": "success",
                    "head_branch": "main",
                    "html_url": "https://github.com/acme/app/actions/runs/1"
                }]}).to_string()
            },
            // A careless server that echoes the token: the plugin must
            // never see it.
            {
                "url": "https://api.github.com/search/issues?q=repo%3Aacme%2Fecho",
                "status": 200,
                "headers": [["content-type", "application/json"]],
                "body": json!({"total_count": 1, "echo": TOKEN}).to_string()
            }
        ]))
        .unwrap(),
    )
    .unwrap();
    path
}

fn fetched(http: &Path) -> Vec<String> {
    let mut log = http.as_os_str().to_owned();
    log.push(".log");
    std::fs::read_to_string(PathBuf::from(log))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

struct Agent {
    child: Child,
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
    http: PathBuf,
    profile: deckpilot_core::ProfileId,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Agent {
    /// A fresh data directory with an active profile, an agent with
    /// plugins enabled, unlocked.
    async fn start(name: &str) -> Self {
        let registry = registry();
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        let profile = deckpilot_core::Profile::new("Examples");
        let store = support::open_store(&paths, &key);
        deckpilot_storage::ConfigStore::save_profile(&store, &profile).unwrap();
        deckpilot_storage::ConfigStore::set_active_profile(&store, Some(profile.id)).unwrap();
        drop(store);
        let http = canned_http(dir.path());
        let log = std::fs::File::create(dir.path().join("agent.log")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
            .args(["--headless", "--no-usb", "--log-level", "debug"])
            .arg("--enable-plugins")
            .arg("--plugin-registry")
            .arg(&registry.work)
            .arg("--plugin-registry-roots")
            .arg(&registry.roots)
            .arg("--plugin-host")
            .arg(&artifacts().host)
            .env("DECKPILOT_DATA_DIR", &paths.data_dir)
            .env("DECKPILOT_TEST_PLUGIN_HTTP", &http)
            .env_remove("DECKPILOT_ENABLE_PLUGINS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("the agent starts");
        let agent = Self {
            child,
            dir,
            paths,
            key,
            http,
            profile: profile.id,
        };
        agent
            .request(Request::Unlock {
                key: KeyMaterial::new(agent.key.to_hex()),
            })
            .await
            .unwrap();
        agent
    }

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

    async fn install_and_enable(&self, plugin: &str, consent: &[&str]) {
        self.request(Request::InstallPlugin {
            source: deckpilot_ipc::plugins::PluginInstallSource::Registry {
                plugin: plugin.into(),
                version: None,
            },
            consent: vec![],
        })
        .await
        .unwrap();
        self.request(Request::EnablePlugin {
            plugin: plugin.into(),
            consent: consent.iter().map(|s| (*s).to_owned()).collect(),
        })
        .await
        .unwrap();
    }

    async fn details(&self, plugin: &str) -> (PluginDetails, usize) {
        let response = self
            .request(Request::GetPlugin {
                plugin: plugin.into(),
            })
            .await
            .unwrap();
        let size = wire_size(&response);
        match response {
            Response::Plugin { plugin } => (*plugin, size),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn invoke(&self, plugin: &str, action: &str, params: Value) -> PluginActionOutcome {
        match self
            .request(Request::InvokePluginAction {
                plugin: plugin.into(),
                action: action.into(),
                params: serde_json::from_value(params).unwrap(),
            })
            .await
            .unwrap()
        {
            Response::PluginActionResult { outcome } => outcome,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn run(&self, plugin: &str, action: &str, params: Value) -> BTreeMap<String, Value> {
        match self.invoke(plugin, action, params).await {
            PluginActionOutcome::Ok { data, .. } => data,
            other => panic!("{action}: expected ok, got {other:?}"),
        }
    }

    async fn failed(&self, plugin: &str, action: &str, params: Value) -> (String, String) {
        match self.invoke(plugin, action, params).await {
            PluginActionOutcome::Failed { code, message, .. } => {
                (code, message.unwrap_or_default())
            }
            other => panic!("{action}: expected a failure, got {other:?}"),
        }
    }

    async fn variable(&self, reference: &str) -> ReferenceStatus {
        let response = self
            .request(Request::PreviewReferences {
                context: Box::new(deckpilot_core::variables::ResolutionContext::preview(
                    self.profile,
                )),
                items: vec![deckpilot_core::variables::PreviewItem {
                    reference: reference.to_owned(),
                    expected: None,
                }],
            })
            .await
            .unwrap();
        match response {
            Response::ReferencePreviews { mut previews } => previews.remove(0).status,
            other => panic!("unexpected {other:?}"),
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("agent.log")).unwrap_or_default()
    }
}

/// The bytes of `response` as the agent frames it.
fn wire_size(response: &Response) -> usize {
    serde_json::to_vec(&ServerMessage::Response {
        id: u64::MAX,
        response: response.clone(),
    })
    .unwrap()
    .len()
}

fn resolved(value: VariableValue) -> ReferenceStatus {
    ReferenceStatus::Resolved {
        value,
        source: ValueSource::System,
    }
}

// ---------------------------------------------------------------- tests

/// Hello World: installed and enabled without granting anything, its one
/// action run in the sandboxed host.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hello_world_greets_without_any_permission() {
    let agent = Agent::start("plugins-examples-hello").await;
    agent.install_and_enable(HELLO, &[]).await;
    let (details, size) = agent.details(HELLO).await;
    assert_eq!(details.summary.state, PluginState::Enabled);
    assert!(details.permissions.is_empty(), "{:?}", details.permissions);
    assert!(size < ANSWER_BUDGET, "getPlugin is {size} bytes");

    match agent.invoke(HELLO, "greet", json!({"name": "Ada"})).await {
        PluginActionOutcome::Ok { message, data } => {
            assert_eq!(message.as_deref(), Some("Hello, Ada!"));
            assert_eq!(data["greeting"], "Hello, Ada!");
        }
        other => panic!("{other:?}"),
    }
    // Checked by the agent against the manifest, before the host runs.
    for bad in [
        json!({}),
        json!({"name": "x".repeat(65)}),
        json!({"who": "Ada"}),
    ] {
        let (code, _) = agent.failed(HELLO, "greet", bad).await;
        assert_eq!(code, "invalid-parameters");
    }
    // A blank required text counts as missing for the agent already (the
    // plugin's own check is the second line of defence).
    let (code, message) = agent.failed(HELLO, "greet", json!({"name": "  "})).await;
    assert_eq!(code, "invalid-parameters");
    assert!(message.contains("`name` is required"), "{message}");
    assert!(
        fetched(&agent.http).is_empty(),
        "Hello World made a request"
    );
}

/// Developer Tools: every action through the real agent and host, with
/// exactly the grants it declares; GitHub only through the broker, the
/// token only through its handle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one scenario, step by step
async fn developer_tools_runs_with_only_its_grants() {
    let agent = Agent::start("plugins-examples-tools").await;
    agent.install_and_enable(TOOLS, TOOLS_GRANTS).await;

    // ---- the answers that carry every action's definition fit easily.
    let (details, size) = agent.details(TOOLS).await;
    assert_eq!(details.summary.state, PluginState::Enabled);
    assert_eq!(details.summary.actions.len(), 7);
    assert!(
        size < ANSWER_BUDGET,
        "getPlugin is {size} bytes, budget {ANSWER_BUDGET}"
    );
    let list = agent.request(Request::ListPlugins).await.unwrap();
    let list_size = wire_size(&list);
    assert!(
        list_size < ANSWER_BUDGET,
        "listPlugins is {list_size} bytes, budget {ANSWER_BUDGET}"
    );
    eprintln!("developer-tools: getPlugin {size} bytes, listPlugins {list_size} bytes");
    for key in TOOLS_GRANTS {
        let status = details
            .permissions
            .iter()
            .find(|p| p.key == *key)
            .map(|p| p.status);
        assert_eq!(status, Some(PermissionStatus::Granted), "{key}");
    }

    // ---- pure actions: no request at all.
    let t = agent
        .run(TOOLS, "timestamp", json!({"value": "1731661200"}))
        .await;
    assert_eq!(t["iso"], "2024-11-15T09:00:00Z");
    assert_eq!(t["unixMillis"], json!(1_731_661_200_000_i64));
    let t = agent
        .run(
            TOOLS,
            "timestamp",
            json!({"value": "2024-11-15T10:00:00+01:00"}),
        )
        .await;
    assert_eq!(t["unixSeconds"], json!(1_731_661_200));
    let b = agent
        .run(
            TOOLS,
            "base64",
            json!({"text": "hello?>", "alphabet": "url-safe"}),
        )
        .await;
    assert_eq!(b["result"], "aGVsbG8_Pg==");
    let b = agent
        .run(
            TOOLS,
            "base64",
            json!({"text": "aGVsbG8/Pg==", "mode": "decode"}),
        )
        .await;
    assert_eq!(b["result"], "hello?>");
    let j = agent
        .run(
            TOOLS,
            "json-format",
            json!({"json": "{\"b\":1,\"a\":2}", "style": "pretty"}),
        )
        .await;
    assert_eq!(j["result"], "{\n  \"a\": 2,\n  \"b\": 1\n}");
    let u = agent
        .run(TOOLS, "uuid-v5", json!({"name": "www.example.com"}))
        .await;
    assert_eq!(u["uuid"], "2ed6657d-e927-568b-95e1-2665a8aea6a2");
    let (code, _) = agent
        .failed(TOOLS, "json-format", json!({"json": "{"}))
        .await;
    assert_eq!(code, "invalid-input");
    assert!(
        fetched(&agent.http).is_empty(),
        "a pure action made a request"
    );

    // ---- nothing checked yet; no repository named or configured.
    assert!(matches!(
        agent.invoke(TOOLS, "last-values", json!({})).await,
        PluginActionOutcome::Skipped { .. }
    ));
    let (code, _) = agent.failed(TOOLS, "open-prs", json!({})).await;
    assert_eq!(code, "repo-required");

    // ---- settings: the default repository, read by the plugin.
    agent
        .request(Request::SetPluginSettings {
            plugin: TOOLS.into(),
            values: BTreeMap::from([("defaultRepo".to_owned(), json!("acme/app"))]),
        })
        .await
        .unwrap();

    // ---- without a stored token: no credential on the wire.
    let prs = agent.run(TOOLS, "open-prs", json!({})).await;
    assert_eq!(prs["count"], 4);
    assert_eq!(prs["repo"], "acme/app");
    assert_eq!(prs["checkedAt"], "Sun, 15 Nov 2026 09:00:00 GMT");
    assert_eq!(fetched(&agent.http), [SEARCH_URL.to_owned()]);
    assert_eq!(
        agent.variable("plugins.acme.developerTools.openPrs").await,
        resolved(VariableValue::Integer(4))
    );

    // ---- with a token: injected by the agent as a bearer token, only
    // towards api.github.com; the plugin never sees it.
    agent
        .request(Request::SetPluginCredential {
            plugin: TOOLS.into(),
            key: "githubToken".into(),
            secret: deckpilot_ipc::plugins::PluginSecret::new(TOKEN),
        })
        .await
        .unwrap();
    agent.run(TOOLS, "open-prs", json!({})).await;
    assert_eq!(
        fetched(&agent.http).last().unwrap(),
        &format!("{SEARCH_URL}\tauthorization: Bearer {TOKEN}")
    );
    let echoed = agent
        .invoke(TOOLS, "open-prs", json!({"repo": "acme/echo"}))
        .await;
    assert!(
        !format!("{echoed:?}").contains(TOKEN),
        "an answer carries the token"
    );

    // ---- build status: published, stored, and the last values from
    // storage without a request.
    let build = agent.run(TOOLS, "build-status", json!({})).await;
    assert_eq!(build["status"], "success");
    assert_eq!(build["branch"], "main");
    assert_eq!(build["workflow"], "CI");
    assert_eq!(build["changed"], false);
    assert_eq!(
        agent
            .variable("plugins.acme.developerTools.buildStatus")
            .await,
        resolved(VariableValue::String("success".into()))
    );
    let requests = fetched(&agent.http).len();
    let stored = agent.run(TOOLS, "last-values", json!({})).await;
    assert_eq!(stored["openPrsRepo"], "acme/echo");
    assert_eq!(stored["buildStatus"], "success");
    assert_eq!(stored["buildRepo"], "acme/app");
    assert!(stored["storageBytes"].as_i64().unwrap() > 0, "{stored:?}");
    assert_eq!(fetched(&agent.http).len(), requests);

    // ---- revoking the network grant. The credential slot names
    // api.github.com, which then has no network grant, so the broker
    // refuses the whole grant set (`CredentialOriginNotGranted`) and the
    // plugin gets no grants at all until the network permission is granted
    // again: fail closed, including the always-on settings and storage.
    // No request goes out; the pure actions, which call nothing, still
    // work.
    agent
        .request(Request::RevokePluginPermission {
            plugin: TOOLS.into(),
            permission: "network:api.github.com".into(),
        })
        .await
        .unwrap();
    let (code, message) = agent.failed(TOOLS, "open-prs", json!({})).await;
    assert_eq!(code, "not-permitted", "{message}");
    assert!(message.contains("settings.get"), "{message}");
    let (code, _) = agent.failed(TOOLS, "last-values", json!({})).await;
    assert_eq!(code, "not-permitted");
    assert!(
        agent.log().contains("the broker refused the grants"),
        "the agent did not log why the plugin has no grants"
    );
    assert_eq!(
        fetched(&agent.http).len(),
        requests,
        "a denied request went out"
    );
    let u = agent
        .run(TOOLS, "uuid-v5", json!({"name": "python.org"}))
        .await;
    assert_eq!(u["uuid"], "886313e1-3b8a-5372-9b90-0c9aee199e5d");
    // Granted again: everything is back, on the very next call.
    agent
        .request(Request::GrantPluginPermissions {
            plugin: TOOLS.into(),
            permissions: vec!["network:api.github.com".into()],
        })
        .await
        .unwrap();
    assert_eq!(agent.run(TOOLS, "open-prs", json!({})).await["count"], 4);
    assert_eq!(fetched(&agent.http).len(), requests + 1);

    // ---- the token is in no log and in no file as clear text.
    assert!(!agent.log().contains(TOKEN), "the agent log has the token");
    for entry in walk(&agent.paths.data_dir.join("plugins")) {
        let bytes = std::fs::read(&entry).unwrap();
        assert!(
            !bytes.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()),
            "{} has the token in clear text",
            entry.display()
        );
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}
