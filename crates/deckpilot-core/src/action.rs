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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionParameters(BTreeMap<String, ParameterValue>);

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
        (ParameterKind::Text { max_length, .. }, ParameterValue::Text(text)) => {
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
        (ParameterKind::Resource { .. }, ParameterValue::Resource(reference)) => match reference {
            ResourceRef::ProjectRole { role } if role.trim().is_empty() => {
                Err(invalid("project role must not be empty".into()))
            }
            _ => Ok(()),
        },
        (kind, _) => Err(ParameterError::WrongType {
            key: key(),
            expected: match kind {
                ParameterKind::Text { .. } => "text",
                ParameterKind::Integer { .. } => "integer",
                ParameterKind::Boolean => "boolean",
                ParameterKind::Choice { .. } => "choice",
                ParameterKind::Resource { .. } => "resource",
                ParameterKind::Project => "project",
            },
        }),
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
