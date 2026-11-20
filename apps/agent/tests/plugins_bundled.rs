//! Bundled first-party plugins through the real agent and the real
//! sandboxed host (feature `plugins`; `docs/marketplace/module-migration.md`
//! §4):
//!
//! * the test fixture `plugins/first-party/test-fixtures/example-default`
//!   is built to wasm with the `guest` profile and staged as an installed
//!   application would ship it (`bundled/<id>/…`), with no registry and no
//!   network: the agent seeds it when the store opens, grants its one
//!   low-tier permission, enables it, and runs its action in the host;
//! * the pins come from `DECKPILOT_TEST_BUNDLED_PINS` (feature `test-hooks`
//!   only), because the agent was compiled before the test built the
//!   plugin; a release agent only trusts pins compiled into it;
//! * a bundled plugin with translations (`locales/<tag>.json`, plugin API
//!   1.6) and visual assets (PNG and JPEG under `assets/`, plugin API 1.8)
//!   is seeded and loads like any other;
//! * tampered files are refused at seeding and at load, the user's disable
//!   and uninstall survive restarts, and a `deckpilot.*` package from a
//!   file is refused.
#![cfg(feature = "plugins")]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use deckpilot_ipc::plugins::{PluginActionOutcome, PluginInstallSource, PluginState};
use deckpilot_ipc::{
    AgentClient, AgentPaths, ClientError, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_plugin_bundled::{BundleClass, Pins};
use deckpilot_plugin_manifest::PluginId;
use deckpilot_storage::DatabaseKey;
use serde_json::{Value, json};
use support::TempDir;

const ID: &str = "deckpilot.example-default";
const STARTUP: Duration = Duration::from_secs(30);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime() -> PathBuf {
    repo().join("plugins/runtime")
}

fn fixture() -> PathBuf {
    repo().join("plugins/first-party/test-fixtures/example-default")
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
            .filter(|dir| dir.join("deckpilot_example_default.wasm").is_file())
            .unwrap_or_else(|| {
                cargo_in_runtime(&[
                    "build",
                    "--locked",
                    "-p",
                    "deckpilot-example-default",
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
            module: wasm_dir.join("deckpilot_example_default.wasm"),
        }
    })
}

/// A data directory, application resources with the fixture bundled, and
/// the pins of exactly those files.
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
        let env = Self { dir, paths, key };
        env.stage();
        env
    }

    /// As [`Env::new`], with the fixture at plugin API 1.8 carrying
    /// translations and visual assets.
    fn localized(name: &str) -> Self {
        let env = Self::new(name);
        env.add_translations_and_visual_assets();
        env
    }

    fn bundled(&self) -> PathBuf {
        self.dir.path().join("resources/bundled")
    }

    fn plugin_dir(&self) -> PathBuf {
        self.bundled().join(ID)
    }

    fn pins_file(&self) -> PathBuf {
        self.dir.path().join("compiled-pins.json")
    }

    /// `bundled/deckpilot.example-default/` as `plugins/first-party/build.sh`
    /// stages it, and the pins the agent "was compiled with".
    fn stage(&self) {
        let dir = self.plugin_dir();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        for file in ["manifest.json", "LICENSE", "assets/icon-128.png"] {
            std::fs::copy(fixture().join(file), dir.join(file)).unwrap();
        }
        std::fs::copy(&artifacts().module, dir.join("plugin.wasm")).unwrap();
        self.pin();
    }

    /// The pins of the staged files, next to them and as the agent "was
    /// compiled with".
    fn pin(&self) {
        let pins = Pins::compute(
            &self.bundled(),
            &[(
                PluginId::parse(ID).unwrap(),
                semver::Version::new(0, 1, 0),
                BundleClass::Default,
            )],
        )
        .unwrap();
        std::fs::write(self.bundled().join("pins.json"), pins.to_json_pretty()).unwrap();
        std::fs::write(self.pins_file(), pins.to_json_pretty()).unwrap();
    }

    /// The staged fixture with `locales/en.json`, `locales/pl.json`, a PNG
    /// icon and a JPEG background under `assets/visual/`, as
    /// `plugins/first-party/build.sh` stages a plugin whose manifest names
    /// them; pinned again.
    fn add_translations_and_visual_assets(&self) {
        let dir = self.plugin_dir();
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(fixture().join("manifest.json")).unwrap())
                .unwrap();
        manifest["engines"]["pluginApi"] = json!("^1.8");
        manifest["name"] = json!("t:plugin.name");
        manifest["locales"] = json!(["en", "pl"]);
        manifest["contributes"]["visualAssets"] = json!({
            "icons": [{"id": "wave", "name": "Wave", "asset": "assets/visual/wave.png"}],
            "backgrounds": [
                {"id": "sky", "name": "Sky", "type": "image", "asset": "assets/visual/sky.jpg"}
            ]
        });
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("locales")).unwrap();
        std::fs::create_dir_all(dir.join("assets/visual")).unwrap();
        std::fs::write(
            dir.join("locales/en.json"),
            r#"{"plugin": {"name": "Bundled Example"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("locales/pl.json"),
            r#"{"plugin": {"name": "Przykład wbudowany"}}"#,
        )
        .unwrap();
        std::fs::copy(
            fixture().join("assets/icon-128.png"),
            dir.join("assets/visual/wave.png"),
        )
        .unwrap();
        // A JPEG of 400 x 200: an APP0 segment, then the frame header.
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend([0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0xc8, 0x01, 0x90]);
        jpeg.extend([0x01, 0x01, 0x11, 0x00, 0xff, 0xd9]);
        std::fs::write(dir.join("assets/visual/sky.jpg"), jpeg).unwrap();
        self.pin();
    }

    /// The installed copy in the plugin store.
    fn installed(&self, file: &str) -> PathBuf {
        self.paths
            .data_dir
            .join("plugins")
            .join(ID)
            .join("0.1.0")
            .join(file)
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

    /// The plugin's state, or `None` when it is not installed.
    async fn state(&self) -> Option<PluginState> {
        match self.request(Request::ListPlugins).await.unwrap() {
            Response::Plugins { plugins } => {
                plugins.into_iter().find(|p| p.id == ID).map(|p| p.state)
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn invoke(&self, params: Value) -> Result<PluginActionOutcome, ClientError> {
        match self
            .request(Request::InvokePluginAction {
                plugin: ID.into(),
                action: "greet".into(),
                params: serde_json::from_value(params).unwrap(),
            })
            .await?
        {
            Response::PluginActionResult { outcome } => Ok(outcome),
            other => panic!("unexpected {other:?}"),
        }
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

fn make_writable(path: &Path) {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).unwrap();
}

fn flip_last_byte(path: &Path) {
    make_writable(path);
    let mut bytes = std::fs::read(path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(path, bytes).unwrap();
}

// ---------------------------------------------------------------- tests

/// A fresh data directory: seeded offline at unlock, enabled with its one
/// low-tier permission, its action run in the sandboxed host.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bundled_plugin_is_seeded_and_runs_in_the_host() {
    let env = Env::new("plugins-bundled-seed");
    let agent = env.start().await;
    assert_eq!(
        agent.state().await,
        Some(PluginState::Enabled),
        "{}",
        agent.log()
    );
    match agent
        .request(Request::GetPlugin { plugin: ID.into() })
        .await
        .unwrap()
    {
        Response::Plugin { plugin } => {
            let granted: Vec<_> = plugin
                .permissions
                .iter()
                .map(|p| (p.key.as_str(), p.status))
                .collect();
            assert_eq!(
                granted,
                [(
                    "notifications",
                    deckpilot_ipc::plugins::PermissionStatus::Granted
                )],
                "{:?}",
                plugin.permissions
            );
        }
        other => panic!("unexpected {other:?}"),
    }
    match agent.invoke(json!({"name": "Ada"})).await.unwrap() {
        PluginActionOutcome::Ok { message, data } => {
            assert_eq!(message.as_deref(), Some("Hello from the bundle, Ada!"));
            assert_eq!(data["greeting"], "Hello from the bundle, Ada!");
        }
        other => panic!("{other:?}\n{}", agent.log()),
    }
    // A restart finds the pinned copy and changes nothing.
    agent.stop().await;
    let agent = env.start().await;
    assert_eq!(agent.state().await, Some(PluginState::Enabled));
    assert!(matches!(
        agent.invoke(json!({"name": "Ada"})).await.unwrap(),
        PluginActionOutcome::Ok { .. }
    ));
}

/// Translations and visual assets are bundled files like the others:
/// pinned, seeded into the plugin store, verified at every load.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bundled_plugin_with_translations_and_visual_assets_is_seeded() {
    let env = Env::localized("plugins-bundled-localized");
    let agent = env.start().await;
    assert_eq!(
        agent.state().await,
        Some(PluginState::Enabled),
        "{}",
        agent.log()
    );
    for file in [
        "locales/en.json",
        "locales/pl.json",
        "assets/visual/wave.png",
        "assets/visual/sky.jpg",
    ] {
        assert_eq!(
            std::fs::read(env.installed(file)).unwrap(),
            std::fs::read(env.plugin_dir().join(file)).unwrap(),
            "{file}"
        );
    }
    assert!(matches!(
        agent.invoke(json!({"name": "Ada"})).await.unwrap(),
        PluginActionOutcome::Ok { .. }
    ));
    // The next start verifies the installed copy, nested files included.
    agent.stop().await;
    let agent = env.start().await;
    assert_eq!(
        agent.state().await,
        Some(PluginState::Enabled),
        "{}",
        agent.log()
    );
    agent.stop().await;

    // A changed translation in the bundle is refused at seeding.
    let env = Env::localized("plugins-bundled-localized-tampered");
    flip_last_byte(&env.plugin_dir().join("locales/pl.json"));
    let agent = env.start().await;
    assert_eq!(agent.state().await, None);
    assert!(agent.log().contains("locales/pl.json"), "{}", agent.log());
}

/// A plugin file in the application's resources that differs from the
/// pins is never installed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tampered_bundle_is_refused_at_seeding() {
    for file in ["plugin.wasm", "manifest.json"] {
        let env = Env::new("plugins-bundled-tampered");
        flip_last_byte(&env.plugin_dir().join(file));
        let agent = env.start().await;
        assert_eq!(agent.state().await, None, "{file}");
        let log = agent.log();
        assert!(log.contains(file), "{file}: {log}");
    }
}

/// The installed copy changed while the agent was stopped: it is marked
/// broken at the next start and never loaded into the host. A copy
/// changed while the agent runs is refused when it is loaded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tampered_installed_copy_is_refused_at_load() {
    let env = Env::new("plugins-bundled-installed");
    let agent = env.start().await;
    assert_eq!(agent.state().await, Some(PluginState::Enabled));
    agent.stop().await;
    flip_last_byte(&env.installed("plugin.wasm"));
    let agent = env.start().await;
    assert_eq!(
        agent.state().await,
        Some(PluginState::Broken),
        "{}",
        agent.log()
    );
    let refused = agent.invoke(json!({"name": "Ada"})).await;
    assert!(
        !matches!(refused, Ok(PluginActionOutcome::Ok { .. })),
        "{refused:?}"
    );
    agent.stop().await;

    // While running: the manifest changes after start-up, before the
    // first invocation loads the plugin.
    let env = Env::new("plugins-bundled-load");
    let agent = env.start().await;
    assert_eq!(agent.state().await, Some(PluginState::Enabled));
    flip_last_byte(&env.installed("manifest.json"));
    let refused = agent.invoke(json!({"name": "Ada"})).await;
    assert!(
        !matches!(refused, Ok(PluginActionOutcome::Ok { .. })),
        "{refused:?}"
    );
    assert_eq!(agent.state().await, Some(PluginState::Broken));
}

/// The user's disable and uninstall stick across restarts; enabling again
/// is the user's own undo.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_users_choices_survive_restarts() {
    let env = Env::new("plugins-bundled-tombstones");
    let agent = env.start().await;
    agent
        .request(Request::DisablePlugin { plugin: ID.into() })
        .await
        .unwrap();
    agent.stop().await;
    for _ in 0..2 {
        let agent = env.start().await;
        assert_eq!(agent.state().await, Some(PluginState::Disabled));
        agent.stop().await;
    }
    let agent = env.start().await;
    agent
        .request(Request::EnablePlugin {
            plugin: ID.into(),
            consent: vec![],
        })
        .await
        .unwrap();
    assert_eq!(agent.state().await, Some(PluginState::Enabled));
    agent
        .request(Request::UninstallPlugin {
            plugin: ID.into(),
            keep: deckpilot_ipc::plugins::PluginRetention::default(),
        })
        .await
        .unwrap();
    agent.stop().await;
    for _ in 0..2 {
        let agent = env.start().await;
        assert_eq!(agent.state().await, None, "{}", agent.log());
        agent.stop().await;
    }
}

/// `deckpilot.*` is DeckPilot's own: a package from a file never installs
/// one, whatever it contains.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_party_package_from_a_file_is_refused() {
    let env = Env::new("plugins-bundled-file");
    let agent = env.start().await;
    agent
        .request(Request::UninstallPlugin {
            plugin: ID.into(),
            keep: deckpilot_ipc::plugins::PluginRetention::default(),
        })
        .await
        .unwrap();
    // A well-formed package of the fixture's files.
    let mut tar = tar::Builder::new(Vec::new());
    for file in [
        "LICENSE",
        "assets/icon-128.png",
        "manifest.json",
        "plugin.wasm",
    ] {
        let bytes = std::fs::read(env.plugin_dir().join(file)).unwrap();
        let mut header = tar::Header::new_ustar();
        header.set_path(file).unwrap();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        tar.append(&header, bytes.as_slice()).unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gzip, &tar.into_inner().unwrap()).unwrap();
    let package = env.dir.path().join(format!("{ID}-0.1.0.dpplugin"));
    std::fs::write(&package, gzip.finish().unwrap()).unwrap();

    let result = agent
        .request(Request::InstallPlugin {
            source: PluginInstallSource::File {
                path: package.display().to_string(),
            },
            consent: vec![],
        })
        .await;
    match result {
        Err(ClientError::Agent(error)) => {
            assert!(
                error
                    .message
                    .contains("cannot be installed from source sideload"),
                "{error:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(agent.state().await, None);
}
