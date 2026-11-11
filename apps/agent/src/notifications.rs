//! Showing the notifications actions ask for: in the configuration app's
//! status bar while it is open, and as native desktop notifications.
//!
//! A notification that cannot be shown anywhere is reported back to the
//! action, which decides whether that fails it; nothing here blocks or
//! interrupts a workflow for longer than the native helper takes.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use deckpilot_core::notify::{self, Delivery, Notification, Notifier};
use deckpilot_core::process::{ProcessLimits, minimal_environment};
use deckpilot_core::{CancellationToken, Os};
use deckpilot_ipc::AgentEvent;
use tokio::sync::broadcast;

/// How long the native helper (`osascript`, `notify-send`, PowerShell) may
/// take.
const NATIVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Native notifications through the platform's own tool.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemNotifier;

impl Notifier for SystemNotifier {
    fn notify(&self, notification: &Notification) -> Result<Delivery, String> {
        let plan = notify::plan(Os::current(), notification)
            .ok_or_else(|| "desktop notifications are not supported here".to_owned())?;
        let mut command = std::process::Command::new(&plan.program);
        command.args(&plan.args);
        minimal_environment(
            &mut command,
            // The session bus, for notify-send.
            &["DBUS_SESSION_BUS_ADDRESS", "DISPLAY", "WAYLAND_DISPLAY"],
        );
        for (name, value) in &plan.env {
            command.env(name, value);
        }
        let token = CancellationToken::new()
            .child_with_deadline(std::time::Instant::now() + NATIVE_TIMEOUT);
        let output = deckpilot_core::process::run(command, Some(&token), ProcessLimits::default())
            .map_err(|error| match error.code() {
                "program-not-found" => format!("{} is not installed", plan.program),
                _ => error.to_string(),
            })?;
        if output.success() {
            Ok(Delivery {
                desktop: true,
                status_bar: false,
            })
        } else {
            let reason = output
                .stderr
                .lines()
                .next()
                .unwrap_or("it was refused")
                .trim();
            Err(format!("the desktop notification failed: {reason}"))
        }
    }
}

/// The agent's notifier: the status bar of every open app window, and the
/// native notifier when one is configured.
#[derive(Debug)]
pub(crate) struct AgentNotifier {
    pub events: broadcast::Sender<AgentEvent>,
    pub desktop_clients: Arc<AtomicUsize>,
    pub native: Option<Arc<dyn Notifier>>,
}

impl Notifier for AgentNotifier {
    fn notify(&self, notification: &Notification) -> Result<Delivery, String> {
        let status_bar = notification.status_bar
            && self.desktop_clients.load(Ordering::SeqCst) > 0
            && self
                .events
                .send(AgentEvent::Notification {
                    notification: notification.clone(),
                })
                .is_ok();
        let (desktop, problem) = match (&self.native, notification.desktop) {
            (_, false) => (false, None),
            (None, true) => (
                false,
                Some("desktop notifications are not available".to_owned()),
            ),
            (Some(native), true) => match native.notify(notification) {
                Ok(delivery) => (delivery.desktop, None),
                Err(reason) => (false, Some(reason)),
            },
        };
        if status_bar || desktop {
            Ok(Delivery {
                desktop,
                status_bar,
            })
        } else {
            Err(problem.unwrap_or_else(|| {
                "the app is not open and desktop notifications are off".to_owned()
            }))
        }
    }
}
