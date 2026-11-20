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

/// Opens (or focuses) the window `id` if the catalog allows it now. The
/// settings (`app.settings`) are a view of the main window: that is shown
/// at `page` instead, and no window is created.
pub fn open_by_id<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    id: &str,
    page: Option<&str>,
) -> CommandResult<()> {
    if id == windows::SETTINGS {
        windows::open_settings(app, state.is_unlocked(), page)?;
        return Ok(());
    }
    let spec = {
        let modules = state.modules();
        windows::resolve(id, &modules, state.is_unlocked())?
    };
    windows::open(app, &spec)?;
    Ok(())
}

/// Async on purpose: a synchronous command runs on the main thread, and
/// creating a webview window there deadlocks on Windows (`WebView2` needs the
/// event loop the command is blocking). The window is then created but
/// never shown, which is what happened to the settings window on Windows
/// in 0.8.0 and 0.8.1.
#[tauri::command]
pub async fn open_window<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: String,
    page: Option<String>,
) -> CommandResult<()> {
    open_by_id(&app, &state, &id, page.as_deref())
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

/// The right edge, in points from the window's left edge, of the native
/// window controls that overlay the content (macOS traffic lights with the
/// `Overlay` titlebar): the toolbar keeps its content to the right of it.
/// `None` where the platform draws its own titlebar (Windows, Linux).
///
/// The buttons' size and spacing differ between macOS versions (larger from
/// macOS 26), and their position comes from `trafficLightPosition` in the
/// window's configuration. Reading the frames from `AppKit` directly would
/// need `unsafe` code, which this crate forbids.
#[tauri::command]
// Always `Some` on macOS; `None` on the other platforms.
#[cfg_attr(target_os = "macos", allow(clippy::unnecessary_wraps))]
pub fn window_controls_inset<R: Runtime>(window: WebviewWindow<R>) -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        let x = window
            .app_handle()
            .config()
            .app
            .windows
            .iter()
            .find(|w| w.label == window.label())
            .and_then(|w| w.traffic_light_position.as_ref())
            .map_or(DEFAULT_TRAFFIC_LIGHT_X, |p| p.x);
        Some(controls_right_edge(x, macos_major_version()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        None
    }
}

/// macOS places the first button 7 pt in; Tauri's default when no position
/// is configured is close to it.
const DEFAULT_TRAFFIC_LIGHT_X: f64 = 7.0;

/// Right edge of the three traffic lights whose first one starts at `x`.
/// Before macOS 26 the buttons are 12 pt wide, 20 pt apart (centre to
/// centre); from macOS 26 14 pt wide, 23 pt apart (measured on screen).
/// An unknown version gets the larger geometry: too much room is harmless,
/// too little overlaps the buttons.
fn controls_right_edge(x: f64, macos_major: Option<u32>) -> f64 {
    let (width, pitch) = match macos_major {
        Some(major) if major < 26 => (12.0, 20.0),
        _ => (14.0, 23.0),
    };
    x + 2.0 * pitch + width
}

#[cfg(target_os = "macos")]
fn macos_major_version() -> Option<u32> {
    let output = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .split('.')
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::{DoubleClick, controls_right_edge, double_click_action};

    #[test]
    fn traffic_lights_end_where_macos_draws_them() {
        // tauri.conf.json: trafficLightPosition.x = 16.
        // macOS 15 and earlier: 12 pt buttons, 20 pt apart.
        assert!((controls_right_edge(16.0, Some(15)) - 68.0).abs() < f64::EPSILON);
        // macOS 26 and later: 14 pt buttons, 23 pt apart (76 pt measured on
        // a macOS 27 Retina screen).
        assert!((controls_right_edge(16.0, Some(26)) - 76.0).abs() < f64::EPSILON);
        assert!((controls_right_edge(16.0, Some(27)) - 76.0).abs() < f64::EPSILON);
        // Unknown version: the larger geometry, never too little room.
        assert!((controls_right_edge(16.0, None) - 76.0).abs() < f64::EPSILON);
    }

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
