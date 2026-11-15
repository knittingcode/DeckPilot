//! How the configuration app's process lives next to the agent.
//!
//! The app is a window onto the agent, not the agent itself: the process
//! runs while one of its windows is open and ends when the last one closes
//! (Tauri's default, kept on purpose). On macOS that is what takes its icon
//! out of the Dock again; the agent keeps running with its menu bar icon,
//! as an accessory application that never appears in the Dock. There is no
//! hidden window and no switching of the activation policy.
//!
//! Three ways to leave, and what each one stops:
//!
//! | How                                   | Windows | App process | Agent |
//! | ------------------------------------- | ------- | ----------- | ----- |
//! | Close Window (⌘W, the close button)   | that one | when it was the last | keeps running |
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

pub fn show_target(unlocked: bool, has_main: bool, macos: bool) -> ShowTarget {
    match (unlocked, has_main) {
        (false, _) => ShowTarget::Prompt,
        (true, true) => ShowTarget::Main,
        (true, false) if macos => ShowTarget::CreateMain,
        (true, false) => ShowTarget::Nothing,
    }
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
