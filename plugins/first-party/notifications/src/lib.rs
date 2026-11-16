//! `deckpilot.notifications`: the first-party Notifications plugin, the
//! replacement of the built-in `notifications` module and the first plugin
//! of the default set (`docs/marketplace/module-migration.md` §4.1, P10).
//! It shows the notifications workflows ask for, and a summary of how a
//! workflow went.
//!
//! **The same actions.** Both actions of the built-in, with the same name
//! (`notifications.<action>` → `deckpilot.notifications.<action>`), the
//! same parameters, outputs, messages and error code.
//! `crates/deckpilot-modules/tests/contracts.rs` compares the manifest with
//! the built-in's definitions, and `apps/agent/tests/plugins_notifications.rs`
//! runs both through the real agent.
//!
//! **Where they go is the agent's decision.** The manifest's
//! `notifications` permission names the plugin's settings `desktop`,
//! `statusBar` and `minimumSeverity` as its routing (G9): the agent reads
//! them, as the user set them, when it shows a notification, and answers
//! where it showed it ([`NotifyDelivery`]). This module never reads them;
//! it only turns the answer into the node's outputs, and fails the node for
//! a `critical` notification shown nowhere.
//!
//! **The workflow summary** comes from `run.context` (G8): the invocation's
//! own workflow run only (its name, node counts and failed node ids), never
//! messages or outputs.
//!
//! **Text is plain** and bounded as the built-in bounds it (title 100,
//! body 500 characters, control characters removed). What is sent to the
//! agent is cut further to what `notify` takes (title 64, body 300), and
//! the agent titles it with the plugin's name, "Notifications: …".
#![deny(unsafe_code)]

use deckpilot_plugin_sdk::RunSummary;
use deckpilot_plugin_sdk::prelude::*;

/// The plugin's id and version, as in `manifest.json`.
pub const ID: &str = "deckpilot.notifications";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The severities, least severe first.
pub const SEVERITIES: [&str; 4] = ["info", "success", "warning", "error"];

/// The built-in's bounds of a notification (`deckpilot_core::notify`).
pub const MAX_TITLE: usize = 100;
pub const MAX_BODY: usize = 500;
/// What `notify` takes (ABI 1 `MAX_NOTIFY_TITLE`, `MAX_NOTIFY_BODY`); the
/// agent refuses longer text.
pub const SENT_TITLE: usize = 64;
pub const SENT_BODY: usize = 300;

/// Stable error codes of the plugin's actions (the built-in's).
pub mod codes {
    /// A critical notification could not be shown.
    pub const NOT_SHOWN: &str = "notification-not-shown";
}

/// The plugin definition; `deckpilot_plugin_sdk::plugin!` exports it.
/// Ids, parameters and defaults equal `manifest.json` (checked by
/// `testing::manifest_mismatches` in the unit tests).
pub fn plugin() -> Plugin {
    let critical = || Param::boolean("critical").default(false);
    Plugin::new(ID, VERSION)
        .action(
            Action::new("show", show)
                .param(Param::text("title").max_length(100).required())
                .param(Param::text("body").max_length(500))
                .param(Param::choice("severity", &SEVERITIES).default("info"))
                .param(critical()),
        )
        .action(
            Action::new("workflow-summary", workflow_summary)
                .param(Param::text("title").max_length(100))
                .param(Param::text("details").max_length(400))
                .param(Param::boolean("onlyOnFailure").default(false))
                .param(critical()),
        )
}

deckpilot_plugin_sdk::plugin!(plugin);

/// `text` without control characters (line breaks only where `lines`),
/// trimmed and cut to `max` characters with an ellipsis: the core's
/// `notify::plain`, so the plugin's titles and messages are the
/// built-in's.
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

/// A severity by name; anything else is `info`, as in the built-in.
pub fn severity(name: &str) -> NotifySeverity {
    match name {
        "success" => NotifySeverity::Success,
        "warning" => NotifySeverity::Warning,
        "error" => NotifySeverity::Error,
        _ => NotifySeverity::Info,
    }
}

/// `3 succeeded, 1 failed (deploy), 2 skipped · 12 s`: the built-in's
/// `describe`.
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

fn text(args: &Args<'_>, key: &str) -> Result<String, PluginError> {
    Ok(args.get::<Option<String>>(key)?.unwrap_or_default())
}

fn flag(args: &Args<'_>, key: &str) -> Result<bool, PluginError> {
    Ok(args.get::<Option<bool>>(key)?.unwrap_or(false))
}

fn show(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let title = text(args, "title")?;
    let body = text(args, "body")?;
    let severity = severity(&args.get::<Option<String>>("severity")?.unwrap_or_default());
    deliver(ctx, &title, &body, severity, flag(args, "critical")?, None)
}

fn workflow_summary(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    // The invocation's own run; an agent without `run.context`, or a run
    // that is not a workflow's, has none.
    let run = ctx.run_context().ok().and_then(|context| context.run);
    if run.as_ref().is_some_and(|r| r.failed == 0) && flag(args, "onlyOnFailure")? {
        return Ok(Output::message("nothing failed, so no summary was shown")
            .with("shown", false)
            .with("desktop", false)
            .with("statusBar", false)
            .with("reason", "nothing failed")
            .with("summary", run.as_ref().map(describe).unwrap_or_default()));
    }
    let described = run.as_ref().map(describe).unwrap_or_default();
    let severity = match &run {
        Some(r) if r.failed > 0 => NotifySeverity::Error,
        Some(r) if r.cancelled > 0 => NotifySeverity::Warning,
        Some(_) => NotifySeverity::Success,
        None => NotifySeverity::Info,
    };
    let title = Some(text(args, "title")?)
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .or_else(|| run.as_ref().and_then(|r| r.workflow.clone()))
        .unwrap_or_else(|| "Workflow".to_owned());
    let details = text(args, "details")?;
    let body = [described.as_str(), details.trim()]
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let critical = flag(args, "critical")?;
    deliver(ctx, &title, &body, severity, critical, Some(described))
}

/// Why a `notify` the agent refused showed nothing, for the node's
/// `reason`.
fn refused(error: &CapabilityError) -> String {
    match error.kind() {
        CapabilityErrorKind::Throttled => {
            "more than 5 notifications a minute; this one was dropped".to_owned()
        }
        CapabilityErrorKind::NotPermitted => {
            "the plugin may not show notifications (Settings › Plugins › Notifications)".to_owned()
        }
        _ => error.detail().to_owned(),
    }
}

/// Asks the agent to show the notification and turns where it was shown
/// into the node's result, as the built-in does: shown somewhere is
/// success; shown nowhere is success with the reason, or the error
/// [`codes::NOT_SHOWN`] when `critical`.
fn deliver(
    ctx: &mut Context<'_>,
    title: &str,
    body: &str,
    severity: NotifySeverity,
    critical: bool,
    summary: Option<String>,
) -> ActionResult {
    let shown_title = plain(title, MAX_TITLE, false);
    let body = plain(body, MAX_BODY, true);
    let delivery = ctx.notify_delivered(
        severity,
        plain(&shown_title, SENT_TITLE, false),
        plain(&body, SENT_BODY, true),
    );
    let outcome = match delivery {
        Ok(Some(d)) if d.shown => Ok((d.desktop, d.status_bar)),
        Ok(Some(d)) => Err(d
            .reason
            .unwrap_or_else(|| "it could not be shown".to_owned())),
        // An agent that predates deliveries shows it in the status bar.
        Ok(None) => Ok((false, true)),
        Err(error) => Err(refused(&error)),
    };
    let mut output = match outcome {
        Ok((desktop, status_bar)) => Output::message(format!("showed “{shown_title}”"))
            .with("shown", true)
            .with("desktop", desktop)
            .with("statusBar", status_bar),
        Err(reason) => {
            let message = format!("“{shown_title}” was not shown: {reason}");
            if critical {
                return Err(PluginError::new(codes::NOT_SHOWN, message));
            }
            Output::message(message)
                .with("shown", false)
                .with("desktop", false)
                .with("statusBar", false)
                .with("reason", reason)
        }
    };
    if let Some(summary) = summary {
        output = output.with("summary", summary);
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
