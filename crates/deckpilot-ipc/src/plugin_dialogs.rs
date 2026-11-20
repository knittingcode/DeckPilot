//! Plugin dialogs (protocol 12, plugin API 1.5, `docs/dialogs.md`).
//!
//! A plugin asks the agent to show a standard dialog (`dialog.open`); the
//! agent checks the request, and sends it to the app as a
//! [`PluginEvent::DialogOpened`](crate::plugins::PluginEvent::DialogOpened),
//! which the app renders with its own templates. The app answers with
//! [`Request::AnswerPluginDialog`](crate::Request::AnswerPluginDialog); the
//! agent validates the answer against the request before the plugin sees
//! it. When the agent ends a dialog itself (timeout, the plugin stopped,
//! the app went away) it sends
//! [`PluginEvent::DialogClosed`](crate::plugins::PluginEvent::DialogClosed).
//!
//! All of it is behind the feature string [`FEATURE`]: the agent lists it in
//! [`PluginRuntimeStatus::features`](crate::plugins::PluginRuntimeStatus::features),
//! and an app that renders plugin dialogs announces it in
//! `subscribePluginEvents { features: ["pluginDialogs"] }` on a protocol 12
//! connection. Only such a connection receives dialogs and may answer them.
//!
//! The spec is declarative data: plain text, typed fields. No markup, no
//! code, no URLs and no images; no secret field kind. Unknown fields are
//! ignored here (agent to app), so the agent may add to it within protocol
//! 12; the app must render text as text.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The feature string of plugin dialogs, on both sides.
pub const FEATURE: &str = "pluginDialogs";

/// Longest dialog id the agent issues (and the app may send back).
pub const MAX_DIALOG_ID: usize = 64;

/// A dialog the app should show for a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDialog {
    /// Random, issued by the agent; known only to the agent and the app.
    pub id: String,
    /// The plugin that asked.
    pub plugin: String,
    /// The plugin's display name from its installed manifest (never text of
    /// the request): the dialog's visible requester.
    pub plugin_name: String,
    /// What to show, as the agent validated it.
    pub spec: DialogSpec,
}

/// The kind of a plugin dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogType {
    Confirm,
    Alert,
    Form,
    Selection,
}

/// A validated dialog request (`docs/dialogs.md`, "Request"). The agent
/// fills in `timeoutSeconds` with the time the dialog really has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogSpec {
    #[serde(rename = "type")]
    pub kind: DialogType,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// `form` only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<DialogField>,
    /// `selection` only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectionOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_label: Option<String>,
    /// `confirm` only: a red confirm button.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub destructive: bool,
    /// How long the dialog stays open before the agent closes it.
    pub timeout_seconds: u32,
}

/// The kind of a form field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Text,
    Integer,
    Boolean,
    Choice,
    Lines,
}

/// One form field. Members that do not apply to `kind` are absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogField {
    pub id: String,
    pub kind: FieldKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    /// Of the field's type: a string (`text`, `choice`), an integer, a
    /// boolean or a list of strings (`lines`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// `text`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiline: bool,
    /// `integer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// `integer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    /// `choice`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ChoiceOption>,
}

/// One option of a `choice` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

/// One option of a `selection` dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The app's answer to a dialog (`answerPluginDialog`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "camelCase", deny_unknown_fields)]
pub enum DialogAnswer {
    /// The primary button. A `form` sends `values` (by field id), a
    /// `selection` sends `selected` (an option id); `confirm` and `alert`
    /// send neither.
    Confirmed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        values: Option<BTreeMap<String, serde_json::Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected: Option<String>,
    },
    /// Cancel, Escape or the close button (`user`), or the app is closing
    /// (`appClosed`).
    Cancelled {
        #[serde(default)]
        reason: AppCancelReason,
    },
}

/// Why the app cancelled a dialog.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppCancelReason {
    #[default]
    User,
    AppClosed,
}

/// Why the agent closed a dialog the app shows
/// ([`PluginEvent::DialogClosed`](crate::plugins::PluginEvent::DialogClosed)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogCloseReason {
    /// Its time ran out.
    Timeout,
    /// The plugin was disabled, updated or uninstalled, crashed, or its
    /// invocation was cancelled or ran out of time.
    PluginStopped,
    /// The app that showed it went away, or another app connection
    /// answered it.
    AppClosed,
    /// Answered (on another connection of the app).
    Answered,
}

/// Checks the identifiers of an answer request.
pub(crate) fn check_answer_ids(dialog: &str, plugin: &str) -> Result<(), String> {
    if dialog.is_empty()
        || dialog.len() > MAX_DIALOG_ID
        || !dialog
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("invalid dialog id".into());
    }
    crate::plugins::check_text("plugin", plugin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_dialog_reads_as_the_agent_sends_it() {
        let dialog: PluginDialog = serde_json::from_value(json!({
            "id": "0f3a", "plugin": "acme.repo", "pluginName": "Repo tools",
            "spec": {
                "type": "form", "title": "Configure repository",
                "fields": [
                    {"id": "repository", "kind": "text", "label": "Repository path",
                     "required": true, "maxLength": 200},
                    {"id": "branch", "kind": "choice", "label": "Branch", "default": "main",
                     "options": [{"value": "main", "label": "main"}]},
                    {"id": "pull", "kind": "boolean", "label": "Pull", "default": true}
                ],
                "confirmLabel": "Save", "timeoutSeconds": 120,
                "later": "ignored"
            }
        }))
        .unwrap();
        assert_eq!(dialog.spec.kind, DialogType::Form);
        assert_eq!(dialog.spec.fields.len(), 3);
        assert_eq!(dialog.spec.fields[0].max_length, Some(200));
        assert_eq!(dialog.spec.fields[1].options[0].value, "main");
        let back = serde_json::to_value(&dialog).unwrap();
        assert_eq!(back["spec"]["type"], "form");
        assert_eq!(back["pluginName"], "Repo tools");
        assert!(back["spec"].get("destructive").is_none());
    }

    #[test]
    fn answers_are_strict() {
        let confirmed: DialogAnswer =
            serde_json::from_value(json!({"outcome": "confirmed", "values": {"pull": true}}))
                .unwrap();
        assert!(matches!(
            confirmed,
            DialogAnswer::Confirmed {
                values: Some(_),
                ..
            }
        ));
        let cancelled: DialogAnswer =
            serde_json::from_value(json!({"outcome": "cancelled"})).unwrap();
        assert_eq!(
            cancelled,
            DialogAnswer::Cancelled {
                reason: AppCancelReason::User
            }
        );
        // The agent's own reasons are not the app's to give.
        for bad in [
            json!({"outcome": "cancelled", "reason": "timeout"}),
            json!({"outcome": "cancelled", "reason": "pluginStopped"}),
            json!({"outcome": "confirmed", "extra": 1}),
            json!({"outcome": "maybe"}),
        ] {
            assert!(
                serde_json::from_value::<DialogAnswer>(bad.clone()).is_err(),
                "{bad}"
            );
        }
        assert!(check_answer_ids("0123abcd", "acme.repo").is_ok());
        assert!(check_answer_ids("", "acme.repo").is_err());
        assert!(check_answer_ids("../x", "acme.repo").is_err());
        assert!(check_answer_ids(&"a".repeat(65), "acme.repo").is_err());
    }
}
