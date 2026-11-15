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
//! Ignored by default; `docs/marketplace/e2e.md` says how to run it.

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

use marketplace::{Driver, Fixture, PASSWORD, Via};

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
