//! DeckPilot configuration application.
//!
//! This process edits configuration and shows the agent's state. Device
//! handling and action execution belong to the background agent
//! (`deckpilot-agent`), which keeps running when this window is closed.

// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod commands;
mod dev_server;
mod error;
mod modules;
mod state;
#[cfg(test)]
mod tests;
mod updates;
mod windows;

use std::time::Duration;

use tauri::{Manager, WebviewWindowBuilder};

/// `--window <id>`: show only that window (e.g. the unlock prompt requested
/// by the agent while no app is open) and exit when it closes.
fn requested_window() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--window" {
            return args.next().filter(|id| deckpilot_ipc::is_window_id(id));
        }
    }
    None
}

/// The main window is created hidden; it is shown once the configuration is
/// unlocked. If the frontend never asks (it failed to load), it is shown
/// anyway so the user is not left without a window.
fn create_main_window(app: &tauri::App) -> tauri::Result<()> {
    let mut config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == windows::MAIN)
        .cloned()
        .unwrap_or_default();
    // Development builds without their Vite server: a diagnostic page
    // instead of a blank window.
    if let Some(url) = dev_server::fallback_url(app.config(), "") {
        config.url = url;
        config.visible = true;
    }
    WebviewWindowBuilder::from_config(app.handle(), &config)?.build()?;
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        let prompts = handle.state::<windows::WindowRegistry>().open_labels();
        if prompts.is_empty()
            && let Some(main) = handle.get_webview_window(windows::MAIN)
            && !main.is_visible().unwrap_or(true)
        {
            let _ = main.show();
        }
    });
    Ok(())
}

fn main() {
    // `--agent`: this executable stands in for the agent (AppImages).
    agent::run_agent_if_requested();
    let window_only = requested_window();
    let mut builder = tauri::Builder::default();
    if tauri::is_dev() {
        builder = builder.register_uri_scheme_protocol(dev_server::SCHEME, |ctx, request| {
            let app = ctx.app_handle();
            let config = app.config();
            let session = dev_server::Session {
                product: config.product_name.clone().unwrap_or_default(),
                identifier: config.identifier.clone(),
                data_dir: deckpilot_ipc::AgentPaths::resolve()
                    .map(|paths| paths.data_dir)
                    .unwrap_or_default(),
            };
            match dev_server::dev_url(config) {
                Some(dev) => dev_server::respond(dev, &session, &request),
                None => tauri::http::Response::new(std::borrow::Cow::Borrowed(&b""[..])),
            }
        });
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::UpdateManager::default())
        .manage(windows::WindowRegistry::default())
        .manage(commands::portability::PendingImport::default())
        .setup(move |app| {
            // Same location as the agent; `DECKPILOT_DATA_DIR` overrides it.
            let paths = deckpilot_ipc::AgentPaths::resolve()?;
            let state = state::AppState::open(paths.clone())?;
            app.manage(state);
            match window_only.clone() {
                None => create_main_window(app)?,
                Some(id) => {
                    // A prompt or module window on its own. Module windows
                    // need the configuration: use the key of an unlocked
                    // agent if there is one.
                    let handle = app.handle().clone();
                    let paths = paths.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = handle.state::<state::AppState>();
                        if let Some(key) = agent::fetch_key(&paths).await {
                            let _ = state.attach(key, false);
                        }
                        if let Err(error) = commands::windows::open_by_id(&handle, &state, &id) {
                            eprintln!("cannot open window `{id}`: {error}");
                            handle.exit(1);
                        }
                    });
                }
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Start the agent on first launch if it is not running yet,
                // unless a development session manages it.
                let managed = std::env::var_os("DECKPILOT_NO_AGENT_AUTOSTART").is_some();
                if !managed
                    && let Ok(connection) = agent::connection(&paths).await
                    && !connection.running
                    && connection.can_start
                    && let Err(error) = agent::start_agent()
                {
                    eprintln!("{error}");
                }
                agent::spawn_bridge(handle, paths);
            });
            updates::spawn_background(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let app = window.app_handle();
                let unlocked = app.state::<state::AppState>().is_unlocked();
                windows::closed(app, window.label(), unlocked);
            }
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("failed to run DeckPilot");
}
