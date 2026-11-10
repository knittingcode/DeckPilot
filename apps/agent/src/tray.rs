//! Native system tray.
//!
//! The tray needs the platform event loop on the main thread (required by
//! macOS and GTK). The agent itself runs on a Tokio runtime in a background
//! thread; state snapshots reach the tray through the event loop proxy, and
//! menu selections go back to the agent as [`TrayCommand`]s.

use deckpilot_device::ConnectionState;
use deckpilot_ipc::{AgentEvent, AgentStatus, SecurityState};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tracing::{error, warn};
use tray_icon::menu::{
    AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::InstanceLock;
use crate::service::{Agent, AgentHandle, AgentOptions, TrayCommand, VERSION};

#[derive(Debug)]
enum UserEvent {
    Ready(AgentHandle),
    State(Box<AgentStatus>),
    Menu(MenuEvent),
    Exit,
}

const OPEN: &str = "open";
const REFRESH: &str = "refresh";
const QUIT: &str = "quit";
const PROFILE_PREFIX: &str = "profile:";

/// Runs the agent with a tray icon until the user quits or the agent stops.
/// Never returns on some platforms; the process exits from the event loop.
pub fn run(options: AgentOptions, lock: InstanceLock) {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        let mut event_loop = event_loop;
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
        run_loop(event_loop, options, lock);
    }
    #[cfg(not(target_os = "macos"))]
    run_loop(event_loop, options, lock);
}

fn run_loop(
    event_loop: tao::event_loop::EventLoop<UserEvent>,
    options: AgentOptions,
    lock: InstanceLock,
) {
    let proxy = event_loop.create_proxy();
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));
    spawn_agent(options, lock, proxy);

    let mut tray: Option<TrayIcon> = None;
    let mut handle: Option<AgentHandle> = None;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => match build_tray() {
                Ok(icon) => tray = Some(icon),
                Err(error) => error!(%error, "cannot create the tray icon"),
            },
            Event::UserEvent(UserEvent::Ready(agent)) => handle = Some(agent),
            Event::UserEvent(UserEvent::State(status)) => {
                if let Some(tray) = &tray {
                    tray.set_menu(Some(Box::new(build_menu(Some(&status)))));
                    let tooltip = match &status.active_profile {
                        Some(profile) => format!("DeckPilot – {}", profile.name),
                        None => "DeckPilot".into(),
                    };
                    let _ = tray.set_tooltip(Some(tooltip));
                }
            }
            Event::UserEvent(UserEvent::Menu(event)) => {
                if let (Some(handle), Some(command)) = (&handle, command_for(event.id.as_ref())) {
                    handle.tray(command);
                }
            }
            Event::UserEvent(UserEvent::Exit) => {
                tray.take();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

fn spawn_agent(options: AgentOptions, lock: InstanceLock, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                error!(%error, "cannot start the async runtime");
                let _ = proxy.send_event(UserEvent::Exit);
                return;
            }
        };
        runtime.block_on(async {
            let agent = match Agent::start(options, lock) {
                Ok(agent) => agent,
                Err(error) => {
                    error!(%error, "agent failed to start");
                    let _ = proxy.send_event(UserEvent::Exit);
                    return;
                }
            };
            let handle = agent.handle();
            let mut events = handle.subscribe();
            let _ = proxy.send_event(UserEvent::Ready(handle.clone()));
            let bridge_proxy = proxy.clone();
            tokio::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match events.recv().await {
                        Ok(AgentEvent::State { status }) => {
                            let _ = bridge_proxy.send_event(UserEvent::State(status));
                        }
                        Ok(_) | Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            if let Err(error) = agent.run(crate::service::shutdown_signal()).await {
                error!(%error, "agent stopped with an error");
            }
        });
        let _ = proxy.send_event(UserEvent::Exit);
    });
}

fn command_for(id: &str) -> Option<TrayCommand> {
    match id {
        OPEN => Some(TrayCommand::OpenWindow),
        REFRESH => Some(TrayCommand::RefreshDevices),
        QUIT => Some(TrayCommand::Quit),
        _ => id
            .strip_prefix(PROFILE_PREFIX)?
            .parse()
            .ok()
            .map(TrayCommand::SelectProfile),
    }
}

fn build_tray() -> Result<TrayIcon, String> {
    TrayIconBuilder::new()
        .with_icon(icon()?)
        .with_tooltip("DeckPilot")
        .with_menu(Box::new(build_menu(None)))
        .build()
        .map_err(|e| e.to_string())
}

fn build_menu(status: Option<&AgentStatus>) -> Menu {
    let menu = Menu::new();
    let append = |result: Result<(), tray_icon::menu::Error>| {
        if let Err(error) = result {
            warn!(%error, "cannot build the tray menu");
        }
    };

    let security = status.map_or(SecurityState::Locked, |s| s.security);
    let open_label = match security {
        SecurityState::Unlocked => "Open DeckPilot",
        _ => "Open DeckPilot to Unlock…",
    };
    append(menu.append(&MenuItem::with_id(OPEN, open_label, true, None)));
    append(menu.append(&PredefinedMenuItem::separator()));
    if let Some(notice) = security_notice(security) {
        append(menu.append(&MenuItem::new(notice, false, None)));
        append(menu.append(&PredefinedMenuItem::separator()));
    }

    let profiles = Submenu::new("Active Profile", security == SecurityState::Unlocked);
    match status.map(|s| s.profiles.as_slice()).unwrap_or_default() {
        [] => append(profiles.append(&MenuItem::new("No profiles", false, None))),
        list => {
            let active = status.and_then(|s| s.active_profile.as_ref()).map(|p| p.id);
            for profile in list {
                let id = format!("{PROFILE_PREFIX}{}", profile.id);
                let item = CheckMenuItem::with_id(
                    id,
                    &profile.name,
                    true,
                    Some(profile.id) == active,
                    None,
                );
                append(profiles.append(&item));
            }
        }
    }
    append(menu.append(&profiles));

    if let Some(project) = status.and_then(|s| s.active_project.as_ref()) {
        let label = format!("Project: {} – {}", project.key, project.name);
        append(menu.append(&MenuItem::new(label, false, None)));
    }

    let devices = Submenu::new("Connected Devices", true);
    match status.map(|s| s.devices.as_slice()).unwrap_or_default() {
        [] => append(devices.append(&MenuItem::new("No devices", false, None))),
        list => {
            for device in list {
                let state = match device.state {
                    ConnectionState::Connected => "connected",
                    ConnectionState::Disconnected => "disconnected",
                };
                let label = format!("{} ({}) – {state}", device.model_name, device.id.serial);
                append(devices.append(&MenuItem::new(label, false, None)));
            }
        }
    }
    append(menu.append(&devices));
    append(menu.append(&MenuItem::with_id(REFRESH, "Refresh Devices", true, None)));
    append(menu.append(&PredefinedMenuItem::separator()));

    let about = AboutMetadata {
        name: Some("DeckPilot".into()),
        version: Some(VERSION.into()),
        license: Some("Apache-2.0".into()),
        website: Some("https://github.com/knittingcode/DeckPilot".into()),
        ..Default::default()
    };
    append(menu.append(&PredefinedMenuItem::about(
        Some("About DeckPilot"),
        Some(about),
    )));
    append(menu.append(&MenuItem::with_id(QUIT, "Quit DeckPilot", true, None)));
    menu
}

/// What the tray says while the configuration cannot be read.
fn security_notice(security: SecurityState) -> Option<&'static str> {
    match security {
        SecurityState::Unlocked => None,
        SecurityState::Locked => Some("Locked – actions are paused"),
        SecurityState::Uninitialized => Some("Set a password to start"),
        SecurityState::LegacyPlaintext => Some("Encrypt your configuration to continue"),
    }
}

/// A 32 x 32 tray icon drawn in code: a rounded key grid with one lit key.
fn icon() -> Result<Icon, String> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let mut put = |x: u32, y: u32, color: [u8; 4]| {
        let offset = ((y * SIZE + x) * 4) as usize;
        rgba[offset..offset + 4].copy_from_slice(&color);
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let corner = |c: u32| !(3..=SIZE - 4).contains(&c);
            if !(corner(x) && corner(y)) {
                put(x, y, [24, 24, 27, 255]);
            }
        }
    }
    for row in 0..3 {
        for column in 0..3 {
            let color = if (row, column) == (1, 1) {
                [56, 189, 248, 255]
            } else {
                [113, 113, 122, 255]
            };
            for y in 0..6 {
                for x in 0..6 {
                    put(5 + column * 8 + x, 5 + row * 8 + y, color);
                }
            }
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_states_are_explained() {
        assert_eq!(security_notice(SecurityState::Unlocked), None);
        for state in [
            SecurityState::Locked,
            SecurityState::Uninitialized,
            SecurityState::LegacyPlaintext,
        ] {
            assert!(security_notice(state).is_some());
        }
    }

    #[test]
    fn menu_ids_map_to_commands() {
        assert_eq!(command_for(OPEN), Some(TrayCommand::OpenWindow));
        assert_eq!(command_for(QUIT), Some(TrayCommand::Quit));
        assert_eq!(command_for(REFRESH), Some(TrayCommand::RefreshDevices));
        let id = deckpilot_core::ProfileId::new();
        assert_eq!(
            command_for(&format!("{PROFILE_PREFIX}{id}")),
            Some(TrayCommand::SelectProfile(id))
        );
        assert_eq!(command_for("profile:not-a-uuid"), None);
        assert_eq!(command_for("unknown"), None);
    }
}
