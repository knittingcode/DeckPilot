//! The plugin registry over HTTPS, end to end (feature `plugins`): the
//! real agent executable with `--plugin-registry-url`, driven over IPC
//! like the configuration app, the real `deckpilot-plugin-host` and the
//! sample plugin (built from `plugins/runtime` by the test, or taken from
//! `DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR`).
//!
//! The registry is made with the publishing tool and throwaway keys and
//! served by a local TLS server (`support/https.rs`) whose throwaway CA
//! the agent trusts only through `DECKPILOT_TEST_REGISTRY_CA` (feature
//! `test-hooks`, never in a release). Nothing here reaches the real
//! registry or the network.
#![cfg(feature = "plugins")]

mod support;

#[path = "support/https.rs"]
mod https;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use deckpilot_ipc::plugins::{
    PluginActionOutcome, PluginInstallSource, PluginRuntimeStatus, PluginState,
    PluginUpdateOutcome, RegistryFreshness, RegistryStaleReason, reason,
};
use deckpilot_ipc::{
    AgentClient, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_plugin_registry::publish::{Io, run};
use deckpilot_storage::DatabaseKey;
use https::{HttpsServer, Route};
use serde_json::{Value, json};
use support::TempDir;

const HELLO: &str = "acme.hello-weather";
const OTHER: &str = "acme.other-weather";
const NETWORK: &str = "network:api.open-meteo.com";
const STARTUP: Duration = Duration::from_secs(30);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime() -> PathBuf {
    repo().join("plugins/runtime")
}

// ------------------------------------------------------------ artifacts

struct Artifacts {
    host: PathBuf,
    hello: Vec<u8>,
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

fn artifacts() -> &'static Artifacts {
    static ARTIFACTS: OnceLock<Artifacts> = OnceLock::new();
    ARTIFACTS.get_or_init(|| {
        let wasm_dir = std::env::var_os("DECKPILOT_E2E_WASM_DIR").map_or_else(
            || {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    "hello-weather",
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
            hello: std::fs::read(wasm_dir.join("hello_weather.wasm")).unwrap(),
        }
    })
}

// ------------------------------------------------------------- registry

/// A registry made with the publishing tool and throwaway keys: `work`
/// (`site/v1/`, `releases/`) is what the server serves; keys, sources and
/// the trusted roots live apart.
struct Registry {
    dir: TempDir,
}

fn manifest(id: &str, version: &str) -> Value {
    let mut manifest: Value = serde_json::from_slice(
        &std::fs::read(repo().join("plugins/examples/hello-weather/manifest.json")).unwrap(),
    )
    .unwrap();
    manifest["id"] = json!(id);
    manifest["version"] = json!(version);
    manifest["publisher"]["id"] = json!(id.split('.').next().unwrap());
    manifest
}

impl Registry {
    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn keys(&self) -> PathBuf {
        self.dir.path().join("keys")
    }

    fn key(&self, name: &str) -> String {
        self.keys().join(name).display().to_string()
    }

    fn roots(&self) -> PathBuf {
        self.keys().join("trusted-roots.json")
    }

    fn run(&self, args: &[&str]) -> String {
        let mut all: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        if args
            .first()
            .is_none_or(|c| *c != "keys" && *c != "package" && *c != "release")
        {
            all.push("--dir".into());
            all.push(self.work().display().to_string());
        }
        let env = |_: &str| None;
        let mut input: &[u8] = b"";
        let mut out = Vec::new();
        let mut io = Io {
            env: &env,
            stdin: &mut input,
            out: &mut out,
        };
        run(&all, &mut io).unwrap_or_else(|e| panic!("deckpilot-registry {args:?}: {e}"));
        String::from_utf8(out).unwrap()
    }

    fn add(&self, id: &str, version: &str) {
        self.add_with(id, version, &[]);
    }

    /// A `deckpilot.*` version, with the publishing tool's explicit option,
    /// released with the (throwaway) first-party publisher signature next
    /// to the package.
    fn add_first_party(&self, id: &str, version: &str) {
        self.add_unsigned_first_party(id, version);
        self.sign_release(id, version);
    }

    /// A `deckpilot.*` version without the publisher signature.
    fn add_unsigned_first_party(&self, id: &str, version: &str) {
        self.add_with(id, version, &["--first-party"]);
    }

    /// `<package>.minisig`, by the test's publisher key, in `releases/`.
    fn sign_release(&self, id: &str, version: &str) {
        let package = self
            .work()
            .join(Self::release_path(id, version).trim_start_matches('/'));
        let mut signature = package.clone().into_os_string();
        signature.push(".minisig");
        self.run(&[
            "release",
            "sign",
            "--package",
            &package.display().to_string(),
            "--key",
            &self.key("publisher.key"),
            "--out",
            &PathBuf::from(signature).display().to_string(),
        ]);
    }

    /// The publisher keys the agent pins in this test
    /// (`DECKPILOT_TEST_PUBLISHER_KEYS`).
    fn publisher_keys(&self) -> PathBuf {
        self.keys().join("publisher-keys.json")
    }

    fn add_with(&self, id: &str, version: &str, extra: &[&str]) {
        let manifest = manifest(id, version);
        let name = format!("{id}-{version}");
        let src = self.keys().join("src").join(&name);
        std::fs::create_dir_all(src.join("assets")).unwrap();
        std::fs::write(
            src.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let sample = repo().join("plugins/examples/hello-weather");
        std::fs::copy(sample.join("LICENSE"), src.join("LICENSE")).unwrap();
        std::fs::copy(
            sample.join("assets/icon-128.png"),
            src.join("assets/icon-128.png"),
        )
        .unwrap();
        let wasm = src.join("module.wasm");
        std::fs::write(&wasm, &artifacts().hello).unwrap();
        let package = self.keys().join(format!("{name}.dpplugin"));
        let (src, wasm, out) = (
            src.display().to_string(),
            wasm.display().to_string(),
            package.display().to_string(),
        );
        let mut args = vec![
            "package",
            "--plugin-dir",
            &src,
            "--module",
            &wasm,
            "--out",
            &out,
        ];
        args.extend_from_slice(extra);
        self.run(&args);
        let detail = self.keys().join(format!("{id}.plugin.json"));
        std::fs::write(
            &detail,
            serde_json::to_vec(&json!({
                "id": id,
                "name": manifest["name"],
                "summary": "A plugin of the HTTPS registry test",
                "description": manifest["description"],
                "license": "Apache-2.0",
                "homepage": "https://acme.example/plugins",
                "source_repository": "acme/plugins",
                "categories": ["other"]
            }))
            .unwrap(),
        )
        .unwrap();
        let (detail, tag, commit) = (
            detail.display().to_string(),
            format!("v{version}"),
            "1".repeat(40),
        );
        let mut args = vec![
            "add-version",
            "--package",
            &out,
            "--plugin-detail",
            &detail,
            "--source-repository",
            "acme/plugins",
            "--tag",
            &tag,
            "--commit",
            &commit,
            "--builder",
            "1.0.0",
        ];
        args.extend_from_slice(extra);
        self.run(&args);
    }

    /// The `deckpilot` publisher record (the index key's say), verified or
    /// not.
    fn first_party_publisher(&self, verified: bool) {
        let mut args = vec![
            "publisher",
            "--namespace",
            "deckpilot",
            "--github-id",
            "4242",
            "--login",
            "knittingcode",
            "--first-party",
        ];
        if verified {
            args.push("--verified");
        }
        self.run(&args);
    }

    /// A root-signed key set that authorizes the `deckpilot` namespace, or
    /// withdraws the authorization.
    fn authorize_first_party(&self, authorized: bool) {
        self.run(&[
            "keyset",
            "rotate",
            if authorized {
                "--first-party"
            } else {
                "--no-first-party"
            },
        ]);
        self.run(&["keyset", "sign", "--key", &self.key("root.key")]);
    }

    /// Index, its signature and a timestamp (signed now, valid 7 days).
    fn publish(&self) {
        self.run(&["build-index", "--source-commit", &"a".repeat(40)]);
        self.run(&["sign-index", "--key", &self.key("index.key")]);
        self.run(&["timestamp", "--key", &self.key("timestamp.key")]);
    }

    /// Re-signs the timestamp so that it expires `expires_in` from now
    /// (negative: in the past), signed an hour before that.
    fn timestamp_expiring(&self, expires_in: TimeDelta) {
        let signed = Utc::now() + expires_in - TimeDelta::hours(1);
        // The tool never signs earlier than the live timestamp; this one
        // stands for a mirror that froze an older one.
        for file in ["timestamp.json", "timestamp.json.minisig"] {
            std::fs::remove_file(self.work().join("site/v1").join(file)).unwrap();
        }
        self.run(&[
            "timestamp",
            "--key",
            &self.key("timestamp.key"),
            "--now",
            &signed.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "--expires-in",
            "1h",
        ]);
    }

    /// Hello 1.0.0 and 1.1.0 (same permissions) and another plugin.
    fn create() -> Self {
        let registry = Self {
            dir: TempDir::new("plugins-https-registry"),
        };
        std::fs::create_dir_all(registry.keys()).unwrap();
        for role in ["root", "index", "timestamp", "publisher"] {
            registry.run(&[
                "keys",
                "generate",
                "--role",
                role,
                "--out",
                &registry.key(&format!("{role}.key")),
                "--no-password",
            ]);
        }
        registry.run(&[
            "keyset",
            "init",
            "--root",
            &registry.key("root.key.pub"),
            "--index",
            &registry.key("index.key.pub"),
            "--timestamp",
            &registry.key("timestamp.key.pub"),
        ]);
        registry.run(&["keyset", "sign", "--key", &registry.key("root.key")]);
        registry.run(&[
            "publisher",
            "--namespace",
            "acme",
            "--github-id",
            "1001",
            "--login",
            "acme",
        ]);
        registry.add(HELLO, "1.0.0");
        registry.add(HELLO, "1.1.0");
        registry.add(OTHER, "1.0.0");
        registry.publish();
        std::fs::copy(registry.work().join("roots.json"), registry.roots()).unwrap();
        let publisher = std::fs::read_to_string(registry.keys().join("publisher.key.pub")).unwrap();
        std::fs::write(
            registry.publisher_keys(),
            serde_json::to_vec(&json!({
                "publisherKeysVersion": 1,
                "publisher": "deckpilot",
                "keys": [publisher.trim()],
                "threshold": 1,
                "revokedKeys": []
            }))
            .unwrap(),
        )
        .unwrap();
        registry
    }

    /// The served path of a release asset.
    fn release_path(id: &str, version: &str) -> String {
        format!("/releases/{id}--v{version}/{id}-{version}.dpplugin")
    }

    fn release(&self, id: &str, version: &str) -> Vec<u8> {
        std::fs::read(
            self.work()
                .join(Self::release_path(id, version).trim_start_matches('/')),
        )
        .unwrap()
    }

    /// A copy of the site as it is now (for a rollback attack later).
    fn snapshot(&self, name: &str) -> PathBuf {
        let to = self.dir.path().join(name);
        copy_dir(&self.work(), &to);
        to
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

// ---------------------------------------------------------------- agent

/// One test's data directory with an encrypted configuration database.
struct Env {
    dir: TempDir,
    paths: AgentPaths,
    key: DatabaseKey,
}

impl Env {
    fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let paths = AgentPaths::in_dir(dir.path().join("data"));
        paths.create_dirs().unwrap();
        let key = support::create_database(&paths);
        std::fs::write(dir.path().join("http.json"), b"[]").unwrap();
        Self { dir, paths, key }
    }

    fn registry_state(&self) -> PathBuf {
        self.paths.data_dir.join("plugin-registry")
    }

    /// The agent with the registry at `server` (`/site/`, `/releases/`).
    fn start(&self, registry: &Registry, server: &HttpsServer) -> Agent {
        let ca = self.dir.path().join("test-ca.pem");
        server.write_ca(&ca);
        let log = std::fs::File::create(self.dir.path().join(format!(
            "agent-{}.log",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )))
        .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
            .args(["--headless", "--no-usb", "--log-level", "debug"])
            .arg("--enable-plugins")
            .arg("--plugin-registry-url")
            .arg(server.url("site/"))
            .arg("--plugin-registry-releases-url")
            .arg(server.url("releases/"))
            .arg("--plugin-registry-roots")
            .arg(registry.roots())
            .arg("--plugin-host")
            .arg(&artifacts().host)
            .env("DECKPILOT_DATA_DIR", &self.paths.data_dir)
            .env("DECKPILOT_TEST_REGISTRY_CA", &ca)
            .env("DECKPILOT_TEST_PUBLISHER_KEYS", registry.publisher_keys())
            .env(
                "DECKPILOT_TEST_PLUGIN_HTTP",
                self.dir.path().join("http.json"),
            )
            .env_remove("DECKPILOT_ENABLE_PLUGINS")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_URL")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_RELEASES_URL")
            .env_remove("DECKPILOT_PLUGIN_REGISTRY_ROOTS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("the agent starts");
        Agent {
            child,
            paths: self.paths.clone(),
            key: self.key.clone(),
        }
    }

    async fn started(&self, registry: &Registry, server: &HttpsServer) -> Agent {
        let agent = self.start(registry, server);
        agent
            .request(Request::Unlock {
                key: KeyMaterial::new(agent.key.to_hex()),
            })
            .await
            .unwrap();
        agent
    }
}

struct Agent {
    child: Child,
    paths: AgentPaths,
    key: DatabaseKey,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Agent {
    async fn request(&self, request: Request) -> Result<Response, ClientError> {
        let what = request.name();
        let deadline = Instant::now() + STARTUP;
        let mut client = loop {
            match AgentClient::connect(&self.paths, ClientKind::Other).await {
                Ok(client) => break client,
                Err(e) => {
                    assert!(Instant::now() < deadline, "the agent did not start: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(120), client.request(request))
            .await
            .unwrap_or_else(|_| panic!("{what} took too long"))
    }

    async fn stop(mut self) {
        let _ = self.request(Request::Shutdown).await;
        let deadline = Instant::now() + STARTUP;
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the agent did not stop");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn runtime(&self) -> PluginRuntimeStatus {
        match self.request(Request::GetPluginRuntime).await.unwrap() {
            Response::PluginRuntime { status } => *status,
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn available(&self, refresh: bool) -> Result<Response, ClientError> {
        self.request(Request::ListAvailablePlugins { refresh })
            .await
    }

    async fn install(&self, plugin: &str, version: &str) -> Result<Response, ClientError> {
        self.request(Request::InstallPlugin {
            source: PluginInstallSource::Registry {
                plugin: plugin.into(),
                version: Some(version.into()),
            },
            consent: vec![],
        })
        .await
    }

    async fn state(&self, plugin: &str) -> (String, PluginState) {
        match self
            .request(Request::GetPlugin {
                plugin: plugin.into(),
            })
            .await
            .unwrap()
        {
            Response::Plugin { plugin } => (plugin.summary.version, plugin.summary.state),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn installed(&self) -> Vec<String> {
        match self.request(Request::ListPlugins).await.unwrap() {
            Response::Plugins { plugins } => plugins.into_iter().map(|p| p.id).collect(),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn greet(&self) -> PluginActionOutcome {
        let Response::PluginActionResult { outcome } = self
            .request(Request::InvokePluginAction {
                plugin: HELLO.into(),
                action: "greet".into(),
                params: serde_json::from_value(json!({"name": "Ada"})).unwrap(),
            })
            .await
            .unwrap()
        else {
            panic!("an action result")
        };
        outcome
    }
}

fn reason_of(result: &Result<impl std::fmt::Debug, ClientError>) -> &str {
    match result {
        Err(ClientError::Agent(error)) => error.reason.as_deref().unwrap_or("(none)"),
        other => panic!("expected an agent error, got {other:?}"),
    }
}

fn message_of(result: &Result<impl std::fmt::Debug, ClientError>) -> String {
    match result {
        Err(ClientError::Agent(error)) => error.message.clone(),
        other => panic!("expected an agent error, got {other:?}"),
    }
}

fn greeted(outcome: &PluginActionOutcome) -> bool {
    matches!(outcome, PluginActionOutcome::Ok { .. })
}

// ---------------------------------------------------------------- tests

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one flow, step by step
async fn install_update_and_revocation_over_https() {
    let registry = Registry::create();
    let server = HttpsServer::start(&registry.work());
    let env = Env::new("plugins-https-flow");
    let agent = env.started(&registry, &server).await;

    // Verified over HTTPS at unlock.
    let status = agent.runtime().await;
    let source = status.registry.clone().expect("a registry");
    assert_eq!(source.source, server.url("site/"), "{status:?}");
    assert_eq!(source.freshness, RegistryFreshness::Fresh, "{status:?}");
    assert_eq!(source.seq, Some(1));
    assert!(
        source.expires.is_some() && source.offline.is_none(),
        "{source:?}"
    );
    let Response::AvailablePlugins { plugins, .. } = agent.available(true).await.unwrap() else {
        panic!("available")
    };
    assert!(plugins.iter().any(|p| p.id == HELLO), "{plugins:?}");

    // A tampered package (one byte changed): refused, nothing installed.
    let mut tampered = registry.release(HELLO, "1.0.0");
    let middle = tampered.len() / 2;
    tampered[middle] ^= 0x40;
    let path = Registry::release_path(HELLO, "1.0.0");
    server.route(&path, Route::Body(tampered));
    let refused = agent.install(HELLO, "1.0.0").await;
    assert_eq!(reason_of(&refused), reason::INVALID_PACKAGE, "{refused:?}");
    assert!(message_of(&refused).contains("pinned"), "{refused:?}");
    // A download cut off midway: refused, nothing installed.
    server.route(&path, Route::CutOff { sent: 4096 });
    let refused = agent.install(HELLO, "1.0.0").await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    // Redirected to another origin: not followed.
    let other = HttpsServer::start(&registry.work());
    server.route(
        &path,
        Route::Redirect {
            status: 302,
            location: other.url(&path),
        },
    );
    let refused = agent.install(HELLO, "1.0.0").await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    assert!(
        message_of(&refused).contains("another origin"),
        "{refused:?}"
    );
    assert!(other.log().is_empty(), "{:?}", other.log());
    assert_eq!(agent.installed().await, Vec::<String>::new());

    // Installed over HTTPS, enabled, run in the real host.
    server.clear_routes();
    let Response::Plugin { plugin } = agent.install(HELLO, "1.0.0").await.unwrap() else {
        panic!("installed")
    };
    assert_eq!(plugin.summary.version, "1.0.0");
    assert!(server.log().contains(&path), "{:?}", server.log());
    agent
        .request(Request::EnablePlugin {
            plugin: HELLO.into(),
            consent: vec![NETWORK.into()],
        })
        .await
        .unwrap();
    assert!(greeted(&agent.greet().await));

    // An update whose download is cut off: 1.0.0 keeps running.
    let update_path = Registry::release_path(HELLO, "1.1.0");
    server.route(&update_path, Route::CutOff { sent: 1000 });
    let update = Request::UpdatePlugin {
        plugin: HELLO.into(),
        version: Some("1.1.0".into()),
        consent: vec![],
    };
    let refused = agent.request(update.clone()).await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    assert_eq!(
        agent.state(HELLO).await,
        ("1.0.0".into(), PluginState::Enabled)
    );
    assert!(greeted(&agent.greet().await));
    // And then it goes through.
    server.clear_routes();
    let Response::PluginUpdate { outcome, .. } = agent.request(update).await.unwrap() else {
        panic!("updated")
    };
    assert!(
        matches!(&outcome, PluginUpdateOutcome::Applied { version, .. } if version == "1.1.0"),
        "{outcome:?}"
    );
    assert!(greeted(&agent.greet().await));

    // The registry revokes 1.1.0; the refresh quarantines it.
    let before_revocation = registry.snapshot("before-revocation");
    registry.run(&[
        "revoke",
        "--id",
        HELLO,
        "--versions",
        "1.1.0",
        "--action",
        "disable",
        "--reason",
        "exfiltrates data",
    ]);
    registry.publish();
    let Response::AvailablePlugins {
        registry: status, ..
    } = agent.available(true).await.unwrap()
    else {
        panic!("available")
    };
    assert_eq!(status.seq, Some(2));
    assert_eq!(
        agent.state(HELLO).await,
        ("1.1.0".into(), PluginState::Quarantined)
    );
    let refused = agent.install(OTHER, "1.0.0").await.map(|_| ());
    assert!(refused.is_ok(), "{refused:?}");

    // Rollback attack: the older, validly signed site without the
    // revocation. Refused; the stored state (seq 2, the revocation)
    // stays.
    server.set_root(&before_revocation);
    let refused = agent.available(true).await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    assert!(message_of(&refused).contains("rollback"), "{refused:?}");
    assert_eq!(agent.runtime().await.registry.unwrap().seq, Some(2));
    assert_eq!(
        agent.state(HELLO).await,
        ("1.1.0".into(), PluginState::Quarantined)
    );

    // A tampered index (on a new publish): refused.
    server.set_root(&registry.work());
    registry.run(&["build-index", "--source-commit", &"b".repeat(40)]);
    registry.run(&["sign-index", "--key", &registry.key("index.key")]);
    registry.run(&["timestamp", "--key", &registry.key("timestamp.key")]);
    let mut index = std::fs::read(registry.work().join("site/v1/index/3.json")).unwrap();
    index[20] ^= 0x01;
    server.route("/site/v1/index/3.json", Route::Body(index));
    let refused = agent.available(true).await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    assert_eq!(agent.runtime().await.registry.unwrap().seq, Some(2));

    // Registry documents redirected to another origin, or oversized:
    // refused.
    server.clear_routes();
    server.route(
        "/site/v1/timestamp.json",
        Route::Redirect {
            status: 302,
            location: other.url("site/v1/timestamp.json"),
        },
    );
    let refused = agent.available(true).await;
    assert!(
        message_of(&refused).contains("another origin"),
        "{refused:?}"
    );
    server.route(
        "/site/v1/timestamp.json",
        Route::Oversized {
            len: 1024 * 1024,
            declared: true,
        },
    );
    let refused = agent.available(true).await;
    assert!(message_of(&refused).contains("larger"), "{refused:?}");
    assert!(other.log().is_empty(), "{:?}", other.log());

    // Back to the real site: seq 3.
    server.clear_routes();
    let Response::AvailablePlugins {
        registry: status, ..
    } = agent.available(true).await.unwrap()
    else {
        panic!("available")
    };
    assert_eq!(status.seq, Some(3));
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_the_agent_works_from_its_verified_cache() {
    let registry = Registry::create();
    let server = HttpsServer::start(&registry.work());
    let env = Env::new("plugins-https-offline");
    let agent = env.started(&registry, &server).await;
    agent.install(HELLO, "1.0.0").await.unwrap();
    agent
        .request(Request::EnablePlugin {
            plugin: HELLO.into(),
            consent: vec![NETWORK.into()],
        })
        .await
        .unwrap();
    // Browsing fetched and cached the details.
    agent.available(false).await.unwrap();
    agent.stop().await;

    // Offline from the start.
    server.set_offline(true);
    let agent = env.started(&registry, &server).await;
    let status = agent.runtime().await.registry.unwrap();
    assert_eq!(status.freshness, RegistryFreshness::Fresh, "{status:?}");
    assert_eq!(status.seq, Some(1));
    assert!(status.offline.is_some(), "{status:?}");
    // Browsing from the verified cache; a refresh is no error offline.
    let Response::AvailablePlugins {
        registry: status,
        plugins,
    } = agent.available(true).await.unwrap()
    else {
        panic!("available")
    };
    assert!(status.offline.is_some(), "{status:?}");
    let ids: Vec<&str> = plugins.iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&HELLO) && ids.contains(&OTHER), "{ids:?}");
    // The installed plugin runs; nothing new can be downloaded.
    assert!(greeted(&agent.greet().await));
    let refused = agent.install(OTHER, "1.0.0").await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    agent.stop().await;

    // The index cache lost while offline: stale, installed plugins run.
    std::fs::remove_file(env.registry_state().join("registry-index.json")).unwrap();
    let agent = env.started(&registry, &server).await;
    let status = agent.runtime().await.registry.unwrap();
    assert_eq!(status.freshness, RegistryFreshness::Stale, "{status:?}");
    assert_eq!(
        status.stale_reason,
        Some(RegistryStaleReason::IndexUnavailable)
    );
    assert!(greeted(&agent.greet().await));
    // Back online: fresh again.
    server.set_offline(false);
    let Response::AvailablePlugins {
        registry: status, ..
    } = agent.available(true).await.unwrap()
    else {
        panic!("available")
    };
    assert_eq!(status.freshness, RegistryFreshness::Fresh, "{status:?}");
    assert!(status.offline.is_none());
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_and_frozen_registries_block_installs() {
    let registry = Registry::create();
    let server = HttpsServer::start(&registry.work());

    // Freeze: a timestamp that expired a day ago. A new agent trusts
    // nothing.
    registry.timestamp_expiring(-TimeDelta::days(1));
    let env = Env::new("plugins-https-frozen");
    let agent = env.started(&registry, &server).await;
    let status = agent.runtime().await;
    let registry_status = status.registry.clone().unwrap();
    assert_eq!(
        registry_status.freshness,
        RegistryFreshness::NeverVerified,
        "{status:?}"
    );
    assert!(
        status
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("expired"),
        "{status:?}"
    );
    let refused = agent.install(HELLO, "1.0.0").await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    agent.stop().await;

    // Expiry: verified while valid (it expires 40 s from now, plus the
    // 5 minute clock skew allowance, minus 5 minutes), then nobody
    // re-signs it.
    registry.timestamp_expiring(TimeDelta::seconds(40) - TimeDelta::minutes(5));
    let env = Env::new("plugins-https-expired");
    let agent = env.started(&registry, &server).await;
    let status = agent.runtime().await.registry.unwrap();
    assert_eq!(status.freshness, RegistryFreshness::Fresh, "{status:?}");
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        let status = agent.runtime().await.registry.unwrap();
        if status.freshness == RegistryFreshness::Stale {
            break status;
        }
        assert!(Instant::now() < deadline, "never stale: {status:?}");
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    assert_eq!(status.stale_reason, Some(RegistryStaleReason::Expired));
    // The registry still serves the expired timestamp: refused, still
    // stale, installs blocked.
    let refused = agent.available(true).await;
    assert!(message_of(&refused).contains("expired"), "{refused:?}");
    let refused = agent.install(HELLO, "1.0.0").await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    assert!(message_of(&refused).contains("expired"), "{refused:?}");
    // Re-signed: fresh, the install goes through.
    registry.run(&["timestamp", "--key", &registry.key("timestamp.key")]);
    agent.install(HELLO, "1.0.0").await.unwrap();
    agent.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_root_keys_nothing_is_fetched() {
    // A build with roots embedded (DECKPILOT_REGISTRY_ROOT_PUBKEYS set at
    // build time) always has root keys; this is about one without.
    if deckpilot_plugin_registry::roots::EMBEDDED.is_some() {
        return;
    }
    let registry = Registry::create();
    let server = HttpsServer::start(&registry.work());
    let env = Env::new("plugins-https-no-roots");
    let ca = env.dir.path().join("test-ca.pem");
    server.write_ca(&ca);
    let child = Command::new(env!("CARGO_BIN_EXE_deckpilot-agent"))
        .args(["--headless", "--no-usb", "--enable-plugins"])
        .arg("--plugin-registry-url")
        .arg(server.url("site/"))
        .arg("--plugin-host")
        .arg(&artifacts().host)
        .env("DECKPILOT_DATA_DIR", &env.paths.data_dir)
        .env("DECKPILOT_TEST_REGISTRY_CA", &ca)
        .env_remove("DECKPILOT_PLUGIN_REGISTRY_ROOTS")
        .env_remove("DECKPILOT_PLUGIN_REGISTRY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let agent = Agent {
        child,
        paths: env.paths.clone(),
        key: env.key.clone(),
    };
    agent
        .request(Request::Unlock {
            key: KeyMaterial::new(agent.key.to_hex()),
        })
        .await
        .unwrap();
    let status = agent.runtime().await;
    assert!(status.registry.is_none(), "{status:?}");
    assert!(
        status
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("--plugin-registry-roots"),
        "{status:?}"
    );
    let refused = agent.install(HELLO, "1.0.0").await;
    assert_eq!(reason_of(&refused), reason::REGISTRY, "{refused:?}");
    assert!(server.log().is_empty(), "{:?}", server.log());
    agent.stop().await;
}

const FIRST_PARTY: &str = "deckpilot.hello-weather";
/// A plugin the agent bundles; the registry lists it anyway.
const BUNDLED: &str = "deckpilot.github";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one flow, step by step
async fn first_party_plugins_need_the_root_authorization_and_a_verified_publisher() {
    let registry = Registry::create();
    // Key set 2 (root-signed) authorizes `deckpilot`; the publisher record
    // is verified while the versions are added, then the index key's
    // record says unverified.
    registry.authorize_first_party(true);
    registry.first_party_publisher(true);
    registry.add_first_party(FIRST_PARTY, "1.0.0");
    registry.add_first_party(FIRST_PARTY, "1.1.0");
    registry.add_unsigned_first_party(BUNDLED, "1.0.0");
    registry.first_party_publisher(false);
    registry.publish();
    let server = HttpsServer::start(&registry.work());
    let env = Env::new("plugins-https-first-party");
    let agent = env.started(&registry, &server).await;
    let Response::AvailablePlugins { plugins, .. } = agent.available(true).await.unwrap() else {
        panic!("available")
    };
    assert!(plugins.iter().any(|p| p.id == FIRST_PARTY), "{plugins:?}");

    // An unverified `deckpilot` publisher: refused before any download.
    let refused = agent.install(FIRST_PARTY, "1.0.0").await;
    assert_eq!(reason_of(&refused), reason::INVALID_PACKAGE, "{refused:?}");
    assert!(
        message_of(&refused).contains("verified publisher"),
        "{refused:?}"
    );
    let release = Registry::release_path(FIRST_PARTY, "1.0.0");
    assert!(!server.log().contains(&release), "{:?}", server.log());

    // Verified, under the authorizing key set: installed from the
    // registry with the first-party rules.
    registry.first_party_publisher(true);
    registry.publish();
    let Response::Plugin { plugin } = agent.install(FIRST_PARTY, "1.0.0").await.unwrap() else {
        panic!("installed")
    };
    assert_eq!(plugin.summary.version, "1.0.0");
    assert!(server.log().contains(&release), "{:?}", server.log());
    assert_eq!(agent.installed().await, vec![FIRST_PARTY.to_owned()]);

    // An official plugin the agent bundles, without the publisher
    // signature: refused, nothing installed (the registry's authorization
    // alone is not enough).
    let refused = agent.install(BUNDLED, "1.0.0").await;
    assert!(refused.is_err(), "{refused:?}");
    assert!(message_of(&refused).contains("minisig"), "{refused:?}");
    assert_eq!(agent.installed().await, vec![FIRST_PARTY.to_owned()]);
    // Signed by the publisher key this agent pins: installed, from the
    // registry, with the first-party rules.
    registry.sign_release(BUNDLED, "1.0.0");
    let Response::Plugin { plugin } = agent.install(BUNDLED, "1.0.0").await.unwrap() else {
        panic!("installed")
    };
    assert_eq!(plugin.summary.version, "1.0.0");
    let mut installed = agent.installed().await;
    installed.sort();
    assert_eq!(installed, vec![BUNDLED.to_owned(), FIRST_PARTY.to_owned()]);
    let audit =
        std::fs::read_to_string(env.paths.data_dir.join("plugin-official/audit.jsonl")).unwrap();
    // One verified release per install, hash-chained.
    assert_eq!(audit.matches("\"verified\"").count(), 2, "{audit}");
    assert!(audit.contains("\"prev\":\"genesis\""), "{audit}");

    // Key set 3 withdraws the authorization: the update is refused before
    // any download, and the installed version stays.
    registry.authorize_first_party(false);
    registry.publish();
    let refused = agent
        .request(Request::UpdatePlugin {
            plugin: FIRST_PARTY.into(),
            version: Some("1.1.0".into()),
            consent: vec![],
        })
        .await;
    assert_eq!(reason_of(&refused), reason::INVALID_PACKAGE, "{refused:?}");
    assert!(message_of(&refused).contains("key set"), "{refused:?}");
    let update = Registry::release_path(FIRST_PARTY, "1.1.0");
    assert!(!server.log().contains(&update), "{:?}", server.log());
    assert_eq!(agent.state(FIRST_PARTY).await.0, "1.0.0");
    assert_eq!(agent.runtime().await.registry.unwrap().seq, Some(4));
    agent.stop().await;
}
