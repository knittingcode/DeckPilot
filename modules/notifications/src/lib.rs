//! The `notifications` feature module: shows notifications from
//! workflows, on the desktop and in the configuration app's status bar.
//!
//! Text is plain (no HTML or markup), bounded and stripped of control
//! characters. A notification that cannot be shown — the app is closed and
//! desktop notifications are off or denied — does not stop the workflow
//! unless the node says it is critical. Notifications contain only what
//! the workflow puts in them; `workflow-summary` reports node counts and
//! the ids of failed nodes, never messages or outputs.

use std::collections::BTreeMap;

use deckpilot_core::graph::{ActionEffect, NodeSpec, OutputSpec, ValueKind};
use deckpilot_core::notify::{Notification, Severity};
use deckpilot_core::{
    ActionHandler, ActionParameters, ActionResult, ActionType, ActionTypeId, Capability,
    ExecutionContext, FeatureModule, ModuleId, ModuleManifest, ParameterKind, ParameterSpec,
    ParameterValue, Portability, RunSummary, SettingScope,
};

pub const MODULE_ID: &str = "notifications";
pub const SHOW: &str = "notifications.show";
pub const WORKFLOW_SUMMARY: &str = "notifications.workflow-summary";

/// Module setting: show native desktop notifications.
pub const DESKTOP: &str = "desktop";
/// Module setting: show them in the app's status bar.
pub const STATUS_BAR: &str = "statusBar";
/// Module setting: less severe notifications are not shown.
pub const MINIMUM_SEVERITY: &str = "minimumSeverity";

const SEVERITIES: [&str; 4] = ["info", "success", "warning", "error"];

/// Stable error codes of the module's actions.
pub mod codes {
    /// A critical notification could not be shown.
    pub const NOT_SHOWN: &str = "notification-not-shown";
}

/// # Panics
///
/// Never: the identifier is a valid constant.
pub fn module_id() -> ModuleId {
    ModuleId::new(MODULE_ID).expect("the module id is valid")
}

/// # Panics
///
/// Never for the constants of this module.
pub fn id(value: &str) -> ActionTypeId {
    ActionTypeId::new(value).expect("the action type id is valid")
}

/// The module's settings.
#[derive(Debug, Clone, Copy)]
pub struct Preferences {
    pub desktop: bool,
    pub status_bar: bool,
    pub minimum: Severity,
}

fn rank(severity: Severity) -> u8 {
    match severity {
        Severity::Info => 0,
        Severity::Success => 1,
        Severity::Warning => 2,
        Severity::Error => 3,
    }
}

pub fn preferences(config: &ActionParameters) -> Preferences {
    Preferences {
        desktop: config.boolean(DESKTOP).unwrap_or(true),
        status_bar: config.boolean(STATUS_BAR).unwrap_or(true),
        minimum: Severity::parse(config.text(MINIMUM_SEVERITY).unwrap_or("info")),
    }
}

fn boolean_setting(key: &str, label: &str, description: &str) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: label.into(),
        description: Some(description.into()),
        kind: ParameterKind::Boolean,
        required: true,
        default: Some(ParameterValue::Boolean(true)),
    }
}

#[derive(Debug)]
pub struct NotificationsModule {
    manifest: ModuleManifest,
}

impl NotificationsModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: module_id(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "Notifications".into(),
                description: "Notifications from workflows on the desktop and in the status bar."
                    .into(),
                platforms: Vec::new(),
                pages: Vec::new(),
                capabilities: vec![Capability::ProfileRead, Capability::Notifications],
                config: vec![
                    boolean_setting(
                        DESKTOP,
                        "Desktop notifications",
                        "Shows them as notifications of the operating system. macOS and Windows \
                         may ask whether to allow them.",
                    ),
                    boolean_setting(
                        STATUS_BAR,
                        "Status bar",
                        "Also lists them in the status bar while DeckPilot is open.",
                    ),
                    ParameterSpec {
                        key: MINIMUM_SEVERITY.into(),
                        label: "Show from".into(),
                        description: Some(
                            "Less important notifications are not shown; their nodes still \
                             succeed."
                                .into(),
                        ),
                        kind: ParameterKind::Choice {
                            options: SEVERITIES.iter().map(|s| (*s).to_owned()).collect(),
                        },
                        required: true,
                        default: Some(ParameterValue::Choice("info".into())),
                    },
                ],
                dependencies: Vec::new(),
                enabled_by_default: true,
                portability: Portability {
                    version: 1,
                    settings: BTreeMap::from([
                        (DESKTOP.to_owned(), SettingScope::Portable),
                        (STATUS_BAR.to_owned(), SettingScope::Portable),
                        (MINIMUM_SEVERITY.to_owned(), SettingScope::Portable),
                    ]),
                },
                migrations: Vec::new(),
                windows: Vec::new(),
            },
        }
    }
}

impl Default for NotificationsModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FeatureModule for NotificationsModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn action_handlers(&self, config: &ActionParameters) -> Vec<Box<dyn ActionHandler>> {
        let preferences = preferences(config);
        vec![
            Box::new(Notify {
                definition: show_definition(),
                summary: false,
                preferences,
            }),
            Box::new(Notify {
                definition: summary_definition(),
                summary: true,
                preferences,
            }),
        ]
    }
}

fn parameter(key: &str, label: &str, kind: ParameterKind, description: &str) -> ParameterSpec {
    ParameterSpec {
        key: key.into(),
        label: label.into(),
        description: Some(description.into()),
        kind,
        required: false,
        default: None,
    }
}

fn text(max: usize, multiline: bool) -> ParameterKind {
    ParameterKind::Text {
        multiline,
        max_length: Some(max),
    }
}

fn critical() -> ParameterSpec {
    ParameterSpec {
        required: true,
        default: Some(ParameterValue::Boolean(false)),
        ..parameter(
            "critical",
            "Critical",
            ParameterKind::Boolean,
            "Fail the node when the notification cannot be shown. Off: the workflow goes on.",
        )
    }
}

fn node(icon: &str, extra: Vec<OutputSpec>) -> NodeSpec {
    let mut outputs = vec![
        OutputSpec::new("shown", "Shown", ValueKind::Boolean),
        OutputSpec::new("desktop", "On the desktop", ValueKind::Boolean),
        OutputSpec::new("statusBar", "In the status bar", ValueKind::Boolean),
        OutputSpec::new("reason", "Why not shown", ValueKind::Text).optional(),
    ];
    outputs.extend(extra);
    NodeSpec {
        icon: Some(icon.into()),
        outputs,
        timeout_ms: 20_000,
        // Failing means nothing was shown, so trying again is safe.
        retry_safe: true,
        idempotent: false,
        effect: ActionEffect::Opens,
        ..NodeSpec::default()
    }
}

fn show_definition() -> ActionType {
    ActionType {
        id: id(SHOW),
        name: "Show notification".into(),
        description: "Shows a notification with a title, text and severity. Text can include \
                      outputs of earlier nodes; it is shown as plain text."
            .into(),
        category: "Notifications".into(),
        parameters: vec![
            ParameterSpec {
                required: true,
                ..parameter("title", "Title", text(100, false), "Plain text, one line.")
            },
            parameter("body", "Text", text(500, true), "Plain text; no HTML."),
            ParameterSpec {
                required: true,
                default: Some(ParameterValue::Choice("info".into())),
                ..parameter(
                    "severity",
                    "Severity",
                    ParameterKind::Choice {
                        options: SEVERITIES.iter().map(|s| (*s).to_owned()).collect(),
                    },
                    "",
                )
            },
            critical(),
        ],
        requires_project: false,
        node: node("bell", Vec::new()),
    }
}

fn summary_definition() -> ActionType {
    ActionType {
        id: id(WORKFLOW_SUMMARY),
        name: "Workflow summary".into(),
        description: "Shows how the workflow went so far: how many steps succeeded, failed or \
                      were skipped, which failed, and how long it took. Success or error \
                      severity follows the result."
            .into(),
        category: "Notifications".into(),
        parameters: vec![
            parameter(
                "title",
                "Title",
                text(100, false),
                "Empty: the workflow's name.",
            ),
            parameter(
                "details",
                "Details",
                text(400, true),
                "Lines added below the summary, e.g. with outputs of earlier nodes.",
            ),
            ParameterSpec {
                required: true,
                default: Some(ParameterValue::Boolean(false)),
                ..parameter(
                    "onlyOnFailure",
                    "Only when something failed",
                    ParameterKind::Boolean,
                    "",
                )
            },
            critical(),
        ],
        requires_project: false,
        node: node(
            "list-checks",
            vec![OutputSpec::new("summary", "Summary", ValueKind::Text)],
        ),
    }
}

/// `3 succeeded, 1 failed (deploy), 2 skipped · 12 s`.
pub fn describe(summary: &RunSummary) -> String {
    let mut parts = vec![format!("{} succeeded", summary.succeeded)];
    if summary.failed > 0 {
        let mut failed = format!("{} failed", summary.failed);
        if !summary.failed_nodes.is_empty() {
            let shown: Vec<&str> = summary
                .failed_nodes
                .iter()
                .take(5)
                .map(String::as_str)
                .collect();
            failed = format!("{failed} ({})", shown.join(", "));
        }
        parts.push(failed);
    }
    if summary.skipped > 0 {
        parts.push(format!("{} skipped", summary.skipped));
    }
    if summary.cancelled > 0 {
        parts.push(format!("{} cancelled", summary.cancelled));
    }
    let seconds = summary.elapsed_ms / 1000;
    let duration = if seconds >= 60 {
        format!("{} min {} s", seconds / 60, seconds % 60)
    } else {
        format!("{}.{} s", seconds, (summary.elapsed_ms % 1000) / 100)
    };
    format!("{} · {duration}", parts.join(", "))
}

#[derive(Debug)]
struct Notify {
    definition: ActionType,
    summary: bool,
    preferences: Preferences,
}

impl ActionHandler for Notify {
    fn definition(&self) -> &ActionType {
        &self.definition
    }

    fn execute(
        &self,
        parameters: &ActionParameters,
        context: &ExecutionContext<'_>,
    ) -> ActionResult {
        let (title, body, severity, summary) = if self.summary {
            let run = context.run_summary();
            let workflow = run
                .and_then(|r| r.workflow)
                .and_then(|id| context.profile().ok().and_then(|p| p.workflow(id)))
                .map(|w| w.name.clone());
            if run.is_some_and(|r| r.failed == 0)
                && parameters.boolean("onlyOnFailure").unwrap_or(false)
            {
                return ActionResult::success("nothing failed, so no summary was shown")
                    .with_data("shown", "false")
                    .with_data("desktop", "false")
                    .with_data("statusBar", "false")
                    .with_data("reason", "nothing failed")
                    .with_data("summary", run.map(describe).unwrap_or_default());
            }
            let described = run.map(describe).unwrap_or_default();
            let severity = match run {
                Some(r) if r.failed > 0 => Severity::Error,
                Some(r) if r.cancelled > 0 => Severity::Warning,
                Some(_) => Severity::Success,
                None => Severity::Info,
            };
            let title = parameters
                .text("title")
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned)
                .or(workflow)
                .unwrap_or_else(|| "Workflow".to_owned());
            let details = parameters.text("details").unwrap_or_default().trim();
            let body = [described.as_str(), details]
                .iter()
                .filter(|p| !p.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            (title, body, severity, Some(described))
        } else {
            (
                parameters.text("title").unwrap_or_default().to_owned(),
                parameters.text("body").unwrap_or_default().to_owned(),
                Severity::parse(parameters.text("severity").unwrap_or("info")),
                None,
            )
        };
        let mut notification = Notification::new(&title, &body, severity);
        notification.desktop = self.preferences.desktop;
        notification.status_bar = self.preferences.status_bar;
        let result = if rank(severity) < rank(self.preferences.minimum) {
            Err("less important than the module's setting".to_owned())
        } else if !notification.desktop && !notification.status_bar {
            Err("notifications are turned off in Settings › Notifications".to_owned())
        } else {
            context.notify(&notification)
        };
        let mut outcome = match result {
            Ok(delivery) => ActionResult::success(format!("showed “{}”", notification.title))
                .with_data("shown", "true")
                .with_data("desktop", delivery.desktop.to_string())
                .with_data("statusBar", delivery.status_bar.to_string()),
            Err(reason) => {
                let message = format!("“{}” was not shown: {reason}", notification.title);
                let failed = if parameters.boolean("critical").unwrap_or(false) {
                    ActionResult::error(codes::NOT_SHOWN, message)
                } else {
                    ActionResult::success(message)
                };
                failed
                    .with_data("shown", "false")
                    .with_data("desktop", "false")
                    .with_data("statusBar", "false")
                    .with_data("reason", reason)
            }
        };
        if let Some(summary) = summary {
            outcome = outcome.with_data("summary", summary);
        }
        outcome
    }
}

#[cfg(test)]
mod tests;
