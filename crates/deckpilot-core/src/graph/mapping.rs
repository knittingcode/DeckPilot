//! References to earlier node outputs.
//!
//! The only syntax is `{{ steps.<node>.output.<key> }}`. Values are inserted
//! as plain text: there are no expressions, filters or code, and nothing is
//! evaluated or passed to a shell.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{Mapping, NodeId, ValueKind};
use crate::variables::{
    ExpressionPart, ReferenceError, ReferenceProblem, Resolution, ResolutionContext, VariableRef,
    VariableResolver, VariableValue, parse_expression,
};

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

/// Fills in the `steps.` references of `template`: the version 1 rendering,
/// kept for tests as the reference the extended grammar must match.
#[cfg(test)]
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

/// Where the references of a mapping are resolved: the run's node outputs,
/// and for other roots the injected resolver in the node's context.
pub(crate) struct Sources<'a> {
    pub outputs: &'a Outputs,
    /// `None` when the run has no variable resolver: `shared.`, `system.`
    /// and `workflow.` references are then unavailable.
    pub variables: Option<&'a dyn VariableResolver>,
    pub context: &'a ResolutionContext,
}

/// Why a mapping has no value. Messages never contain a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MappingError {
    /// The legacy failures of node-output mappings (`mapping`).
    Text(String),
    /// A reference that could not be resolved (`reference-*`).
    Reference(ReferenceError),
}

impl MappingError {
    /// Stable code for [`super::StepError::code`].
    pub fn code(&self) -> &'static str {
        match self {
            Self::Text(_) => "mapping",
            Self::Reference(error) => error.code(),
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Text(message) => message.clone(),
            Self::Reference(error) => error.to_string(),
        }
    }
}

impl From<String> for MappingError {
    fn from(message: String) -> Self {
        Self::Text(message)
    }
}

/// A resolved mapping: text (node outputs, templates), or a typed value of
/// a whole-value reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolved {
    Text(String),
    Value(VariableValue),
}

/// The resolved value of `mapping` and whether it is sensitive.
pub(crate) fn resolve_value(
    mapping: &Mapping,
    sources: &Sources<'_>,
) -> Result<(Resolved, bool), MappingError> {
    match mapping {
        Mapping::Output { reference, default } => match lookup(sources.outputs, reference) {
            Some(value) => Ok((Resolved::Text(value.clone()), false)),
            None => default
                .clone()
                .map(|text| (Resolved::Text(text), false))
                .ok_or_else(|| MappingError::Text(format!("{reference} has no value"))),
        },
        Mapping::Template { template } => render_expression(template, sources),
        Mapping::Variable {
            reference: VariableRef::Step(reference),
            default,
        } => match lookup(sources.outputs, reference) {
            Some(value) => Ok((Resolved::Text(value.clone()), false)),
            None => default
                .clone()
                .map(|value| (Resolved::Value(value), false))
                .ok_or_else(|| MappingError::Text(format!("{reference} has no value"))),
        },
        Mapping::Variable { reference, default } => match variable(reference, sources) {
            Resolution::Value {
                value, sensitive, ..
            } => Ok((Resolved::Value(value), sensitive)),
            other => match (other, default) {
                (Resolution::Missing(_), Some(default)) => {
                    Ok((Resolved::Value(default.clone()), false))
                }
                (other, _) => Err(reference_error(None, reference, &other)),
            },
        },
    }
}

/// The text value of `mapping`, for values that are text: workflow
/// inputs, outputs and variables, and parameters filled from text.
pub(crate) fn resolve_text(
    mapping: &Mapping,
    sources: &Sources<'_>,
) -> Result<(String, bool), MappingError> {
    let (resolved, sensitive) = resolve_value(mapping, sources)?;
    match resolved {
        Resolved::Text(text) => Ok((text, sensitive)),
        Resolved::Value(value) => match value.render_text() {
            Some(text) => Ok((text, sensitive)),
            None => Err(MappingError::Text(format!(
                "a {} value cannot be used as text",
                value.value_type().name()
            ))),
        },
    }
}

fn variable(reference: &VariableRef, sources: &Sources<'_>) -> Resolution {
    match sources.variables {
        Some(resolver) => resolver.resolve(reference, sources.context),
        None => Resolution::UnavailableUntilRuntime,
    }
}

pub(crate) fn reference_error(
    parameter: Option<&str>,
    reference: &VariableRef,
    resolution: &Resolution,
) -> MappingError {
    let problem = match resolution {
        Resolution::Missing(reason) => ReferenceProblem::Missing(*reason),
        Resolution::PermissionDenied => ReferenceProblem::PermissionDenied,
        Resolution::UnavailableUntilRuntime | Resolution::Value { .. } => {
            ReferenceProblem::Unavailable
        }
    };
    MappingError::Reference(ReferenceError {
        parameter: parameter.map(str::to_owned),
        reference: reference.to_string(),
        problem,
    })
}

/// Fills in the references of `template` with the extended grammar:
/// node outputs from the run, other roots through the resolver. Values are
/// inserted verbatim and never expanded again.
fn render_expression(
    template: &str,
    sources: &Sources<'_>,
) -> Result<(Resolved, bool), MappingError> {
    let parts = parse_expression(template).map_err(|e| MappingError::Text(e.to_string()))?;
    let mut text = String::new();
    let mut sensitive = false;
    for part in parts {
        match part {
            ExpressionPart::Text(literal) => text.push_str(&literal),
            ExpressionPart::Reference(VariableRef::Step(reference)) => {
                match lookup(sources.outputs, &reference) {
                    Some(value) => text.push_str(value),
                    None => {
                        return Err(MappingError::Text(
                            TemplateError::Missing(reference).to_string(),
                        ));
                    }
                }
            }
            ExpressionPart::Reference(reference) => match variable(&reference, sources) {
                Resolution::Value {
                    value,
                    sensitive: secret,
                    ..
                } => match value.render_text() {
                    Some(rendered) => {
                        sensitive |= secret;
                        text.push_str(&rendered);
                    }
                    None => {
                        return Err(MappingError::Reference(ReferenceError {
                            parameter: None,
                            reference: reference.to_string(),
                            problem: ReferenceProblem::TypeMismatch {
                                found: value.value_type(),
                                expected: "text",
                            },
                        }));
                    }
                },
                other => return Err(reference_error(None, &reference, &other)),
            },
        }
    }
    Ok((Resolved::Text(text), sensitive))
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

    #[test]
    fn the_extended_grammar_renders_node_outputs_like_version_1() {
        let mut outputs = Outputs::new();
        outputs.insert(
            node("repo"),
            BTreeMap::from([("value".to_owned(), "{{ shared.a.b }} $(x)".to_owned())]),
        );
        let context = ResolutionContext::preview(crate::ProfileId::new());
        let sources = Sources {
            outputs: &outputs,
            variables: None,
            context: &context,
        };
        for template in [
            "cd {{ steps.repo.output.value }}!",
            "{{steps.repo.output.value}}{{ steps.repo.output.value }}",
            "plain",
            "",
        ] {
            let (text, sensitive) = resolve_text(
                &Mapping::Template {
                    template: template.into(),
                },
                &sources,
            )
            .unwrap();
            assert_eq!(text, render(template, &outputs).unwrap(), "{template}");
            assert!(!sensitive);
        }
        // Missing outputs fail with the same message as before.
        let missing = Mapping::Template {
            template: "{{ steps.repo.output.other }}".into(),
        };
        assert_eq!(
            resolve_text(&missing, &sources).unwrap_err(),
            MappingError::Text(
                render("{{ steps.repo.output.other }}", &outputs)
                    .unwrap_err()
                    .to_string()
            )
        );
        // Without a resolver, other roots are unavailable, never empty.
        let shared = Mapping::Template {
            template: "cd {{ shared.paths.root }}".into(),
        };
        let error = resolve_text(&shared, &sources).unwrap_err();
        assert_eq!(error.code(), "reference-unavailable");
        assert!(error.message().contains("shared.paths.root"));
    }
}
