//! Plugin dialogs (`dialog.open`, plugin API 1.5, `docs/dialogs.md`).
//!
//! A plugin does not draw a modal window: it describes a standard dialog
//! (confirm, alert, form or selection) as data, and DeckPilot shows it
//! with its own components, titled with the plugin's name. The request is
//! strict: unknown fields, unknown dialog types and unknown field kinds are
//! rejected, and every text is bounded plain text. There is no secret or
//! password field kind: secrets belong in credential slots.
//!
//! The answer the app sends is checked against the request here
//! ([`DialogRequest::check_answer`]) before the plugin receives it, so a
//! plugin only ever gets values of the types and within the bounds it
//! asked for.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Longest title, in characters (one line).
pub const MAX_DIALOG_TITLE: usize = 80;
/// Longest message, in characters (plain text; line breaks allowed).
pub const MAX_DIALOG_MESSAGE: usize = 600;
/// Most fields of a `form`.
pub const MAX_DIALOG_FIELDS: usize = 12;
/// Longest field id: `[a-z][a-zA-Z0-9_-]{0,31}`.
pub const MAX_FIELD_ID: usize = 32;
/// Longest field label, in characters.
pub const MAX_FIELD_LABEL: usize = 60;
/// Longest field or option description, in characters.
pub const MAX_FIELD_DESCRIPTION: usize = 200;
/// Largest `maxLength` of a `text` field, and its value without one.
pub const MAX_TEXT_LENGTH: u32 = 2000;
/// Most options of a `choice` field.
pub const MAX_CHOICE_OPTIONS: usize = 50;
/// Most options of a `selection` dialog.
pub const MAX_SELECTION_OPTIONS: usize = 100;
/// Longest option value (`choice`) or id (`selection`), in characters.
pub const MAX_OPTION_VALUE: usize = 64;
/// Longest option label, in characters.
pub const MAX_OPTION_LABEL: usize = 60;
/// Longest `confirmLabel` / `cancelLabel`, in characters.
pub const MAX_BUTTON_LABEL: usize = 24;
/// Most lines of a `lines` field.
pub const MAX_LINES: usize = 100;
/// Longest line of a `lines` field, in characters.
pub const MAX_LINE_LENGTH: usize = 200;
/// Shortest, longest and default `timeoutSeconds`.
pub const MIN_DIALOG_TIMEOUT: u32 = 10;
pub const MAX_DIALOG_TIMEOUT: u32 = 300;
pub const DEFAULT_DIALOG_TIMEOUT: u32 = 120;
/// Largest serialized request (it travels to the app in one IPC message).
pub const MAX_DIALOG_REQUEST_BYTES: usize = 32 << 10;
/// Largest serialized answer `values` (it travels back in one IPC message).
pub const MAX_DIALOG_ANSWER_BYTES: usize = 48 << 10;

/// The arguments of `dialog.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum DialogRequest {
    /// A question with a confirm and a cancel button.
    #[serde(rename_all = "camelCase")]
    Confirm {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirm_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_label: Option<String>,
        /// A red confirm button, for something that cannot be undone.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        destructive: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_seconds: Option<u32>,
    },
    /// Information with one button.
    #[serde(rename_all = "camelCase")]
    Alert {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_seconds: Option<u32>,
    },
    /// Typed fields; the answer has their values by id.
    #[serde(rename_all = "camelCase")]
    Form {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        fields: Vec<DialogField>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirm_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_seconds: Option<u32>,
    },
    /// One of a list; the answer is the chosen option's id.
    #[serde(rename_all = "camelCase")]
    Selection {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        options: Vec<SelectionOption>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirm_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_seconds: Option<u32>,
    },
}

/// One field of a `form`, by `kind`. The manifest's parameter kinds, minus
/// those that make no sense in a dialog; never a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum DialogField {
    /// A string, one line unless `multiline`.
    #[serde(rename_all = "camelCase")]
    Text {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// A non-empty value is needed.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        required: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
        /// 1 to [`MAX_TEXT_LENGTH`] characters; that much without it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u32>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
    },
    /// A whole number within `min` and `max`.
    #[serde(rename_all = "camelCase")]
    Integer {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        required: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    /// On or off.
    #[serde(rename_all = "camelCase")]
    Boolean {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        required: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<bool>,
    },
    /// One of `options`, by value.
    #[serde(rename_all = "camelCase")]
    Choice {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        required: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<String>,
        options: Vec<ChoiceOption>,
    },
    /// A list of lines (at most [`MAX_LINES`] of [`MAX_LINE_LENGTH`]
    /// characters).
    #[serde(rename_all = "camelCase")]
    Lines {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        required: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<Vec<String>>,
    },
}

/// One option of a `choice` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

/// One option of a `selection` dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The `ok` value of `dialog.open`. Unknown fields are **ignored** (host to
/// guest only), and a reason added later reads as
/// [`CancelReason::Other`], so the agent can add to it within ABI 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum DialogResult {
    /// The user pressed the primary button. A `form` has `values` (by
    /// field id; a field the user left empty is absent), a `selection`
    /// has `selected`; `confirm` and `alert` have neither.
    Confirmed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        values: Option<Map<String, Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected: Option<String>,
    },
    /// The dialog ended without the primary button.
    Cancelled { reason: CancelReason },
}

impl DialogResult {
    /// Cancelled for `reason`.
    pub fn cancelled(reason: CancelReason) -> Self {
        Self::Cancelled { reason }
    }

    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }
}

/// Why a dialog was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CancelReason {
    /// Cancel, Escape or the close button.
    User,
    /// `timeoutSeconds` (or the invocation's deadline) passed.
    Timeout,
    /// The plugin was disabled, updated or uninstalled, or crashed. A
    /// stopped plugin never receives it; the app is told.
    PluginStopped,
    /// DeckPilot's window closed.
    AppClosed,
    /// A reason this guest does not know yet.
    #[serde(other)]
    Other,
}

/// Why a dialog request or answer was refused: the member, never its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogError(pub &'static str);

impl std::fmt::Display for DialogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for DialogError {}

fn err<T>(what: &'static str) -> Result<T, DialogError> {
    Err(DialogError(what))
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

/// Plain text of one line: 1 to `max` characters, not only white space, no
/// control characters.
fn one_line(text: &str, max: usize, what: &'static str) -> Result<(), DialogError> {
    if text.trim().is_empty() || chars(text) > max || text.chars().any(char::is_control) {
        return err(what);
    }
    Ok(())
}

/// Plain text that may have line breaks (and tabs): at most `max`
/// characters, no other control characters.
fn multi_line(text: &str, max: usize, what: &'static str) -> Result<(), DialogError> {
    if chars(text) > max
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return err(what);
    }
    Ok(())
}

/// `[a-z][a-zA-Z0-9_-]{0,31}`.
pub fn is_field_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    id.len() <= MAX_FIELD_ID
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn button(label: Option<&String>) -> Result<(), DialogError> {
    label.map_or(Ok(()), |l| one_line(l, MAX_BUTTON_LABEL, "button label"))
}

fn timeout(seconds: Option<u32>) -> Result<(), DialogError> {
    match seconds {
        Some(s) if !(MIN_DIALOG_TIMEOUT..=MAX_DIALOG_TIMEOUT).contains(&s) => err("timeoutSeconds"),
        _ => Ok(()),
    }
}

fn unique<'a>(items: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    items.into_iter().all(|item| seen.insert(item))
}

impl DialogRequest {
    pub fn title(&self) -> &str {
        match self {
            Self::Confirm { title, .. }
            | Self::Alert { title, .. }
            | Self::Form { title, .. }
            | Self::Selection { title, .. } => title,
        }
    }

    fn message(&self) -> Option<&String> {
        match self {
            Self::Confirm { message, .. }
            | Self::Alert { message, .. }
            | Self::Form { message, .. }
            | Self::Selection { message, .. } => message.as_ref(),
        }
    }

    /// The `timeoutSeconds` the plugin asked for (or the default).
    pub fn timeout_seconds(&self) -> u32 {
        match self {
            Self::Confirm {
                timeout_seconds, ..
            }
            | Self::Alert {
                timeout_seconds, ..
            }
            | Self::Form {
                timeout_seconds, ..
            }
            | Self::Selection {
                timeout_seconds, ..
            } => timeout_seconds.unwrap_or(DEFAULT_DIALOG_TIMEOUT),
        }
    }

    /// The same request with `timeoutSeconds` set to `seconds` (what the
    /// app is told the dialog really has).
    #[must_use]
    pub fn with_timeout(mut self, seconds: u32) -> Self {
        match &mut self {
            Self::Confirm {
                timeout_seconds, ..
            }
            | Self::Alert {
                timeout_seconds, ..
            }
            | Self::Form {
                timeout_seconds, ..
            }
            | Self::Selection {
                timeout_seconds, ..
            } => *timeout_seconds = Some(seconds),
        }
        self
    }

    /// Checks every bound of the request (`docs/dialogs.md`, "Request").
    pub fn validate(&self) -> Result<(), DialogError> {
        one_line(self.title(), MAX_DIALOG_TITLE, "title")?;
        if let Some(message) = self.message() {
            multi_line(message, MAX_DIALOG_MESSAGE, "message")?;
        }
        match self {
            Self::Confirm {
                confirm_label,
                cancel_label,
                timeout_seconds,
                ..
            } => {
                button(confirm_label.as_ref())?;
                button(cancel_label.as_ref())?;
                timeout(*timeout_seconds)?;
            }
            Self::Alert {
                timeout_seconds, ..
            } => timeout(*timeout_seconds)?,
            Self::Form {
                fields,
                confirm_label,
                cancel_label,
                timeout_seconds,
                ..
            } => {
                button(confirm_label.as_ref())?;
                button(cancel_label.as_ref())?;
                timeout(*timeout_seconds)?;
                if fields.is_empty() || fields.len() > MAX_DIALOG_FIELDS {
                    return err("fields");
                }
                if !unique(fields.iter().map(DialogField::id)) {
                    return err("field id");
                }
                fields.iter().try_for_each(DialogField::validate)?;
            }
            Self::Selection {
                options,
                confirm_label,
                cancel_label,
                timeout_seconds,
                ..
            } => {
                button(confirm_label.as_ref())?;
                button(cancel_label.as_ref())?;
                timeout(*timeout_seconds)?;
                if options.is_empty() || options.len() > MAX_SELECTION_OPTIONS {
                    return err("options");
                }
                for option in options {
                    one_line(&option.id, MAX_OPTION_VALUE, "option id")?;
                    one_line(&option.label, MAX_OPTION_LABEL, "option label")?;
                    if let Some(description) = &option.description {
                        one_line(description, MAX_FIELD_DESCRIPTION, "option description")?;
                    }
                }
                if !unique(options.iter().map(|o| o.id.as_str())) {
                    return err("option id");
                }
            }
        }
        let size = serde_json::to_vec(self).map_or(usize::MAX, |v| v.len());
        if size > MAX_DIALOG_REQUEST_BYTES {
            return err("dialog");
        }
        Ok(())
    }

    /// Checks the app's answer against the request and builds what the
    /// plugin receives: `values` for a `form` (by field id, typed, within
    /// the field's bounds; required fields present; nothing else),
    /// `selected` for a `selection` (one of its option ids), neither for
    /// `confirm` and `alert`. Text is returned as entered, `\r\n` as `\n`;
    /// lines without the empty ones.
    pub fn check_answer(
        &self,
        values: Option<&Map<String, Value>>,
        selected: Option<&str>,
    ) -> Result<DialogResult, DialogError> {
        match self {
            Self::Confirm { .. } | Self::Alert { .. } => {
                if values.is_some() || selected.is_some() {
                    return err("answer");
                }
                Ok(DialogResult::Confirmed {
                    values: None,
                    selected: None,
                })
            }
            Self::Selection { options, .. } => {
                if values.is_some() {
                    return err("answer");
                }
                match selected {
                    Some(id) if options.iter().any(|o| o.id == id) => Ok(DialogResult::Confirmed {
                        values: None,
                        selected: Some(id.to_owned()),
                    }),
                    _ => err("selected"),
                }
            }
            Self::Form { fields, .. } => {
                if selected.is_some() {
                    return err("answer");
                }
                let Some(values) = values else {
                    return err("values");
                };
                if values.keys().any(|id| !fields.iter().any(|f| f.id() == id)) {
                    return err("unknown field");
                }
                let size = serde_json::to_vec(values).map_or(usize::MAX, |v| v.len());
                if size > MAX_DIALOG_ANSWER_BYTES {
                    return err("values");
                }
                let mut out = Map::new();
                for field in fields {
                    let value = values.get(field.id()).filter(|v| !v.is_null());
                    if let Some(value) = field.check_value(value)? {
                        out.insert(field.id().to_owned(), value);
                    }
                }
                Ok(DialogResult::Confirmed {
                    values: Some(out),
                    selected: None,
                })
            }
        }
    }
}

impl DialogField {
    pub fn id(&self) -> &str {
        match self {
            Self::Text { id, .. }
            | Self::Integer { id, .. }
            | Self::Boolean { id, .. }
            | Self::Choice { id, .. }
            | Self::Lines { id, .. } => id,
        }
    }

    fn common(&self) -> (&str, Option<&String>, bool) {
        match self {
            Self::Text {
                label,
                description,
                required,
                ..
            }
            | Self::Integer {
                label,
                description,
                required,
                ..
            }
            | Self::Boolean {
                label,
                description,
                required,
                ..
            }
            | Self::Choice {
                label,
                description,
                required,
                ..
            }
            | Self::Lines {
                label,
                description,
                required,
                ..
            } => (label, description.as_ref(), *required),
        }
    }

    fn validate(&self) -> Result<(), DialogError> {
        if !is_field_id(self.id()) {
            return err("field id");
        }
        let (label, description, _) = self.common();
        one_line(label, MAX_FIELD_LABEL, "field label")?;
        if let Some(description) = description {
            one_line(description, MAX_FIELD_DESCRIPTION, "field description")?;
        }
        match self {
            Self::Text {
                default,
                max_length,
                multiline,
                ..
            } => {
                if max_length.is_some_and(|m| m == 0 || m > MAX_TEXT_LENGTH) {
                    return err("maxLength");
                }
                if let Some(default) = default {
                    check_text(default, self.max_text(), *multiline, "default")?;
                }
            }
            Self::Integer {
                default, min, max, ..
            } => {
                if let (Some(min), Some(max)) = (min, max)
                    && min > max
                {
                    return err("min");
                }
                if default.is_some_and(|d| !in_range(d, *min, *max)) {
                    return err("default");
                }
            }
            Self::Boolean { .. } => {}
            Self::Choice {
                default, options, ..
            } => {
                if options.is_empty() || options.len() > MAX_CHOICE_OPTIONS {
                    return err("options");
                }
                for option in options {
                    one_line(&option.value, MAX_OPTION_VALUE, "option value")?;
                    one_line(&option.label, MAX_OPTION_LABEL, "option label")?;
                }
                if !unique(options.iter().map(|o| o.value.as_str())) {
                    return err("option value");
                }
                if default
                    .as_ref()
                    .is_some_and(|d| !options.iter().any(|o| &o.value == d))
                {
                    return err("default");
                }
            }
            Self::Lines { default, .. } => {
                if let Some(default) = default {
                    check_lines(default, "default")?;
                }
            }
        }
        Ok(())
    }

    fn max_text(&self) -> usize {
        match self {
            Self::Text { max_length, .. } => max_length.unwrap_or(MAX_TEXT_LENGTH) as usize,
            _ => MAX_TEXT_LENGTH as usize,
        }
    }

    /// The value the plugin receives for this field, or `None` when it is
    /// absent (allowed only for a field that is not required).
    fn check_value(&self, value: Option<&Value>) -> Result<Option<Value>, DialogError> {
        let (_, _, required) = self.common();
        let Some(value) = value else {
            return if required { err("required") } else { Ok(None) };
        };
        let out = match self {
            Self::Text { multiline, .. } => {
                let text = value.as_str().ok_or(DialogError("value type"))?;
                let text = text.replace("\r\n", "\n");
                check_text(&text, self.max_text(), *multiline, "value")?;
                if text.trim().is_empty() {
                    return if required { err("required") } else { Ok(None) };
                }
                Value::String(text)
            }
            Self::Integer { min, max, .. } => {
                let n = value.as_i64().ok_or(DialogError("value type"))?;
                if !in_range(n, *min, *max) {
                    return err("value");
                }
                Value::from(n)
            }
            Self::Boolean { .. } => Value::Bool(value.as_bool().ok_or(DialogError("value type"))?),
            Self::Choice { options, .. } => {
                let choice = value.as_str().ok_or(DialogError("value type"))?;
                if !options.iter().any(|o| o.value == choice) {
                    return err("value");
                }
                Value::String(choice.to_owned())
            }
            Self::Lines { .. } => {
                let items = value.as_array().ok_or(DialogError("value type"))?;
                let lines: Vec<String> = items
                    .iter()
                    .map(|item| item.as_str().map(str::to_owned))
                    .collect::<Option<_>>()
                    .ok_or(DialogError("value type"))?;
                let lines: Vec<String> = lines
                    .into_iter()
                    .filter(|line| !line.trim().is_empty())
                    .collect();
                check_lines(&lines, "value")?;
                if lines.is_empty() {
                    return if required { err("required") } else { Ok(None) };
                }
                Value::from(lines)
            }
        };
        Ok(Some(out))
    }
}

fn in_range(n: i64, min: Option<i64>, max: Option<i64>) -> bool {
    min.is_none_or(|min| n >= min) && max.is_none_or(|max| n <= max)
}

fn check_text(
    text: &str,
    max: usize,
    multiline: bool,
    what: &'static str,
) -> Result<(), DialogError> {
    let control = |c: char| c.is_control() && !(multiline && (c == '\n' || c == '\t'));
    if chars(text) > max || text.chars().any(control) {
        return err(what);
    }
    Ok(())
}

fn check_lines(lines: &[String], what: &'static str) -> Result<(), DialogError> {
    if lines.len() > MAX_LINES
        || lines
            .iter()
            .any(|line| chars(line) > MAX_LINE_LENGTH || line.chars().any(char::is_control))
    {
        return err(what);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: Value) -> Result<DialogRequest, serde_json::Error> {
        serde_json::from_value(value)
    }

    fn form() -> Value {
        json!({
            "type": "form",
            "title": "Configure repository",
            "message": "Choose the repository this key opens.",
            "fields": [
                {"id": "repository", "kind": "text", "label": "Repository path",
                 "required": true, "maxLength": 200},
                {"id": "branch", "kind": "choice", "label": "Branch", "options": [
                    {"value": "main", "label": "main"}, {"value": "develop", "label": "develop"}],
                 "default": "main"},
                {"id": "pull", "kind": "boolean", "label": "Pull before opening", "default": true},
                {"id": "depth", "kind": "integer", "label": "Depth", "min": 1, "max": 50},
                {"id": "remotes", "kind": "lines", "label": "Remotes"},
                {"id": "notes", "kind": "text", "label": "Notes", "multiline": true}
            ],
            "confirmLabel": "Save",
            "timeoutSeconds": 120
        })
    }

    fn valid(value: Value) -> DialogRequest {
        let request = parse(value).expect("parses");
        request.validate().expect("valid");
        request
    }

    fn invalid(value: Value) -> DialogError {
        parse(value)
            .expect("parses")
            .validate()
            .expect_err("invalid")
    }

    fn with(mut base: Value, path: &[&str], value: Value) -> Value {
        let mut at = &mut base;
        for (i, key) in path.iter().enumerate() {
            if i + 1 == path.len() {
                if let Ok(index) = key.parse::<usize>() {
                    at[index] = value.clone();
                } else {
                    at[*key] = value.clone();
                }
            } else if let Ok(index) = key.parse::<usize>() {
                at = &mut at[index];
            } else {
                at = &mut at[*key];
            }
        }
        base
    }

    #[test]
    fn the_documented_requests_are_valid() {
        let request = valid(form());
        assert_eq!(request.timeout_seconds(), 120);
        valid(json!({"type": "confirm", "title": "Delete the branch?",
            "destructive": true, "confirmLabel": "Delete"}));
        valid(json!({"type": "alert", "title": "Done", "message": "Two lines\nof text."}));
        let selection = valid(json!({"type": "selection", "title": "Profile", "options": [
            {"id": "coding", "label": "Coding"},
            {"id": "meetings", "label": "Meetings", "description": "Calls and reviews"}]}));
        assert_eq!(selection.timeout_seconds(), DEFAULT_DIALOG_TIMEOUT);
        assert_eq!(selection.clone().with_timeout(30).timeout_seconds(), 30);
        // The JSON a plugin sends is the JSON it reads back.
        let back = serde_json::to_value(&request).unwrap();
        assert_eq!(back, form());
    }

    #[test]
    fn malformed_requests_do_not_parse() {
        for bad in [
            // Unknown types, kinds and fields.
            json!({"type": "custom", "title": "x"}),
            json!({"type": "alert"}),
            json!({"type": "alert", "title": "x", "html": "<b>x</b>"}),
            json!({"type": "alert", "title": "x", "icon": "lock"}),
            json!({"type": "alert", "title": "x", "blocking": true}),
            json!({"type": "alert", "title": "x", "plugin": "other.plugin"}),
            // `destructive` is for confirm only; options and fields too.
            json!({"type": "alert", "title": "x", "destructive": true}),
            json!({"type": "form", "title": "x", "fields": [], "destructive": true}),
            json!({"type": "alert", "title": "x", "confirmLabel": "OK"}),
            json!({"type": "confirm", "title": "x", "fields": []}),
            json!({"type": "selection", "title": "x"}),
            json!({"type": "form", "title": "x"}),
            json!({"type": "form", "title": 7, "fields": []}),
            json!({"type": "alert", "title": "x", "timeoutSeconds": -1}),
            json!({"type": "alert", "title": "x", "timeoutSeconds": "60"}),
            // Option and field members.
            json!({"type": "selection", "title": "x", "options": [{"id": "a"}]}),
            json!({"type": "selection", "title": "x", "options": [
                {"id": "a", "label": "A", "url": "https://x.test"}]}),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn there_is_no_secret_field_and_kinds_are_closed() {
        for kind in [
            "secret",
            "password",
            "credential",
            "file",
            "html",
            "markdown",
            "project",
        ] {
            let bad = with(form(), &["fields", "0", "kind"], json!(kind));
            assert!(parse(bad).is_err(), "{kind}");
        }
        // Members of another kind are unknown fields.
        for (index, member, value) in [
            ("0", "min", json!(1)),
            ("2", "maxLength", json!(3)),
            ("1", "multiline", json!(true)),
            ("3", "options", json!([])),
            ("0", "secret", json!(true)),
            ("0", "masked", json!(true)),
        ] {
            let bad = with(form(), &["fields", index, member], value);
            assert!(parse(bad.clone()).is_err(), "{bad}");
        }
        // Defaults of the wrong type.
        for (index, value) in [("0", json!(5)), ("2", json!("yes")), ("3", json!(1.5))] {
            let bad = with(form(), &["fields", index, "default"], value);
            assert!(parse(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_limit_is_enforced() {
        let alert = |title: &str, message: &str| json!({"type": "alert", "title": title, "message": message});
        // Title: 1-80 characters, one line, not blank.
        valid(alert(&"é".repeat(80), "m"));
        assert_eq!(invalid(alert(&"t".repeat(81), "m")).0, "title");
        assert_eq!(invalid(alert("", "m")).0, "title");
        assert_eq!(invalid(alert("   ", "m")).0, "title");
        assert_eq!(invalid(alert("two\nlines", "m")).0, "title");
        assert_eq!(invalid(alert("bell\u{7}", "m")).0, "title");
        // Message: at most 600, line breaks allowed, no other control.
        valid(alert("t", &"m".repeat(600)));
        assert_eq!(invalid(alert("t", &"m".repeat(601))).0, "message");
        assert_eq!(invalid(alert("t", "esc\u{1b}[31m")).0, "message");

        // Buttons: at most 24.
        let confirm = |label: &str| json!({"type": "confirm", "title": "t", "confirmLabel": label});
        valid(confirm(&"b".repeat(24)));
        assert_eq!(invalid(confirm(&"b".repeat(25))).0, "button label");
        assert_eq!(invalid(confirm("")).0, "button label");
        let cancel = json!({"type": "confirm", "title": "t", "cancelLabel": "c".repeat(25)});
        assert_eq!(invalid(cancel).0, "button label");

        // Timeout: 10-300.
        for (seconds, ok) in [
            (9, false),
            (10, true),
            (300, true),
            (301, false),
            (0, false),
        ] {
            let r = json!({"type": "alert", "title": "t", "timeoutSeconds": seconds});
            assert_eq!(parse(r).unwrap().validate().is_ok(), ok, "{seconds}");
        }

        // Fields: 1-12, ids unique and well formed.
        let text = |id: &str| json!({"id": id, "kind": "text", "label": "L"});
        let fields = |n: usize| {
            json!({"type": "form", "title": "t",
                "fields": (0..n).map(|i| text(&format!("f{i}"))).collect::<Vec<_>>()})
        };
        valid(fields(12));
        assert_eq!(invalid(fields(13)).0, "fields");
        assert_eq!(invalid(fields(0)).0, "fields");
        let dup = json!({"type": "form", "title": "t", "fields": [text("a"), text("a")]});
        assert_eq!(invalid(dup).0, "field id");
        for id in ["", "A", "1a", "a b", "a.b", "a/b", "é", &"a".repeat(33)] {
            let r = json!({"type": "form", "title": "t", "fields": [text(id)]});
            assert_eq!(invalid(r).0, "field id", "{id:?}");
        }
        for id in ["a", "aB_9-x", &"a".repeat(32)] {
            valid(json!({"type": "form", "title": "t", "fields": [text(id)]}));
        }

        // Label 60, description 200.
        let field = |member: &str, value: Value| with(form(), &["fields", "0", member], value);
        valid(field("label", json!("l".repeat(60))));
        assert_eq!(
            invalid(field("label", json!("l".repeat(61)))).0,
            "field label"
        );
        assert_eq!(invalid(field("label", json!(""))).0, "field label");
        valid(field("description", json!("d".repeat(200))));
        assert_eq!(
            invalid(field("description", json!("d".repeat(201)))).0,
            "field description"
        );
        // Text: maxLength 1-2000, default within it and one line.
        valid(field("maxLength", json!(2000)));
        assert_eq!(invalid(field("maxLength", json!(2001))).0, "maxLength");
        assert_eq!(invalid(field("maxLength", json!(0))).0, "maxLength");
        assert_eq!(
            invalid(field("default", json!("x".repeat(201)))).0,
            "default"
        );
        assert_eq!(invalid(field("default", json!("a\nb"))).0, "default");
        valid(with(form(), &["fields", "5", "default"], json!("a\nb")));
        // Integer: min <= max, default within.
        let int = |member: &str, value: Value| with(form(), &["fields", "3", member], value);
        assert_eq!(invalid(int("min", json!(60))).0, "min");
        assert_eq!(invalid(int("default", json!(51))).0, "default");
        valid(int("default", json!(50)));
        // Choice: 1-50 options, unique values, default among them.
        let options = |n: usize| {
            json!(
                (0..n)
                    .map(|i| json!({"value": format!("v{i}"), "label": "L"}))
                    .collect::<Vec<_>>()
            )
        };
        let choice = |member: &str, value: Value| with(form(), &["fields", "1", member], value);
        let fifty = with(
            choice("options", options(50)),
            &["fields", "1", "default"],
            json!("v0"),
        );
        valid(fifty);
        let fifty_one = with(
            choice("options", options(51)),
            &["fields", "1", "default"],
            json!("v0"),
        );
        assert_eq!(invalid(fifty_one).0, "options");
        assert_eq!(invalid(choice("options", json!([]))).0, "options");
        assert_eq!(invalid(choice("default", json!("other"))).0, "default");
        let dup = json!([{"value": "main", "label": "a"}, {"value": "main", "label": "b"}]);
        assert_eq!(invalid(choice("options", dup)).0, "option value");
        // Lines: at most 100 of 200.
        let lines = |value: Value| with(form(), &["fields", "4", "default"], value);
        valid(lines(json!(vec!["x".repeat(200); 100])));
        assert_eq!(invalid(lines(json!(vec!["x"; 101]))).0, "default");
        assert_eq!(invalid(lines(json!(["x".repeat(201)]))).0, "default");

        // Selection: 1-100 options, unique ids.
        let selection = |n: usize| {
            json!({"type": "selection", "title": "t", "options":
                (0..n).map(|i| json!({"id": format!("o{i}"), "label": "L"})).collect::<Vec<_>>()})
        };
        valid(selection(100));
        assert_eq!(invalid(selection(101)).0, "options");
        assert_eq!(invalid(selection(0)).0, "options");
        let dup = json!({"type": "selection", "title": "t", "options": [
            {"id": "a", "label": "A"}, {"id": "a", "label": "B"}]});
        assert_eq!(invalid(dup).0, "option id");
        let long = json!({"type": "selection", "title": "t", "options": [
            {"id": "a", "label": "A", "description": "d".repeat(201)}]});
        assert_eq!(invalid(long).0, "option description");
    }

    #[test]
    fn answers_are_checked_against_the_request() {
        let request = valid(form());
        let values = |v: Value| v.as_object().cloned().unwrap();
        let ok = request
            .check_answer(
                Some(&values(json!({"repository": "~/code/shop\r\n".trim_end(),
                    "branch": "main", "pull": false, "depth": 3,
                    "remotes": ["origin", "", "  ", "fork"], "notes": "a\r\nb"}))),
                None,
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            json!({"outcome": "confirmed", "values": {"repository": "~/code/shop",
                "branch": "main", "pull": false, "depth": 3, "remotes": ["origin", "fork"],
                "notes": "a\nb"}})
        );
        // Optional fields may be left out (or null, or empty).
        let minimal = request
            .check_answer(
                Some(&values(
                    json!({"repository": "x", "notes": "  ", "depth": null}),
                )),
                None,
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&minimal).unwrap(),
            json!({"outcome": "confirmed", "values": {"repository": "x"}})
        );

        for (bad, what) in [
            (json!({}), "required"),
            (json!({"repository": ""}), "required"),
            (json!({"repository": "   "}), "required"),
            (json!({"repository": 5}), "value type"),
            (json!({"repository": "x".repeat(201)}), "value"),
            (json!({"repository": "a\nb"}), "value"),
            (json!({"repository": "x", "other": 1}), "unknown field"),
            (json!({"repository": "x", "branch": "release"}), "value"),
            (json!({"repository": "x", "branch": 1}), "value type"),
            (json!({"repository": "x", "pull": "true"}), "value type"),
            (json!({"repository": "x", "depth": 0}), "value"),
            (json!({"repository": "x", "depth": 51}), "value"),
            (json!({"repository": "x", "depth": 2.5}), "value type"),
            (json!({"repository": "x", "depth": "3"}), "value type"),
            (
                json!({"repository": "x", "remotes": "origin"}),
                "value type",
            ),
            (json!({"repository": "x", "remotes": [1]}), "value type"),
            (
                json!({"repository": "x", "remotes": vec!["r"; 101]}),
                "value",
            ),
        ] {
            let error = request
                .check_answer(Some(&values(bad.clone())), None)
                .unwrap_err();
            assert_eq!(error.0, what, "{bad}");
        }
        assert_eq!(request.check_answer(None, None).unwrap_err().0, "values");
        assert_eq!(
            request
                .check_answer(Some(&values(json!({"repository": "x"}))), Some("a"))
                .unwrap_err()
                .0,
            "answer"
        );
        // Too large in all: refused.
        let wide = valid(json!({"type": "form", "title": "t", "fields":
            (0..12).map(|i| json!({"id": format!("f{i}"), "kind": "text", "label": "L"}))
                .collect::<Vec<_>>()}));
        let huge: Map<String, Value> = (0..12)
            .map(|i| (format!("f{i}"), json!("\u{1F600}".repeat(2000))))
            .collect();
        assert_eq!(
            wide.check_answer(Some(&huge), None).unwrap_err().0,
            "values"
        );

        let selection = valid(json!({"type": "selection", "title": "t", "options": [
            {"id": "coding", "label": "Coding"}]}));
        assert_eq!(
            serde_json::to_value(selection.check_answer(None, Some("coding")).unwrap()).unwrap(),
            json!({"outcome": "confirmed", "selected": "coding"})
        );
        assert_eq!(
            selection.check_answer(None, Some("other")).unwrap_err().0,
            "selected"
        );
        assert_eq!(
            selection.check_answer(None, None).unwrap_err().0,
            "selected"
        );
        assert_eq!(
            selection
                .check_answer(Some(&Map::new()), Some("coding"))
                .unwrap_err()
                .0,
            "answer"
        );

        let confirm = valid(json!({"type": "confirm", "title": "t"}));
        assert_eq!(
            serde_json::to_value(confirm.check_answer(None, None).unwrap()).unwrap(),
            json!({"outcome": "confirmed"})
        );
        assert!(confirm.check_answer(Some(&Map::new()), None).is_err());
        assert!(confirm.check_answer(None, Some("x")).is_err());
    }

    #[test]
    fn results_read_compatibly() {
        let cancelled = DialogResult::cancelled(CancelReason::PluginStopped);
        assert_eq!(
            serde_json::to_value(&cancelled).unwrap(),
            json!({"outcome": "cancelled", "reason": "pluginStopped"})
        );
        for reason in ["user", "timeout", "appClosed"] {
            let r: DialogResult =
                serde_json::from_value(json!({"outcome": "cancelled", "reason": reason})).unwrap();
            assert!(!r.is_confirmed());
        }
        // A reason or a member added later does not break this guest.
        let later: DialogResult = serde_json::from_value(
            json!({"outcome": "cancelled", "reason": "lockedScreen", "at": 1}),
        )
        .unwrap();
        assert_eq!(later, DialogResult::cancelled(CancelReason::Other));
    }
}
