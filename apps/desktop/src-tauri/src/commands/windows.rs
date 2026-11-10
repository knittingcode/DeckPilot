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

/// Sizes the calling window to its content. `width`/`height`: the content's
/// natural size; `viewport_*`: the webview's viewport when it was measured
/// (logical pixels).
#[tauri::command]
pub fn fit_window<R: Runtime>(
    window: WebviewWindow<R>,
    width: f64,
    height: f64,
    viewport_width: f64,
    viewport_height: f64,
) -> CommandResult<(f64, f64)> {
    let valid = |v: f64| v.is_finite() && (0.0..100_000.0).contains(&v);
    if !(valid(width) && valid(height) && valid(viewport_width) && valid(viewport_height))
        || width <= 0.0
        || height <= 0.0
    {
        return Err(CommandError::Invalid("invalid content size".into()));
    }
    Ok(windows::fit(
        &window,
        (width, height),
        (viewport_width, viewport_height),
    )?)
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

/// What double-clicking the titlebar does, from the system preference.
/// Only macOS has an overlay titlebar; elsewhere this is used by tests.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleClick {
    /// Zoom: maximize, or restore the previous size.
    Zoom,
    Minimize,
    Nothing,
}

/// Interprets macOS's "Double-click a window's title bar to" setting
/// (`AppleActionOnDoubleClick`, and the older `AppleMiniaturizeOnDoubleClick`).
/// "Fill" has no public equivalent for other apps' windows; it zooms.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn double_click_action(action: Option<&str>, legacy_minimize: Option<&str>) -> DoubleClick {
    match action.map(str::trim) {
        Some("Minimize") => DoubleClick::Minimize,
        Some("None") => DoubleClick::Nothing,
        None if legacy_minimize.map(str::trim) == Some("1") => DoubleClick::Minimize,
        // "Maximize", "Fill", or nothing set: the default is to zoom.
        _ => DoubleClick::Zoom,
    }
}

#[cfg(target_os = "macos")]
fn global_default(key: &str) -> Option<String> {
    let output = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "-g", key])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Double-click on empty titlebar space of a window whose content extends
/// into the titlebar (macOS overlay titlebar). Other platforms keep their
/// native titlebar, which handles double-clicks itself.
#[tauri::command]
pub fn titlebar_double_click<R: Runtime>(window: WebviewWindow<R>) -> CommandResult<()> {
    #[cfg(target_os = "macos")]
    let action = double_click_action(
        global_default("AppleActionOnDoubleClick").as_deref(),
        global_default("AppleMiniaturizeOnDoubleClick").as_deref(),
    );
    #[cfg(not(target_os = "macos"))]
    let action = DoubleClick::Nothing;
    let result = match action {
        DoubleClick::Zoom if window.is_resizable().unwrap_or(false) => {
            if window.is_maximized().unwrap_or(false) {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        DoubleClick::Minimize => window.minimize(),
        DoubleClick::Zoom | DoubleClick::Nothing => Ok(()),
    };
    result.map_err(|error| CommandError::Invalid(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{DoubleClick, double_click_action};

    #[test]
    fn titlebar_double_click_follows_the_system_setting() {
        assert_eq!(double_click_action(None, None), DoubleClick::Zoom);
        assert_eq!(
            double_click_action(Some("Maximize\n"), None),
            DoubleClick::Zoom
        );
        assert_eq!(double_click_action(Some("Fill"), None), DoubleClick::Zoom);
        assert_eq!(
            double_click_action(Some("Minimize\n"), None),
            DoubleClick::Minimize
        );
        assert_eq!(
            double_click_action(Some("None"), Some("1")),
            DoubleClick::Nothing
        );
        assert_eq!(
            double_click_action(None, Some("1\n")),
            DoubleClick::Minimize
        );
        assert_eq!(double_click_action(None, Some("0")), DoubleClick::Zoom);
    }
}
