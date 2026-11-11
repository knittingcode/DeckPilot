//! References to earlier node outputs.
//!
//! The only syntax is `{{ steps.<node>.output.<key> }}`. Values are inserted
//! as plain text: there are no expressions, filters or code, and nothing is
//! evaluated or passed to a shell.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{Mapping, NodeId, ValueKind};

/// `steps.<node>.output.<key>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub node: NodeId,
    pub output: String,
}

impl Reference {
    /// Parses `steps.<node>.output.<key>` (without braces).
    pub fn parse(text: &str) -> Result<Self, TemplateError> {
        let invalid = || TemplateError::Invalid(text.to_owned());
        let rest = text.trim().strip_prefix("steps.").ok_or_else(invalid)?;
        let (node, rest) = rest.split_once('.').ok_or_else(invalid)?;
        let key = rest.strip_prefix("output.").ok_or_else(invalid)?;
        let valid_key = !key.is_empty()
            && key.len() <= 64
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid_key {
            return Err(invalid());
        }
        Ok(Self {
            node: NodeId::new(node).map_err(|_| invalid())?,
            output: key.to_owned(),
        })
    }
}

impl std::fmt::Display for Reference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "steps.{}.output.{}", self.node, self.output)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateError {
    #[error("`{0}` is not a reference; use {{{{ steps.<node>.output.<key> }}}}")]
    Invalid(String),
    #[error("a `{{{{` is not closed")]
    Unclosed,
    #[error("{0} has no value")]
    Missing(Reference),
}

/// A parsed template: literal text and references, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Reference(Reference),
}

pub fn parse_template(template: &str) -> Result<Vec<Part>, TemplateError> {
    let mut parts = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            parts.push(Part::Text(rest[..start].to_owned()));
        }
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or(TemplateError::Unclosed)?;
        parts.push(Part::Reference(Reference::parse(&after[..end])?));
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest.to_owned()));
    }
    Ok(parts)
}

/// Every reference in `template`.
pub fn references_in(template: &str) -> Result<Vec<Reference>, TemplateError> {
    Ok(parse_template(template)?
        .into_iter()
        .filter_map(|part| match part {
            Part::Reference(reference) => Some(reference),
            Part::Text(_) => None,
        })
        .collect())
}

/// Outputs of finished nodes.
pub type Outputs = BTreeMap<NodeId, BTreeMap<String, String>>;

pub fn lookup<'a>(outputs: &'a Outputs, reference: &Reference) -> Option<&'a String> {
    outputs.get(&reference.node)?.get(&reference.output)
}

/// Fills in the references of `template`.
pub fn render(template: &str, outputs: &Outputs) -> Result<String, TemplateError> {
    let mut text = String::new();
    for part in parse_template(template)? {
        match part {
            Part::Text(literal) => text.push_str(&literal),
            Part::Reference(reference) => match lookup(outputs, &reference) {
                Some(value) => text.push_str(value),
                None => return Err(TemplateError::Missing(reference)),
            },
        }
    }
    Ok(text)
}

/// The text value of `mapping`: a whole output (or its default) or a
/// rendered template. Nothing is evaluated.
pub(crate) fn resolve(mapping: &Mapping, outputs: &Outputs) -> Result<String, String> {
    match mapping {
        Mapping::Output { reference, default } => match lookup(outputs, reference) {
            Some(value) => Ok(value.clone()),
            None => default
                .clone()
                .ok_or_else(|| format!("{reference} has no value")),
        },
        Mapping::Template { template } => render(template, outputs).map_err(|e| e.to_string()),
    }
}

/// `text` checked as a value of `kind`: whole numbers and true/false must
/// parse (and are trimmed), project keys must belong to `profile`. Text,
/// paths and URLs are taken as they are.
pub(crate) fn convert(
    kind: ValueKind,
    text: &str,
    profile: &crate::Profile,
) -> Result<String, String> {
    match kind {
        ValueKind::Text | ValueKind::Path | ValueKind::Url => Ok(text.to_owned()),
        ValueKind::Integer => text
            .trim()
            .parse::<i64>()
            .map(|n| n.to_string())
            .map_err(|_| format!("expects a whole number, got “{text}”")),
        ValueKind::Boolean => match text.trim() {
            value @ ("true" | "false") => Ok(value.to_owned()),
            _ => Err(format!("expects true or false, got “{text}”")),
        },
        ValueKind::ProjectKey => profile
            .projects
            .iter()
            .find(|p| p.key.as_str() == text.trim())
            .map(|p| p.key.to_string())
            .ok_or_else(|| format!("this profile has no project “{text}”")),
    }
}

/// Whether `key` is a valid input or output key: 1–64 letters, digits and
/// `_` (what a reference accepts).
pub(crate) fn is_value_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str) -> NodeId {
        NodeId::new(id).unwrap()
    }

    #[test]
    fn parses_references_and_rejects_anything_else() {
        assert_eq!(
            Reference::parse(" steps.git_status.output.branch ").unwrap(),
            Reference {
                node: node("git_status"),
                output: "branch".into()
            }
        );
        for bad in [
            "steps.x",
            "step.x.output.y",
            "steps.x.output.",
            "steps.X.output.y",
            "steps.x.output.y z",
            "steps.x.output.$(rm)",
            "1 + 1",
        ] {
            assert!(Reference::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn renders_plain_text_without_evaluating_values() {
        let mut outputs = Outputs::new();
        outputs.insert(
            node("repo"),
            BTreeMap::from([(
                "value".to_owned(),
                "$(touch /tmp/x) {{ steps.a.output.b }}".to_owned(),
            )]),
        );
        let text = render("cd {{ steps.repo.output.value }}!", &outputs).unwrap();
        // The value is inserted verbatim; placeholders inside values are not
        // expanded again.
        assert_eq!(text, "cd $(touch /tmp/x) {{ steps.a.output.b }}!");
        assert_eq!(
            render("{{ steps.repo.output.other }}", &outputs),
            Err(TemplateError::Missing(Reference {
                node: node("repo"),
                output: "other".into()
            }))
        );
        assert_eq!(
            render("{{ steps.repo.output.value", &outputs),
            Err(TemplateError::Unclosed)
        );
        assert_eq!(
            render("no placeholders", &outputs).unwrap(),
            "no placeholders"
        );
    }
}
