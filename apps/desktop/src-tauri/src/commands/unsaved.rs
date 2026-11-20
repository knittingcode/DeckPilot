//! Unsaved layout edits and quitting the app (⌘Q, the macOS menu's
//! "Quit DeckPilot UI"): the editor's draft lives only in the window and on
//! the device of its live preview, so quitting with unsaved edits asks
//! first. The window tells whether it has any ([`set_unsaved_edits`]); a
//! quit the user asked for is then held back and the window asks
//! ([`QUIT_REQUESTED_EVENT`]), quitting with [`quit_app`] once the edits
//! are discarded.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Runtime, State};

/// Asks the main window to confirm quitting over unsaved edits.
pub const QUIT_REQUESTED_EVENT: &str = "quit-requested";

/// Whether the main window has unsaved layout edits.
#[derive(Debug, Default)]
pub struct UnsavedEdits(AtomicBool);

impl UnsavedEdits {
    pub fn get(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The main window has (or no longer has) unsaved layout edits.
#[tauri::command]
pub fn set_unsaved_edits(edits: State<'_, UnsavedEdits>, unsaved: bool) {
    edits.0.store(unsaved, Ordering::SeqCst);
}

/// Closes the window that asked, after the user discarded unsaved edits
/// (closing it asks first while there are any).
#[tauri::command]
pub fn close_main_window<R: Runtime>(window: tauri::Window<R>, edits: State<'_, UnsavedEdits>) {
    edits.0.store(false, Ordering::SeqCst);
    let _ = window.close();
}

/// Quits after the user discarded unsaved edits.
#[tauri::command]
pub fn quit_app<R: Runtime>(app: AppHandle<R>, edits: State<'_, UnsavedEdits>) {
    edits.0.store(false, Ordering::SeqCst);
    app.exit(0);
}
