//! Native windows: opening by identifier, sizing to content, closing.

use tauri::{AppHandle, Emitter, Runtime, State, WebviewWindow};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;
use crate::windows::{self, WindowSpec};

/// Event sent to every window after the lock state changed.
pub const SECURITY_CHANGED: &str = "deckpilot://security-changed";

impl From<windows::WindowError> for CommandError {
    fn from(error: windows::WindowError) -> Self {
        Self::Invalid(error.to_string())
    }
}

/// Opens (or focuses) the window `id` if the catalog allows it now.
pub fn open_by_id<R: Runtime>(app: &AppHandle<R>, state: &AppState, id: &str) -> CommandResult<()> {
    let spec = {
        let modules = state.modules();
        windows::resolve(id, &modules, state.is_unlocked())?
    };
    windows::open(app, &spec)?;
    Ok(())
}

#[tauri::command]
pub fn open_window<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<()> {
    open_by_id(&app, &state, &id)
}

/// What the calling window is; `None` for the main window.
#[tauri::command]
pub fn current_window<R: Runtime>(
    window: WebviewWindow<R>,
    registry: State<'_, windows::WindowRegistry>,
) -> Option<WindowSpec> {
    registry.spec(window.label())
}

/// Sizes the calling window to its content (logical pixels).
#[tauri::command]
pub fn fit_window<R: Runtime>(
    window: WebviewWindow<R>,
    width: f64,
    height: f64,
) -> CommandResult<(f64, f64)> {
    if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
        return Err(CommandError::Invalid("invalid content size".into()));
    }
    Ok(windows::fit(&window, (width, height))?)
}

#[tauri::command]
pub fn close_window<R: Runtime>(window: WebviewWindow<R>) -> CommandResult<()> {
    if window.label() == windows::MAIN {
        return Err(CommandError::Invalid(
            "the main window is closed by the user".into(),
        ));
    }
    window
        .close()
        .map_err(|e| CommandError::Invalid(e.to_string()))
}

/// Shows the main window if this process has one.
#[tauri::command]
pub fn show_main_window<R: Runtime>(app: AppHandle<R>) -> bool {
    windows::show_main(&app)
}

/// A security prompt finished: tells every window, shows the main window
/// (when there is one) and closes the prompt.
#[tauri::command]
pub fn finish_security_prompt<R: Runtime>(app: AppHandle<R>, window: WebviewWindow<R>) {
    let _ = app.emit(SECURITY_CHANGED, ());
    windows::show_main(&app);
    if window.label() != windows::MAIN {
        let _ = window.close();
    }
}
