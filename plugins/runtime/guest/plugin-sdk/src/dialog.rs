//! Standard dialogs (plugin API 1.5): [`Context::dialog`](crate::Context::dialog).
//!
//! A plugin describes a dialog as data with [`Dialog`] and [`Field`]; the
//! agent checks it, DeckPilot shows it with its own components (titled
//! with the plugin's name), and the plugin receives a [`DialogResult`]
//! whose values the agent checked against the request.

pub use deckpilot_plugin_abi::dialog::{
    CancelReason, ChoiceOption, DEFAULT_DIALOG_TIMEOUT, DialogField, DialogRequest, DialogResult,
    MAX_BUTTON_LABEL, MAX_CHOICE_OPTIONS, MAX_DIALOG_FIELDS, MAX_DIALOG_MESSAGE,
    MAX_DIALOG_TIMEOUT, MAX_DIALOG_TITLE, MAX_FIELD_DESCRIPTION, MAX_FIELD_LABEL, MAX_LINE_LENGTH,
    MAX_LINES, MAX_SELECTION_OPTIONS, MAX_TEXT_LENGTH, MIN_DIALOG_TIMEOUT, SelectionOption,
};
use serde_json::{Map, Value};

/// A dialog to show: build one with [`Dialog::confirm`], [`Dialog::alert`],
/// [`Dialog::form`] or [`Dialog::selection`], then set what it needs.
/// Members that do not apply to the dialog's type are ignored by the
/// builder (they would be refused by the agent).
///
/// ```
/// use deckpilot_plugin_sdk::dialog::{Dialog, Field};
///
/// let dialog = Dialog::form(
///     "Configure repository",
///     [
///         Field::text("repository", "Repository path").required().max_length(200),
///         Field::choice("branch", "Branch", [("main", "main"), ("develop", "develop")])
///             .default_value("main"),
///         Field::boolean("pull", "Pull before opening").default_value(true),
///     ],
/// )
/// .message("Choose the repository this key opens.")
/// .confirm_label("Save");
/// assert_eq!(dialog.request().title(), "Configure repository");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog(DialogRequest);

impl Dialog {
    /// A question with a confirm and a cancel button.
    pub fn confirm(title: impl Into<String>) -> Self {
        Self(DialogRequest::Confirm {
            title: title.into(),
            message: None,
            confirm_label: None,
            cancel_label: None,
            destructive: false,
            timeout_seconds: None,
        })
    }

    /// Information with one button.
    pub fn alert(title: impl Into<String>) -> Self {
        Self(DialogRequest::Alert {
            title: title.into(),
            message: None,
            timeout_seconds: None,
        })
    }

    /// A form of 1 to 12 fields.
    pub fn form(title: impl Into<String>, fields: impl IntoIterator<Item = Field>) -> Self {
        Self(DialogRequest::Form {
            title: title.into(),
            message: None,
            fields: fields.into_iter().map(|f| f.0).collect(),
            confirm_label: None,
            cancel_label: None,
            timeout_seconds: None,
        })
    }

    /// One of 1 to 100 options, as `(id, label)`.
    pub fn selection<I: Into<String>, L: Into<String>>(
        title: impl Into<String>,
        options: impl IntoIterator<Item = (I, L)>,
    ) -> Self {
        Self(DialogRequest::Selection {
            title: title.into(),
            message: None,
            options: options
                .into_iter()
                .map(|(id, label)| SelectionOption {
                    id: id.into(),
                    label: label.into(),
                    description: None,
                })
                .collect(),
            confirm_label: None,
            cancel_label: None,
            timeout_seconds: None,
        })
    }

    /// Plain text below the title (at most 600 characters; line breaks
    /// are kept, nothing else is formatted).
    #[must_use]
    pub fn message(mut self, text: impl Into<String>) -> Self {
        let text = Some(text.into());
        match &mut self.0 {
            DialogRequest::Confirm { message, .. }
            | DialogRequest::Alert { message, .. }
            | DialogRequest::Form { message, .. }
            | DialogRequest::Selection { message, .. } => *message = text,
        }
        self
    }

    /// The primary button's label (not for an alert).
    #[must_use]
    pub fn confirm_label(mut self, label: impl Into<String>) -> Self {
        match &mut self.0 {
            DialogRequest::Confirm { confirm_label, .. }
            | DialogRequest::Form { confirm_label, .. }
            | DialogRequest::Selection { confirm_label, .. } => *confirm_label = Some(label.into()),
            DialogRequest::Alert { .. } => {}
        }
        self
    }

    /// The cancel button's label (not for an alert).
    #[must_use]
    pub fn cancel_label(mut self, label: impl Into<String>) -> Self {
        match &mut self.0 {
            DialogRequest::Confirm { cancel_label, .. }
            | DialogRequest::Form { cancel_label, .. }
            | DialogRequest::Selection { cancel_label, .. } => *cancel_label = Some(label.into()),
            DialogRequest::Alert { .. } => {}
        }
        self
    }

    /// A red confirm button (a confirmation only).
    #[must_use]
    pub fn destructive(mut self) -> Self {
        if let DialogRequest::Confirm { destructive, .. } = &mut self.0 {
            *destructive = true;
        }
        self
    }

    /// How long the dialog stays open: 10 to 300 seconds, 120 without it,
    /// and never longer than the action's own time limit.
    #[must_use]
    pub fn timeout_seconds(self, seconds: u32) -> Self {
        Self(self.0.with_timeout(seconds))
    }

    /// Describes the selection option `id` (at most 200 characters).
    #[must_use]
    pub fn option_description(mut self, id: &str, text: impl Into<String>) -> Self {
        if let DialogRequest::Selection { options, .. } = &mut self.0
            && let Some(option) = options.iter_mut().find(|o| o.id == id)
        {
            option.description = Some(text.into());
        }
        self
    }

    /// The request as it is sent.
    pub fn request(&self) -> &DialogRequest {
        &self.0
    }
}

impl From<Dialog> for DialogRequest {
    fn from(dialog: Dialog) -> Self {
        dialog.0
    }
}

/// One field of a [`Dialog::form`]. There is no secret or password field:
/// a plugin's secrets belong in credential slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field(DialogField);

impl Field {
    /// A string, one line unless [`Field::multiline`].
    pub fn text(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self(DialogField::Text {
            id: id.into(),
            label: label.into(),
            description: None,
            required: false,
            default: None,
            max_length: None,
            multiline: false,
        })
    }

    /// A whole number.
    pub fn integer(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self(DialogField::Integer {
            id: id.into(),
            label: label.into(),
            description: None,
            required: false,
            default: None,
            min: None,
            max: None,
        })
    }

    /// On or off.
    pub fn boolean(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self(DialogField::Boolean {
            id: id.into(),
            label: label.into(),
            description: None,
            required: false,
            default: None,
        })
    }

    /// One of up to 50 options, as `(value, label)`.
    pub fn choice<V: Into<String>, L: Into<String>>(
        id: impl Into<String>,
        label: impl Into<String>,
        options: impl IntoIterator<Item = (V, L)>,
    ) -> Self {
        Self(DialogField::Choice {
            id: id.into(),
            label: label.into(),
            description: None,
            required: false,
            default: None,
            options: options
                .into_iter()
                .map(|(value, label)| ChoiceOption {
                    value: value.into(),
                    label: label.into(),
                })
                .collect(),
        })
    }

    /// A list of lines (at most 100 of 200 characters).
    pub fn lines(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self(DialogField::Lines {
            id: id.into(),
            label: label.into(),
            description: None,
            required: false,
            default: None,
        })
    }

    /// Help text below the field (at most 200 characters).
    #[must_use]
    pub fn description(mut self, text: impl Into<String>) -> Self {
        let text = Some(text.into());
        match &mut self.0 {
            DialogField::Text { description, .. }
            | DialogField::Integer { description, .. }
            | DialogField::Boolean { description, .. }
            | DialogField::Choice { description, .. }
            | DialogField::Lines { description, .. } => *description = text,
        }
        self
    }

    /// The user must fill it in.
    #[must_use]
    pub fn required(mut self) -> Self {
        match &mut self.0 {
            DialogField::Text { required, .. }
            | DialogField::Integer { required, .. }
            | DialogField::Boolean { required, .. }
            | DialogField::Choice { required, .. }
            | DialogField::Lines { required, .. } => *required = true,
        }
        self
    }

    /// The value the field starts with, of its type (a string for text and
    /// choice, an integer, a boolean, a list of strings for lines). A value
    /// of another type is ignored.
    #[must_use]
    pub fn default_value(mut self, value: impl Into<Value>) -> Self {
        let value = value.into();
        match &mut self.0 {
            DialogField::Text { default, .. } | DialogField::Choice { default, .. } => {
                *default = value.as_str().map(str::to_owned).or(default.take());
            }
            DialogField::Integer { default, .. } => *default = value.as_i64().or(*default),
            DialogField::Boolean { default, .. } => *default = value.as_bool().or(*default),
            DialogField::Lines { default, .. } => {
                *default = serde_json::from_value(value).ok().or(default.take());
            }
        }
        self
    }

    /// At most `max` characters (text; 1 to 2,000).
    #[must_use]
    pub fn max_length(mut self, max: u32) -> Self {
        if let DialogField::Text { max_length, .. } = &mut self.0 {
            *max_length = Some(max);
        }
        self
    }

    /// Several lines of text (text).
    #[must_use]
    pub fn multiline(mut self) -> Self {
        if let DialogField::Text { multiline, .. } = &mut self.0 {
            *multiline = true;
        }
        self
    }

    /// The smallest and largest value (integer).
    #[must_use]
    pub fn range(mut self, low: Option<i64>, high: Option<i64>) -> Self {
        if let DialogField::Integer { min, max, .. } = &mut self.0 {
            *min = low;
            *max = high;
        }
        self
    }
}

/// Reading a [`DialogResult`].
pub trait DialogResultExt {
    /// The form's values (empty for any other answer).
    fn values(&self) -> Map<String, Value>;
    /// The chosen option of a selection.
    fn selected(&self) -> Option<&str>;
    /// A text or choice value.
    fn text(&self, id: &str) -> Option<String>;
    /// An integer value.
    fn integer(&self, id: &str) -> Option<i64>;
    /// A boolean value.
    fn flag(&self, id: &str) -> Option<bool>;
    /// A lines value.
    fn lines(&self, id: &str) -> Vec<String>;
    /// Why it was cancelled; `None` when confirmed.
    fn cancel_reason(&self) -> Option<CancelReason>;
}

impl DialogResultExt for DialogResult {
    fn values(&self) -> Map<String, Value> {
        match self {
            DialogResult::Confirmed {
                values: Some(values),
                ..
            } => values.clone(),
            _ => Map::new(),
        }
    }

    fn selected(&self) -> Option<&str> {
        match self {
            DialogResult::Confirmed { selected, .. } => selected.as_deref(),
            DialogResult::Cancelled { .. } => None,
        }
    }

    fn text(&self, id: &str) -> Option<String> {
        self.values().get(id)?.as_str().map(str::to_owned)
    }

    fn integer(&self, id: &str) -> Option<i64> {
        self.values().get(id)?.as_i64()
    }

    fn flag(&self, id: &str) -> Option<bool> {
        self.values().get(id)?.as_bool()
    }

    fn lines(&self, id: &str) -> Vec<String> {
        self.values()
            .get(id)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    fn cancel_reason(&self) -> Option<CancelReason> {
        match self {
            DialogResult::Cancelled { reason } => Some(*reason),
            DialogResult::Confirmed { .. } => None,
        }
    }
}

/// What [`MockHost::allow_all`](crate::testing::MockHost::allow_all)
/// answers: confirmed with the defaults (the first option of a selection).
pub(crate) fn confirmed_with_defaults(request: &DialogRequest) -> DialogResult {
    match request {
        DialogRequest::Confirm { .. } | DialogRequest::Alert { .. } => DialogResult::Confirmed {
            values: None,
            selected: None,
        },
        DialogRequest::Selection { options, .. } => DialogResult::Confirmed {
            values: None,
            selected: options.first().map(|o| o.id.clone()),
        },
        DialogRequest::Form { fields, .. } => {
            let values = fields
                .iter()
                .filter_map(|field| {
                    let default = match field {
                        DialogField::Text { default, .. } | DialogField::Choice { default, .. } => {
                            default.clone().map(Value::from)
                        }
                        DialogField::Integer { default, .. } => default.map(Value::from),
                        DialogField::Boolean { default, .. } => default.map(Value::from),
                        DialogField::Lines { default, .. } => default.clone().map(Value::from),
                    }?;
                    Some((field.id().to_owned(), default))
                })
                .collect();
            DialogResult::Confirmed {
                values: Some(values),
                selected: None,
            }
        }
    }
}
