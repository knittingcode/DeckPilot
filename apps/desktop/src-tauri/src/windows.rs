//! Native windows besides the main configuration window.
//!
//! Every window has a stable identifier such as `security.unlock`. The
//! [catalog](resolve) decides whether a window exists and may open now: the
//! application's own windows, and windows declared by feature modules (only
//! while the module is enabled and the configuration is unlocked). The view
//! is part of the application bundle and looked up by the same identifier in
//! the frontend; nothing but the identifier is ever passed in, so no markup
//! or code can be injected through a window request.
//!
//! Windows are singletons: requesting an open window focuses it. They start
//! hidden, are sized to their content (reported by the frontend), clamped to
//! the work area of their monitor and shown once sized, so they do not jump.
//! Sizes are logical (device-independent) pixels, so display scaling is
//! handled by the window system.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, Runtime, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use deckpilot_core::{ModuleRegistry, WindowSize};

/// Label of the main configuration window.
pub const MAIN: &str = "main";
const LABEL_PREFIX: &str = "w-";
/// Space kept free between a window and the edge of the work area.
const MARGIN: f64 = 24.0;

/// A window that may open, as the frontend needs to know it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSpec {
    pub id: String,
    pub title: String,
    pub size: WindowSize,
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowError {
    #[error("there is no window `{0}`")]
    Unknown(String),
    #[error("the {0} module is disabled")]
    ModuleDisabled(String),
    #[error("the configuration is locked")]
    Locked,
    #[error("the window could not be created: {0}")]
    Platform(String),
}

/// Windows of the application itself (not of a feature module).
fn core_window(id: &str) -> Option<WindowSpec> {
    let spec = |title: &str, size, blocking| WindowSpec {
        id: id.to_owned(),
        title: title.to_owned(),
        size,
        blocking,
    };
    Some(match id {
        "security.setup" => spec("Set up DeckPilot", WindowSize::Compact, true),
        "security.unlock" => spec("Unlock DeckPilot", WindowSize::Compact, true),
        "security.migrate" => spec("Encrypt your configuration", WindowSize::Compact, true),
        "security.confirm" => spec("Confirm your password", WindowSize::Compact, true),
        "workflows.execution-summary" => spec("Workflow summary", WindowSize::Medium, false),
        _ => return None,
    })
}

/// Whether a window may open while the configuration is locked: only the
/// windows that unlock it.
fn allowed_while_locked(id: &str) -> bool {
    id.starts_with("security.")
}

/// Looks up `id` in the catalog and checks that it may open now.
pub fn resolve(
    id: &str,
    modules: &ModuleRegistry,
    unlocked: bool,
) -> Result<WindowSpec, WindowError> {
    if !deckpilot_ipc::is_window_id(id) {
        return Err(WindowError::Unknown(id.to_owned()));
    }
    let spec = match core_window(id) {
        Some(spec) => spec,
        None => match modules.window(id) {
            Some(Ok(window)) => WindowSpec {
                id: window.id.clone(),
                title: window.title.clone(),
                size: window.size,
                blocking: window.blocking,
            },
            Some(Err(module)) => return Err(WindowError::ModuleDisabled(module)),
            None => return Err(WindowError::Unknown(id.to_owned())),
        },
    };
    if !unlocked && !allowed_while_locked(id) {
        return Err(WindowError::Locked);
    }
    Ok(spec)
}

/// Size limits of a mode, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub min: (f64, f64),
    pub max: (f64, f64),
    /// The user may resize; content changes then no longer resize it.
    pub resizable: bool,
}

pub fn limits(size: WindowSize) -> Limits {
    let limits = |min, max, resizable| Limits {
        min,
        max,
        resizable,
    };
    match size {
        WindowSize::Fit => limits((320.0, 120.0), (960.0, 900.0), false),
        WindowSize::Compact => limits((360.0, 120.0), (440.0, 760.0), false),
        WindowSize::Medium => limits((480.0, 160.0), (680.0, 860.0), false),
        WindowSize::Large => limits((720.0, 420.0), (1280.0, 1000.0), true),
        WindowSize::Fullscreen => limits((480.0, 320.0), (f64::MAX, f64::MAX), true),
    }
}

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64, width: f64, height: f64) -> bool {
        x >= self.x
            && y >= self.y
            && x + width <= self.x + self.width
            && y + height <= self.y + self.height
    }
}

/// The work area (screen without menu bar, dock or taskbar) of a monitor in
/// logical pixels.
pub fn logical_work_area(monitor: &Monitor) -> Rect {
    let area = monitor.work_area();
    to_logical(
        (area.position.x, area.position.y),
        (area.size.width, area.size.height),
        monitor.scale_factor(),
    )
}

/// Converts a physical rectangle of a display with `scale` to logical pixels.
pub fn to_logical(position: (i32, i32), size: (u32, u32), scale: f64) -> Rect {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    Rect {
        x: f64::from(position.0) / scale,
        y: f64::from(position.1) / scale,
        width: f64::from(size.0) / scale,
        height: f64::from(size.1) / scale,
    }
}

/// The window size for `content`, within the mode's limits and the work
/// area. Content larger than that scrolls inside the window.
pub fn fit_size(content: (f64, f64), limits: Limits, work: Rect) -> (f64, f64) {
    let clamp = |value: f64, min: f64, max: f64, available: f64| {
        let max = max.min(available - 2.0 * MARGIN).max(min.min(available));
        value.ceil().clamp(min.min(max), max)
    };
    (
        clamp(content.0, limits.min.0, limits.max.0, work.width),
        clamp(content.1, limits.min.1, limits.max.1, work.height),
    )
}

/// What the frontend measured and what the window currently is, in logical
/// pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    /// Natural size of the content (the window shell's border box).
    pub content: (f64, f64),
    /// The webview's viewport when the content was measured.
    pub viewport: (f64, f64),
    /// The window size the platform reports (what `set_size` sets).
    pub window: (f64, f64),
    /// Outer frame minus window size: decorations outside of it.
    pub frame: (f64, f64),
}

/// Largest plausible decoration inside the window size (a title bar).
const MAX_INNER_DECORATION: f64 = 120.0;

/// The window size that shows `content` completely.
///
/// Platforms differ in what the window size includes. On macOS the title
/// bar can be part of it, so the webview's viewport is smaller than the
/// window; that difference is measured, not assumed, and added. Decorations
/// outside the window size (frames on Windows and Linux) reduce the space
/// available on the screen instead.
pub fn window_size(measurement: Measurement, limits: Limits, work: Rect) -> (f64, f64) {
    let inside = |window: f64, viewport: f64| {
        if viewport > 0.0 {
            (window - viewport).clamp(0.0, MAX_INNER_DECORATION)
        } else {
            0.0
        }
    };
    let decoration = (
        inside(measurement.window.0, measurement.viewport.0),
        inside(measurement.window.1, measurement.viewport.1),
    );
    let available = Rect {
        width: work.width - measurement.frame.0.max(0.0),
        height: work.height - measurement.frame.1.max(0.0),
        ..work
    };
    fit_size(
        (
            measurement.content.0 + decoration.0,
            measurement.content.1 + decoration.1,
        ),
        limits,
        available,
    )
}

/// Where a window of `size` goes: where it is, if it still fits the work
/// area there (no jumps while content changes), otherwise centred.
pub fn place(size: (f64, f64), work: Rect, current: Option<(f64, f64)>) -> (f64, f64) {
    if let Some((x, y)) = current
        && work.contains(x, y, size.0, size.1)
    {
        return (x, y);
    }
    let x = work.x + ((work.width - size.0) / 2.0).max(0.0);
    let y = work.y + ((work.height - size.1) / 3.0).max(0.0);
    (x.round(), y.round())
}

pub fn label_for(id: &str) -> String {
    format!("{LABEL_PREFIX}{}", id.replace('.', "_"))
}

/// Open windows and whether they were sized yet.
#[derive(Debug, Default)]
pub struct WindowRegistry(Mutex<HashMap<String, (WindowSpec, bool)>>);

impl WindowRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (WindowSpec, bool)>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn spec(&self, label: &str) -> Option<WindowSpec> {
        self.lock().get(label).map(|(spec, _)| spec.clone())
    }

    pub fn remove(&self, label: &str) -> Option<WindowSpec> {
        self.lock().remove(label).map(|(spec, _)| spec)
    }

    pub fn open_labels(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }
}

/// The monitor under the mouse, else the main window's, else the primary.
/// (Tauri's mock runtime used by unit tests cannot list monitors; there the
/// fallback work area is used. Placement itself is tested separately.)
#[cfg(test)]
fn target_monitor<R: Runtime>(_app: &AppHandle<R>) -> Option<Monitor> {
    None
}

/// The monitor under the mouse, else the main window's, else the primary.
#[cfg(not(test))]
fn target_monitor<R: Runtime>(app: &AppHandle<R>) -> Option<Monitor> {
    app.cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| {
            app.get_webview_window(MAIN)
                .and_then(|main| main.current_monitor().ok().flatten())
        })
        .or_else(|| app.primary_monitor().ok().flatten())
}

/// A work area to use when the platform reports no monitor (tests).
const FALLBACK_WORK_AREA: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 1280.0,
    height: 800.0,
};

/// Opens the window `spec`, or focuses it if it is open already.
pub fn open<R: Runtime>(app: &AppHandle<R>, spec: &WindowSpec) -> Result<(), WindowError> {
    let label = label_for(&spec.id);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let limits = limits(spec.size);
    let work = target_monitor(app).map_or(FALLBACK_WORK_AREA, |m| logical_work_area(&m));
    let size = fit_size(limits.min, limits, work);
    let (x, y) = place(size, work, None);
    let url = WebviewUrl::App(format!("index.html?window={}", spec.id).into());
    let mut builder = WebviewWindowBuilder::new(app, &label, url)
        .title(&spec.title)
        .inner_size(size.0, size.1)
        .position(x, y)
        .min_inner_size(limits.min.0, limits.min.1)
        .resizable(limits.resizable)
        .maximizable(limits.resizable)
        .minimizable(!spec.blocking)
        // Shown once the content has been measured (see `fit`).
        .visible(false)
        .focused(true);
    if spec.size == WindowSize::Fullscreen {
        builder = builder.fullscreen(true);
    }
    app.state::<WindowRegistry>()
        .lock()
        .insert(label.clone(), (spec.clone(), false));
    if let Err(error) = builder.build() {
        app.state::<WindowRegistry>().remove(&label);
        return Err(WindowError::Platform(error.to_string()));
    }
    // If the content never reports its size (a script error, a platform
    // that does not render hidden windows), show the window anyway so the
    // user is never left with an invisible prompt.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SHOW_UNSIZED_AFTER).await;
        let not_sized = handle
            .state::<WindowRegistry>()
            .lock()
            .get(&label)
            .is_some_and(|(_, fitted)| !fitted);
        if not_sized && let Some(window) = handle.get_webview_window(&label) {
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
    Ok(())
}

/// How long a new window may stay hidden waiting for its content size.
const SHOW_UNSIZED_AFTER: std::time::Duration = std::time::Duration::from_millis(1500);

/// Resizes `window` to show its content completely (`content` and
/// `viewport` in logical pixels, measured by the frontend) and shows it the
/// first time, centred. Later changes keep the window where it is unless it
/// would leave the screen. Resizable windows are only sized once, so manual
/// resizing is kept.
pub fn fit<R: Runtime>(
    window: &WebviewWindow<R>,
    content: (f64, f64),
    viewport: (f64, f64),
) -> Result<(f64, f64), WindowError> {
    let app = window.app_handle();
    let label = window.label().to_owned();
    let registry = app.state::<WindowRegistry>();
    let (spec, fitted) = registry
        .lock()
        .get(&label)
        .cloned()
        .ok_or_else(|| WindowError::Unknown(label.clone()))?;
    let limits = limits(spec.size);
    let work = window
        .current_monitor()
        .ok()
        .flatten()
        .map_or(FALLBACK_WORK_AREA, |m| logical_work_area(&m));
    let scale = window.scale_factor().unwrap_or(1.0);
    let logical = |w: u32, h: u32| (f64::from(w) / scale, f64::from(h) / scale);
    let current = window
        .inner_size()
        .map(|s| logical(s.width, s.height))
        .unwrap_or_default();
    let outer = window
        .outer_size()
        .map_or(current, |s| logical(s.width, s.height));
    let target = if fitted && limits.resizable {
        current
    } else {
        window_size(
            Measurement {
                content,
                viewport,
                window: current,
                frame: (outer.0 - current.0, outer.1 - current.1),
            },
            limits,
            work,
        )
    };
    // Changes below a pixel would only cause resize feedback loops.
    let changed = (target.0 - current.0).abs() >= 1.0 || (target.1 - current.1).abs() >= 1.0;
    if changed {
        window
            .set_size(LogicalSize::new(target.0, target.1))
            .map_err(|e| WindowError::Platform(e.to_string()))?;
    }
    if changed || !fitted {
        let position = window
            .outer_position()
            .ok()
            .map(|p| (f64::from(p.x) / scale, f64::from(p.y) / scale));
        // Centred once the first real size is known, then left in place.
        let (x, y) = place(target, work, if fitted { position } else { None });
        if position != Some((x, y)) {
            let _ = window.set_position(LogicalPosition::new(x, y));
        }
    }
    if !fitted {
        if let Some(entry) = registry.lock().get_mut(&label) {
            entry.1 = true;
        }
        let _ = window.show();
        let _ = window.set_focus();
    }
    Ok(target)
}

/// Shows the main window, if this process has one. Returns whether it did.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(main) = app.get_webview_window(MAIN) else {
        return false;
    };
    let _ = main.unminimize();
    let _ = main.show();
    let _ = main.set_focus();
    true
}

/// Called when a window is gone. Returns focus to the main window, and
/// quits when a blocking prompt was dismissed while nothing else can be
/// used (the main window is hidden because the configuration is locked).
pub fn closed<R: Runtime>(app: &AppHandle<R>, label: &str, unlocked: bool) {
    let Some(spec) = app.state::<WindowRegistry>().remove(label) else {
        return;
    };
    let main = app.get_webview_window(MAIN);
    let main_visible = main
        .as_ref()
        .is_some_and(|main| main.is_visible().unwrap_or(false));
    if spec.blocking && !unlocked && !main_visible && main.is_some() {
        app.exit(0);
        return;
    }
    if main_visible && let Some(main) = main {
        let _ = main.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_core::{
        ActionHandler, ActionParameters, FeatureModule, ModuleId, ModuleManifest, ModuleWindow, Os,
    };

    struct Notes(ModuleManifest);

    impl FeatureModule for Notes {
        fn manifest(&self) -> &ModuleManifest {
            &self.0
        }
        fn action_handlers(&self, _: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
            Vec::new()
        }
    }

    fn registry() -> ModuleRegistry {
        let mut registry = deckpilot_modules::builtin(Os::Linux);
        registry
            .register(Box::new(Notes(ModuleManifest {
                id: ModuleId::new("notes").unwrap(),
                version: "1.0.0".into(),
                name: "Notes".into(),
                description: String::new(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: Vec::new(),
                config: Vec::new(),
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: deckpilot_core::Portability::default(),
                migrations: Vec::new(),
                windows: vec![ModuleWindow {
                    id: "notes.viewer".into(),
                    title: "Notes".into(),
                    size: WindowSize::Large,
                    blocking: false,
                }],
            })))
            .unwrap();
        registry
    }

    #[test]
    fn the_catalog_checks_existence_modules_and_lock_state() {
        let mut modules = registry();
        let unlock = resolve("security.unlock", &modules, false).unwrap();
        assert_eq!((unlock.size, unlock.blocking), (WindowSize::Compact, true));
        assert_eq!(
            resolve("workflows.execution-summary", &modules, false),
            Err(WindowError::Locked)
        );
        assert!(resolve("workflows.execution-summary", &modules, true).is_ok());
        assert_eq!(
            resolve("notes.viewer", &modules, true).unwrap().size,
            WindowSize::Large
        );
        assert_eq!(
            resolve("notes.viewer", &modules, false),
            Err(WindowError::Locked)
        );
        for unknown in [
            "security.unknown",
            "notes.other",
            "nothing.here",
            "../index.html",
            "a",
        ] {
            assert!(matches!(
                resolve(unknown, &modules, true),
                Err(WindowError::Unknown(_))
            ));
        }
        modules
            .set_enabled(&ModuleId::new("notes").unwrap(), false)
            .unwrap();
        assert_eq!(
            resolve("notes.viewer", &modules, true),
            Err(WindowError::ModuleDisabled("Notes".into()))
        );
    }

    #[test]
    fn labels_round_trip() {
        assert_eq!(label_for("security.unlock"), "w-security_unlock");
        assert_eq!(
            label_for("workflows.execution-summary"),
            "w-workflows_execution-summary"
        );
    }

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 25.0,
        width: 1440.0,
        height: 875.0,
    };

    #[test]
    fn windows_fit_their_content_within_limits() {
        let compact = limits(WindowSize::Compact);
        // Grows with content, never below the minimum or above the maximum.
        assert_eq!(fit_size((380.4, 312.2), compact, SCREEN), (381.0, 313.0));
        assert_eq!(fit_size((10.0, 10.0), compact, SCREEN), (360.0, 120.0));
        assert_eq!(fit_size((2000.0, 2000.0), compact, SCREEN), (440.0, 760.0));
        // A small screen limits the height further; the content scrolls.
        let small = Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 500.0,
        };
        assert_eq!(fit_size((400.0, 900.0), compact, small), (400.0, 452.0));
        // A screen narrower than the minimum still gets a window that fits.
        let tiny = Rect {
            width: 300.0,
            ..small
        };
        assert!(fit_size((400.0, 200.0), compact, tiny).0 <= 300.0);
    }

    #[test]
    fn decorations_are_measured_not_assumed() {
        let compact = limits(WindowSize::Compact);
        // macOS here: the window size includes a 28 pt title bar, so a
        // 440 x 239 window has a 440 x 211 viewport. Content of 440 x 239
        // needs a 440 x 267 window.
        let mac = Measurement {
            content: (440.0, 239.0),
            viewport: (440.0, 211.0),
            window: (440.0, 239.0),
            frame: (0.0, 0.0),
        };
        assert_eq!(window_size(mac, compact, SCREEN), (440.0, 267.0));
        // Once the window has that size, the same content needs no change:
        // no resize loop.
        let settled = Measurement {
            viewport: (440.0, 239.0),
            window: (440.0, 267.0),
            ..mac
        };
        assert_eq!(window_size(settled, compact, SCREEN), (440.0, 267.0));
        // Windows/Linux: the viewport is the whole window size; the frame
        // lies outside and only reduces the room on the screen.
        let framed = Measurement {
            content: (400.0, 300.0),
            viewport: (360.0, 120.0),
            window: (360.0, 120.0),
            frame: (16.0, 39.0),
        };
        assert_eq!(window_size(framed, compact, SCREEN), (400.0, 300.0));
        let short = Rect {
            height: 400.0,
            ..SCREEN
        };
        let tall = Measurement {
            content: (400.0, 600.0),
            ..framed
        };
        assert_eq!(
            window_size(tall, compact, short).1,
            400.0 - 39.0 - 2.0 * MARGIN
        );
        // A hidden window may report no viewport yet: nothing is added.
        let hidden = Measurement {
            viewport: (0.0, 0.0),
            ..mac
        };
        assert_eq!(window_size(hidden, compact, SCREEN), (440.0, 239.0));
        // Implausible differences are capped.
        let odd = Measurement {
            viewport: (440.0, 10.0),
            window: (440.0, 700.0),
            ..mac
        };
        assert_eq!(
            window_size(odd, compact, SCREEN).1,
            239.0 + MAX_INNER_DECORATION
        );
    }

    #[test]
    fn windows_open_on_screen_and_do_not_jump() {
        let size = (400.0, 300.0);
        // Centred horizontally, in the upper third.
        assert_eq!(place(size, SCREEN, None), (520.0, 217.0));
        // A second monitor to the left (negative coordinates).
        let left = Rect {
            x: -1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1050.0,
        };
        let (x, y) = place(size, left, None);
        assert!(x >= -1920.0 && x + size.0 <= 0.0 && y >= 0.0);
        // A window that still fits stays where it is when its height changes.
        assert_eq!(
            place((400.0, 340.0), SCREEN, Some((100.0, 100.0))),
            (100.0, 100.0)
        );
        // One that would leave the screen is moved back.
        let moved = place((400.0, 340.0), SCREEN, Some((1300.0, 700.0)));
        assert!(SCREEN.contains(moved.0, moved.1, 400.0, 340.0));
    }

    #[test]
    fn work_areas_are_converted_for_display_scaling() {
        // A 2x display reports 2880 x 1750 physical pixels for a 1440 x 875
        // logical work area; a 1.5x display on the left has a negative origin.
        let logical = to_logical((0, 50), (2880, 1750), 2.0);
        assert_eq!(logical, SCREEN);
        assert_eq!(
            to_logical((-2880, 0), (2880, 1620), 1.5),
            Rect {
                x: -1920.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0
            }
        );
        assert_eq!(to_logical((0, 0), (100, 100), 0.0).width, 100.0);
        assert_eq!(
            fit_size((400.0, 300.0), limits(WindowSize::Compact), logical),
            (400.0, 300.0)
        );
    }
}
