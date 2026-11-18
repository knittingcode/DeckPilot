//! How the configuration app's process lives next to the agent.
//!
//! The app is a window onto the agent, not the agent itself. The agent is a
//! menu bar (accessory) application that never appears in the Dock
//! (`LSUIElement` in its embedded `Info.plist`, and the accessory policy of
//! its event loop).
//!
//! **macOS** (like Docker Desktop): closing the main window (the close
//! button, ⌘W) hides it and switches the app to the accessory activation
//! policy, so its icon leaves the Dock while the process keeps running.
//! Showing any window again (the tray's *Open DeckPilot*, opening the app
//! from Finder, Spotlight or the Dock, a password prompt) switches back to
//! the regular policy first. The policy follows the windows: regular while
//! at least one is visible, accessory when none is ([`in_dock`],
//! [`sync_dock`]). **Windows and Linux** keep Tauri's default: the process
//! ends when its last window closes.
//!
//! Three ways to leave, and what each one stops:
//!
//! | How                                   | Windows | App process | Agent |
//! | ------------------------------------- | ------- | ----------- | ----- |
//! | Close Window (⌘W, the close button)   | that one | macOS: keeps running, out of the Dock; elsewhere: ends with the last | keeps running |
//! | Quit DeckPilot UI (⌘Q, the app menu)  | all     | ends        | keeps running |
//! | Quit DeckPilot (the agent's tray)     | all     | ends        | stops |
//!
//! The tray's Quit reaches the app as the agent's `shuttingDown` event with
//! the reason `quit` ([`on_agent_stop`]). The agent's "Open DeckPilot"
//! reaches a running app as `showWindow` ([`show_target`]); only when no app
//! is connected does the agent start one, so there is never a second app.
//!
//! Both decisions are macOS behaviour: on Windows and Linux the app keeps
//! what it did before (it stays open after the tray's Quit, and an app
//! without a main window ignores `showWindow`). They take the platform as a
//! parameter ([`MACOS`] in the app) so that both sides are tested
//! everywhere.

use deckpilot_ipc::ShutdownReason;

/// Whether this build follows the macOS lifecycle.
pub const MACOS: bool = cfg!(target_os = "macos");

/// What the app does when its agent stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnAgentStop {
    /// The user quit DeckPilot completely: close every window and end.
    Exit,
    /// A restart, an update or a crash: stay open, show the agent as
    /// stopped and reconnect when it is back.
    Reconnect,
}

/// On macOS the tray's Quit ends the app too; elsewhere the app stays.
pub fn on_agent_stop(reason: Option<ShutdownReason>, macos: bool) -> OnAgentStop {
    if macos && ShutdownReason::closes_apps(reason) {
        OnAgentStop::Exit
    } else {
        OnAgentStop::Reconnect
    }
}

/// What "Open DeckPilot" (the agent's `showWindow`) shows in this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowTarget {
    /// Bring the main window to the front.
    Main,
    /// This process has no main window (it was started for one prompt or
    /// module window only): create it, so the request is never lost
    /// (macOS).
    CreateMain,
    /// The configuration is locked: the password prompt.
    Prompt,
    /// No main window and not macOS: nothing, as before.
    Nothing,
}

/// What closing the main window (the close button, ⌘W) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainClose {
    /// Tauri's default: the window is destroyed; the process ends with the
    /// last window (Windows, Linux).
    Destroy,
    /// macOS: hide it and leave the Dock; the process keeps running.
    HideToMenuBar,
}

pub fn main_close(macos: bool) -> MainClose {
    if macos {
        MainClose::HideToMenuBar
    } else {
        MainClose::Destroy
    }
}

/// Whether the app belongs in the Dock (the regular activation policy):
/// while at least one of its windows is visible.
pub fn in_dock(visible_windows: usize) -> bool {
    visible_windows > 0
}

/// Whether the main window may be shown by the startup fallback (shown
/// after a while if the frontend never asks). Not once it was shown: the
/// user may have closed it to the menu bar since.
pub fn startup_fallback_shows(
    main_visible: bool,
    presented_before: bool,
    prompts_open: bool,
) -> bool {
    !main_visible && !presented_before && !prompts_open
}

pub fn show_target(unlocked: bool, has_main: bool, macos: bool) -> ShowTarget {
    match (unlocked, has_main) {
        (false, _) => ShowTarget::Prompt,
        (true, true) => ShowTarget::Main,
        (true, false) if macos => ShowTarget::CreateMain,
        (true, false) => ShowTarget::Nothing,
    }
}

/// Set once the main window was shown (see [`startup_fallback_shows`]).
#[derive(Debug, Default)]
pub struct MainPresented(pub std::sync::atomic::AtomicBool);

/// Shows `window` and brings it to the front. On macOS the app first
/// returns to the Dock (regular policy), so the window and its Dock icon
/// come back together. Every path that shows a window goes through here.
pub fn present<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    use tauri::Manager;
    let app = window.app_handle();
    #[cfg(target_os = "macos")]
    {
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    }
    if window.label() == crate::windows::MAIN
        && let Some(presented) = app.try_state::<MainPresented>()
    {
        presented.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// macOS: the activation policy that matches the visible windows, after a
/// window was hidden or destroyed. `leaving` is a window that is going
/// away and still counts as visible. Elsewhere nothing.
pub fn sync_dock<R: tauri::Runtime>(app: &tauri::AppHandle<R>, leaving: Option<&str>) {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        let visible = app
            .webview_windows()
            .values()
            .filter(|w| Some(w.label()) != leaving && w.is_visible().unwrap_or(false))
            .count();
        let policy = if in_dock(visible) {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        let _ = app.set_activation_policy(policy);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, leaving);
}

/// macOS: the main window's close button or ⌘W. Hides the window instead
/// of destroying it and leaves the Dock when no other window is visible.
pub fn hide_to_menu_bar<R: tauri::Runtime>(window: &tauri::Window<R>) {
    use tauri::Manager;
    let _ = window.hide();
    sync_dock(window.app_handle(), Some(window.label()));
}

/// The app's window events: on macOS the main window's close button and ⌘W
/// hide it and take the app out of the Dock, and the Dock follows every
/// window that is destroyed.
pub fn on_window_event<R: tauri::Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    use tauri::Manager;
    match event {
        tauri::WindowEvent::CloseRequested { api, .. }
            if window.label() == crate::windows::MAIN
                && main_close(MACOS) == MainClose::HideToMenuBar =>
        {
            api.prevent_close();
            hide_to_menu_bar(window);
        }
        tauri::WindowEvent::Destroyed => {
            let app = window.app_handle();
            let unlocked = app.state::<crate::state::AppState>().is_unlocked();
            crate::windows::closed(app, window.label(), unlocked);
            sync_dock(app, Some(window.label()));
        }
        _ => {}
    }
}

/// The app's run loop events: on macOS, opening the app from Finder,
/// Spotlight or the Dock while it runs without a visible window (closed to
/// the menu bar) shows it again, like the tray's *Open DeckPilot*.
pub fn on_run_event<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: &tauri::RunEvent) {
    #[cfg(target_os = "macos")]
    if let tauri::RunEvent::Reopen {
        has_visible_windows: false,
        ..
    } = event
    {
        crate::agent::show_window(app);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, event);
}

/// The macOS application menu's Quit item. It ends only this app, so it
/// does not share the tray's "Quit DeckPilot", which stops the agent.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const QUIT_UI: &str = "Quit DeckPilot UI";

/// An entry of the macOS application menu, in order.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMenuItem {
    About,
    Separator,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    /// Ends this app (⌘Q), labelled [`QUIT_UI`].
    Quit,
}

/// The macOS application menu: the standard one, with Quit named for what
/// it does.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const APP_MENU: &[AppMenuItem] = &[
    AppMenuItem::About,
    AppMenuItem::Separator,
    AppMenuItem::Services,
    AppMenuItem::Separator,
    AppMenuItem::Hide,
    AppMenuItem::HideOthers,
    AppMenuItem::ShowAll,
    AppMenuItem::Separator,
    AppMenuItem::Quit,
];

/// Tauri's default menu with [`APP_MENU`] as the application menu. Other
/// platforms keep Tauri's default menu.
#[cfg(target_os = "macos")]
pub fn menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{AboutMetadata, IsMenuItem, Menu, PredefinedMenuItem, Submenu};

    let package = app.package_info();
    let about = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        authors: app.config().bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };
    let items = APP_MENU
        .iter()
        .map(|item| match item {
            AppMenuItem::About => PredefinedMenuItem::about(app, None, Some(about.clone())),
            AppMenuItem::Separator => PredefinedMenuItem::separator(app),
            AppMenuItem::Services => PredefinedMenuItem::services(app, None),
            AppMenuItem::Hide => PredefinedMenuItem::hide(app, None),
            AppMenuItem::HideOthers => PredefinedMenuItem::hide_others(app, None),
            AppMenuItem::ShowAll => PredefinedMenuItem::show_all(app, None),
            AppMenuItem::Quit => PredefinedMenuItem::quit(app, Some(QUIT_UI)),
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let items: Vec<&dyn IsMenuItem<R>> = items.iter().map(|i| i as &dyn IsMenuItem<R>).collect();
    let application = Submenu::with_items(app, package.name.clone(), true, &items)?;
    // The default's first entry is its application menu.
    let menu = Menu::default(app)?;
    menu.remove_at(0)?;
    menu.prepend(&application)?;
    Ok(menu)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAYS: [Option<ShutdownReason>; 5] = [
        Some(ShutdownReason::Requested),
        Some(ShutdownReason::Update),
        Some(ShutdownReason::Signal),
        Some(ShutdownReason::Other),
        None,
    ];

    #[test]
    fn closing_the_main_window_hides_it_to_the_menu_bar_on_macos_only() {
        assert_eq!(main_close(true), MainClose::HideToMenuBar);
        assert_eq!(main_close(false), MainClose::Destroy);
    }

    #[test]
    fn the_app_is_in_the_dock_while_a_window_is_visible() {
        assert!(!in_dock(0));
        assert!(in_dock(1));
        assert!(in_dock(3));
    }

    #[test]
    fn the_startup_fallback_never_reopens_a_main_window_the_user_closed() {
        // Never shown, nothing else open: the fallback shows it.
        assert!(startup_fallback_shows(false, false, false));
        // Shown once and closed to the menu bar since: it stays closed.
        assert!(!startup_fallback_shows(false, true, false));
        // Already visible, or a password prompt is open: nothing to do.
        assert!(!startup_fallback_shows(true, false, false));
        assert!(!startup_fallback_shows(false, false, true));
    }

    #[test]
    fn on_macos_only_quitting_deckpilot_ends_the_app_with_the_agent() {
        assert_eq!(
            on_agent_stop(Some(ShutdownReason::Quit), true),
            OnAgentStop::Exit
        );
        // A restart from the app, an update, a signal, an older agent that
        // gives no reason, a reason added later: the app stays.
        for reason in STAYS {
            assert_eq!(
                on_agent_stop(reason, true),
                OnAgentStop::Reconnect,
                "{reason:?}"
            );
        }
    }

    #[test]
    fn elsewhere_the_app_always_stays_when_the_agent_stops() {
        for reason in STAYS.into_iter().chain([Some(ShutdownReason::Quit)]) {
            assert_eq!(
                on_agent_stop(reason, false),
                OnAgentStop::Reconnect,
                "{reason:?}"
            );
        }
    }

    #[test]
    fn this_build_follows_its_platform() {
        assert_eq!(MACOS, cfg!(target_os = "macos"));
        let expected = if cfg!(target_os = "macos") {
            (OnAgentStop::Exit, ShowTarget::CreateMain)
        } else {
            (OnAgentStop::Reconnect, ShowTarget::Nothing)
        };
        assert_eq!(
            (
                on_agent_stop(Some(ShutdownReason::Quit), MACOS),
                show_target(true, false, MACOS)
            ),
            expected
        );
    }

    #[test]
    fn open_deckpilot_on_macos_always_shows_something() {
        assert_eq!(show_target(true, true, true), ShowTarget::Main);
        // A process started for one window only gets a main window.
        assert_eq!(show_target(true, false, true), ShowTarget::CreateMain);
        assert_eq!(show_target(false, true, true), ShowTarget::Prompt);
        assert_eq!(show_target(false, false, true), ShowTarget::Prompt);
    }

    #[test]
    fn open_deckpilot_elsewhere_is_unchanged() {
        assert_eq!(show_target(true, true, false), ShowTarget::Main);
        assert_eq!(show_target(true, false, false), ShowTarget::Nothing);
        assert_eq!(show_target(false, true, false), ShowTarget::Prompt);
        assert_eq!(show_target(false, false, false), ShowTarget::Prompt);
    }

    #[test]
    fn the_app_menu_quits_only_the_ui() {
        assert_eq!(APP_MENU.last(), Some(&AppMenuItem::Quit));
        assert_eq!(
            APP_MENU
                .iter()
                .filter(|item| **item == AppMenuItem::Quit)
                .count(),
            1
        );
        assert_eq!(QUIT_UI, "Quit DeckPilot UI");
        // Distinct from the tray's item, which stops the agent.
        assert_ne!(QUIT_UI, "Quit DeckPilot");
        for item in [
            AppMenuItem::About,
            AppMenuItem::Services,
            AppMenuItem::Hide,
            AppMenuItem::HideOthers,
            AppMenuItem::ShowAll,
        ] {
            assert!(APP_MENU.contains(&item), "{item:?}");
        }
    }
}
