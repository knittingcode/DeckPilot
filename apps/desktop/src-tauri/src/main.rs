//! DeckPilot configuration application.
//!
//! This process only edits configuration. Device handling belongs to the
//! background agent, which must keep running when this window is closed.

// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod error;
mod state;
#[cfg(test)]
mod tests;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // `DECKPILOT_DATA_DIR` allows isolated data directories during development.
            let data_dir = match std::env::var_os("DECKPILOT_DATA_DIR") {
                Some(dir) => dir.into(),
                None => app.path().app_data_dir()?,
            };
            let state = state::AppState::open(&data_dir)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("failed to run DeckPilot");
}
