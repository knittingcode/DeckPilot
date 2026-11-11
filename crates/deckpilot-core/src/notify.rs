//! Notifications an action asks to show, and how they reach the desktop.
//!
//! An action builds a [`Notification`] and calls
//! [`ExecutionContext::notify`](crate::ExecutionContext::notify), which
//! requires the module's
//! [`Capability::Notifications`](crate::Capability::Notifications) and
//! hands it to the engine's
//! [`Notifier`]. The agent's notifier shows it in the app's status bar and
//! as a native notification ([`plan`]). Text is plain: control characters
//! are removed, lengths are bounded, and markup is escaped where a
//! notification service would interpret it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Os;

/// Longest title, in characters.
pub const MAX_TITLE: usize = 100;
/// Longest body, in characters.
pub const MAX_BODY: usize = 500;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl Severity {
    pub fn parse(text: &str) -> Self {
        match text {
            "success" => Self::Success,
            "warning" => Self::Warning,
            "error" => Self::Error,
            _ => Self::Info,
        }
    }
}

/// A notification, already made safe with [`Notification::new`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default)]
    pub severity: Severity,
    /// Shown as a native desktop notification.
    #[serde(default)]
    pub desktop: bool,
    /// Shown in the configuration app's status bar while it is open.
    #[serde(default)]
    pub status_bar: bool,
}

impl Notification {
    /// A notification with `title` and `body` made safe: one line of title,
    /// no control characters, bounded lengths.
    pub fn new(title: &str, body: &str, severity: Severity) -> Self {
        Self {
            title: plain(title, MAX_TITLE, false),
            body: plain(body, MAX_BODY, true),
            severity,
            desktop: true,
            status_bar: true,
        }
    }
}

/// `text` without control characters (line breaks only where `lines`),
/// trimmed and cut to `max` characters with an ellipsis.
pub fn plain(text: &str, max: usize, lines: bool) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control() || (lines && *c == '\n'))
        // Bidirectional overrides can make text read differently.
        .filter(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= max {
        return cleaned.to_owned();
    }
    let mut cut: String = cleaned.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Where a notification was shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub desktop: bool,
    pub status_bar: bool,
}

impl Delivery {
    pub fn shown(self) -> bool {
        self.desktop || self.status_bar
    }
}

/// Shows notifications. `Err` carries why nothing could be shown (for
/// example notifications are not available on this desktop).
pub trait Notifier: Send + Sync + fmt::Debug {
    fn notify(&self, notification: &Notification) -> Result<Delivery, String>;
}

/// Shows nothing; every notification fails.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoNotifier;

impl Notifier for NoNotifier {
    fn notify(&self, _: &Notification) -> Result<Delivery, String> {
        Err("notifications are not available".into())
    }
}

/// Records notifications instead of showing them, for tests.
#[derive(Debug, Default)]
pub struct RecordingNotifier {
    shown: std::sync::Mutex<Vec<Notification>>,
    fail: bool,
}

impl RecordingNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// A notifier whose notifications cannot be shown, like a desktop
    /// where they are denied.
    pub fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }

    pub fn shown(&self) -> Vec<Notification> {
        self.shown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Notifier for RecordingNotifier {
    fn notify(&self, notification: &Notification) -> Result<Delivery, String> {
        if self.fail {
            return Err("notifications are turned off for DeckPilot".into());
        }
        self.shown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(notification.clone());
        Ok(Delivery {
            desktop: notification.desktop,
            status_bar: notification.status_bar,
        })
    }
}

/// The program that shows a native notification on `os`. Text travels as
/// arguments or environment variables, never inside a script, so it cannot
/// be interpreted as code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyPlan {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// PowerShell that shows a toast with the text from `DECKPILOT_TITLE` and
/// `DECKPILOT_BODY`, inserted as XML text nodes (escaped by the DOM).
const WINDOWS_TOAST: &str = r#"$ErrorActionPreference = 'Stop'
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument
$xml.LoadXml('<toast><visual><binding template="ToastGeneric"><text/><text/></binding></visual></toast>')
$texts = $xml.GetElementsByTagName('text')
$texts.Item(0).AppendChild($xml.CreateTextNode($env:DECKPILOT_TITLE)) | Out-Null
$texts.Item(1).AppendChild($xml.CreateTextNode($env:DECKPILOT_BODY)) | Out-Null
$app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($xml))"#;

/// Escapes the markup some Linux notification services interpret.
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn plan(os: Os, notification: &Notification) -> Option<NotifyPlan> {
    match os {
        Os::MacOs => Some(NotifyPlan {
            program: "osascript".into(),
            args: vec![
                "-e".into(),
                "on run argv".into(),
                "-e".into(),
                "display notification (item 2 of argv) with title (item 1 of argv)".into(),
                "-e".into(),
                "end run".into(),
                notification.title.clone(),
                notification.body.clone(),
            ],
            env: Vec::new(),
        }),
        Os::Linux => Some(NotifyPlan {
            program: "notify-send".into(),
            args: vec![
                "--app-name=DeckPilot".into(),
                format!(
                    "--urgency={}",
                    match notification.severity {
                        Severity::Error => "critical",
                        Severity::Warning => "normal",
                        Severity::Info | Severity::Success => "low",
                    }
                ),
                "--".into(),
                escape_markup(&notification.title),
                escape_markup(&notification.body),
            ],
            env: Vec::new(),
        }),
        Os::Windows => Some(NotifyPlan {
            program: "powershell.exe".into(),
            args: vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-Command".into(),
                WINDOWS_TOAST.into(),
            ],
            env: vec![
                ("DECKPILOT_TITLE".into(), notification.title.clone()),
                ("DECKPILOT_BODY".into(), notification.body.clone()),
            ],
        }),
        Os::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_plain_and_bounded() {
        let n = Notification::new(
            "  Build\u{7} done\nnow  ",
            "a\tb\nc\u{1b}[31m\u{202e}x",
            Severity::Success,
        );
        assert_eq!(n.title, "Build donenow");
        assert_eq!(n.body, "a b\nc[31mx");
        let long = Notification::new(&"t".repeat(300), &"b".repeat(900), Severity::Info);
        assert_eq!(long.title.chars().count(), MAX_TITLE);
        assert!(long.title.ends_with('…'));
        assert_eq!(long.body.chars().count(), MAX_BODY);
        assert_eq!(Severity::parse("warning"), Severity::Warning);
        assert_eq!(Severity::parse("nonsense"), Severity::Info);
    }

    #[test]
    fn text_never_becomes_code() {
        let n = Notification::new(
            "\" & do shell script \"rm -rf ~",
            "<b>$(whoami)</b>",
            Severity::Error,
        );
        let mac = plan(Os::MacOs, &n).unwrap();
        // The script is fixed; the text is an argument.
        assert_eq!(mac.args[1], "on run argv");
        assert_eq!(mac.args[6], n.title);
        let linux = plan(Os::Linux, &n).unwrap();
        assert_eq!(linux.args[1], "--urgency=critical");
        assert_eq!(linux.args[2], "--");
        assert_eq!(linux.args[4], "&lt;b&gt;$(whoami)&lt;/b&gt;");
        let windows = plan(Os::Windows, &n).unwrap();
        assert!(
            !windows
                .args
                .iter()
                .any(|a| a.contains("whoami") || a.contains("rm -rf"))
        );
        assert_eq!(windows.env[1], ("DECKPILOT_BODY".into(), n.body.clone()));
        assert!(plan(Os::Other, &n).is_none());
    }

    #[test]
    fn recording_notifiers_record_or_fail() {
        let notifier = RecordingNotifier::new();
        let n = Notification::new("A", "", Severity::Info);
        assert!(notifier.notify(&n).unwrap().shown());
        assert_eq!(notifier.shown(), std::slice::from_ref(&n));
        assert!(RecordingNotifier::failing().notify(&n).is_err());
        assert!(NoNotifier.notify(&n).is_err());
    }
}
