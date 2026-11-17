//! The marketplace scenario (`tests/support/marketplace.rs`) at the
//! command layer: the app's real commands, through Tauri's mock runtime
//! with the frontend's argument names, against a real agent process (built
//! with `plugins,test-hooks`), the real sandboxed plugin host, a real
//! registry over HTTPS and real storage. Only the webview is missing.
//!
//! This is how the scenario runs on macOS, which has no `WebDriver` for
//! `WKWebView`: the macOS GUI is **not** driven. Linux and Windows run it
//! through the UI as well (`tests/marketplace_gui.rs`).
//!
//! The mock runtime runs the app inside this test process, which cannot
//! pass an environment of its own to an agent the app would start, so the
//! test starts the agent itself (with the arguments the GUI test hands the
//! app's agent) and stops it through the app's `stop_agent`.
//!
//! Ignored by default; `docs/marketplace/e2e.md` says how to run it. The
//! live smoke tests (`live_*`) also do nothing unless
//! `DECKPILOT_LIVE_REGISTRY=1`: they need the network and an agent built
//! with the official registry's roots embedded.

use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{App, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::state::AppState;

#[path = "../tests/support/marketplace.rs"]
mod marketplace;

use marketplace::{Driver, Fixture, PASSWORD, Source, Trust, Via};

const APP_ORIGIN: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

/// One start of the app.
struct Running {
    window: WebviewWindow<MockRuntime>,
    _app: App<MockRuntime>,
}

#[derive(Default)]
struct Commands {
    app: Option<Running>,
}

impl Commands {
    fn launch(&mut self, fx: &Fixture) {
        let app = mock_builder()
            .manage(
                AppState::with_presence(
                    fx.paths.clone(),
                    std::sync::Arc::new(deckpilot_presence::Unavailable),
                )
                .expect("open state"),
            )
            .manage(crate::updates::UpdateManager::default())
            .manage(crate::commands::portability::PendingImport::default())
            .manage(crate::commands::plugins::PluginEvents::default())
            .manage(crate::windows::WindowRegistry::default())
            .invoke_handler(crate::commands::handler())
            .build(mock_context(noop_assets()))
            .expect("build app");
        let window = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
            .build()
            .expect("build window");
        self.app = Some(Running { window, _app: app });
        // The unlock window's command, with the password.
        self.ok("unlock", json!({"password": PASSWORD}));
    }
}

impl Driver for Commands {
    fn name(&self) -> &'static str {
        "commands"
    }

    fn invoke(&mut self, command: &str, args: Value) -> Result<Value, Value> {
        let window = &self.app.as_ref().expect("the app runs").window;
        let request = InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: APP_ORIGIN.parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: tauri::http::HeaderMap::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        get_ipc_response(window, request).map(|body| body.deserialize().unwrap())
    }

    fn start(&mut self, fx: &mut Fixture) -> Vec<Via> {
        fx.spawn_agent();
        self.launch(fx);
        vec![Via::Fixture, Via::AppIpc]
    }

    fn restart(&mut self, fx: &mut Fixture) -> Vec<Via> {
        self.ok("stop_agent", json!({}));
        fx.wait_for_agent(false);
        fx.stop_agent();
        self.app = None;
        fx.spawn_agent();
        self.launch(fx);
        vec![Via::AppIpc, Via::Fixture]
    }
}

#[test]
#[ignore = "needs an agent built with plugins,test-hooks, the plugin host and the sample (docs/marketplace/e2e.md)"]
fn the_marketplace_works_end_to_end_through_the_app_commands() {
    let mut fx = Fixture::new("marketplace-commands");
    let mut driver = Commands::default();
    marketplace::run(&mut driver, &mut fx);
    driver.app = None;
}

/// The live smoke tests run only with `DECKPILOT_LIVE_REGISTRY=1`: they
/// reach the official registry over the network, so `--ignored` alone (as
/// in CI) skips them.
fn live() -> bool {
    let on = std::env::var_os("DECKPILOT_LIVE_REGISTRY").is_some_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set DECKPILOT_LIVE_REGISTRY=1 to run the live registry smoke test");
    }
    on
}

/// The agent built with the official registry's roots embedded
/// (`DECKPILOT_REGISTRY_ROOT_PUBKEYS` = `keys/roots.json` of
/// knittingcode/deckpilot-plugins at build time).
const LIVE_AGENT_BIN: &str = "DECKPILOT_E2E_LIVE_AGENT_BIN";

#[test]
#[ignore = "needs the network and an agent with the production roots embedded (docs/marketplace/e2e.md)"]
fn live_registry_installs_and_runs_a_plugin_with_the_embedded_roots() {
    if !live() {
        return;
    }
    let mut fx = Fixture::with(
        "marketplace-live",
        LIVE_AGENT_BIN,
        Source::Live,
        Trust::Build,
    );
    let mut driver = Commands::default();
    marketplace::run_live(&mut driver, &mut fx);
    driver.app = None;
}

#[test]
#[ignore = "needs the network (docs/marketplace/e2e.md)"]
fn live_registry_is_refused_by_an_agent_trusting_other_roots() {
    if !live() {
        return;
    }
    // The agent without embedded roots, given throwaway ones.
    let mut fx = Fixture::with(
        "marketplace-live-other-roots",
        "DECKPILOT_E2E_AGENT_BIN",
        Source::Live,
        Trust::Throwaway,
    );
    let mut driver = Commands::default();
    marketplace::run_refused(
        &mut driver,
        &mut fx,
        marketplace::LIVE_REGISTRY,
        marketplace::MEMORY,
    );
    driver.app = None;
}

#[test]
#[ignore = "needs an agent with the production roots embedded and the sample (docs/marketplace/e2e.md)"]
fn live_production_roots_refuse_a_registry_signed_with_other_keys() {
    if !live() {
        return;
    }
    // The production-roots agent, given the local throwaway registry and
    // no roots: its embedded roots must not accept it.
    let mut fx = Fixture::with(
        "marketplace-live-throwaway-registry",
        LIVE_AGENT_BIN,
        Source::Local,
        Trust::Build,
    );
    let mut driver = Commands::default();
    marketplace::run_refused(
        &mut driver,
        &mut fx,
        "https://127.0.0.1:",
        marketplace::HELLO,
    );
    assert!(
        !fx.registry_log().is_empty(),
        "the local registry was not asked: {:?}",
        fx.registry_log()
    );
    driver.app = None;
}
