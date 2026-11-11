//! Action types, their parameter schemas, and configured action instances.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ActionInstanceId, IdentifierError, ProjectId, ResourceKind, ResourceRef};

/// Identifier of an action type, e.g. `projects.next`. The first segment is
/// the id of the module that provides the type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ActionTypeId(String);

impl ActionTypeId {
    pub fn new(id: impl Into<String>) -> Result<Self, IdentifierError> {
        let id = id.into();
        crate::id::validate_dotted("action type id", &id)?;
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ActionTypeId {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ActionTypeId> for String {
    fn from(value: ActionTypeId) -> Self {
        value.0
    }
}

impl fmt::Display for ActionTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The type of value a parameter accepts, including its constraints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ParameterKind {
    Text {
        #[serde(default)]
        multiline: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
    },
    Integer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    Boolean,
    Choice {
        options: Vec<String>,
    },
    /// A logical resource, either fixed or taken from the active project.
    Resource {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ResourceKind>,
    },
    /// A project of the profile the action belongs to.
    Project,
    /// A list edited item by item, stored as text with one item per line
    /// (e.g. command arguments). Items keep their spaces; nothing is split
    /// or interpreted.
    Lines {
        /// What one item is, e.g. "Argument".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<usize>,
        /// Longest item, in characters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
    },
    /// Name/value rows, stored as text with one `name<separator>value` per
    /// line (e.g. HTTP headers with `: `, query parameters with `=`).
    Pairs {
        /// `:` or `=`.
        separator: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<usize>,
        /// Names compare without regard to case (duplicates are found so).
        #[serde(default)]
        case_insensitive: bool,
        /// Names that look like secrets are refused: they belong in a stored
        /// credential.
        #[serde(default)]
        no_secrets: bool,
    },
    /// Source text in a language such as `json`, stored as text.
    Code {
        language: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
    },
    /// The name of a credential stored on this computer, stored as text.
    /// The secret itself is never part of the action.
    Credential,
}

/// Whether a name looks like it holds a secret (`Authorization`, cookies,
/// tokens, API keys, passwords).
pub fn looks_secret(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "proxy-authenticate"
            | "key"
    ) || [
        "token", "secret", "api-key", "api_key", "apikey", "password", "session",
    ]
    .iter()
    .any(|word| name.contains(word))
        || (name.contains("auth") && !name.contains("author"))
}

/// The items of a [`ParameterKind::Lines`] value: its non-empty lines.
pub fn lines_of(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.trim().is_empty())
}

/// Schema entry for one parameter of an action type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterSpec {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: ParameterKind,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ParameterValue>,
}

/// A configured parameter value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ParameterValue {
    Text(String),
    Integer(i64),
    Boolean(bool),
    Choice(String),
    Resource(ResourceRef),
    Project(ProjectId),
}

/// Parameter values of an action instance, keyed by [`ParameterSpec::key`].
///
/// A parameter that is not set is absent; `null` in JSON input means the
/// same, so optional parameters such as `project_id` can be cleared with
/// `null`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ActionParameters(BTreeMap<String, ParameterValue>);

impl<'de> Deserialize<'de> for ActionParameters {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values = BTreeMap::<String, Option<ParameterValue>>::deserialize(deserializer)?;
        Ok(Self(
            values
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key, value)))
                .collect(),
        ))
    }
}

impl ActionParameters {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: ParameterValue) -> Self {
        self.0.insert(key.into(), value);
        self
    }

    pub fn get(&self, key: &str) -> Option<&ParameterValue> {
        self.0.get(key)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            ParameterValue::Text(value) | ParameterValue::Choice(value) => Some(value),
            _ => None,
        }
    }

    pub fn integer(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            ParameterValue::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub fn boolean(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            ParameterValue::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    pub fn resource(&self, key: &str) -> Option<&ResourceRef> {
        match self.get(key)? {
            ParameterValue::Resource(value) => Some(value),
            _ => None,
        }
    }

    pub fn project(&self, key: &str) -> Option<ProjectId> {
        match self.get(key)? {
            ParameterValue::Project(value) => Some(*value),
            _ => None,
        }
    }

    /// Every project referenced by a parameter value.
    pub fn projects(&self) -> impl Iterator<Item = ProjectId> + '_ {
        self.0.values().filter_map(|value| match value {
            ParameterValue::Project(id) => Some(*id),
            _ => None,
        })
    }

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut ParameterValue> {
        self.0.values_mut()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ParameterValue)> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Values from `overrides` replace values with the same key.
    #[must_use]
    pub fn overlay(&self, overrides: &ActionParameters) -> ActionParameters {
        let mut merged = self.clone();
        for (key, value) in &overrides.0 {
            merged.0.insert(key.clone(), value.clone());
        }
        merged
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParameterError {
    #[error("unknown parameter `{0}`")]
    Unknown(String),
    #[error("parameter `{0}` is required")]
    Missing(String),
    #[error("parameter `{key}` expects a {expected} value")]
    WrongType { key: String, expected: &'static str },
    #[error("parameter `{key}`: {reason}")]
    Invalid { key: String, reason: String },
}

/// Definition of an action: what it does and which parameters it accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionType {
    pub id: ActionTypeId,
    pub name: String,
    pub description: String,
    pub category: String,
    pub parameters: Vec<ParameterSpec>,
    /// Whether the action can only run while a project is active.
    pub requires_project: bool,
    /// How the action behaves as a workflow node.
    #[serde(default)]
    pub node: crate::graph::NodeSpec,
}

impl ActionType {
    /// Validates `parameters` against the schema and returns them with
    /// defaults filled in. All problems are reported, not only the first one.
    pub fn normalize(
        &self,
        parameters: &ActionParameters,
    ) -> Result<ActionParameters, Vec<ParameterError>> {
        let mut errors: Vec<ParameterError> = parameters
            .0
            .keys()
            .filter(|key| !self.parameters.iter().any(|spec| &spec.key == *key))
            .map(|key| ParameterError::Unknown(key.clone()))
            .collect();

        let mut normalized = ActionParameters::new();
        for spec in &self.parameters {
            match parameters.get(&spec.key).or(spec.default.as_ref()) {
                Some(value) => match check_value(spec, value) {
                    Ok(()) => {
                        normalized.0.insert(spec.key.clone(), value.clone());
                    }
                    Err(error) => errors.push(error),
                },
                None if spec.required => errors.push(ParameterError::Missing(spec.key.clone())),
                None => {}
            }
        }

        if errors.is_empty() {
            Ok(normalized)
        } else {
            Err(errors)
        }
    }
}

fn check_value(spec: &ParameterSpec, value: &ParameterValue) -> Result<(), ParameterError> {
    let key = || spec.key.clone();
    let invalid = |reason: String| ParameterError::Invalid { key: key(), reason };
    match (&spec.kind, value) {
        (
            ParameterKind::Text { max_length, .. } | ParameterKind::Code { max_length, .. },
            ParameterValue::Text(text),
        ) => {
            if spec.required && text.trim().is_empty() {
                return Err(ParameterError::Missing(key()));
            }
            match max_length {
                Some(max) if text.chars().count() > *max => {
                    Err(invalid(format!("must be at most {max} characters")))
                }
                _ => Ok(()),
            }
        }
        (ParameterKind::Integer { min, max }, ParameterValue::Integer(number)) => {
            if min.is_some_and(|min| *number < min) || max.is_some_and(|max| *number > max) {
                Err(invalid(format!("{number} is out of range")))
            } else {
                Ok(())
            }
        }
        (ParameterKind::Boolean, ParameterValue::Boolean(_))
        | (ParameterKind::Project, ParameterValue::Project(_)) => Ok(()),
        (ParameterKind::Choice { options }, ParameterValue::Choice(choice)) => {
            if options.contains(choice) {
                Ok(())
            } else {
                Err(invalid(format!(
                    "`{choice}` is not one of {}",
                    options.join(", ")
                )))
            }
        }
        (
            ParameterKind::Lines {
                max_items,
                max_length,
                ..
            },
            ParameterValue::Text(text),
        ) => check_lines(spec, text, *max_items, *max_length),
        (kind @ ParameterKind::Pairs { .. }, ParameterValue::Text(text)) => {
            check_pairs(spec, kind, text)
        }
        (ParameterKind::Credential, ParameterValue::Text(text)) => {
            let text = text.trim();
            if text.is_empty() {
                return if spec.required {
                    Err(ParameterError::Missing(key()))
                } else {
                    Ok(())
                };
            }
            crate::CredentialId::new(text)
                .map(|_| ())
                .map_err(|_| invalid(format!("“{text}” is not a credential name")))
        }
        (ParameterKind::Resource { .. }, ParameterValue::Resource(reference)) => match reference {
            ResourceRef::ProjectRole { role } if role.trim().is_empty() => {
                Err(invalid("project role must not be empty".into()))
            }
            _ => Ok(()),
        },
        (kind, _) => Err(ParameterError::WrongType {
            key: key(),
            expected: match kind {
                ParameterKind::Text { .. }
                | ParameterKind::Lines { .. }
                | ParameterKind::Pairs { .. }
                | ParameterKind::Code { .. }
                | ParameterKind::Credential => "text",
                ParameterKind::Integer { .. } => "integer",
                ParameterKind::Boolean => "boolean",
                ParameterKind::Choice { .. } => "choice",
                ParameterKind::Resource { .. } => "resource",
                ParameterKind::Project => "project",
            },
        }),
    }
}

fn check_lines(
    spec: &ParameterSpec,
    text: &str,
    max_items: Option<usize>,
    max_length: Option<usize>,
) -> Result<(), ParameterError> {
    let invalid = |reason: String| ParameterError::Invalid {
        key: spec.key.clone(),
        reason,
    };
    let items: Vec<&str> = lines_of(text).collect();
    if spec.required && items.is_empty() {
        return Err(ParameterError::Missing(spec.key.clone()));
    }
    if let Some(max) = max_items
        && items.len() > max
    {
        return Err(invalid(format!("at most {max} items are allowed")));
    }
    if let Some(max) = max_length
        && let Some(index) = items.iter().position(|i| i.chars().count() > max)
    {
        return Err(invalid(format!(
            "item {} is longer than {max} characters",
            index + 1
        )));
    }
    if text.contains('\0') {
        return Err(invalid("items cannot contain null characters".into()));
    }
    Ok(())
}

fn check_pairs(
    spec: &ParameterSpec,
    kind: &ParameterKind,
    text: &str,
) -> Result<(), ParameterError> {
    let ParameterKind::Pairs {
        separator,
        max_items,
        case_insensitive,
        no_secrets,
        ..
    } = kind
    else {
        return Ok(());
    };
    let invalid = |reason: String| ParameterError::Invalid {
        key: spec.key.clone(),
        reason,
    };
    let mut names: Vec<String> = Vec::new();
    for line in lines_of(text) {
        let Some((name, _)) = line.split_once(separator.as_str()) else {
            return Err(invalid(format!("“{line}” has no {separator}")));
        };
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid(format!("“{line}” has no name")));
        }
        if *no_secrets && looks_secret(name) {
            return Err(invalid(format!(
                "{name} looks like a secret; use a stored credential instead"
            )));
        }
        let normalized = if *case_insensitive {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        };
        if names.contains(&normalized) {
            return Err(invalid(format!("{name} appears more than once")));
        }
        names.push(normalized);
    }
    if spec.required && names.is_empty() {
        return Err(ParameterError::Missing(spec.key.clone()));
    }
    match max_items {
        Some(max) if names.len() > *max => Err(invalid(format!("at most {max} rows are allowed"))),
        _ => Ok(()),
    }
}

/// A user-configured action: an action type plus concrete parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionInstance {
    pub id: ActionInstanceId,
    pub name: String,
    pub action_type: ActionTypeId,
    pub parameters: ActionParameters,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Icon name from the UI icon set (lowercase, digits and `-`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Short text shown on a key instead of the name. Supports `{profile}`,
    /// `{project.key}` and `{project.name}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Longest accepted key label.
pub const MAX_LABEL_CHARS: usize = 32;

/// Checks presentation fields shared by actions and bindings.
pub fn validate_presentation(icon: Option<&str>, label: Option<&str>) -> Result<(), String> {
    if let Some(icon) = icon {
        let valid = !icon.is_empty()
            && icon.len() <= 64
            && icon
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid {
            return Err(format!("invalid icon name `{icon}`"));
        }
    }
    if let Some(label) = label
        && label.chars().count() > MAX_LABEL_CHARS
    {
        return Err(format!(
            "label must be at most {MAX_LABEL_CHARS} characters"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_parameters_are_not_set() {
        let parameters: ActionParameters =
            serde_json::from_str(r#"{"project_id": null, "a": {"type": "integer", "value": 1}}"#)
                .unwrap();
        assert_eq!(
            parameters,
            ActionParameters::new().with("a", ParameterValue::Integer(1))
        );
    }
    use crate::ResourceKey;

    fn action_type() -> ActionType {
        ActionType {
            id: ActionTypeId::new("test.action").unwrap(),
            name: "Test".into(),
            description: String::new(),
            category: "Test".into(),
            parameters: vec![
                ParameterSpec {
                    key: "message".into(),
                    label: "Message".into(),
                    description: None,
                    kind: ParameterKind::Text {
                        multiline: false,
                        max_length: Some(5),
                    },
                    required: true,
                    default: None,
                },
                ParameterSpec {
                    key: "count".into(),
                    label: "Count".into(),
                    description: None,
                    kind: ParameterKind::Integer {
                        min: Some(1),
                        max: Some(10),
                    },
                    required: false,
                    default: Some(ParameterValue::Integer(1)),
                },
                ParameterSpec {
                    key: "mode".into(),
                    label: "Mode".into(),
                    description: None,
                    kind: ParameterKind::Choice {
                        options: vec!["a".into(), "b".into()],
                    },
                    required: false,
                    default: None,
                },
                ParameterSpec {
                    key: "target".into(),
                    label: "Target".into(),
                    description: None,
                    kind: ParameterKind::Resource { kind: None },
                    required: false,
                    default: None,
                },
            ],
            requires_project: false,
            node: crate::graph::NodeSpec::default(),
        }
    }

    #[test]
    fn fills_defaults() {
        let params = ActionParameters::new().with("message", ParameterValue::Text("hi".into()));
        let normalized = action_type().normalize(&params).unwrap();
        assert_eq!(normalized.integer("count"), Some(1));
        assert_eq!(normalized.text("message"), Some("hi"));
        assert!(normalized.get("mode").is_none());
    }

    #[test]
    fn reports_every_problem() {
        let params = ActionParameters::new()
            .with("count", ParameterValue::Integer(42))
            .with("mode", ParameterValue::Choice("c".into()))
            .with("extra", ParameterValue::Boolean(true))
            .with("target", ParameterValue::Text("/tmp".into()));
        let errors = action_type().normalize(&params).unwrap_err();
        assert!(errors.contains(&ParameterError::Unknown("extra".into())));
        assert!(errors.contains(&ParameterError::Missing("message".into())));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParameterError::Invalid { key, .. } if key == "count"))
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParameterError::Invalid { key, .. } if key == "mode"))
        );
        assert!(errors.contains(&ParameterError::WrongType {
            key: "target".into(),
            expected: "resource"
        }));
        assert_eq!(errors.len(), 5);
    }

    #[test]
    fn enforces_text_constraints() {
        let too_long =
            ActionParameters::new().with("message", ParameterValue::Text("123456".into()));
        assert!(action_type().normalize(&too_long).is_err());
        let blank = ActionParameters::new().with("message", ParameterValue::Text("  ".into()));
        assert_eq!(
            action_type().normalize(&blank).unwrap_err(),
            vec![ParameterError::Missing("message".into())]
        );
    }

    #[test]
    fn parameter_values_serialize_tagged() {
        let value = ParameterValue::Resource(ResourceRef::Fixed {
            key: ResourceKey::new("shop.repo").unwrap(),
        });
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json,
            r#"{"type":"resource","value":{"type":"fixed","key":"shop.repo"}}"#
        );
        assert_eq!(
            serde_json::from_str::<ParameterValue>(&json).unwrap(),
            value
        );
    }
}

#[cfg(test)]
mod structured_text_tests {
    use super::*;

    fn spec(kind: ParameterKind) -> ActionType {
        ActionType {
            id: ActionTypeId::new("test.structured").unwrap(),
            name: "Test".into(),
            description: String::new(),
            category: String::new(),
            parameters: vec![ParameterSpec {
                key: "value".into(),
                label: "Value".into(),
                description: None,
                kind,
                required: false,
                default: None,
            }],
            requires_project: false,
            node: crate::graph::NodeSpec::default(),
        }
    }

    fn check(kind: ParameterKind, text: &str) -> Result<(), String> {
        spec(kind)
            .normalize(&ActionParameters::new().with("value", ParameterValue::Text(text.into())))
            .map(|_| ())
            .map_err(|e| e[0].to_string())
    }

    #[test]
    fn lists_keep_their_items_and_limits() {
        let kind = || ParameterKind::Lines {
            item_label: None,
            max_items: Some(2),
            max_length: Some(10),
        };
        assert!(check(kind(), "--name\nJohn Smith\n\n").is_ok());
        assert_eq!(lines_of("a b\r\n\n  \nc").collect::<Vec<_>>(), ["a b", "c"]);
        assert!(check(kind(), "a\nb\nc").unwrap_err().contains("at most 2"));
        assert!(
            check(kind(), "much too long here")
                .unwrap_err()
                .contains("item 1")
        );
    }

    #[test]
    fn rows_need_names_without_duplicates_or_secrets() {
        let headers = || ParameterKind::Pairs {
            separator: ":".into(),
            name_label: None,
            value_label: None,
            max_items: None,
            case_insensitive: true,
            no_secrets: true,
        };
        assert!(check(headers(), "Accept: application/json\nX-Id: 7").is_ok());
        assert!(
            check(headers(), "Accept: a\naccept: b")
                .unwrap_err()
                .contains("more than once")
        );
        assert!(
            check(headers(), "no separator")
                .unwrap_err()
                .contains("has no :")
        );
        assert!(check(headers(), ": value").unwrap_err().contains("no name"));
        assert!(
            check(headers(), "X-Api-Key: 1")
                .unwrap_err()
                .contains("stored credential")
        );
        assert!(check(headers(), "X-Author: me").is_ok());
        let query = ParameterKind::Pairs {
            separator: "=".into(),
            name_label: None,
            value_label: None,
            max_items: None,
            case_insensitive: false,
            no_secrets: false,
        };
        assert!(check(query, "a=1\nA=2").is_ok());
    }

    #[test]
    fn credentials_are_names_and_code_is_bounded_text() {
        assert!(check(ParameterKind::Credential, "github.token").is_ok());
        assert!(check(ParameterKind::Credential, "").is_ok());
        assert!(check(ParameterKind::Credential, "Not A Name").is_err());
        let code = ParameterKind::Code {
            language: "json".into(),
            max_length: Some(5),
        };
        assert!(check(code, "{\"long\": 1}").is_err());
        // Stored values stay plain text, as before.
        let json = serde_json::to_value(ParameterKind::Credential).unwrap();
        assert_eq!(json, serde_json::json!({"type": "credential"}));
    }
}
