//! Where a shared variable is used: references (`{ "type": "reference" }`
//! parameter values, `variable`/`output` mappings) and templates (`{
//! "type": "template" }` parameter values, template mappings, key labels)
//! in the profiles. Used to refuse deleting a variable, or changing its
//! type, while anything refers to it.
//!
//! The scan works on the serialized form of actions, workflows and
//! bindings, so it covers every place such a value can be stored —
//! parameters, step and node overrides, node mappings, workflow inputs,
//! inline invocations — without knowing each shape. Plain `text` values are
//! never parsed: `{{` in them is literal text.

use serde::Serialize;
use serde_json::Value;

use deckpilot_core::graph::NodeId;
use deckpilot_core::trigger::TriggerKind;
use deckpilot_core::variables::{VariableKey, VariableRef, references_in_expression};
use deckpilot_core::{ActionInstanceId, LayoutId, PageId, Profile, ProfileId, Trigger, WorkflowId};
use deckpilot_device::ControlId;

/// One place that uses a shared variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum VariableUse {
    /// A parameter or the label of an action instance.
    Action { action: ActionInstanceId },
    /// A step, node, input or other part of a workflow.
    Workflow {
        workflow: WorkflowId,
        /// The graph node, when the use is in one.
        #[serde(skip_serializing_if = "Option::is_none")]
        node: Option<NodeId>,
    },
    /// The label (or inline action) of a layout binding.
    Binding {
        layout: LayoutId,
        page: PageId,
        control: ControlId,
        trigger: Trigger,
    },
    /// A trigger of a workflow that uses the variable. Indirect: it goes
    /// away with the workflow's use.
    Trigger {
        workflow: WorkflowId,
        index: usize,
        kind: TriggerKind,
    },
}

/// A use with names for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariableReference {
    pub profile: ProfileId,
    pub profile_name: String,
    #[serde(rename = "use")]
    pub site: VariableUse,
    /// The action, workflow or layout name.
    pub owner_name: String,
    /// Where inside it, e.g. `parameters.project_id` or
    /// `graph.nodes[2].mappings.message`.
    pub path: String,
}

/// Shared variable keys referenced anywhere in `value`, with the JSON path
/// of each reference.
fn shared_in(value: &Value, path: &str, found: &mut Vec<(VariableKey, String)>) {
    let mut add = |reference: VariableRef| {
        if let VariableRef::Shared(key) = reference {
            found.push((key, path.to_owned()));
        }
    };
    match value {
        Value::Object(fields) => {
            let kind = fields.get("type").and_then(Value::as_str);
            let text = |name: &str| fields.get(name).and_then(Value::as_str);
            // Parameter values: `{"type": "reference" | "template", "value": …}`.
            match (kind, text("value")) {
                (Some("reference"), Some(reference)) => {
                    if let Ok(reference) = VariableRef::parse(reference) {
                        add(reference);
                    }
                }
                (Some("template"), Some(template)) => {
                    for reference in references_in_expression(template).unwrap_or_default() {
                        add(reference);
                    }
                }
                _ => {}
            }
            // Mappings: `{"reference": …}` (output, variable) and
            // `{"template": …}`.
            if let Some(reference) = text("reference")
                && let Ok(reference) = VariableRef::parse(reference)
            {
                add(reference);
            }
            if let Some(template) = text("template") {
                for reference in references_in_expression(template).unwrap_or_default() {
                    add(reference);
                }
            }
            for (name, child) in fields {
                let child_path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                shared_in(child, &child_path, found);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                shared_in(item, &format!("{path}[{index}]"), found);
            }
        }
        _ => {}
    }
}

/// Keys referenced in a label: labels are templates (decision D6).
fn shared_in_label(label: Option<&str>, path: &str, found: &mut Vec<(VariableKey, String)>) {
    if let Some(label) = label {
        for reference in references_in_expression(label).unwrap_or_default() {
            if let VariableRef::Shared(key) = reference {
                found.push((key, path.to_owned()));
            }
        }
    }
}

fn scan<T: Serialize>(item: &T) -> Vec<(VariableKey, String)> {
    let mut found = Vec::new();
    if let Ok(value) = serde_json::to_value(item) {
        shared_in(&value, "", &mut found);
    }
    found
}

/// The graph node a path such as `graph.nodes[2].mappings.x` points into.
fn node_of(workflow: &deckpilot_core::Workflow, path: &str) -> Option<NodeId> {
    let index: usize = path
        .strip_prefix("graph.nodes[")?
        .split(']')
        .next()?
        .parse()
        .ok()?;
    Some(workflow.graph.as_ref()?.nodes.get(index)?.id.clone())
}

/// Every use of `key` in `profile`.
pub fn variable_references(profile: &Profile, key: &VariableKey) -> Vec<VariableReference> {
    let reference = |site, owner_name: &str, path: String| VariableReference {
        profile: profile.id,
        profile_name: profile.name.clone(),
        site,
        owner_name: owner_name.to_owned(),
        path,
    };
    let mut references = Vec::new();
    for action in &profile.actions {
        let mut found = scan(&action.parameters)
            .into_iter()
            .map(|(k, path)| (k, format!("parameters.{path}")))
            .collect::<Vec<_>>();
        shared_in_label(action.label.as_deref(), "label", &mut found);
        for (_, path) in found.into_iter().filter(|(k, _)| k == key) {
            references.push(reference(
                VariableUse::Action { action: action.id },
                &action.name,
                path,
            ));
        }
    }
    for workflow in &profile.workflows {
        let before = references.len();
        for (_, path) in scan(workflow).into_iter().filter(|(k, _)| k == key) {
            references.push(reference(
                VariableUse::Workflow {
                    workflow: workflow.id,
                    node: node_of(workflow, &path),
                },
                &workflow.name,
                path,
            ));
        }
        if references.len() > before {
            for (index, trigger) in workflow.triggers.iter().enumerate() {
                references.push(reference(
                    VariableUse::Trigger {
                        workflow: workflow.id,
                        index,
                        kind: trigger.kind.clone(),
                    },
                    &workflow.name,
                    format!("triggers[{index}]"),
                ));
            }
        }
    }
    for layout in &profile.layouts {
        for page in &layout.pages {
            for binding in &page.bindings {
                let mut found = scan(&binding.target)
                    .into_iter()
                    .map(|(k, path)| (k, format!("target.{path}")))
                    .collect::<Vec<_>>();
                shared_in_label(binding.label.as_deref(), "label", &mut found);
                for (_, path) in found.into_iter().filter(|(k, _)| k == key) {
                    references.push(reference(
                        VariableUse::Binding {
                            layout: layout.id,
                            page: page.id,
                            control: binding.control,
                            trigger: binding.trigger,
                        },
                        &layout.name,
                        format!("{} › {path}", page.name),
                    ));
                }
            }
        }
    }
    references
}

#[cfg(test)]
mod tests;
