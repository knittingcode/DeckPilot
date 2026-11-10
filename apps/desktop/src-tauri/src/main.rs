//! DeckPilot configuration application.
//!
//! This process edits configuration and shows the agent's state. Device
//! handling and action execution belong to the background agent
//! (`deckpilot-agent`), which keeps running when this window is closed.

// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod commands;
mod error;
mod state;
#[cfg(test)]
mod tests;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // Same location as the agent; `DECKPILOT_DATA_DIR` overrides it.
            let paths = deckpilot_ipc::AgentPaths::resolve()?;
            let state = state::AppState::open(paths.clone())?;
            app.manage(state);
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Start the agent on first launch if it is not running yet.
                if let Ok(connection) = agent::connection(&paths).await
                    && !connection.running
                    && connection.can_start
                    && let Err(error) = agent::start_agent()
                {
                    eprintln!("{error}");
                }
                agent::spawn_bridge(handle, paths);
            });
            Ok(())
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("failed to run DeckPilot");
}
